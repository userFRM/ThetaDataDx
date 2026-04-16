package thetadatadx

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// tlsReaderMarkers is the authoritative list of substrings that
// identify an FFI thread-local error read. Any Go source line
// containing one of these consults the `LAST_ERROR` thread_local in
// `ffi/src/lib.rs`, so the enclosing function MUST have executed
// `runtime.LockOSThread()` (with a matching deferred unlock) before
// reaching that line.
//
// Paired with `GO_TLS_READER_MARKERS` in
// `crates/thetadatadx/build_support/sdk_surface.rs`. The two lists
// MUST stay byte-identical — enforced by the
// `go_tls_marker_list_mirrors_rust` integration test at
// `crates/thetadatadx/tests/go_tls_marker_parity.rs`. A divergent
// addition on either side fails that test naming the missing
// entries.
var tlsReaderMarkers = []string{
	"lastError(",
	"lastErrorRaw(",
	"f.fpssCall(",
	"C.tdx_last_error(",
	// Helpers that themselves read the TLS slot — callers must pin
	// BEFORE invoking them. (The helpers also self-pin defensively,
	// which makes them safe even if a caller forgets, but the
	// static audit is stricter.)
	"stringArrayToGo(",
}

// expectedPinnedMethods is the exact count of Go functions in
// `sdks/go/*.go` (excluding `*_test.go`) that read the FFI's
// thread-local error slot and are therefore required to pin. Updated
// INTENTIONALLY when a TLS-reading method is genuinely added or
// removed — a silent drift in either direction fails the test and
// forces the reviewer to confirm the change is expected.
//
// Current breakdown (W3 round-6):
//   - 61 historical endpoints    (historical.go, 53 via lastErrorRaw +
//                                 8 list endpoints via stringArrayToGo)
//   - 20 fpss_methods.go         (subscribe/unsubscribe + lookup)
//   -  2 utilities               (AllGreeks, ImpliedVolatility)
//   -  2 hand-written client     (Connect, NewFpssClient)
//   -  2 hand-written credentials (NewCredentials, CredentialsFromFile)
//   -  1 hand-written helper     (stringArrayToGo, self-pin defensive)
//
// Total: 88. The TLS helpers themselves (`lastError`, `lastErrorRaw`,
// `fpssCall`) are excluded via `isTLSHelper` — they ARE the read, and
// the test audits their callers.
const expectedPinnedMethods = 88

