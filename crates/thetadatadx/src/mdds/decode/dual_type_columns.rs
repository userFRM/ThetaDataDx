//! Hand-written parsers for v3 MDDS payload shapes that the macro-generated
//! parser cannot model directly.
//!
//! v3 publishes some columns as text (ISO dates, "PUT"/"CALL" rights, the
//! calendar `type` column) where the schema would otherwise expect numeric
//! cells. The hand-written parsers here dispatch on the cell's own wire
//! type, surfacing mismatches as [`DecodeError::TypeMismatch`] rather than
//! coalescing silently.

use crate::proto;
use tdbe::types::tick::{CalendarDay, OptionContract};

use super::cell::{cell_type, row_price_f64, row_text};
use super::error::{observed_name, DecodeError};
use super::headers::find_header;

/// Hand-written parser for `OptionContract` that handles the v3 server's
/// text-formatted fields (expiration as ISO date, right as "PUT"/"CALL").
///
/// The `expiration` and `right` columns legitimately arrive as either `Number`
/// or `Text` depending on the upstream version, so the parser dispatches on
/// the cell's own type rather than coalescing silently. Mismatched types
/// propagate as [`DecodeError::TypeMismatch`].
///
/// # Errors
///
/// Returns [`DecodeError`] on type mismatch or missing cell.
pub fn parse_option_contracts_v3(
    table: &crate::proto::DataTable,
) -> Result<Vec<OptionContract>, DecodeError> {
    let h: Vec<&str> = table
        .headers
        .iter()
        .map(std::string::String::as_str)
        .collect();

    // Same schema-drift guard as the generated parsers: "no contracts today"
    // is legitimate, but a rows-present response missing the required `root`
    // column is a silent data-loss trap. The wire column is still named
    // `root` (or `symbol` via the v3 alias in `decode::HEADER_ALIASES`); the
    // `symbol` binding here is the public-API field name documented in the
    // v3 vendor migration guide.
    let symbol_idx = match find_header(&h, "root") {
        Some(i) => i,
        None => {
            if table.data_table.is_empty() {
                return Ok(vec![]);
            }
            return Err(DecodeError::MissingRequiredHeader {
                header: "root",
                rows: table.data_table.len(),
                available: h.join(","),
            });
        }
    };
    let exp_idx = find_header(&h, "expiration");
    let strike_idx = find_header(&h, "strike");
    let right_idx = find_header(&h, "right");

    table
        .data_table
        .iter()
        .map(|row| {
            let symbol = row_text(row, symbol_idx)?.unwrap_or_default();

            // Expiration: `Number` carries YYYYMMDD directly; `Text`
            // carries an ISO "YYYY-MM-DD". Both pass through
            // `is_valid_yyyymmdd` after the wire `int64` is bounds-
            // checked against `i32`. `NullValue` -> 0.
            let expiration = match exp_idx {
                Some(i) => match cell_type(row, i)? {
                    Some(proto::data_value::DataType::Number(n)) => {
                        let n32 = match i32::try_from(*n) {
                            Ok(v) => v,
                            Err(_) => {
                                return Err(DecodeError::InvalidDate { raw: n.to_string() });
                            }
                        };
                        if !tdbe::time::is_valid_yyyymmdd(n32) {
                            return Err(DecodeError::InvalidDate { raw: n.to_string() });
                        }
                        n32
                    }
                    Some(proto::data_value::DataType::Text(s)) => parse_iso_date(s)?,
                    Some(proto::data_value::DataType::NullValue(_)) => 0,
                    None => {
                        return Err(DecodeError::TypeMismatch {
                            column: i,
                            expected: "Number|Text",
                            observed: "Unset",
                        });
                    }
                    other => {
                        return Err(DecodeError::TypeMismatch {
                            column: i,
                            expected: "Number|Text",
                            observed: observed_name(other),
                        });
                    }
                },
                None => 0,
            };

            let strike = match strike_idx {
                Some(i) => row_price_f64(row, i)?.unwrap_or(0.0),
                None => 0.0,
            };

            // Right: `Number` carries the ASCII code directly; `Text`
            // carries "PUT"/"CALL"/"P"/"C". Numeric arms accept only
            // the canonical bytes 67 (`'C'`) and 80 (`'P'`); any other
            // value surfaces as `UnknownEnumVariant`. `NullValue` -> 0.
            let right = match right_idx {
                Some(i) => match cell_type(row, i)? {
                    Some(proto::data_value::DataType::Number(n)) => {
                        let n32 = match i32::try_from(*n) {
                            Ok(v) => v,
                            Err(_) => {
                                return Err(DecodeError::UnknownEnumVariant {
                                    field: "right",
                                    raw: n.to_string(),
                                });
                            }
                        };
                        match n32 {
                            67 | 80 => n32,
                            _ => {
                                return Err(DecodeError::UnknownEnumVariant {
                                    field: "right",
                                    raw: n32.to_string(),
                                });
                            }
                        }
                    }
                    Some(proto::data_value::DataType::Text(s)) => match s.as_str() {
                        "CALL" | "C" => 67, // ASCII 'C'
                        "PUT" | "P" => 80,  // ASCII 'P'
                        other => {
                            return Err(DecodeError::UnknownEnumVariant {
                                field: "right",
                                raw: other.to_string(),
                            });
                        }
                    },
                    Some(proto::data_value::DataType::NullValue(_)) => 0,
                    None => {
                        return Err(DecodeError::TypeMismatch {
                            column: i,
                            expected: "Number|Text",
                            observed: "Unset",
                        });
                    }
                    other => {
                        return Err(DecodeError::TypeMismatch {
                            column: i,
                            expected: "Number|Text",
                            observed: observed_name(other),
                        });
                    }
                },
                None => 0,
            };

            Ok(OptionContract {
                symbol,
                expiration,
                strike,
                right,
            })
        })
        .collect()
}

