//! Thread-local error slot plus the `thetadatadx_last_error` / `thetadatadx_clear_error`
//! FFI accessors and the `require_cstr!` macro used by endpoint wrappers.
//!
//! Contract: the error slot is scoped to the OS thread that set it. C++
//! and Python never migrate threads implicitly, so no pinning is needed
//! there. Any third-party FFI consumer whose runtime can migrate a logical
//! execution unit across OS threads (a green-thread runtime that parks on
//! one thread and resumes on another) MUST pin the execution unit for
//! the duration of a clear/call/check sequence — typically via the host
//! runtime's lock-to-OS-thread primitive + deferred unlock.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
    static LAST_ERROR_CODE: std::cell::Cell<i32> = const { std::cell::Cell::new(THETADATADX_ERR_NONE) };
    /// Server-supplied rate-limit back-off in milliseconds, or `-1` when
    /// the last error carries no `RetryInfo` hint. Read via
    /// [`thetadatadx_last_error_retry_after_ms`] so the C++ `RateLimitError`
    /// surfaces the back-off as a typed value.
    static LAST_ERROR_RETRY_AFTER_MS: std::cell::Cell<i64> = const { std::cell::Cell::new(-1) };
}

/// Sentinel returned by [`thetadatadx_last_error_retry_after_ms`] when the last
/// error carries no rate-limit back-off hint.
pub const THETADATADX_RETRY_AFTER_NONE: i64 = -1;

/// Typed error-code discriminants surfaced via [`thetadatadx_last_error_code`].
///
/// Higher-level bindings (the C++ exception hierarchy in
/// `thetadatadx-cpp/include/thetadatadx.hpp`, the typed napi error subclasses in
/// `thetadatadx-ts/src/lib.rs`) use these codes to choose which
/// concrete exception / error subclass to throw without having to
/// substring-match the formatted error string. The mapping mirrors
/// the Python `to_py_err` hierarchy one-for-one so the leaf set stays
/// uniform across bindings.
/// No error: the last call succeeded.
pub const THETADATADX_ERR_NONE: i32 = 0;
/// Uncategorized failure that maps to no more specific discriminant.
pub const THETADATADX_ERR_OTHER: i32 = 1;
/// Authentication failed (credentials rejected during the auth handshake).
pub const THETADATADX_ERR_AUTHENTICATION: i32 = 2;
/// Credentials are malformed or incomplete before any handshake is attempted.
pub const THETADATADX_ERR_INVALID_CREDENTIALS: i32 = 3;
/// The account's subscription does not grant access to the requested data.
pub const THETADATADX_ERR_SUBSCRIPTION: i32 = 4;
/// The server rate-limited the request; see [`thetadatadx_last_error_retry_after_ms`].
pub const THETADATADX_ERR_RATE_LIMIT: i32 = 5;
/// The requested resource (symbol, contract, or endpoint) does not exist.
pub const THETADATADX_ERR_NOT_FOUND: i32 = 6;
/// The call exceeded its deadline before the server responded.
pub const THETADATADX_ERR_DEADLINE_EXCEEDED: i32 = 7;
/// The service is temporarily unavailable (transient server-side fault).
pub const THETADATADX_ERR_UNAVAILABLE: i32 = 8;
/// A transport-layer failure prevented the request from completing.
pub const THETADATADX_ERR_NETWORK: i32 = 9;
/// A decoded payload did not match the expected wire schema.
pub const THETADATADX_ERR_SCHEMA_MISMATCH: i32 = 10;
/// A streaming session failed mid-flight (drop, partial reconnect, or
/// stream-level protocol fault).
pub const THETADATADX_ERR_STREAM: i32 = 11;
/// An environmental configuration fault (config-file I/O, TOML parse, or an
/// internal invariant); distinct from [`THETADATADX_ERR_INVALID_PARAMETER`].
pub const THETADATADX_ERR_CONFIG: i32 = 12;
/// A client-side parameter was rejected by input validation (a bad
/// value, an out-of-range number, a missing required field). Distinct
/// from [`THETADATADX_ERR_CONFIG`], which stays reserved for environmental
/// configuration faults (config-file I/O, TOML parse, internal
/// invariant) so a rejected argument is distinguishable by code from an
/// unrelated configuration failure.
pub const THETADATADX_ERR_INVALID_PARAMETER: i32 = 13;

