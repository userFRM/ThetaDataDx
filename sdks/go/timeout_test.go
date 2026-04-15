package thetadatadx

import (
	"os"
	"strings"
	"testing"
	"time"
)

// TestStringListEndpointTimeoutReturnsError covers Finding 2 end-to-end:
// `stock_list_symbols` returns `{nullptr, 0}` for both success-empty
// (rare) AND failure-empty (e.g. timeout). With the W3 round-2 fix the
// Go wrapper must consult `tdx_last_error` via `lastErrorRaw()` and
// surface a real error, not silently return `(nil, nil)`.
//
// Gated on `THETADX_TEST_CREDS=path/to/creds.txt` because it needs
// network access; CI runs it in the surfaces job which has creds, local
// `cargo test` skips silently.
func TestStringListEndpointTimeoutReturnsError(t *testing.T) {
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

	// 1ms deadline guarantees the gRPC call doesn't complete in time.
	got, err := client.StockListSymbols(WithTimeoutMs(1))
	if err == nil {
		t.Fatalf("StockListSymbols(WithTimeoutMs(1)) returned (%d symbols, nil), want error", len(got))
	}
	if !strings.Contains(strings.ToLower(err.Error()), "request deadline exceeded") {
		t.Errorf("expected timeout error containing \"Request deadline exceeded\", got: %v", err)
	}

	// Subsequent call on the same handle must succeed (W3 contract).
	got, err = client.StockListSymbols(WithTimeoutMs(60_000))
	if err != nil {
		t.Fatalf("subsequent StockListSymbols after timeout returned %v, want success", err)
	}
	if len(got) == 0 {
		t.Error("expected non-empty symbol list, got 0")
	}
}

// TestWithDeadlineNegativeClampsToImmediate (Finding 4) — a negative
// time.Duration would silently wrap to a multi-century unsigned timeout
// after the uint64 cast. WithDeadline must clamp so the deadline is
// effectively expired (immediate timeout) instead of effectively
// infinite. We pick clamp-to-1ms: "deadline already in the past" fires
// immediately, matches the user's intent.
func TestWithDeadlineNegativeClampsToImmediate(t *testing.T) {
	opts := &EndpointRequestOptions{}
	WithDeadline(-1 * time.Second)(opts)
	if opts.TimeoutMs == nil {
		t.Fatal("WithDeadline(negative) returned nil TimeoutMs, want clamped value")
	}
	if *opts.TimeoutMs != 1 {
		t.Errorf("WithDeadline(-1s) set TimeoutMs=%d, want 1 (clamp-to-immediate)", *opts.TimeoutMs)
	}
}

// TestWithDeadlinePositiveValuePassesThrough — sanity check that the
// clamp doesn't intercept legitimate positive durations.
func TestWithDeadlinePositiveValuePassesThrough(t *testing.T) {
	opts := &EndpointRequestOptions{}
	WithDeadline(60 * time.Second)(opts)
	if opts.TimeoutMs == nil {
		t.Fatal("WithDeadline(60s) returned nil TimeoutMs")
	}
	if *opts.TimeoutMs != 60_000 {
		t.Errorf("WithDeadline(60s) set TimeoutMs=%d, want 60000", *opts.TimeoutMs)
	}
}

// TestWithTimeoutMsZeroPassesThroughAsZero — `0` becomes `Some(0)` at
// the EndpointOption layer; the Rust-side `EndpointArgs::set_timeout_ms`
// then normalizes Some(0) to None. We verify the Go layer doesn't
// intercept the 0 sentinel itself.
func TestWithTimeoutMsZeroPassesThroughAsZero(t *testing.T) {
	opts := &EndpointRequestOptions{}
	WithTimeoutMs(0)(opts)
	if opts.TimeoutMs == nil {
		t.Fatal("WithTimeoutMs(0) returned nil TimeoutMs")
	}
	if *opts.TimeoutMs != 0 {
		t.Errorf("WithTimeoutMs(0) set TimeoutMs=%d, want 0", *opts.TimeoutMs)
	}
}
