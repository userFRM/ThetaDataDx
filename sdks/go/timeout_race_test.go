package thetadatadx

import (
	"os"
	"strings"
	"sync"
	"testing"
)

// TestTimeoutConcurrent exercises the W3 round-3 fix: the FFI error slot
// is a Rust `thread_local!`, so a goroutine that gets migrated to a
// different OS thread between the cgo call (which sets the error on
// thread A) and the post-call error read (which would read thread B's
// empty slot) would silently mis-classify a real timeout as "no rows".
//
// Generated wrappers pin the goroutine via `runtime.LockOSThread()` +
// deferred unlock, so every concurrent goroutine must independently
// observe its own timeout error. This test fires N goroutines, each
// calling `StockListSymbols(WithTimeoutMs(1))`, and asserts every one
// receives a "Request deadline exceeded" error (not nil).
//
// Gated on `THETADX_TEST_CREDS=path/to/creds.txt`; CI runs it in the
// surfaces job which has creds, local `go test` skips silently.
//
// Also runs clean under `go test -race` — there's no shared state
// mutated across goroutines beyond the per-goroutine OS-thread pin.
func TestTimeoutConcurrent(t *testing.T) {
	credsPath := os.Getenv("THETADX_TEST_CREDS")
	if credsPath == "" {
		t.Skip("set THETADX_TEST_CREDS to a creds.txt path to enable this live test")
	}
	creds, err := CredentialsFromFile(credsPath)
	if err != nil {
		t.Fatalf("creds: %v", err)
	}
	defer creds.Close()
	cfg := ProductionConfig()
	defer cfg.Close()
	client, err := Connect(creds, cfg)
	if err != nil {
		t.Fatalf("connect: %v", err)
	}
	defer client.Close()

	const goroutines = 10
	var wg sync.WaitGroup
	errs := make([]error, goroutines)
	for i := 0; i < goroutines; i++ {
		wg.Add(1)
		go func(idx int) {
			defer wg.Done()
			// 1ms deadline: the FFI call never completes in time. If the
			// OS-thread-pin invariant held, each goroutine sees its own
			// error. If the pin leaked, some goroutines would see a
			// stale/empty slot and return (nil, nil) instead of the
			// timeout error.
			_, e := client.StockListSymbols(WithTimeoutMs(1))
			errs[idx] = e
		}(i)
	}
	wg.Wait()

	for i, e := range errs {
		if e == nil {
			t.Errorf("goroutine %d: StockListSymbols(WithTimeoutMs(1)) returned nil, want timeout error", i)
			continue
		}
		if !strings.Contains(strings.ToLower(e.Error()), "request deadline exceeded") {
			t.Errorf("goroutine %d: error does not mention deadline: %v", i, e)
		}
	}
}
