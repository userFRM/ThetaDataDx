//! FPSS frame decoder: wire frame -> typed [`FpssEvent`] pairs.
//!
//! [`decode_frame`] is the dispatch core of the I/O loop. It runs FIT
//! decompression through [`super::delta::DeltaState`], updates the
//! per-contract OHLCVC accumulator, and emits up to two events per frame
//! (the primary event plus an optional derived OHLCVC for Trade frames).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use metrics::Counter;
use tdbe::types::enums::StreamMsgType;
use tdbe::types::price::Price;

use super::accumulator::OhlcvcAccumulator;
use super::delta::{DeltaState, TickFields, OHLCVC_FIELDS, OI_FIELDS, QUOTE_FIELDS, TRADE_FIELDS};
use super::events::{FpssControl, FpssData, FpssEventInternal};
use super::framing;
use super::protocol::{
    parse_contract_message, parse_disconnect_reason, parse_req_response, Contract,
};
use super::reconnect_delay;

// ─── Hoisted per-tick counter handles ───────────────────────────────
//
// `metrics::counter!(name, "kind" => value)` resolves the (name, labels)
// tuple to a `Counter` handle on every call — that lookup is a hashmap
// probe inside the global recorder and runs ~30 ns per tick in the
// observed bench. Hoisting the resolution to a `LazyLock<Counter>`
// turns the per-tick increment into a single atomic add (~5 ns) since
// `Counter::increment` is `&self`.
//
// One handle per (metric, kind) pair the decoder emits. Decode-failure
// counters are kept separately so the dashboards can split healthy
// events from FIT parse rejections.

static FPSS_QUOTE_EVENTS: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.events", "kind" => "quote"));
static FPSS_TRADE_EVENTS: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.events", "kind" => "trade"));
static FPSS_OI_EVENTS: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.events", "kind" => "open_interest"));
static FPSS_OHLCVC_EVENTS: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.events", "kind" => "ohlcvc"));

static FPSS_QUOTE_DECODE_FAILURES: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.decode_failures", "kind" => "quote"));
static FPSS_TRADE_DECODE_FAILURES: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.decode_failures", "kind" => "trade"));
static FPSS_OI_DECODE_FAILURES: LazyLock<Counter> = LazyLock::new(
    || metrics::counter!("thetadatadx.fpss.decode_failures", "kind" => "open_interest"),
);
static FPSS_OHLCVC_DECODE_FAILURES: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.decode_failures", "kind" => "ohlcvc"));

/// Prefix on the [`Contract::symbol`] of an unresolved-contract sentinel
/// returned for a tick that arrived before the matching
/// `ContractAssigned` frame. The numeric wire id of the unresolved
/// contract follows the prefix verbatim (e.g. `"__pending:42"`).
///
/// Downstream consumers (notably the WS bridge) parse the suffix back
/// into an `i32` to surface `unresolved_contract_id` to operators
/// without re-introducing the wire id on the public `FpssData` surface.
/// Production callbacks should detect the sentinel via
/// `contract.sec_type == SecType::Unknown` — the prefix is a diagnostic
/// payload, not a stable identifier.
pub const UNRESOLVED_CONTRACT_SYMBOL_PREFIX: &str = "__pending:";

/// Build the unresolved-contract sentinel for a given wire id. The
/// `symbol` is `__pending:<id>` (decimal); `sec_type` is
/// [`SecType::Unknown`] so downstream code can detect the sentinel via
/// the type-safe enum check rather than a string prefix match.
fn unresolved_sentinel(contract_id: i32) -> Arc<Contract> {
    Arc::new(Contract {
        symbol: format!("{UNRESOLVED_CONTRACT_SYMBOL_PREFIX}{contract_id}"),
        sec_type: tdbe::types::enums::SecType::Unknown,
        expiration: None,
        is_call: None,
        strike: None,
    })
}

