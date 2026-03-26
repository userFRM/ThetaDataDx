use std::cell::RefCell;

use crate::error::Error;
use crate::proto;
use crate::types::tick::*;

/// Helper: find a column index by name, logging a warning if not found.
/// Returns `None` when the header is missing.
fn find_header(headers: &[&str], name: &str) -> Option<usize> {
    let pos = headers.iter().position(|&s| s == name);
    if pos.is_none() {
        tracing::warn!(
            header = name,
            "expected column header not found in DataTable"
        );
    }
    pos
}

thread_local! {
    /// Reusable zstd decompressor — avoids allocating a fresh decompressor context
    /// on every `decompress_response` call.
    static ZSTD_DECOMPRESSOR: RefCell<zstd::bulk::Decompressor<'static>> =
        RefCell::new(zstd::bulk::Decompressor::new().expect("failed to create zstd decompressor"));
}

/// Decompress a ResponseData payload. Returns the raw protobuf bytes of the DataTable.
///
/// # Unknown compression algorithms
///
/// Prost's `.algo()` silently maps unknown enum values to the default (None=0),
/// so we check the raw i32 to detect truly unknown algorithms. Without this,
/// an unrecognized algorithm would be treated as uncompressed, producing garbage.
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
            let decompressed = ZSTD_DECOMPRESSOR
                .with(|cell| {
                    let mut dec = cell.borrow_mut();
                    dec.decompress(&response.compressed_data, original_size)
                })
                .map_err(|e| Error::Decompress(e.to_string()))?;
            Ok(decompressed)
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
                    _ => None,
                })
        })
        .collect()
}

/// Extract a column of Price values from a DataTable by header name.
pub fn extract_price_column(
    table: &proto::DataTable,
    header: &str,
) -> Vec<Option<crate::types::Price>> {
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
                        Some(crate::types::Price::from_proto(p))
                    }
                    _ => None,
                })
        })
        .collect()
}

/// Helper to get a number from a row at a given column index, defaulting to 0.
fn row_number(row: &proto::DataValueList, idx: usize) -> i32 {
    row.values
        .get(idx)
        .and_then(|dv| dv.data_type.as_ref())
        .and_then(|dt| match dt {
            proto::data_value::DataType::Number(n) => Some(*n as i32),
            _ => None,
        })
        .unwrap_or(0)
}

/// Helper to get a price value from a row at a given column index.
fn row_price_value(row: &proto::DataValueList, idx: usize) -> i32 {
    row.values
        .get(idx)
        .and_then(|dv| dv.data_type.as_ref())
        .and_then(|dt| match dt {
            proto::data_value::DataType::Price(p) => Some(p.value),
            _ => None,
        })
        .unwrap_or(0)
}

