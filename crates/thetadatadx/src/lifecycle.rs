//! Dispatcher thread lifecycle type shared across the workspace.
//!
//! Defining the enum once here eliminates three near-identical copies:
//! - `crates/thetadatadx/src/client.rs` (`DispatcherSession`)
//! - `ffi/src/streaming.rs` (`FfpssDispatcherSession`)
//! - `sdks/python/src/fpss_client.rs` (`PyFpssDispatcherSession`)
//!
//! All three had identical shape: `Idle / Running { handle } / Failed { reason }`.
//! A single canonical type removes the drift risk.

/// Dispatcher thread lifecycle.
///
/// A single `Mutex<DispatcherSession>` covers the single-flight
/// serialisation, the `JoinHandle`, and the failure payload.
/// Collapsed from three separate primitives in earlier revisions:
/// `start_lock: Mutex<()>`, `dispatcher_handle: Mutex<Option<JoinHandle<()>>>`,
/// and `dispatcher_failed: Arc<AtomicBool>`. Dispatcher panic state is
/// carried by the `Failed` variant's payload — derived from
/// `JoinHandle::join()` returning `Err(_)`, no separate atomic needed.
#[doc(hidden)]
pub enum DispatcherSession {
    /// No dispatcher is running; `start_streaming` has not been called or
    /// has been cleanly stopped.
    Idle,
    /// Dispatcher thread is live. `JoinHandle` is required so
    /// `stop_streaming` can join it and observe a clean exit or a panic
    /// payload.
    Running { handle: std::thread::JoinHandle<()> },
    /// Dispatcher terminated via an uncaught panic in the event-iteration
    /// machinery (NOT a user-callback panic — those are caught by
    /// per-invocation `catch_unwind` inside `poll_batch`). The payload
    /// is the downcasted `Box<dyn Any>` message, or a fixed string when
    /// the payload type is not `&str` / `String`.
    Failed { reason: String },
}
