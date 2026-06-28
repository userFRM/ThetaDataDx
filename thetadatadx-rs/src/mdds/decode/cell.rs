//! Per-cell strict decoders (`row_*`) and the generated parser surface.
//!
//! Each `row_*` function dispatches on the cell's wire type rather than
//! coalescing silently — wire-protocol anomalies (`DataValue` with the
//! `data_type` oneof unset) surface as
//! [`DecodeError::TypeMismatch { observed: "Unset" }`] rather than collapsing
//! to a default value.
//!
//! These are the canonical single-cell decoders: the bulk column
//! extraction in `super::column` drives them per column for the
//! generated parsers, and the hand-written v3 parsers
//! (`super::dual_type_columns`) call them per cell.
//!
//! The macro-generated `decode_generated` module (assembled by `build.rs` from
//! `tick_schema.toml`) is included from this module; its emitted parser
//! functions reference `crate::decode::*` for the cross-cutting helpers.

use super::dual_type_columns::parse_iso_date;
use super::error::observed_name;
use super::error::DecodeError;
use super::headers::find_header;
use crate::proto;
use crate::tdbe::types::tick::{
    CalendarDay, EodTick, GreeksAllTick, GreeksEodTick, GreeksFirstOrderTick,
    GreeksSecondOrderTick, GreeksThirdOrderTick, IndexPriceAtTimeTick, InterestRateTick, IvTick,
    MarketValueTick, OhlcTick, OpenInterestTick, OptionContract, PriceTick, QuoteTick,
    TradeGreeksAllTick, TradeGreeksFirstOrderTick, TradeGreeksImpliedVolatilityTick,
    TradeGreeksSecondOrderTick, TradeGreeksThirdOrderTick, TradeQuoteTick, TradeTick,
};

/// Extract a date (YYYYMMDD) from a `Number`, `Timestamp`, or `Text` cell,
/// strictly.
///
/// Used by generated parsers when the schema declares a `date` field. The
/// v3 MDDS server is consistent about Number/Timestamp for most date columns
/// but the `interest_rate/history/eod` endpoint emits an ISO `"YYYY-MM-DD"`
/// string under the header `created`; accepting `Text` here keeps every
/// `date`-typed parser tolerant of either wire shape with no per-parser
/// branching. `Number` carries the date already in YYYYMMDD form and is
/// validated via [`crate::tdbe::time::is_valid_yyyymmdd`]; `Timestamp` is
/// converted to an Eastern-Time YYYYMMDD integer; `Text` flows through
/// [`parse_iso_date`]. `NullValue` yields `Ok(None)`; any other type
/// yields `Err(TypeMismatch)`.
///
/// # Errors
///
/// Returns [`DecodeError::TypeMismatch`] when the cell is not one of
/// `Number`, `Timestamp`, `Text`, or `NullValue` (including the
/// `data_type` oneof being unset). Returns [`DecodeError::MissingCell`]
/// when the row has fewer cells than `idx`. Returns
/// [`DecodeError::InvalidDate`] when a `Number` cell does not fit `i32`
/// or fails [`crate::tdbe::time::is_valid_yyyymmdd`].
#[inline]
pub(crate) fn row_date(row: &proto::DataValueList, idx: usize) -> Result<Option<i32>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(n)) => {
            let n32 = match i32::try_from(*n) {
                Ok(v) => v,
                Err(_) => return Err(DecodeError::InvalidDate { raw: n.to_string() }),
            };
            if !crate::tdbe::time::is_valid_yyyymmdd(n32) {
                return Err(DecodeError::InvalidDate { raw: n.to_string() });
            }
            Ok(Some(n32))
        }
        Some(proto::data_value::DataType::Timestamp(ts)) => {
            crate::tdbe::time::try_timestamp_to_date(ts.epoch_ms)
                .map(Some)
                .ok_or(DecodeError::TimestampOutOfRange { raw: ts.epoch_ms })
        }
        Some(proto::data_value::DataType::Text(s)) => Ok(Some(parse_iso_date(s)?)),
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Number|Timestamp|Text",
            observed: observed_name(other),
        }),
    }
}

