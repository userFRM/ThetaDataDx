//! Public-API surface smoke for the callback-watchdog deliverable.
//!
//! The behavioural watchdog tests drive the real drain primitive
//! (`StreamingClient::poll_batch`, the single point every drain path
//! funnels the user callback through) against an in-memory ring, so
//! no production FPSS TLS connection is needed. They live in-crate in
//! the `slow_callback_watchdog_tests` module of `src/fpss/mod.rs` and
//! pin down the contract:
//!
//! - `slow_callback_over_armed_threshold_is_counted` — a callback
//!   that sleeps past an armed budget increments `slow_callback_count`.
//! - `disabled_watchdog_never_counts_even_a_slow_callback` — a slow
//!   callback with threshold = 0 must NOT increment the counter.
//! - `fast_callback_under_armed_threshold_is_not_counted` — a
//!   callback inside the budget is never counted.
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
