//! Hand-written Python context manager that mirrors the C++ RAII
//! lifecycle for FPSS streaming.
//!
//! `with tdx.streaming(callback) as session:` enters by calling
//! `start_streaming(callback)` and exits by calling `stop_streaming()`
//! followed by `await_drain(5_000)`. The drain barrier matches the
//! C ABI / C++ wrapper contract: by the time control returns to the
//! caller the consumer thread has finished firing the callback, so
//! the closure stack the callback closed over can be released without
//! a use-after-free race against the LMAX Disruptor consumer.
//!
//! SSOT: every public method on `ThetaDataDxClient` is reachable on the
//! `StreamingSession` by virtue of `__getattr__` proxying. There is
//! NO hand-listed mirror of `subscribe_*` / `unsubscribe_*` /
//! `active_subscriptions` here -- adding a new public method to
//! `ThetaDataDxClient` automatically makes it callable through the session,
//! with zero drift between the wrapper and the wrapped surface.

use pyo3::exceptions::PyRuntimeWarning;
use pyo3::prelude::*;

/// Drain timeout applied on `__exit__`. Matches the C++ destructor's
/// 5 s budget in `sdks/cpp/src/thetadx.cpp` and the FFI free-path
/// budget in `ffi/src/streaming.rs::FREE_DRAIN_TIMEOUT`. Cross-binding
/// parity matters more than tunability here -- a slow Python callback
/// that needs >5 s to drain is already a contract violation worth
/// surfacing.
const EXIT_DRAIN_TIMEOUT_MS: u64 = 5_000;

/// Context manager returned by `ThetaDataDxClient.streaming(callback)`.
///
/// Holds a strong reference to the `ThetaDataDxClient` and the user
/// callback. `__enter__` registers the callback via `start_streaming`,
/// `__exit__` calls `stop_streaming` + `await_drain`. Every other
/// method call is forwarded through `__getattr__` to the wrapped
/// `ThetaDataDxClient` instance.
#[pyclass(module = "thetadatadx", name = "StreamingSession")]
pub(crate) struct StreamingSession {
    /// Erased pyclass handle. Carries either a `ThetaDataDxClient` (the
    /// unified entry point) or the standalone `FpssClient` pyclass —
    /// both expose `start_streaming` / `stop_streaming` / `await_drain`
    /// with identical signatures, so the context-manager protocol
    /// dispatches uniformly via PyO3 attribute lookup. Using `Py<PyAny>`
    /// here keeps the proxy SSOT: there is one `StreamingSession`
    /// pyclass for both transports, not two parallel copies that
    /// could drift.
    pub(crate) tdx: Py<PyAny>,
    pub(crate) callback: Option<Py<PyAny>>,
}

