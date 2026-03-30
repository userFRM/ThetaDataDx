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

/// Parse a DataTable into TradeTicks.
/// Expects headers matching the trade tick schema.
///
/// If a required header (`ms_of_day` or `price`) is missing, returns an empty
/// vec and logs a warning. Optional headers that are missing cause the
/// corresponding field to default to 0 for each row.
pub fn parse_trade_ticks(table: &proto::DataTable) -> Vec<TradeTick> {
    // Build header index map
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    // Required headers -- if missing, we cannot produce meaningful ticks.
    let Some(ms_idx) = find_header(&h, "ms_of_day") else {
        return vec![];
    };
    let Some(price_idx) = find_header(&h, "price") else {
        return vec![];
    };

    // Optional headers -- default to None, field defaults to 0 per row.
    let seq_idx = find_header(&h, "sequence");
    let ext1_idx = find_header(&h, "ext_condition1");
    let ext2_idx = find_header(&h, "ext_condition2");
    let ext3_idx = find_header(&h, "ext_condition3");
    let ext4_idx = find_header(&h, "ext_condition4");
    let cond_idx = find_header(&h, "condition");
    let size_idx = find_header(&h, "size");
    let exg_idx = find_header(&h, "exchange");
    let cf_idx = find_header(&h, "condition_flags");
    let pf_idx = find_header(&h, "price_flags");
    let vt_idx = find_header(&h, "volume_type");
    let rb_idx = find_header(&h, "records_back");
    let pt_idx = find_header(&h, "price_type");
    let date_idx = find_header(&h, "date");

    // Precompute whether "price" column is a Price-typed column (vs plain number).
    let price_is_typed = h.contains(&"price");

    /// Helper: get a number from a row at an optional column index, defaulting to 0.
    fn opt_number(row: &proto::DataValueList, idx: Option<usize>) -> i32 {
        match idx {
            Some(i) => row_number(row, i),
            None => 0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| {
            let pt = if price_is_typed {
                row_price_type(row, price_idx)
            } else {
                opt_number(row, pt_idx)
            };

            TradeTick {
                ms_of_day: row_number(row, ms_idx),
                sequence: opt_number(row, seq_idx),
                ext_condition1: opt_number(row, ext1_idx),
                ext_condition2: opt_number(row, ext2_idx),
                ext_condition3: opt_number(row, ext3_idx),
                ext_condition4: opt_number(row, ext4_idx),
                condition: opt_number(row, cond_idx),
                size: opt_number(row, size_idx),
                exchange: opt_number(row, exg_idx),
                price: if price_is_typed {
                    row_price_value(row, price_idx)
                } else {
                    row_number(row, price_idx)
                },
                condition_flags: opt_number(row, cf_idx),
                price_flags: opt_number(row, pf_idx),
                volume_type: opt_number(row, vt_idx),
                records_back: opt_number(row, rb_idx),
                price_type: pt,
                date: opt_number(row, date_idx),
            }
        })
        .collect()
}

/// Parse a DataTable into QuoteTicks.
///
/// If required headers (`ms_of_day`, `bid`, `ask`) are missing, returns an
/// empty vec. Optional headers default to 0 per row.
pub fn parse_quote_ticks(table: &proto::DataTable) -> Vec<QuoteTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    // Required headers.
    let Some(ms_idx) = find_header(&h, "ms_of_day") else {
        return vec![];
    };
    let Some(bid_idx) = find_header(&h, "bid") else {
        return vec![];
    };
    let Some(ask_idx) = find_header(&h, "ask") else {
        return vec![];
    };

    // Optional headers.
    let bs_idx = find_header(&h, "bid_size");
    let be_idx = find_header(&h, "bid_exchange");
    let bc_idx = find_header(&h, "bid_condition");
    let as_idx = find_header(&h, "ask_size");
    let ae_idx = find_header(&h, "ask_exchange");
    let ac_idx = find_header(&h, "ask_condition");
    let pt_idx = find_header(&h, "price_type");
    let date_idx = find_header(&h, "date");

    // Precompute whether bid/ask columns are Price-typed (vs plain number).
    let bid_is_typed = h.contains(&"bid");
    let ask_is_typed = h.contains(&"ask");

    fn opt_number(row: &proto::DataValueList, idx: Option<usize>) -> i32 {
        match idx {
            Some(i) => row_number(row, i),
            None => 0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| {
            let pt = if bid_is_typed {
                row_price_type(row, bid_idx)
            } else {
                opt_number(row, pt_idx)
            };

            QuoteTick {
                ms_of_day: row_number(row, ms_idx),
                bid_size: opt_number(row, bs_idx),
                bid_exchange: opt_number(row, be_idx),
                bid: if bid_is_typed {
                    row_price_value(row, bid_idx)
                } else {
                    row_number(row, bid_idx)
                },
                bid_condition: opt_number(row, bc_idx),
                ask_size: opt_number(row, as_idx),
                ask_exchange: opt_number(row, ae_idx),
                ask: if ask_is_typed {
                    row_price_value(row, ask_idx)
                } else {
                    row_number(row, ask_idx)
                },
                ask_condition: opt_number(row, ac_idx),
                price_type: pt,
                date: opt_number(row, date_idx),
            }
        })
        .collect()
}

/// Parse a DataTable into OhlcTicks.
///
/// If required headers (`ms_of_day`, `open`) are missing, returns an empty
/// vec. Optional headers default to 0 per row.
pub fn parse_ohlc_ticks(table: &proto::DataTable) -> Vec<OhlcTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    // Required headers.
    let Some(ms_idx) = find_header(&h, "ms_of_day") else {
        return vec![];
    };
    let Some(o_idx) = find_header(&h, "open") else {
        return vec![];
    };

    // Optional headers.
    let hi_idx = find_header(&h, "high");
    let lo_idx = find_header(&h, "low");
    let c_idx = find_header(&h, "close");
    let vol_idx = find_header(&h, "volume");
    let cnt_idx = find_header(&h, "count");
    let pt_idx = find_header(&h, "price_type");
    let date_idx = find_header(&h, "date");

    // Precompute whether OHLC columns are Price-typed (vs plain number).
    let open_is_typed = h.contains(&"open");
    let high_is_typed = hi_idx.is_some() && h.contains(&"high");
    let low_is_typed = lo_idx.is_some() && h.contains(&"low");
    let close_is_typed = c_idx.is_some() && h.contains(&"close");

    fn opt_number(row: &proto::DataValueList, idx: Option<usize>) -> i32 {
        match idx {
            Some(i) => row_number(row, i),
            None => 0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| {
            let pt = if open_is_typed {
                row_price_type(row, o_idx)
            } else {
                opt_number(row, pt_idx)
            };

            OhlcTick {
                ms_of_day: row_number(row, ms_idx),
                open: if open_is_typed {
                    row_price_value(row, o_idx)
                } else {
                    row_number(row, o_idx)
                },
                high: match hi_idx {
                    Some(i) if high_is_typed => row_price_value(row, i),
                    Some(i) => row_number(row, i),
                    None => 0,
                },
                low: match lo_idx {
                    Some(i) if low_is_typed => row_price_value(row, i),
                    Some(i) => row_number(row, i),
                    None => 0,
                },
                close: match c_idx {
                    Some(i) if close_is_typed => row_price_value(row, i),
                    Some(i) => row_number(row, i),
                    None => 0,
                },
                volume: opt_number(row, vol_idx),
                count: opt_number(row, cnt_idx),
                price_type: pt,
                date: opt_number(row, date_idx),
            }
        })
        .collect()
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

/// Parse a DataTable into TradeQuoteTicks.
///
/// Expects headers matching the combined trade + quote schema.
pub fn parse_trade_quote_ticks(table: &proto::DataTable) -> Vec<TradeQuoteTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let Some(ms_idx) = find_header(&h, "ms_of_day") else {
        return vec![];
    };
    let Some(price_idx) = find_header(&h, "price") else {
        return vec![];
    };

    let seq_idx = find_header(&h, "sequence");
    let ext1_idx = find_header(&h, "ext_condition1");
    let ext2_idx = find_header(&h, "ext_condition2");
    let ext3_idx = find_header(&h, "ext_condition3");
    let ext4_idx = find_header(&h, "ext_condition4");
    let cond_idx = find_header(&h, "condition");
    let size_idx = find_header(&h, "size");
    let exg_idx = find_header(&h, "exchange");
    let cf_idx = find_header(&h, "condition_flags");
    let pf_idx = find_header(&h, "price_flags");
    let vt_idx = find_header(&h, "volume_type");
    let rb_idx = find_header(&h, "records_back");
    let qms_idx = find_header(&h, "quote_ms_of_day");
    let bs_idx = find_header(&h, "bid_size");
    let be_idx = find_header(&h, "bid_exchange");
    let bid_idx = find_header(&h, "bid");
    let bc_idx = find_header(&h, "bid_condition");
    let as_idx = find_header(&h, "ask_size");
    let ae_idx = find_header(&h, "ask_exchange");
    let ask_idx = find_header(&h, "ask");
    let ac_idx = find_header(&h, "ask_condition");
    let qpt_idx = find_header(&h, "quote_price_type");
    let pt_idx = find_header(&h, "price_type");
    let date_idx = find_header(&h, "date");

    let price_is_typed = h.contains(&"price");

    fn opt_number(row: &proto::DataValueList, idx: Option<usize>) -> i32 {
        match idx {
            Some(i) => row_number(row, i),
            None => 0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| {
            let pt = if price_is_typed {
                row_price_type(row, price_idx)
            } else {
                opt_number(row, pt_idx)
            };

            TradeQuoteTick {
                ms_of_day: row_number(row, ms_idx),
                sequence: opt_number(row, seq_idx),
                ext_condition1: opt_number(row, ext1_idx),
                ext_condition2: opt_number(row, ext2_idx),
                ext_condition3: opt_number(row, ext3_idx),
                ext_condition4: opt_number(row, ext4_idx),
                condition: opt_number(row, cond_idx),
                size: opt_number(row, size_idx),
                exchange: opt_number(row, exg_idx),
                price: if price_is_typed {
                    row_price_value(row, price_idx)
                } else {
                    row_number(row, price_idx)
                },
                condition_flags: opt_number(row, cf_idx),
                price_flags: opt_number(row, pf_idx),
                volume_type: opt_number(row, vt_idx),
                records_back: opt_number(row, rb_idx),
                quote_ms_of_day: opt_number(row, qms_idx),
                bid_size: opt_number(row, bs_idx),
                bid_exchange: opt_number(row, be_idx),
                bid: match bid_idx {
                    Some(i) => row_price_value(row, i),
                    None => 0,
                },
                bid_condition: opt_number(row, bc_idx),
                ask_size: opt_number(row, as_idx),
                ask_exchange: opt_number(row, ae_idx),
                ask: match ask_idx {
                    Some(i) => row_price_value(row, i),
                    None => 0,
                },
                ask_condition: opt_number(row, ac_idx),
                quote_price_type: opt_number(row, qpt_idx),
                price_type: pt,
                date: opt_number(row, date_idx),
            }
        })
        .collect()
}

/// Parse a DataTable into OpenInterestTicks.
pub fn parse_open_interest_ticks(table: &proto::DataTable) -> Vec<OpenInterestTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let Some(ms_idx) = find_header(&h, "ms_of_day") else {
        return vec![];
    };

    let oi_idx = find_header(&h, "open_interest");
    let date_idx = find_header(&h, "date");

    fn opt_number(row: &proto::DataValueList, idx: Option<usize>) -> i32 {
        match idx {
            Some(i) => row_number(row, i),
            None => 0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| OpenInterestTick {
            ms_of_day: row_number(row, ms_idx),
            open_interest: opt_number(row, oi_idx),
            date: opt_number(row, date_idx),
        })
        .collect()
}

/// Parse a DataTable into MarketValueTicks.
pub fn parse_market_value_ticks(table: &proto::DataTable) -> Vec<MarketValueTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let Some(ms_idx) = find_header(&h, "ms_of_day") else {
        return vec![];
    };

    let mc_idx = find_header(&h, "market_cap");
    let so_idx = find_header(&h, "shares_outstanding");
    let ev_idx = find_header(&h, "enterprise_value");
    let bv_idx = find_header(&h, "book_value");
    let ff_idx = find_header(&h, "free_float");
    let date_idx = find_header(&h, "date");

    fn opt_i64(row: &proto::DataValueList, idx: Option<usize>) -> i64 {
        match idx {
            Some(i) => row_number_i64(row, i),
            None => 0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| MarketValueTick {
            ms_of_day: row_number(row, ms_idx),
            market_cap: opt_i64(row, mc_idx),
            shares_outstanding: opt_i64(row, so_idx),
            enterprise_value: opt_i64(row, ev_idx),
            book_value: opt_i64(row, bv_idx),
            free_float: opt_i64(row, ff_idx),
            date: date_idx.map(|i| row_number(row, i)).unwrap_or(0),
        })
        .collect()
}

/// Parse a DataTable into GreeksTicks.
///
/// Greeks columns use `Number` (f64) values in the DataTable, not `Price` cells.
pub fn parse_greeks_ticks(table: &proto::DataTable) -> Vec<GreeksTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let Some(ms_idx) = find_header(&h, "ms_of_day") else {
        return vec![];
    };

    let iv_idx = find_header(&h, "implied_volatility");
    let delta_idx = find_header(&h, "delta");
    let gamma_idx = find_header(&h, "gamma");
    let theta_idx = find_header(&h, "theta");
    let vega_idx = find_header(&h, "vega");
    let rho_idx = find_header(&h, "rho");
    let ive_idx = find_header(&h, "iv_error");
    let vanna_idx = find_header(&h, "vanna");
    let charm_idx = find_header(&h, "charm");
    let vomma_idx = find_header(&h, "vomma");
    let veta_idx = find_header(&h, "veta");
    let speed_idx = find_header(&h, "speed");
    let zomma_idx = find_header(&h, "zomma");
    let color_idx = find_header(&h, "color");
    let ultima_idx = find_header(&h, "ultima");
    let d1_idx = find_header(&h, "d1");
    let d2_idx = find_header(&h, "d2");
    let dd_idx = find_header(&h, "dual_delta");
    let dg_idx = find_header(&h, "dual_gamma");
    let eps_idx = find_header(&h, "epsilon");
    let lam_idx = find_header(&h, "lambda");
    let vera_idx = find_header(&h, "vera");
    let date_idx = find_header(&h, "date");

    fn opt_float(row: &proto::DataValueList, idx: Option<usize>) -> f64 {
        match idx {
            Some(i) => row_float(row, i),
            None => 0.0,
        }
    }

    fn opt_number(row: &proto::DataValueList, idx: Option<usize>) -> i32 {
        match idx {
            Some(i) => row_number(row, i),
            None => 0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| GreeksTick {
            ms_of_day: row_number(row, ms_idx),
            implied_volatility: opt_float(row, iv_idx),
            delta: opt_float(row, delta_idx),
            gamma: opt_float(row, gamma_idx),
            theta: opt_float(row, theta_idx),
            vega: opt_float(row, vega_idx),
            rho: opt_float(row, rho_idx),
            iv_error: opt_float(row, ive_idx),
            vanna: opt_float(row, vanna_idx),
            charm: opt_float(row, charm_idx),
            vomma: opt_float(row, vomma_idx),
            veta: opt_float(row, veta_idx),
            speed: opt_float(row, speed_idx),
            zomma: opt_float(row, zomma_idx),
            color: opt_float(row, color_idx),
            ultima: opt_float(row, ultima_idx),
            d1: opt_float(row, d1_idx),
            d2: opt_float(row, d2_idx),
            dual_delta: opt_float(row, dd_idx),
            dual_gamma: opt_float(row, dg_idx),
            epsilon: opt_float(row, eps_idx),
            lambda: opt_float(row, lam_idx),
            vera: opt_float(row, vera_idx),
            date: opt_number(row, date_idx),
        })
        .collect()
}

/// Parse a DataTable into IvTicks.
///
/// IV columns use `Number` (f64) values in the DataTable.
pub fn parse_iv_ticks(table: &proto::DataTable) -> Vec<IvTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let Some(ms_idx) = find_header(&h, "ms_of_day") else {
        return vec![];
    };

    let iv_idx = find_header(&h, "implied_volatility");
    let ive_idx = find_header(&h, "iv_error");
    let date_idx = find_header(&h, "date");

    fn opt_float(row: &proto::DataValueList, idx: Option<usize>) -> f64 {
        match idx {
            Some(i) => row_float(row, i),
            None => 0.0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| IvTick {
            ms_of_day: row_number(row, ms_idx),
            implied_volatility: opt_float(row, iv_idx),
            iv_error: opt_float(row, ive_idx),
            date: date_idx.map(|i| row_number(row, i)).unwrap_or(0),
        })
        .collect()
}

/// Parse a DataTable into PriceTicks.
pub fn parse_price_ticks(table: &proto::DataTable) -> Vec<PriceTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let Some(ms_idx) = find_header(&h, "ms_of_day") else {
        return vec![];
    };
    let Some(price_idx) = find_header(&h, "price") else {
        return vec![];
    };

    let pt_idx = find_header(&h, "price_type");
    let date_idx = find_header(&h, "date");

    let price_is_typed = h.contains(&"price");

    fn opt_number(row: &proto::DataValueList, idx: Option<usize>) -> i32 {
        match idx {
            Some(i) => row_number(row, i),
            None => 0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| {
            let pt = if price_is_typed {
                row_price_type(row, price_idx)
            } else {
                opt_number(row, pt_idx)
            };

            PriceTick {
                ms_of_day: row_number(row, ms_idx),
                price: if price_is_typed {
                    row_price_value(row, price_idx)
                } else {
                    row_number(row, price_idx)
                },
                price_type: pt,
                date: opt_number(row, date_idx),
            }
        })
        .collect()
}

/// Parse a DataTable into CalendarDays.
pub fn parse_calendar_days(table: &proto::DataTable) -> Vec<CalendarDay> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let Some(date_idx) = find_header(&h, "date") else {
        return vec![];
    };

    let open_idx = find_header(&h, "is_open");
    let ot_idx = find_header(&h, "open_time");
    let ct_idx = find_header(&h, "close_time");
    let status_idx = find_header(&h, "status");

    fn opt_number(row: &proto::DataValueList, idx: Option<usize>) -> i32 {
        match idx {
            Some(i) => row_number(row, i),
            None => 0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| CalendarDay {
            date: row_number(row, date_idx),
            is_open: opt_number(row, open_idx),
            open_time: opt_number(row, ot_idx),
            close_time: opt_number(row, ct_idx),
            status: opt_number(row, status_idx),
        })
        .collect()
}

/// Parse a DataTable into InterestRateTicks.
pub fn parse_interest_rate_ticks(table: &proto::DataTable) -> Vec<InterestRateTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let Some(ms_idx) = find_header(&h, "ms_of_day") else {
        return vec![];
    };

    let rate_idx = find_header(&h, "rate");
    let date_idx = find_header(&h, "date");

    table
        .data_table
        .iter()
        .map(|row| InterestRateTick {
            ms_of_day: row_number(row, ms_idx),
            rate: rate_idx.map(|i| row_float(row, i)).unwrap_or(0.0),
            date: date_idx.map(|i| row_number(row, i)).unwrap_or(0),
        })
        .collect()
}

/// Parse a DataTable into OptionContracts.
pub fn parse_option_contracts(table: &proto::DataTable) -> Vec<OptionContract> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();

    let Some(root_idx) = find_header(&h, "root") else {
        return vec![];
    };

    let exp_idx = find_header(&h, "expiration");
    let strike_idx = find_header(&h, "strike");
    let right_idx = find_header(&h, "right");
    let spt_idx = find_header(&h, "strike_price_type");

    fn opt_number(row: &proto::DataValueList, idx: Option<usize>) -> i32 {
        match idx {
            Some(i) => row_number(row, i),
            None => 0,
        }
    }

    table
        .data_table
        .iter()
        .map(|row| OptionContract {
            root: row_text(row, root_idx),
            expiration: opt_number(row, exp_idx),
            strike: opt_number(row, strike_idx),
            right: opt_number(row, right_idx),
            strike_price_type: opt_number(row, spt_idx),
        })
        .collect()
}

