//! Verifies that `ffi_boundary!` contains panics inside `extern "C"` bodies.
//!
//! Background: Rust 1.81+ converts a panic unwinding across an `extern "C"`
//! boundary into a process abort (pre-1.81 the behavior was undefined). Either
//! way, a single bad invariant inside any of the ~145 `extern "C"` entry
//! points would take down the host process (C / Go / Python). The
//! `ffi_boundary!` macro wraps every body in `catch_unwind`, converts the
//! panic into the function's declared default return value, and stashes the
//! payload into the thread-local `tdx_last_error` slot.
//!
//! This test calls the two feature-gated panic helpers and asserts all three
//! invariants: process keeps running, the default (`-1`) is returned, and the
//! panic message is readable via `tdx_last_error()`.
//!
//! Only compiled when the `testing-panic-boundary` feature is enabled, so the
//! production shared library never carries the panic-on-demand symbols. Run
//! with: `cargo test -p thetadatadx-ffi --features testing-panic-boundary`.

#![cfg(feature = "testing-panic-boundary")]

use std::ffi::CStr;

use thetadatadx_ffi::{tdx_clear_error, tdx_last_error, tdx_test_panic_str, tdx_test_panic_string};

/// Read `tdx_last_error` into a Rust `String`. Returns an empty string if
/// the slot is null, which lets the test assert "set to something" with
/// `!is_empty()` rather than decoding raw pointers inline.
fn last_error_string() -> String {
    // SAFETY: the returned pointer is owned by the thread-local slot and
    // remains valid until the next FFI call on this thread. We copy into a
    // Rust String before dropping it, so no dangling reference escapes.
    unsafe {
        let p = tdx_last_error();
        if p.is_null() {
            return String::new();
        }
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

#[test]
fn panic_with_static_str_returns_default_and_sets_last_error() {
    // Start from a clean error slot so we can attribute whatever shows up
    // in `tdx_last_error` to *this* call and not a previous test.
    tdx_clear_error();
    assert!(
        last_error_string().is_empty(),
        "precondition: last_error should start cleared"
    );

    // Call the feature-gated extern "C" test helper. It is declared
    // `extern "C"` but also public on the Rust side of the rlib, so we
    // can invoke it without an `unsafe` block — the body panics
    // internally and the `ffi_boundary!` macro must catch that panic.
    // If the catch failed the test binary would abort (Rust 1.81+)
    // rather than reach the assertion below.
    let rc = tdx_test_panic_str();

    assert_eq!(
        rc, -1,
        "panic must surface as the declared default return value (-1 for i32)",
    );

    let err = last_error_string();
    assert!(
        err.starts_with("panic at FFI boundary:"),
        "tdx_last_error should carry the boundary-caught panic prefix, got {err:?}",
    );
    assert!(
        err.contains("intentional test panic via &'static str"),
        "tdx_last_error should include the panic message payload, got {err:?}",
    );
}

#[test]
fn panic_with_string_returns_default_and_sets_last_error() {
    tdx_clear_error();
    assert!(last_error_string().is_empty());

    // See the `&'static str` variant for why this call is safe.
    let rc = tdx_test_panic_string();

    assert_eq!(rc, -1);

    let err = last_error_string();
    assert!(
        err.starts_with("panic at FFI boundary:"),
        "tdx_last_error should carry the boundary-caught panic prefix, got {err:?}",
    );
    assert!(
        err.contains("intentional test panic via String"),
        "tdx_last_error should include the String-typed panic payload, got {err:?}",
    );
}
