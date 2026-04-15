package thetadatadx

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

// TestEveryEndpointPinsOSThread is the DETERMINISTIC counterpart of
// TestTimeoutConcurrent: instead of hoping the Go scheduler triggers
// a goroutine migration in the microsecond window between successive
// cgo calls (which it rarely does under CI load), we statically grep
// the generated `historical.go` for the exact pin idiom that the fix
// installs. Every non-streaming endpoint must open with:
//
//     runtime.LockOSThread()
//     defer runtime.UnlockOSThread()
//
// This directly encodes the W3 round-3 contract. If someone
// accidentally removes the pin from the Go generator (or someone adds
// a new endpoint category that bypasses `render_go_endpoint_method`),
// this test fails immediately with the offending method name.
//
// Deterministic (no concurrency, no timing, no live network).
func TestEveryEndpointPinsOSThread(t *testing.T) {
	_, thisFile, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller failed")
	}
	pkgDir := filepath.Dir(thisFile)
	historical, err := os.ReadFile(filepath.Join(pkgDir, "historical.go"))
	if err != nil {
		t.Fatalf("read historical.go: %v", err)
	}
	source := string(historical)

	// Collect every `func (c *Client) <Name>(...) ...` declaration and
	// check the two lines after the opening brace are the pin idiom.
	// Scanning line-by-line is simpler than parsing Go; the generated
	// file is mechanically uniform.
	lines := strings.Split(source, "\n")
	missing := []string{}
	for i, line := range lines {
		if !strings.HasPrefix(line, "func (c *Client) ") || !strings.HasSuffix(line, "{") {
			continue
		}
		methodName := extractMethodName(line)
		// The next two non-empty lines must be LockOSThread + deferred
		// UnlockOSThread. Skip to the first non-empty post-brace line.
		next1 := nextNonEmpty(lines, i+1)
		next2 := nextNonEmpty(lines, next1+1)
		if next1 == -1 || next2 == -1 {
			missing = append(missing, methodName+" (unexpected EOF)")
			continue
		}
		l1 := strings.TrimSpace(lines[next1])
		l2 := strings.TrimSpace(lines[next2])
		if l1 != "runtime.LockOSThread()" || l2 != "defer runtime.UnlockOSThread()" {
			missing = append(missing, methodName+" (got: "+l1+" / "+l2+")")
		}
	}

	if len(missing) > 0 {
		t.Errorf("W3 round-3 regression: %d generated endpoint(s) missing runtime.LockOSThread pin:", len(missing))
		for _, m := range missing {
			t.Errorf("  - %s", m)
		}
	}
}

func extractMethodName(line string) string {
	// Input: `func (c *Client) StockListSymbols(opts ...EndpointOption) ([]string, error) {`
	// Output: `StockListSymbols`
	start := strings.Index(line, "Client) ")
	if start == -1 {
		return "???"
	}
	rest := line[start+len("Client) "):]
	end := strings.Index(rest, "(")
	if end == -1 {
		return rest
	}
	return rest[:end]
}

func nextNonEmpty(lines []string, from int) int {
	for j := from; j < len(lines); j++ {
		if strings.TrimSpace(lines[j]) != "" {
			return j
		}
	}
	return -1
}