/// Parse EOD ticks from a `DataTable` using header-based column lookup.
///
/// Handles both Price-typed and Number-typed columns transparently.
pub fn parse_eod_ticks(table: &proto::DataTable) -> Vec<EodTick> {
    let h: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();
    let find = |name: &str| h.iter().position(|&s| s == name);

    // EOD rows may have Price-typed cells (value + type) or plain Number cells.
    fn eod_num(row: &proto::DataValueList, idx: usize) -> i32 {
        row.values
            .get(idx)
            .and_then(|dv| dv.data_type.as_ref())
            .and_then(|dt| match dt {
                proto::data_value::DataType::Number(n) => Some(*n as i32),
                proto::data_value::DataType::Price(p) => Some(p.value),
                _ => None,
            })
            .unwrap_or(0)
    }

    let ms_of_day_idx = find("ms_of_day");
    let ms_of_day2_idx = find("ms_of_day2");
    let open_idx = find("open");
    let high_idx = find("high");
    let low_idx = find("low");
    let close_idx = find("close");
    let volume_idx = find("volume");
    let count_idx = find("count");
    let bid_size_idx = find("bid_size");
    let bid_exchange_idx = find("bid_exchange");
    let bid_idx = find("bid");
    let bid_condition_idx = find("bid_condition");
    let ask_size_idx = find("ask_size");
    let ask_exchange_idx = find("ask_exchange");
    let ask_idx = find("ask");
    let ask_condition_idx = find("ask_condition");
    let date_idx = find("date");

    table
        .data_table
        .iter()
        .map(|row| {
            let pt = open_idx.map(|i| row_price_type(row, i)).unwrap_or(0);

            EodTick {
                ms_of_day: ms_of_day_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                ms_of_day2: ms_of_day2_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                open: open_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                high: high_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                low: low_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                close: close_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                volume: volume_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                count: count_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                bid_size: bid_size_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                bid_exchange: bid_exchange_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                bid: bid_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                bid_condition: bid_condition_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                ask_size: ask_size_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                ask_exchange: ask_exchange_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                ask: ask_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                ask_condition: ask_condition_idx.map(|i| eod_num(row, i)).unwrap_or(0),
                price_type: pt,
                date: date_idx.map(|i| eod_num(row, i)).unwrap_or(0),
            }
        })
        .collect()
}

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
}
