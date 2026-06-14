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
    /// A cell's wire type fell outside the accept-set its schema column
    /// declares. The bulk column extractors (`decode::column`) raise this
    /// in place of [`DecodeError::TypeMismatch`]: the schema column name
    /// and the offending row index make a corrupt cell in a 100K-row
    /// frame locatable from the error alone.
    #[error("column `{header}` (index {column}, row {row}): expected {expected}, got {observed}")]
    ColumnTypeMismatch {
        /// Schema-side column name (as declared in `tick_schema.toml`).
        header: &'static str,
        /// Resolved wire column index.
        column: usize,
        /// Zero-based row index within the decoded `DataTable`.
        row: usize,
        /// Accept-set the schema declares for this column.
        expected: &'static str,
        /// `DataType` variant observed on the wire.
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
    /// header set established by the first chunk. Retaining the first chunk's
    /// header set while accumulating rows from a diverging chunk would
    /// transparently corrupt the row set, so the accumulator surfaces the
    /// drift as this error rather than hiding it.
    #[error(
        "chunk {chunk_index} headers drifted from first-chunk schema; \
         first: [{first}]; chunk: [{chunk}]"
    )]
    ChunkHeaderDrift {
        chunk_index: usize,
        first: String,
        chunk: String,
    },
    /// A `Text` cell in a date-typed column did not match the documented
    /// ISO `YYYY-MM-DD` or compact `YYYYMMDD` shapes. The v3 wire path
    /// publishes some date columns (notably `interest_rate_history_eod.created`,
    /// `calendar_day.date`, and `OptionContract.expiration`) as text.
    /// Surfacing a malformed value as an error rather than coalescing it to
    /// `0` mirrors the strict-decode policy on every other column type and
    /// prevents silent corruption of downstream timestamps.
    #[error("invalid date text {raw:?} (expected YYYY-MM-DD or YYYYMMDD)")]
    InvalidDate {
        /// The exact text that failed to parse, captured verbatim from
        /// the wire for diagnostics.
        raw: String,
    },
    /// A `Text` cell in a time-typed column did not match the documented
    /// `HH:MM:SS` shape. Used on the v3 calendar `open` / `close`
    /// columns. Surfacing a malformed value as an error rather than
    /// coalescing it to `0` prevents silent corruption of trading-session
    /// timestamps in downstream consumers.
    #[error("invalid time text {raw:?} (expected HH:MM:SS)")]
    InvalidTime {
        /// The exact text that failed to parse, captured verbatim from
        /// the wire for diagnostics.
        raw: String,
    },
    /// A `Text` cell in an enum-typed column carried a value outside the
    /// documented vendor vocabulary. Used on the v3 `right` (option
    /// CALL/PUT) and calendar `type` (open / early_close / full_close /
    /// weekend) columns. Surfacing an unknown variant as an error rather
    /// than folding it to a default (`0` for `right`, an unknown-status
    /// sentinel for calendar) keeps upstream schema drift visible and
    /// matches the strict-decode policy on every other typed column.
    #[error("unknown enum variant {raw:?} on field `{field}`")]
    UnknownEnumVariant {
        /// Static name of the wire column (`right`, `calendar.type`,
        /// etc.) so the error is greppable in operator logs.
        field: &'static str,
        /// The exact text that failed to map to a known variant,
        /// captured verbatim from the wire for diagnostics.
        raw: String,
    },
    /// A `Price` cell carried a `price_type` outside
    /// `0..=crate::tdbe::types::price::MAX_PRICE_TYPE`. Mirrors
    /// `crate::tdbe::types::price::PriceError::PriceTypeOutOfRange` on the
    /// decode boundary.
    #[error("invalid price_type {raw} (expected 0..=19)")]
    InvalidPriceType {
        /// The wire `price_type` value, captured verbatim.
        raw: i32,
    },
    /// A wire `Number` cell carried an `int64` value outside the
    /// `i32` range expected by the destination field.
    #[error("integer overflow: int64 {raw} does not fit i32")]
    NumericOverflow {
        /// The wire `int64` value, captured verbatim.
        raw: String,
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
