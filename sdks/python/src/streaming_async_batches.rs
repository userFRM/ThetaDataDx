//! Arrow IPC zero-copy batched streaming for FPSS.
//!
//! Sibling of [`crate::streaming_async_session::StreamingAsyncSession`]:
//! same wake-FD plumbing, same backpressure semantics, same context-
//! manager and async-iterator surface — but the `__anext__` drain
//! materialises the accumulated batch as a single
//! [`arrow::record_batch::RecordBatch`] (zero-copied into pyarrow via
//! the Arrow C Data Interface) rather than a `list[FpssEvent]` of
//! per-tick `Py<...>` allocations.
//!
//! # Why
//!
//! At sustained event rates (≥ 5 k events/sec for QQQ quotes during US
//! cash hours, ≥ 100 k events/sec for full-stream OPRA option quotes)
//! the per-tick PyObject construction in
//! [`StreamingAsyncSession::__anext__`] dominates wall time on the
//! consumer side. Each event crosses the PyO3 boundary as a discrete
//! pyclass instance plus an `Arc<Contract>` refcount bump; the user
//! consumer's `for ev in batch:` loop then pays Python-side attribute
//! lookups on every field.
//!
//! Arrow IPC batching delivers the same data as ONE columnar buffer
//! per drain, zero-copy aliased into pyarrow via the
//! [`arrow_pyarrow::IntoPyArrow`] / Arrow C Data Interface bridge.
//! Vectorised downstream processing (`batch.to_pandas()`, polars,
//! datafusion) bypasses the per-row Python loop entirely.
//!
//! Standard pattern across databento, Tardis, ArcticDB, kdb+ →
//! kx-streaming. The Kairos `streaming_async_batches()` reference
//! (see `kairos-streaming-recap.md`) is the immediate source of
//! truth.
//!
//! # Schema
//!
//! Union-schema approach: ONE [`Schema`] with a `kind` discriminator
//! column (string: `"Quote"` / `"Trade"` / `"OpenInterest"` /
//! `"Ohlcvc"`) and a superset of all per-variant fields, nullable
//! where the variant does not populate them. Consumers filter via
//! `batch.filter(pc.field("kind") == "Quote")` — the standard Arrow
//! pattern.
//!
//! The schema is intentionally NOT inferred from the
//! `fpss_event_schema.toml` — the pull-iter streaming surface emits a
//! mixed-variant stream, so the schema must always be the union of
//! every variant's columns. Control events (LoginSuccess /
//! ContractAssigned / …) are filtered out of the data batch — they
//! still flow through the per-tick `StreamingAsyncSession` surface for
//! callers who need them.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, Float64Array, Float64Builder, Int32Array, Int32Builder, Int64Builder, StringArray,
    StringBuilder, UInt64Array,
};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::pyarrow::IntoPyArrow;
use arrow::record_batch::RecordBatch;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use pyo3::prelude::*;

use thetadatadx::fpss::wake::WakeFd;
#[cfg(unix)]
use thetadatadx::fpss::BackpressurePolicy as RustBackpressurePolicy;
use thetadatadx::fpss::{FpssData, FpssEvent};
use thetadatadx::{EventIterator as RustEventIterator, NextEvent};

use crate::streaming_async_session::BackpressurePolicy;

/// Drain timeout applied on `__aexit__`. Same 5 s budget as the sync
/// `StreamingSession` / `StreamingIterSession` and the per-tick async
/// session so all four context managers' teardown behaviour stays
/// uniform.
const EXIT_DRAIN_TIMEOUT_MS: u64 = 5_000;

/// Default `max_queue_depth` for the async pull-iter surfaces when the
/// caller does not override. Matches
/// [`crate::streaming_async_session::StreamingAsyncSession`]'s default
/// of 4096 — keeping the same bound so callers can swap surfaces
/// without retuning queue size.
const DEFAULT_MAX_QUEUE_DEPTH: usize = 4096;

/// Typed handle to the underlying streaming client. Mirrors the
/// `AsyncStreamableHandle` enum in `streaming_async_session.rs` —
/// duplicated here because that one is `pub(crate)` and lives in a
/// sibling module, and a third indirection through a shared module
/// would force both surfaces to depend on a "common-batches" carrier
/// type that adds zero value. Keeping the two enums separate is the
/// simpler SSOT split: each surface owns its handle dispatch.
pub(crate) enum AsyncBatchesStreamableHandle {
    /// Unified `ThetaDataDxClient` (MDDS + FPSS).
    Tdx(Py<crate::ThetaDataDxClient>),
    /// Standalone FPSS-only client.
    Fpss(Py<crate::fpss_client::FpssClient>),
}

impl AsyncBatchesStreamableHandle {
    fn bind_any<'py>(&'py self, py: Python<'py>) -> Bound<'py, PyAny> {
        match self {
            Self::Tdx(handle) => handle.bind(py).clone().into_any(),
            Self::Fpss(handle) => handle.bind(py).clone().into_any(),
        }
    }