#[pymethods]
impl StreamingSession {
    /// Register the stored callback via the public `start_streaming`
    /// method on the wrapped `ThetaDataDxClient`. Returns `self` so users
    /// access subscribe/unsubscribe methods through the session
    /// (which proxies via `__getattr__`).
    fn __enter__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<PyRef<'py, Self>> {
        let callback = slf.callback.as_ref().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err(
                "StreamingSession callback already consumed -- one session enters at most once",
            )
        })?;
        let cb = callback.clone_ref(py);
        let bound = slf.tdx.bind(py);
        bound.call_method1("start_streaming", (cb,))?;
        Ok(slf)
    }

    /// Stop streaming + block on the drain barrier so the consumer
    /// thread is guaranteed to have finished firing the registered
    /// callback before this returns. Returns `False` so the `with`
    /// block does NOT swallow exceptions raised inside the body.
    #[pyo3(signature = (exc_type=None, exc_value=None, traceback=None))]
    fn __exit__(
        &mut self,
        py: Python<'_>,
        exc_type: Option<Py<PyAny>>,
        exc_value: Option<Py<PyAny>>,
        traceback: Option<Py<PyAny>>,
    ) -> PyResult<bool> {
        // The three exception args are part of the context-manager
        // protocol; we don't inspect them but accept them unconditionally
        // so Python's `with` machinery can pass `None` triplets.
        let _ = (exc_type, exc_value, traceback);

        let bound = self.tdx.bind(py);
        bound.call_method0("stop_streaming")?;
        // `await_drain` releases the GIL internally (see
        // `streaming_methods.rs`), so the Disruptor consumer can acquire
        // the GIL to finish firing any in-flight callback before flipping
        // the drain bit. Returns `True` if the drain completed within the
        // timeout, `False` on timeout or on a fresh handle that never
        // streamed.
        let drained_obj = bound.call_method1("await_drain", (EXIT_DRAIN_TIMEOUT_MS,))?;
        let drained: bool = drained_obj.extract()?;
        // Drop the stored callback now that the consumer is quiesced.
        // Holding it longer would leak a Python reference until the
        // session itself is collected.
        self.callback = None;
        if !drained {
            // RuntimeWarning rather than a hard exception: the streaming
            // pipeline is already torn down (`stop_streaming` ran), and
            // the drain is best-effort observability. Re-raising here
            // would swallow any exception from the `with` body, which
            // breaks the standard context-manager contract.
            let warnings = py.import("warnings")?;
            let msg = format!(
                "ThetaDataDxClient streaming drain timed out after {EXIT_DRAIN_TIMEOUT_MS}ms; \
                 consumer callback may still be firing. The Python callback closure \
                 will remain referenced until the consumer exits."
            );
            // `warnings.warn(msg, RuntimeWarning, stacklevel=2)` so the
            // warning point-of-blame is the caller's `with` exit, not
            // this Rust frame.
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("stacklevel", 2_u32)?;
            warnings.call_method(
                "warn",
                (msg, py.get_type::<PyRuntimeWarning>()),
                Some(&kwargs),
            )?;
        }
        // Returning `false` from `__exit__` tells the Python `with`
        // protocol NOT to swallow exceptions raised inside the body.
        Ok(false)
    }

    /// Forward unknown attribute access to the wrapped `ThetaDataDxClient`.
    ///
    /// This is the SSOT proxy: every public method on `ThetaDataDxClient`
    /// (`subscribe(sub)` / `subscribe_many([...])` / `unsubscribe(sub)` /
    /// `unsubscribe_many([...])`, `active_subscriptions`,
    /// `dropped_event_count`, `reconnect`, …) is reachable on the
    /// session without duplication
    /// here. Adding a new method to `ThetaDataDxClient` makes it callable
    /// through the session automatically -- zero drift surface.
    ///
    /// PyO3 calls `__getattr__` only after the C-level attribute lookup
    /// fails, so `__enter__` / `__exit__` / `tdx` / `callback` defined
    /// on this class take precedence and never reach this proxy.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let bound = self.tdx.bind(py);
        Ok(bound.getattr(name)?.unbind())
    }
}

/// Factory method on `ThetaDataDxClient` -- second `#[pymethods]` impl block
/// enabled by the `multiple-pymethods` PyO3 feature flag (see
/// `Cargo.toml`). The generated `streaming_methods.rs` owns the
/// rest of the streaming surface; the context-manager constructor lives
/// here because it is hand-written and references the hand-written
/// `StreamingSession` pyclass.
#[pymethods]
impl crate::ThetaDataDxClient {
    /// Open a context-managed streaming session.
    ///
    /// `with tdx.streaming(callback) as session:` registers `callback`
    /// via `start_streaming` on enter and pairs `stop_streaming()` +
    /// `await_drain(5_000)` on exit, mirroring the C++ RAII destructor
    /// in `sdks/cpp/src/thetadx.cpp`. Subscription methods on the bound
    /// `session` forward to the underlying `ThetaDataDxClient` via
    /// `StreamingSession.__getattr__`, so the public surface stays a
    /// single source of truth rooted in the wrapped class.
    ///
    /// If the drain barrier times out (5000 ms), a `RuntimeWarning`
    /// fires but the `with` block exits normally. A timeout means the
    /// consumer thread is still firing the registered callback; the
    /// callback closure remains referenced by the consumer until it
    /// finishes.
    fn streaming(
        slf: Py<Self>,
        py: Python<'_>,
        callback: Py<PyAny>,
    ) -> PyResult<Py<StreamingSession>> {
        Py::new(
            py,
            StreamingSession {
                tdx: slf.into_any(),
                callback: Some(callback),
            },
        )
    }
}
