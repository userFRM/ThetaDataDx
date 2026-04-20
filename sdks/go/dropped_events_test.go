package thetadatadx

import (
	"os"
	"testing"
)

// TestDroppedEventsCallableOnNilClient pins the contract from audit
// finding A-04: `FpssClient.DroppedEvents()` must be safe to call
// before / after `Close()` and on a nil receiver. Parity with the
// Python / TypeScript / C++ getters, all of which return 0 instead
// of panicking on an uninitialised handle. This is the defensive
// path — Go consumers often defer `client.Close()` then still touch
// metrics in a post-shutdown cleanup hook, and a panic there would
// mask the real failure.
func TestDroppedEventsCallableOnNilClient(t *testing.T) {
	var f *FpssClient
	if got := f.DroppedEvents(); got != 0 {
		t.Errorf("nil FpssClient.DroppedEvents() = %d, want 0", got)
	}

	empty := &FpssClient{handle: nil}
	if got := empty.DroppedEvents(); got != 0 {
		t.Errorf("closed FpssClient.DroppedEvents() = %d, want 0", got)
	}
}

// TestDroppedEventsSurvivesReconnect is the live-credential sibling of
// the Python / TypeScript dropped_events tests. Gated on
// THETADX_TEST_CREDS because `NewFpssClient` needs a real FPSS
// handshake. Pins the A-02/A-04 contract: the counter stays readable
// across reconnect and is monotonically non-decreasing.
func TestDroppedEventsSurvivesReconnect(t *testing.T) {
	credsPath := os.Getenv("THETADX_TEST_CREDS")
	if credsPath == "" {
		t.Skip("set THETADX_TEST_CREDS=path/to/creds.txt to enable this live test")
	}

	creds, err := CredentialsFromFile(credsPath)
	if err != nil {
		t.Fatalf("creds: %v", err)
	}
	defer creds.Close()

	cfg := ProductionConfig()
	defer cfg.Close()

	client, err := NewFpssClient(creds, cfg)
	if err != nil {
		t.Fatalf("connect: %v", err)
	}
	defer client.Close()

	pre := client.DroppedEvents()

	if err := client.Reconnect(); err != nil {
		t.Fatalf("reconnect: %v", err)
	}
	post := client.DroppedEvents()

	// Monotonically non-decreasing across reconnect. A reset would
	// mean the closure-local counter regression was reintroduced.
	if post < pre {
		t.Errorf("DroppedEvents() reset across reconnect: pre=%d post=%d", pre, post)
	}
}