    #[cfg(unix)]
    fn start(
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

    fn call_proxy(&self, py: Python<'_>, name: &str, arg: &Bound<'_, PyAny>) -> PyResult<()> {
        let bound = self.bind_any(py);
        bound.call_method1(name, (arg.clone(),))?;
        Ok(())
    }

    fn stop_streaming(&self, py: Python<'_>) -> PyResult<()> {
        let bound = self.bind_any(py);
        bound.call_method0("stop_streaming")?;
        Ok(())
    }

    fn await_drain(&self, py: Python<'_>, timeout_ms: u64) -> PyResult<bool> {
        let bound = self.bind_any(py);
        bound.call_method1("await_drain", (timeout_ms,))?.extract()
    }
}

/// Asyncio-native context manager + async iterator that yields one
/// `pyarrow.RecordBatch` per OS wake.
///
/// Acquired via `client.streaming_async_batches()` on either
/// `ThetaDataDxClient` or `FpssClient`. The session takes ownership of
/// the same wake-FD plumbing as [`crate::streaming_async_session::StreamingAsyncSession`]:
///
/// * a self-pipe `(read_fd, write_fd)` allocated in `__aenter__`,
/// * a Rust [`EventIterator`] draining the bounded queue,
/// * an `asyncio.Event` set from the `add_reader` callback,
/// * the shared `Arc<WakeFd>` so the reader can `rearm()` before
///   draining the pipe.
///
/// The difference is the `__anext__` drain shape: instead of
/// constructing one typed pyclass per event, it accumulates the batch
/// into typed Arrow array builders and emits a single
/// `pyarrow.RecordBatch` carrying the whole drain via the Arrow C
/// Data Interface (zero-copy buffer alias).
#[pyclass(
    module = "thetadatadx",
    name = "StreamingAsyncBatchesSession",
    unsendable
)]
pub(crate) struct StreamingAsyncBatchesSession {
    handle: AsyncBatchesStreamableHandle,
    read_fd: i32,
    wake: Option<Arc<WakeFd>>,
    iterator: Option<Arc<RustEventIterator>>,
    event_loop: Option<Py<PyAny>>,
    asyncio_event: Option<Py<PyAny>>,
    closed: bool,
    max_queue_depth: usize,
    backpressure: BackpressurePolicy,
    /// Cached Arrow schema. Reused across every batch so a downstream
    /// `pyarrow` consumer sees stable column ordering and types — the
    /// builder allocates ~zero per batch (the schema Arc is cloned via
    /// refcount bump). Lazily initialised on first `__aenter__` since
    /// the constructor must stay cheap.
    schema: Arc<Schema>,
}

#[pymethods]
impl StreamingAsyncBatchesSession {
    /// Async context-manager entry. Same protocol as
    /// [`crate::streaming_async_session::StreamingAsyncSession::__aenter__`]
    /// — allocate the self-pipe, hand the write-end to the Rust core
    /// via the policy-aware constructor, register the read-end on the
    /// asyncio loop's `add_reader`.
    fn __aenter__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let mut session = session_handle.borrow_mut(py);
                session.aenter_inner(py)?;
                Ok::<Py<PyAny>, PyErr>(session_handle.clone_ref(py).into_any())
            })
        })
    }

    /// Async context-manager exit.
    #[pyo3(signature = (exc_type=None, exc_value=None, traceback=None))]
    fn __aexit__<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        exc_type: Option<Py<PyAny>>,
        exc_value: Option<Py<PyAny>>,
        traceback: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let _ = (exc_type, exc_value, traceback);
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let mut session = session_handle.borrow_mut(py);
                session.aexit_inner(py)?;
                Ok::<Py<PyAny>, PyErr>(py.None())
            })
        })
    }

    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// `async for batch in session:` — drain one pyarrow.RecordBatch
    /// per OS wake.
    fn __anext__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            anext_step_batches(session_handle).await
        })
    }

    /// Awaitable subscribe — proxies through to the underlying client
    /// pyclass so the FPSS-protocol round-trip semantics match the
    /// per-tick async session.
    fn subscribe<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        sub: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let session = session_handle.borrow(py);
                session.handle.call_proxy(py, "subscribe", sub.bind(py))?;
                Ok::<Py<PyAny>, PyErr>(py.None())
            })
        })
    }

    fn subscribe_many<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        subs: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let session = session_handle.borrow(py);
                session
                    .handle
                    .call_proxy(py, "subscribe_many", subs.bind(py))?;
                Ok::<Py<PyAny>, PyErr>(py.None())
            })
        })
    }

    fn unsubscribe<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        sub: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let session = session_handle.borrow(py);
                session.handle.call_proxy(py, "unsubscribe", sub.bind(py))?;
                Ok::<Py<PyAny>, PyErr>(py.None())
            })
        })
    }

    fn unsubscribe_many<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        subs: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let session = session_handle.borrow(py);
                session
                    .handle
                    .call_proxy(py, "unsubscribe_many", subs.bind(py))?;
                Ok::<Py<PyAny>, PyErr>(py.None())
            })
        })
    }

    /// Instantaneous queue depth — same getter as the per-tick async
    /// session.
    fn queue_len(&self) -> usize {
        self.iterator.as_ref().map_or(0, |it| it.queue_len())
    }

    /// Alias for [`Self::queue_len`] matching the kdb+ / BLPAPI
    /// operator vocabulary.
    fn queue_depth(&self) -> usize {
        self.queue_len()
    }

    fn dropped_event_count(&self, py: Python<'_>) -> PyResult<u64> {
        let bound = self.handle.bind_any(py);
        bound.call_method0("dropped_event_count")?.extract()
    }

    #[getter]
    fn max_queue_depth(&self) -> usize {
        self.max_queue_depth
    }

    #[getter]
    fn backpressure(&self) -> BackpressurePolicy {
        self.backpressure
    }

    /// Snapshot of the Arrow schema this session emits. Exposed as a
    /// `pyarrow.Schema` so callers can build downstream pipelines
    /// (parquet writers, polars/datafusion schema bridges) without
    /// having to drain a batch first.
    fn schema(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        // Materialise an empty RecordBatch with the cached schema and
        // pull `.schema` off the pyarrow side — the C Data Interface
        // exposes the schema field directly, but the easiest
        // round-trip is via an empty batch since pyarrow ships a typed
        // accessor on RecordBatch and not on a free schema bridge.
        let schema = Arc::clone(&self.schema);
        let empty_batch = empty_record_batch(schema.clone())?;
        let obj = batch_to_pyarrow(py, empty_batch)?;
        let bound = obj.bind(py);
        Ok(bound.getattr("schema")?.unbind())
    }
}

