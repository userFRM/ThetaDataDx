//! Per-cell strict decoders (`row_*`) and the generated parser surface.
//!
//! Each `row_*` function dispatches on the cell's wire type rather than
//! coalescing silently — wire-protocol anomalies (`DataValue` with the
//! `data_type` oneof unset) surface as
//! [`DecodeError::TypeMismatch { observed: "Unset" }`] rather than collapsing
//! to a default value.
//!
//! The macro-generated `decode_generated` module (assembled by `build.rs` from
//! `tick_schema.toml`) is included from this module; its emitted parser
//! functions reference `crate::decode::*` for the cross-cutting helpers and
//! `tdbe::time::*` for Eastern-time conversion.

use super::error::observed_name;
use super::error::DecodeError;
use super::headers::find_header;
use super::v3::parse_iso_date;
use crate::proto;
use tdbe::types::tick::{
    CalendarDay, EodTick, GreeksAllTick, GreeksFirstOrderTick, GreeksSecondOrderTick,
    GreeksThirdOrderTick, InterestRateTick, IvTick, MarketValueTick, OhlcTick, OpenInterestTick,
    OptionContract, PriceTick, QuoteTick, TradeQuoteTick, TradeTick,
};

/// Extract a date (YYYYMMDD) from a `Number` or `Timestamp` cell, strictly.
///
/// Used by generated parsers when the `date` field maps to a `timestamp` column.
/// `Number` carries the date already in YYYYMMDD form; `Timestamp` is converted
/// to an Eastern-Time YYYYMMDD integer. `NullValue` yields `Ok(None)`; any
/// other type yields `Err(TypeMismatch)`.
///
/// # Errors
///
/// Returns [`DecodeError::TypeMismatch`] if the cell is neither a `Number`,
/// `Timestamp`, nor `NullValue` — including the case where the `DataValue`
/// arrived with its `data_type` oneof unset (`observed: "Unset"`), which is a
/// wire-protocol anomaly we fail loud on. Returns [`DecodeError::MissingCell`]
/// only when the row has fewer cells than `idx` (index out of bounds).
// Reason: number values from protobuf fit in i32 for date/integer fields.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn row_date(row: &proto::DataValueList, idx: usize) -> Result<Option<i32>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(n)) => Ok(Some(*n as i32)),
        Some(proto::data_value::DataType::Timestamp(ts)) => {
            Ok(Some(tdbe::time::timestamp_to_date(ts.epoch_ms)))
        }
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Number|Timestamp",
            observed: observed_name(other),
        }),
    }
}

/// Decode an `i32`-valued cell with Java-matching strict semantics.
///
/// Accepts:
/// - `Number(n)` → `Ok(Some(n as i32))`.
/// - `Timestamp(ts)` → `Ok(Some(ms_of_day))` — v3 MDDS sends time columns as
///   proto `Timestamp`; the parser expects milliseconds-of-day in Eastern Time.
/// - `NullValue` → `Ok(None)`, matching Java `null` return.
///
/// Any other variant produces [`DecodeError::TypeMismatch`], including the
/// case where the `DataValue` arrived with its `data_type` oneof unset
/// (`observed: "Unset"`) — a wire anomaly we fail loud on. A row shorter than
/// `idx` (index out of bounds) produces [`DecodeError::MissingCell`].
///
/// # Errors
///
/// See variant list above.
// Reason: protocol-defined integer widths from Java FPSS specification.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn row_number(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<i32>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(n)) => Ok(Some(*n as i32)),
        Some(proto::data_value::DataType::Timestamp(ts)) => {
            Ok(Some(tdbe::time::timestamp_to_ms_of_day(ts.epoch_ms)))
        }
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Number|Timestamp",
            observed: observed_name(other),
        }),
    }
}

/// Extract raw price value from a `Price` cell (test-only helper).
///
/// `Price(p)` → `Ok(Some(p.value))`; `NullValue` → `Ok(None)`; other types
/// error. Missing cell errors.
///
/// # Errors
///
/// See [`row_number`].
#[cfg(test)]
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn row_price_value(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<i32>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Price(p)) => Ok(Some(p.value)),
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Price",
            observed: observed_name(other),
        }),
    }
}

/// Extract raw price type from a `Price` cell (test-only helper).
///
/// # Errors
///
/// See [`row_price_value`].
#[cfg(test)]
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn row_price_type(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<i32>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Price(p)) => Ok(Some(p.r#type)),
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Price",
            observed: observed_name(other),
        }),
    }
}

