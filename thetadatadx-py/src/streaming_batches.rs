//! Hand-written support for the pull-based Arrow `RecordBatch` reader.
//!
//! The generated `StreamView.batches(..)` entry (from `sdk_surface.toml`)
//! constructs the [`RecordBatchStream`] pyclass defined here. The reader is
//! the columnar sibling of the per-event callback: one object that is both a
//! synchronous `Iterable` and an `AsyncIterable`, doubles as a sync/async
//! context manager that closes the stream on exit, and yields
//! `pyarrow.RecordBatch` values exported zero-copy over the Arrow C Data
//! Interface.
//!
//! The reader pyclass is hand-written (like `StreamView` itself) because its
//! protocol surface — `__iter__` / `__next__` / `__aiter__` / `__anext__` /
//! `__enter__` / `__exit__` / `__aenter__` / `__aexit__` plus the `schema` /
//! `dropped` properties — is intrinsic Python protocol shape, not a
//! per-endpoint projection. The generator still owns the entry method so the
//! cross-binding surface stays in lockstep.

use std::sync::Arc;

use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration, PyStopIteration};
use pyo3::prelude::*;

use thetadatadx::streaming::{Backpressure, RecordBatchStream as CoreRecordBatchStream};

use crate::to_py_err;

/// Map the binding's optional `backpressure` string to the core enum.
///
/// `"block"` (default, lossless) or `"drop_oldest"` (bounded buffer; needs a
/// `capacity`). Case-insensitive. Any other value is a `ValueError`.
fn parse_backpressure(kind: Option<&str>, capacity: Option<usize>) -> PyResult<Backpressure> {
    match kind.map(str::to_ascii_lowercase).as_deref() {
        None | Some("block") => Ok(Backpressure::Block),
        Some("drop_oldest") => Ok(Backpressure::DropOldest {
            capacity: capacity.unwrap_or(4).max(1),
        }),
        Some(other) => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unknown backpressure {other:?}; expected \"block\" or \"drop_oldest\""
        ))),
    }
}

/// Open a [`RecordBatchStream`] over the unified client's stream.
///
/// Called by the generated `StreamView.batches(..)` entry. Starts streaming with
/// a batching dispatcher and returns the reader pyclass.
pub(crate) fn open_reader(
    py: Python<'_>,
    client: &Arc<thetadatadx::Client>,
    batch_size: Option<usize>,
    linger_ms: Option<u64>,
    backpressure: Option<&str>,
    capacity: Option<usize>,
) -> PyResult<RecordBatchStream> {
    let backpressure = parse_backpressure(backpressure, capacity)?;
    // Bind the `StreamSurface` so the borrowed `BatchReaderBuilder` outlives
    // the chained configuration calls.
    let stream_surface = client.stream();
    let mut builder = stream_surface.batches();
    if let Some(rows) = batch_size {
        builder = builder.batch_size(rows);
    }
    if let Some(ms) = linger_ms {
        builder = builder.linger(std::time::Duration::from_millis(ms));
    }
    builder = builder.backpressure(backpressure);
    // Never hold the GIL across the blocking streaming connect the builder
    // performs; a sibling Python thread keeps running while the handshake is
    // in flight. The build path and its `Result` are pure Rust — no Python
    // object is touched inside the detached region.
    let stream = py.detach(|| builder.build()).map_err(to_py_err)?;
    Ok(RecordBatchStream {
        inner: Arc::new(stream),
    })
}

/// A pull reader of `pyarrow.RecordBatch` values off the live streaming session.
///
/// Both a synchronous `Iterable` (the blocking `__next__` releases the GIL
/// so other Python threads run while it waits) and an `AsyncIterable`
/// (`__anext__` awaits the next batch on a worker thread). Also a sync and
/// async context manager that closes the stream — unsubscribing and tearing
/// the session down — on exit. Yields columnar batches under a fixed schema
/// (see `schema`); concatenate them freely.
///
/// The core [`RecordBatchStream`] is held behind a bare `Arc`: its own
/// methods take `&self` (the internal queue lock is released across the
/// blocking wait), so `close()` / `__exit__` can signal shutdown via
/// [`CoreRecordBatchStream::close_shared`] CONCURRENTLY with a blocking pull
/// in flight on another thread — there is no handle-level lock for the two
/// to contend on, so close can never deadlock against an in-flight pull.
/// After close, the pull unblocks and every subsequent pull returns
/// end-of-iteration. The session tears down here when the last `Arc`
/// reference drops (the core `Drop` is idempotent with the `close_shared`
/// signal).
#[pyclass]
pub struct RecordBatchStream {
    inner: Arc<CoreRecordBatchStream>,
}

