//! Verifies the per-callback panic isolation API surface exposed via the C ABI.
//!
//! The behavioral contract — panic on event N is caught, event N+1 continues,
//! `panic_count()` increments — is validated in the core crate's in-crate
//! `panic_isolation_tests` module (see
//! `thetadatadx-rs/src/fpss/mod.rs`). The per-invocation `catch_unwind`
//! boundary is provided by `StreamingClient::for_each` via `poll_batch`; both the
//! Rust dispatcher in `thetadatadx-rs/src/client.rs` and the FFI
//! dispatchers in `thetadatadx-ffi/src/streaming.rs` use `for_each` as the dispatch
//! vehicle, so the isolation guarantee is structural.
//!
//! This file verifies the C ABI entry points exist and link correctly.
//! A build failure here means a symbol was removed or renamed.

use thetadatadx_ffi::{thetadatadx_client_panic_count, thetadatadx_streaming_panic_count};

/// Compile-time + link-time witness: `thetadatadx_streaming_panic_count` exists and takes
/// `*const ThetaDataDxStreamHandle` with return type `u64`. Calling it with a null
/// pointer is defined (returns 0) per the documented null-safety contract.
#[test]
fn thetadatadx_streaming_panic_count_links_and_returns_zero_on_null() {
    // SAFETY: null is explicitly documented as a safe input that returns 0.
    let count = unsafe { thetadatadx_streaming_panic_count(std::ptr::null()) };
    assert_eq!(count, 0, "null-handle panic_count must return 0");
}

/// Compile-time + link-time witness: `thetadatadx_client_panic_count` exists and
/// takes `*const ThetaDataDxClient` with return type `u64`. Calling it with a null
/// pointer is defined (returns 0) per the documented null-safety contract.
#[test]
fn thetadatadx_client_panic_count_links_and_returns_zero_on_null() {
    // SAFETY: null is explicitly documented as a safe input that returns 0.
    let count = unsafe { thetadatadx_client_panic_count(std::ptr::null()) };
    assert_eq!(count, 0, "null-handle panic_count must return 0");
}