impl StreamingAsyncBatchesSession {
    pub(crate) fn from_tdx(
        handle: Py<crate::ThetaDataDxClient>,
        max_queue_depth: usize,
        backpressure: BackpressurePolicy,
    ) -> Self {
        Self {
            handle: AsyncBatchesStreamableHandle::Tdx(handle),
            read_fd: -1,
            wake: None,
            iterator: None,
            event_loop: None,
            asyncio_event: None,
            closed: false,
            max_queue_depth,
            backpressure,
            schema: fpss_event_schema(),
        }
    }

    pub(crate) fn from_fpss(
        handle: Py<crate::fpss_client::FpssClient>,
        max_queue_depth: usize,
        backpressure: BackpressurePolicy,
    ) -> Self {
        Self {
            handle: AsyncBatchesStreamableHandle::Fpss(handle),
            read_fd: -1,
            wake: None,
            iterator: None,
            event_loop: None,
            asyncio_event: None,
            closed: false,
            max_queue_depth,
            backpressure,
            schema: fpss_event_schema(),
        }
    }

    #[cfg(unix)]
    fn aenter_inner(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.iterator.is_some() {
            return Err(PyRuntimeError::new_err(
                "StreamingAsyncBatchesSession is already entered -- one session enters at most once",
            ));
        }

        let (read_fd, write_fd) = crate::streaming_async_session::alloc_wake_pipe()?;

        let (rust_iter, wake) = match self.handle.start(
            py,
            write_fd,
            self.max_queue_depth,
            self.backpressure.to_core(),
        ) {
            Ok(pair) => pair,
            Err(err) => {
                // SAFETY: both FDs are open, owned by this scope,
                // and not yet shared.
                unsafe {
                    libc::close(read_fd);
                    libc::close(write_fd);
                }
                return Err(err);
            }
        };

        let asyncio = py.import("asyncio")?;
        let event_loop = asyncio.call_method0("get_running_loop")?;
        let asyncio_event = asyncio.call_method0("Event")?;

        let set_event = asyncio_event.getattr("set")?;
        event_loop.call_method1("add_reader", (read_fd, set_event))?;

        self.read_fd = read_fd;
        self.wake = Some(wake);
        self.iterator = Some(Arc::new(rust_iter));
        self.event_loop = Some(event_loop.unbind());
        self.asyncio_event = Some(asyncio_event.unbind());
        self.closed = false;
        Ok(())
    }

    #[cfg(not(unix))]
    fn aenter_inner(&mut self, _py: Python<'_>) -> PyResult<()> {
        Err(PyRuntimeError::new_err(
            "streaming_async_batches() requires a POSIX platform (Linux / macOS / BSD); \
             Windows asyncio's ProactorEventLoop does not support add_reader on pipes. \
             Use client.streaming(callback) or client.streaming_iter() instead.",
        ))
    }

