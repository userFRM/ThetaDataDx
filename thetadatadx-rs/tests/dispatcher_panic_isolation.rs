//! Verifies the per-callback panic isolation API surface.
//!
//! The full behavioral test — a callback that panics on event 0, events 1+
//! continuing, `panic_count() == 1` — runs in-crate where it can drive
//! the Disruptor consumer via the test-only `StreamingClient::for_self_join_test`
//! harness without a production FPSS TLS connection. See
//! `thetadatadx-rs/src/fpss/mod.rs` module `panic_isolation_tests`.
//!
//! This integration test verifies the public API shapes exposed on
//! `Client` and `StreamingClient`. A successful build proves the
//! signatures exist; no live connection is required.

use thetadatadx::fpss::StreamingClient;
use thetadatadx::Client;

/// Compile-time witness: `panic_count()` is reachable on `&Client`
/// and returns `u64`. A rename or signature change fails the build.
#[test]
fn panic_count_accessor_present_on_unified_client() {
    let _witness = |client: &Client| {
        let _: u64 = client.stream().panic_count();
    };
}

/// Compile-time witness: `panic_count()` is reachable on `&StreamingClient`
/// (the core streaming client) and returns `u64`.
#[test]
fn panic_count_accessor_present_on_fpss_client() {
    let _witness = |client: &StreamingClient| {
        let _: u64 = client.panic_count();
    };
}