/// Decode an `i32`-valued cell with strict wire-matching semantics.
///
/// Accepts:
/// - `Number(n)` → `Ok(Some(n))` after bounds-checking the wire `int64`
///   against the destination `i32` range.
/// - `Timestamp(ts)` → `Ok(Some(ms_of_day))` — v3 MDDS sends time columns as
///   proto `Timestamp`; the parser expects milliseconds-of-day in Eastern Time.
/// - `NullValue` → `Ok(None)`, matching the wire's null sentinel.
///
/// Any other variant produces [`DecodeError::TypeMismatch`], including the
/// case where the `DataValue` arrived with its `data_type` oneof unset
/// (`observed: "Unset"`) — a wire anomaly we fail loud on. A row shorter than
/// `idx` (index out of bounds) produces [`DecodeError::MissingCell`].
///
/// # Errors
///
/// Returns [`DecodeError::NumericOverflow`] when the wire `int64` value
/// does not fit `i32`. See the [`DecodeError::TypeMismatch`] /
/// [`DecodeError::MissingCell`] variants for the remaining error modes
/// above.
#[inline]
pub(crate) fn row_number(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<i32>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(n)) => {
            let n32 = i32::try_from(*n)
                .map_err(|_| DecodeError::NumericOverflow { raw: n.to_string() })?;
            Ok(Some(n32))
        }
        Some(proto::data_value::DataType::Timestamp(ts)) => {
            crate::tdbe::time::try_timestamp_to_ms_of_day(ts.epoch_ms)
                .map(Some)
                .ok_or(DecodeError::TimestampOutOfRange { raw: ts.epoch_ms })
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
/// Returns [`DecodeError::TypeMismatch`] on any other cell type and
/// [`DecodeError::MissingCell`] on a missing cell. Returns
/// [`DecodeError::InvalidPriceType`] when the wire `price_type` falls
/// outside `0..=crate::tdbe::types::price::MAX_PRICE_TYPE`.
// Reason: protocol-defined integer widths from the FPSS wire specification.
#[allow(clippy::cast_possible_truncation)]
#[inline]
pub(crate) fn row_price_f64(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<f64>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Price(p)) => {
            let price = crate::tdbe::types::price::Price::with_value_and_type(p.value, p.r#type)
                .map_err(|_| DecodeError::InvalidPriceType { raw: p.r#type })?;
            Ok(Some(price.to_f64()))
        }
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
#[inline]
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
/// # Errors
///
/// Returns [`DecodeError::TypeMismatch`] for any other cell variant.
/// Returns [`DecodeError::MissingCell`] for an out-of-bounds column
/// index. Returns [`DecodeError::InvalidPriceType`] when `price_type`
/// is outside `0..=crate::tdbe::types::price::MAX_PRICE_TYPE`.
#[inline]
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
            let v = i64::from(p.value);
            if v == 0 {
                return Ok(Some(0));
            }
            if !(0..=crate::tdbe::types::price::MAX_PRICE_TYPE).contains(&p.r#type) {
                return Err(DecodeError::InvalidPriceType { raw: p.r#type });
            }
            let price_type = p.r#type;
            let exp = price_type - 10;
            // exp ∈ [-10, 9] after the range check; scale-up tops out
            // at i32::MAX * 10^9 ≈ 2.15e18 < i64::MAX.
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

/// Decode an EOD numeric/time cell to `i32`.
///
/// The wildcard EOD report mixes wire shapes per column: plain
/// `Number` (bounds-checked against `i32`), `Price` (raw `value`,
/// no scaling — the report publishes whole integers under the
/// `Price` shape), and proto `Timestamp` (converted to Eastern-Time
/// milliseconds-of-day). `NullValue` yields `Ok(None)`.
///
/// # Errors
///
/// Returns [`DecodeError::NumericOverflow`] when a `Number` cell does
/// not fit `i32`, [`DecodeError::TypeMismatch`] on any other variant
/// (including an unset `data_type` oneof), and
/// [`DecodeError::MissingCell`] when the row is shorter than `idx`.
#[inline]
pub(crate) fn row_eod_number(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<i32>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(n)) => i32::try_from(*n)
            .map(Some)
            .map_err(|_| DecodeError::NumericOverflow { raw: n.to_string() }),
        Some(proto::data_value::DataType::Price(p)) => Ok(Some(p.value)),
        Some(proto::data_value::DataType::Timestamp(ts)) => {
            crate::tdbe::time::try_timestamp_to_ms_of_day(ts.epoch_ms)
                .map(Some)
                .ok_or(DecodeError::TimestampOutOfRange { raw: ts.epoch_ms })
        }
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Number|Price|Timestamp",
            observed: observed_name(other),
        }),
    }
}

/// Decode an EOD numeric cell widened to `i64` (volume / count).
///
/// Same accept-set as [`row_eod_number`] with an `i64` target: `Number`
/// passes through unbounded, `Price` contributes its raw `value`
/// (no scaling), `Timestamp` converts to milliseconds-of-day.
/// `NullValue` yields `Ok(None)`.
///
/// # Errors
///
/// Returns [`DecodeError::TypeMismatch`] on any other variant and
/// [`DecodeError::MissingCell`] when the row is shorter than `idx`.
#[inline]
pub(crate) fn row_eod_number_i64(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<i64>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(n)) => Ok(Some(*n)),
        Some(proto::data_value::DataType::Price(p)) => Ok(Some(i64::from(p.value))),
        Some(proto::data_value::DataType::Timestamp(ts)) => {
            crate::tdbe::time::try_timestamp_to_ms_of_day(ts.epoch_ms)
                .map(|ms| Some(i64::from(ms)))
                .ok_or(DecodeError::TimestampOutOfRange { raw: ts.epoch_ms })
        }
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Number|Price|Timestamp",
            observed: observed_name(other),
        }),
    }
}

