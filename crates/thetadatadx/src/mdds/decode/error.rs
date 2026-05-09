//! Per-cell decode errors and `DataType` variant naming for diagnostics.
//!
//! Schema-drift guards in the generated parsers raise
//! [`DecodeError::MissingRequiredHeader`] when an upstream column is
//! absent, and the streaming accumulator raises
//! [`DecodeError::ChunkHeaderDrift`] when a mid-stream chunk's header set
//! diverges from the first chunk's schema.
//!
//! Behaviour mirrors the upstream Java terminal.

use crate::proto;
use thiserror::Error as ThisError;

/// Per-cell decode failure. Produced by the `row_*` helpers when a cell does
/// not match the column's declared type, or when the requested column index is
/// past the end of the row.
#[derive(Debug, ThisError, PartialEq, Eq)]
pub enum DecodeError {
    /// Cell exists but its `DataType` variant does not match the declared
    /// schema for this column.
    #[error("column {column}: expected {expected}, got {observed}")]
    TypeMismatch {
        column: usize,
        expected: &'static str,
        observed: &'static str,
    },
    /// Row has fewer cells than the requested column index.
    #[error("column {column}: missing cell")]
    MissingCell { column: usize },
    /// A required header (declared in `tick_schema.toml` under
    /// `required = [...]`) is absent from a non-empty `DataTable`. Emitted by
    /// the generated parsers when the server has added or renamed the column —
    /// surfacing this as an error is the only way to prevent silent data loss
    /// when the upstream schema drifts (see `HEADER_ALIASES` for known
    /// synonyms). Empty `DataTable`s (no rows) still return `Ok(vec![])`
    /// because "no trades today" is a legitimate outcome.
    #[error(
        "required column `{header}` missing from {rows}-row DataTable; \
         available headers: {available}"
    )]
    MissingRequiredHeader {
        header: &'static str,
        rows: usize,
        available: String,
    },
    /// A mid-stream gRPC chunk carries a header set that does not match the
    /// header set established by the first chunk. The stream accumulator
    /// used to silently retain the first header set and accumulate rows
    /// from every chunk underneath it, which would transparently corrupt
    /// a row set if the server's wire schema changed mid-response. This
    /// variant surfaces the drift instead of hiding it.
    #[error(
        "chunk {chunk_index} headers drifted from first-chunk schema; \
         first: [{first}]; chunk: [{chunk}]"
    )]
    ChunkHeaderDrift {
        chunk_index: usize,
        first: String,
        chunk: String,
    },
}

/// Name the `DataType` variant for error messages. `None` is treated as a
/// missing `data_type` oneof (protobuf cell with no variant set).
pub(crate) fn observed_name(dt: Option<&proto::data_value::DataType>) -> &'static str {
    match dt {
        Some(proto::data_value::DataType::Number(_)) => "Number",
        Some(proto::data_value::DataType::Text(_)) => "Text",
        Some(proto::data_value::DataType::Price(_)) => "Price",
        Some(proto::data_value::DataType::Timestamp(_)) => "Timestamp",
        Some(proto::data_value::DataType::NullValue(_)) => "NullValue",
        None => "Unset",
    }
}