/// Helper to get price type from a row at a given column index.
fn row_price_type(row: &proto::DataValueList, idx: usize) -> i32 {
    row.values
        .get(idx)
        .and_then(|dv| dv.data_type.as_ref())
        .and_then(|dt| match dt {
            proto::data_value::DataType::Price(p) => Some(p.r#type),
            _ => None,
        })
        .unwrap_or(0)
}

/// Parse a DataTable into TradeTicks.
/// Expects headers matching the trade tick schema.
pub fn parse_trade_ticks(table: &proto::DataTable) -> Vec<TradeTick> {
    // Build header index map
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let ms_idx = find_header(&h, "ms_of_day").unwrap_or(0);
    let seq_idx = find_header(&h, "sequence").unwrap_or(1);
    let ext1_idx = find_header(&h, "ext_condition1").unwrap_or(2);
    let ext2_idx = find_header(&h, "ext_condition2").unwrap_or(3);
    let ext3_idx = find_header(&h, "ext_condition3").unwrap_or(4);
    let ext4_idx = find_header(&h, "ext_condition4").unwrap_or(5);
    let cond_idx = find_header(&h, "condition").unwrap_or(6);
    let size_idx = find_header(&h, "size").unwrap_or(7);
    let exg_idx = find_header(&h, "exchange").unwrap_or(8);
    let price_idx = find_header(&h, "price").unwrap_or(9);
    let cf_idx = find_header(&h, "condition_flags").unwrap_or(10);
    let pf_idx = find_header(&h, "price_flags").unwrap_or(11);
    let vt_idx = find_header(&h, "volume_type").unwrap_or(12);
    let rb_idx = find_header(&h, "records_back").unwrap_or(13);
    let pt_idx = find_header(&h, "price_type").unwrap_or(14);
    let date_idx = find_header(&h, "date").unwrap_or(15);

    // Precompute whether "price" column is a Price-typed column (vs plain number).
    let price_is_typed = h.contains(&"price");

    table
        .data_table
        .iter()
        .map(|row| {
            let pt = if price_is_typed {
                row_price_type(row, price_idx)
            } else {
                row_number(row, pt_idx)
            };

            TradeTick {
                ms_of_day: row_number(row, ms_idx),
                sequence: row_number(row, seq_idx),
                ext_condition1: row_number(row, ext1_idx),
                ext_condition2: row_number(row, ext2_idx),
                ext_condition3: row_number(row, ext3_idx),
                ext_condition4: row_number(row, ext4_idx),
                condition: row_number(row, cond_idx),
                size: row_number(row, size_idx),
                exchange: row_number(row, exg_idx),
                price: if price_is_typed {
                    row_price_value(row, price_idx)
                } else {
                    row_number(row, price_idx)
                },
                condition_flags: row_number(row, cf_idx),
                price_flags: row_number(row, pf_idx),
                volume_type: row_number(row, vt_idx),
                records_back: row_number(row, rb_idx),
                price_type: pt,
                date: row_number(row, date_idx),
            }
        })
        .collect()
}

/// Parse a DataTable into QuoteTicks.
pub fn parse_quote_ticks(table: &proto::DataTable) -> Vec<QuoteTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let ms_idx = find_header(&h, "ms_of_day").unwrap_or(0);
    let bs_idx = find_header(&h, "bid_size").unwrap_or(1);
    let be_idx = find_header(&h, "bid_exchange").unwrap_or(2);
    let bid_idx = find_header(&h, "bid").unwrap_or(3);
    let bc_idx = find_header(&h, "bid_condition").unwrap_or(4);
    let as_idx = find_header(&h, "ask_size").unwrap_or(5);
    let ae_idx = find_header(&h, "ask_exchange").unwrap_or(6);
    let ask_idx = find_header(&h, "ask").unwrap_or(7);
    let ac_idx = find_header(&h, "ask_condition").unwrap_or(8);
    let pt_idx = find_header(&h, "price_type").unwrap_or(9);
    let date_idx = find_header(&h, "date").unwrap_or(10);

    // Precompute whether bid/ask columns are Price-typed (vs plain number).
    let find = |name: &str| h.iter().position(|&s| s == name);
    let bid_is_typed = find("bid").is_some();
    let ask_is_typed = find("ask").is_some();

    table
        .data_table
        .iter()
        .map(|row| {
            let pt = if bid_is_typed {
                row_price_type(row, bid_idx)
            } else {
                row_number(row, pt_idx)
            };

            QuoteTick {
                ms_of_day: row_number(row, ms_idx),
                bid_size: row_number(row, bs_idx),
                bid_exchange: row_number(row, be_idx),
                bid: if bid_is_typed {
                    row_price_value(row, bid_idx)
                } else {
                    row_number(row, bid_idx)
                },
                bid_condition: row_number(row, bc_idx),
                ask_size: row_number(row, as_idx),
                ask_exchange: row_number(row, ae_idx),
                ask: if ask_is_typed {
                    row_price_value(row, ask_idx)
                } else {
                    row_number(row, ask_idx)
                },
                ask_condition: row_number(row, ac_idx),
                price_type: pt,
                date: row_number(row, date_idx),
            }
        })
        .collect()
}

/// Parse a DataTable into OhlcTicks.
pub fn parse_ohlc_ticks(table: &proto::DataTable) -> Vec<OhlcTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let ms_idx = find_header(&h, "ms_of_day").unwrap_or(0);
    let o_idx = find_header(&h, "open").unwrap_or(1);
    let hi_idx = find_header(&h, "high").unwrap_or(2);
    let lo_idx = find_header(&h, "low").unwrap_or(3);
    let c_idx = find_header(&h, "close").unwrap_or(4);
    let vol_idx = find_header(&h, "volume").unwrap_or(5);
    let cnt_idx = find_header(&h, "count").unwrap_or(6);
    let pt_idx = find_header(&h, "price_type").unwrap_or(7);
    let date_idx = find_header(&h, "date").unwrap_or(8);

    // Precompute whether OHLC columns are Price-typed (vs plain number).
    let find = |name: &str| h.iter().position(|&s| s == name);
    let open_is_typed = find("open").is_some();
    let high_is_typed = find("high").is_some();
    let low_is_typed = find("low").is_some();
    let close_is_typed = find("close").is_some();

    table
        .data_table
        .iter()
        .map(|row| {
            let pt = if open_is_typed {
                row_price_type(row, o_idx)
            } else {
                row_number(row, pt_idx)
            };

            OhlcTick {
                ms_of_day: row_number(row, ms_idx),
                open: if open_is_typed {
                    row_price_value(row, o_idx)
                } else {
                    row_number(row, o_idx)
                },
                high: if high_is_typed {
                    row_price_value(row, hi_idx)
                } else {
                    row_number(row, hi_idx)
                },
                low: if low_is_typed {
                    row_price_value(row, lo_idx)
                } else {
                    row_number(row, lo_idx)
                },
                close: if close_is_typed {
                    row_price_value(row, c_idx)
                } else {
                    row_number(row, c_idx)
                },
                volume: row_number(row, vol_idx),
                count: row_number(row, cnt_idx),
                price_type: pt,
                date: row_number(row, date_idx),
            }
        })
        .collect()
}