    fn aexit_inner(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;

        if let (Some(loop_obj), read_fd) = (self.event_loop.as_ref(), self.read_fd) {
            if read_fd >= 0 {
                let bound = loop_obj.bind(py);
                let _ = bound.call_method1("remove_reader", (read_fd,))?;
            }
        }

        #[cfg(unix)]
        if self.read_fd >= 0 {
            // SAFETY: `self.read_fd` was allocated in `aenter_inner`,
            // owned by this session, and the asyncio reader was
            // removed in the step above so no other thread touches it.
            unsafe {
                libc::close(self.read_fd);
            }
            self.read_fd = -1;
        }

        self.handle.stop_streaming(py)?;
        let drained = self.handle.await_drain(py, EXIT_DRAIN_TIMEOUT_MS)?;

        self.iterator = None;
        self.wake = None;
        self.event_loop = None;
        self.asyncio_event = None;

        if !drained {
            let warnings = py.import("warnings")?;
            let msg = format!(
                "streaming_async_batches drain timed out after {EXIT_DRAIN_TIMEOUT_MS}ms; \
                 the consumer thread may still be draining residual events."
            );
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("stacklevel", 2_u32)?;
            let runtime_warning = py.get_type::<pyo3::exceptions::PyRuntimeWarning>();
            warnings.call_method("warn", (msg, runtime_warning), Some(&kwargs))?;
        }
        Ok(())
    }
}

/// Union schema for the FPSS data stream. Single source of truth for
/// every column the batched surface emits; the per-variant builders
/// below populate the columns or write nulls based on which
/// `FpssData` variant the event carries.
///
/// Layout:
///
/// * `kind` (string) — variant tag, `"Quote"` / `"Trade"` /
///   `"OpenInterest"` / `"Ohlcvc"`.
/// * `symbol` / `sec_type` / `expiration` / `right` / `strike` —
///   contract identity (`expiration` / `right` / `strike` nullable for
///   stocks / indices).
/// * `ms_of_day` / `date` / `received_at_ns` — timestamp axis.
/// * Quote-only: `bid_size` / `bid_exchange` / `bid` / `bid_condition`
///   / `ask_size` / `ask_exchange` / `ask` / `ask_condition`.
/// * Trade-only: `trade_price` / `trade_size` / `trade_exchange` /
///   `trade_condition` / `sequence` / `condition_flags` / `price_flags`
///   / `volume_type` / `records_back` / `ext_condition1..4`.
/// * OpenInterest-only: `open_interest`.
/// * OHLCVC-only: `open` / `high` / `low` / `close` / `volume` /
///   `count`.
///
/// Every numeric field that does not apply to a given variant is
/// written as null on that row; the union-schema pattern is the
/// idiomatic Arrow approach for a heterogeneous tagged-union stream.
fn fpss_event_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("kind", DataType::Utf8, false),
        // Contract identity
        Field::new("symbol", DataType::Utf8, false),
        Field::new("sec_type", DataType::Utf8, false),
        Field::new("expiration", DataType::Int32, true),
        Field::new("right", DataType::Utf8, true),
        Field::new("strike", DataType::Int32, true),
        // Timestamp axis
        Field::new("ms_of_day", DataType::Int32, false),
        Field::new("date", DataType::Int32, false),
        Field::new("received_at_ns", DataType::UInt64, false),
        // Quote-only
        Field::new("bid", DataType::Float64, true),
        Field::new("ask", DataType::Float64, true),
        Field::new("bid_size", DataType::Int32, true),
        Field::new("ask_size", DataType::Int32, true),
        Field::new("bid_exchange", DataType::Int32, true),
        Field::new("ask_exchange", DataType::Int32, true),
        Field::new("bid_condition", DataType::Int32, true),
        Field::new("ask_condition", DataType::Int32, true),
        // Trade-only
        Field::new("trade_price", DataType::Float64, true),
        Field::new("trade_size", DataType::Int32, true),
        Field::new("trade_exchange", DataType::Int32, true),
        Field::new("trade_condition", DataType::Int32, true),
        Field::new("sequence", DataType::Int32, true),
        Field::new("condition_flags", DataType::Int32, true),
        Field::new("price_flags", DataType::Int32, true),
        Field::new("volume_type", DataType::Int32, true),
        Field::new("records_back", DataType::Int32, true),
        Field::new("ext_condition1", DataType::Int32, true),
        Field::new("ext_condition2", DataType::Int32, true),
        Field::new("ext_condition3", DataType::Int32, true),
        Field::new("ext_condition4", DataType::Int32, true),
        // OpenInterest-only
        Field::new("open_interest", DataType::Int32, true),
        // OHLCVC-only
        Field::new("open", DataType::Float64, true),
        Field::new("high", DataType::Float64, true),
        Field::new("low", DataType::Float64, true),
        Field::new("close", DataType::Float64, true),
        Field::new("volume", DataType::Int64, true),
        Field::new("count", DataType::Int64, true),
    ]))
}

/// Build an empty `RecordBatch` with the supplied schema. Used by
/// [`StreamingAsyncBatchesSession::schema`] to surface a typed schema
/// reference to Python without paying a full batch construction.
fn empty_record_batch(schema: Arc<Schema>) -> PyResult<RecordBatch> {
    let columns: Vec<ArrayRef> = schema
        .fields()
        .iter()
        .map(|f| match f.data_type() {
            DataType::Utf8 => Arc::new(StringArray::new_null(0)) as ArrayRef,
            DataType::Int32 => Arc::new(Int32Array::new_null(0)) as ArrayRef,
            DataType::Int64 => Arc::new(arrow::array::Int64Array::new_null(0)) as ArrayRef,
            DataType::UInt64 => Arc::new(UInt64Array::new_null(0)) as ArrayRef,
            DataType::Float64 => Arc::new(Float64Array::new_null(0)) as ArrayRef,
            other => panic!("unsupported empty-batch column type: {other:?}"),
        })
        .collect();
    RecordBatch::try_new(schema, columns)
        .map_err(|e| PyRuntimeError::new_err(format!("failed to build empty batch: {e}")))
}

