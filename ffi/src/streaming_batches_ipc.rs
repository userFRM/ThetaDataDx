//! Arrow IPC serialisation helpers for the streaming `RecordBatch` reader's
//! C ABI.
//!
//! Each batch crosses the C boundary as an Arrow IPC stream, the same
//! columnar wire format the per-tick `thetadatadx_*_to_arrow_ipc` terminals
//! use, so the C++ SDK decodes a streaming batch with arrow-cpp's IPC reader
//! exactly as it already decodes a historical result.

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::Schema;

/// Serialise a [`RecordBatch`] as an Arrow IPC stream byte buffer.
///
/// # Errors
///
/// Returns a human-readable message on an IPC writer / write / finish
/// failure, surfaced to the caller through `thetadatadx_last_error()`.
pub(crate) fn batch_to_ipc(batch: &RecordBatch) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = StreamWriter::try_new(std::io::Cursor::new(&mut buf), &batch.schema())
            .map_err(|e| format!("arrow ipc writer init failed: {e}"))?;
        writer
            .write(batch)
            .map_err(|e| format!("arrow ipc write failed: {e}"))?;
        writer
            .finish()
            .map_err(|e| format!("arrow ipc finish failed: {e}"))?;
    }
    Ok(buf)
}

/// Serialise just a schema as a schema-only Arrow IPC stream (no batches),
/// so a reader can report its fixed schema before the first batch.
///
/// # Errors
///
/// Returns a human-readable message on an IPC writer / finish failure.
pub(crate) fn schema_to_ipc(schema: &Arc<Schema>) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let writer = StreamWriter::try_new(std::io::Cursor::new(&mut buf), schema.as_ref())
            .map_err(|e| format!("arrow ipc schema writer init failed: {e}"))?;
        // `try_new` writes the schema message; `finish` closes the stream
        // with the end-of-stream marker. No batch is written, so the result
        // is a valid schema-only stream.
        writer
            .into_inner()
            .map_err(|e| format!("arrow ipc schema finish failed: {e}"))?;
    }
    Ok(buf)
}