/// Parse an ISO date string to a YYYYMMDD integer.
///
/// Accepts both compact `YYYYMMDD` (e.g. `"20260413"`) and ISO
/// `YYYY-MM-DD` (e.g. `"2026-04-13"`). Both shapes are validated
/// through [`tdbe::time::is_valid_yyyymmdd`] /
/// [`tdbe::time::is_valid_gregorian_date`].
///
/// # Errors
///
/// Returns [`DecodeError::InvalidDate`] when the input matches neither
/// documented shape, or when the parsed `(year, month, day)` triple is
/// not a real Gregorian date.
// Reason: date parsing with known-safe integer ranges.
#[allow(clippy::cast_possible_truncation, clippy::missing_panics_doc)]
pub(crate) fn parse_iso_date(s: &str) -> Result<i32, DecodeError> {
    if let Ok(n) = s.parse::<i32>() {
        if tdbe::time::is_valid_yyyymmdd(n) {
            return Ok(n);
        }
        return Err(DecodeError::InvalidDate { raw: s.to_string() });
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 3 {
        if let (Ok(y), Ok(m), Ok(d)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<u32>(),
            parts[2].parse::<u32>(),
        ) {
            if tdbe::time::is_valid_gregorian_date(y, m, d) {
                return Ok(y * 10_000 + (m as i32) * 100 + (d as i32));
            }
        }
    }
    Err(DecodeError::InvalidDate { raw: s.to_string() })
}

/// Parse a time string `"HH:MM:SS"` to milliseconds from midnight.
///
/// Used on the v3 calendar `open` / `close` columns. Components are
/// validated against `0..=23` / `0..=59` / `0..=59`.
///
/// # Errors
///
/// Returns [`DecodeError::InvalidTime`] when the input does not split
/// into three colon-delimited integer components, or when any
/// component is outside its clock range.
pub(crate) fn parse_time_text(s: &str) -> Result<i32, DecodeError> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        if let (Ok(h), Ok(m), Ok(sec)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<i32>(),
            parts[2].parse::<i32>(),
        ) {
            if (0..=23).contains(&h) && (0..=59).contains(&m) && (0..=59).contains(&sec) {
                return Ok((h * 3_600 + m * 60 + sec) * 1_000);
            }
        }
    }
    Err(DecodeError::InvalidTime { raw: s.to_string() })
}

