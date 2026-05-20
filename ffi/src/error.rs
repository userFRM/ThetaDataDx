//! Thread-local error slot plus the `tdx_last_error` / `tdx_clear_error`
//! FFI accessors and the `require_cstr!` macro used by endpoint wrappers.
//!
//! Contract: the error slot is scoped to the OS thread that set it. C++
//! and Python never migrate threads implicitly, so no pinning is needed
//! there. Any third-party FFI consumer whose runtime can migrate a logical
//! execution unit across OS threads (e.g. Go's goroutines, which can park
//! on one thread and resume on another) MUST pin the execution unit for
//! the duration of a clear/call/check sequence — typically via the host
//! runtime's equivalent of `runtime.LockOSThread` + deferred unlock.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
    static LAST_ERROR_CODE: std::cell::Cell<i32> = const { std::cell::Cell::new(TDX_ERR_NONE) };
}

/// Typed error-code discriminants surfaced via [`tdx_last_error_code`].
///
/// Higher-level bindings (the C++ exception hierarchy in
/// `sdks/cpp/include/thetadx.hpp`, the typed napi error subclasses in
/// `sdks/typescript/src/lib.rs`) use these codes to choose which
/// concrete exception / error subclass to throw without having to
/// substring-match the formatted error string. The mapping mirrors
/// the Python `to_py_err` hierarchy one-for-one so the leaf set stays
/// uniform across bindings.
pub const TDX_ERR_NONE: i32 = 0;
pub const TDX_ERR_OTHER: i32 = 1;
pub const TDX_ERR_AUTHENTICATION: i32 = 2;
pub const TDX_ERR_INVALID_CREDENTIALS: i32 = 3;
pub const TDX_ERR_SUBSCRIPTION: i32 = 4;
pub const TDX_ERR_RATE_LIMIT: i32 = 5;
pub const TDX_ERR_NOT_FOUND: i32 = 6;
pub const TDX_ERR_DEADLINE_EXCEEDED: i32 = 7;
pub const TDX_ERR_UNAVAILABLE: i32 = 8;
pub const TDX_ERR_NETWORK: i32 = 9;
pub const TDX_ERR_SCHEMA_MISMATCH: i32 = 10;
pub const TDX_ERR_STREAM: i32 = 11;
pub const TDX_ERR_CONFIG: i32 = 12;

pub(crate) fn set_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
    LAST_ERROR_CODE.with(|c| c.set(TDX_ERR_OTHER));
}

/// Set both the formatted error string AND the typed discriminant
/// from a [`thetadatadx::Error`]. The string keeps the previous
/// surface; the code is what the C++ / TypeScript bindings dispatch
/// on to pick the right exception class.
pub(crate) fn set_error_from(err: &thetadatadx::Error) {
    set_error_string_only(&err.to_string());
    LAST_ERROR_CODE.with(|c| c.set(error_code_for(err)));
}