pub(crate) fn set_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
    LAST_ERROR_CODE.with(|c| c.set(THETADATADX_ERR_OTHER));
    clear_retry_after();
}

/// Set the error string and pin the typed discriminant explicitly. Used by
/// FFI entry points that surface validation failures whose category is known
/// at the call site (e.g. an out-of-range enum int maps to [`THETADATADX_ERR_CONFIG`]
/// rather than the default [`THETADATADX_ERR_OTHER`]).
pub(crate) fn set_error_with_code(msg: &str, code: i32) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
    LAST_ERROR_CODE.with(|c| c.set(code));
    clear_retry_after();
}

/// Set both the formatted error string AND the typed discriminant
/// from a [`thetadatadx::Error`]. The string keeps the previous
/// surface; the code is what the C++ / TypeScript bindings dispatch
/// on to pick the right exception class. A rate-limit back-off hint, if
/// the error carries one, is stashed for [`thetadatadx_last_error_retry_after_ms`].
pub(crate) fn set_error_from(err: &thetadatadx::Error) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(err.to_string()).ok();
    });
    LAST_ERROR_CODE.with(|c| c.set(error_code_for(err)));
    let retry_after_ms = err
        .retry_after()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(THETADATADX_RETRY_AFTER_NONE);
    LAST_ERROR_RETRY_AFTER_MS.with(|c| c.set(retry_after_ms));
}

/// Reset the rate-limit back-off slot to "no hint". Called by every
/// error setter that does not carry a `RetryInfo` detail so a stale hint
/// from a prior rate-limit error is never misattributed to an unrelated
/// failure.
fn clear_retry_after() {
    LAST_ERROR_RETRY_AFTER_MS.with(|c| c.set(THETADATADX_RETRY_AFTER_NONE));
}

/// Map a `thetadatadx::Error` to its typed C ABI discriminant. The
/// mapping mirrors the Python `to_py_err` leaf set so a single Rust
/// variant lands on the same conceptual class across every binding.
pub(crate) fn error_code_for(err: &thetadatadx::Error) -> i32 {
    use thetadatadx::error::{AuthErrorKind, GrpcStatusKind, StreamErrorKind};
    use thetadatadx::Error;
    match err {
        Error::Auth { kind, .. } => match kind {
            AuthErrorKind::InvalidCredentials => THETADATADX_ERR_INVALID_CREDENTIALS,
            AuthErrorKind::NetworkError => THETADATADX_ERR_NETWORK,
            AuthErrorKind::Timeout => THETADATADX_ERR_DEADLINE_EXCEEDED,
            _ => THETADATADX_ERR_AUTHENTICATION,
        },
        Error::Grpc { kind, .. } => match kind {
            GrpcStatusKind::PermissionDenied => THETADATADX_ERR_SUBSCRIPTION,
            GrpcStatusKind::ResourceExhausted => THETADATADX_ERR_RATE_LIMIT,
            GrpcStatusKind::NotFound => THETADATADX_ERR_NOT_FOUND,
            GrpcStatusKind::DeadlineExceeded => THETADATADX_ERR_DEADLINE_EXCEEDED,
            GrpcStatusKind::Unauthenticated => THETADATADX_ERR_AUTHENTICATION,
            GrpcStatusKind::Unavailable => THETADATADX_ERR_UNAVAILABLE,
            _ => THETADATADX_ERR_OTHER,
        },
        Error::NoData => THETADATADX_ERR_NOT_FOUND,
        Error::Timeout { .. } => THETADATADX_ERR_DEADLINE_EXCEEDED,
        Error::Transport { .. } | Error::Tls(_) | Error::Io(_) | Error::Http(_) => {
            THETADATADX_ERR_NETWORK
        }
        Error::Decode { .. } | Error::Decompress { .. } => THETADATADX_ERR_SCHEMA_MISMATCH,
        // Split user-input validation failures out from environmental
        // config faults so the C++ / C ABI surface a dedicated
        // invalid-parameter class while file-I/O / TOML / internal
        // faults stay on the generic config code.
        Error::Config { kind, .. } => {
            if kind.is_invalid_parameter() {
                THETADATADX_ERR_INVALID_PARAMETER
            } else {
                THETADATADX_ERR_CONFIG
            }
        }
        Error::Stream { kind, .. } => match kind {
            StreamErrorKind::TooManyRequests => THETADATADX_ERR_RATE_LIMIT,
            StreamErrorKind::Timeout => THETADATADX_ERR_DEADLINE_EXCEEDED,
            StreamErrorKind::ConnectionRefused | StreamErrorKind::Disconnected => {
                THETADATADX_ERR_NETWORK
            }
            _ => THETADATADX_ERR_STREAM,
        },
        Error::FlatFilesUnavailable(_) | Error::PartialReconnect { .. } => THETADATADX_ERR_STREAM,
        _ => THETADATADX_ERR_OTHER,
    }
}