/// Decode a price-valued cell to `f64`, using the cell's own `price_type`.
///
/// Accepts both `Price` (the schema type) and `Number` — v3 MDDS occasionally
/// sends whole-dollar quantities as plain `Number` cells where the schema
/// would otherwise expect `Price`. `NullValue` returns `Ok(None)`.
///
/// # Errors
///
/// Errors on any other cell type or missing cell.
// Reason: protocol-defined integer widths from Java FPSS specification.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn row_price_f64(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<f64>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Price(p)) => Ok(Some(
            tdbe::types::price::Price::new(p.value, p.r#type).to_f64(),
        )),
        Some(proto::data_value::DataType::Number(n)) => Ok(Some(*n as f64)),
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Price|Number",
            observed: observed_name(other),
        }),
    }
}

/// Decode a text-valued cell.
///
/// `Text(s)` → `Ok(Some(s))`, `NullValue` → `Ok(None)`.
///
/// # Errors
///
/// Errors on any other cell type or missing cell.
pub(crate) fn row_text(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<String>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Text(s)) => Ok(Some(s.clone())),
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Text",
            observed: observed_name(other),
        }),
    }
}

/// Decode an `i64`-valued cell.
///
/// `Number(n)` → `Ok(Some(n))`; `Price(p)` → scaled with i64-native
/// arithmetic (no f64 hop), so values past `2^53` round-trip bit-exact;
/// `NullValue` → `Ok(None)`.
///
/// Used by the generated parsers for schema columns typed `i64` — added
/// with the EodTick `volume`/`count` widening (where on high-volume
/// symbols the values exceed `i32::MAX`).
///
/// `price_type` is clamped to `0..=19` to match
/// [`tdbe::types::price::Price::new`], so the same wire cell decodes
/// identically through this function and [`row_price_f64`].
///
/// # Errors
///
/// Returns `DecodeError::TypeMismatch` for any other cell variant. Returns
/// `DecodeError::MissingCell` for an out-of-bounds column index. Under the
/// clamped `0..=19` price-type contract, scale-up cannot overflow `i64`
/// (max product is `i32::MAX * 10^9 ≈ 2.15e18`, well under `i64::MAX`).
pub(crate) fn row_number_i64(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<i64>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(n)) => Ok(Some(*n)),
        Some(proto::data_value::DataType::Price(p)) => {
            // Vendor convention: real_value = value * 10^(type - 10).
            // Clamp `type` to 0..=19 to match `tdbe::Price::new`, so the
            // same wire cell decodes identically through `row_price_f64`
            // and `row_number_i64`. Positive exp scales up; negative exp
            // scales down. v == 0 short-circuits to 0 so a zero price
            // never trips the scale-up overflow guard.
            let v = i64::from(p.value);
            if v == 0 {
                return Ok(Some(0));
            }
            let price_type = p.r#type.clamp(0, 19);
            let exp = price_type - 10;
            // After clamping, exp ∈ [-10, 9]. Scale-up: i32::MAX * 10^9
            // ≈ 2.147e18 < i64::MAX (≈ 9.22e18), so checked_mul cannot
            // overflow. checked_mul preserves the contract anyway.
            let scaled = if exp >= 0 {
                10i64
                    .checked_pow(exp.unsigned_abs())
                    .and_then(|m| v.checked_mul(m))
            } else {
                Some(v / 10i64.pow(exp.unsigned_abs()))
            };
            match scaled {
                Some(n) => Ok(Some(n)),
                None => Err(DecodeError::TypeMismatch {
                    column: idx,
                    expected: "i64-fitting Price",
                    observed: "Price overflowing i64",
                }),
            }
        }
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Number|Price",
            observed: observed_name(other),
        }),
    }
}

/// Borrow the cell at `idx`, returning an error if the row is too short.
pub(crate) fn cell_type(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<&proto::data_value::DataType>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    Ok(dv.data_type.as_ref())
}

// Generated code -- parser functions from tick_schema.toml by build.rs.
//
// The emitted parser bodies reference:
//   * `crate::proto::*` for wire types
//   * `crate::decode::{observed_name, parse_iso_date, ...}` for shared helpers
//   * `tdbe::time::{timestamp_to_ms_of_day, timestamp_to_date}` for ET conversion
//
// All of these resolve through the re-exports in `crate::mdds::decode` (which
// `crate::decode` re-exports at the crate root) so the generator's path
// assumptions remain intact after the split.
#[allow(clippy::pedantic)] // Reason: auto-generated parser code, not under our control.
mod decode_generated {
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/decode_generated.rs"));
}
pub use decode_generated::*;
