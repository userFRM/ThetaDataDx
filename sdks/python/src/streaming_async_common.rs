//! Shared machinery between the two asyncio-native streaming surfaces.
//!
//! [`StreamingAsyncSession`](crate::StreamingAsyncSession) yields
//! `list[FpssEvent]` per OS wake; [`StreamingAsyncBatchesSession`](crate::StreamingAsyncBatchesSession)
//! yields one `pyarrow.RecordBatch` per OS wake. Both run the same
//! self-pipe / wake-FD lifecycle on top of a Rust [`EventIterator`] —
//! this module owns the SSOT pieces both files share so neither has
//! to keep a private copy.
//!
//! What lives here:
//!
//! * [`AsyncStreamableHandle`] — the closed sum of the two streaming
//!   pyclasses (`ThetaDataDxClient` + `FpssClient`). Both sessions
//!   dispatch subscribe / unsubscribe / stop_streaming through it.
//!
//! The wake-FD helpers (`alloc_wake_pipe`, `drain_read_pipe`,
//! `close_read_fd_propagating_error`) and the `BackpressurePolicy`
//! pyenum still live in `streaming_async_session` for backwards-compat
//! re-export. Both batched and per-tick paths import them from there.

use std::sync::Arc;

use pyo3::prelude::*;

use thetadatadx::fpss::wake::WakeFd;
#[cfg(unix)]
use thetadatadx::fpss::BackpressurePolicy as RustBackpressurePolicy;
use thetadatadx::EventIterator as RustEventIterator;

/// Typed handle to the underlying streaming client used by BOTH the
/// per-tick `StreamingAsyncSession` and the Arrow-batched
/// `StreamingAsyncBatchesSession`. The two enums in the predecessor
/// versions of this codebase were identical modulo name; they now
/// share this single definition.
pub(crate) enum AsyncStreamableHandle {
    /// Unified `ThetaDataDxClient` (MDDS + FPSS).
    Tdx(Py<crate::ThetaDataDxClient>),
    /// Standalone FPSS-only client.
    Fpss(Py<crate::fpss_client::FpssClient>),
}

impl AsyncStreamableHandle {
    /// Bind the inner pyclass as a `Bound<PyAny>` for attribute /
    /// method dispatch via the Python proxy.
    pub(crate) fn bind_any<'py>(&'py self, py: Python<'py>) -> Bound<'py, PyAny> {
        match self {
            Self::Tdx(handle) => handle.bind(py).clone().into_any(),
            Self::Fpss(handle) => handle.bind(py).clone().into_any(),
        }
    }

    /// Open the iterator + wake FD pair on the underlying client.
    /// Returns the Rust iterator and the shared `Arc<WakeFd>` so the
    /// session can `rearm()` from the asyncio reader path.
    #[cfg(unix)]
    pub(crate) fn start(
        &self,
        py: Python<'_>,
        write_fd: i32,
        max_queue_depth: usize,
        backpressure: RustBackpressurePolicy,
    ) -> PyResult<(RustEventIterator, Arc<WakeFd>)> {
        match self {
            Self::Tdx(handle) => handle.borrow(py).start_streaming_async_inner(
                write_fd,
                max_queue_depth,
                backpressure,
            ),
            Self::Fpss(handle) => handle.borrow(py).start_streaming_async_inner(
                write_fd,
                max_queue_depth,
                backpressure,
            ),
        }
    }

    /// Proxy `subscribe` / `subscribe_many` / `unsubscribe` /
    /// `unsubscribe_many` through Python attribute lookup. The
    /// underlying pyclass exposes these via `#[pymethods]` so the
    /// Python method dispatch path is the SSOT.
    pub(crate) fn call_proxy(
        &self,
        py: Python<'_>,
        name: &str,
        arg: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let bound = self.bind_any(py);
        bound.call_method1(name, (arg.clone(),))?;
        Ok(())
    }

    pub(crate) fn stop_streaming(&self, py: Python<'_>) -> PyResult<()> {
        let bound = self.bind_any(py);
        bound.call_method0("stop_streaming")?;
        Ok(())
    }

    pub(crate) fn await_drain(&self, py: Python<'_>, timeout_ms: u64) -> PyResult<bool> {
        let bound = self.bind_any(py);
        bound.call_method1("await_drain", (timeout_ms,))?.extract()
    }
}