/// Retrieve the typed discriminant of the last FFI error on this
/// thread. Returns [`THETADATADX_ERR_NONE`] when no error is set or after
/// [`thetadatadx_clear_error`].
///
/// Callers should pair this with [`thetadatadx_last_error`] for the
/// human-readable message — the code routes to the right exception
/// class, the string carries the diagnostic.
#[no_mangle]
pub extern "C" fn thetadatadx_last_error_code() -> i32 {
    ffi_boundary!(THETADATADX_ERR_OTHER, {
        LAST_ERROR_CODE.with(std::cell::Cell::get)
    })
}

/// Retrieve the server-supplied rate-limit back-off of the last FFI
/// error on this thread, in milliseconds, or [`THETADATADX_RETRY_AFTER_NONE`]
/// (`-1`) when the error carries no `RetryInfo` hint.
///
/// Set only for a rate-limit error whose upstream status attached a
/// `google.rpc.RetryInfo` detail; every other error (and a rate-limit
/// error without the detail) reads `-1`. The C++ `RateLimitError`
/// exposes this as a typed `retry_after()` value so a caller can honour
/// the back-off without parsing the message text.
#[no_mangle]
pub extern "C" fn thetadatadx_last_error_retry_after_ms() -> i64 {
    ffi_boundary!(THETADATADX_RETRY_AFTER_NONE, {
        LAST_ERROR_RETRY_AFTER_MS.with(std::cell::Cell::get)
    })
}

/// Retrieve the last error message (or null if no error).
///
/// The returned pointer is valid until the next FFI call on the same thread.
/// Do NOT free this pointer.
#[no_mangle]
pub extern "C" fn thetadatadx_last_error() -> *const c_char {
    ffi_boundary!(ptr::null(), {
        LAST_ERROR.with(|e| {
            let borrow = e.borrow();
            match borrow.as_ref() {
                Some(s) => s.as_ptr(),
                None => ptr::null(),
            }
        })
    })
}

/// Clear the thread-local error string.
///
/// Wrappers in higher-level languages (C++, Python) should call this
/// before issuing an FFI call so they can distinguish "the call set a new
/// error" from "the previous call left a stale error in the slot". Critical
/// for endpoints that return an empty value sentinel on both success
/// (no rows) and failure (e.g. timeout) — without clearing first, the
/// caller can't tell the two apart from the array alone.
#[no_mangle]
pub extern "C" fn thetadatadx_clear_error() {
    ffi_boundary!((), {
        LAST_ERROR.with(|e| {
            *e.borrow_mut() = None;
        });
        LAST_ERROR_CODE.with(|c| c.set(THETADATADX_ERR_NONE));
        LAST_ERROR_RETRY_AFTER_MS.with(|c| c.set(THETADATADX_RETRY_AFTER_NONE));
    })
}

