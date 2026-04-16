package thetadatadx

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// The authoritative TLS marker list and pinned-method count are generated
// into timeout_pin_generated_test.go from sdk_surface.toml plus the current
// checked-in Go source tree.

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
// Adding or removing a TLS-reading wrapper requires regenerating
// timeout_pin_generated_test.go so the derived count stays in sync.
// Either direction of drift fails the test with a clear delta.
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
			"TLS-pinned-method count changed: got %d, want exactly %d. If this change is intentional, regenerate timeout_pin_generated_test.go.",
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