/// Convert a `RecordBatch` to a `pyarrow.RecordBatch` via the Arrow C
/// Data Interface (zero-copy buffer alias).
fn batch_to_pyarrow(py: Python<'_>, batch: RecordBatch) -> PyResult<Py<PyAny>> {
    let obj = batch.into_pyarrow(py)?;
    Ok(obj.unbind())
}

/// Drain the iterator's queue into a single Arrow RecordBatch.
///
/// Returns the batch and a `terminal` flag. `terminal` is `true` when
/// the iterator reported `Closed` — the caller surfaces that as
/// `StopAsyncIteration` once the batch (possibly carrying the last
/// tail of events) is delivered to the consumer.
///
/// Control events (LoginSuccess / ContractAssigned / Reconnecting /
/// …) are filtered out of the data batch — the typed per-tick
/// `StreamingAsyncSession` surface remains the canonical path for
/// callers that need control-event observation. Mixing control events
/// into the data batch would force every column to grow another
/// nullable arm per control-variant payload, exploding the schema for
/// negligible benefit on a path explicitly chosen for vectorised data
/// processing.
fn drain_batch_arrow(
    schema: Arc<Schema>,
    iterator: &RustEventIterator,
) -> PyResult<(RecordBatch, bool)> {
    let mut builder = FpssBatchBuilder::new(Arc::clone(&schema));
    let mut terminal = false;
    loop {
        match iterator.try_next() {
            NextEvent::Ready(evt) => {
                builder.append(&evt);
            }
            NextEvent::Timeout => break,
            NextEvent::Closed => {
                terminal = true;
                break;
            }
        }
    }
    let batch = builder.finish()?;
    Ok((batch, terminal))
}

/// Columnar builder for the FPSS union schema. Each row is a
/// `FpssData` variant; the builder writes the variant tag plus the
/// applicable per-variant columns, and null-fills the rest. Allocated
/// per-drain so the next batch starts at zero rows.
///
/// The builders are typed
/// [`arrow::array::builder::PrimitiveBuilder`] /
/// [`arrow::array::builder::StringBuilder`] variants — same primitives
/// the historical `tick_arrow.rs` codegen emits. Per-row append cost
/// is one push per typed column, ~ tens of nanoseconds total per
/// event vs the per-tick `Py<...>` allocation the per-tick session
/// pays.
struct FpssBatchBuilder {
    schema: Arc<Schema>,
    kind: StringBuilder,
    symbol: StringBuilder,
    sec_type: StringBuilder,
    expiration: Int32Builder,
    right: StringBuilder,
    strike: Int32Builder,
    ms_of_day: Int32Builder,
    date: Int32Builder,
    received_at_ns: arrow::array::builder::UInt64Builder,
    // Quote
    bid: Float64Builder,
    ask: Float64Builder,
    bid_size: Int32Builder,
    ask_size: Int32Builder,
    bid_exchange: Int32Builder,
    ask_exchange: Int32Builder,
    bid_condition: Int32Builder,
    ask_condition: Int32Builder,
    // Trade
    trade_price: Float64Builder,
    trade_size: Int32Builder,
    trade_exchange: Int32Builder,
    trade_condition: Int32Builder,
    sequence: Int32Builder,
    condition_flags: Int32Builder,
    price_flags: Int32Builder,
    volume_type: Int32Builder,
    records_back: Int32Builder,
    ext_condition1: Int32Builder,
    ext_condition2: Int32Builder,
    ext_condition3: Int32Builder,
    ext_condition4: Int32Builder,
    // OpenInterest
    open_interest: Int32Builder,
    // Ohlcvc
    open: Float64Builder,
    high: Float64Builder,
    low: Float64Builder,
    close: Float64Builder,
    volume: Int64Builder,
    count: Int64Builder,
}

