//! FPSS frame decoder: wire frame -> typed [`StreamEvent`] pairs.
//!
//! [`decode_frame`] is the dispatch core of the I/O loop. It runs FIT
//! decompression through [`super::delta::DeltaState`] and emits the typed
//! event a frame decodes to. OHLCVC bars arrive as their own wire frames
//! (code 24) and decode alongside the other events; the decoder emits
//! no derived events (for example, no trade-to-OHLCVC).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use crate::tdbe::types::enums::StreamMsgType;
use crate::tdbe::types::price::Price;
use metrics::Counter;

use super::delta::{DeltaState, TickFields, OHLCVC_FIELDS, OI_FIELDS, QUOTE_FIELDS, TRADE_FIELDS};
use super::events::{FpssEventInternal, StreamControl, StreamData};
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
static FPSS_MARKET_VALUE_EVENTS: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.events", "kind" => "market_value"));

static FPSS_QUOTE_DECODE_FAILURES: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.decode_failures", "kind" => "quote"));
static FPSS_TRADE_DECODE_FAILURES: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.decode_failures", "kind" => "trade"));
static FPSS_OI_DECODE_FAILURES: LazyLock<Counter> = LazyLock::new(
    || metrics::counter!("thetadatadx.fpss.decode_failures", "kind" => "open_interest"),
);
static FPSS_OHLCVC_DECODE_FAILURES: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.decode_failures", "kind" => "ohlcvc"));
static FPSS_MARKET_VALUE_DECODE_FAILURES: LazyLock<Counter> = LazyLock::new(
    || metrics::counter!("thetadatadx.fpss.decode_failures", "kind" => "market_value"),
);

static FPSS_INVALID_PRICE_TYPE_QUOTE: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.invalid_price_type", "kind" => "quote"));
static FPSS_INVALID_PRICE_TYPE_TRADE: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.invalid_price_type", "kind" => "trade"));
static FPSS_INVALID_PRICE_TYPE_OHLCVC: LazyLock<Counter> =
    LazyLock::new(|| metrics::counter!("thetadatadx.fpss.invalid_price_type", "kind" => "ohlcvc"));
static FPSS_INVALID_PRICE_TYPE_MARKET_VALUE: LazyLock<Counter> = LazyLock::new(
    || metrics::counter!("thetadatadx.fpss.invalid_price_type", "kind" => "market_value"),
);

/// Reassemble an FPSS wire `Price` cell. Returns `None` when
/// `price_type` is outside `0..=crate::tdbe::types::price::MAX_PRICE_TYPE`.
#[inline]
fn strict_fpss_price(value: i32, price_type: i32) -> Option<f64> {
    Price::with_value_and_type(value, price_type)
        .ok()
        .map(|p| p.to_f64())
}

/// Calculated market bid/ask, in raw wire-integer price units (the same
/// scale as the source quote's `bid` / `ask` cells — convert to dollars
/// only at the typed boundary via [`strict_fpss_price`]).
///
/// # Algorithm
///
/// Market value is a calculated theoretical price derived from the quote
/// bid/ask, not a raw field. This is the SDK's own market-value
/// calculation, computed to agree with the JVM terminal's per-contract
/// market value so the no-JVM SDK and the JVM terminal report the same
/// number for the same quote.
///
/// The market ask is derived first and replaces the ask cell, then the
/// market bid is derived from the original bid and the *original* ask.
/// Two ordering facts are load-bearing:
///
/// * The market bid is computed after the ask cell already holds
///   `market_ask`, so every ask the bid branches re-read (the
///   bid-equals-ask short-circuit, the `bid < ask` comparison, and the
///   `max(ask - 1, 0)` clamp) is `market_ask`, never the raw ask.
/// * The raw ask is used only for the bid-equals-ask equality test.
///
/// ```text
/// market_ask:
///     if (ask_size > bid_size)     ask + 1
///     else if (ask <= 2)           ask
///     else                         ask - 1
///
/// // the ask cell now holds market_ask; `raw_ask` is the original ask.
/// market_bid:
///     if (bid == raw_ask)          market_ask
///     else if (ask_size > bid_size) (bid <= 1) ? bid : bid - 1
///     else if (bid == 0)           0
///     else if (bid < market_ask)   min(max(market_ask - 1, 0), bid + 1)
///     else                         bid + 1
/// ```
///
/// `bid` / `ask` here are the raw integer quote cells (`buf[3]` / `buf[7]`),
/// `bid_size` / `ask_size` are `buf[1]` / `buf[5]`. The `+1` / `-1` nudges
/// are in wire-integer price units; the result is reassembled to dollars
/// at the typed boundary like every other price field, so it matches the
/// terminal's market value to the cent.
//
// The ±1 nudges saturate at the `i32` bounds. On a well-formed quote the
// operands are real wire prices nowhere near `i32::MAX` / `i32::MIN`, so
// saturation never engages. It exists only to keep the decoder total over
// the full `i32` domain: a malformed or adversarial frame can decode to a
// degenerate `i32::MAX` / `i32::MIN` bid/ask, and the decoder must not panic
// on arbitrary wire bytes. Clamping a degenerate price by one unit is the
// correct floor/ceiling for a price nudge — the alternative (plain `+ 1`)
// panics in debug and wraps a degenerate max price to a degenerate min price
// in release, which is strictly worse.
#[inline]
fn calculate_market_value(bid: i32, ask: i32, bid_size: i32, ask_size: i32) -> (i32, i32) {
    let market_ask = if ask_size > bid_size {
        ask.saturating_add(1)
    } else if ask <= 2 {
        ask
    } else {
        ask.saturating_sub(1)
    };

    let market_bid = if bid == ask {
        // Bid equals the original ask: the market bid takes the value the
        // ask cell now holds, i.e. `market_ask`.
        market_ask
    } else if ask_size > bid_size {
        if bid <= 1 {
            bid
        } else {
            bid.saturating_sub(1)
        }
    } else if bid == 0 {
        0
    } else if bid < market_ask {
        // Compare and clamp against the already-derived `market_ask`, not
        // the raw ask, so the result agrees with the terminal.
        market_ask
            .saturating_sub(1)
            .max(0)
            .min(bid.saturating_add(1))
    } else {
        bid.saturating_add(1)
    };

    (market_bid, market_ask)
}

/// Integer midpoint of the market bid/ask, computed as the terminal does
/// for a quote price: `bid/2 + ask/2 + (bid%2 + ask%2)/2` (overflow-safe
/// floor of the mean). In wire-integer units; converted to dollars at the
/// boundary.
#[inline]
fn market_value_midpoint(market_bid: i32, market_ask: i32) -> i32 {
    market_bid / 2 + market_ask / 2 + (market_bid % 2 + market_ask % 2) / 2
}