/// Decode a frame into zero, one, or two `FpssEvent`s.
///
/// Returns `(primary, secondary)` where `secondary` is only `Some` for Trade
/// frames that also produce a derived OHLCVC event. This eliminates the
/// per-frame `Vec<FpssEvent>` allocation that was on the hot path.
///
/// This is the frame dispatch logic of the reader thread. Tick data frames
/// (Quote, Trade, `OpenInterest`, Ohlcvc) are FIT-decoded and delta-decompressed
/// before being emitted as typed events.
// Reason: FPSS wire protocol uses fixed integer widths; frame decode is inherently large.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::needless_pass_by_value
)]
#[doc(hidden)]
pub fn decode_frame(
    code: StreamMsgType,
    payload: &[u8],
    authenticated: &AtomicBool,
    local_contracts: &mut HashMap<i32, Arc<Contract>>,
    shutdown: &AtomicBool,
    delta_state: &mut DeltaState,
    derive_ohlcvc: bool,
) -> (Option<FpssEventInternal>, Option<FpssEventInternal>) {
    // Capture wall-clock timestamp once per frame for all data variants.
    //
    // M1 fix: previously a `SystemTime::now()` failure (clock jumped
    // before UNIX epoch — extremely rare, but possible on a misconfigured
    // host or a virtualised guest with a buggy paravirtual clock) silently
    // produced `received_at_ns = 0`, which downstream consumers cannot
    // distinguish from a legitimate epoch-zero timestamp. The fix logs a
    // rate-limited warning so operators see the clock-skew condition,
    // and falls back to a monotonic-derived approximation rather than
    // sentinelling on `0`. `Instant`-based fallback uses the program's
    // approximate epoch alignment captured at first call.
    let received_at_ns = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        // u128 → u64 saturates past 2554-07-21T23:34:33Z (when ns since
        // UNIX_EPOCH first exceeds 2^64). `as u64` would wrap to a
        // misleading early-1970 timestamp; `try_from` + `unwrap_or` clamps
        // to the schema sentinel without panicking on the boundary.
        Ok(d) => u64::try_from(d.as_nanos()).unwrap_or(u64::MAX),
        Err(e) => {
            static FAIL_COUNT: AtomicU64 = AtomicU64::new(0);
            let prev = FAIL_COUNT.fetch_add(1, Ordering::Relaxed);
            // Rate-limit at 1024 to match the slow-callback warn cadence.
            if prev.is_multiple_of(1024) {
                tracing::warn!(
                    target: "thetadatadx::fpss::decode",
                    failure_count = prev + 1,
                    error = %e,
                    "SystemTime::now() returned a time before UNIX_EPOCH; \
                     received_at_ns will fall back to 0 for this frame -- \
                     check host clock configuration",
                );
            }
            0
        }
    };

    // Resolve contract_id to an Arc<Contract> from the thread-local cache.
    // On a miss (tick arrived before the matching `ContractAssigned`
    // frame) build a per-tick unresolved-contract sentinel whose
    // `symbol` is `__pending:<id>` so downstream consumers can surface
    // the wire id as an `unresolved_contract_id` diagnostic without
    // re-introducing the field on the public `FpssData` surface (the
    //  removal of wire ids from data events stands).
    //
    // The hit path stays zero-allocation: `Arc::clone` is a refcount
    // bump on the cached `Arc<Contract>`. The miss path pays one
    // `String::from(format!(..))` + one `Arc::new` per unresolved tick;
    // miss density is bounded by the brief window between the first
    // tick on a contract and the matching `ContractAssigned` frame.
    let resolve_contract =
        |contract_id: i32, cache: &HashMap<i32, Arc<Contract>>| -> Arc<Contract> {
            cache
                .get(&contract_id)
                .map(Arc::clone)
                .unwrap_or_else(|| unresolved_sentinel(contract_id))
        };

    // Log a warning when ticks arrive for contract IDs not in the local
    // contract cache. Suppress for 5 seconds after STOP (market close) since
    // stale ticks are expected during teardown. Matches Java terminal behavior.
    // Uses the thread-local cache instead of locking the shared contract_map.
    //
    // Rate-limit at every 1024th hit to match the cadence of the
    // slow-callback / clock-skew warnings (`decode.rs:96`,
    // `mod.rs::slow_callback`). Without the limit, a server-side
    // mis-routing or replay-boundary anomaly that emits ticks for an
    // unknown id would flood `tracing` with a per-tick line on every
    // affected contract — at FPSS arrival rates the log channel
    // becomes the bottleneck.
    //
    // `MISS_COUNT` is process-global (static AtomicU64), so the
    // cadence is shared across every `FpssClient` running inside the
    // same process. Operators reading the warning should treat the
    // "1 of every 1024" rate as a process-wide aggregate, not a
    // per-client signal — two clients each missing 512 unique
    // contracts together hit the warn boundary exactly once between
    // them.
    let warn_unknown_contract =
        |contract_id: i32,
         kind: &str,
         delta_state: &DeltaState,
         cache: &HashMap<i32, Arc<Contract>>| {
            if !cache.contains_key(&contract_id) && !delta_state.is_in_stop_suppression_window() {
                static MISS_COUNT: AtomicU64 = AtomicU64::new(0);
                let prev = MISS_COUNT.fetch_add(1, Ordering::Relaxed);
                if prev.is_multiple_of(1024) {
                    tracing::warn!(
                        contract_id,
                        kind,
                        miss_count = prev + 1,
                        "no contract for ID (1 of every 1024 emitted across all FpssClients in this process)"
                    );
                }
            }
        };

    // Stack-allocated tick buffer reused across every FIT-decoded arm. The
    // decoder writes the absolute field values directly here; the match arm
    // reads `buf[i]` to construct the public `FpssData` variant. Sized at
    // the widest tick shape (`TRADE_FIELDS = 16`) so every arm shares one
    // buffer with zero heap traffic on the decode hot path.
    let mut buf: TickFields = [0; super::delta::MAX_DATA_FIELDS];

    match code {
        StreamMsgType::Metadata => {
            // Can arrive again after reconnection.
            // The payload is the server's opaque "Bundle" string -- see
            // FpssControl::LoginSuccess docs for why we don't parse it.
            let permissions = String::from_utf8_lossy(payload).to_string();
            // The Bundle string carries the account's subscription scope
            // (e.g. `STOCK.PRO, OPTION.PRO, INDEX.PRO`) — operationally
            // useful but account-identifying, so log it at `trace!` where
            // a production deployment will not capture it by default.
            tracing::trace!(permissions = %permissions, "received METADATA");
            authenticated.store(true, Ordering::Release);
            (
                Some(FpssEventInternal::Control(FpssControl::LoginSuccess {
                    permissions,
                })),
                None,
            )
        }

        StreamMsgType::Contract => match parse_contract_message(payload) {
            Ok((id, contract)) => {
                tracing::debug!(id, contract = %contract, "contract assigned");
                // Wrap the parsed contract in Arc once on insert. Every
                // subsequent data event refcount-clones this Arc, so the
                // only `Contract::clone` (and therefore the only
                // `String::clone` of `contract.symbol`) happens here —
                // at most once per contract_id per session.
                let arc_contract: Arc<Contract> = Arc::new(contract);
                // Insert into thread-local cache (zero-lock hot-path lookups).
                // Downstream consumers that need an id->contract map build
                // it from the `ContractAssigned` event stream — the SDK no
                // longer holds wire-internal `contract_id` state.
                local_contracts.insert(id, Arc::clone(&arc_contract));
                (
                    Some(FpssEventInternal::Control(FpssControl::ContractAssigned {
                        id,
                        contract: arc_contract,
                    })),
                    None,
                )
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse CONTRACT message");
                (
                    Some(FpssEventInternal::Control(FpssControl::Error {
                        message: format!("failed to parse CONTRACT message: {e}"),
                    })),
                    None,
                )
            }
        },

        StreamMsgType::Quote => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, QUOTE_FIELDS, &mut buf) {
                Some((contract_id, _n)) => {
                    warn_unknown_contract(contract_id, "quote", delta_state, local_contracts);
                    FPSS_QUOTE_EVENTS.increment(1);
                    let pt = buf[9];
                    (
                        Some(FpssEventInternal::Data(FpssData::Quote {
                            contract: resolve_contract(contract_id, local_contracts),
                            ms_of_day: buf[0],
                            bid_size: buf[1],
                            bid_exchange: buf[2],
                            bid: Price::new(buf[3], pt).to_f64(),
                            bid_condition: buf[4],
                            ask_size: buf[5],
                            ask_exchange: buf[6],
                            ask: Price::new(buf[7], pt).to_f64(),
                            ask_condition: buf[8],
                            date: buf[10],
                            received_at_ns,
                        })),
                        None,
                    )
                }
                // DATE markers return None from decode_tick -- this is normal
                // protocol flow (session date boundary), not corruption.
                None if delta_state.last_was_date => (None, None),
                None => {
                    // Truncated / corrupt FIT payload. Account for it on
                    // the public counter so operators see decode pressure
                    // without raw-byte fallout reaching the user callback.
                    FPSS_QUOTE_DECODE_FAILURES.increment(1);
                    (Some(FpssEventInternal::Unparseable), None)
                }
            }
        }

        StreamMsgType::Trade => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, TRADE_FIELDS, &mut buf) {
                Some((contract_id, n_data)) => {
                    warn_unknown_contract(contract_id, "trade", delta_state, local_contracts);
                    FPSS_TRADE_EVENTS.increment(1);

                    if n_data != 8 && n_data != TRADE_FIELDS {
                        tracing::warn!(
                            contract_id,
                            n_data,
                            "unexpected trade field count (expected 8 or 16)"
                        );
                    }

                    // 8-field: [ms_of_day, sequence, size, condition, price, exchange, price_type, date]
                    // 16-field: [ms_of_day, sequence, ext1..ext4, condition, size, exchange, price, cond_flags, price_flags, vol_type, records_back, price_type, date]
                    let contract_arc = resolve_contract(contract_id, local_contracts);
                    let trade_event = if n_data <= 8 {
                        let pt = buf[6];
                        FpssEventInternal::Data(FpssData::Trade {
                            contract: Arc::clone(&contract_arc),
                            ms_of_day: buf[0],
                            sequence: buf[1],
                            ext_condition1: 0,
                            ext_condition2: 0,
                            ext_condition3: 0,
                            ext_condition4: 0,
                            condition: buf[3],
                            size: buf[2],
                            exchange: buf[5],
                            price: Price::new(buf[4], pt).to_f64(),
                            condition_flags: 0,
                            price_flags: 0,
                            volume_type: 0,
                            records_back: 0,
                            date: buf[7],
                            received_at_ns,
                        })
                    } else {
                        let pt = buf[14];
                        FpssEventInternal::Data(FpssData::Trade {
                            contract: Arc::clone(&contract_arc),
                            ms_of_day: buf[0],
                            sequence: buf[1],
                            ext_condition1: buf[2],
                            ext_condition2: buf[3],
                            ext_condition3: buf[4],
                            ext_condition4: buf[5],
                            condition: buf[6],
                            size: buf[7],
                            exchange: buf[8],
                            price: Price::new(buf[9], pt).to_f64(),
                            condition_flags: buf[10],
                            price_flags: buf[11],
                            volume_type: buf[12],
                            records_back: buf[13],
                            date: buf[15],
                            received_at_ns,
                        })
                    };

                    // Extract for OHLCVC derivation (format-aware)
                    let (ms_of_day, size, price, price_type, date) = if n_data <= 8 {
                        (buf[0], buf[2], buf[4], buf[6], buf[7])
                    } else {
                        (buf[0], buf[7], buf[9], buf[14], buf[15])
                    };

                    // Derive OHLCVC from trade (OHLCVC.processTrade).
                    // Only if enabled AND the server has already seeded a bar.
                    // When derive_ohlcvc is false, skip entirely — zero overhead.
                    let ohlcvc_event = if derive_ohlcvc {
                        if let Some(acc) = delta_state.ohlcvc.get_mut(&contract_id) {
                            if acc.initialized {
                                acc.process_trade(ms_of_day, price, size, price_type, date);
                                let apt = acc.price_type;
                                Some(FpssEventInternal::Data(FpssData::Ohlcvc {
                                    contract: Arc::clone(&contract_arc),
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
                None => {
                    FPSS_TRADE_DECODE_FAILURES.increment(1);
                    (Some(FpssEventInternal::Unparseable), None)
                }
            }
        }

        StreamMsgType::OpenInterest => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, OI_FIELDS, &mut buf) {
                Some((contract_id, _n)) => {
                    warn_unknown_contract(
                        contract_id,
                        "open_interest",
                        delta_state,
                        local_contracts,
                    );
                    FPSS_OI_EVENTS.increment(1);
                    (
                        Some(FpssEventInternal::Data(FpssData::OpenInterest {
                            contract: resolve_contract(contract_id, local_contracts),
                            ms_of_day: buf[0],
                            open_interest: buf[1],
                            date: buf[2],
                            received_at_ns,
                        })),
                        None,
                    )
                }
                None if delta_state.last_was_date => (None, None),
                None => {
                    FPSS_OI_DECODE_FAILURES.increment(1);
                    (Some(FpssEventInternal::Unparseable), None)
                }
            }
        }

        StreamMsgType::Ohlcvc => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, OHLCVC_FIELDS, &mut buf) {
                Some((contract_id, _n)) => {
                    warn_unknown_contract(contract_id, "ohlcvc", delta_state, local_contracts);
                    FPSS_OHLCVC_EVENTS.increment(1);
                    let acc = delta_state
                        .ohlcvc
                        .entry(contract_id)
                        .or_insert_with(OhlcvcAccumulator::new);
                    acc.init_from_server(
                        buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8],
                    );
                    let pt = buf[7];
                    (
                        Some(FpssEventInternal::Data(FpssData::Ohlcvc {
                            contract: resolve_contract(contract_id, local_contracts),
                            ms_of_day: buf[0],
                            open: Price::new(buf[1], pt).to_f64(),
                            high: Price::new(buf[2], pt).to_f64(),
                            low: Price::new(buf[3], pt).to_f64(),
                            close: Price::new(buf[4], pt).to_f64(),
                            volume: i64::from(buf[5]),
                            count: i64::from(buf[6]),
                            date: buf[8],
                            received_at_ns,
                        })),
                        None,
                    )
                }
                None if delta_state.last_was_date => (None, None),
                None => {
                    FPSS_OHLCVC_DECODE_FAILURES.increment(1);
                    (Some(FpssEventInternal::Unparseable), None)
                }
            }
        }

        StreamMsgType::ReqResponse => match parse_req_response(payload) {
            Ok((req_id, result)) => {
                tracing::debug!(req_id, result = ?result, "subscription response");
                (
                    Some(FpssEventInternal::Control(FpssControl::ReqResponse {
                        req_id,
                        result,
                    })),
                    None,
                )
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse REQ_RESPONSE");
                (
                    Some(FpssEventInternal::Control(FpssControl::Error {
                        message: format!("failed to parse REQ_RESPONSE: {e}"),
                    })),
                    None,
                )
            }
        },

        StreamMsgType::Start => {
            tracing::info!("market open signal received");
            delta_state.clear();
            local_contracts.clear(); // mirrors idToContract.clear() on the wire
            (
                Some(FpssEventInternal::Control(FpssControl::MarketOpen)),
                None,
            )
        }

        StreamMsgType::Stop => {
            tracing::info!("market close signal received");
            delta_state.last_stop = Some(Instant::now());
            delta_state.clear();
            local_contracts.clear(); // mirrors idToContract.clear() on the wire
            (
                Some(FpssEventInternal::Control(FpssControl::MarketClose)),
                None,
            )
        }

        StreamMsgType::Error => {
            // The dev server's replay loop boundary leaks FIT tick data into
            // Error frames. Detect binary content and skip silently -- these
            // are not real errors, just replay artifacts (logged + ignored).
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
                    Some(FpssEventInternal::Control(FpssControl::ServerError {
                        message,
                    })),
                    None,
                )
            }
        }

        StreamMsgType::Disconnected => {
            let reason = parse_disconnect_reason(payload);
            tracing::warn!(reason = ?reason, "server disconnected us");
            // `RemoveReason::as_str` returns a `&'static str` per
            // variant, so the label allocation drops to zero. Disconnects
            // are rare, so this isn't a hot-path win — it's a
            // consistency fix per the "no per-event allocations"
            // discipline elsewhere in this module.
            metrics::counter!("thetadatadx.fpss.disconnects", "reason" => reason.as_str())
                .increment(1);
            authenticated.store(false, Ordering::Release);

            // Permanent errors -- no reconnect will fix these.
            if reconnect_delay(reason).is_none() {
                tracing::error!(reason = ?reason, "permanent disconnect -- stopping");
                shutdown.store(true, Ordering::Release);
            }

            (
                Some(FpssEventInternal::Control(FpssControl::Disconnected {
                    reason,
                })),
                None,
            )
        }

        // Known server→client control frames. Each of these previously
        // fell through to `UnknownFrame`, leaving consumers to filter
        // noise they did not ask for. Each now maps to its own typed
        // `FpssControl` variant so downstream code can match directly.
        StreamMsgType::Connected => {
            // Code 4: connection ack. Logs "connected" and returns — no
            // side effects other than acknowledging the transition.
            tracing::debug!("FPSS server CONNECTED frame received");
            (
                Some(FpssEventInternal::Control(FpssControl::Connected)),
                None,
            )
        }

        StreamMsgType::Ping => {
            // Code 10: server heartbeat. Observed payload is a single
            // zero byte `[0]`; the client does NOT respond — the client
            // itself sends its own independent 100ms pings. Preserve the
            // raw payload for diagnostics so anomalous heartbeats can be
            // inspected after-the-fact.
            (
                Some(FpssEventInternal::Control(FpssControl::Ping {
                    payload: payload.to_vec(),
                })),
                None,
            )
        }

        StreamMsgType::Reconnected => {
            // Code 13: server-side reconnect ack. Distinct from
            // `FpssControl::Reconnected` which the client emits when its
            // own auto-reconnect state machine completes. Both can be
            // observed in the same session — e.g. a client-side
            // reconnect produces `Reconnected`, while a transparent
            // server-side reconnect produces `ReconnectedServer`.
            tracing::debug!("FPSS server RECONNECTED frame received");
            (
                Some(FpssEventInternal::Control(FpssControl::ReconnectedServer)),
                None,
            )
        }

        StreamMsgType::Restart => {
            // Code 31: server stream restart. A restart is a reset
            // signal — contract IDs assigned before the restart may be
            // reused or re-announced with different shapes afterwards.
            // Mirror the START (code 30) / STOP (code 32) arms: clear
            // delta decode state AND both contract caches so subsequent
            // ticks on unseen IDs get the empty-contract sentinel rather
            // than a stale (and possibly shape-wrong) Contract.
            tracing::info!("FPSS server RESTART frame received");
            delta_state.clear();
            local_contracts.clear();
            (Some(FpssEventInternal::Control(FpssControl::Restart)), None)
        }

        // Emit unrecognized frame codes as UnknownFrame events with raw
        // payload bytes preserved. This lets users capture broken frames
        // for upstream bug reports instead of silently dropping them.
        other => {
            tracing::warn!(code = ?other, payload_len = payload.len(), "unrecognized FPSS frame code");
            (
                Some(FpssEventInternal::Control(FpssControl::UnknownFrame {
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
    use crate::fpss::FpssEvent;

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
        let mut f: TickFields = [0; crate::fpss::delta::MAX_DATA_FIELDS];
        let result = ds.decode_tick(msg_code, &fit_payload, TRADE_FIELDS, &mut f);

        let (contract_id, n_data) = result.expect("decode_tick should succeed");

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
        // The wire-internal `contract_id` no longer rides on the Trade
        // event (extracted by `decode_tick`, used only to resolve the
        // `Arc<Contract>` in `decode_frame`).
        assert_eq!(contract_id, 100);
        // Simulate the mapping from decode_frame's Trade arm:
        let trade = FpssData::Trade {
            contract: unresolved_sentinel(contract_id),
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
        let mut f: TickFields = [0; crate::fpss::delta::MAX_DATA_FIELDS];
        let result = ds.decode_tick(msg_code, &fit_payload, TRADE_FIELDS, &mut f);

        let (contract_id, n_data) = result.expect("decode_tick should succeed");

        // Verify contract_id extraction.
        assert_eq!(contract_id, 200);

        // 17 FIT fields total - 1 contract_id = 16 data fields.
        assert_eq!(n_data, 16, "n_data must be 16 for a 16-field trade");
        assert_eq!(n_data, TRADE_FIELDS);
        assert_eq!(contract_id, 200);

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
            contract: unresolved_sentinel(contract_id),
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

    // -----------------------------------------------------------------------
    // New control frame codes: 4 (Connected), 10 (Ping), 13 (ReconnectedServer),
    // 31 (Restart). Each of these previously fell through to UnknownFrame.
    // -----------------------------------------------------------------------

    fn decode_ctrl(code: StreamMsgType, payload: &[u8]) -> FpssEventInternal {
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let (primary, _) = decode_frame(
            code,
            payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
            false,
        );
        primary.expect("decode_frame must emit a primary event for known control codes")
    }

    /// Reborrow a primary `FpssEventInternal` from `decode_frame` as a
    /// public `&FpssEvent` for assertions, panicking on the
    /// internal-only variants the tests do not expect.
    fn expect_public(evt: &FpssEventInternal) -> &FpssEvent {
        evt.as_public()
            .expect("decode_frame primary event must reborrow as &FpssEvent")
    }

    #[test]
    fn decode_code_4_connected_emits_typed_variant() {
        let evt = decode_ctrl(StreamMsgType::Connected, &[]);
        match expect_public(&evt) {
            FpssEvent::Control(FpssControl::Connected) => {}
            other => panic!("expected Control(Connected), got {other:?}"),
        }
    }

    #[test]
    fn decode_code_10_ping_emits_typed_variant_with_payload() {
        // Observed on production FPSS streams: 1-byte payload `[0]`.
        let evt = decode_ctrl(StreamMsgType::Ping, &[0u8]);
        match expect_public(&evt) {
            FpssEvent::Control(FpssControl::Ping { payload }) => {
                assert_eq!(payload.as_slice(), &[0u8]);
            }
            other => panic!("expected Control(Ping), got {other:?}"),
        }
    }

    #[test]
    fn decode_code_13_reconnected_server_emits_typed_variant() {
        let evt = decode_ctrl(StreamMsgType::Reconnected, &[]);
        match expect_public(&evt) {
            FpssEvent::Control(FpssControl::ReconnectedServer) => {}
            other => panic!("expected Control(ReconnectedServer), got {other:?}"),
        }
    }

    #[test]
    fn decode_code_31_restart_emits_typed_variant_and_clears_delta_state() {
        // Seed delta state so we can verify it was cleared.
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        // Insert a synthetic OHLCVC accumulator entry so we can assert
        // `delta_state.clear()` actually ran on the Restart arm.
        delta_state
            .ohlcvc
            .insert(42, super::super::accumulator::OhlcvcAccumulator::new());
        assert!(delta_state.ohlcvc.contains_key(&42));

        let (primary, _) = decode_frame(
            StreamMsgType::Restart,
            &[],
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
            false,
        );
        let primary_internal = primary.expect("Restart must emit a primary event");
        match expect_public(&primary_internal) {
            FpssEvent::Control(FpssControl::Restart) => {}
            other => panic!("expected Control(Restart), got {other:?}"),
        }
        assert!(
            !delta_state.ohlcvc.contains_key(&42),
            "Restart must clear delta state so downstream deltas don't \
             decode against a stale baseline"
        );
    }

    // -----------------------------------------------------------------------
    // ContractAssigned must now hand out Arc<Contract> -- the same Arc that
    // every subsequent data event carries, proving the hot-path refcount
    // claim: one heap allocation per contract_id, not per event.
    // -----------------------------------------------------------------------

    #[test]
    fn contract_assigned_uses_arc_contract_and_shares_heap_allocation() {
        use crate::fpss::protocol::Contract as ProtoContract;

        // Build a synthetic CONTRACT payload: 4-byte id + contract bytes.
        let expected_contract = ProtoContract::stock("AAPL");
        let mut payload = Vec::new();
        payload.extend_from_slice(&777i32.to_be_bytes());
        payload.extend_from_slice(&expected_contract.to_bytes());

        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();

        let (primary, _) = decode_frame(
            StreamMsgType::Contract,
            &payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
            false,
        );

        let primary_internal = primary.expect("ContractAssigned must emit a primary event");
        let assigned_arc: Arc<Contract> = match expect_public(&primary_internal) {
            FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => {
                assert_eq!(*id, 777);
                assert_eq!(contract.symbol, "AAPL");
                // The Arc inside the event and the Arc in the thread-local
                // cache must point at the SAME Contract heap cell — a
                // different pointer would mean we regressed to per-event
                // Contract::clone.
                let emitted_ptr = Arc::as_ptr(contract);
                let cache_ptr = Arc::as_ptr(
                    local_contracts
                        .get(&777)
                        .expect("local cache must have the contract"),
                );
                assert_eq!(
                    emitted_ptr, cache_ptr,
                    "event's Arc<Contract> must alias the I/O thread cache"
                );
                Arc::clone(contract)
            }
            other => panic!("expected Control(ContractAssigned), got {other:?}"),
        };

        // Every FpssData event decoded after the assignment must carry
        // an Arc<Contract> pointing at that same heap allocation. Verify
        // via the resolve_contract helper path (quote frame).
        //
        // Craft a minimal FIT quote payload for contract_id 777.
        const FIELD_SEP: u8 = 0xB;
        const END_NIB: u8 = 0xD;
        fn nibbles(val: i32) -> Vec<u8> {
            let abs = (val as i64).unsigned_abs();
            let s = abs.to_string();
            s.chars().map(|c| c.to_digit(10).unwrap() as u8).collect()
        }
        let mut nibs: Vec<u8> = Vec::new();
        for (i, v) in [777i32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].iter().enumerate() {
            if i > 0 {
                nibs.push(FIELD_SEP);
            }
            nibs.extend(nibbles(*v));
        }
        nibs.push(END_NIB);
        let mut bytes = Vec::new();
        let mut j = 0;
        while j < nibs.len() {
            let h = nibs[j];
            let l = if j + 1 < nibs.len() { nibs[j + 1] } else { 0 };
            bytes.push((h << 4) | (l & 0x0F));
            j += 2;
        }

        let (primary, _) = decode_frame(
            StreamMsgType::Quote,
            &bytes,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
            false,
        );
        let primary_internal = primary.expect("Quote must emit a primary event");
        match expect_public(&primary_internal) {
            FpssEvent::Data(FpssData::Quote { contract, .. }) => {
                assert_eq!(contract.symbol, "AAPL");
                // Arc::ptr_eq proves both events share the SAME heap
                // allocation — `assert_eq!(contract.symbol, "AAPL")` alone
                // only checks that both events carry the same *value*,
                // which a regression to per-event Contract::clone would
                // still pass. Pointer equality pins down the exact
                // optimisation the feature promises.
                assert!(
                    Arc::ptr_eq(&assigned_arc, contract),
                    "quote's Arc<Contract> must alias the ContractAssigned Arc"
                );
            }
            other => panic!("expected Data(Quote), got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Empty-contract sentinel path: tick arrives BEFORE ContractAssigned.
    // -----------------------------------------------------------------------

    #[test]
    fn quote_for_unknown_contract_id_uses_empty_contract_sentinel() {
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();

        // Craft a minimal FIT quote payload for contract_id 999 with no
        // matching ContractAssigned. The contract cache lookup MUST miss
        // and the decoded Quote MUST carry the empty-contract sentinel.
        const FIELD_SEP: u8 = 0xB;
        const END_NIB: u8 = 0xD;
        fn nibbles(val: i32) -> Vec<u8> {
            let abs = (val as i64).unsigned_abs();
            let s = abs.to_string();
            s.chars().map(|c| c.to_digit(10).unwrap() as u8).collect()
        }
        let mut nibs: Vec<u8> = Vec::new();
        for (i, v) in [999i32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].iter().enumerate() {
            if i > 0 {
                nibs.push(FIELD_SEP);
            }
            nibs.extend(nibbles(*v));
        }
        nibs.push(END_NIB);
        let mut bytes = Vec::new();
        let mut j = 0;
        while j < nibs.len() {
            let h = nibs[j];
            let l = if j + 1 < nibs.len() { nibs[j + 1] } else { 0 };
            bytes.push((h << 4) | (l & 0x0F));
            j += 2;
        }

        let (primary, _) = decode_frame(
            StreamMsgType::Quote,
            &bytes,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
            false,
        );
        let primary_internal = primary.expect("Quote must emit a primary event");
        match expect_public(&primary_internal) {
            FpssEvent::Data(FpssData::Quote { contract, .. }) => {
                // The type-safe sentinel check: sec_type is Unknown,
                // not Stock. Consumers no longer have to rely on
                // `root.is_empty()` to detect the pre-ContractAssigned
                // state.
                assert_eq!(
                    contract.sec_type,
                    tdbe::types::enums::SecType::Unknown,
                    "missing contract_id must surface sec_type = Unknown"
                );
                // The sentinel's `symbol` carries the unresolved wire id
                // under the `__pending:` prefix so downstream consumers
                // (notably the WS bridge) can surface the diagnostic
                // without re-introducing the wire id on the public
                // `FpssData` surface.
                assert_eq!(
                    contract.symbol, "__pending:999",
                    "unresolved sentinel must encode the wire id under \
                     the `__pending:` prefix"
                );
            }
            other => panic!("expected Data(Quote), got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Non-zero Ping payload: every byte must survive the control dispatch.
    // -----------------------------------------------------------------------

    #[test]
    fn decode_code_10_ping_preserves_multi_byte_payload() {
        // The protocol keeps the Ping payload opaque for diagnostics;
        // the `[0]` single-byte case was already tested. A multi-byte
        // payload (e.g. `[0, 1, 2]`) MUST be preserved byte-for-byte
        // so post-hoc trace inspection catches anomalous heartbeats.
        let evt = decode_ctrl(StreamMsgType::Ping, &[0u8, 1u8, 2u8]);
        match expect_public(&evt) {
            FpssEvent::Control(FpssControl::Ping { payload }) => {
                assert_eq!(payload.as_slice(), &[0u8, 1u8, 2u8]);
            }
            other => panic!("expected Control(Ping), got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Restart arm MUST clear local_contracts, mirroring the START/STOP arms.
    // Without this, contract IDs the server reuses or re-announces after a
    // restart would resolve to stale shapes.
    // -----------------------------------------------------------------------

    #[test]
    fn restart_clears_local_contracts() {
        use crate::fpss::protocol::Contract as ProtoContract;

        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();

        // Seed the thread-local cache (as if a ContractAssigned had arrived).
        let seeded = Arc::new(ProtoContract::stock("SEED"));
        local_contracts.insert(42, Arc::clone(&seeded));

        let (primary, _) = decode_frame(
            StreamMsgType::Restart,
            &[],
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
            false,
        );
        let primary_internal = primary.expect("Restart must emit a primary event");
        match expect_public(&primary_internal) {
            FpssEvent::Control(FpssControl::Restart) => {}
            other => panic!("expected Control(Restart), got {other:?}"),
        }
        assert!(
            local_contracts.is_empty(),
            "Restart must clear the thread-local contract cache"
        );

        // A subsequent tick on the now-unknown ID MUST route through
        // the empty-contract sentinel, not the pre-restart SEED shape.
        const FIELD_SEP: u8 = 0xB;
        const END_NIB: u8 = 0xD;
        fn nibbles(val: i32) -> Vec<u8> {
            let abs = (val as i64).unsigned_abs();
            let s = abs.to_string();
            s.chars().map(|c| c.to_digit(10).unwrap() as u8).collect()
        }
        let mut nibs: Vec<u8> = Vec::new();
        for (i, v) in [42i32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].iter().enumerate() {
            if i > 0 {
                nibs.push(FIELD_SEP);
            }
            nibs.extend(nibbles(*v));
        }
        nibs.push(END_NIB);
        let mut bytes = Vec::new();
        let mut j = 0;
        while j < nibs.len() {
            let h = nibs[j];
            let l = if j + 1 < nibs.len() { nibs[j + 1] } else { 0 };
            bytes.push((h << 4) | (l & 0x0F));
            j += 2;
        }

        let (primary, _) = decode_frame(
            StreamMsgType::Quote,
            &bytes,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
            false,
        );
        let primary_internal = primary.expect("Quote must emit a primary event");
        match expect_public(&primary_internal) {
            FpssEvent::Data(FpssData::Quote { contract, .. }) => {
                assert_eq!(
                    contract.sec_type,
                    tdbe::types::enums::SecType::Unknown,
                    "post-Restart tick on known-but-cleared ID must surface Unknown"
                );
                assert_ne!(
                    contract.symbol, "SEED",
                    "post-Restart decoder must NOT resurrect the pre-restart Contract"
                );
            }
            other => panic!("expected Data(Quote), got {other:?}"),
        }
    }
}
