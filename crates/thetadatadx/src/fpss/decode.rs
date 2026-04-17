//! FPSS frame decoder: wire frame -> typed [`FpssEvent`] pairs.
//!
//! [`decode_frame`] is the dispatch core of the I/O loop. It runs FIT
//! decompression through [`super::delta::DeltaState`], updates the
//! per-contract OHLCVC accumulator, and emits up to two events per frame
//! (the primary event plus an optional derived OHLCVC for Trade frames).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tdbe::types::enums::StreamMsgType;
use tdbe::types::price::Price;

use super::accumulator::OhlcvcAccumulator;
use super::delta::{DeltaState, OHLCVC_FIELDS, OI_FIELDS, QUOTE_FIELDS, TRADE_FIELDS};
use super::events::{FpssControl, FpssData, FpssEvent};
use super::framing;
use super::protocol::{
    parse_contract_message, parse_disconnect_reason, parse_req_response, Contract,
};
use super::reconnect_delay;

/// Empty symbol placeholder used when the contract_id has not been resolved yet.
static EMPTY_SYMBOL: std::sync::LazyLock<Arc<str>> = std::sync::LazyLock::new(|| Arc::from(""));

/// Decode a frame into zero, one, or two `FpssEvent`s.
///
/// Returns `(primary, secondary)` where `secondary` is only `Some` for Trade
/// frames that also produce a derived OHLCVC event. This eliminates the
/// per-frame `Vec<FpssEvent>` allocation that was on the hot path.
///
/// This is the frame dispatch logic from `FPSSClient.java`'s reader thread.
/// Tick data frames (Quote, Trade, `OpenInterest`, Ohlcvc) are FIT-decoded and
/// delta-decompressed before being emitted as typed events.
// Reason: FPSS protocol uses Java-defined integer widths; frame decode is inherently large.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::needless_pass_by_value
)]
pub(super) fn decode_frame(
    code: StreamMsgType,
    payload: &[u8],
    authenticated: &AtomicBool,
    contract_map: &Mutex<HashMap<i32, Contract>>,
    local_symbols: &mut HashMap<i32, Arc<str>>,
    shutdown: &AtomicBool,
    delta_state: &mut DeltaState,
    derive_ohlcvc: bool,
) -> (Option<FpssEvent>, Option<FpssEvent>) {
    // Capture wall-clock timestamp once per frame for all data variants.
    let received_at_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    // Resolve contract_id to a symbol string from the thread-local cache.
    // Returns Arc::clone of the cached symbol, or the empty-string sentinel.
    // Zero allocation, zero Mutex locks on the hot path -- the Arc<str> was
    // built once in the ContractAssigned handler below and inserted into the
    // local HashMap owned by the I/O thread.
    let resolve_symbol = |contract_id: i32, syms: &HashMap<i32, Arc<str>>| -> Arc<str> {
        syms.get(&contract_id)
            .map(Arc::clone)
            .unwrap_or_else(|| Arc::clone(&EMPTY_SYMBOL))
    };

    // Log a warning when ticks arrive for contract IDs not in the local
    // symbol cache. Suppress for 5 seconds after STOP (market close) since
    // stale ticks are expected during teardown. Matches Java terminal behavior.
    // Uses the thread-local cache instead of locking the shared contract_map.
    let warn_unknown_contract =
        |contract_id: i32, kind: &str, delta_state: &DeltaState, syms: &HashMap<i32, Arc<str>>| {
            if !syms.contains_key(&contract_id) && !delta_state.is_in_stop_suppression_window() {
                tracing::warn!(contract_id, kind, "no contract for ID");
            }
        };

    match code {
        StreamMsgType::Metadata => {
            // Can arrive again after reconnection.
            // The payload is the server's opaque "Bundle" string -- see
            // FpssControl::LoginSuccess docs for why we don't parse it.
            let permissions = String::from_utf8_lossy(payload).to_string();
            tracing::debug!(permissions = %permissions, "received METADATA");
            authenticated.store(true, Ordering::Release);
            (
                Some(FpssEvent::Control(FpssControl::LoginSuccess {
                    permissions,
                })),
                None,
            )
        }

        StreamMsgType::Contract => match parse_contract_message(payload) {
            Ok((id, contract)) => {
                tracing::debug!(id, contract = %contract, "contract assigned");
                // Pre-render the symbol string once and cache as Arc<str>
                // so resolve_symbol() on the hot path is just Arc::clone.
                // `Arc::<str>::from(String)` consumes the String's heap buffer
                // directly instead of allocating a fresh copy from the &str.
                let symbol_str: Arc<str> = Arc::<str>::from(contract.to_string());
                // Insert into thread-local cache (zero-lock hot-path lookups).
                local_symbols.insert(id, symbol_str);
                // Also update shared map for external callers (contract_map(),
                // contract_lookup() public APIs on FpssClient).
                contract_map
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(id, contract.clone());
                (
                    Some(FpssEvent::Control(FpssControl::ContractAssigned {
                        id,
                        contract,
                    })),
                    None,
                )
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse CONTRACT message");
                (
                    Some(FpssEvent::Control(FpssControl::Error {
                        message: format!("failed to parse CONTRACT message: {e}"),
                    })),
                    None,
                )
            }
        },

        StreamMsgType::Quote => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, QUOTE_FIELDS) {
                Some((contract_id, f, _n)) => {
                    warn_unknown_contract(contract_id, "quote", delta_state, local_symbols);
                    metrics::counter!("thetadatadx.fpss.events", "kind" => "quote").increment(1);
                    let pt = f[9];
                    (
                        Some(FpssEvent::Data(FpssData::Quote {
                            contract_id,
                            symbol: resolve_symbol(contract_id, local_symbols),
                            ms_of_day: f[0],
                            bid_size: f[1],
                            bid_exchange: f[2],
                            bid: Price::new(f[3], pt).to_f64(),
                            bid_condition: f[4],
                            ask_size: f[5],
                            ask_exchange: f[6],
                            ask: Price::new(f[7], pt).to_f64(),
                            ask_condition: f[8],
                            date: f[10],
                            received_at_ns,
                        })),
                        None,
                    )
                }
                // DATE markers return None from decode_tick -- this is normal
                // protocol flow (session date boundary), not corruption.
                None if delta_state.last_was_date => (None, None),
                None => (
                    Some(FpssEvent::RawData {
                        code: code as u8,
                        payload: payload.to_vec(),
                    }),
                    None,
                ),
            }
        }

        StreamMsgType::Trade => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, TRADE_FIELDS) {
                Some((contract_id, f, n_data)) => {
                    warn_unknown_contract(contract_id, "trade", delta_state, local_symbols);
                    metrics::counter!("thetadatadx.fpss.events", "kind" => "trade").increment(1);

                    if n_data != 8 && n_data != TRADE_FIELDS {
                        tracing::warn!(
                            contract_id,
                            n_data,
                            "unexpected trade field count (expected 8 or 16)"
                        );
                    }

                    // 8-field: [ms_of_day, sequence, size, condition, price, exchange, price_type, date]
                    // 16-field: [ms_of_day, sequence, ext1..ext4, condition, size, exchange, price, cond_flags, price_flags, vol_type, records_back, price_type, date]
                    let sym = resolve_symbol(contract_id, local_symbols);
                    let trade_event = if n_data <= 8 {
                        let pt = f[6];
                        FpssEvent::Data(FpssData::Trade {
                            contract_id,
                            symbol: Arc::clone(&sym),
                            ms_of_day: f[0],
                            sequence: f[1],
                            ext_condition1: 0,
                            ext_condition2: 0,
                            ext_condition3: 0,
                            ext_condition4: 0,
                            condition: f[3],
                            size: f[2],
                            exchange: f[5],
                            price: Price::new(f[4], pt).to_f64(),
                            condition_flags: 0,
                            price_flags: 0,
                            volume_type: 0,
                            records_back: 0,
                            date: f[7],
                            received_at_ns,
                        })
                    } else {
                        let pt = f[14];
                        FpssEvent::Data(FpssData::Trade {
                            contract_id,
                            symbol: Arc::clone(&sym),
                            ms_of_day: f[0],
                            sequence: f[1],
                            ext_condition1: f[2],
                            ext_condition2: f[3],
                            ext_condition3: f[4],
                            ext_condition4: f[5],
                            condition: f[6],
                            size: f[7],
                            exchange: f[8],
                            price: Price::new(f[9], pt).to_f64(),
                            condition_flags: f[10],
                            price_flags: f[11],
                            volume_type: f[12],
                            records_back: f[13],
                            date: f[15],
                            received_at_ns,
                        })
                    };

                    // Extract for OHLCVC derivation (format-aware)
                    let (ms_of_day, size, price, price_type, date) = if n_data <= 8 {
                        (f[0], f[2], f[4], f[6], f[7])
                    } else {
                        (f[0], f[7], f[9], f[14], f[15])
                    };

                    // Derive OHLCVC from trade (Java: OHLCVC.processTrade).
                    // Only if enabled AND the server has already seeded a bar.
                    // When derive_ohlcvc is false, skip entirely — zero overhead.
                    let ohlcvc_event = if derive_ohlcvc {
                        if let Some(acc) = delta_state.ohlcvc.get_mut(&contract_id) {
                            if acc.initialized {
                                acc.process_trade(ms_of_day, price, size, price_type, date);
                                let apt = acc.price_type;
                                Some(FpssEvent::Data(FpssData::Ohlcvc {
                                    contract_id,
                                    symbol: Arc::clone(&sym),
                                    ms_of_day: acc.ms_of_day,
                                    open: Price::new(acc.open, apt).to_f64(),
                                    high: Price::new(acc.high, apt).to_f64(),
                                    low: Price::new(acc.low, apt).to_f64(),
                                    close: Price::new(acc.close, apt).to_f64(),
                                    volume: acc.volume,
                                    count: acc.count,
                                    date: acc.date,
                                    received_at_ns,
                                }))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    (Some(trade_event), ohlcvc_event)
                }
                // DATE markers return None from decode_tick -- normal protocol flow.
                None if delta_state.last_was_date => (None, None),
                None => (
                    Some(FpssEvent::RawData {
                        code: code as u8,
                        payload: payload.to_vec(),
                    }),
                    None,
                ),
            }
        }

        StreamMsgType::OpenInterest => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, OI_FIELDS) {
                Some((contract_id, f, _n)) => {
                    warn_unknown_contract(contract_id, "open_interest", delta_state, local_symbols);
                    metrics::counter!("thetadatadx.fpss.events", "kind" => "open_interest")
                        .increment(1);
                    (
                        Some(FpssEvent::Data(FpssData::OpenInterest {
                            contract_id,
                            symbol: resolve_symbol(contract_id, local_symbols),
                            ms_of_day: f[0],
                            open_interest: f[1],
                            date: f[2],
                            received_at_ns,
                        })),
                        None,
                    )
                }
                None if delta_state.last_was_date => (None, None),
                None => (
                    Some(FpssEvent::RawData {
                        code: code as u8,
                        payload: payload.to_vec(),
                    }),
                    None,
                ),
            }
        }

        StreamMsgType::Ohlcvc => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, OHLCVC_FIELDS) {
                Some((contract_id, f, _n)) => {
                    warn_unknown_contract(contract_id, "ohlcvc", delta_state, local_symbols);
                    metrics::counter!("thetadatadx.fpss.events", "kind" => "ohlcvc").increment(1);
                    let acc = delta_state
                        .ohlcvc
                        .entry(contract_id)
                        .or_insert_with(OhlcvcAccumulator::new);
                    acc.init_from_server(f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], f[8]);
                    let pt = f[7];
                    (
                        Some(FpssEvent::Data(FpssData::Ohlcvc {
                            contract_id,
                            symbol: resolve_symbol(contract_id, local_symbols),
                            ms_of_day: f[0],
                            open: Price::new(f[1], pt).to_f64(),
                            high: Price::new(f[2], pt).to_f64(),
                            low: Price::new(f[3], pt).to_f64(),
                            close: Price::new(f[4], pt).to_f64(),
                            volume: i64::from(f[5]),
                            count: i64::from(f[6]),
                            date: f[8],
                            received_at_ns,
                        })),
                        None,
                    )
                }
                None if delta_state.last_was_date => (None, None),
                None => (
                    Some(FpssEvent::RawData {
                        code: code as u8,
                        payload: payload.to_vec(),
                    }),
                    None,
                ),
            }
        }

        StreamMsgType::ReqResponse => match parse_req_response(payload) {
            Ok((req_id, result)) => {
                tracing::debug!(req_id, result = ?result, "subscription response");
                (
                    Some(FpssEvent::Control(FpssControl::ReqResponse {
                        req_id,
                        result,
                    })),
                    None,
                )
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse REQ_RESPONSE");
                (
                    Some(FpssEvent::Control(FpssControl::Error {
                        message: format!("failed to parse REQ_RESPONSE: {e}"),
                    })),
                    None,
                )
            }
        },

        StreamMsgType::Start => {
            tracing::info!("market open signal received");
            delta_state.clear();
            local_symbols.clear();
            contract_map
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear(); // Java: idToContract.clear()
            (Some(FpssEvent::Control(FpssControl::MarketOpen)), None)
        }

        StreamMsgType::Stop => {
            tracing::info!("market close signal received");
            delta_state.last_stop = Some(Instant::now());
            delta_state.clear();
            local_symbols.clear();
            contract_map
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear(); // Java: idToContract.clear()
            (Some(FpssEvent::Control(FpssControl::MarketClose)), None)
        }

        StreamMsgType::Error => {
            // The dev server's replay loop boundary leaks FIT tick data into
            // Error frames. Detect binary content and skip silently -- these
            // are not real errors, just replay artifacts. Matches Java terminal
            // behavior (logs and ignores).
            let is_binary = framing::is_binary_payload(payload);
            if is_binary {
                tracing::debug!(
                    len = payload.len(),
                    "skipping binary Error frame (replay boundary artifact)"
                );
                (None, None)
            } else {
                let message = String::from_utf8_lossy(payload).to_string();
                tracing::warn!(message = %message, "server error");
                (
                    Some(FpssEvent::Control(FpssControl::ServerError { message })),
                    None,
                )
            }
        }

        StreamMsgType::Disconnected => {
            let reason = parse_disconnect_reason(payload);
            tracing::warn!(reason = ?reason, "server disconnected us");
            metrics::counter!("thetadatadx.fpss.disconnects", "reason" => format!("{:?}", reason))
                .increment(1);
            authenticated.store(false, Ordering::Release);

            // Permanent errors -- no reconnect will fix these.
            if reconnect_delay(reason).is_none() {
                tracing::error!(reason = ?reason, "permanent disconnect -- stopping");
                shutdown.store(true, Ordering::Release);
            }

            (
                Some(FpssEvent::Control(FpssControl::Disconnected { reason })),
                None,
            )
        }

        // Emit unrecognized frame codes as UnknownFrame events with raw
        // payload bytes preserved. This lets users capture broken frames
        // for upstream bug reports instead of silently dropping them.
        other => {
            tracing::warn!(code = ?other, payload_len = payload.len(), "unrecognized FPSS frame code");
            (
                Some(FpssEvent::Control(FpssControl::UnknownFrame {
                    code: other as u8,
                    payload: payload.to_vec(),
                })),
                None,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // FIT encoding helpers for trade mapping tests
    // -----------------------------------------------------------------------

    const FIELD_SEP: u8 = 0xB;
    const END_NIB: u8 = 0xD;
    const NEG_NIB: u8 = 0xE;

    /// Collect the decimal digits of an absolute i32 value as nibbles.
    /// Pushes a NEGATIVE nibble first if the value is negative.
    fn int_to_nibbles(val: i32) -> Vec<u8> {
        let mut nibbles = Vec::new();
        if val < 0 {
            nibbles.push(NEG_NIB);
        }
        let abs = (val as i64).unsigned_abs();
        if abs == 0 {
            nibbles.push(0);
            return nibbles;
        }
        let s = abs.to_string();
        for ch in s.chars() {
            nibbles.push(ch.to_digit(10).unwrap() as u8);
        }
        nibbles
    }

    /// Encode a slice of i32 values into a FIT byte buffer.
    /// Fields are separated by FIELD_SEP, terminated by END.
    fn encode_fit_row(fields: &[i32]) -> Vec<u8> {
        let mut nibbles: Vec<u8> = Vec::new();
        for (i, &val) in fields.iter().enumerate() {
            if i > 0 {
                nibbles.push(FIELD_SEP);
            }
            nibbles.extend(int_to_nibbles(val));
        }
        nibbles.push(END_NIB);

        // Pack nibbles into bytes (two per byte). Pad with 0 nibble if odd.
        let mut bytes = Vec::new();
        let mut i = 0;
        while i < nibbles.len() {
            let high = nibbles[i];
            let low = if i + 1 < nibbles.len() {
                nibbles[i + 1]
            } else {
                0
            };
            bytes.push((high << 4) | (low & 0x0F));
            i += 2;
        }
        bytes
    }

    // -----------------------------------------------------------------------
    // 8-field trade mapping
    // -----------------------------------------------------------------------

    #[test]
    fn decode_tick_8field_trade_returns_correct_n_data_and_fields() {
        // 8-field trade layout (dev server format):
        //   FIT fields: [contract_id, ms_of_day, sequence, size, condition,
        //                price, exchange, price_type, date]
        //   = 1 contract_id + 8 data fields = 9 FIT fields total
        let fit_payload = encode_fit_row(&[
            100,      // contract_id
            34200000, // ms_of_day
            12345,    // sequence
            50,       // size
            6,        // condition
            5500000,  // price
            57,       // exchange
            6,        // price_type
            20250428, // date
        ]);

        let mut ds = DeltaState::new();
        let msg_code = StreamMsgType::Trade as u8;
        let result = ds.decode_tick(msg_code, &fit_payload, TRADE_FIELDS);

        let (contract_id, f, n_data) = result.expect("decode_tick should succeed");

        // Verify contract_id extraction.
        assert_eq!(contract_id, 100);

        // The first absolute tick records the actual field count.
        // 9 FIT fields total - 1 contract_id = 8 data fields.
        assert_eq!(n_data, 8, "n_data must be 8 for an 8-field trade");

        // Verify 8-field mapping produces correct Trade event fields.
        // 8-field layout: [ms_of_day, sequence, size, condition, price, exchange, price_type, date]
        assert_eq!(f[0], 34200000, "ms_of_day");
        assert_eq!(f[1], 12345, "sequence");
        assert_eq!(f[2], 50, "size");
        assert_eq!(f[3], 6, "condition");
        assert_eq!(f[4], 5500000, "price");
        assert_eq!(f[5], 57, "exchange");
        assert_eq!(f[6], 6, "price_type");
        assert_eq!(f[7], 20250428, "date");

        // Verify the n_data <= 8 mapping path produces the correct Trade variant.
        assert!(n_data <= 8);
        // Simulate the mapping from decode_frame's Trade arm:
        let trade = FpssData::Trade {
            contract_id,
            symbol: Arc::from(""),
            ms_of_day: f[0],
            sequence: f[1],
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: f[3],
            size: f[2],
            exchange: f[5],
            price: Price::new(f[4], f[6]).to_f64(),
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            date: f[7],
            received_at_ns: 0,
        };

        match trade {
            FpssData::Trade {
                contract_id: cid,
                ms_of_day,
                sequence,
                size,
                condition,
                price,
                exchange,
                date,
                ext_condition1,
                ext_condition2,
                ext_condition3,
                ext_condition4,
                condition_flags,
                price_flags,
                volume_type,
                records_back,
                ..
            } => {
                assert_eq!(cid, 100);
                assert_eq!(ms_of_day, 34200000);
                assert_eq!(sequence, 12345);
                assert_eq!(size, 50);
                assert_eq!(condition, 6);
                assert_eq!(price, Price::new(5500000, 6).to_f64());
                assert_eq!(exchange, 57);
                assert_eq!(date, 20250428);
                // 8-field trades zero out extended fields.
                assert_eq!(ext_condition1, 0);
                assert_eq!(ext_condition2, 0);
                assert_eq!(ext_condition3, 0);
                assert_eq!(ext_condition4, 0);
                assert_eq!(condition_flags, 0);
                assert_eq!(price_flags, 0);
                assert_eq!(volume_type, 0);
                assert_eq!(records_back, 0);
            }
            other => panic!("expected Trade, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // 16-field trade mapping
    // -----------------------------------------------------------------------

    #[test]
    fn decode_tick_16field_trade_returns_correct_n_data_and_fields() {
        // 16-field trade layout (production format):
        //   FIT fields: [contract_id, ms_of_day, sequence, ext1, ext2, ext3, ext4,
        //                condition, size, exchange, price, cond_flags, price_flags,
        //                vol_type, records_back, price_type, date]
        //   = 1 contract_id + 16 data fields = 17 FIT fields total
        let fit_payload = encode_fit_row(&[
            200,      // contract_id
            34200000, // ms_of_day (f[0])
            99999,    // sequence  (f[1])
            1,        // ext_condition1 (f[2])
            2,        // ext_condition2 (f[3])
            3,        // ext_condition3 (f[4])
            4,        // ext_condition4 (f[5])
            15,       // condition (f[6])
            500,      // size (f[7])
            57,       // exchange (f[8])
            18750000, // price (f[9])
            7,        // condition_flags (f[10])
            3,        // price_flags (f[11])
            1,        // volume_type (f[12])
            0,        // records_back (f[13])
            8,        // price_type (f[14])
            20250428, // date (f[15])
        ]);

        let mut ds = DeltaState::new();
        let msg_code = StreamMsgType::Trade as u8;
        let result = ds.decode_tick(msg_code, &fit_payload, TRADE_FIELDS);

        let (contract_id, f, n_data) = result.expect("decode_tick should succeed");

        // Verify contract_id extraction.
        assert_eq!(contract_id, 200);

        // 17 FIT fields total - 1 contract_id = 16 data fields.
        assert_eq!(n_data, 16, "n_data must be 16 for a 16-field trade");
        assert_eq!(n_data, TRADE_FIELDS);

        // Verify all 16 data fields.
        assert_eq!(f[0], 34200000, "ms_of_day");
        assert_eq!(f[1], 99999, "sequence");
        assert_eq!(f[2], 1, "ext_condition1");
        assert_eq!(f[3], 2, "ext_condition2");
        assert_eq!(f[4], 3, "ext_condition3");
        assert_eq!(f[5], 4, "ext_condition4");
        assert_eq!(f[6], 15, "condition");
        assert_eq!(f[7], 500, "size");
        assert_eq!(f[8], 57, "exchange");
        assert_eq!(f[9], 18750000, "price");
        assert_eq!(f[10], 7, "condition_flags");
        assert_eq!(f[11], 3, "price_flags");
        assert_eq!(f[12], 1, "volume_type");
        assert_eq!(f[13], 0, "records_back");
        assert_eq!(f[14], 8, "price_type");
        assert_eq!(f[15], 20250428, "date");

        // Verify the n_data > 8 mapping path produces the correct Trade variant.
        assert!(n_data > 8);
        let trade = FpssData::Trade {
            contract_id,
            symbol: Arc::from(""),
            ms_of_day: f[0],
            sequence: f[1],
            ext_condition1: f[2],
            ext_condition2: f[3],
            ext_condition3: f[4],
            ext_condition4: f[5],
            condition: f[6],
            size: f[7],
            exchange: f[8],
            price: Price::new(f[9], f[14]).to_f64(),
            condition_flags: f[10],
            price_flags: f[11],
            volume_type: f[12],
            records_back: f[13],
            date: f[15],
            received_at_ns: 0,
        };

        match trade {
            FpssData::Trade {
                contract_id: cid,
                ms_of_day,
                sequence,
                ext_condition1,
                ext_condition2,
                ext_condition3,
                ext_condition4,
                condition,
                size,
                exchange,
                price,
                condition_flags,
                price_flags,
                volume_type,
                records_back,
                date,
                ..
            } => {
                assert_eq!(cid, 200);
                assert_eq!(ms_of_day, 34200000);
                assert_eq!(sequence, 99999);
                assert_eq!(ext_condition1, 1);
                assert_eq!(ext_condition2, 2);
                assert_eq!(ext_condition3, 3);
                assert_eq!(ext_condition4, 4);
                assert_eq!(condition, 15);
                assert_eq!(size, 500);
                assert_eq!(exchange, 57);
                assert_eq!(price, Price::new(18750000, 8).to_f64());
                assert_eq!(condition_flags, 7);
                assert_eq!(price_flags, 3);
                assert_eq!(volume_type, 1);
                assert_eq!(records_back, 0);
                assert_eq!(date, 20250428);
            }
            other => panic!("expected Trade, got {other:?}"),
        }
    }
}