/// Increment the per-kind invalid-price-type counter and emit a
/// rate-limited warning (one in every 1024 occurrences).
fn warn_invalid_price_type(kind: &'static str, contract_id: i32, price_type: i32) {
    static INVALID_COUNT: AtomicU64 = AtomicU64::new(0);
    let prev = INVALID_COUNT.fetch_add(1, Ordering::Relaxed);
    if prev.is_multiple_of(1024) {
        tracing::warn!(
            target: "thetadatadx::fpss::decode",
            kind,
            contract_id,
            price_type,
            invalid_count = prev + 1,
            "dropping FPSS frame: wire price_type outside 0..=MAX_PRICE_TYPE"
        );
    }
}

/// Prefix on the [`Contract::symbol`] of an unresolved-contract sentinel
/// returned for a tick that arrived before the matching
/// `ContractAssigned` frame. The numeric wire id of the unresolved
/// contract follows the prefix verbatim (e.g. `"__pending:42"`).
///
/// Downstream consumers (notably the WS bridge) parse the suffix back
/// into an `i32` to surface `unresolved_contract_id` to operators
/// without re-introducing the wire id on the public `StreamData` surface.
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
        symbol: Arc::from(format!("{UNRESOLVED_CONTRACT_SYMBOL_PREFIX}{contract_id}").as_str()),
        sec_type: crate::tdbe::types::enums::SecType::Unknown,
        expiration: None,
        is_call: None,
        strike_thousandths: None,
    })
}

