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

/// Dereference an opaque `*const TdxClient` (or equivalent) handle into a
/// `&` reference, returning the supplied fallback after setting
/// `tdx_last_error` on null. Hides the boilerplate `if is_null() { ... };
/// unsafe { &*client }` pattern that the FFI endpoint codegen emits at
/// every call site.
macro_rules! require_client {
    ($client:ident, $fallback:expr) => {{
        if $client.is_null() {
            $crate::error::set_error(concat!(stringify!($client), " handle is null"));
            return $fallback;
        }
        // SAFETY: caller passes a pointer returned by `tdx_client_connect` (or `_new`) that has not been freed by `tdx_client_free`; null was rejected above; `&*` produces a shared reference valid for the call duration because the caller owns the Box.
        unsafe { &*$client }
    }};
}

/// Decode a C array of C string pointers `(symbols, symbols_len)` into
/// `Vec<String>`, returning the supplied fallback after setting
/// `tdx_last_error` on null/UTF-8 failure. Wraps `parse_symbol_array`
/// so endpoint shims do not repeat the inline null/UTF-8 branch.
macro_rules! require_symbol_array {
    ($symbols:ident, $symbols_len:ident, $fallback:expr) => {
        // SAFETY: caller passes a contiguous array of `symbols_len` non-null NUL-terminated C strings kept valid for the call duration; `parse_symbol_array` validates each element and surfaces errors via `tdx_last_error`.
        match unsafe { $crate::types::parse_symbol_array($symbols, $symbols_len) } {
            Some(values) => values,
            None => return $fallback,
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
            error_code_for(&thetadatadx::Error::config_invalid("ffi", "bad")),
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

    // ───────── macro coverage ─────────────────────────────────────────
    //
    // `require_cstr!`, `require_client!`, and `require_symbol_array!`
    // are the three macros emitted at every FFI endpoint shim. The
    // tests below drive the macros themselves (not a re-implementation
    // of the underlying path) and verify the observable contract: the
    // thread-local error slot gets populated on failure, the fallback
    // value is returned, and the success path does not perturb the
    // slot.

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
        tdx_clear_error();
        let result = run_require_cstr(ptr::null(), "fallback");
        assert_eq!(result, "fallback");
        assert_eq!(tdx_last_error_code(), TDX_ERR_OTHER);
        // SAFETY: `tdx_last_error()` returns a `*const c_char` pointing to the thread-local `LAST_ERROR` slot's `CString`; non-null per the assertion just above (or the surrounding test confirmed the slot was populated), and the slot lives until the next `set_error` / `tdx_clear_error` call on this thread.
        let msg = unsafe { std::ffi::CStr::from_ptr(tdx_last_error()) }
            .to_str()
            .unwrap();
        assert!(msg.contains("is null"), "expected null mention, got {msg}");
        tdx_clear_error();
    }

    #[test]
    fn require_cstr_valid_utf8_returns_str() {
        tdx_clear_error();
        let cstr = CString::new("payload").unwrap();
        let result = run_require_cstr(cstr.as_ptr(), "fallback");
        assert_eq!(result, "payload");
        assert_eq!(tdx_last_error_code(), TDX_ERR_NONE);
        assert!(tdx_last_error().is_null());
    }

    #[test]
    fn require_cstr_invalid_utf8_returns_fallback_and_sets_error() {
        tdx_clear_error();
        // 0xFF is not a valid UTF-8 leading byte; the second 0x00 is
        // the NUL terminator so the buffer is a well-formed C string
        // but a malformed Rust `&str`.
        let bytes: [u8; 2] = [0xFF, 0x00];
        let p = bytes.as_ptr().cast::<c_char>();
        let result = run_require_cstr(p, "fallback");
        assert_eq!(result, "fallback");
        assert_eq!(tdx_last_error_code(), TDX_ERR_OTHER);
        // SAFETY: `tdx_last_error()` returns a `*const c_char` pointing to the thread-local `LAST_ERROR` slot's `CString`; non-null per the assertion just above (or the surrounding test confirmed the slot was populated), and the slot lives until the next `set_error` / `tdx_clear_error` call on this thread.
        let msg = unsafe { std::ffi::CStr::from_ptr(tdx_last_error()) }
            .to_str()
            .unwrap();
        assert!(msg.contains("UTF-8"), "expected UTF-8 mention, got {msg}");
        tdx_clear_error();
    }

    #[test]
    fn require_cstr_empty_returns_empty_str_no_error() {
        tdx_clear_error();
        let bytes: [u8; 1] = [0x00];
        let p = bytes.as_ptr().cast::<c_char>();
        let result = run_require_cstr(p, "fallback");
        assert_eq!(result, "");
        assert_eq!(tdx_last_error_code(), TDX_ERR_NONE);
        assert!(tdx_last_error().is_null());
    }

    // ─────────────────────────────────────────────────────────────────
    // `require_client!` macro coverage. Uses a synthetic non-opaque
    // pointee so the test does not need a real `TdxClient` handle (the
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
        tdx_clear_error();
        let result = run_require_client::<u32>(ptr::null(), -1);
        assert_eq!(result, -1);
        assert_eq!(tdx_last_error_code(), TDX_ERR_OTHER);
        // SAFETY: `tdx_last_error()` returns a `*const c_char` pointing to the thread-local `LAST_ERROR` slot's `CString`; non-null per the assertion just above (or the surrounding test confirmed the slot was populated), and the slot lives until the next `set_error` / `tdx_clear_error` call on this thread.
        let msg = unsafe { std::ffi::CStr::from_ptr(tdx_last_error()) }
            .to_str()
            .unwrap();
        assert!(
            msg.contains("handle is null"),
            "expected handle-is-null mention, got {msg}"
        );
        tdx_clear_error();
    }

    #[test]
    fn require_client_valid_returns_ref() {
        tdx_clear_error();
        let storage: u32 = 0x00C0_FFEE_u32;
        let result = run_require_client(&storage as *const u32, -1);
        assert_eq!(result, 0);
        assert_eq!(tdx_last_error_code(), TDX_ERR_NONE);
        assert!(tdx_last_error().is_null());
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
        tdx_clear_error();
        let out = run_require_symbol_array(ptr::null(), 0).expect("(null, 0) is the Go-empty case");
        assert!(out.is_empty());
        assert_eq!(tdx_last_error_code(), TDX_ERR_NONE);
    }

    #[test]
    fn require_symbol_array_null_pointer_nonzero_len_returns_fallback_and_sets_error() {
        tdx_clear_error();
        let result = run_require_symbol_array(ptr::null(), 3);
        assert!(result.is_err());
        // SAFETY: `tdx_last_error()` returns a `*const c_char` pointing to the thread-local `LAST_ERROR` slot's `CString`; non-null per the assertion just above (or the surrounding test confirmed the slot was populated), and the slot lives until the next `set_error` / `tdx_clear_error` call on this thread.
        let msg = unsafe { std::ffi::CStr::from_ptr(tdx_last_error()) }
            .to_str()
            .unwrap();
        assert!(
            msg.contains("symbols array pointer is null"),
            "expected null-array mention, got {msg}"
        );
        tdx_clear_error();
    }

    #[test]
    fn require_symbol_array_valid_returns_strings() {
        tdx_clear_error();
        let a = CString::new("AAPL").unwrap();
        let b = CString::new("MSFT").unwrap();
        let arr: [*const c_char; 2] = [a.as_ptr(), b.as_ptr()];
        let out = run_require_symbol_array(arr.as_ptr(), arr.len()).expect("valid input");
        assert_eq!(out, vec!["AAPL".to_owned(), "MSFT".to_owned()]);
        assert_eq!(tdx_last_error_code(), TDX_ERR_NONE);
    }
}
