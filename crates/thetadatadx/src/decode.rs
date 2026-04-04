use std::cell::RefCell;

use crate::error::Error;
use crate::proto;
use tdbe::types::tick::*;

/// Header aliases: v3 MDDS uses different column names than the tick schema.
/// This maps schema names to their v3 equivalents so parsers work with both.
const HEADER_ALIASES: &[(&str, &str)] = &[
    ("ms_of_day", "timestamp"),
    ("ms_of_day", "created"),
    ("ms_of_day2", "timestamp2"),
    ("ms_of_day2", "last_trade"),
    ("date", "timestamp"),
    ("date", "created"),
    // v3 renames: root -> symbol, implied_volatility -> implied_vol
    ("root", "symbol"),
    ("implied_volatility", "implied_vol"),
];

/// Helper: find a column index by name, with alias fallback.
///
/// The v3 MDDS server uses `timestamp` where the tick schema says `ms_of_day`.
/// This function checks the primary name first, then falls back to known aliases.
fn find_header(headers: &[&str], name: &str) -> Option<usize> {
    // Try exact match first.
    if let Some(pos) = headers.iter().position(|&s| s == name) {
        return Some(pos);
    }
    // Try aliases.
    for &(schema_name, server_name) in HEADER_ALIASES {
        if name == schema_name {
            if let Some(pos) = headers.iter().position(|&s| s == server_name) {
                return Some(pos);
            }
        }
    }
    tracing::warn!(
        header = name,
        "expected column header not found in DataTable"
    );
    None
}