/// Decode an EOD date cell to a `YYYYMMDD` integer.
///
/// Numeric payloads (`Number` after an `i32` bounds check, `Price`
/// via its raw `value`) validate through
/// [`crate::tdbe::time::is_valid_yyyymmdd`]; `Timestamp` converts to an
/// Eastern-Time `YYYYMMDD`. `NullValue` yields `Ok(None)`.
///
/// # Errors
///
/// Returns [`DecodeError::InvalidDate`] for calendar-impossible
/// numeric payloads (or a `Number` outside `i32`),
/// [`DecodeError::TypeMismatch`] on any other variant, and
/// [`DecodeError::MissingCell`] when the row is shorter than `idx`.
#[inline]
pub(crate) fn row_eod_date(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<i32>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(n)) => {
            let n32 = match i32::try_from(*n) {
                Ok(v) => v,
                Err(_) => return Err(DecodeError::InvalidDate { raw: n.to_string() }),
            };
            if !crate::tdbe::time::is_valid_yyyymmdd(n32) {
                return Err(DecodeError::InvalidDate { raw: n.to_string() });
            }
            Ok(Some(n32))
        }
        Some(proto::data_value::DataType::Price(p)) => {
            if !crate::tdbe::time::is_valid_yyyymmdd(p.value) {
                return Err(DecodeError::InvalidDate {
                    raw: p.value.to_string(),
                });
            }
            Ok(Some(p.value))
        }
        Some(proto::data_value::DataType::Timestamp(ts)) => {
            crate::tdbe::time::try_timestamp_to_date(ts.epoch_ms)
                .map(Some)
                .ok_or(DecodeError::TimestampOutOfRange { raw: ts.epoch_ms })
        }
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Number|Price|Timestamp",
            observed: observed_name(other),
        }),
    }
}

