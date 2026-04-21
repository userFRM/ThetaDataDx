//! `ffi_boundary!` macro — wraps every `extern "C"` body in `catch_unwind`.
//!
//! Rust 1.81+ aborts when a panic crosses an `extern "C"` boundary; pre-1.81
//! the behavior is undefined. Both modes crash the host process, which is
//! unacceptable for a library binding (a typo in a macro arg or an unexpected
//! invariant violation inside `tokio::runtime::block_on` would take down the
//! user's entire program). Wrapping the body in `catch_unwind` keeps the
//! crash contained in the thread and surfaces the reason through the normal
//! FFI error channel.

/// Wrap an `extern "C"` fn body. Catches panics that would otherwise
/// abort the host process (C / Go / Python) and converts them into a
/// well-defined error return plus a thread-local `last_error` entry.
///
/// The wrapped block must return `T`. On panic, `default` is returned and
/// an error string describing the panic payload (if extractable) is set
/// via `set_error(...)`.
///
/// Rust 1.81+ aborts when a panic crosses an `extern "C"` boundary;
/// pre-1.81 the behavior is undefined. Both modes crash the host process,
/// which is unacceptable for a library binding (a typo in a macro arg or
/// an unexpected invariant violation inside `tokio::runtime::block_on`
/// would take down the user's entire program). Wrapping the body in
/// `catch_unwind` keeps the crash contained in the thread and surfaces
/// the reason through the normal FFI error channel.
macro_rules! ffi_boundary {
    ($default:expr, $body:block) => {{
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $body)) {
            Ok(v) => v,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&'static str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("<non-string panic>");
                tracing::error!(
                    target: "thetadatadx::ffi::panic",
                    msg,
                    "FFI boundary caught panic",
                );
                $crate::error::set_error(&format!("panic at FFI boundary: {msg}"));
                $default
            }
        }
    }};
}