/// Eastern Time UTC offset in milliseconds for a given epoch_ms.
///
/// US DST rules changed over time:
///
/// **2007-onward** (Energy Policy Act of 2005):
/// - EDT (UTC-4): second Sunday of March at 2:00 AM local -> first Sunday of November at 2:00 AM local
/// - EST (UTC-5): rest of the year
///
/// **Before 2007** (Uniform Time Act of 1966):
/// - EDT (UTC-4): first Sunday of April at 2:00 AM local -> last Sunday of October at 2:00 AM local
/// - EST (UTC-5): rest of the year
///
/// We compute the transition points in UTC and compare. This avoids
/// external timezone crate dependencies while being correct for all
/// dates with US Eastern Time DST rules.
fn eastern_offset_ms(epoch_ms: u64) -> i64 {
    // First, determine the UTC year/month/day to find DST boundaries.
    let epoch_secs = epoch_ms as i64 / 1000;
    let days_since_epoch = epoch_secs / 86400;

    // Civil date from days since 1970-01-01 (Euclidean algorithm).
    let z = days_since_epoch + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = yoe as i32 + (era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    let (dst_start_utc, dst_end_utc) = if year >= 2007 {
        // Post-2007: second Sunday of March -> first Sunday of November.
        (
            march_second_sunday_utc(year),
            november_first_sunday_utc(year),
        )
    } else {
        // Pre-2007: first Sunday of April -> last Sunday of October.
        (april_first_sunday_utc(year), october_last_sunday_utc(year))
    };

    let epoch_ms_i64 = epoch_ms as i64;
    if epoch_ms_i64 >= dst_start_utc && epoch_ms_i64 < dst_end_utc {
        -4 * 3600 * 1000 // EDT
    } else {
        -5 * 3600 * 1000 // EST
    }
}

/// Epoch ms of the second Sunday of March at 7:00 AM UTC (= 2:00 AM EST).
fn march_second_sunday_utc(year: i32) -> i64 {
    // March 1 day-of-week, then find second Sunday.
    let mar1 = civil_to_epoch_days(year, 3, 1);
    // 1970-01-01 is Thursday. (days + 3) % 7 gives 0=Mon..6=Sun.
    let dow = ((mar1 + 3) % 7 + 7) % 7;
    let days_to_first_sunday = (6 - dow + 7) % 7; // days from Mar 1 to first Sunday
    let second_sunday = mar1 + days_to_first_sunday + 7; // second Sunday
    second_sunday * 86_400_000 + 7 * 3600 * 1000 // 7:00 AM UTC = 2:00 AM EST
}

/// Epoch ms of the first Sunday of November at 6:00 AM UTC (= 2:00 AM EDT).
fn november_first_sunday_utc(year: i32) -> i64 {
    let nov1 = civil_to_epoch_days(year, 11, 1);
    let dow = ((nov1 + 3) % 7 + 7) % 7;
    let days_to_first_sunday = (6 - dow + 7) % 7;
    let first_sunday = nov1 + days_to_first_sunday;
    first_sunday * 86_400_000 + 6 * 3600 * 1000 // 6:00 AM UTC = 2:00 AM EDT
}

/// Epoch ms of the first Sunday of April at 7:00 AM UTC (= 2:00 AM EST).
///
/// Used for pre-2007 DST start (Uniform Time Act of 1966).
fn april_first_sunday_utc(year: i32) -> i64 {
    let apr1 = civil_to_epoch_days(year, 4, 1);
    let dow = ((apr1 + 3) % 7 + 7) % 7;
    let days_to_first_sunday = (6 - dow + 7) % 7;
    let first_sunday = apr1 + days_to_first_sunday;
    first_sunday * 86_400_000 + 7 * 3600 * 1000 // 7:00 AM UTC = 2:00 AM EST
}

/// Epoch ms of the last Sunday of October at 6:00 AM UTC (= 2:00 AM EDT).
///
/// Used for pre-2007 DST end (Uniform Time Act of 1966).
fn october_last_sunday_utc(year: i32) -> i64 {
    // Start from October 31 and walk back to find the last Sunday.
    let oct31 = civil_to_epoch_days(year, 10, 31);
    let dow = ((oct31 + 3) % 7 + 7) % 7; // 0=Mon..6=Sun
    let days_back = (dow + 1) % 7; // days back from Oct 31 to last Sunday
    let last_sunday = oct31 - days_back;
    last_sunday * 86_400_000 + 6 * 3600 * 1000 // 6:00 AM UTC = 2:00 AM EDT
}

/// Convert civil date to days since 1970-01-01 (inverse of the Euclidean algorithm).
fn civil_to_epoch_days(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 {
        year as i64 - 1
    } else {
        year as i64
    };
    let m = if month <= 2 {
        month as i64 + 9
    } else {
        month as i64 - 3
    };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let doy = (153 * m as u64 + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// Convert epoch_ms to milliseconds-of-day in Eastern Time (DST-aware).
fn timestamp_to_ms_of_day(epoch_ms: u64) -> i32 {
    let offset = eastern_offset_ms(epoch_ms);
    let local_ms = epoch_ms as i64 + offset;
    (local_ms.rem_euclid(86_400_000)) as i32
}

/// Convert epoch_ms to YYYYMMDD date integer in Eastern Time (DST-aware).
pub(crate) fn timestamp_to_date(epoch_ms: u64) -> i32 {
    let offset = eastern_offset_ms(epoch_ms);
    let local_secs = (epoch_ms as i64 + offset) / 1000;
    let days = local_secs / 86400 + 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32) * 10000 + (m as i32) * 100 + (d as i32)
}

/// Extract a date (YYYYMMDD) or ms_of_day from a Timestamp cell.
///
/// Used by generated parsers when the `date` field maps to a `timestamp` column.
pub(crate) fn row_date(row: &proto::DataValueList, idx: usize) -> i32 {
    row.values
        .get(idx)
        .and_then(|dv| dv.data_type.as_ref())
        .and_then(|dt| match dt {
            proto::data_value::DataType::Number(n) => Some(*n as i32),
            proto::data_value::DataType::Timestamp(ts) => Some(timestamp_to_date(ts.epoch_ms)),
            _ => None,
        })
        .unwrap_or(0)
}

thread_local! {
    /// Reusable zstd decompressor **and** output buffer — avoids allocating both
    /// a fresh decompressor context and a fresh `Vec<u8>` on every call.
    ///
    /// The decompressor context (~128 KB of zstd internal state) is recycled, and
    /// the output buffer retains its capacity across calls so that repeated
    /// decompressions of similar-sized payloads hit no allocator at all.
    ///
    /// We use `decompress_to_buffer` which writes into the pre-existing Vec
    /// without reallocating when capacity is sufficient. The final `.clone()`
    /// is necessary since we return ownership, but the internal buffer capacity
    /// persists across calls — the key win is avoiding repeated alloc/dealloc
    /// cycles for the working buffer.
    static ZSTD_STATE: RefCell<(zstd::bulk::Decompressor<'static>, Vec<u8>)> = RefCell::new((
        // Infallible in practice: zstd decompressor creation only fails on OOM.
        // thread_local! does not support Result, so unwrap is intentional here.
        zstd::bulk::Decompressor::new().expect("zstd decompressor creation failed (possible OOM)"),
        Vec::with_capacity(1024 * 1024), // 1 MB initial capacity
    ));
}

/// Decompress a ResponseData payload. Returns the raw protobuf bytes of the DataTable.
///
/// # Unknown compression algorithms
///
/// Prost's `.algo()` silently maps unknown enum values to the default (None=0),
/// so we check the raw i32 to detect truly unknown algorithms. Without this,
/// an unrecognized algorithm would be treated as uncompressed, producing garbage.
///
/// # Buffer recycling
///
/// Uses a thread-local `(Decompressor, Vec<u8>)` pair. The `Vec` retains its
/// capacity across calls, so repeated decompressions of similar-sized payloads
/// avoid hitting the allocator for the working buffer. The returned `Vec<u8>`
/// is a clone (we must return ownership), but the internal slab persists.
pub fn decompress_response(response: &proto::ResponseData) -> Result<Vec<u8>, Error> {
    let algo_raw = response
        .compression_description
        .as_ref()
        .map(|cd| cd.algo)
        .unwrap_or(0);

    match proto::CompressionAlgo::try_from(algo_raw) {
        Ok(proto::CompressionAlgo::None) => Ok(response.compressed_data.clone()),
        Ok(proto::CompressionAlgo::Zstd) => {
            let original_size = response.original_size as usize;
            ZSTD_STATE.with(|cell| {
                let (ref mut dec, ref mut buf) = *cell.borrow_mut();
                buf.clear();
                buf.resize(original_size, 0);
                let n = dec
                    .decompress_to_buffer(&response.compressed_data, buf)
                    .map_err(|e| Error::Decompress(e.to_string()))?;
                buf.truncate(n);
                Ok(buf.clone())
            })
        }
        _ => Err(Error::Decompress(format!(
            "unknown compression algorithm: {}",
            algo_raw
        ))),
    }
}

/// Decode a ResponseData into a DataTable.
pub fn decode_data_table(response: &proto::ResponseData) -> Result<proto::DataTable, Error> {
    let bytes = decompress_response(response)?;
    let table: proto::DataTable =
        prost::Message::decode(bytes.as_slice()).map_err(|e| Error::Decode(e.to_string()))?;
    Ok(table)
}

/// Extract a column of i64 values from a DataTable by header name.
pub fn extract_number_column(table: &proto::DataTable, header: &str) -> Vec<Option<i64>> {
    let col_idx = match table.headers.iter().position(|h| h == header) {
        Some(i) => i,
        None => return vec![],
    };

    table
        .data_table
        .iter()
        .map(|row| {
            row.values
                .get(col_idx)
                .and_then(|dv| dv.data_type.as_ref())
                .and_then(|dt| match dt {
                    proto::data_value::DataType::Number(n) => Some(*n),
                    _ => None,
                })
        })
        .collect()
}

/// Extract a column of string values from a DataTable by header name.
pub fn extract_text_column(table: &proto::DataTable, header: &str) -> Vec<Option<String>> {
    let col_idx = match table.headers.iter().position(|h| h == header) {
        Some(i) => i,
        None => return vec![],
    };

    table
        .data_table
        .iter()
        .map(|row| {
            row.values
                .get(col_idx)
                .and_then(|dv| dv.data_type.as_ref())
                .and_then(|dt| match dt {
                    proto::data_value::DataType::Text(s) => Some(s.clone()),
                    proto::data_value::DataType::Number(n) => Some(n.to_string()),
                    proto::data_value::DataType::Price(p) => {
                        Some(format!("{}", tdbe::Price::new(p.value, p.r#type).to_f64()))
                    }
                    _ => None,
                })
        })
        .collect()
}

/// Extract a column of Price values from a DataTable by header name.
pub fn extract_price_column(table: &proto::DataTable, header: &str) -> Vec<Option<tdbe::Price>> {
    let col_idx = match table.headers.iter().position(|h| h == header) {
        Some(i) => i,
        None => return vec![],
    };

    table
        .data_table
        .iter()
        .map(|row| {
            row.values
                .get(col_idx)
                .and_then(|dv| dv.data_type.as_ref())
                .and_then(|dt| match dt {
                    proto::data_value::DataType::Price(p) => {
                        Some(tdbe::Price::new(p.value, p.r#type))
                    }
                    _ => None,
                })
        })
        .collect()
}

/// Helper to get a number from a row at a given column index, defaulting to 0.
///
/// Returns 0 for missing cells, `NullValue` cells, or non-Number types.
/// Tick schemas don't have nullable fields in practice — NullValue only appears
/// in column-oriented endpoints like Greeks/calendar which use `extract_number_column`
/// (which returns `Option`). For tick parsing, defaulting to 0 is correct and
/// matches the Java terminal's behavior.
pub(crate) fn row_number(row: &proto::DataValueList, idx: usize) -> i32 {
    row.values
        .get(idx)
        .and_then(|dv| dv.data_type.as_ref())
        .and_then(|dt| match dt {
            proto::data_value::DataType::Number(n) => Some(*n as i32),
            // v3 MDDS returns Timestamp for time columns.
            // Extract milliseconds-of-day (ET timezone).
            proto::data_value::DataType::Timestamp(ts) => Some(timestamp_to_ms_of_day(ts.epoch_ms)),
            other => {
                tracing::trace!(
                    column = idx,
                    data_type = ?other,
                    "unexpected cell type in tick row, defaulting to 0"
                );
                None
            }
        })
        .unwrap_or(0)
}

/// Helper to get a price value from a row at a given column index.
///
/// See [`row_number`] for null/missing cell handling rationale.
pub(crate) fn row_price_value(row: &proto::DataValueList, idx: usize) -> i32 {
    row.values
        .get(idx)
        .and_then(|dv| dv.data_type.as_ref())
        .and_then(|dt| match dt {
            proto::data_value::DataType::Price(p) => Some(p.value),
            other => {
                tracing::trace!(
                    column = idx,
                    data_type = ?other,
                    "unexpected cell type in tick row (expected Price), defaulting to 0"
                );
                None
            }
        })
        .unwrap_or(0)
}

/// Helper to get price type from a row at a given column index.
///
/// # Known limitation: per-row price type variation
///
/// A single row may contain multiple Price-typed columns (e.g., bid, ask, last)
/// with *different* `price_type` values. However, tick structs store only one
/// `price_type` field, extracted from a designated "source" column (typically the
/// primary price column, e.g., `price` for trades). If the other Price columns
/// in the same row use a different price type, that information is lost. This is
/// an inherent limitation of the flat tick struct design.
///
/// See [`row_number`] for null/missing cell handling rationale.
pub(crate) fn row_price_type(row: &proto::DataValueList, idx: usize) -> i32 {
    row.values
        .get(idx)
        .and_then(|dv| dv.data_type.as_ref())
        .and_then(|dt| match dt {
            proto::data_value::DataType::Price(p) => Some(p.r#type),
            other => {
                tracing::trace!(
                    column = idx,
                    data_type = ?other,
                    "unexpected cell type in tick row (expected Price type), defaulting to 0"
                );
                None
            }
        })
        .unwrap_or(0)
}

/// Helper to get an f64 from a row at a given column index, defaulting to 0.0.
///
/// Greeks and implied volatility columns use `Number` (f64) values in the DataTable,
/// not `Price` cells. This helper extracts the raw f64 value.
pub(crate) fn row_float(row: &proto::DataValueList, idx: usize) -> f64 {
    row.values
        .get(idx)
        .and_then(|dv| dv.data_type.as_ref())
        .and_then(|dt| match dt {
            proto::data_value::DataType::Number(n) => Some(*n as f64),
            // v3 MDDS sends Greeks and other f64 fields as Price-encoded values.
            // Decode using the same formula as Price::to_f64().
            proto::data_value::DataType::Price(p) => {
                if p.r#type == 0 {
                    Some(0.0)
                } else {
                    let exp = p.r#type - 10;
                    if exp >= 0 {
                        Some(p.value as f64 * 10f64.powi(exp))
                    } else {
                        Some(p.value as f64 / 10f64.powi(-exp))
                    }
                }
            }
            _ => None,
        })
        .unwrap_or(0.0)
}

/// Helper to get a String from a row at a given column index, defaulting to empty.
pub(crate) fn row_text(row: &proto::DataValueList, idx: usize) -> String {
    row.values
        .get(idx)
        .and_then(|dv| dv.data_type.as_ref())
        .and_then(|dt| match dt {
            proto::data_value::DataType::Text(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Helper to get an i64 from a row at a given column index, defaulting to 0.
///
/// Market value fields (market_cap, shares_outstanding, etc.) can exceed i32 range.
pub(crate) fn row_number_i64(row: &proto::DataValueList, idx: usize) -> i64 {
    row.values
        .get(idx)
        .and_then(|dv| dv.data_type.as_ref())
        .and_then(|dt| match dt {
            proto::data_value::DataType::Number(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0)
}

// Parser functions are generated from endpoint_schema.toml by build.rs.
include!(concat!(env!("OUT_DIR"), "/decode_generated.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a DataValue containing a Number.
    fn dv_number(n: i64) -> proto::DataValue {
        proto::DataValue {
            data_type: Some(proto::data_value::DataType::Number(n)),
        }
    }

    /// Build a DataValue containing a Price.
    fn dv_price(value: i32, r#type: i32) -> proto::DataValue {
        proto::DataValue {
            data_type: Some(proto::data_value::DataType::Price(proto::Price {
                value,
                r#type,
            })),
        }
    }

    /// Build a DataValue containing NullValue.
    fn dv_null() -> proto::DataValue {
        proto::DataValue {
            data_type: Some(proto::data_value::DataType::NullValue(0)),
        }
    }

    /// Build a DataValue with no data_type set (missing).
    fn dv_missing() -> proto::DataValue {
        proto::DataValue { data_type: None }
    }

    fn row_of(values: Vec<proto::DataValue>) -> proto::DataValueList {
        proto::DataValueList { values }
    }

    #[test]
    fn row_number_returns_value_for_number_cell() {
        let row = row_of(vec![dv_number(42)]);
        assert_eq!(row_number(&row, 0), 42);
    }

    #[test]
    fn row_number_returns_0_for_null_cell() {
        let row = row_of(vec![dv_null()]);
        assert_eq!(row_number(&row, 0), 0);
    }

    #[test]
    fn row_number_returns_0_for_missing_cell() {
        let row = row_of(vec![dv_missing()]);
        assert_eq!(row_number(&row, 0), 0);
    }

    #[test]
    fn row_number_returns_0_for_out_of_bounds() {
        let row = row_of(vec![]);
        assert_eq!(row_number(&row, 5), 0);
    }

    #[test]
    fn row_price_value_returns_value_for_price_cell() {
        let row = row_of(vec![dv_price(12345, 10)]);
        assert_eq!(row_price_value(&row, 0), 12345);
    }

    #[test]
    fn row_price_value_returns_0_for_null_cell() {
        let row = row_of(vec![dv_null()]);
        assert_eq!(row_price_value(&row, 0), 0);
    }

    #[test]
    fn row_price_type_returns_type_for_price_cell() {
        let row = row_of(vec![dv_price(12345, 10)]);
        assert_eq!(row_price_type(&row, 0), 10);
    }

    #[test]
    fn row_price_type_returns_0_for_null_cell() {
        let row = row_of(vec![dv_null()]);
        assert_eq!(row_price_type(&row, 0), 0);
    }

    #[test]
    fn null_cells_dont_corrupt_trade_ticks() {
        // Build a minimal DataTable with one row that has a NullValue in a field.
        // Note: "price" header triggers Price-typed extraction, so we use a Price cell.
        let table = proto::DataTable {
            headers: vec![
                "ms_of_day".into(),
                "sequence".into(),
                "ext_condition1".into(),
                "ext_condition2".into(),
                "ext_condition3".into(),
                "ext_condition4".into(),
                "condition".into(),
                "size".into(),
                "exchange".into(),
                "price".into(),
                "condition_flags".into(),
                "price_flags".into(),
                "volume_type".into(),
                "records_back".into(),
                "date".into(),
            ],
            data_table: vec![row_of(vec![
                dv_number(34200000), // ms_of_day
                dv_number(1),        // sequence
                dv_null(),           // ext_condition1 = NullValue
                dv_number(0),        // ext_condition2
                dv_number(0),        // ext_condition3
                dv_number(0),        // ext_condition4
                dv_number(0),        // condition
                dv_number(100),      // size
                dv_number(4),        // exchange
                dv_price(15000, 10), // price (Price-typed because header is "price")
                dv_number(0),        // condition_flags
                dv_number(0),        // price_flags
                dv_number(0),        // volume_type
                dv_number(0),        // records_back
                dv_number(20240301), // date
            ])],
        };

        let ticks = parse_trade_ticks(&table);
        assert_eq!(ticks.len(), 1);
        let tick = &ticks[0];
        assert_eq!(tick.ms_of_day, 34200000);
        // NullValue should default to 0, not corrupt subsequent fields.
        assert_eq!(tick.ext_condition1, 0);
        assert_eq!(tick.size, 100);
        assert_eq!(tick.price, 15000);
        assert_eq!(tick.price_type, 10);
        assert_eq!(tick.date, 20240301);
    }

    #[test]
    fn extract_number_column_returns_none_for_null() {
        let table = proto::DataTable {
            headers: vec!["val".into()],
            data_table: vec![
                row_of(vec![dv_number(10)]),
                row_of(vec![dv_null()]),
                row_of(vec![dv_number(30)]),
            ],
        };

        let col = extract_number_column(&table, "val");
        assert_eq!(col, vec![Some(10), None, Some(30)]);
    }

    #[test]
    fn timestamp_to_ms_of_day_edt() {
        // 2026-04-01 09:30:00 ET (EDT, UTC-4) = 2026-04-01 13:30:00 UTC
        // epoch_ms for 2026-04-01 13:30:00 UTC
        let epoch_ms: u64 = 1_775_050_200_000; // Apr 1 2026, 13:30 UTC
        let ms = super::timestamp_to_ms_of_day(epoch_ms);
        assert_eq!(ms, 34_200_000, "9:30 AM ET in milliseconds");
    }

    #[test]
    fn timestamp_to_ms_of_day_est() {
        // 2026-01-15 09:30:00 ET (EST, UTC-5) = 2026-01-15 14:30:00 UTC
        let epoch_ms: u64 = 1_768_487_400_000;
        let ms = super::timestamp_to_ms_of_day(epoch_ms);
        assert_eq!(ms, 34_200_000, "9:30 AM ET in milliseconds (winter)");
    }

    #[test]
    fn timestamp_to_date_edt() {
        let epoch_ms: u64 = 1_775_050_200_000; // Apr 1 2026, 13:30 UTC
        let date = super::timestamp_to_date(epoch_ms);
        assert_eq!(date, 20260401);
    }

    #[test]
    fn timestamp_to_date_est() {
        let epoch_ms: u64 = 1_768_487_400_000; // Jan 15 2026, 14:30 UTC
        let date = super::timestamp_to_date(epoch_ms);
        assert_eq!(date, 20260115);
    }

    #[test]
    fn dst_transition_march_2026() {
        // 2026 DST starts March 8 (second Sunday of March)
        // Before: EST (UTC-5) at 06:59 UTC. After: EDT (UTC-4) at 07:01 UTC.
        let before: u64 = 1_772_953_140_000; // Mar 8 2026, 06:59 UTC
        assert_eq!(super::eastern_offset_ms(before), -5 * 3600 * 1000);
        let after: u64 = 1_772_953_260_000; // Mar 8 2026, 07:01 UTC
        assert_eq!(super::eastern_offset_ms(after), -4 * 3600 * 1000);
    }

    #[test]
    fn pre2007_dst_summer_uses_old_rules() {
        // 2006: old rules apply (first Sunday April -> last Sunday October).
        // 2006-07-15 18:00:00 UTC = 2006-07-15 14:00:00 EDT (summer, mid-July).
        // This is well within DST under both old and new rules, so EDT (UTC-4).
        let epoch_ms: u64 = 1_153_065_600_000; // Jul 15 2006, 18:00 UTC
        assert_eq!(
            super::eastern_offset_ms(epoch_ms),
            -4 * 3600 * 1000,
            "mid-July 2006 should be EDT under old DST rules"
        );
    }

    #[test]
    fn pre2007_est_before_april_dst_start() {
        // 2006: old rules — DST starts first Sunday of April (April 2, 2006).
        // 2006-02-15 15:00:00 UTC = 2006-02-15 10:00:00 EST (winter, mid-Feb).
        // Under old rules, February is EST.
        let epoch_ms: u64 = 1_140_015_600_000; // Feb 15 2006, 15:00 UTC
        assert_eq!(
            super::eastern_offset_ms(epoch_ms),
            -5 * 3600 * 1000,
            "mid-February 2006 should be EST under old DST rules"
        );
    }
}

/// Hand-written parser for OptionContract that handles the v3 server's
/// text-formatted fields (expiration as ISO date, right as "PUT"/"CALL").
pub fn parse_option_contracts_v3(table: &crate::proto::DataTable) -> Vec<OptionContract> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let root_idx = match find_header(&h, "root") {
        Some(i) => i,
        None => return vec![],
    };
    let exp_idx = find_header(&h, "expiration");
    let strike_idx = find_header(&h, "strike");
    let right_idx = find_header(&h, "right");

    table
        .data_table
        .iter()
        .map(|row| {
            let root = row_text(row, root_idx);

            // Expiration: may be YYYYMMDD int or ISO date string "2026-04-13"
            let expiration = exp_idx
                .map(|i| {
                    let n = row_number(row, i);
                    if n != 0 {
                        return n;
                    }
                    // Try text: "2026-04-13" -> 20260413
                    let s = row_text(row, i);
                    parse_iso_date(&s)
                })
                .unwrap_or(0);

            // Strike: may be Price or Number
            let (strike, strike_price_type) = strike_idx
                .map(|i| {
                    let pv = row_price_value(row, i);
                    if pv != 0 {
                        (pv, row_price_type(row, i))
                    } else {
                        (row_number(row, i), 0)
                    }
                })
                .unwrap_or((0, 0));

            // Right: may be int or text "PUT"/"CALL"/"C"/"P"
            let right = right_idx
                .map(|i| {
                    let n = row_number(row, i);
                    if n != 0 {
                        return n;
                    }
                    let s = row_text(row, i);
                    match s.as_str() {
                        "CALL" | "C" => 67, // ASCII 'C'
                        "PUT" | "P" => 80,  // ASCII 'P'
                        _ => 0,
                    }
                })
                .unwrap_or(0);

            OptionContract {
                root,
                expiration,
                strike,
                right,
                strike_price_type,
            }
        })
        .collect()
}

/// Parse an ISO date string "2026-04-13" to YYYYMMDD integer 20260413.
fn parse_iso_date(s: &str) -> i32 {
    // Fast path: already numeric (YYYYMMDD)
    if let Ok(n) = s.parse::<i32>() {
        return n;
    }
    // ISO format: YYYY-MM-DD
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 3 {
        if let (Ok(y), Ok(m), Ok(d)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<i32>(),
            parts[2].parse::<i32>(),
        ) {
            return y * 10000 + m * 100 + d;
        }
    }
    0
}
