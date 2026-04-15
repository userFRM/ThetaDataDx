package thetadatadx

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// pinScannedFiles is the list of Go source files whose top-level
// methods must be scanned for the FFI-TLS OS-thread pin invariant.
// Every method in these files that touches the FFI's thread-local
// error slot (lastError, lastErrorRaw, or f.fpssCall) MUST open with:
//
//	runtime.LockOSThread()
//	defer runtime.UnlockOSThread()
//
// Without the pin, the Go runtime can migrate the goroutine between
// successive cgo calls and the error read lands on the wrong OS
// thread (Rust thread_local!, empty on the new thread). See
// `docs/dev/w3-async-cancellation-design.md` "cgo thread-local
// correctness" for the full contract.
//
// The list covers every Go source file in this package that crosses
// the cgo boundary AND reads the FFI error slot. If new Go files are
// added that issue FFI calls with a TLS error read, they must be
// added here too or the invariant is not enforced for them.
var pinScannedFiles = []string{
	"historical.go",   // generated `*_with_options` endpoint wrappers
	"fpss_methods.go", // generated FPSS subscribe / unsubscribe / lookup
	"utilities.go",    // generated AllGreeks / ImpliedVolatility
	"client.go",       // hand-written Connect
	"fpss.go",         // hand-written NewFpssClient
}

// minPinnedMethods is a regression guard against the generator
// silently dropping wrappers. Set below the current exact count so an
// incidental +/- 1 from the spec doesn't flake the test; the main
// signal is "did the generator stop emitting?", not an exact counter.
//
// Current breakdown (roughly): 61 historical endpoints + ~16 fpss
// methods that read the error slot + 2 utilities + 2 hand-written.
const minPinnedMethods = 75

// TestEveryEndpointPinsOSThread is the DETERMINISTIC arm of the W3
// round-3 fix verification (see TestTimeoutConcurrent for the dynamic
// arm). It statically grep-scans each file in `pinScannedFiles` and
// requires every method whose body references the TLS error-reading
// helpers to open with the pin idiom.
//
// The test FAILS on broken code. Verified manually by temporarily
// disabling `runtime.LockOSThread()` in `historical.go` and in
// `fpss_methods.go` and re-running this test: it reported every
// affected method by name, for both files.
//
// The test also enforces a minimum total count via
// `minPinnedMethods` — a regression guard against the generator
// silently dropping wrappers.
func TestEveryEndpointPinsOSThread(t *testing.T) {
	_, thisFile, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller failed")
	}
	pkgDir := filepath.Dir(thisFile)

	type missingEntry struct {
		file   string
		method string
		got    string
	}
	var missing []missingEntry
	totalPinned := 0

	for _, fname := range pinScannedFiles {
		path := filepath.Join(pkgDir, fname)
		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatalf("read %s: %v", fname, err)
		}
		lines := strings.Split(string(data), "\n")
		// Identify methods by the one-line opening pattern our
		// generators / hand-written files use:
		//   `func (<recv>) <Name>(...) (...) {`
		// or `func <Name>(...) (...) {` for package-level fns.
		for i, line := range lines {
			if !strings.HasPrefix(line, "func ") || !strings.HasSuffix(line, "{") {
				continue
			}
			methodName := extractMethodName(line)
			if methodName == "" {
				continue
			}
			// Skip helpers that are themselves called from already-
			// pinned wrappers — their pinning is the caller's
			// responsibility.
			if methodName == "fpssCall" || methodName == "Close" {
				continue
			}

			// Look at the body (from the opening brace to the next
			// `}` at column 0). If any body line references a
			// TLS-error helper, the pin idiom MUST appear earlier in
			// the body — lines above the first TLS-reading line.
			// Hand-written wrappers (Connect, NewFpssClient) may do
			// nil checks before acquiring the pin; that's fine as
			// long as the pin is in place before the cgo call that
			// sets the error slot.
			bodyEnd := findMethodBodyEnd(lines, i+1)
			bodyLines := lines[i+1 : bodyEnd]
			firstTLSRead := findFirstTLSReadLine(bodyLines)
			if firstTLSRead == -1 {
				continue
			}

			if !hasPinBefore(bodyLines, firstTLSRead) {
				// Report the first TLS-reading line as the offense
				// so the error message points at the broken code.
				missing = append(missing, missingEntry{
					fname,
					methodName,
					strings.TrimSpace(bodyLines[firstTLSRead]),
				})
				continue
			}
			totalPinned++
		}
	}

	if len(missing) > 0 {
		t.Errorf("W3 cgo-TLS pin regression: %d method(s) read the FFI error slot without runtime.LockOSThread:", len(missing))
		for _, m := range missing {
			t.Errorf("  - %s :: %s (got: %s)", m.file, m.method, m.got)
		}
	}

	if totalPinned < minPinnedMethods {
		t.Errorf("pinned-method count dropped: got %d, want >= %d (generator may have stopped emitting wrappers)", totalPinned, minPinnedMethods)
	}
}

func extractMethodName(line string) string {
	// Input: `func (c *Client) StockListSymbols(opts ...EndpointOption) ([]string, error) {`
	//   or:  `func AllGreeks(...) (...) {`
	// Output: `StockListSymbols` / `AllGreeks`
	rest := strings.TrimPrefix(line, "func ")
	// Strip the `(receiver) ` block if present.
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

// findFirstTLSReadLine returns the index (within body) of the first
// line that reads the FFI's thread_local error slot, or -1 if none.
func findFirstTLSReadLine(body []string) int {
	for j, l := range body {
		if strings.Contains(l, "lastError(") ||
			strings.Contains(l, "lastErrorRaw(") ||
			strings.Contains(l, "f.fpssCall(") {
			return j
		}
	}
	return -1
}

// hasPinBefore returns true if `body` contains both
// `runtime.LockOSThread()` and `defer runtime.UnlockOSThread()` on
// lines STRICTLY BEFORE `before` (exclusive).
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
