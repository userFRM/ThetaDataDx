//! Thread-local error slot plus the `tdx_last_error` / `tdx_clear_error`
//! FFI accessors and the `require_cstr!` macro used by endpoint wrappers.
//!
//! Contract: the error slot is scoped to the OS thread that set it. Higher-
//! level languages whose runtime can migrate a logical execution unit
//! across OS threads (notably Go, where a goroutine can park on one thread
//! and resume on another) MUST pin the execution unit for the duration of
//! a clear/call/check sequence. The generated Go wrappers do this via
//! `runtime.LockOSThread` + deferred unlock (see
//! `crates/thetadatadx/build_support/endpoints/render/go.rs` —
//! `render_go_endpoint_method`). C++ and Python never migrate threads
//! implicitly, so no pinning is needed there.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn set_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
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
