//! Hand-written napi support for the pull-based Arrow `RecordBatch` reader.
//!
//! The generated `StreamView.batches(..)` entry constructs the
//! [`RecordBatchStreamHandle`] napi class defined here. The handle crosses
//! the napi boundary as Arrow IPC byte buffers — the same columnar wire
//! format the SDK's `tradeTickToArrowIpc` export uses — and the package's
//! JS wrapper (`streaming-session.js`) decodes each buffer with apache-arrow
//! `tableFromIPC` and presents the TC39 `AsyncIterable<RecordBatch>` plus
//! `Symbol.asyncDispose` surface.
//!
//! Splitting the IPC transport (Rust napi) from the apache-arrow decode and
//! the async-iteration protocol (JS wrapper) keeps the napi surface minimal
//! and the `RecordBatch` type the user sees the native apache-arrow one.

use std::sync::Arc;

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use thetadatadx::streaming::{Backpressure, RecordBatchStream};

use crate::to_napi_err;

/// Validate a tuning value the caller passed as a JS `number`, rejecting a
/// negative the way the Python binding's `usize` parameters do (a JS `-1` would
/// otherwise coerce silently to a huge unsigned value). A non-negative value is
/// returned as `usize`; the core upper-clamps it (see
/// `thetadatadx::streaming::MAX_BATCH_SIZE` / `MAX_QUEUE_DEPTH`), so no upper
/// check is needed here. Keeps TS and Python input handling consistent:
/// negative is an error in both, oversized is clamped in both.
fn checked_tuning(value: i64, field: &str) -> napi::Result<usize> {
    if value < 0 {
        return Err(napi::Error::from_reason(format!(
            "{field} must be non-negative, got {value}"
        )));
    }
    Ok(value as usize)
}

/// Map the optional `backpressure` string to the core enum. `"block"`
/// (default) or `"dropOldest"` / `"drop_oldest"` (needs a `capacity`).
fn parse_backpressure(kind: Option<String>, capacity: Option<i64>) -> napi::Result<Backpressure> {
    match kind.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("block") => Ok(Backpressure::Block),
        Some("dropoldest") | Some("drop_oldest") => {
            let capacity = match capacity {
                Some(c) => checked_tuning(c, "capacity")?.max(1),
                None => 4,
            };
            Ok(Backpressure::DropOldest { capacity })
        }
        Some(other) => Err(napi::Error::from_reason(format!(
            "unknown backpressure {other:?}; expected \"block\" or \"dropOldest\""
        ))),
    }
}

/// Serialise one [`arrow_array::RecordBatch`] as an Arrow IPC stream byte
/// buffer (the same path the per-tick `*ToArrowIpc` exports use).
///
/// Declared ahead of the `#[napi]` items below so the parity gate's
/// free-function scan (which keys off a preceding `#[napi]` attribute) never
/// mistakes this internal helper for a cross-binding utility.
fn batch_to_ipc(batch: &arrow_array::RecordBatch) -> napi::Result<Vec<u8>> {
    // Seed from an estimate keyed on the row COUNT, so the IPC body is written
    // without re-growing the Vec from empty. Sizing from
    // `get_array_memory_size()` would seed from the builder's preallocated
    // column capacity (now batch-size-wide), so a one-row linger-flushed batch
    // would over-allocate by orders of magnitude; `estimated_ipc_len` keys on
    // the used rows instead. The same estimate is used in the FFI encoder.
    let mut buf: Vec<u8> =
        Vec::with_capacity(thetadatadx::streaming::estimated_ipc_len(batch.num_rows()));
    {
        let mut writer = arrow_ipc::writer::StreamWriter::try_new(
            std::io::Cursor::new(&mut buf),
            &batch.schema(),
        )
        .map_err(|e| napi::Error::from_reason(format!("arrow ipc writer init failed: {e}")))?;
        writer
            .write(batch)
            .map_err(|e| napi::Error::from_reason(format!("arrow ipc write failed: {e}")))?;
        writer
            .finish()
            .map_err(|e| napi::Error::from_reason(format!("arrow ipc finish failed: {e}")))?;
    }
    Ok(buf)
}

/// Serialise the fixed schema as a schema-only Arrow IPC stream.
fn schema_to_ipc(schema: &Arc<arrow_schema::Schema>) -> napi::Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let writer = arrow_ipc::writer::StreamWriter::try_new(
            std::io::Cursor::new(&mut buf),
            schema.as_ref(),
        )
        .map_err(|e| {
            napi::Error::from_reason(format!("arrow ipc schema writer init failed: {e}"))
        })?;
        writer.into_inner().map_err(|e| {
            napi::Error::from_reason(format!("arrow ipc schema finish failed: {e}"))
        })?;
    }
    Ok(buf)
}

/// Tuning knobs for `StreamView.batches(options?)`. Each field maps to a
/// builder setter; `None` keeps the production default. napi renders this as
/// a TypeScript object `{ batchSize?, lingerMs?, backpressure?, capacity? }`,
/// which is the documented call form. The prior positional parameters threw a
/// coercion error when a caller passed that documented options object.
#[napi(object)]
#[derive(Default)]
pub struct BatchesOptions {
    pub batch_size: Option<i64>,
    pub linger_ms: Option<i64>,
    pub backpressure: Option<String>,
    pub capacity: Option<i64>,
}

