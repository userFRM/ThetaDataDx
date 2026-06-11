//! Bulk column extraction for the generated `DataTable` parsers.
//!
//! The generated tick parsers decode in two phases:
//!
//! 1. **Layout resolution, once per table** — resolve every schema
//!    column to its wire index (`find_header` + the required-header
//!    guards), so no per-row work depends on header lookups or on
//!    whether an optional column is present.
//! 2. **Bulk extraction, once per column** — fill the output ticks
//!    column-by-column through [`extract_column`], monomorphized per
//!    (column type, tick field) pair at codegen time. Each column runs
//!    a tight loop whose cell decode is pinned to that column's
//!    accept-set, instead of re-selecting the decode shape cell by
//!    cell inside a row-shaped loop. Columns absent from the wire
//!    response cost nothing per row — the seed value stands.
//!
//! # Per-cell dispatch stays, per column
//!
//! The wire format types every cell individually, and upstream
//! genuinely mixes shapes within one column: `NullValue` interleaves
//! with data cells wherever the server has no value, price columns mix
//! `Price` and whole-dollar `Number` cells, and date columns arrive as
//! `Number`, `Timestamp`, or ISO `Text` depending on endpoint and
//! upstream version. A safe decoder therefore must inspect every
//! cell's tag; what the bulk path removes is the per-cell *selection*
//! of which accept-set applies (and the per-cell handling of absent
//! columns), not the tag check itself. Each extractor delegates to the
//! canonical single-cell decoder in [`super::cell`] — one strict
//! accept-set per column type, shared with every other decode surface.
//!
//! # Blocked iteration
//!
//! Tick rows are wider than a cache line, so naive whole-table column
//! passes evict each row between passes once the frame outgrows L2 —
//! measured 2.1x slower than the row-shaped decode on the transport
//! harness's large frame (130K rows x 8 columns). The generated
//! parsers instead run the column passes within [`BLOCK_ROWS`]-row
//! blocks: each block's rows and output ticks stay cache-resident
//! across all column passes. On the same frame the blocked shape
//! measured ~15% faster than the row-shaped decode (block sizes
//! 64-1024 within a few percent of each other; 256 best).

use super::error::DecodeError;
use crate::proto;

/// Rows per extraction block.
///
/// Sized so one block of wire rows plus its output ticks stays
/// L2-resident across every column pass: at the widest tick shape
/// (24 columns, ~32 B per cell, ~200 B per output tick) a 256-row
/// block is ~250 KiB. Measured best among 64..=4096 on the transport
/// harness's large-frame shape; see the module docs.
pub(crate) const BLOCK_ROWS: usize = 256;

/// Re-shape a single-cell decode error for the bulk path: a
/// [`DecodeError::TypeMismatch`] gains the schema column name and the
/// table-level row index ([`DecodeError::ColumnTypeMismatch`]); every
/// other error (`MissingCell`, `InvalidDate`, `NumericOverflow`, ...)
/// already carries its diagnostic payload and passes through verbatim.
fn attach_column_context(err: DecodeError, header: &'static str, row: usize) -> DecodeError {
    match err {
        DecodeError::TypeMismatch {
            column,
            expected,
            observed,
        } => DecodeError::ColumnTypeMismatch {
            header,
            column,
            row,
            expected,
            observed,
        },
        other => other,
    }
}

/// Decode one column across a block of rows into the matching tick
/// slots.
///
/// `cell` is the canonical single-cell decoder for the column's schema
/// type (one of the `row_*` functions in [`super::cell`]); `set`
/// writes the decoded value into its tick field. A `NullValue` cell —
/// `Ok(None)` from the cell decoder — falls back to `V::default()`,
/// matching the zero-fill the row-shaped decode applied per cell.
/// `row_base` is the index of `rows[0]` within the full table, so
/// diagnostics name the absolute row.
///
/// # Errors
///
/// Propagates the cell decoder's typed errors verbatim, except
/// [`DecodeError::TypeMismatch`], which is re-shaped to
/// [`DecodeError::ColumnTypeMismatch`] with the schema column name and
/// absolute row index attached.
pub(crate) fn extract_column<T, V: Default>(
    rows: &[proto::DataValueList],
    ticks: &mut [T],
    row_base: usize,
    column: usize,
    header: &'static str,
    cell: impl Fn(&proto::DataValueList, usize) -> Result<Option<V>, DecodeError>,
    set: impl Fn(&mut T, V),
) -> Result<(), DecodeError> {
    for (offset, (row, tick)) in rows.iter().zip(ticks.iter_mut()).enumerate() {
        let value = cell(row, column)
            .map_err(|err| attach_column_context(err, header, row_base + offset))?
            .unwrap_or_default();
        set(tick, value);
    }
    Ok(())
}