impl FpssBatchBuilder {
    fn new(schema: Arc<Schema>) -> Self {
        Self {
            schema,
            kind: StringBuilder::new(),
            symbol: StringBuilder::new(),
            sec_type: StringBuilder::new(),
            expiration: Int32Builder::new(),
            right: StringBuilder::new(),
            strike: Int32Builder::new(),
            ms_of_day: Int32Builder::new(),
            date: Int32Builder::new(),
            received_at_ns: arrow::array::builder::UInt64Builder::new(),
            bid: Float64Builder::new(),
            ask: Float64Builder::new(),
            bid_size: Int32Builder::new(),
            ask_size: Int32Builder::new(),
            bid_exchange: Int32Builder::new(),
            ask_exchange: Int32Builder::new(),
            bid_condition: Int32Builder::new(),
            ask_condition: Int32Builder::new(),
            trade_price: Float64Builder::new(),
            trade_size: Int32Builder::new(),
            trade_exchange: Int32Builder::new(),
            trade_condition: Int32Builder::new(),
            sequence: Int32Builder::new(),
            condition_flags: Int32Builder::new(),
            price_flags: Int32Builder::new(),
            volume_type: Int32Builder::new(),
            records_back: Int32Builder::new(),
            ext_condition1: Int32Builder::new(),
            ext_condition2: Int32Builder::new(),
            ext_condition3: Int32Builder::new(),
            ext_condition4: Int32Builder::new(),
            open_interest: Int32Builder::new(),
            open: Float64Builder::new(),
            high: Float64Builder::new(),
            low: Float64Builder::new(),
            close: Float64Builder::new(),
            volume: Int64Builder::new(),
            count: Int64Builder::new(),
        }
    }

    /// Append one row for the given event. Filters control events
    /// (which carry no data payload) — the batched surface only emits
    /// `FpssData` rows. Returns whether a row was actually appended.
    fn append(&mut self, evt: &FpssEvent) -> bool {
        let FpssEvent::Data(data) = evt else {
            return false;
        };
        match data {
            FpssData::Quote {
                contract,
                ms_of_day,
                bid_size,
                bid_exchange,
                bid,
                bid_condition,
                ask_size,
                ask_exchange,
                ask,
                ask_condition,
                date,
                received_at_ns,
            } => {
                self.kind.append_value("Quote");
                self.append_contract(contract);
                self.ms_of_day.append_value(*ms_of_day);
                self.date.append_value(*date);
                self.received_at_ns.append_value(*received_at_ns);
                self.bid.append_value(*bid);
                self.ask.append_value(*ask);
                self.bid_size.append_value(*bid_size);
                self.ask_size.append_value(*ask_size);
                self.bid_exchange.append_value(*bid_exchange);
                self.ask_exchange.append_value(*ask_exchange);
                self.bid_condition.append_value(*bid_condition);
                self.ask_condition.append_value(*ask_condition);
                self.append_null_trade();
                self.open_interest.append_null();
                self.append_null_ohlcvc();
            }
            FpssData::Trade {
                contract,
                ms_of_day,
                sequence,
                ext_condition1,
                ext_condition2,
                ext_condition3,
                ext_condition4,
                condition,
                size,
                exchange,
                price,
                condition_flags,
                price_flags,
                volume_type,
                records_back,
                date,
                received_at_ns,
            } => {
                self.kind.append_value("Trade");
                self.append_contract(contract);
                self.ms_of_day.append_value(*ms_of_day);
                self.date.append_value(*date);
                self.received_at_ns.append_value(*received_at_ns);
                self.append_null_quote();
                self.trade_price.append_value(*price);
                self.trade_size.append_value(*size);
                self.trade_exchange.append_value(*exchange);
                self.trade_condition.append_value(*condition);
                self.sequence.append_value(*sequence);
                self.condition_flags.append_value(*condition_flags);
                self.price_flags.append_value(*price_flags);
                self.volume_type.append_value(*volume_type);
                self.records_back.append_value(*records_back);
                self.ext_condition1.append_value(*ext_condition1);
                self.ext_condition2.append_value(*ext_condition2);
                self.ext_condition3.append_value(*ext_condition3);
                self.ext_condition4.append_value(*ext_condition4);
                self.open_interest.append_null();
                self.append_null_ohlcvc();
            }
            FpssData::OpenInterest {
                contract,
                ms_of_day,
                open_interest,
                date,
                received_at_ns,
            } => {
                self.kind.append_value("OpenInterest");
                self.append_contract(contract);
                self.ms_of_day.append_value(*ms_of_day);
                self.date.append_value(*date);
                self.received_at_ns.append_value(*received_at_ns);
                self.append_null_quote();
                self.append_null_trade();
                self.open_interest.append_value(*open_interest);
                self.append_null_ohlcvc();
            }
            FpssData::Ohlcvc {
                contract,
                ms_of_day,
                open,
                high,
                low,
                close,
                volume,
                count,
                date,
                received_at_ns,
            } => {
                self.kind.append_value("Ohlcvc");
                self.append_contract(contract);
                self.ms_of_day.append_value(*ms_of_day);
                self.date.append_value(*date);
                self.received_at_ns.append_value(*received_at_ns);
                self.append_null_quote();
                self.append_null_trade();
                self.open_interest.append_null();
                self.open.append_value(*open);
                self.high.append_value(*high);
                self.low.append_value(*low);
                self.close.append_value(*close);
                self.volume.append_value(*volume);
                self.count.append_value(*count);
            }
            // `FpssData` is `#[non_exhaustive]`; surface an
            // `UnknownData` row tagged with a sentinel `kind` so any
            // new variant the core crate adds shows up in the batch
            // rather than getting silently dropped. Operators can
            // filter via `pc.field("kind") == "UnknownData"` to flag
            // up the missed variant and upstream a schema bump.
            _ => {
                self.kind.append_value("UnknownData");
                self.symbol.append_value("");
                self.sec_type.append_value("UNKNOWN");
                self.expiration.append_null();
                self.right.append_null();
                self.strike.append_null();
                self.ms_of_day.append_value(0);
                self.date.append_value(0);
                self.received_at_ns.append_value(0);
                self.append_null_quote();
                self.append_null_trade();
                self.open_interest.append_null();
                self.append_null_ohlcvc();
            }
        }
        true
    }

