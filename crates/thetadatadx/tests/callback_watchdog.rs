//! Public-API surface smoke for the callback-watchdog  deliverable.
//!
//! The full live-wire watchdog soak — replicating the io_loop's
//! consumer closure with the slow-callback timer plumbed in — runs
//! in-crate (`crates/thetadatadx/src/fpss/streaming_soak_tests.rs`)
//! where it can drive the Disruptor consumer wiring without standing
//! up a production FPSS TLS connection. Two soak tests there pin
//! down the contract:
//!
//! - `slow_callback_threshold_counts_overbudget_invocations` —
//!   100 events, every 10th sleeps 100 ms, threshold 50 ms,
//!   `slow_callback_count == 10`.
//! - `slow_callback_disabled_when_threshold_zero` — slow callback
//!   plus threshold = 0 must NOT increment the counter.
//!
//! Standing up a `Client` handle for an integration test
//! requires a live gRPC + valid credentials (`async connect`) — the
//! integration runner has neither. The public method handles
//! (`set_slow_callback_threshold`, `slow_callback_count`) are
//! verified at compile time below: a successful build proves the
//! signatures land on `Client`.

use std::time::Duration;

use thetadatadx::Client;

/// Compile-time witness: both `set_slow_callback_threshold` and
/// `slow_callback_count` are reachable on `&Client` and have the
/// expected signatures. The closure is not invoked — pointing at the
/// methods inside a `let _ = || { ... };` is enough to fail the build
/// on a breaking rename / signature change without leaving a
/// dead-named function around.
#[test]
fn slow_callback_api_signature_compiles() {
    let _witness = |client: &Client| {
        client.stream().set_slow_callback_threshold(Duration::ZERO);
        client
            .stream()
            .set_slow_callback_threshold(Duration::from_millis(50));
        client
            .stream()
            .set_slow_callback_threshold(Duration::from_secs(u64::MAX / 2));
        let _: u64 = client.stream().slow_callback_count();
    };
}