/// Decode a possibly-null C string pointer.
///
/// - `p.is_null()` → `Ok(None)` (caller chose not to pass this argument).
/// - Non-null with valid UTF-8 → `Ok(Some(&str))`.
/// - Non-null with invalid UTF-8 → `Err(Utf8Error)`.
///
/// Callers must distinguish these cases: a null pointer is usually a
/// legal "omit this optional arg" sentinel, while invalid UTF-8 is a bug
/// in the caller that should be surfaced through `thetadatadx_last_error`.
pub(crate) unsafe fn cstr_to_str<'a>(
    p: *const c_char,
) -> Result<Option<&'a str>, std::str::Utf8Error> {
    if p.is_null() {
        return Ok(None);
    }
    // SAFETY: caller supplies a NUL-terminated C string valid for the call duration.
    unsafe { CStr::from_ptr(p) }.to_str().map(Some)
}

/// Extract a required C string arg. On failure, calls `set_error` with a
/// message that distinguishes null-pointer vs invalid-UTF-8 and returns
/// the given fallback value from the enclosing function.
macro_rules! require_cstr {
    ($p:ident, $fallback:expr) => {
        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.
        match unsafe { $crate::error::cstr_to_str($p) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                $crate::error::set_error(concat!(stringify!($p), " is null"));
                return $fallback;
            }
            Err(e) => {
                $crate::error::set_error(&format!("{} is not valid UTF-8: {e}", stringify!($p)));
                return $fallback;
            }
        }
    };
}

/// Dereference an opaque `*const ThetaDataDxHistoricalClient` (or equivalent) handle into a
/// `&` reference, returning the supplied fallback after setting
/// `thetadatadx_last_error` on null. Hides the boilerplate `if is_null() { ... };
/// unsafe { &*client }` pattern that the FFI endpoint codegen emits at
/// every call site.
macro_rules! require_client {
    ($client:ident, $fallback:expr) => {{
        if $client.is_null() {
            $crate::error::set_error(concat!(stringify!($client), " handle is null"));
            return $fallback;
        }
        // SAFETY: caller passes a pointer returned by `thetadatadx_historical_connect` that has not been freed by `thetadatadx_historical_free`; null was rejected above; `&*` produces a shared reference valid for the call duration because the caller owns the Box.
        unsafe { &*$client }
    }};
}

/// Decode a C array of C string pointers `(symbols, symbols_len)` into
/// `Vec<String>`, returning the supplied fallback after setting
/// `thetadatadx_last_error` on null/UTF-8 failure. Wraps `parse_symbol_array`
/// so endpoint shims do not repeat the inline null/UTF-8 branch.
macro_rules! require_symbol_array {
    ($symbols:ident, $symbols_len:ident, $fallback:expr) => {
        // SAFETY: caller passes a contiguous array of `symbols_len` non-null NUL-terminated C strings kept valid for the call duration; `parse_symbol_array` validates each element and surfaces errors via `thetadatadx_last_error`.
        match unsafe { $crate::types::parse_symbol_array($symbols, $symbols_len) } {
            Some(values) => values,
            None => return $fallback,
        }
    };
}

/// Dereference an opaque `*mut ThetaDataDxConfig` handle into a `&mut`
/// reference. Null is treated as a no-op: every `thetadatadx_config_set_*`
/// shim silently returns without setting an error when the caller
/// passes null (matches the per-call-site behaviour the macro
/// replaces). Use [`require_client!`] when null must produce an `Err`
/// instead.
///
/// Centralises the `if is_null() { return; }; unsafe { &mut *config }`
/// pattern across config setter entrypoints. The SAFETY block names
/// the actual invariant (pointer returned by `thetadatadx_*_new`, not yet
/// freed) once, instead of paraphrasing it inline at every setter.
macro_rules! require_config_mut {
    ($config:ident) => {{
        if $config.is_null() {
            return;
        }
        // SAFETY: caller passes a pointer returned by `thetadatadx_direct_config_new` that has not been freed; null was rejected above; `&mut *` produces a unique reference valid for the call duration because the caller owns the Box and the FFI contract forbids concurrent calls on the same handle.
        unsafe { &mut *$config }
    }};
}

