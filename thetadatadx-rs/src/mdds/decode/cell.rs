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
//! The tick parsers are expanded here from the tick table
//! (`crate::__ticks!`), next to the single-cell decoders they drive.

use super::dual_type_columns::parse_iso_date;
use super::error::DecodeError;
use super::error::observed_name;
use super::headers::find_header;
use crate::proto;
use crate::tdbe::types::tick::{
    CalendarDay, EodTick, GreeksAllTick, GreeksEodTick, GreeksFirstOrderTick,
    GreeksSecondOrderTick, GreeksThirdOrderTick, IndexPriceAtTimeTick, InterestRateTick,
    IntervalQuoteTick, IvTick, MarketValueTick, OhlcTick, OpenInterestTick, OptionContract,
    PriceTick, QuoteTick, TradeGreeksAllTick, TradeGreeksFirstOrderTick,
    TradeGreeksImpliedVolatilityTick, TradeGreeksSecondOrderTick, TradeGreeksThirdOrderTick,
    TradeQuoteTick, TradeTick,
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
/// would otherwise expect `Price`, and the flat-file responses send prices as
/// decimal `Text`, which must parse to a finite number. `NullValue` returns
/// `Ok(None)`.
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
        // The flat-file responses send every price as its decimal text.
        Some(proto::data_value::DataType::Text(s)) => match s.parse::<f64>() {
            Ok(v) if v.is_finite() => Ok(Some(v)),
            _ => Err(DecodeError::TypeMismatch {
                column: idx,
                expected: "Price|Number|decimal Text",
                observed: "Text",
            }),
        },
        Some(proto::data_value::DataType::NullValue(_)) => Ok(None),
        other => Err(DecodeError::TypeMismatch {
            column: idx,
            expected: "Price|Number|decimal Text",
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

// The tick decoders, expanded from the tick table: one two-phase parser per
// type, plus its wire-column presence and chain sort key.
//
// An empty response (a holiday, a symbol with no activity) decodes to no rows.
// Otherwise phase 1 resolves the column layout once, and a required header
// missing from it is schema drift that fails the decode rather than yielding
// zero-seeded ticks. Phase 2 seeds the output, then extracts
// column by column in cache-resident row blocks; a column the response
// omitted keeps its seed, and a nullable column's presence flag stays false.
// A wildcard response carries the contract columns under their exact wire
// names.
//
// `CalendarDay` and `OptionContract` are decoded by hand in
// `dual_type_columns`.
macro_rules! tick_decoders {
    ($($(#[$attr:ident])* $name:ident $body:tt)*) => {
        $(crate::__tick! { [$(#[$attr])* $name $body] => tick_decoder {} })*
    };
}

macro_rules! tick_decoder {
    ($name:ident $kind:ident contract $contract:tt doc $doc:tt summary $summary:tt more $more:tt
        required $required:tt columns [$({ $f:ident $w:ident $ty:ident $has:tt $d:tt })*]
        timestamps $timestamps:tt flags $flags:tt) => {
        tick_decoder! { @parse $name $required $contract [$({ $f $w $ty $has })*] }

        impl crate::columns::WireColumns for crate::tdbe::types::tick::$name {
            fn present_columns(headers: &[&str]) -> crate::columns::ColumnPresence {
                tick_decoder!(@present $name headers $contract [$(($w, $f))*])
            }

            fn all_columns() -> crate::columns::ColumnPresence {
                tick_decoder!(@all $contract [$($f)*])
            }
        }

        tick_decoder! { @chain $name $contract }
    };
    (@parse CalendarDay $($rest:tt)*) => {};
    (@parse OptionContract $($rest:tt)*) => {};
    (@parse $name:ident [$($req:ident)*] [$($exp:ident $strike:ident $right:ident)?]
        [$({ $f:ident $w:ident $ty:ident [$($has:ident)?] })*]) => {
        crate::__paste! {
            pub fn [<parse_ $name:snake s>](table: &crate::proto::DataTable) -> Result<Vec<$name>, DecodeError> {
                let rows = table.data_table.as_slice();
                if rows.is_empty() {
                    return Ok(Vec::new());
                }
                let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();
                $(let [<$w _idx>] = find_header(&h, stringify!($w));)*
                $(if [<$req _idx>].is_none() {
                    return Err(DecodeError::MissingRequiredHeader {
                        header: stringify!($req),
                        rows: rows.len(),
                        available: h.join(","),
                    });
                })*
                $(
                    let [<$exp _idx>] = h.iter().position(|c| *c == stringify!($exp));
                    let [<$strike _idx>] = h.iter().position(|c| *c == stringify!($strike));
                    let [<$right _idx>] = h.iter().position(|c| *c == stringify!($right));
                )?

                let mut ticks = vec![
                    $name {
                        $($f: tick_decoder!(@seed $ty), $($has: false,)?)*
                        $($exp: 0, $strike: 0.0, $right: '\0',)?
                    };
                    rows.len()
                ];
                let mut row_base = 0_usize;
                for (rows, ticks) in rows
                    .chunks(crate::decode::column::BLOCK_ROWS)
                    .zip(ticks.chunks_mut(crate::decode::column::BLOCK_ROWS))
                {
                    $(if let Some(idx) = [<$w _idx>] {
                        tick_decoder!(@extract rows ticks row_base idx $w $f [$($has)?] tick_decoder!(@seed $ty), tick_decoder!(@decoder $f $ty));
                    })*
                    $(
                        if let Some(idx) = [<$exp _idx>] {
                            crate::decode::column::extract_column(rows, ticks, row_base, idx, stringify!($exp), 0, row_contract_expiration, |t, v| t.$exp = v)?;
                        }
                        if let Some(idx) = [<$strike _idx>] {
                            crate::decode::column::extract_column(rows, ticks, row_base, idx, stringify!($strike), 0.0, row_price_f64, |t, v| t.$strike = v)?;
                        }
                        if let Some(idx) = [<$right _idx>] {
                            crate::decode::column::extract_column(rows, ticks, row_base, idx, stringify!($right), '\0', row_contract_right, |t, v| t.$right = v)?;
                        }
                    )?
                    row_base += rows.len();
                }
                Ok(ticks)
            }
        }
    };
    (@extract $rows:ident $ticks:ident $base:ident $idx:ident $w:ident $f:ident [] $seed:expr, $decoder:expr) => {
        crate::decode::column::extract_column($rows, $ticks, $base, $idx, stringify!($w), $seed, $decoder, |t, v| t.$f = v)?
    };
    (@extract $rows:ident $ticks:ident $base:ident $idx:ident $w:ident $f:ident [$has:ident] $seed:expr, $decoder:expr) => {
        crate::decode::column::extract_nullable_column($rows, $ticks, $base, $idx, stringify!($w), $seed, $decoder, |t, v, present| {
            t.$f = v;
            t.$has = present;
        })?
    };
    // The calendar parser renames its wire columns, so its presence has its
    // own table.
    (@present CalendarDay $headers:ident $($rest:tt)*) => {
        crate::columns::present_calendar_day_columns($headers)
    };
    (@present $name:ident $headers:ident [$($cid:ident)*] [$(($w:ident, $f:ident))*]) => {
        crate::columns::present_columns_from(
            $headers,
            &[$((stringify!($w), stringify!($f))),*],
            tick_decoder!(@contract $($cid)*),
        )
    };
    (@contract) => { false };
    (@contract $exp:ident $strike:ident $right:ident) => { true };
    (@all [$($cid:ident)*] [$($f:ident)*]) => {
        crate::columns::ColumnPresence::from_names([$(stringify!($f),)* $(stringify!($cid)),*])
    };
    (@chain $name:ident [$exp:ident $strike:ident $right:ident]) => {
        impl crate::mdds::shard::ChainSortKey for $name {
            fn chain_sort_key(&self) -> Option<crate::mdds::shard::ChainKey> {
                Some(crate::mdds::shard::ChainKey::new(self.$exp, self.$strike, self.$right))
            }
        }
    };
    (@chain $name:ident []) => {
        impl crate::mdds::shard::ChainSortKey for $name {
            fn chain_sort_key(&self) -> Option<crate::mdds::shard::ChainKey> {
                None
            }
        }
    };
    (@seed String) => { String::new() };
    (@seed right) => { '\0' };
    (@seed f64) => { 0.0 };
    (@seed price) => { 0.0 };
    (@seed eod_price) => { 0.0 };
    (@seed $ty:ident) => { 0 };
    (@decoder date i32) => { row_date };
    (@decoder underlying_date i32) => { row_date };
    (@decoder $f:ident i32) => { row_number };
    (@decoder $f:ident i64) => { row_number_i64 };
    (@decoder $f:ident f64) => { row_price_f64 };
    (@decoder $f:ident price) => { row_price_f64 };
    (@decoder $f:ident String) => { row_text };
    (@decoder $f:ident right) => { row_contract_right };
    (@decoder $f:ident eod_num) => { row_eod_number };
    (@decoder $f:ident eod_num64) => { row_eod_number_i64 };
    (@decoder $f:ident eod_date) => { row_eod_date };
    (@decoder $f:ident eod_price) => { row_price_f64 };
}

crate::__ticks! { tick_decoders {} }