// TestEveryFFIErrorReaderPinsOSThread is the DETERMINISTIC arm of the
// W3 fix verification (see TestTimeoutConcurrent for the behavioral
// load arm). It walks every non-test .go file in `sdks/go/` and, for
// every function body that contains a TLS-reading marker, requires
// the enclosing function to have executed `runtime.LockOSThread()` +
// `defer runtime.UnlockOSThread()` on lines STRICTLY BEFORE the
// first TLS-reading line.
//
// This is file-agnostic by design: a new Go file added to the
// package that happens to read the FFI error slot will automatically
// be picked up and audited. No allowlist to keep in sync.
//
// The `expectedPinnedMethods` constant is an EXACT-MATCH counter.
// Adding a new TLS-reading wrapper must be accompanied by bumping
// this constant; removing one must decrement it. Either direction of
// drift fails the test with a clear delta, so a reviewer must make
// the change intentional.
//
// Verified as a true negative test (round-6 run): temporarily
// removing the pin from `thetadx.go::CredentialsFromFile` fails the
// test with the file/function/offending-line listed. Same for any
// other pinned method.
func TestEveryFFIErrorReaderPinsOSThread(t *testing.T) {
	_, thisFile, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller failed")
	}
	pkgDir := filepath.Dir(thisFile)

	goFiles, err := filepath.Glob(filepath.Join(pkgDir, "*.go"))
	if err != nil {
		t.Fatalf("glob: %v", err)
	}

	type missingEntry struct {
		file    string
		method  string
		offense string
	}
	var missing []missingEntry
	var pinned []string

	for _, path := range goFiles {
		fname := filepath.Base(path)
		// Skip test files: they may legitimately read TLS to set up
		// fixtures without needing the production pin invariant.
		if strings.HasSuffix(fname, "_test.go") {
			continue
		}
		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatalf("read %s: %v", fname, err)
		}
		// Strip trailing CR so the HasSuffix check works on Windows
		// checkouts that use CRLF line endings. Handwritten .go files
		// without an `eol=lf` rule in .gitattributes land with \r\n on
		// Windows CI runners, which silently broke the count before.
		rawLines := strings.Split(string(data), "\n")
		lines := make([]string, len(rawLines))
		for idx, l := range rawLines {
			lines[idx] = strings.TrimRight(l, "\r")
		}
		for i, line := range lines {
			if !strings.HasPrefix(line, "func ") || !strings.HasSuffix(line, "{") {
				continue
			}
			methodName := extractMethodName(line)
			if methodName == "" || isTLSHelper(methodName) {
				// The helpers ARE the TLS reads; pinning them would
				// be a nested pin, harmless but pointless. Their
				// callers are what the audit enforces.
				continue
			}
			bodyEnd := findMethodBodyEnd(lines, i+1)
			body := lines[i+1 : bodyEnd]
			firstReadIdx := findFirstTLSRead(body)
			if firstReadIdx == -1 {
				continue
			}
			if !hasPinBefore(body, firstReadIdx) {
				missing = append(missing, missingEntry{
					file:    fname,
					method:  methodName,
					offense: strings.TrimSpace(body[firstReadIdx]),
				})
				continue
			}
			pinned = append(pinned, fname+"::"+methodName)
		}
	}

	if len(missing) > 0 {
		t.Errorf("W3 cgo-TLS pin regression: %d method(s) read the FFI error slot without runtime.LockOSThread:", len(missing))
		for _, m := range missing {
			t.Errorf("  - %s :: %s (got: %s)", m.file, m.method, m.offense)
		}
	}

	if len(pinned) != expectedPinnedMethods {
		t.Errorf(
			"TLS-pinned-method count changed: got %d, want exactly %d. If this change is intentional, update expectedPinnedMethods in timeout_pin_test.go.",
			len(pinned),
			expectedPinnedMethods,
		)
		if testing.Verbose() {
			for _, m := range pinned {
				t.Logf("  pinned: %s", m)
			}
		}
	}
}

// isTLSHelper identifies the helpers whose bodies ARE the TLS read
// (so the ones that need to pin are their callers, not these).
func isTLSHelper(name string) bool {
	switch name {
	case "lastError", "lastErrorRaw", "fpssCall":
		return true
	}
	return false
}

func extractMethodName(line string) string {
	rest := strings.TrimPrefix(line, "func ")
	if strings.HasPrefix(rest, "(") {
		end := strings.Index(rest, ") ")
		if end == -1 {
			return ""
		}
		rest = rest[end+len(") "):]
	}
	end := strings.Index(rest, "(")
	if end == -1 {
		return ""
	}
	return strings.TrimSpace(rest[:end])
}

func findMethodBodyEnd(lines []string, from int) int {
	for j := from; j < len(lines); j++ {
		if strings.HasPrefix(lines[j], "}") {
			return j
		}
	}
	return len(lines)
}

func findFirstTLSRead(body []string) int {
	for j, l := range body {
		for _, marker := range tlsReaderMarkers {
			if strings.Contains(l, marker) {
				return j
			}
		}
	}
	return -1
}

func hasPinBefore(body []string, before int) bool {
	sawLock := false
	sawDeferUnlock := false
	for j := 0; j < before && j < len(body); j++ {
		trim := strings.TrimSpace(body[j])
		if trim == "runtime.LockOSThread()" {
			sawLock = true
		}
		if trim == "defer runtime.UnlockOSThread()" {
			sawDeferUnlock = true
		}
	}
	return sawLock && sawDeferUnlock
}