/// Set the error string without touching the typed code. Used by the
/// `set_error_from` helper above (which sets the code separately) and
/// by call sites that surface a non-thetadatadx error (e.g. parse
/// errors raised inside the FFI before any RPC fires).
fn set_error_string_only(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

/// Map a `thetadatadx::Error` to its typed C ABI discriminant. The
/// mapping mirrors the Python `to_py_err` leaf set so a single Rust
/// variant lands on the same conceptual class across every binding.
pub(crate) fn error_code_for(err: &thetadatadx::Error) -> i32 {
    use thetadatadx::error::{AuthErrorKind, FpssErrorKind, GrpcStatusKind};
    use thetadatadx::Error;
    match err {
        Error::Auth { kind, .. } => match kind {
            AuthErrorKind::InvalidCredentials => TDX_ERR_INVALID_CREDENTIALS,
            AuthErrorKind::NetworkError => TDX_ERR_NETWORK,
            AuthErrorKind::Timeout => TDX_ERR_DEADLINE_EXCEEDED,
            _ => TDX_ERR_AUTHENTICATION,
        },
        Error::Grpc { kind, .. } => match kind {
            GrpcStatusKind::PermissionDenied => TDX_ERR_SUBSCRIPTION,
            GrpcStatusKind::ResourceExhausted => TDX_ERR_RATE_LIMIT,
            GrpcStatusKind::NotFound => TDX_ERR_NOT_FOUND,
            GrpcStatusKind::DeadlineExceeded => TDX_ERR_DEADLINE_EXCEEDED,
            GrpcStatusKind::Unauthenticated => TDX_ERR_AUTHENTICATION,
            GrpcStatusKind::Unavailable => TDX_ERR_UNAVAILABLE,
            _ => TDX_ERR_OTHER,
        },
        Error::NoData => TDX_ERR_NOT_FOUND,
        Error::Timeout { .. } => TDX_ERR_DEADLINE_EXCEEDED,
        Error::Transport { .. } | Error::Tls(_) | Error::Io(_) | Error::Http(_) => TDX_ERR_NETWORK,
        Error::Decode { .. } | Error::Decompress { .. } => TDX_ERR_SCHEMA_MISMATCH,
        Error::Config { .. } => TDX_ERR_CONFIG,
        Error::Fpss { kind, .. } => match kind {
            FpssErrorKind::TooManyRequests => TDX_ERR_RATE_LIMIT,
            FpssErrorKind::Timeout => TDX_ERR_DEADLINE_EXCEEDED,
            FpssErrorKind::ConnectionRefused | FpssErrorKind::Disconnected => TDX_ERR_NETWORK,
            _ => TDX_ERR_STREAM,
        },
        _ => TDX_ERR_OTHER,
    }
}

/// Retrieve the typed discriminant of the last FFI error on this
/// thread. Returns [`TDX_ERR_NONE`] when no error is set or after
/// [`tdx_clear_error`].
///
/// Callers should pair this with [`tdx_last_error`] for the
/// human-readable message — the code routes to the right exception
/// class, the string carries the diagnostic.
#[no_mangle]
pub extern "C" fn tdx_last_error_code() -> i32 {
    ffi_boundary!(TDX_ERR_OTHER, {
        LAST_ERROR_CODE.with(std::cell::Cell::get)
    })
}

/// Retrieve the last error message (or null if no error).
///
/// The returned pointer is valid until the next FFI call on the same thread.
/// Do NOT free this pointer.
#[no_mangle]
pub extern "C" fn tdx_last_error() -> *const c_char {
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
/// Wrappers in higher-level languages (Go, C++, Python) should call this
/// before issuing an FFI call so they can distinguish "the call set a new
/// error" from "the previous call left a stale error in the slot". Critical
/// for endpoints that return an empty value sentinel on both success
/// (no rows) and failure (e.g. timeout) — without clearing first, the
/// caller can't tell the two apart from the array alone.
#[no_mangle]
pub extern "C" fn tdx_clear_error() {
    ffi_boundary!((), {
        LAST_ERROR.with(|e| {
            *e.borrow_mut() = None;
        });
        LAST_ERROR_CODE.with(|c| c.set(TDX_ERR_NONE));
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
/// in the caller that should be surfaced through `tdx_last_error`.
pub(crate) unsafe fn cstr_to_str<'a>(
    p: *const c_char,
) -> Result<Option<&'a str>, std::str::Utf8Error> {
    if p.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(p) }.to_str().map(Some)
}

/// Extract a required C string arg. On failure, calls `set_error` with a
/// message that distinguishes null-pointer vs invalid-UTF-8 and returns
/// the given fallback value from the enclosing function.
macro_rules! require_cstr {
    ($p:ident, $fallback:expr) => {
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

#[cfg(test)]
mod tests {
    //! Unit tests for the typed error-code surface introduced by the
    //! C++ / TS exception-class refactor. Pins the mapping so a future
    //! Rust-side `Error` variant addition fails the test rather than
    //! silently routing to `TDX_ERR_OTHER`.

    use super::*;
    use thetadatadx::error::{AuthErrorKind, FpssErrorKind, GrpcStatusKind};

    fn grpc(kind: GrpcStatusKind) -> thetadatadx::Error {
        thetadatadx::Error::Grpc {
            kind,
            message: String::new(),
        }
    }

    fn auth(kind: AuthErrorKind) -> thetadatadx::Error {
        thetadatadx::Error::Auth {
            kind,
            message: String::new(),
        }
    }

    fn fpss(kind: FpssErrorKind) -> thetadatadx::Error {
        thetadatadx::Error::Fpss {
            kind,
            message: String::new(),
        }
    }

    #[test]
    fn grpc_kinds_route_to_expected_codes() {
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::PermissionDenied)),
            TDX_ERR_SUBSCRIPTION
        );
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::ResourceExhausted)),
            TDX_ERR_RATE_LIMIT
        );
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::NotFound)),
            TDX_ERR_NOT_FOUND
        );
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::DeadlineExceeded)),
            TDX_ERR_DEADLINE_EXCEEDED
        );
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::Unauthenticated)),
            TDX_ERR_AUTHENTICATION
        );
        assert_eq!(
            error_code_for(&grpc(GrpcStatusKind::Unavailable)),
            TDX_ERR_UNAVAILABLE
        );
    }

    #[test]
    fn auth_kinds_route_to_expected_codes() {
        assert_eq!(
            error_code_for(&auth(AuthErrorKind::InvalidCredentials)),
            TDX_ERR_INVALID_CREDENTIALS
        );
        assert_eq!(
            error_code_for(&auth(AuthErrorKind::NetworkError)),
            TDX_ERR_NETWORK
        );
        assert_eq!(
            error_code_for(&auth(AuthErrorKind::Timeout)),
            TDX_ERR_DEADLINE_EXCEEDED
        );
        assert_eq!(
            error_code_for(&auth(AuthErrorKind::ServerError)),
            TDX_ERR_AUTHENTICATION
        );
    }

    #[test]
    fn umbrella_variants_route_to_expected_codes() {
        assert_eq!(
            error_code_for(&thetadatadx::Error::NoData),
            TDX_ERR_NOT_FOUND
        );
        assert_eq!(
            error_code_for(&thetadatadx::Error::Timeout { duration_ms: 500 }),
            TDX_ERR_DEADLINE_EXCEEDED
        );
        assert_eq!(
            error_code_for(&thetadatadx::Error::Transport {
                kind: thetadatadx::error::TransportErrorKind::ConnectionClosed,
                message: "dead".into(),
            }),
            TDX_ERR_NETWORK
        );
        assert_eq!(
            error_code_for(&thetadatadx::Error::decode_codec("cell type mismatch")),
            TDX_ERR_SCHEMA_MISMATCH
        );
        assert_eq!(
            error_code_for(&thetadatadx::Error::config_other("bad")),
            TDX_ERR_CONFIG
        );
    }

    #[test]
    fn fpss_kinds_route_to_expected_codes() {
        assert_eq!(
            error_code_for(&fpss(FpssErrorKind::TooManyRequests)),
            TDX_ERR_RATE_LIMIT
        );
        assert_eq!(
            error_code_for(&fpss(FpssErrorKind::Timeout)),
            TDX_ERR_DEADLINE_EXCEEDED
        );
        assert_eq!(
            error_code_for(&fpss(FpssErrorKind::Disconnected)),
            TDX_ERR_NETWORK
        );
        assert_eq!(
            error_code_for(&fpss(FpssErrorKind::ProtocolError)),
            TDX_ERR_STREAM
        );
    }

    #[test]
    fn set_error_from_populates_both_slots() {
        // Pin the contract: callers that observe a non-zero
        // `tdx_last_error_code` must also see the matching string.
        set_error_from(&grpc(GrpcStatusKind::PermissionDenied));
        assert_eq!(tdx_last_error_code(), TDX_ERR_SUBSCRIPTION);
        assert!(!tdx_last_error().is_null());
        tdx_clear_error();
        assert_eq!(tdx_last_error_code(), TDX_ERR_NONE);
        assert!(tdx_last_error().is_null());
    }

    #[test]
    fn set_error_string_only_defaults_to_other_code() {
        // Plain `set_error` is used by sites that surface a
        // non-thetadatadx error (e.g. UTF-8 parse failures) — the
        // discriminator must default to `TDX_ERR_OTHER` so the C++
        // dispatcher routes to the umbrella `ThetaDataError` class.
        tdx_clear_error();
        set_error("plain error");
        assert_eq!(tdx_last_error_code(), TDX_ERR_OTHER);
        tdx_clear_error();
    }
}