    fn append_contract(&mut self, contract: &thetadatadx::fpss::protocol::Contract) {
        self.symbol.append_value(&contract.symbol);
        self.sec_type.append_value(contract.sec_type.as_str());
        self.expiration.append_option(contract.expiration);
        self.right
            .append_option(contract.right().map(|r| r.as_char().to_string()));
        self.strike.append_option(contract.strike);
    }

    fn append_null_quote(&mut self) {
        self.bid.append_null();
        self.ask.append_null();
        self.bid_size.append_null();
        self.ask_size.append_null();
        self.bid_exchange.append_null();
        self.ask_exchange.append_null();
        self.bid_condition.append_null();
        self.ask_condition.append_null();
    }

    fn append_null_trade(&mut self) {
        self.trade_price.append_null();
        self.trade_size.append_null();
        self.trade_exchange.append_null();
        self.trade_condition.append_null();
        self.sequence.append_null();
        self.condition_flags.append_null();
        self.price_flags.append_null();
        self.volume_type.append_null();
        self.records_back.append_null();
        self.ext_condition1.append_null();
        self.ext_condition2.append_null();
        self.ext_condition3.append_null();
        self.ext_condition4.append_null();
    }

    fn append_null_ohlcvc(&mut self) {
        self.open.append_null();
        self.high.append_null();
        self.low.append_null();
        self.close.append_null();
        self.volume.append_null();
        self.count.append_null();
    }

    /// Finalise into a `RecordBatch`. Column order follows the schema
    /// returned by [`fpss_event_schema`] one-for-one.
    fn finish(mut self) -> PyResult<RecordBatch> {
        let columns: Vec<ArrayRef> = vec![
            Arc::new(self.kind.finish()),
            Arc::new(self.symbol.finish()),
            Arc::new(self.sec_type.finish()),
            Arc::new(self.expiration.finish()),
            Arc::new(self.right.finish()),
            Arc::new(self.strike.finish()),
            Arc::new(self.ms_of_day.finish()),
            Arc::new(self.date.finish()),
            Arc::new(self.received_at_ns.finish()),
            Arc::new(self.bid.finish()),
            Arc::new(self.ask.finish()),
            Arc::new(self.bid_size.finish()),
            Arc::new(self.ask_size.finish()),
            Arc::new(self.bid_exchange.finish()),
            Arc::new(self.ask_exchange.finish()),
            Arc::new(self.bid_condition.finish()),
            Arc::new(self.ask_condition.finish()),
            Arc::new(self.trade_price.finish()),
            Arc::new(self.trade_size.finish()),
            Arc::new(self.trade_exchange.finish()),
            Arc::new(self.trade_condition.finish()),
            Arc::new(self.sequence.finish()),
            Arc::new(self.condition_flags.finish()),
            Arc::new(self.price_flags.finish()),
            Arc::new(self.volume_type.finish()),
            Arc::new(self.records_back.finish()),
            Arc::new(self.ext_condition1.finish()),
            Arc::new(self.ext_condition2.finish()),
            Arc::new(self.ext_condition3.finish()),
            Arc::new(self.ext_condition4.finish()),
            Arc::new(self.open_interest.finish()),
            Arc::new(self.open.finish()),
            Arc::new(self.high.finish()),
            Arc::new(self.low.finish()),
            Arc::new(self.close.finish()),
            Arc::new(self.volume.finish()),
            Arc::new(self.count.finish()),
        ];
        RecordBatch::try_new(self.schema, columns)
            .map_err(|e| PyRuntimeError::new_err(format!("failed to build FPSS RecordBatch: {e}")))
    }
}

