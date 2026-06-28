//! Dispatcher thread lifecycle type shared across the workspace.
//!
//! Defining the enum once here eliminates three near-identical copies:
//! - `thetadatadx-rs/src/client.rs` (`DispatcherSession`)
//! - `thetadatadx-ffi/src/streaming.rs` (`FfpssDispatcherSession`)
//! - `thetadatadx-py/src/fpss_client.rs` (`PyFpssDispatcherSession`)
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
    ///
    /// `on_teardown` is an optional one-shot, run while the session is being
    /// retired and just before the join. It lets a teardown that bypasses the
    /// consumer-facing close (a `Client` drop / `stop_streaming` /
    /// `reconnect_streaming`) deliver the same wakeup the close path does, so a
    /// dispatcher parked on its own backpressure primitive is released and the
    /// join cannot hang. Two kinds of dispatcher park off the event ring and so
    /// install a hook: the columnar pull dispatcher parked on its bounded batch
    /// queue, and a binding whose per-event handler hands each event to a
    /// bounded queue and blocks once it is full (the TypeScript
    /// `ThreadsafeFunction` path). A handler that parks only on the event ring
    /// (which the client shutdown already signals) installs `None`. The hook is
    /// fired only as a fallback, after a dispatcher fails to exit on its own
    /// within a short grace window, so a destructive wake (the TypeScript abort,
    /// which permanently closes the function) does not run on a session that
    /// exits cleanly and could be re-used by `reconnect`.
    ///
    /// `registers_drain_flag` records whether this session has a user callback
    /// that [`crate::StreamSurface::await_drain`] must wait for. The per-event
    /// callback sessions set it `true`; the columnar pull session has no
    /// callback and sets it `false` (its `drained` flag stays unset until the
    /// reader handle is dropped, so registering it would make a manual
    /// `await_drain` time out spuriously while a closed-but-not-dropped reader
    /// is alive). It is an explicit field rather than being inferred from
    /// whether `on_teardown` is present, because a callback session can now also
    /// carry a hook. Bindings that run their own teardown (FFI, Python, the
    /// TypeScript standalone client) ignore this field; it is read only by the
    /// unified [`crate::Client`] teardown.
    Running {
        handle: std::thread::JoinHandle<()>,
        on_teardown: Option<Box<dyn FnOnce() + Send>>,
        registers_drain_flag: bool,
    },
    /// Dispatcher terminated via an uncaught panic in the event-iteration
    /// machinery (NOT a user-callback panic — those are caught by
    /// per-invocation `catch_unwind` inside `poll_batch`). The payload
    /// is the downcasted `Box<dyn Any>` message, or a fixed string when
    /// the payload type is not `&str` / `String`.
    Failed { reason: String },
}
