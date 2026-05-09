//! Hand-written PyO3 wrapper for the pull-iter FPSS delivery mode.
//!
//! Returned by the [`crate::ThetaDataDxClient::start_streaming_iter`]
//! method, or yielded inside `with tdx.streaming_iter() as it:` via
//! the [`StreamingIterSession`] context manager (recommended). The
//! Disruptor consumer thread pushes typed
//! [`thetadatadx::fpss::FpssEvent`] clones into a shared bounded
//! queue; this iterator drains the queue from the user thread,
//! converting each event to the typed PyO3 dataclass on demand and
//! holding the GIL across the drain so a `for event in it:` loop
//! pays one GIL acquisition per batch instead of once per delivered
//! tick.
//!
//! The wrapper mirrors the C++ iterator and the TypeScript async
//! iterator; SSOT is the Rust [`thetadatadx::EventIterator`] in
//! `crates/thetadatadx/src/fpss/mod.rs`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use pyo3::exceptions::{PyRuntimeError, PyStopIteration};
use pyo3::prelude::*;
use thetadatadx::{EventIterator as RustEventIterator, NextEvent};

use crate::buffered_event_to_typed;
use crate::fpss_event_to_buffered;

/// Pull-iter handle returned by
/// `ThetaDataDxClient.start_streaming_iter()` and by the
/// `with tdx.streaming_iter() as session:` context manager.
///
/// Iterates typed FPSS event objects (the same `Quote` / `Trade` /
/// `Ohlcvc` / control classes the push-callback path emits). The
/// `__next__` method blocks until an event arrives or the underlying
/// streaming session shuts down; it raises `StopIteration` once the
/// queue has drained on a stopped session.
///
/// # Lifecycle
///
/// 1. `iterator = tdx.start_streaming_iter()` — opens the FPSS
///    connection, returns this handle. The Disruptor consumer thread
///    runs in pull-mode for the lifetime of the handle.
/// 2. `tdx.subscribe(...)` — install subscriptions. Same surface as
///    the push-callback path; subscriptions queue independently of
///    the iterator.
/// 3. `for event in iterator:` — drain. The loop holds the GIL across
///    each pop, so under load N events are delivered under one GIL
///    acquisition rather than N.
/// 4. `iterator.close()` — explicitly retire the iterator without
///    shutting down the streaming session. The next `__next__`
///    raises `StopIteration` once the queue is drained.
/// 5. `tdx.stop_streaming()` — shut down the FPSS session. The next
///    `__next__` raises `StopIteration` once the queue is drained
///    even if `close()` was not called.
#[pyclass(module = "thetadatadx", name = "EventIterator", unsendable)]
pub(crate) struct EventIterator {
    /// The Rust-side iterator owns the queue handle and the upstream
    /// shutdown signal. Wrapped in an `Arc` so the Python `__next__`
    /// path can release the GIL across the (potentially blocking)
    /// `next_timeout` call and the iterator can be cloned-by-reference
    /// internally if the binding ever exposes secondary observation
    /// helpers (we do not today).
    inner: Arc<RustEventIterator>,
    /// Set on `close()` so subsequent `__next__` calls return
    /// immediately once the queue is drained, without waiting on the
    /// upstream `client_shutdown` flag. Independent of the Rust
    /// iterator's own `finished` flag because that one flips only on
    /// `RustEventIterator::close()`; the `closed` flag here is a
    /// Python-binding-level state echo so the
    /// `__enter__` / `__exit__` context-manager path can short-circuit
    /// the wait without forcing a global stop.
    closed: Arc<AtomicBool>,
}