/// One `__anext__` step. Mirrors the per-tick session's `anext_step`
/// but yields a `pyarrow.RecordBatch` instead of a `list[FpssEvent]`.
async fn anext_step_batches(
    session_handle: Py<StreamingAsyncBatchesSession>,
) -> PyResult<Py<PyAny>> {
    loop {
        let drained = Python::attach(|py| -> PyResult<Option<(Py<PyAny>, bool, usize)>> {
            let session = session_handle.borrow(py);
            if session.closed {
                let empty = empty_record_batch(Arc::clone(&session.schema))?;
                let obj = batch_to_pyarrow(py, empty)?;
                return Ok(Some((obj, true, 0)));
            }
            let iterator = session.iterator.as_ref().ok_or_else(|| {
                PyRuntimeError::new_err(
                    "StreamingAsyncBatchesSession not entered -- call `async with session:` first",
                )
            })?;
            let wake = session.wake.as_ref().ok_or_else(|| {
                PyRuntimeError::new_err(
                    "StreamingAsyncBatchesSession wake handle missing -- enter the context manager first",
                )
            })?;
            wake.rearm();
            crate::streaming_async_session::drain_read_pipe(session.read_fd);
            let (batch, terminal) = drain_batch_arrow(Arc::clone(&session.schema), iterator)?;
            let rows = batch.num_rows();
            if rows > 0 || terminal {
                let obj = batch_to_pyarrow(py, batch)?;
                Ok(Some((obj, terminal, rows)))
            } else {
                Ok(None)
            }
        })?;

        if let Some((batch, terminal, rows)) = drained {
            if terminal && rows == 0 {
                return Err(PyStopAsyncIteration::new_err(()));
            }
            return Ok(batch);
        }

        let event_awaitable: Py<PyAny> = Python::attach(|py| -> PyResult<Py<PyAny>> {
            let session = session_handle.borrow(py);
            let event = session.asyncio_event.as_ref().ok_or_else(|| {
                PyRuntimeError::new_err(
                    "StreamingAsyncBatchesSession asyncio.Event missing -- \
                     enter the context manager first",
                )
            })?;
            let bound = event.bind(py);
            let coro = bound.call_method0("wait")?;
            Ok(coro.unbind())
        })?;

        let await_fut = Python::attach(|py| {
            pyo3_async_runtimes::tokio::into_future(event_awaitable.bind(py).clone())
        })?;
        let _ = await_fut.await?;

        Python::attach(|py| -> PyResult<()> {
            let session = session_handle.borrow(py);
            if let Some(event) = session.asyncio_event.as_ref() {
                event.bind(py).call_method0("clear")?;
            }
            Ok(())
        })?;
    }
}

// ── PyO3 surfaces: `streaming_async_batches()` on each client ──────

#[pymethods]
impl crate::ThetaDataDxClient {
    /// Open the FPSS connection in pull-iter mode with an asyncio FD
    /// wake-up signal, and return the
    /// [`StreamingAsyncBatchesSession`] context manager.
    ///
    /// Each `__anext__` drain yields one `pyarrow.RecordBatch` whose
    /// rows match every event delivered since the previous drain.
    ///
    /// ```python
    /// async with client.streaming_async_batches() as session:
    ///     await session.subscribe(Contract.stock("QQQ").quote())
    ///     async for batch in session:
    ///         df = batch.to_pandas()
    ///         # ... vectorised processing ...
    /// ```
    ///
    /// The Arrow schema is the union of every `FpssData` variant's
    /// fields plus a `kind` discriminator column — see
    /// [`StreamingAsyncBatchesSession::schema`] for the typed access.
    ///
    /// # Performance
    ///
    /// Per-event cost is one push per typed column builder (tens of
    /// nanoseconds total) plus zero PyObject allocation — the
    /// per-tick [`crate::streaming_async_session::StreamingAsyncSession`]
    /// pays one full pyclass instance allocation per event, which on
    /// dense streams (≥ 5 k events/sec QQQ quotes, ≥ 100 k events/sec
    /// full-stream OPRA option quotes) dominates wall time. Closes
    /// #562.
    ///
    /// # Backpressure
    ///
    /// `max_queue_depth` and `backpressure` mirror
    /// [`Self::streaming_async`] — see there for the
    /// `BackpressurePolicy` axis. Default `Block` preserves every
    /// event at the cost of upstream pressure into the TLS reader.
    #[pyo3(signature = (*, max_queue_depth = DEFAULT_MAX_QUEUE_DEPTH, backpressure = BackpressurePolicy::Block))]
    fn streaming_async_batches(
        slf: Py<Self>,
        py: Python<'_>,
        max_queue_depth: usize,
        backpressure: BackpressurePolicy,
    ) -> PyResult<Py<StreamingAsyncBatchesSession>> {
        Py::new(
            py,
            StreamingAsyncBatchesSession::from_tdx(slf, max_queue_depth, backpressure),
        )
    }
}

#[pymethods]
impl crate::fpss_client::FpssClient {
    /// Open the FPSS connection in pull-iter mode with an asyncio FD
    /// wake-up signal, and return the
    /// [`StreamingAsyncBatchesSession`] context manager bound to the
    /// standalone FPSS client.
    ///
    /// Same surface as [`crate::ThetaDataDxClient::streaming_async_batches`]
    /// but opens NO MDDS / Nexus surface, matching the deferred-
    /// connect contract of the rest of the standalone FPSS client.
    #[pyo3(signature = (*, max_queue_depth = DEFAULT_MAX_QUEUE_DEPTH, backpressure = BackpressurePolicy::Block))]
    fn streaming_async_batches(
        slf: Py<Self>,
        py: Python<'_>,
        max_queue_depth: usize,
        backpressure: BackpressurePolicy,
    ) -> PyResult<Py<StreamingAsyncBatchesSession>> {
        Py::new(
            py,
            StreamingAsyncBatchesSession::from_fpss(slf, max_queue_depth, backpressure),
        )
    }
}
