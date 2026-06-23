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

/// Map the optional `backpressure` string to the core enum. `"block"`
/// (default) or `"dropOldest"` / `"drop_oldest"` (needs a `capacity`).
fn parse_backpressure(kind: Option<String>, capacity: Option<u32>) -> napi::Result<Backpressure> {
    match kind.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("block") => Ok(Backpressure::Block),
        Some("dropoldest") | Some("drop_oldest") => Ok(Backpressure::DropOldest {
            capacity: capacity.unwrap_or(4).max(1) as usize,
        }),
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
    let mut buf: Vec<u8> = Vec::new();
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
    pub batch_size: Option<u32>,
    pub linger_ms: Option<u32>,
    pub backpressure: Option<String>,
    pub capacity: Option<u32>,
}

/// Open a [`RecordBatchStreamHandle`] over the unified client's stream.
///
/// Called by the generated `StreamView.batches(..)` entry. The connect runs
/// on a blocking worker so the Node event loop is never frozen.
pub(crate) async fn open_handle(
    client: Arc<thetadatadx::Client>,
    batch_size: Option<u32>,
    linger_ms: Option<u32>,
    backpressure: Option<String>,
    capacity: Option<u32>,
) -> napi::Result<RecordBatchStreamHandle> {
    let backpressure = parse_backpressure(backpressure, capacity)?;
    let stream = tokio::task::spawn_blocking(move || {
        // Bind the `StreamSurface` so the borrowed `BatchReaderBuilder`
        // outlives the chained configuration calls.
        let stream_surface = client.stream();
        let mut builder = stream_surface.batches();
        if let Some(rows) = batch_size {
            builder = builder.batch_size(rows as usize);
        }
        if let Some(ms) = linger_ms {
            builder = builder.linger(std::time::Duration::from_millis(u64::from(ms)));
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
