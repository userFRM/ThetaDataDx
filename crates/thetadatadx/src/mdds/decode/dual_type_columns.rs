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

            // Expiration: `Number` carries YYYYMMDD directly; `Text` carries
            // an ISO "2026-04-13" that we parse here — malformed text
            // propagates as `DecodeError::InvalidDate` rather than
            // silently coalescing to 0. `NullValue` → 0 (legit null).
            // An unset oneof is a wire anomaly → TypeMismatch.
            let expiration = match exp_idx {
                Some(i) => match cell_type(row, i)? {
                    Some(proto::data_value::DataType::Number(n)) => *n as i32,
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
            // carries "PUT"/"CALL"/"P"/"C". Unknown text →
            // `UnknownEnumVariant` rather than silent coalesce to 0
            // (which previously masked wire-schema drift). `NullValue`
            // is still a legit null and coalesces to 0. An unset oneof
            // is a wire anomaly → TypeMismatch.
            let right = match right_idx {
                Some(i) => match cell_type(row, i)? {
                    Some(proto::data_value::DataType::Number(n)) => *n as i32,
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

/// Parse an ISO date string "2026-04-13" to YYYYMMDD integer 20260413.
///
/// Accepts:
/// - Compact `YYYYMMDD` (already-numeric) — e.g. `"20260413"`.
/// - ISO `YYYY-MM-DD` — e.g. `"2026-04-13"` — which is the v3 wire
///   shape on `interest_rate_history_eod.created` and the v3 calendar
///   `date` column.
///
/// Anything else returns [`DecodeError::InvalidDate`] with the raw
/// text captured for diagnostics. Previously this function returned
/// `0` on parse failure, which silently corrupted downstream
/// timestamps when the upstream schema drifted.
///
/// # Errors
///
/// Returns [`DecodeError::InvalidDate`] when the input matches neither
/// of the documented shapes.
// Reason: date parsing with known-safe integer ranges.
#[allow(clippy::cast_possible_truncation, clippy::missing_panics_doc)]
pub(crate) fn parse_iso_date(s: &str) -> Result<i32, DecodeError> {
    // Fast path: already numeric (YYYYMMDD)
    if let Ok(n) = s.parse::<i32>() {
        return Ok(n);
    }
    // ISO format: YYYY-MM-DD
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 3 {
        if let (Ok(y), Ok(m), Ok(d)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<i32>(),
            parts[2].parse::<i32>(),
        ) {
            return Ok(y * 10_000 + m * 100 + d);
        }
    }
    Err(DecodeError::InvalidDate { raw: s.to_string() })
}

/// Parse a time string "HH:MM:SS" to milliseconds from midnight.
///
/// Used on the v3 calendar `open` / `close` columns. Anything that
/// does not match the documented `HH:MM:SS` shape returns
/// [`DecodeError::InvalidTime`] with the raw text captured for
/// diagnostics. Previously this function returned `0` on parse
/// failure, silently corrupting trading-session timestamps in
/// downstream consumers.
///
/// # Errors
///
/// Returns [`DecodeError::InvalidTime`] when the input does not split
/// into three colon-delimited integer components.
pub(crate) fn parse_time_text(s: &str) -> Result<i32, DecodeError> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        if let (Ok(h), Ok(m), Ok(sec)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<i32>(),
            parts[2].parse::<i32>(),
        ) {
            return Ok((h * 3_600 + m * 60 + sec) * 1_000);
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
            // date: Number carries YYYYMMDD, Timestamp converts to ET date,
            // Text "2025-01-01" parses to YYYYMMDD. `NullValue` → 0 (legit
            // null). Unset oneof is a wire anomaly → TypeMismatch.
            let date = match date_idx {
                Some(i) => match cell_type(row, i)? {
                    Some(proto::data_value::DataType::Number(n)) => *n as i32,
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

            // type: Text "open"/"full_close"/"early_close"/"weekend"; Number
            // kept as a future-proofing path. `NullValue` → (0, 0). Unset
            // oneof is a wire anomaly → TypeMismatch.
            let (is_open, status) = match type_idx {
                Some(i) => match cell_type(row, i)? {
                    Some(proto::data_value::DataType::Text(s)) => calendar_type_text(s)?,
                    Some(proto::data_value::DataType::Number(n)) => {
                        let n = *n as i32;
                        (i32::from(n != 0), n)
                    }
                    Some(proto::data_value::DataType::NullValue(_)) => (0, 0),
                    None => {
                        return Err(DecodeError::TypeMismatch {
                            column: i,
                            expected: "Text|Number",
                            observed: "Unset",
                        });
                    }
                    other => {
                        return Err(DecodeError::TypeMismatch {
                            column: i,
                            expected: "Text|Number",
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

/// Decode a calendar `open`/`close` column. `Text "HH:MM:SS"` → ms-of-day;
/// `Number` kept as future-proofing. `NullValue` / absent column → 0. An unset
/// oneof is a wire anomaly → [`DecodeError::TypeMismatch`].
fn decode_calendar_time(
    row: &proto::DataValueList,
    idx: Option<usize>,
) -> Result<i32, DecodeError> {
    let Some(i) = idx else {
        return Ok(0);
    };
    match cell_type(row, i)? {
        Some(proto::data_value::DataType::Text(s)) => parse_time_text(s),
        Some(proto::data_value::DataType::Number(n)) => Ok(*n as i32),
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