/// Calendar day status constants.
///
/// The v3 MDDS server sends a `type` column with text values. We map them to
/// integer constants for the `CalendarDay.status` field:
///
/// | Server text    | Constant | Meaning                           |
/// |----------------|----------|-----------------------------------|
/// | `"open"`       | `0`      | Normal trading day                |
/// | `"early_close"`| `1`      | Early close (e.g. day after Thanksgiving) |
/// | `"full_close"` | `2`      | Market closed (holiday)           |
/// | `"weekend"`    | `3`      | Weekend                           |
///
/// The `CALENDAR_STATUS_UNKNOWN` sentinel is retained for downstream
/// consumers that need to label data they synthesise locally (e.g.
/// gap-fill for missing dates) but the wire decoder no longer maps
/// unknown server text to it — unknown text now surfaces as
/// [`DecodeError::UnknownEnumVariant`] so schema drift is loud, not
/// silent.
pub const CALENDAR_STATUS_OPEN: i32 = 0;
pub const CALENDAR_STATUS_EARLY_CLOSE: i32 = 1;
pub const CALENDAR_STATUS_FULL_CLOSE: i32 = 2;
pub const CALENDAR_STATUS_WEEKEND: i32 = 3;
// `CALENDAR_STATUS_UNKNOWN` is retained for workspace bindings that need to
// label synthesised / gap-fill data locally; the decoder itself never emits
// it (unknown text surfaces as `DecodeError::UnknownEnumVariant`). Gate on
// `__internal` because it has no crate-internal callers.
#[cfg(feature = "__internal")]
pub const CALENDAR_STATUS_UNKNOWN: i32 = -1;

/// Map a v3 calendar `type` text to `(is_open, status)`.
///
/// Returns [`DecodeError::UnknownEnumVariant`] when the text falls
/// outside the documented vendor vocabulary (`open` / `early_close` /
/// `full_close` / `weekend`). Previously this swallowed the unknown
/// case to `(0, CALENDAR_STATUS_UNKNOWN)` which silently mis-classified
/// a future schema change as "market closed, unknown reason" — losing
/// the diagnostic context downstream consumers need to alert on.
fn calendar_type_text(s: &str) -> Result<(i32, i32), DecodeError> {
    match s {
        "open" => Ok((1, CALENDAR_STATUS_OPEN)),
        "early_close" => Ok((1, CALENDAR_STATUS_EARLY_CLOSE)),
        "full_close" => Ok((0, CALENDAR_STATUS_FULL_CLOSE)),
        "weekend" => Ok((0, CALENDAR_STATUS_WEEKEND)),
        other => Err(DecodeError::UnknownEnumVariant {
            field: "calendar.type",
            raw: other.to_string(),
        }),
    }
}

