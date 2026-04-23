package thetadatadx

import (
	"fmt"
	"os"
	"runtime"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
)

// TestTimeoutConcurrent is a regression guard for the W3 round-3 fix:
// the FFI error slot is a Rust `thread_local!`, so a goroutine migrated
// to a different OS thread between the cgo call (which sets the error
// on thread A) and the post-call error read (which reads thread B's
// empty slot) would silently mis-classify a real timeout as "no rows".
// The fix pins each endpoint call to one OS thread via
// `runtime.LockOSThread()` + deferred unlock.
//
// What this test asserts directly:
//   - Every single call across all goroutines and iterations returns a
//     non-nil error containing "request deadline exceeded".
//   - The race detector is clean across that concurrent load.
//
// What this test CANNOT directly force:
//   - A deterministic cgo-time goroutine migration between the three
//     successive cgo calls inside a single wrapper (clear/call/check).
//     cgo calls are non-preemptible: while a goroutine is executing on
//     the C side, the Go runtime won't migrate it. The migration
//     window is only the brief Go-land gap between successive cgo
//     calls. That window is microseconds wide and the scheduler has
//     no obligation to pick it up.
//
// So this is a regression guard, not a reliable negative test: a
// cleanly-working implementation passes, and a broken implementation
// MAY pass too on lightly-loaded CI because the migration window
// never widens enough. The real deterministic argument is the code
// itself (the pin is on every generated wrapper, a grep on
// `runtime.LockOSThread` shows 61 of 61 non-streaming endpoints) plus
// the `thread_local!`-contract comment in `ffi/src/lib.rs`.
//
// Design choices that still buy us detection in the cases where
// migration DOES happen:
//
//  1. Raise GOMAXPROCS to ensure multiple OS threads are available to
//     host goroutines (baseline is runtime.NumCPU(), usually >= 2).
//  2. Synchronize the start: every goroutine waits on `<-start` before
//     issuing its first call, so the scheduler is under maximum
//     contention at release — any migration is most likely to happen
//     in that window.
//  3. Amplify the sample: each goroutine issues N=100 calls in a tight
//     loop (10 goroutines × 100 calls = 1000 cgo call sequences). Volume
//     alone does not make the detection reliable — on a 1-CPU runner
//     or with a scheduler that keeps the goroutines on one OS thread,
//     the per-call migration probability p can be effectively zero and
//     a broken implementation still passes. This test is a BEHAVIORAL
//     LOAD GUARD, not a statistical proof; TestEveryEndpointPinsOSThread
//     in timeout_pin_test.go is the deterministic arm.
//  4. Per-call assertion: every call must return an error containing
//     "request deadline exceeded". Even one nil-error return from any
//     iteration of any goroutine fails the test with the goroutine id
//     and iteration number in the failure message.
//
// The `-race` detector is orthogonal to the TLS-migration bug (it
// catches Go-managed memory races, not wrong-thread reads of Rust
// TLS), so running under `go test -race` gives both guarantees:
// data-race-free AND no regressions from the pin itself.
//
// Gated on `THETADX_TEST_CREDS=path/to/creds.txt`; CI runs it in the
// surfaces job with creds, local `go test` without creds skips.
func TestTimeoutConcurrent(t *testing.T) {
	credsPath := os.Getenv("THETADX_TEST_CREDS")
	if credsPath == "" {
		t.Skip("set THETADX_TEST_CREDS to a creds.txt path to enable this live test")
	}

	// Force multi-threaded scheduling so migration is possible. On a
	// single-core GOMAXPROCS=1 host the goroutines would share one OS
	// thread and never trigger the bug.
	prevMax := runtime.GOMAXPROCS(0)
	if prevMax < 2 {
		runtime.GOMAXPROCS(2)
		defer runtime.GOMAXPROCS(prevMax)
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

	const (
		goroutines = 10
		iterations = 100
	)

	start := make(chan struct{})
	var wg sync.WaitGroup
	var successCount int64
	// Buffered so goroutines don't block on send; capacity is the
	// worst case (every call fails). Drained after wg.Wait.
	failures := make(chan string, goroutines*iterations)

	for g := 0; g < goroutines; g++ {
		wg.Add(1)
		go func(gid int) {
			defer wg.Done()
			<-start // maximum contention on release
			for i := 0; i < iterations; i++ {
				_, err := client.StockListSymbols(WithTimeoutMs(1))
				if err == nil {
					// The bug signature: SDK returned success (nil
					// error, empty slice) for what should have been
					// a timeout. If we see this even once, the
					// OS-thread pin is not effective.
					failures <- fmt.Sprintf("goroutine=%d iter=%d: expected timeout error, got nil (possible cgo TLS race)", gid, i)
					continue
				}
				msg := strings.ToLower(err.Error())
				if !strings.Contains(msg, "request deadline exceeded") {
					failures <- fmt.Sprintf("goroutine=%d iter=%d: unexpected error text: %v", gid, i, err)
					continue
				}
				atomic.AddInt64(&successCount, 1)
			}
		}(g)
	}

	close(start) // release all goroutines simultaneously
	wg.Wait()
	close(failures)

	for f := range failures {
		t.Error(f)
	}

	want := int64(goroutines * iterations)
	if got := atomic.LoadInt64(&successCount); got != want {
		t.Fatalf("expected %d timeout errors across all goroutines/iterations, got %d", want, got)
	}
}
