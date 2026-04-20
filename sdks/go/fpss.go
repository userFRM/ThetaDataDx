package thetadatadx

/*
#include "ffi_bridge.h"
*/
import "C"

import (
	"fmt"
	"runtime"
)

// FPSS event types (FpssQuote, FpssTrade, FpssOpenInterest, FpssOhlcvc,
// FpssControl, FpssRawData, FpssEvent) + kind enum + FpssCtrl* constants
// are generated from crates/thetadatadx/fpss_event_schema.toml; see
// fpss_event_structs.go in this package.

// FpssClient wraps the FPSS real-time streaming handle.
type FpssClient struct {
	handle *C.TdxFpssHandle
}

// NewFpssClient connects to the FPSS streaming servers and returns a client.
//
// Pins the goroutine to one OS thread across the cgo call + TLS error
// read because the FFI's last-error slot is a Rust thread_local.
// See `docs/dev/w3-async-cancellation-design.md` "cgo thread-local
// correctness".
func NewFpssClient(creds *Credentials, config *Config) (*FpssClient, error) {
	if creds == nil || creds.handle == nil {
		return nil, fmt.Errorf("thetadatadx: credentials handle is nil")
	}
	if config == nil || config.handle == nil {
		return nil, fmt.Errorf("thetadatadx: config handle is nil")
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	h := C.tdx_fpss_connect(creds.handle, config.handle)
	if h == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return &FpssClient{handle: h}, nil
}

// fpssCall is called by the generated subscribe/unsubscribe wrappers in
// fpss_methods.go. Every such wrapper has already pinned its goroutine
// via runtime.LockOSThread() so the in-flight cgo call and this
// lastError() read both run on the same OS thread.
func (f *FpssClient) fpssCall(rc C.int) (int, error) {
	if rc < 0 {
		return int(rc), fmt.Errorf("thetadatadx: %s", lastError())
	}
	return int(rc), nil
}

// Subscription represents a single active subscription entry.
type Subscription struct {
	Kind     string // "Quote", "Trade", or "OpenInterest"
	Contract string // "SPY" or "SPY 20260417 550 C"
}

// Close frees the FPSS handle. Call after Shutdown.
func (f *FpssClient) Close() {
	if f.handle != nil {
		C.tdx_fpss_free(f.handle)
		f.handle = nil
	}
}

// DroppedEvents returns the cumulative count of FPSS events dropped
// because the internal receiver was gone (channel disconnected) when
// the callback tried to deliver. Survives Reconnect.
//
// Parity with the Python `tdx.dropped_events()` and TypeScript
// `tdx.droppedEvents()` getters: ops teams diagnosing silent drops on
// production Go consumers get a cheap sample path without scraping
// `RUST_LOG=thetadatadx::ffi::streaming=debug` logs.
//
// Safe to call on a nil / closed handle; returns 0 in either case.
func (f *FpssClient) DroppedEvents() uint64 {
	if f == nil || f.handle == nil {
		return 0
	}
	return uint64(C.tdx_fpss_dropped_events(f.handle))
}
