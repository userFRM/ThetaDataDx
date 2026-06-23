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
    // Seed the buffer from an estimate keyed on the row COUNT, so the IPC body
    // is written without re-growing the Vec from empty. Sizing from
    // `get_array_memory_size()` would seed from the builder's preallocated
    // column capacity (now batch-size-wide), so a one-row linger-flushed batch
    // would over-allocate by orders of magnitude; `estimated_ipc_len` keys on
    // the used rows instead.
    let mut buf: Vec<u8> =
        Vec::with_capacity(thetadatadx::streaming::estimated_ipc_len(batch.num_rows()));
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
    use arrow_array::{ArrayRef, Float64Array, Int32Array, StringArray};
    use arrow_schema::{DataType, Field};

    /// Build a representative multi-column batch of `rows` rows. The seed
    /// estimate is keyed on the row count and is schema-agnostic, so a few
    /// typed columns suffice to exercise the IPC body-vs-seed relationship.
    fn sample_batch(rows: usize) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("i", DataType::Int32, false),
            Field::new("f", DataType::Float64, false),
            Field::new("s", DataType::Utf8, false),
        ]));
        let ints = Int32Array::from((0..rows as i32).collect::<Vec<_>>());
        let floats = Float64Array::from((0..rows).map(|i| i as f64).collect::<Vec<_>>());
        let strings = StringArray::from((0..rows).map(|_| "SPY").collect::<Vec<_>>());
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(ints) as ArrayRef,
                Arc::new(floats) as ArrayRef,
                Arc::new(strings) as ArrayRef,
            ],
        )
        .expect("sample batch")
    }

    /// The IPC seed estimate keeps a tiny (linger-flushed) batch's buffer small
    /// and is large enough to hold a full batch's body without a doubling
    /// regrow. This is the concrete check that the over-allocation regression
    /// (seeding from buffer capacity, which is now batch-size-wide) is gone:
    /// the seed for one row is a few kilobytes, and for a full batch it is at
    /// least the actual serialized IPC length.
    #[test]
    fn ipc_seed_is_small_for_one_row_and_covers_a_full_batch() {
        // One row: the seed is the framing overhead plus a row, well under
        // 64 KiB (a buffer-capacity estimate would have reported megabytes).
        let one = thetadatadx::streaming::estimated_ipc_len(1);
        assert!(one < 64 * 1024, "one-row seed must be small, was {one}");

        // Full batch: the seed must be at least the real serialized body so the
        // writer never re-grows the Vec by doubling.
        let rows = 65_536;
        let body = batch_to_ipc(&sample_batch(rows)).expect("encode").len();
        let seed = thetadatadx::streaming::estimated_ipc_len(rows);
        assert!(
            seed >= body,
            "seed ({seed}) must cover the actual IPC body ({body}) so there is no regrow"
        );
    }
}