/// Open a [`RecordBatchStreamHandle`] over the unified client's stream.
///
/// Called by the generated `StreamView.batches(..)` entry. The connect runs
/// on a blocking worker so the Node event loop is never frozen.
pub(crate) async fn open_handle(
    client: Arc<thetadatadx::Client>,
    batch_size: Option<i64>,
    linger_ms: Option<i64>,
    backpressure: Option<String>,
    capacity: Option<i64>,
) -> napi::Result<RecordBatchStreamHandle> {
    let backpressure = parse_backpressure(backpressure, capacity)?;
    // Validate the numeric knobs at the boundary, rejecting negatives the way
    // the Python binding does, before handing off to the blocking worker. The
    // core upper-clamps the magnitude, so only the sign is checked here.
    let batch_size = batch_size
        .map(|v| checked_tuning(v, "batchSize"))
        .transpose()?;
    let linger_ms = linger_ms
        .map(|v| checked_tuning(v, "lingerMs"))
        .transpose()?;
    let stream = tokio::task::spawn_blocking(move || {
        // Bind the `StreamSurface` so the borrowed `BatchReaderBuilder`
        // outlives the chained configuration calls.
        let stream_surface = client.stream();
        let mut builder = stream_surface.batches();
        if let Some(rows) = batch_size {
            builder = builder.batch_size(rows);
        }
        if let Some(ms) = linger_ms {
            builder = builder.linger(std::time::Duration::from_millis(ms as u64));
        }
        builder.backpressure(backpressure).build()
    })
    .await
    .map_err(|e| napi::Error::from_reason(format!("batches open task failed: {e}")))?
    .map_err(to_napi_err)?;

    Ok(RecordBatchStreamHandle {
        inner: Arc::new(stream),
    })
}

/// napi handle to a live pull-based Arrow `RecordBatch` reader.
///
/// Yields each batch as an Arrow IPC `Buffer` from [`Self::next_ipc`]; the
/// JS wrapper decodes it with apache-arrow. The core [`RecordBatchStream`]
/// is held behind a bare `Arc`: its methods take `&self` (the internal queue
/// lock is released across the blocking wait), so [`Self::close`] can signal
/// shutdown via [`RecordBatchStream::close_shared`] CONCURRENTLY with a
/// blocking pull in flight on a worker thread — no handle-level lock for the
/// two to contend on, so close never deadlocks against an in-flight pull.
/// The session tears down when the last `Arc` reference drops (the core
/// `Drop` is idempotent with the `close_shared` signal).
#[napi]
pub struct RecordBatchStreamHandle {
    inner: Arc<RecordBatchStream>,
}

#[napi]
impl RecordBatchStreamHandle {
    /// Await the next batch as an Arrow IPC `Buffer`, or `null` at clean end
    /// of stream (or after close). The pull runs off the Node event loop, so
    /// it never blocks the main thread. Internal transport for the
    /// `RecordBatchStream` wrapper; consumers iterate the wrapper instead.
    #[napi(js_name = "nextIpc")]
    pub async fn next_ipc(&self) -> napi::Result<Option<Buffer>> {
        let inner = Arc::clone(&self.inner);
        let pulled = tokio::task::spawn_blocking(move || inner.next_blocking())
            .await
            .map_err(|e| napi::Error::from_reason(format!("batch pull task failed: {e}")))?
            .map_err(|e| to_napi_err(thetadatadx::Error::from(e)))?;

        match pulled {
            Some(batch) => Ok(Some(Buffer::from(batch_to_ipc(&batch)?))),
            None => Ok(None),
        }
    }

    /// The fixed schema as a schema-only Arrow IPC `Buffer`, so the JS
    /// wrapper can expose `.schema` before the first batch arrives.
    #[napi(js_name = "schemaIpc")]
    pub fn schema_ipc(&self) -> napi::Result<Buffer> {
        let schema = self.inner.schema();
        Ok(Buffer::from(schema_to_ipc(&schema)?))
    }

    /// Number of batches dropped so far under the `dropOldest` backpressure
    /// policy. Always `0` under `block` (the default).
    #[napi(js_name = "dropped", getter)]
    pub fn dropped(&self) -> i64 {
        i64::try_from(self.inner.dropped()).unwrap_or(i64::MAX)
    }

    /// Close the stream: unsubscribe and tear the FPSS session down.
    /// Idempotent; subsequent pulls return `null`.
    #[napi]
    pub fn close(&self) {
        // Signal shutdown through the core's shared-reference close: it shuts
        // the client (detaching its own join), wakes any in-flight pull on a
        // worker thread, and lets the dispatcher exit. No handle-level lock,
        // so a concurrent `nextIpc` pull is unblocked rather than deadlocked.
        self.inner.close_shared();
    }
}
