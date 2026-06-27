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

/// Serialise a [`RecordBatch`] as an Arrow IPC stream byte buffer, seeding the
/// output `Vec`'s capacity with `cap` bytes.
///
/// The streaming reader passes `estimated_ipc_len(num_rows)` so the IPC body is
/// written without re-growing from empty; its seed is keyed on the row COUNT
/// rather than `get_array_memory_size()`, which would seed from the builder's
/// preallocated column capacity (now batch-size-wide) and over-allocate a
/// one-row linger-flushed batch by orders of magnitude. The per-tick and
/// flat-file terminals (whose schemas differ from the calibrated 40-column
/// streaming schema) pass `0` and let the writer grow the buffer.
///
/// # Errors
///
/// Returns a human-readable message on an IPC writer / write / finish
/// failure, surfaced to the caller through `thetadatadx_last_error()`.
pub(crate) fn batch_to_ipc(batch: &RecordBatch, cap: usize) -> Result<Vec<u8>, String> {
    let mut buf: Vec<u8> = Vec::with_capacity(cap);
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

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{ArrayRef, Float64Array, Int32Array, Int64Array, StringArray, UInt64Array};
    use arrow_schema::DataType;

    /// Build a batch of `rows` rows under the REAL fixed streaming schema (the
    /// same 40 columns the live reader emits), so the seed estimate is tested
    /// against a body whose per-row and framing sizes match what
    /// `estimated_ipc_len` is calibrated for, not an unrepresentative sample.
    /// Each field gets a column of its declared type; the values are arbitrary.
    fn streaming_batch(rows: usize) -> RecordBatch {
        let schema = thetadatadx::streaming::stream_batch_schema();
        let columns: Vec<ArrayRef> = schema
            .fields()
            .iter()
            .map(|field| -> ArrayRef {
                match field.data_type() {
                    DataType::Int32 => {
                        Arc::new(Int32Array::from((0..rows as i32).collect::<Vec<_>>()))
                    }
                    DataType::Int64 => {
                        Arc::new(Int64Array::from((0..rows as i64).collect::<Vec<_>>()))
                    }
                    DataType::UInt64 => {
                        Arc::new(UInt64Array::from((0..rows as u64).collect::<Vec<_>>()))
                    }
                    DataType::Float64 => Arc::new(Float64Array::from(
                        (0..rows).map(|i| i as f64).collect::<Vec<_>>(),
                    )),
                    DataType::Utf8 => Arc::new(StringArray::from(
                        (0..rows).map(|_| "SPY").collect::<Vec<_>>(),
                    )),
                    other => panic!("unexpected streaming-schema column type {other:?}"),
                }
            })
            .collect();
        RecordBatch::try_new(schema, columns).expect("streaming batch")
    }

    /// The IPC seed estimate keeps a tiny (linger-flushed) batch's buffer small
    /// AND covers the real serialized body without a doubling regrow, for both
    /// the smallest and a full batch. This is the concrete check that the
    /// over-allocation regression (seeding from buffer capacity, now
    /// batch-size-wide) is gone and that the framing allowance is large enough
    /// for the smallest batch (whose body is dominated by the 40-field schema
    /// preamble). Tested against the REAL 40-column schema so the per-row and
    /// framing figures are the ones being validated.
    #[test]
    fn ipc_seed_is_small_for_one_row_and_covers_real_batches() {
        // One row: the seed stays well under 64 KiB (a buffer-capacity estimate
        // would have reported megabytes) ...
        let one_seed = thetadatadx::streaming::estimated_ipc_len(1);
        assert!(
            one_seed < 64 * 1024,
            "one-row seed must be small, was {one_seed}"
        );
        // ... and still covers the real one-row IPC body (dominated by the
        // schema preamble), so even the smallest linger-flushed batch needs no
        // realloc.
        let one_body = batch_to_ipc(
            &streaming_batch(1),
            thetadatadx::streaming::estimated_ipc_len(1),
        )
        .expect("encode 1")
        .len();
        assert!(
            one_seed >= one_body,
            "one-row seed ({one_seed}) must cover the real one-row IPC body ({one_body})"
        );

        // Full batch: the seed must be at least the real serialized body so the
        // writer never re-grows the Vec by doubling.
        let rows = 65_536;
        let full_seed = thetadatadx::streaming::estimated_ipc_len(rows);
        let full_body = batch_to_ipc(&streaming_batch(rows), full_seed)
            .expect("encode full")
            .len();
        assert!(
            full_seed >= full_body,
            "full-batch seed ({full_seed}) must cover the real IPC body ({full_body})"
        );
    }
}