/// Decode a contract `expiration` cell to a `YYYYMMDD` integer.
///
/// Wildcard responses inject the contract-identity columns, and the
/// `expiration` column legitimately arrives as `Number` (`YYYYMMDD`)
/// or `Text` (ISO `"2026-04-13"`) depending on upstream version —
/// dispatch on the cell's own type rather than coalescing silently.
/// `NullValue` yields `Ok(None)`.
///
/// # Errors
///
/// Returns [`DecodeError::InvalidDate`] for calendar-impossible or
/// non-`i32` numeric payloads and malformed text,
/// [`DecodeError::TypeMismatch`] on any other variant, and
/// [`DecodeError::MissingCell`] when the row is shorter than `idx`.
#[inline]
pub(crate) fn row_contract_expiration(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<i32>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(n)) => {
            let n32 = match i32::try_from(*n) {
                Ok(v) => v,
                Err(_) => return Err(DecodeError::InvalidDate { raw: n.to_string() }),
            };
            if !crate::tdbe::time::is_valid_yyyymmdd(n32) {
                return Err(DecodeError::InvalidDate { raw: n.to_string() });
            }
            Ok(Some(n32))
        }
        Some(proto::data_value::DataType::Text(s)) => Ok(Some(parse_iso_date(s)?)),
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Number|Text",
            observed: observed_name(other),
        }),
    }
}

/// Decode a contract `right` cell to its logical character (`'C'` for
/// a call, `'P'` for a put).
///
/// Wildcard responses inject the contract-identity columns; `right`
/// arrives as a `Number` ASCII code (67 / 80) or as `Text`
/// (`"CALL"`/`"C"`/`"PUT"`/`"P"`). Both wire encodings decode to the
/// character so the typed surface never carries the undecoded wire
/// integer. `NullValue` yields `Ok(None)` (the absent-contract-id
/// fill is `'\0'`).
///
/// # Errors
///
/// Returns [`DecodeError::UnknownEnumVariant`] for any value outside
/// the documented vocabulary (including numeric payloads outside
/// `i32`), [`DecodeError::TypeMismatch`] on any other variant, and
/// [`DecodeError::MissingCell`] when the row is shorter than `idx`.
#[inline]
pub(crate) fn row_contract_right(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<char>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(n)) => match *n {
            67 => Ok(Some('C')),
            80 => Ok(Some('P')),
            other => Err(DecodeError::UnknownEnumVariant {
                field: "right",
                raw: other.to_string(),
            }),
        },
        Some(proto::data_value::DataType::Text(s)) => match s.as_str() {
            "CALL" | "C" => Ok(Some('C')),
            "PUT" | "P" => Ok(Some('P')),
            other => Err(DecodeError::UnknownEnumVariant {
                field: "right",
                raw: other.to_string(),
            }),
        },
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Number|Text",
            observed: observed_name(other),
        }),
    }
}

/// Decode a logical boolean cell (`Number` 0 / 1 → `bool`).
///
/// Used by schema columns typed `bool` (e.g. `CalendarDay.is_open`).
/// `NullValue` yields `Ok(None)` so the absent-column fill stays
/// `false`.
///
/// # Errors
///
/// Returns [`DecodeError::UnknownEnumVariant`] for numeric payloads
/// outside `{0, 1}`, [`DecodeError::TypeMismatch`] on any other
/// variant, and [`DecodeError::MissingCell`] when the row is shorter
/// than `idx`.
#[inline]
pub(crate) fn row_bool(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<bool>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Number(0)) => Ok(Some(false)),
        Some(proto::data_value::DataType::Number(1)) => Ok(Some(true)),
        Some(proto::data_value::DataType::Number(n)) => Err(DecodeError::UnknownEnumVariant {
            field: "bool",
            raw: n.to_string(),
        }),
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Number",
            observed: observed_name(other),
        }),
    }
}