#[cfg(test)]
mod tests {
    //! Unit tests for the typed error-code surface introduced by the
    //! C++ / TS exception-class refactor. Pins the mapping so a future
    //! Rust-side `Error` variant addition fails the test rather than
    //! silently routing to `THETADATADX_ERR_OTHER`.

    use super::*;
    use thetadatadx::error::{AuthErrorKind, GrpcStatusKind, StreamErrorKind};

    fn grpc(kind: GrpcStatusKind) -> thetadatadx::Error {
        thetadatadx::Error::Grpc {
            kind,
            message: String::new(),
            retry_after: None,
        }
    }

    fn auth(kind: AuthErrorKind) -> thetadatadx::Error {
        thetadatadx::Error::Auth {
            kind,
            message: String::new(),
        }
    }

    fn fpss(kind: StreamErrorKind) -> thetadatadx::Error {
        thetadatadx::Error::Stream {
            kind,
            message: String::new(),
        }
    }

    #[test]
    fn grpc_kinds_route_to_expected_codes() {
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::PermissionDenied)),
            THETADATADX_ERR_SUBSCRIPTION
        );
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::ResourceExhausted)),
            THETADATADX_ERR_RATE_LIMIT
        );
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::NotFound)),
            THETADATADX_ERR_NOT_FOUND
        );
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::DeadlineExceeded)),
            THETADATADX_ERR_DEADLINE_EXCEEDED
        );
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::Unauthenticated)),
            THETADATADX_ERR_AUTHENTICATION
        );
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::Unavailable)),
            THETADATADX_ERR_UNAVAILABLE
        );
    }

    #[test]
    fn auth_kinds_route_to_expected_codes() {
        assert_eq!(
            error_code_for(&auth(AuthErrorKind::InvalidCredentials)),
            THETADATADX_ERR_INVALID_CREDENTIALS
        );
        assert_eq!(
            error_code_for(&auth(AuthErrorKind::NetworkError)),
            THETADATADX_ERR_NETWORK
        );
        assert_eq!(
            error_code_for(&auth(AuthErrorKind::Timeout)),
            THETADATADX_ERR_DEADLINE_EXCEEDED
        );
        assert_eq!(
            error_code_for(&auth(AuthErrorKind::ServerError)),
            THETADATADX_ERR_AUTHENTICATION
        );
    }

    #[test]
    fn umbrella_variants_route_to_expected_codes() {
        assert_eq!(
            error_code_for(&thetadatadx::Error::NoData),
            THETADATADX_ERR_NOT_FOUND
        );
        assert_eq!(
            error_code_for(&thetadatadx::Error::Timeout { duration_ms: 500 }),
            THETADATADX_ERR_DEADLINE_EXCEEDED
        );
        assert_eq!(
            error_code_for(&thetadatadx::Error::Transport {
                kind: thetadatadx::error::TransportErrorKind::ConnectionClosed,
                message: "dead".into(),
            }),
            THETADATADX_ERR_NETWORK
        );
        assert_eq!(
            error_code_for(&thetadatadx::Error::decode_codec("cell type mismatch")),
            THETADATADX_ERR_SCHEMA_MISMATCH
        );
    }

    #[test]
    fn config_validation_routes_to_invalid_parameter_others_to_config() {
        // User-input validation failures get the dedicated discriminant.
        assert_eq!(
            error_code_for(&thetadatadx::Error::config_invalid("ffi", "bad")),
            THETADATADX_ERR_INVALID_PARAMETER
        );
        assert_eq!(
            error_code_for(&thetadatadx::Error::config_out_of_range("ffi", 0, 1, 9)),
            THETADATADX_ERR_INVALID_PARAMETER
        );
        assert_eq!(
            error_code_for(&thetadatadx::Error::config_missing("ffi")),
            THETADATADX_ERR_INVALID_PARAMETER
        );
        // The flat-file dataset gate rejects an unserved (security,
        // request) pair with a `config_invalid` error, so the C-ABI
        // surfaces it as the dedicated invalid-parameter discriminant —
        // the same code C++ maps onto its invalid-argument exception.
        assert_eq!(
            error_code_for(&thetadatadx::Error::config_invalid(
                "flatfiles.dataset",
                "flat-file service does not serve stock open_interest"
            )),
            THETADATADX_ERR_INVALID_PARAMETER
        );
        // Environmental config faults stay on the generic config code.
        assert_eq!(
            error_code_for(&thetadatadx::Error::config_io("file not found")),
            THETADATADX_ERR_CONFIG
        );
        assert_eq!(
            error_code_for(&thetadatadx::Error::config_toml("expected `]`")),
            THETADATADX_ERR_CONFIG
        );
    }

    #[test]
    fn set_error_from_stashes_retry_after_hint() {
        // A rate-limit error with a RetryInfo hint populates the
        // retry-after slot in milliseconds; reading it back returns the
        // hint, and clearing resets it to the "no hint" sentinel.
        thetadatadx_clear_error();
        set_error_from(&thetadatadx::Error::Grpc {
            kind: GrpcStatusKind::ResourceExhausted,
            message: "429".into(),
            retry_after: Some(std::time::Duration::from_millis(1500)),
        });
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_RATE_LIMIT);
        assert_eq!(thetadatadx_last_error_retry_after_ms(), 1500);

        // A rate-limit error without a hint reads the sentinel.
        set_error_from(&grpc(GrpcStatusKind::ResourceExhausted));
        assert_eq!(
            thetadatadx_last_error_retry_after_ms(),
            THETADATADX_RETRY_AFTER_NONE
        );

        // A plain `set_error` clears any prior hint so it is never
        // misattributed to an unrelated failure.
        set_error_from(&thetadatadx::Error::Grpc {
            kind: GrpcStatusKind::ResourceExhausted,
            message: "429".into(),
            retry_after: Some(std::time::Duration::from_millis(2000)),
        });
        assert_eq!(thetadatadx_last_error_retry_after_ms(), 2000);
        set_error("unrelated failure");
        assert_eq!(
            thetadatadx_last_error_retry_after_ms(),
            THETADATADX_RETRY_AFTER_NONE
        );

        thetadatadx_clear_error();
        assert_eq!(
            thetadatadx_last_error_retry_after_ms(),
            THETADATADX_RETRY_AFTER_NONE
        );
    }

    #[test]
    fn flatfiles_unavailable_routes_to_stream() {
        assert_eq!(
            error_code_for(&thetadatadx::Error::FlatFilesUnavailable(
                thetadatadx::flatfiles::FlatFilesUnavailableReason::RequestRejected {
                    server_message: "no subscription".into(),
                },
            )),
            THETADATADX_ERR_STREAM
        );
    }

    #[test]
    fn partial_reconnect_routes_to_stream() {
        assert_eq!(
            error_code_for(&thetadatadx::Error::PartialReconnect { failed: Vec::new() }),
            THETADATADX_ERR_STREAM
        );
    }

    #[test]
    fn fpss_kinds_route_to_expected_codes() {
        assert_eq!(
            error_code_for(&fpss(StreamErrorKind::TooManyRequests)),
            THETADATADX_ERR_RATE_LIMIT
        );
        assert_eq!(
            error_code_for(&fpss(StreamErrorKind::Timeout)),
            THETADATADX_ERR_DEADLINE_EXCEEDED
        );
        assert_eq!(
            error_code_for(&fpss(StreamErrorKind::Disconnected)),
            THETADATADX_ERR_NETWORK
        );
        assert_eq!(
            error_code_for(&fpss(StreamErrorKind::ProtocolError)),
            THETADATADX_ERR_STREAM
        );
    }

    #[test]
    fn set_error_from_populates_both_slots() {
        // Pin the contract: callers that observe a non-zero
        // `thetadatadx_last_error_code` must also see the matching string.
        set_error_from(&grpc(GrpcStatusKind::PermissionDenied));
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_SUBSCRIPTION);
        assert!(!thetadatadx_last_error().is_null());
        thetadatadx_clear_error();
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_NONE);
        assert!(thetadatadx_last_error().is_null());
    }

    #[test]
    fn set_error_defaults_to_other_code() {
        // Plain `set_error` is used by sites that surface a
        // non-thetadatadx error (e.g. UTF-8 parse failures) — the
        // discriminator must default to `THETADATADX_ERR_OTHER` so the C++
        // dispatcher routes to the umbrella `ThetaDataError` class.
        thetadatadx_clear_error();
        set_error("plain error");
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_OTHER);
        thetadatadx_clear_error();
    }

    // ───────── macro coverage ─────────────────────────────────────────
    //
    // `require_cstr!`, `require_client!`, and `require_symbol_array!`
    // are the three macros emitted at every FFI endpoint shim. The
    // tests below drive the macros themselves (not a re-implementation
    // of the underlying path) and verify the observable contract: the
    // thread-local error slot gets populated on failure, the fallback
    // value is returned, and the success path does not perturb the
    // slot.

    /// Read the current value of the thread-local `LAST_ERROR` slot
    /// through the C ABI surface (`thetadatadx_last_error()`) and return the
    /// owned UTF-8 string, or `None` if the slot is empty.
    ///
    /// SAFETY: `thetadatadx_last_error()` returns a pointer into the thread-
    /// local `LAST_ERROR` slot's `CString`. The slot lives until the
    /// next `set_error` / `thetadatadx_clear_error` call on this thread, and
    /// the test scope above clears the slot before each call to keep
    /// this lifetime invariant true at every call site. `CStr::from_ptr`
    /// also requires NUL-termination — guaranteed by `CString`'s
    /// representation. The function only runs in test threads that do
    /// not race the FFI surface, so no concurrent mutator invalidates
    /// the pointer between read and copy.
    fn last_error_message() -> Option<String> {
        let p = thetadatadx_last_error();
        if p.is_null() {
            return None;
        }
        // SAFETY: see the function-level SAFETY block — the pointer is
        // non-null per the guard above and refers to the thread-local
        // slot's NUL-terminated `CString`.
        let cstr = unsafe { std::ffi::CStr::from_ptr(p) };
        Some(cstr.to_str().expect("LAST_ERROR slot is UTF-8").to_owned())
    }

    use std::ffi::{c_char, CString};
    use std::ptr;

    // Helper that returns the macro's `&str` result (or the supplied
    // fallback) so the test body can inspect both branches. Returning
    // `&'static str` keeps the helper signature simple — the macro
    // itself produces a `&str` borrowed from the input C string, so
    // for the success path the test holds the `CString` alive across
    // the call.
    fn run_require_cstr(p: *const c_char, fallback: &str) -> &str {
        let s_ptr = p;
        require_cstr!(s_ptr, fallback)
    }

    #[test]
    fn require_cstr_null_returns_fallback_and_sets_error() {
        thetadatadx_clear_error();
        let result = run_require_cstr(ptr::null(), "fallback");
        assert_eq!(result, "fallback");
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_OTHER);
        let msg = last_error_message().expect("error slot populated");
        assert!(msg.contains("is null"), "expected null mention, got {msg}");
        thetadatadx_clear_error();
    }

    #[test]
    fn require_cstr_valid_utf8_returns_str() {
        thetadatadx_clear_error();
        let cstr = CString::new("payload").unwrap();
        let result = run_require_cstr(cstr.as_ptr(), "fallback");
        assert_eq!(result, "payload");
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_NONE);
        assert!(thetadatadx_last_error().is_null());
    }

    #[test]
    fn require_cstr_invalid_utf8_returns_fallback_and_sets_error() {
        thetadatadx_clear_error();
        // 0xFF is not a valid UTF-8 leading byte; the second 0x00 is
        // the NUL terminator so the buffer is a well-formed C string
        // but a malformed Rust `&str`.
        let bytes: [u8; 2] = [0xFF, 0x00];
        let p = bytes.as_ptr().cast::<c_char>();
        let result = run_require_cstr(p, "fallback");
        assert_eq!(result, "fallback");
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_OTHER);
        let msg = last_error_message().expect("error slot populated");
        assert!(msg.contains("UTF-8"), "expected UTF-8 mention, got {msg}");
        thetadatadx_clear_error();
    }

    #[test]
    fn require_cstr_empty_returns_empty_str_no_error() {
        thetadatadx_clear_error();
        let bytes: [u8; 1] = [0x00];
        let p = bytes.as_ptr().cast::<c_char>();
        let result = run_require_cstr(p, "fallback");
        assert_eq!(result, "");
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_NONE);
        assert!(thetadatadx_last_error().is_null());
    }

    // ─────────────────────────────────────────────────────────────────
    // `require_client!` macro coverage. Uses a synthetic non-opaque
    // pointee so the test does not need a real `ThetaDataDxHistoricalClient` handle (the
    // macro only reads the pointer null-ness and dereferences for
    // borrowing).
    fn run_require_client<T>(p: *const T, fallback: i32) -> i32 {
        let client = p;
        let _ref = require_client!(client, fallback);
        // Return a sentinel distinct from `fallback` so the success
        // branch is observable.
        0
    }

    #[test]
    fn require_client_null_returns_fallback_and_sets_error() {
        thetadatadx_clear_error();
        let result = run_require_client::<u32>(ptr::null(), -1);
        assert_eq!(result, -1);
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_OTHER);
        let msg = last_error_message().expect("error slot populated");
        assert!(
            msg.contains("handle is null"),
            "expected handle-is-null mention, got {msg}"
        );
        thetadatadx_clear_error();
    }

    #[test]
    fn require_client_valid_returns_ref() {
        thetadatadx_clear_error();
        let storage: u32 = 0x00C0_FFEE_u32;
        let result = run_require_client(&storage as *const u32, -1);
        assert_eq!(result, 0);
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_NONE);
        assert!(thetadatadx_last_error().is_null());
    }

    // ─────────────────────────────────────────────────────────────────
    // `require_symbol_array!` macro coverage. Drives the macro with
    // the three input shapes endpoint shims see in the wild.
    fn run_require_symbol_array(
        symbols: *const *const c_char,
        symbols_len: usize,
    ) -> Result<Vec<String>, ()> {
        let symbols_arg = symbols;
        let symbols_len_arg = symbols_len;
        Ok(require_symbol_array!(symbols_arg, symbols_len_arg, Err(())))
    }

    #[test]
    fn require_symbol_array_null_pointer_zero_len_ok() {
        thetadatadx_clear_error();
        let out =
            run_require_symbol_array(ptr::null(), 0).expect("(null, 0) is the empty-slice case");
        assert!(out.is_empty());
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_NONE);
    }

    #[test]
    fn require_symbol_array_null_pointer_nonzero_len_returns_fallback_and_sets_error() {
        thetadatadx_clear_error();
        let result = run_require_symbol_array(ptr::null(), 3);
        assert!(result.is_err());
        let msg = last_error_message().expect("error slot populated");
        assert!(
            msg.contains("symbols array pointer is null"),
            "expected null-array mention, got {msg}"
        );
        thetadatadx_clear_error();
    }

    #[test]
    fn require_symbol_array_valid_returns_strings() {
        thetadatadx_clear_error();
        let a = CString::new("AAPL").unwrap();
        let b = CString::new("MSFT").unwrap();
        let arr: [*const c_char; 2] = [a.as_ptr(), b.as_ptr()];
        let out = run_require_symbol_array(arr.as_ptr(), arr.len()).expect("valid input");
        assert_eq!(out, vec!["AAPL".to_owned(), "MSFT".to_owned()]);
        assert_eq!(thetadatadx_last_error_code(), THETADATADX_ERR_NONE);
    }
}