/// Decode a frame into zero or one `StreamEvent`.
///
/// Returns `Some(event)` for a frame that decodes to a typed event, `None`
/// for a frame that emits nothing (a DATE marker or a skipped binary Error
/// frame). Emitting straight into the ring keeps the hot path free of any
/// per-frame `Vec<StreamEvent>` allocation.
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
) -> Option<FpssEventInternal> {
    // Capture wall-clock timestamp once per frame for all data variants.
    //
    // On clock skew (`SystemTime::now()` before UNIX_EPOCH — possible on
    // a misconfigured host or virtualised guest with a buggy paravirtual
    // clock) we surface `received_at_ns = 0` and emit a rate-limited
    // warning. Consumers distinguishing a genuine epoch-zero timestamp
    // from the skew sentinel must cross-check the warn log.
    let received_at_ns = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        // u128 → u64 saturates past 2554-07-21T23:34:33Z (when ns since
        // UNIX_EPOCH first exceeds 2^64). `as u64` would wrap to a
        // misleading early-1970 timestamp; `try_from` + `unwrap_or` clamps
        // to the schema sentinel without panicking on the boundary.
        Ok(d) => u64::try_from(d.as_nanos()).unwrap_or(u64::MAX),
        Err(e) => {
            static FAIL_COUNT: AtomicU64 = AtomicU64::new(0);
            let prev = FAIL_COUNT.fetch_add(1, Ordering::Relaxed);
            // Rate-limit at 1024 to match the panic-count warn cadence.
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

    // Resolve contract_id via the thread-local cache. Hit: Arc::clone
    // (zero-alloc refcount bump). Miss: per-tick unresolved-sentinel
    // (`__pending:<id>`); downstream consumers detect via
    // `sec_type == SecType::Unknown`. Misses are bounded by the brief
    // window between first tick and the matching `ContractAssigned`.
    let resolve_contract =
        |contract_id: i32, cache: &HashMap<i32, Arc<Contract>>| -> Arc<Contract> {
            cache
                .get(&contract_id)
                .map(Arc::clone)
                .unwrap_or_else(|| unresolved_sentinel(contract_id))
        };

    // Warn on contract-id misses outside the post-STOP suppression
    // window, rate-limited at every 1024th hit (matches the slow-
    // callback / clock-skew warn cadence). MISS_COUNT is process-
    // global so the "1 of every 1024" rate aggregates across every
    // StreamingClient in the same process.
    let warn_unknown_contract =
        |contract_id: i32, kind: &str, cache: &HashMap<i32, Arc<Contract>>| {
            if !cache.contains_key(&contract_id) {
                static MISS_COUNT: AtomicU64 = AtomicU64::new(0);
                let prev = MISS_COUNT.fetch_add(1, Ordering::Relaxed);
                if prev.is_multiple_of(1024) {
                    tracing::warn!(
                        contract_id,
                        kind,
                        miss_count = prev + 1,
                        "no contract for ID (1 of every 1024 emitted across all streaming clients in this process)"
                    );
                }
            }
        };

    // Stack-allocated tick buffer reused across every FIT-decoded arm. The
    // decoder writes the absolute field values directly here; the match arm
    // reads `buf[i]` to construct the public `StreamData` variant. Sized at
    // the widest tick shape (`MAX_DATA_FIELDS`) so every arm shares one
    // buffer with zero heap traffic on the decode hot path.
    let mut buf: TickFields = [0; super::delta::MAX_DATA_FIELDS];

    match code {
        StreamMsgType::Metadata => {
            // Can arrive again after reconnection.
            // The payload is the server's opaque "Bundle" string -- see
            // StreamControl::LoginSuccess docs for why we don't parse it.
            let permissions = String::from_utf8_lossy(payload).to_string();
            // The Bundle string carries the account's subscription scope
            // (e.g. `STOCK.PRO, OPTION.PRO, INDEX.PRO`) — operationally
            // useful but account-identifying, so log it at `trace!` where
            // a production deployment will not capture it by default.
            tracing::trace!(permissions = %permissions, "received METADATA");
            authenticated.store(true, Ordering::Release);
            Some(FpssEventInternal::Control(StreamControl::LoginSuccess {
                permissions,
            }))
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
                Some(FpssEventInternal::Control(
                    StreamControl::ContractAssigned {
                        id,
                        contract: arc_contract,
                    },
                ))
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse CONTRACT message");
                Some(FpssEventInternal::Control(StreamControl::Error {
                    message: format!("failed to parse CONTRACT message: {e}"),
                }))
            }
        },

        StreamMsgType::Quote => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, QUOTE_FIELDS, &mut buf) {
                Some((contract_id, _n)) => {
                    warn_unknown_contract(contract_id, "quote", local_contracts);
                    let pt = buf[9];
                    let (Some(bid_f64), Some(ask_f64)) =
                        (strict_fpss_price(buf[3], pt), strict_fpss_price(buf[7], pt))
                    else {
                        FPSS_QUOTE_DECODE_FAILURES.increment(1);
                        FPSS_INVALID_PRICE_TYPE_QUOTE.increment(1);
                        warn_invalid_price_type("quote", contract_id, pt);
                        return Some(FpssEventInternal::Unparseable);
                    };
                    FPSS_QUOTE_EVENTS.increment(1);
                    Some(FpssEventInternal::Data(StreamData::Quote {
                        contract: resolve_contract(contract_id, local_contracts),
                        ms_of_day: buf[0],
                        bid_size: buf[1],
                        bid_exchange: buf[2],
                        bid: bid_f64,
                        bid_condition: buf[4],
                        ask_size: buf[5],
                        ask_exchange: buf[6],
                        ask: ask_f64,
                        ask_condition: buf[8],
                        date: buf[10],
                        received_at_ns,
                    }))
                }
                // DATE markers return None from decode_tick -- this is normal
                // protocol flow (session date boundary), not corruption.
                None if delta_state.last_was_date => None,
                None => {
                    // Truncated / corrupt FIT payload. Account for it on
                    // the public counter so operators see decode pressure
                    // without raw-byte fallout reaching the user callback.
                    FPSS_QUOTE_DECODE_FAILURES.increment(1);
                    Some(FpssEventInternal::Unparseable)
                }
            }
        }

        StreamMsgType::Trade => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, TRADE_FIELDS, &mut buf) {
                Some((contract_id, _n)) => {
                    warn_unknown_contract(contract_id, "trade", local_contracts);

                    // `decode_tick` rejects any row whose width is not exactly
                    // TRADE_FIELDS (the only stream-trade layout; the 16-field
                    // "extended" trade is an MDDS gRPC shape on a different
                    // protocol and never reaches this decoder), so a `Some`
                    // here is guaranteed to be the 8-field layout.
                    // 8-field: [ms_of_day, sequence, size, condition, price, exchange, price_type, date]
                    let (price_idx, pt_idx) = (4, 6);
                    let pt = buf[pt_idx];
                    let Some(price_f64) = strict_fpss_price(buf[price_idx], pt) else {
                        FPSS_TRADE_DECODE_FAILURES.increment(1);
                        FPSS_INVALID_PRICE_TYPE_TRADE.increment(1);
                        warn_invalid_price_type("trade", contract_id, pt);
                        return Some(FpssEventInternal::Unparseable);
                    };
                    FPSS_TRADE_EVENTS.increment(1);

                    let contract_arc = resolve_contract(contract_id, local_contracts);
                    let trade_event = FpssEventInternal::Data(StreamData::Trade {
                        contract: contract_arc,
                        ms_of_day: buf[0],
                        sequence: buf[1],
                        condition: buf[3],
                        size: buf[2],
                        exchange: buf[5],
                        price: price_f64,
                        date: buf[7],
                        received_at_ns,
                    });
                    Some(trade_event)
                }
                // DATE markers return None from decode_tick -- normal protocol flow.
                None if delta_state.last_was_date => None,
                None => {
                    FPSS_TRADE_DECODE_FAILURES.increment(1);
                    Some(FpssEventInternal::Unparseable)
                }
            }
        }

        StreamMsgType::OpenInterest => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, OI_FIELDS, &mut buf) {
                Some((contract_id, _n)) => {
                    warn_unknown_contract(contract_id, "open_interest", local_contracts);
                    FPSS_OI_EVENTS.increment(1);
                    Some(FpssEventInternal::Data(StreamData::OpenInterest {
                        contract: resolve_contract(contract_id, local_contracts),
                        ms_of_day: buf[0],
                        open_interest: buf[1],
                        date: buf[2],
                        received_at_ns,
                    }))
                }
                None if delta_state.last_was_date => None,
                None => {
                    FPSS_OI_DECODE_FAILURES.increment(1);
                    Some(FpssEventInternal::Unparseable)
                }
            }
        }

        StreamMsgType::Ohlcvc => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, OHLCVC_FIELDS, &mut buf) {
                Some((contract_id, _n)) => {
                    warn_unknown_contract(contract_id, "ohlcvc", local_contracts);
                    let pt = buf[7];
                    let (Some(o), Some(h), Some(l), Some(c)) = (
                        strict_fpss_price(buf[1], pt),
                        strict_fpss_price(buf[2], pt),
                        strict_fpss_price(buf[3], pt),
                        strict_fpss_price(buf[4], pt),
                    ) else {
                        FPSS_OHLCVC_DECODE_FAILURES.increment(1);
                        FPSS_INVALID_PRICE_TYPE_OHLCVC.increment(1);
                        warn_invalid_price_type("ohlcvc", contract_id, pt);
                        return Some(FpssEventInternal::Unparseable);
                    };
                    FPSS_OHLCVC_EVENTS.increment(1);
                    // Cumulative volume (buf[5]) and trade count (buf[6]) are
                    // unsigned 32-bit wire fields. Widen through `u32` so a
                    // value above `i32::MAX` (a normal full-session volume on
                    // a liquid symbol) lands as a positive `i64` instead of
                    // being sign-extended into a negative number.
                    let volume = i64::from(buf[5] as u32);
                    let count = i64::from(buf[6] as u32);
                    Some(FpssEventInternal::Data(StreamData::Ohlcvc {
                        contract: resolve_contract(contract_id, local_contracts),
                        ms_of_day: buf[0],
                        open: o,
                        high: h,
                        low: l,
                        close: c,
                        volume,
                        count,
                        date: buf[8],
                        received_at_ns,
                    }))
                }
                None if delta_state.last_was_date => None,
                None => {
                    FPSS_OHLCVC_DECODE_FAILURES.increment(1);
                    Some(FpssEventInternal::Unparseable)
                }
            }
        }

        StreamMsgType::MarketValue => {
            let msg_code = code as u8;
            // The MARKET_VALUE frame carries the same 11-field FIT quote
            // layout as a Quote frame, so decode it with `QUOTE_FIELDS`,
            // then apply the market-value calculation to the decoded
            // bid/ask.
            match delta_state.decode_tick(msg_code, payload, QUOTE_FIELDS, &mut buf) {
                Some((contract_id, _n)) => {
                    warn_unknown_contract(contract_id, "market_value", local_contracts);
                    let pt = buf[9];
                    // Compute market value on the raw integer bid/ask/sizes,
                    // keeping wire scale, then reassemble to dollars at the
                    // boundary like every other price field.
                    let (market_bid_i, market_ask_i) =
                        calculate_market_value(buf[3], buf[7], buf[1], buf[5]);
                    let market_price_i = market_value_midpoint(market_bid_i, market_ask_i);
                    let (Some(market_bid), Some(market_ask), Some(market_price)) = (
                        strict_fpss_price(market_bid_i, pt),
                        strict_fpss_price(market_ask_i, pt),
                        strict_fpss_price(market_price_i, pt),
                    ) else {
                        FPSS_MARKET_VALUE_DECODE_FAILURES.increment(1);
                        FPSS_INVALID_PRICE_TYPE_MARKET_VALUE.increment(1);
                        warn_invalid_price_type("market_value", contract_id, pt);
                        return Some(FpssEventInternal::Unparseable);
                    };
                    FPSS_MARKET_VALUE_EVENTS.increment(1);
                    Some(FpssEventInternal::Data(StreamData::MarketValue {
                        contract: resolve_contract(contract_id, local_contracts),
                        ms_of_day: buf[0],
                        market_bid,
                        market_ask,
                        market_price,
                        date: buf[10],
                        received_at_ns,
                    }))
                }
                None if delta_state.last_was_date => None,
                None => {
                    FPSS_MARKET_VALUE_DECODE_FAILURES.increment(1);
                    Some(FpssEventInternal::Unparseable)
                }
            }
        }

        StreamMsgType::ReqResponse => match parse_req_response(payload) {
            Ok((req_id, result)) => {
                tracing::debug!(req_id, result = ?result, "subscription response");
                Some(FpssEventInternal::Control(StreamControl::ReqResponse {
                    req_id,
                    result,
                }))
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse REQ_RESPONSE");
                Some(FpssEventInternal::Control(StreamControl::Error {
                    message: format!("failed to parse REQ_RESPONSE: {e}"),
                }))
            }
        },

        StreamMsgType::Start => {
            tracing::info!("market open signal received");
            delta_state.clear();
            local_contracts.clear(); // mirrors idToContract.clear() on the wire
            Some(FpssEventInternal::Control(StreamControl::MarketOpen))
        }

        StreamMsgType::Stop => {
            tracing::info!("market close signal received");
            delta_state.clear();
            local_contracts.clear(); // mirrors idToContract.clear() on the wire
            Some(FpssEventInternal::Control(StreamControl::MarketClose))
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
                None
            } else {
                let message = String::from_utf8_lossy(payload).to_string();
                tracing::warn!(message = %message, "server error");
                Some(FpssEventInternal::Control(StreamControl::ServerError {
                    message,
                }))
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

            Some(FpssEventInternal::Control(StreamControl::Disconnected {
                reason,
            }))
        }

        // Known server→client control frames. Each of these previously
        // fell through to `UnknownFrame`, leaving consumers to filter
        // noise they did not ask for. Each now maps to its own typed
        // `StreamControl` variant so downstream code can match directly.
        StreamMsgType::Connected => {
            // Code 4: connection ack. Logs "connected" and returns — no
            // side effects other than acknowledging the transition.
            tracing::debug!("FPSS server CONNECTED frame received");
            Some(FpssEventInternal::Control(StreamControl::Connected))
        }

        StreamMsgType::Ping => {
            // Code 10: server heartbeat. Observed payload is a single
            // zero byte `[0]`; the client does NOT respond — the client
            // itself sends its own independent 100ms pings. Preserve the
            // raw payload for diagnostics so anomalous heartbeats can be
            // inspected after-the-fact.
            Some(FpssEventInternal::Control(StreamControl::Ping {
                payload: payload.to_vec(),
            }))
        }

        StreamMsgType::Reconnected => {
            // Code 13: server-side reconnect ack. Distinct from
            // `StreamControl::Reconnected` which the client emits when its
            // own auto-reconnect state machine completes. Both can be
            // observed in the same session — e.g. a client-side
            // reconnect produces `Reconnected`, while a transparent
            // server-side reconnect produces `ReconnectedServer`.
            //
            // A server-side reconnect re-establishes the upstream session
            // WITHOUT dropping the client's TCP socket, so the io_loop's own
            // reconnect path (which clears delta state) never runs. The fresh
            // server backend restarts its FIT delta stream — the first tick
            // per contract is absolute again — and may re-announce contract
            // IDs with different shapes. Decoding those fresh absolute rows
            // against the pre-reconnect baseline accumulates deltas onto stale
            // values and mangles every subsequent tick until a START/STOP/
            // Restart clears it. Mirror the START/STOP/Restart arms: clear the
            // delta decode state AND both contract caches so post-reconnect
            // ticks decode as fresh baselines.
            tracing::debug!("FPSS server RECONNECTED frame received");
            delta_state.clear();
            local_contracts.clear();
            Some(FpssEventInternal::Control(StreamControl::ReconnectedServer))
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
            Some(FpssEventInternal::Control(StreamControl::Restart))
        }

        // Emit unrecognized frame codes as UnknownFrame events with raw
        // payload bytes preserved. This lets users capture broken frames
        // for upstream bug reports instead of silently dropping them.
        other => {
            tracing::warn!(code = ?other, payload_len = payload.len(), "unrecognized FPSS frame code");
            Some(FpssEventInternal::Control(StreamControl::UnknownFrame {
                code: other as u8,
                payload: payload.to_vec(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpss::StreamEvent;

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
    // 8-field trade mapping (drives the full `decode_frame` pipeline)
    // -----------------------------------------------------------------------

    /// Drive `decode_frame` with a synthetic FIT-encoded `Trade` frame
    /// and assert on the resulting `StreamEvent::Data(StreamData::Trade{..})`.
    /// Asserting on the production decode-entry point pins the
    /// integration contract (FIT decode + tick-buffer -> field
    /// extraction + Arc<Contract> resolution + Price reassembly) rather
    /// than re-implementing the mapping inside the test body.
    #[test]
    fn decode_frame_8field_trade_emits_trade_data_with_correct_fields() {
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

        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        let aapl: Arc<Contract> = Arc::new(Contract::stock("AAPL"));
        local_contracts.insert(100, Arc::clone(&aapl));

        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::Trade,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        let evt = primary.expect("decode_frame must emit a primary Trade event");
        let public = expect_public(&evt);
        match public {
            StreamEvent::Data(StreamData::Trade {
                contract,
                ms_of_day,
                sequence,
                size,
                condition,
                price,
                exchange,
                date,
                ..
            }) => {
                // Contract resolves via the seeded local_contracts cache
                // (Arc refcount-clone, no unresolved-sentinel sym leak).
                assert_eq!(&*contract.symbol, "AAPL");
                assert_eq!(*ms_of_day, 34200000);
                assert_eq!(*sequence, 12345);
                assert_eq!(*size, 50);
                assert_eq!(*condition, 6);
                assert!(
                    (*price - Price::new(5500000, 6).to_f64()).abs() < f64::EPSILON,
                    "price must reassemble via Price::new(value, price_type)"
                );
                assert_eq!(*exchange, 57);
                assert_eq!(*date, 20250428);
            }
            other => panic!("expected StreamEvent::Data(Trade), got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Off-spec trade width rejection
    // -----------------------------------------------------------------------

    /// A trade row whose data-field count is not 8 has no known cell
    /// layout: forcing the 8-field index set onto it would read
    /// never-populated slots as price / price_type / date and emit a
    /// silently-wrong tick, and because the width is cached per contract it
    /// would mis-decode every later row too. The decoder must reject the row
    /// as `Unparseable` instead of continuing, mirroring how the
    /// invalid-price_type path handles a bad trade.
    #[test]
    fn decode_frame_offspec_trade_width_is_unparseable_not_silently_wrong() {
        // 10 data fields (+ contract_id = 11 FIT fields): a width the
        // decoder has no layout for.
        let fit_payload = encode_fit_row(&[
            300, // contract_id
            34200000, 12345, 50, 6, 5500000, 57, 6, 20250428, 1, 2,
        ]);

        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(300, Arc::new(Contract::stock("AAPL")));

        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::Trade,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );

        assert!(
            matches!(primary, Some(FpssEventInternal::Unparseable)),
            "an off-spec trade width must decode as Unparseable, got {primary:?}"
        );
    }

    /// A complete-but-narrow quote row (fewer than QUOTE_FIELDS data fields)
    /// must reject as `Unparseable`, not emit a tick whose trailing fields
    /// (date, price_type index) are silently read from never-populated slots.
    /// The Quote arm reads fixed indices, so without a width guard a short row
    /// would surface a zero-filled `date`/wrong price_type as real data.
    #[test]
    fn decode_frame_narrow_quote_width_is_unparseable_not_silently_wrong() {
        // 9 data fields (+ contract_id = 10 FIT fields); QUOTE_FIELDS is 11.
        let fit_payload = encode_fit_row(&[400, 34200000, 10, 5, 15025, 0, 20, 6, 15030, 0]);

        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(400, Arc::new(Contract::stock("AAPL")));

        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::Quote,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );

        assert!(
            matches!(primary, Some(FpssEventInternal::Unparseable)),
            "a narrow quote width must decode as Unparseable, got {primary:?}"
        );
    }

    /// A complete-but-narrow OHLCVC row (fewer than OHLCVC_FIELDS data fields)
    /// must reject as `Unparseable` rather than emit a bar whose high/low/close
    /// or trailing volume/count/date come from unpopulated slots.
    #[test]
    fn decode_frame_narrow_ohlcvc_width_is_unparseable_not_silently_wrong() {
        // 7 data fields (+ contract_id = 8 FIT fields); OHLCVC_FIELDS is 9.
        let fit_payload = encode_fit_row(&[500, 34200000, 15000, 15100, 14900, 15050, 1000, 6]);

        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(500, Arc::new(Contract::stock("AAPL")));

        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::Ohlcvc,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );

        assert!(
            matches!(primary, Some(FpssEventInternal::Unparseable)),
            "a narrow ohlcvc width must decode as Unparseable, got {primary:?}"
        );
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
        let primary = decode_frame(
            code,
            payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        primary.expect("decode_frame must emit a primary event for known control codes")
    }

    /// Reborrow a primary `FpssEventInternal` from `decode_frame` as a
    /// public `&StreamEvent` for assertions, panicking on the
    /// internal-only variants the tests do not expect.
    fn expect_public(evt: &FpssEventInternal) -> &StreamEvent {
        evt.as_public()
            .expect("decode_frame primary event must reborrow as &StreamEvent")
    }

    #[test]
    fn decode_code_4_connected_emits_typed_variant() {
        let evt = decode_ctrl(StreamMsgType::Connected, &[]);
        match expect_public(&evt) {
            StreamEvent::Control(StreamControl::Connected) => {}
            other => panic!("expected Control(Connected), got {other:?}"),
        }
    }

    #[test]
    fn decode_code_10_ping_emits_typed_variant_with_payload() {
        // Observed on production FPSS streams: 1-byte payload `[0]`.
        let evt = decode_ctrl(StreamMsgType::Ping, &[0u8]);
        match expect_public(&evt) {
            StreamEvent::Control(StreamControl::Ping { payload }) => {
                assert_eq!(payload.as_slice(), &[0u8]);
            }
            other => panic!("expected Control(Ping), got {other:?}"),
        }
    }

    #[test]
    fn decode_code_13_reconnected_server_emits_typed_variant() {
        let evt = decode_ctrl(StreamMsgType::Reconnected, &[]);
        match expect_public(&evt) {
            StreamEvent::Control(StreamControl::ReconnectedServer) => {}
            other => panic!("expected Control(ReconnectedServer), got {other:?}"),
        }
    }

    #[test]
    fn decode_code_31_restart_emits_typed_variant_and_clears_delta_state() {
        // Seed delta state so we can verify it was cleared.
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(42, Arc::new(Contract::stock("AAPL")));
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        // Seed a real absolute trade so the delta-decode baseline is
        // populated; the Restart arm must clear it.
        let seed_row = encode_fit_row(&[42, 34_200_000, 1, 50, 6, 5_500_000, 57, 6, 20_250_428]);
        let seed = decode_frame(
            StreamMsgType::Trade,
            &seed_row,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        assert!(seed.is_some(), "seed trade must decode");
        assert_ne!(
            delta_state.state_sizes().0,
            0,
            "seed trade must populate the delta baseline"
        );

        let primary = decode_frame(
            StreamMsgType::Restart,
            &[],
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        let primary_internal = primary.expect("Restart must emit a primary event");
        match expect_public(&primary_internal) {
            StreamEvent::Control(StreamControl::Restart) => {}
            other => panic!("expected Control(Restart), got {other:?}"),
        }
        assert_eq!(
            delta_state.state_sizes().0,
            0,
            "Restart must clear delta state so downstream deltas don't \
             decode against a stale baseline"
        );
    }

    /// A server-side reconnect (code 13, `ReconnectedServer`) re-establishes
    /// the upstream session without dropping the client's TCP socket, so the
    /// io_loop's own reconnect clear never runs. The fresh backend restarts
    /// its FIT delta stream: the first tick per contract is absolute again.
    /// The `Reconnected` arm must clear the delta-decode state so that fresh
    /// absolute row decodes as a clean baseline. Without the clear the row is
    /// mistaken for a delta and accumulated onto the stale pre-reconnect
    /// baseline, mangling the tick — the user-reported post-reconnect desync.
    ///
    /// Pins the value path (the emitted trade price), which is unambiguous:
    /// with the clear the post-reconnect absolute price is exactly the fresh
    /// row's price; without it the price is `fresh + stale` and the assertion
    /// fails. This is the test that fails before the fix and passes after.
    #[test]
    fn server_reconnect_resets_delta_state_so_next_tick_is_fresh_baseline() {
        let cid = 200;
        // An 8-field FPSS stream trade row for `cid`, price cell = 18_750_000.
        let abs_row = |price: i32| {
            encode_fit_row(&[
                cid,        // contract_id
                34_200_000, // ms_of_day
                1,          // sequence
                100,        // size
                0,          // condition
                price,      // price
                57,         // exchange
                8,          // price_type
                20_250_428, // date
            ])
        };

        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(cid, Arc::new(Contract::stock("SPY")));
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();

        let price_of = |evt: Option<FpssEventInternal>| -> f64 {
            match expect_public(&evt.expect("trade must emit a primary event")) {
                StreamEvent::Data(StreamData::Trade { price, .. }) => *price,
                other => panic!("expected Data(Trade), got {other:?}"),
            }
        };

        // 1) Seed an absolute baseline trade for `cid`.
        let baseline_price = 18_750_000;
        let p0 = decode_frame(
            StreamMsgType::Trade,
            &abs_row(baseline_price),
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        let p0 = price_of(p0);

        // 2) Server-side transparent reconnect: socket stays up, server
        //    restarts its delta stream. This arm must clear delta state.
        let rc = decode_frame(
            StreamMsgType::Reconnected,
            &[],
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        match expect_public(&rc.expect("Reconnected must emit a primary event")) {
            StreamEvent::Control(StreamControl::ReconnectedServer) => {}
            other => panic!("expected Control(ReconnectedServer), got {other:?}"),
        }

        // 3) Fresh absolute trade from the restarted stream, different price.
        //    The contract cache was cleared too, so re-announce it (mirrors
        //    the server re-sending CONTRACT after a restart).
        local_contracts.insert(cid, Arc::new(Contract::stock("SPY")));
        let fresh_price = 99_990_000;
        let p1 = decode_frame(
            StreamMsgType::Trade,
            &abs_row(fresh_price),
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        let p1 = price_of(p1);

        let expected = Price::new(fresh_price, 8).to_f64();
        assert!(
            (p0 - Price::new(baseline_price, 8).to_f64()).abs() < f64::EPSILON,
            "baseline trade must decode to its absolute price"
        );
        // The load-bearing assertion: post-reconnect the price is the FRESH
        // absolute value, not `fresh + stale`. Without the `Reconnected` clear
        // the row is treated as a delta and `price` decodes to
        // `(fresh + baseline)` (a mangled tick) and this fails.
        assert!(
            (p1 - expected).abs() < f64::EPSILON,
            "post server-reconnect trade must decode as a FRESH absolute \
             baseline (expected price {expected}, got {p1}); a stale baseline \
             would accumulate the fresh row as a delta and mangle the tick"
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

        let primary = decode_frame(
            StreamMsgType::Contract,
            &payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );

        let primary_internal = primary.expect("ContractAssigned must emit a primary event");
        let assigned_arc: Arc<Contract> = match expect_public(&primary_internal) {
            StreamEvent::Control(StreamControl::ContractAssigned { id, contract }) => {
                assert_eq!(*id, 777);
                assert_eq!(&*contract.symbol, "AAPL");
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

        // Every StreamData event decoded after the assignment must carry
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

        let primary = decode_frame(
            StreamMsgType::Quote,
            &bytes,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        let primary_internal = primary.expect("Quote must emit a primary event");
        match expect_public(&primary_internal) {
            StreamEvent::Data(StreamData::Quote { contract, .. }) => {
                assert_eq!(&*contract.symbol, "AAPL");
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

        let primary = decode_frame(
            StreamMsgType::Quote,
            &bytes,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        let primary_internal = primary.expect("Quote must emit a primary event");
        match expect_public(&primary_internal) {
            StreamEvent::Data(StreamData::Quote { contract, .. }) => {
                // The type-safe sentinel check: sec_type is Unknown,
                // not Stock. Consumers no longer have to rely on
                // `root.is_empty()` to detect the pre-ContractAssigned
                // state.
                assert_eq!(
                    contract.sec_type,
                    crate::tdbe::types::enums::SecType::Unknown,
                    "missing contract_id must surface sec_type = Unknown"
                );
                // The sentinel's `symbol` carries the unresolved wire id
                // under the `__pending:` prefix so downstream consumers
                // (notably the WS bridge) can surface the diagnostic
                // without re-introducing the wire id on the public
                // `StreamData` surface.
                assert_eq!(
                    &*contract.symbol, "__pending:999",
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
            StreamEvent::Control(StreamControl::Ping { payload }) => {
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

        let primary = decode_frame(
            StreamMsgType::Restart,
            &[],
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        let primary_internal = primary.expect("Restart must emit a primary event");
        match expect_public(&primary_internal) {
            StreamEvent::Control(StreamControl::Restart) => {}
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

        let primary = decode_frame(
            StreamMsgType::Quote,
            &bytes,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        let primary_internal = primary.expect("Quote must emit a primary event");
        match expect_public(&primary_internal) {
            StreamEvent::Data(StreamData::Quote { contract, .. }) => {
                assert_eq!(
                    contract.sec_type,
                    crate::tdbe::types::enums::SecType::Unknown,
                    "post-Restart tick on known-but-cleared ID must surface Unknown"
                );
                assert_ne!(
                    &*contract.symbol, "SEED",
                    "post-Restart decoder must NOT resurrect the pre-restart Contract"
                );
            }
            other => panic!("expected Data(Quote), got {other:?}"),
        }
    }

    #[test]
    fn decode_frame_quote_invalid_price_type_drops_to_unparseable() {
        // 11-field quote layout (FIT prefix: contract_id):
        //   [contract_id, ms_of_day, bid_size, bid_exchange, bid,
        //    bid_condition, ask_size, ask_exchange, ask, ask_condition,
        //    price_type, date]
        let fit_payload = encode_fit_row(&[
            300, 34_200_000, 10, 4, 15_025, 0, 12, 4, 15_030, 0, 20, 20_250_428,
        ]);
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(300, Arc::new(Contract::stock("AAPL")));
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::Quote,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        match primary {
            Some(FpssEventInternal::Unparseable) => {}
            other => {
                panic!("expected Unparseable for out-of-range Quote price_type, got {other:?}")
            }
        }
    }

    #[test]
    fn decode_frame_8field_trade_invalid_price_type_drops_to_unparseable() {
        // 8-field trade layout:
        //   [contract_id, ms_of_day, sequence, size, condition,
        //    price, exchange, price_type, date]
        let fit_payload = encode_fit_row(&[
            400, 34_200_000, 12_345, 50, 6, 5_500_000, 57, 21, 20_250_428,
        ]);
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(400, Arc::new(Contract::stock("AAPL")));
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::Trade,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        match primary {
            Some(FpssEventInternal::Unparseable) => {}
            other => {
                panic!("expected Unparseable for out-of-range Trade price_type, got {other:?}")
            }
        }
    }

    #[test]
    fn decode_frame_ohlcvc_invalid_price_type_drops_to_unparseable() {
        // 9-field OHLCVC layout (server-seeded bar):
        //   [contract_id, ms_of_day, open, high, low, close, volume,
        //    count, price_type, date]
        let fit_payload = encode_fit_row(&[
            600, 34_200_000, 15_025, 15_100, 14_950, 15_080, 1_000, 10, -1, 20_250_428,
        ]);
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(600, Arc::new(Contract::stock("AAPL")));
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::Ohlcvc,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        match primary {
            Some(FpssEventInternal::Unparseable) => {}
            other => {
                panic!("expected Unparseable for negative Ohlcvc price_type, got {other:?}")
            }
        }
    }

    #[test]
    fn decode_frame_ohlcvc_volume_count_are_unsigned() {
        // Cumulative volume and trade count are unsigned 32-bit wire fields.
        // A full-session volume on a liquid symbol routinely exceeds
        // `i32::MAX`; on the wire that arrives as a 32-bit word whose top
        // bit is set. Feeding the signed-i32 bit patterns `-2_069_356_102`
        // (== `2_225_611_194_u32`) for volume and `-2_008_126_979`
        // (== `2_286_840_317_u32`) for count must decode to the positive
        // cumulative values, not sign-extended negatives.
        //
        // 9-field OHLCVC layout (server-seeded bar):
        //   [contract_id, ms_of_day, open, high, low, close, volume,
        //    count, price_type, date]
        let fit_payload = encode_fit_row(&[
            600,
            34_200_000,
            15_025,
            15_100,
            14_950,
            15_080,
            -2_069_356_102, // volume wire word (== 2_225_611_194_u32)
            -2_008_126_979, // count wire word  (== 2_286_840_317_u32)
            8,
            20_250_428,
        ]);
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(600, Arc::new(Contract::stock("QQQ")));
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::Ohlcvc,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        match primary {
            Some(FpssEventInternal::Data(StreamData::Ohlcvc { volume, count, .. })) => {
                assert_eq!(volume, 2_225_611_194_i64, "volume must decode as unsigned");
                assert_eq!(count, 2_286_840_317_i64, "count must decode as unsigned");
            }
            other => panic!("expected decoded Ohlcvc, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Market value calc — pure-function unit tests covering every branch of
    // the market bid/ask calculation. Expected values are computed by hand
    // from the formula (see `calculate_market_value` doc) in wire-integer
    // units, pinning agreement with the terminal's market value.
    // -----------------------------------------------------------------------

    #[test]
    fn calculate_market_value_balanced_book_below_ask() {
        // bid < market_ask, balanced sizes, ask > 2.
        // market_ask = ask-1 = 15029; market_bid = min(max(15028,0), 15026) = 15026.
        let (mb, ma) = calculate_market_value(15_025, 15_030, 10, 10);
        assert_eq!(ma, 15_029);
        assert_eq!(mb, 15_026);
        assert_eq!(market_value_midpoint(mb, ma), 15_027);
    }

    #[test]
    fn calculate_market_value_ask_heavy_imbalance() {
        // ask_size > bid_size: market_ask = ask+1; market_bid = bid-1.
        let (mb, ma) = calculate_market_value(15_025, 15_030, 5, 40);
        assert_eq!(ma, 15_031);
        assert_eq!(mb, 15_024);
        assert_eq!(market_value_midpoint(mb, ma), 15_027);
    }

    #[test]
    fn calculate_market_value_locked_bid_equals_ask() {
        // bid == raw ask short-circuits market_bid to the overwritten ask cell
        // (market_ask), which itself is ask-1 here.
        let (mb, ma) = calculate_market_value(15_030, 15_030, 10, 10);
        assert_eq!(ma, 15_029);
        assert_eq!(mb, 15_029);
        assert_eq!(market_value_midpoint(mb, ma), 15_029);
    }

    #[test]
    fn calculate_market_value_bid_at_or_above_market_ask() {
        // Balanced sizes, bid not < market_ask → market_bid = bid+1.
        let (mb, ma) = calculate_market_value(15_030, 15_031, 10, 10);
        assert_eq!(ma, 15_030);
        assert_eq!(mb, 15_031);
        assert_eq!(market_value_midpoint(mb, ma), 15_030);
    }

    #[test]
    fn calculate_market_value_tiny_ask_unchanged() {
        // ask <= 2 and balanced sizes leaves the ask cell unchanged.
        // bid(1) < market_ask(2) → min(max(2-1,0), 1+1) = min(1,2) = 1.
        let (mb, ma) = calculate_market_value(1, 2, 10, 10);
        assert_eq!(ma, 2);
        assert_eq!(mb, 1);
    }

    #[test]
    fn calculate_market_value_zero_bid_balanced_stays_zero() {
        // Balanced sizes, bid == 0 → market_bid = 0.
        let (mb, ma) = calculate_market_value(0, 50, 10, 10);
        assert_eq!(ma, 49);
        assert_eq!(mb, 0);
    }

    #[test]
    fn calculate_market_value_ask_heavy_bid_one_floor() {
        // ask_size > bid_size and bid <= 1 → market_bid = bid (no decrement).
        let (mb, ma) = calculate_market_value(1, 30, 1, 9);
        assert_eq!(ma, 31);
        assert_eq!(mb, 1);
    }

    /// A malformed or adversarial frame can decode to a degenerate
    /// `i32::MAX` / `i32::MIN` bid/ask. The ±1 nudges must saturate at the
    /// `i32` bounds rather than overflow — the decoder must not panic on
    /// arbitrary wire bytes. This pins the saturation on every branch that
    /// nudges by ±1, at both extremes and across the size-imbalance and
    /// equality paths.
    #[test]
    fn calculate_market_value_saturates_at_i32_extremes() {
        // ask_size > bid_size, ask == i32::MAX → `ask + 1` would overflow.
        // market_ask saturates to i32::MAX; bid != ask so the ask-heavy
        // bid branch decrements bid (15_000 - 1).
        let (mb, ma) = calculate_market_value(15_000, i32::MAX, 0, 1);
        assert_eq!(ma, i32::MAX);
        assert_eq!(mb, 14_999);

        // Balanced sizes, ask == i32::MIN (≤ 2) → ask cell unchanged; bid == 0
        // short-circuits market_bid to 0.
        let (mb, ma) = calculate_market_value(0, i32::MIN, 10, 10);
        assert_eq!(ma, i32::MIN);
        assert_eq!(mb, 0);

        // Balanced sizes, ask large (> 2) → `ask - 1`; bid == i32::MAX is not
        // < market_ask, so the final branch nudges `bid + 1` — saturates.
        let (mb, ma) = calculate_market_value(i32::MAX, i32::MAX - 1, 10, 10);
        assert_eq!(ma, i32::MAX - 2);
        assert_eq!(mb, i32::MAX);

        // Balanced sizes, bid == i32::MIN < market_ask → the clamp branch:
        // `(market_ask - 1).max(0).min(bid + 1)`. The `.max(0)` floors only
        // `market_ask - 1` (99 - 1 = 98); the `.min(bid + 1)` then takes the
        // saturating `bid + 1` (i32::MIN + 1), which is the smaller operand.
        let (mb, ma) = calculate_market_value(i32::MIN, 100, 10, 10);
        assert_eq!(ma, 99);
        assert_eq!(mb, i32::MIN + 1);

        // ask_size > bid_size with both bid and ask at i32::MIN. bid == ask
        // short-circuits market_bid to market_ask, which saturates on
        // `ask + 1` (i32::MIN + 1, no overflow) — exercises the equality path
        // at the low extreme.
        let (mb, ma) = calculate_market_value(i32::MIN, i32::MIN, 0, 1);
        assert_eq!(ma, i32::MIN + 1);
        assert_eq!(mb, i32::MIN + 1);

        // The midpoint of any saturated pair must itself stay in range.
        let mid = market_value_midpoint(i32::MAX, i32::MAX);
        assert_eq!(mid, i32::MAX);
        let mid = market_value_midpoint(i32::MIN, i32::MIN);
        assert_eq!(mid, i32::MIN);
    }

    /// Drive the full `decode_frame` pipeline with a synthetic FIT
    /// MARKET_VALUE frame (same 11-field quote layout) and assert the
    /// emitted `StreamData::MarketValue` carries the calculated bid/ask/price
    /// reassembled to dollars via the same `Price` path as every other tick.
    #[test]
    fn decode_frame_market_value_emits_calculated_fields() {
        // 11-field quote layout (FIT prefix: contract_id):
        //   [contract_id, ms_of_day, bid_size, bid_exchange, bid,
        //    bid_condition, ask_size, ask_exchange, ask, ask_condition,
        //    price_type, date]
        let fit_payload = encode_fit_row(&[
            100,        // contract_id
            34_200_000, // ms_of_day
            10,         // bid_size
            4,          // bid_exchange
            15_025,     // bid
            0,          // bid_condition
            10,         // ask_size
            4,          // ask_exchange
            15_030,     // ask
            0,          // ask_condition
            8,          // price_type
            20_250_428, // date
        ]);
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(100, Arc::new(Contract::stock("AAPL")));
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::MarketValue,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        let evt = primary.expect("decode_frame must emit a primary MarketValue event");
        match expect_public(&evt) {
            StreamEvent::Data(StreamData::MarketValue {
                contract,
                ms_of_day,
                market_bid,
                market_ask,
                market_price,
                date,
                ..
            }) => {
                assert_eq!(&*contract.symbol, "AAPL");
                assert_eq!(*ms_of_day, 34_200_000);
                assert_eq!(*date, 20_250_428);
                // Balanced book, ask>2: market_ask=15029, market_bid=15026,
                // mid=15027 — reassembled through Price(value, price_type=8).
                assert!((*market_ask - Price::new(15_029, 8).to_f64()).abs() < f64::EPSILON);
                assert!((*market_bid - Price::new(15_026, 8).to_f64()).abs() < f64::EPSILON);
                assert!((*market_price - Price::new(15_027, 8).to_f64()).abs() < f64::EPSILON);
                // Parity invariant: bid <= price <= ask.
                assert!(*market_bid <= *market_price && *market_price <= *market_ask);
            }
            other => panic!("expected Data(MarketValue), got {other:?}"),
        }
    }

    #[test]
    fn decode_frame_market_value_invalid_price_type_drops_to_unparseable() {
        let fit_payload = encode_fit_row(&[
            300, 34_200_000, 10, 4, 15_025, 0, 12, 4, 15_030, 0, 20, 20_250_428,
        ]);
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(300, Arc::new(Contract::stock("AAPL")));
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::MarketValue,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        match primary {
            Some(FpssEventInternal::Unparseable) => {}
            other => panic!("expected Unparseable for out-of-range price_type, got {other:?}"),
        }
    }

    #[test]
    fn decode_frame_quote_in_range_price_type_still_emits_data() {
        let fit_payload = encode_fit_row(&[
            700, 34_200_000, 10, 4, 15_025, 0, 12, 4, 15_030, 0, 8, 20_250_428,
        ]);
        let mut local_contracts: HashMap<i32, Arc<Contract>> = HashMap::new();
        local_contracts.insert(700, Arc::new(Contract::stock("AAPL")));
        let authenticated = AtomicBool::new(true);
        let shutdown = AtomicBool::new(false);
        let mut delta_state = DeltaState::new();
        let primary = decode_frame(
            StreamMsgType::Quote,
            &fit_payload,
            &authenticated,
            &mut local_contracts,
            &shutdown,
            &mut delta_state,
        );
        let evt = primary.expect("in-range Quote frame must emit a primary event");
        match expect_public(&evt) {
            StreamEvent::Data(StreamData::Quote { bid, ask, .. }) => {
                assert!((*bid - Price::new(15_025, 8).to_f64()).abs() < f64::EPSILON);
                assert!((*ask - Price::new(15_030, 8).to_f64()).abs() < f64::EPSILON);
            }
            other => panic!("expected Data(Quote) for in-range price_type, got {other:?}"),
        }
    }

    /// The delta-decode maps grow with the live universe and retain every
    /// distinct contract id within a session: there is no per-session cap
    /// (the terminal imposes none), so a large distinct-id universe is held
    /// in full. The maps reset only at START/STOP/RESTART/RECONNECTED
    /// session boundaries via `DeltaState::clear`.
    #[test]
    fn delta_state_normal_session_retains_all_contracts() {
        let mut delta_state = DeltaState::new();
        let mut out: TickFields = [0; crate::fpss::delta::MAX_DATA_FIELDS];

        let n = 500i32; // a generous live universe.
        for id in 0..n {
            let payload = encode_fit_row(&[id, 34_200_000, 0, 50, 6, 5_500_000, 57, 6, 20_250_428]);
            delta_state
                .decode_tick(StreamMsgType::Trade as u8, &payload, TRADE_FIELDS, &mut out)
                .expect("absolute tick decodes");
        }

        let (prev, field_counts) = delta_state.state_sizes();
        assert_eq!(prev, n as usize, "every distinct id retained in prev");
        assert_eq!(
            field_counts, n as usize,
            "every distinct id retained in field_counts"
        );
    }
}