/// Decode a calendar day-type cell to the typed
/// [`crate::tdbe::types::enums::CalendarStatus`].
///
/// Accepts the vendor's `Text` vocabulary (`"open"` / `"early_close"`
/// / `"full_close"` / `"weekend"`) and the integer codes `0..=3`.
/// Unknown values fail loudly so schema drift surfaces as a typed
/// error instead of a silent mis-classification. `NullValue` yields
/// `Ok(None)`.
///
/// # Errors
///
/// Returns [`DecodeError::UnknownEnumVariant`] for values outside the
/// documented vocabulary, [`DecodeError::TypeMismatch`] on any other
/// variant, and [`DecodeError::MissingCell`] when the row is shorter
/// than `idx`.
#[inline]
pub(crate) fn row_calendar_status(
    row: &proto::DataValueList,
    idx: usize,
) -> Result<Option<crate::tdbe::CalendarStatus>, DecodeError> {
    let Some(dv) = row.values.get(idx) else {
        return Err(DecodeError::MissingCell { column: idx });
    };
    match dv.data_type.as_ref() {
        Some(proto::data_value::DataType::Text(s)) => {
            match crate::tdbe::CalendarStatus::from_wire_text(s) {
                Some(status) => Ok(Some(status)),
                None => Err(DecodeError::UnknownEnumVariant {
                    field: "calendar.type",
                    raw: s.clone(),
                }),
            }
        }
        Some(proto::data_value::DataType::Number(n)) => {
            let code = i32::try_from(*n)
                .ok()
                .and_then(crate::tdbe::CalendarStatus::from_code);
            match code {
                Some(status) => Ok(Some(status)),
                None => Err(DecodeError::UnknownEnumVariant {
                    field: "calendar.type",
                    raw: n.to_string(),
                }),
            }
        }
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Text|Number",
            observed: observed_name(other),
        }),
    }
}

// Generated code -- parser functions from tick_schema.toml by build.rs.
//
// The emitted parser bodies reference:
//   * `crate::proto::*` for wire types
//   * the `row_*` single-cell decoders in this module (in scope via
//     `use super::*`)
//   * `crate::decode::column::{extract_column, BLOCK_ROWS}` for the
//     bulk column extraction
//
// The generated `decode_generated.rs` emits `pub fn` for every tick type.
// Most are called by the always-compiled MDDS endpoint macros, but a few
// (`parse_calendar_days`, `parse_option_contracts`) are dead in default builds
// and only consumed by workspace bindings (`thetadatadx-py`).
//
// Strategy: compile the generated module unconditionally (to keep the
// always-needed functions available), but suppress dead-code lints on the
// module with `#[cfg_attr]`. The `__internal` glob re-export makes the
// otherwise-unreachable functions visible to workspace bindings.
// The `#[cfg(not(feature = "__internal"))]` explicit list avoids the dead-code
// lint on the re-export side; the module-level lint is suppressed by the
// `allow(dead_code)` on the inner module only (a narrow scope that does NOT
// apply to the enclosing crate — this is not a crate-wide allowance).
#[allow(clippy::pedantic)] // Reason: auto-generated parser code, not under our control.
#[allow(dead_code)] // Reason: generated functions `parse_calendar_days` and
                    // `parse_option_contracts` have no default-build callers;
                    // they are consumed by `thetadatadx-py` under `__internal`.
                    // Scope: this inner module only — not the enclosing crate.
mod decode_generated {
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/decode_generated.rs"));
}

// Full glob for workspace bindings that use every generated parser.
#[cfg(feature = "__internal")]
pub use decode_generated::*;
// Explicit subset for default builds: exactly the functions the generated MDDS
// endpoint macros (`mdds_parsed_endpoints_generated.rs`) call via `decode::parse_*`.
#[cfg(not(feature = "__internal"))]
pub use decode_generated::{
    parse_eod_ticks, parse_greeks_all_ticks, parse_greeks_eod_ticks,
    parse_greeks_first_order_ticks, parse_greeks_second_order_ticks,
    parse_greeks_third_order_ticks, parse_index_price_at_time_ticks, parse_interest_rate_ticks,
    parse_iv_ticks, parse_market_value_ticks, parse_ohlc_ticks, parse_open_interest_ticks,
    parse_price_ticks, parse_quote_ticks, parse_trade_greeks_all_ticks,
    parse_trade_greeks_first_order_ticks, parse_trade_greeks_implied_volatility_ticks,
    parse_trade_greeks_second_order_ticks, parse_trade_greeks_third_order_ticks,
    parse_trade_quote_ticks, parse_trade_ticks,
};