/// Hand-written parser for `CalendarDay` that handles the v3 server's
/// text-formatted fields.
///
/// The v3 MDDS server sends calendar data with different column names and types
/// than the generated parser expects:
///
/// | Schema field | Server header | Server type | Mapping                               |
/// |--------------|---------------|-------------|---------------------------------------|
/// | `date`       | `date`        | Text        | "2025-01-01" -> 20250101              |
/// | `is_open`    | `type`        | Text        | "`open"/"early_close`" -> 1, else -> 0  |
/// | `open_time`  | `open`        | Text / Null | "09:30:00" -> 34200000 ms             |
/// | `close_time` | `close`       | Text / Null | "16:00:00" -> 57600000 ms             |
/// | `status`     | `type`        | Text        | See [`CALENDAR_STATUS_OPEN`] etc.     |
///
/// Note: `calendar_on_date` and `calendar_open_today` omit the `date` column.
/// Each column dispatches on the cell's own type rather than coalescing
/// silently — mismatched types propagate as [`DecodeError::TypeMismatch`].
///
/// # Errors
///
/// Returns [`DecodeError`] on type mismatch or missing cell.
pub fn parse_calendar_days_v3(
    table: &crate::proto::DataTable,
) -> Result<Vec<CalendarDay>, DecodeError> {
    let h: Vec<&str> = table
        .headers
        .iter()
        .map(std::string::String::as_str)
        .collect();

    let date_idx = h.iter().position(|&s| s == "date");
    let type_idx = h.iter().position(|&s| s == "type");
    let open_idx = h.iter().position(|&s| s == "open");
    let close_idx = h.iter().position(|&s| s == "close");

    table
        .data_table
        .iter()
        .map(|row| {
            // date: Number carries YYYYMMDD (bounds-checked against i32
            // then validated as a Gregorian date), Timestamp converts to
            // ET date, Text "YYYY-MM-DD" parses to YYYYMMDD. `NullValue`
            // -> 0.
            let date = match date_idx {
                Some(i) => match cell_type(row, i)? {
                    Some(proto::data_value::DataType::Number(n)) => {
                        let n32 = match i32::try_from(*n) {
                            Ok(v) => v,
                            Err(_) => {
                                return Err(DecodeError::InvalidDate { raw: n.to_string() });
                            }
                        };
                        if !tdbe::time::is_valid_yyyymmdd(n32) {
                            return Err(DecodeError::InvalidDate { raw: n.to_string() });
                        }
                        n32
                    }
                    Some(proto::data_value::DataType::Timestamp(ts)) => {
                        tdbe::time::timestamp_to_date(ts.epoch_ms)
                    }
                    Some(proto::data_value::DataType::Text(s)) => parse_iso_date(s)?,
                    Some(proto::data_value::DataType::NullValue(_)) => 0,
                    None => {
                        return Err(DecodeError::TypeMismatch {
                            column: i,
                            expected: "Number|Timestamp|Text",
                            observed: "Unset",
                        });
                    }
                    other => {
                        return Err(DecodeError::TypeMismatch {
                            column: i,
                            expected: "Number|Timestamp|Text",
                            observed: observed_name(other),
                        });
                    }
                },
                None => 0,
            };

            // type: Text "open"/"full_close"/"early_close"/"weekend"
            // (the documented v3 wire shape). `NullValue` -> (0, 0).
            // Any other variant -> TypeMismatch.
            let (is_open, status) = match type_idx {
                Some(i) => match cell_type(row, i)? {
                    Some(proto::data_value::DataType::Text(s)) => calendar_type_text(s)?,
                    Some(proto::data_value::DataType::NullValue(_)) => (0, 0),
                    None => {
                        return Err(DecodeError::TypeMismatch {
                            column: i,
                            expected: "Text",
                            observed: "Unset",
                        });
                    }
                    other => {
                        return Err(DecodeError::TypeMismatch {
                            column: i,
                            expected: "Text",
                            observed: observed_name(other),
                        });
                    }
                },
                None => (0, 0),
            };

            let open_time = decode_calendar_time(row, open_idx)?;
            let close_time = decode_calendar_time(row, close_idx)?;

            Ok(CalendarDay {
                date,
                is_open,
                open_time,
                close_time,
                status,
            })
        })
        .collect()
}

/// Decode a calendar `open`/`close` column to ms-of-day.
///
/// `Text "HH:MM:SS"` flows through [`parse_time_text`]. `Number`
/// carries ms-of-day directly; the wire `int64` is bounds-checked
/// against `i32` and the resulting value against `0..=86_400_000`.
/// `NullValue` / absent column -> `0`.
///
/// # Errors
///
/// Returns [`DecodeError::NumericOverflow`] when the wire `int64`
/// exceeds `i32`, [`DecodeError::InvalidTime`] when the value is
/// outside the ms-of-day window, and [`DecodeError::TypeMismatch`]
/// for any other cell variant.
fn decode_calendar_time(
    row: &proto::DataValueList,
    idx: Option<usize>,
) -> Result<i32, DecodeError> {
    let Some(i) = idx else {
        return Ok(0);
    };
    match cell_type(row, i)? {
        Some(proto::data_value::DataType::Text(s)) => parse_time_text(s),
        Some(proto::data_value::DataType::Number(n)) => {
            let n32 = i32::try_from(*n)
                .map_err(|_| DecodeError::NumericOverflow { raw: n.to_string() })?;
            if !(0..=86_400_000).contains(&n32) {
                return Err(DecodeError::InvalidTime { raw: n.to_string() });
            }
            Ok(n32)
        }
        Some(proto::data_value::DataType::NullValue(_)) => Ok(0),
        None => Err(DecodeError::TypeMismatch {
            column: i,
            expected: "Text|Number",
            observed: "Unset",
        }),
        other => Err(DecodeError::TypeMismatch {
            column: i,
            expected: "Text|Number",
            observed: observed_name(other),
        }),
    }
}