#[pymethods]
impl RecordBatchStream {
    /// Iterable protocol: the reader is its own iterator.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Blocking pull of the next `pyarrow.RecordBatch`. Releases the GIL
    /// across the wait so other Python threads keep running, raising
    /// `StopIteration` at end of stream.
    fn __next__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let inner = Arc::clone(&self.inner);
        // GIL released for the blocking ring wait; re-acquired only to build
        // the pyarrow object after a batch lands. This is the no-GIL
        // discipline on the blocking pull. The core stream releases its own
        // queue lock across the wait, so a concurrent `close()` is honored
        // promptly.
        let batch = py
            .detach(|| inner.next_blocking())
            .map_err(|e| to_py_err(thetadatadx::Error::from(e)))?;
        match batch {
            Some(batch) => record_batch_to_pyarrow(py, batch),
            None => Err(PyStopIteration::new_err(())),
        }
    }

    /// AsyncIterable protocol: the reader is its own async iterator.
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Await the next `pyarrow.RecordBatch`. The blocking pull runs on a
    /// blocking-pool thread (it never holds the GIL); the awaitable resolves
    /// to the batch or raises `StopAsyncIteration` at end of stream.
    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = Arc::clone(&self.inner);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            // The pull is blocking; run it on the blocking pool so the async
            // executor thread is never parked. No GIL is held here.
            let pulled = tokio::task::spawn_blocking(move || inner.next_blocking())
                .await
                .map_err(|e| PyRuntimeError::new_err(format!("batch pull task failed: {e}")))?
                .map_err(|e| to_py_err(thetadatadx::Error::from(e)))?;
            match pulled {
                Some(batch) => Python::attach(|py| record_batch_to_pyarrow(py, batch)),
                None => Err(PyStopAsyncIteration::new_err(())),
            }
        })
        .map(pyo3::Bound::into_any)
    }

    /// Sync context manager entry: returns the reader.
    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    /// Sync context manager exit: close the stream (unsubscribe + tear down).
    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: Option<Py<PyAny>>,
        _exc_value: Option<Py<PyAny>>,
        _traceback: Option<Py<PyAny>>,
    ) -> bool {
        self.close(py);
        false
    }

    /// Async context manager entry: returns the reader.
    fn __aenter__<'py>(slf: Py<Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(slf) })
            .map(pyo3::Bound::into_any)
    }

    /// Async context manager exit: close the stream.
    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: Option<Py<PyAny>>,
        _exc_value: Option<Py<PyAny>>,
        _traceback: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.close(py);
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(false) })
            .map(pyo3::Bound::into_any)
    }

    /// Close the stream: unsubscribe and tear the streaming session down.
    /// Idempotent; further pulls return end-of-iteration.
    fn close(&self, py: Python<'_>) {
        // Signal shutdown through the core's shared-reference close, with the
        // GIL released: it shuts the client (which detaches its own join),
        // wakes any in-flight blocking pull on another thread, and lets the
        // dispatcher exit. No handle-level lock is taken, so a concurrent
        // `__next__` / `__anext__` pull is unblocked rather than deadlocked.
        // Releasing the GIL keeps the dispatcher's per-batch `Python::attach`
        // from contending with this call.
        let inner = Arc::clone(&self.inner);
        py.detach(move || inner.close_shared());
    }

    /// The fixed Arrow schema every yielded batch carries, as a
    /// `pyarrow.Schema`.
    #[getter]
    fn schema(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let schema = self.inner.schema();
        schema_to_pyarrow(py, &schema)
    }

    /// Number of batches dropped so far under the `drop_oldest` backpressure
    /// policy. Always `0` under `block` (the default).
    #[getter]
    fn dropped(&self) -> u64 {
        self.inner.dropped()
    }
}

/// Export one [`arrow::array::RecordBatch`] to a `pyarrow.RecordBatch`,
/// zero-copy, over the Arrow C Data Interface.
///
/// Imports the batch through `pyarrow.RecordBatchReader._import_from_c` (the
/// same version-neutral C Stream Interface path the historical `to_arrow`
/// terminal uses) and pulls the single batch back out with
/// `read_next_batch`, so the result is a `pyarrow.RecordBatch` rather than a
/// `Table`.
fn record_batch_to_pyarrow(
    py: Python<'_>,
    batch: arrow::array::RecordBatch,
) -> PyResult<Py<PyAny>> {
    use arrow::ffi_stream::FFI_ArrowArrayStream;
    use arrow::record_batch::RecordBatchIterator;

    let schema = batch.schema();
    let reader = RecordBatchIterator::new(std::iter::once(Ok(batch)), schema);
    // Stack-owned stream handed to pyarrow by address; `_import_from_c`
    // takes ownership of the contents and nulls the release callback, so the
    // `Drop` when this returns is a no-op on the moved-out handle.
    let mut stream = FFI_ArrowArrayStream::new(Box::new(reader));
    let stream_addr = std::ptr::addr_of_mut!(stream) as usize;

    let pyarrow = py.import("pyarrow")?;
    let reader_obj = pyarrow
        .getattr("RecordBatchReader")?
        .call_method1("_import_from_c", (stream_addr,))?;
    let batch_obj = reader_obj.call_method0("read_next_batch")?;
    Ok(batch_obj.unbind())
}

/// Export the fixed schema to a `pyarrow.Schema` via an empty
/// `RecordBatchReader` import.
fn schema_to_pyarrow(
    py: Python<'_>,
    schema: &Arc<arrow::datatypes::Schema>,
) -> PyResult<Py<PyAny>> {
    use arrow::ffi_stream::FFI_ArrowArrayStream;
    use arrow::record_batch::RecordBatchIterator;

    let reader = RecordBatchIterator::new(std::iter::empty(), Arc::clone(schema));
    let mut stream = FFI_ArrowArrayStream::new(Box::new(reader));
    let stream_addr = std::ptr::addr_of_mut!(stream) as usize;

    let pyarrow = py.import("pyarrow")?;
    let reader_obj = pyarrow
        .getattr("RecordBatchReader")?
        .call_method1("_import_from_c", (stream_addr,))?;
    let schema_obj = reader_obj.getattr("schema")?;
    Ok(schema_obj.unbind())
}
