//! Hand-written PyO3 wiring for the pull-iter delivery mode.
//!
//! Two surfaces:
//!
//! 1. `ThetaDataDxClient.start_streaming_iter()` — opens an FPSS
//!    streaming connection in pull-iter mode and returns an
//!    [`crate::event_iterator::EventIterator`] handle. The handle is
//!    iterable: `for event in tdx.start_streaming_iter(): ...`.
//!
//! 2. `with tdx.streaming_iter() as session:` — context manager that
//!    pairs `start_streaming_iter` on enter with `stop_streaming` +
//!    `await_drain` on exit, mirroring the push-callback `streaming`
//!    helper in `streaming_session.rs`. `session` is the
//!    `EventIterator` itself; `for event in session:` drains it.
//!
//! Mutual exclusion with `start_streaming(callback)`: both go through
//! the same FPSS slot on the underlying `ThetaDataDxClient`, so calling
//! either while streaming is already running raises `RuntimeError`.

use pyo3::exceptions::PyRuntimeWarning;
use pyo3::prelude::*;

use crate::event_iterator::EventIterator;
use crate::streaming_session::StreamableHandle;

/// Drain timeout applied on `with`-block exit. Same 5 s budget as
/// `streaming_session.rs` so the two context managers' teardown
/// behaviour stays uniform.
const EXIT_DRAIN_TIMEOUT_MS: u64 = 5_000;

/// Context manager returned by `ThetaDataDxClient.streaming_iter()`.
///
/// Holds a strong reference to the streaming pyclass. `__enter__`
/// calls `start_streaming_iter()` on the wrapped pyclass and returns
/// the resulting [`EventIterator`]; `__exit__` calls `close()` on the
/// iterator, then `stop_streaming()` + `await_drain()` on the client
/// so the consumer thread has finished pushing the residual queue
/// before control returns.
///
/// Subscribe / unsubscribe through the `client` field directly — the
/// fluent API keeps subscriptions decoupled from the iterator
/// lifetime.
#[pyclass(module = "thetadatadx", name = "StreamingIterSession")]
pub(crate) struct StreamingIterSession {
    /// Typed handle to the streaming pyclass — closed sum of the two
    /// transports the session knows how to drive (replaces the
    /// duck-typed `Py<PyAny>` slot the field previously carried).
    pub(crate) tdx: StreamableHandle,
    /// Captured at `__enter__` so `__exit__` can close it without
    /// going through the wrapped client. `None` before `__enter__`
    /// and after `__exit__` to make repeated lifecycle errors loud.
    pub(crate) iterator: Option<Py<EventIterator>>,
}

#[pymethods]
impl StreamingIterSession {
    /// Open a pull-iter streaming session and return the iterator
    /// handle. `with tdx.streaming_iter() as it: for event in it:`.
    fn __enter__<'py>(
        mut slf: PyRefMut<'py, Self>,
        py: Python<'py>,
    ) -> PyResult<Py<EventIterator>> {
        if slf.iterator.is_some() {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "StreamingIterSession is already entered -- one session enters at most once",
            ));
        }
        let iter_value = slf.tdx.start_streaming_iter(py)?;
        let iterator: Py<EventIterator> = Py::new(py, iter_value)?;
        slf.iterator = Some(iterator.clone_ref(py));
        Ok(iterator)
    }

    /// Stop streaming + block on the drain barrier so the consumer
    /// thread is guaranteed to have stopped pushing into the iterator
    /// queue before this returns. Closes the iterator first so any
    /// lingering `for event in iter:` loop in the caller (e.g. on a
    /// helper thread) returns promptly.
    #[pyo3(signature = (exc_type=None, exc_value=None, traceback=None))]
    fn __exit__(
        &mut self,
        py: Python<'_>,
        exc_type: Option<Py<PyAny>>,
        exc_value: Option<Py<PyAny>>,
        traceback: Option<Py<PyAny>>,
    ) -> PyResult<bool> {
        let _ = (exc_type, exc_value, traceback);

        if let Some(it) = self.iterator.take() {
            // Best-effort close — clears the iterator's `closed`
            // flag and bubbles `StopIteration` to any in-flight
            // `__next__` once the queue drains.
            let bound = it.bind(py);
            let _ = bound.call_method0("close");
        }

        self.tdx.stop_streaming(py);
        let drained = self.tdx.await_drain(py, EXIT_DRAIN_TIMEOUT_MS);
        if !drained {
            // Mirror the `StreamingSession` warning so operators see
            // the same drain-timeout message regardless of delivery
            // mode. RuntimeWarning rather than a hard exception so
            // any exception raised inside the `with` body still
            // propagates.
            let warnings = py.import("warnings")?;
            let msg = format!(
                "streaming iter drain timed out after \
                 {EXIT_DRAIN_TIMEOUT_MS}ms; the consumer thread may still be pushing \
                 events into the queue."
            );
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("stacklevel", 2_u32)?;
            warnings.call_method(
                "warn",
                (msg, py.get_type::<PyRuntimeWarning>()),
                Some(&kwargs),
            )?;
        }
        Ok(false)
    }

    /// Forward unknown attribute access to the wrapped streaming
    /// pyclass so subscribe / unsubscribe / metric getters are
    /// reachable through `session.subscribe(...)` etc., matching the
    /// push-callback `StreamingSession` proxying. Methods owned by
    /// this class (`__enter__`, `__exit__`, `tdx`, `iterator`) take
    /// precedence — PyO3 only invokes `__getattr__` on lookup miss.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        Ok(self.tdx.bind_any(py).getattr(name)?.unbind())
    }
}

/// Hand-written `start_streaming_iter` and `streaming_iter` factory on
/// `ThetaDataDxClient` — second `#[pymethods]` block enabled by the
/// `multiple-pymethods` PyO3 feature flag.
#[pymethods]
impl crate::ThetaDataDxClient {
    /// Start FPSS streaming in pull-iter delivery mode.
    ///
    /// Returns an [`EventIterator`] that drains the per-client
    /// bounded queue on the calling thread. The iterator is iterable
    /// directly (`for event in tdx.start_streaming_iter():`) and
    /// supports the context-manager protocol (`with iter:` for
    /// guaranteed cleanup).
    ///
    /// Raises `RuntimeError` when streaming is already running on
    /// this client. Pull and push are mutually exclusive on a given
    /// `ThetaDataDxClient`; switch by calling `stop_streaming()` first.
    pub(crate) fn start_streaming_iter(&self) -> PyResult<EventIterator> {
        let inner = self
            .tdx
            .start_streaming_iter()
            .map_err(crate::errors::to_py_err)?;
        Ok(EventIterator::new(inner))
    }

    /// Open a context-managed pull-iter streaming session.
    ///
    /// `with tdx.streaming_iter() as it: for event in it: ...` opens
    /// the FPSS connection in pull-iter mode on enter, drains the
    /// iterator inside the body, and pairs `stop_streaming()` +
    /// `await_drain(5_000)` on exit. Subscribe / unsubscribe via the
    /// session: `session.subscribe(...)` is forwarded to the wrapped
    /// client through `StreamingIterSession.__getattr__`.
    fn streaming_iter(slf: Py<Self>, py: Python<'_>) -> PyResult<Py<StreamingIterSession>> {
        Py::new(
            py,
            StreamingIterSession {
                tdx: StreamableHandle::Tdx(slf),
                iterator: None,
            },
        )
    }
}