impl EventIterator {
    pub(crate) fn new(inner: RustEventIterator) -> Self {
        Self {
            inner: Arc::new(inner),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[pymethods]
impl EventIterator {
    /// `__iter__` returns `self` so `for event in iterator:` works
    /// directly on the handle. Idiomatic Python-iterator contract.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Pop the next typed FPSS event, blocking until one arrives or
    /// the streaming session shuts down. Releases the GIL across the
    /// blocking wait so the Disruptor consumer thread is unobstructed
    /// on the producer side. Raises `StopIteration` once the queue is
    /// drained on a stopped session.
    fn __next__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        loop {
            // Local `close()` short-circuits the wait. Re-check the
            // queue once before raising so any events that landed
            // between the close call and now still surface (matches
            // the Rust EventIterator::next ordering).
            if self.closed.load(Ordering::Acquire) {
                // `try_next` since 9.1.0 returns the typed `NextEvent`
                // trichotomy. Only `Ready(evt)` surfaces an event;
                // both `Timeout` (queue empty) and `Closed` (drained +
                // shutdown) raise `StopIteration` here because the
                // local close flag has already fired, so the caller's
                // contract is "drain residuals, then end".
                return match self.inner.try_next() {
                    NextEvent::Ready(evt) => convert_event(py, &evt),
                    NextEvent::Timeout | NextEvent::Closed => Err(PyStopIteration::new_err(())),
                };
            }
            // Release the GIL across the blocking wait. The Disruptor
            // consumer thread must acquire the GIL only when it
            // converts an event to the typed pyclass below, NOT inside
            // the queue-push path; this `detach` ensures the
            // consumer's pushes never contend with this thread's GIL
            // hold. 50 ms is long enough that quiet periods don't
            // burn CPU re-checking the close flag and short enough
            // that a `KeyboardInterrupt` propagates within a
            // human-perceptible delay.
            let outcome = py.detach(|| {
                self.inner
                    .next_timeout(std::time::Duration::from_millis(50))
            });
            match outcome {
                NextEvent::Ready(evt) => return convert_event(py, &evt),
                NextEvent::Closed => {
                    // Upstream client shut down AND the queue is
                    // drained. Distinct from `Timeout`: the SDK now
                    // signals `StopIteration` so a `for event in
                    // iterator:` loop exits instead of spinning.
                    return Err(PyStopIteration::new_err(()));
                }
                NextEvent::Timeout => {
                    // Re-check Python signals so a Ctrl+C breaks the
                    // loop. PyErr_CheckSignals returns -1 on pending
                    // signal; the PyO3 helper raises that as a Python
                    // exception which we propagate through `?`.
                    py.check_signals()?;
                    // Continue looping — the queue was empty within
                    // the 50 ms slice but the upstream is still live.
                    continue;
                }
            }
        }
    }

    /// Try to pop the next event without blocking. Returns `None` on
    /// either an empty-but-live queue OR a terminal end-of-stream
    /// (queue drained on a stopped session). Useful for non-blocking
    /// polling integrations (e.g. `select`-style multiplexing).
    /// Callers that need to distinguish the two cases should use the
    /// blocking `__next__` path, which raises `StopIteration` only on
    /// terminal end-of-stream.
    fn try_next(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        // `NextEvent::Ready` surfaces an event; both `Timeout` (queue
        // empty, upstream live) and `Closed` (drained + shutdown) map
        // to `None` here so the Python public surface stays a simple
        // optional. The 9.1.0 typed-enum upgrade lives on the
        // blocking `__next__` path which can raise `StopIteration` on
        // `Closed`; non-blocking polling stays single-state by design.
        match self.inner.try_next() {
            NextEvent::Ready(evt) => Ok(Some(convert_event(py, &evt)?)),
            NextEvent::Timeout | NextEvent::Closed => Ok(None),
        }
    }

    /// Number of events currently buffered between the Disruptor
    /// consumer and this iterator. Diagnostic only — the value is
    /// racy because the consumer pushes concurrently.
    fn queue_len(&self) -> usize {
        self.inner.queue_len()
    }

    /// Mark the iterator as closed. Subsequent `__next__` calls
    /// raise `StopIteration` once the queue is drained, without
    /// shutting down the underlying streaming session. Idempotent.
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.inner.close();
    }

    /// Context-manager entry — returns `self` so `with iter as it:`
    /// is a no-op, matching the iterator-protocol convention. The
    /// `__exit__` companion calls `close()` so a `for event in iter:`
    /// loop wrapped in `with` cleans up the iterator handle even on
    /// exception.
    fn __enter__<'py>(slf: PyRef<'py, Self>) -> PyRef<'py, Self> {
        slf
    }

    #[pyo3(signature = (exc_type=None, exc_value=None, traceback=None))]
    fn __exit__(
        &mut self,
        exc_type: Option<Py<PyAny>>,
        exc_value: Option<Py<PyAny>>,
        traceback: Option<Py<PyAny>>,
    ) -> bool {
        let _ = (exc_type, exc_value, traceback);
        self.close();
        // Returning `False` lets exceptions raised inside the `with`
        // body propagate, matching the Python context-manager
        // contract for non-exception-suppressing managers.
        false
    }
}

fn convert_event(py: Python<'_>, event: &thetadatadx::fpss::FpssEvent) -> PyResult<Py<PyAny>> {
    let buffered = fpss_event_to_buffered(event);
    buffered_event_to_typed(py, &buffered).map_err(|e| {
        PyRuntimeError::new_err(format!(
            "failed to convert FPSS event to typed Python class: {e}"
        ))
    })
}
