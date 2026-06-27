//! JSON payload shaping for outgoing WebSocket frames.

use std::sync::atomic::{AtomicU64, Ordering};

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{
    StreamControl, StreamData, StreamEvent, UNRESOLVED_CONTRACT_SYMBOL_PREFIX,
};

/// Total events dropped because their JSON serialization failed.
///
/// Distinct from the WS broadcast-channel drop counter — this counts
/// the upstream "we built the JSON value but `to_string` returned `Err`"
/// branch in [`fpss_event_to_ws_json`]. M1 fix: previously these
/// failures returned `None` silently; the operator could not tell
/// whether the WS feed was healthy + idle or quietly leaking events
/// to a serialization bug.
static JSON_SERIALIZE_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Snapshot of [`JSON_SERIALIZE_FAILURES`]. Used by metrics exporters
/// and integration tests to assert the counter increments as expected.
#[must_use]
pub fn json_serialize_failure_count() -> u64 {
    JSON_SERIALIZE_FAILURES.load(Ordering::Relaxed)
}

/// Wrap a `sonic_rs::to_string` call so a serialization failure logs
/// (rate-limited at every 1024 failures) and bumps the public counter.
/// Returns `None` exactly when `to_string` returned `Err`.
fn try_serialize(msg: &sonic_rs::Value) -> Option<String> {
    match sonic_rs::to_string(msg) {
        Ok(s) => Some(s),
        Err(e) => {
            let prev = JSON_SERIALIZE_FAILURES.fetch_add(1, Ordering::Relaxed);
            if prev.is_multiple_of(1024) {
                tracing::error!(
                    target: "thetadatadx_server::ws::format",
                    failure_count = prev + 1,
                    error = %e,
                    "fpss_event_to_ws_json: sonic_rs::to_string failed; \
                     event dropped from the WS broadcast feed",
                );
            }
            None
        }
    }
}

/// Convert an `StreamEvent` to the JVM terminal's WebSocket JSON format.
///
/// `peeked_contract` should be the `Arc<Contract>` carried on the event
/// (see [`super::contract_map::lookup_event_contract`]). Passing it in
/// keeps the broadcast task contract-aware without re-deriving the
/// reference from the event on every serialisation.
pub(super) fn fpss_event_to_ws_json(
    event: &StreamEvent,
    peeked_contract: Option<&Contract>,
) -> Option<String> {
    match event {
        StreamEvent::Data(data) => {
            let (event_type, body) = match data {
                StreamData::Quote {
                    ms_of_day,
                    bid_size,
                    bid_exchange,
                    bid,
                    bid_condition,
                    ask_size,
                    ask_exchange,
                    ask,
                    ask_condition,
                    date,
                    ..
                } => (
                    "QUOTE",
                    // Field set matches `EventSerializer.serializeQuote`
                    // exactly. The terminal's quote frame carries no
                    // `received_at_ns`; a strict client never sees it.
                    sonic_rs::json!({
                        "ms_of_day": ms_of_day,
                        "bid_size": bid_size,
                        "bid_exchange": bid_exchange,
                        "bid": bid,
                        "bid_condition": bid_condition,
                        "ask_size": ask_size,
                        "ask_exchange": ask_exchange,
                        "ask": ask,
                        "ask_condition": ask_condition,
                        "date": date,
                    }),
                ),
                StreamData::Trade {
                    ms_of_day,
                    sequence,
                    condition,
                    size,
                    exchange,
                    price,
                    date,
                    ..
                } => (
                    "TRADE",
                    // Field set matches `EventSerializer.serializeTrade`
                    // exactly: `ms_of_day, sequence, size, condition,
                    // price, exchange, date`. The terminal's trade frame
                    // does NOT carry the extended-condition columns
                    // (`ext_condition1..4`), the `condition_flags` /
                    // `price_flags` bitsets, `volume_type`, `records_back`,
                    // or `received_at_ns`; those stay on the REST/Arrow
                    // surface so a strict terminal WS client sees only the
                    // columns it parses.
                    sonic_rs::json!({
                        "ms_of_day": ms_of_day,
                        "sequence": sequence,
                        "size": size,
                        "condition": condition,
                        "price": price,
                        "exchange": exchange,
                        "date": date,
                    }),
                ),
                StreamData::Ohlcvc {
                    ms_of_day,
                    open,
                    high,
                    low,
                    close,
                    volume,
                    count,
                    date,
                    ..
                } => (
                    "OHLC",
                    // Field set matches `EventSerializer.serializeOhlc`
                    // exactly. No `received_at_ns` on the terminal's bar.
                    sonic_rs::json!({
                        "ms_of_day": ms_of_day,
                        "open": open,
                        "high": high,
                        "low": low,
                        "close": close,
                        "volume": volume,
                        "count": count,
                        "date": date,
                    }),
                ),
                StreamData::MarketValue {
                    ms_of_day,
                    market_bid,
                    market_ask,
                    market_price,
                    date,
                    ..
                } => (
                    "MARKET_VALUE",
                    // Field set matches `EventSerializer.serializeMarketValue`
                    // exactly. No `received_at_ns` on the terminal frame.
                    sonic_rs::json!({
                        "date": date,
                        "ms_of_day": ms_of_day,
                        "market_bid": market_bid,
                        "market_ask": market_ask,
                        "market_price": market_price,
                    }),
                ),
                // The terminal's `EventSerializer` defines no open-interest
                // WS frame: it serializes only STATUS / TRADE / QUOTE / OHLC
                // / MARKET_VALUE / REQ_RESPONSE / STATE. Open-interest ticks
                // therefore never become a WS frame here — emitting an
                // `OPEN_INTEREST` frame would hand a strict terminal client
                // a `header.type` it does not recognize. The data stays
                // available on the REST / Arrow surfaces.
                _ => return None,
            };

            // The event always carries an `Arc<Contract>` directly; the
            // bridge mirrors that into `peeked_contract`. The unresolved-
            // contract sentinel (`sec_type == SecType::Unknown`) carries
            // the wire id under the `__pending:<id>` prefix on its
            // `symbol`. Surface that as a top-level
            // `unresolved_contract_id` integer so operators can
            // correlate the pre-`ContractAssigned` tick with the
            // matching control frame downstream — without leaking the
            // wire id onto the resolved-contract path.
            //
            // Previously this branch silently emitted an empty
            // `Contract` object for unresolved ticks, leaving operators
            // no way to disambiguate a backlog of pre-ContractAssigned
            // ticks from a serialisation regression.
            let mut msg_obj = sonic_rs::Object::new();
            msg_obj.insert("header", sonic_rs::json!({ "type": event_type }));
            match peeked_contract {
                Some(c) if is_unresolved_contract(c) => {
                    // Pre-ContractAssigned tick. Emit a `pending`
                    // contract status header and the parsed wire id.
                    msg_obj.insert("contract", sonic_rs::json!({ "status": "pending" }));
                    if let Some(id) = parse_unresolved_contract_id(&c.symbol) {
                        msg_obj.insert("unresolved_contract_id", sonic_rs::Value::from(id));
                    }
                }
                Some(c) => {
                    msg_obj.insert("contract", contract_to_json(c));
                }
                None => {
                    msg_obj.insert("contract", sonic_rs::json!({}));
                }
            }
            let lc_type = event_type.to_ascii_lowercase();
            msg_obj.insert(lc_type.as_str(), body);
            let msg = sonic_rs::Value::from(msg_obj);
            try_serialize(&msg)
        }

        StreamEvent::Control(ctrl) => match ctrl {
            StreamControl::ReqResponse { req_id, result } => {
                // Publish the stream-verification token, never the Rust
                // variant identifier: clients match on the documented
                // vocabulary (`SUBSCRIBED` / `ERROR` / `MAX_STREAMS_REACHED`
                // / `INVALID_PERMS`), which `as_wire_str` is the single
                // source of. Shape mirrors `EventSerializer.serializeResponse`.
                let msg = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": result.as_wire_str(),
                        "req_id": req_id,
                    }
                });
                try_serialize(&msg)
            }
            // The terminal signals stream start/stop with a `STATE` frame
            // (`EventSerializer.serializeState`), driven by the FPSS
            // `START` (code 30) / `STOP` (code 32) lifecycle frames. The
            // SDK decodes those same two frame codes to `MarketOpen` /
            // `MarketClose`, so they map onto the terminal's STATE START /
            // STOP — not a bespoke `STATUS:MARKET_OPEN` / `MARKET_CLOSE`,
            // which is a `header.status` value the terminal never emits.
            StreamControl::MarketOpen => {
                let msg = sonic_rs::json!({
                    "header": { "type": "STATE", "state": "START" }
                });
                try_serialize(&msg)
            }
            StreamControl::MarketClose => {
                let msg = sonic_rs::json!({
                    "header": { "type": "STATE", "state": "STOP" }
                });
                try_serialize(&msg)
            }
            StreamControl::Disconnected { .. } => {
                // The terminal's heartbeat reports the connection state via
                // a `STATUS` frame whose `status` is one of `CONNECTED` /
                // `UNVERIFIED` / `DISCONNECTED` (`serializeStatus`). It
                // carries no `reason` field, so the disconnect surfaces as a
                // bare `STATUS:DISCONNECTED` here — the per-second heartbeat
                // already reports the live state, and the extra `reason`
                // token is an our-only field a strict terminal client never
                // sees.
                let msg = sonic_rs::json!({
                    "header": { "type": "STATUS", "status": "DISCONNECTED" }
                });
                try_serialize(&msg)
            }
            // `ContractAssigned` (the terminal carries the contract inline
            // on every tick frame, never as a standalone `CONTRACT` frame),
            // `ServerError` (no `header.type=ERROR` in the terminal's
            // serializer — errors surface as a `REQ_RESPONSE` with
            // `response=ERROR`), and every other control variant have no
            // terminal WS frame, so they emit nothing. This keeps the
            // emitted `header.type` set to exactly the terminal's:
            // STATUS / TRADE / QUOTE / OHLC / MARKET_VALUE / REQ_RESPONSE /
            // STATE.
            _ => None,
        },

        _ => None,
    }
}

/// Whether `c` is the unresolved-contract sentinel emitted by the FPSS
/// decoder when a tick arrives before its matching `ContractAssigned`
/// frame. The canonical, type-safe check is `sec_type ==
/// SecType::Unknown`; the secondary `symbol` check below is a defensive
/// fence that catches the unlikely case of a real contract whose
/// type-tag round-trips to `Unknown` from a downstream decoder bug.
fn is_unresolved_contract(c: &Contract) -> bool {
    c.sec_type == thetadatadx::SecType::Unknown
        && c.symbol.starts_with(UNRESOLVED_CONTRACT_SYMBOL_PREFIX)
}

/// Parse the wire id out of an unresolved-contract sentinel's symbol
/// (`__pending:<id>` -> `<id>`). Returns `None` if the suffix is not a
/// valid `i32`; the WS payload then omits the field rather than emitting
/// a garbage value.
fn parse_unresolved_contract_id(symbol: &str) -> Option<i32> {
    symbol
        .strip_prefix(UNRESOLVED_CONTRACT_SYMBOL_PREFIX)
        .and_then(|s| s.parse::<i32>().ok())
}

/// Convert a `Contract` to the terminal's v3 WebSocket contract object.
///
/// Field set and key names mirror `EventSerializer.readContract` exactly
/// so a client written against the terminal stream reads the contract
/// envelope without a remap:
///
/// - `security_type` / `root` are always present (the terminal names the
///   keys `security_type` and `root`, not `sec_type` / `symbol`).
/// - `expiration` / `strike` / `right` are present only for options.
/// - `strike` is dollars (a `$550.00` strike serializes as `550.0`), the
///   same unit every other public surface speaks; the wire's fixed-point
///   integer never leaves the codec layer.
fn contract_to_json(c: &Contract) -> sonic_rs::Value {
    let mut obj = sonic_rs::Object::new();
    obj.insert("security_type", sonic_rs::Value::from(c.sec_type.as_str()));
    obj.insert("root", sonic_rs::Value::from(&*c.symbol));
    if let Some(exp) = c.expiration {
        obj.insert("expiration", sonic_rs::Value::from(exp));
    }
    // `strike` is dollars on every public surface; the wire's
    // fixed-point integer never leaves the codec layer.
    if let Some(strike) = c.strike_dollars() {
        obj.insert(
            "strike",
            sonic_rs::to_value(&strike).expect("f64 should serialize"),
        );
    }
    if let Some(is_call) = c.is_call {
        obj.insert(
            "right",
            sonic_rs::Value::from(if is_call { "C" } else { "P" }),
        );
    }
    sonic_rs::Value::from(obj)
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::contract_map::lookup_event_contract;
    use super::*;

    fn make_quote(contract: Arc<Contract>) -> StreamEvent {
        StreamEvent::Data(StreamData::Quote {
            contract,
            ms_of_day: 0,
            bid_size: 0,
            bid_exchange: 0,
            bid: 0.0,
            bid_condition: 0,
            ask_size: 0,
            ask_exchange: 0,
            ask: 0.0,
            ask_condition: 0,
            date: 0,
            received_at_ns: 0,
        })
    }

    fn make_trade(contract: Arc<Contract>) -> StreamEvent {
        StreamEvent::Data(StreamData::Trade {
            contract,
            ms_of_day: 1,
            sequence: 2,
            ext_condition1: 3,
            ext_condition2: 4,
            ext_condition3: 5,
            ext_condition4: 6,
            condition: 7,
            size: 8,
            exchange: 9,
            price: 10.5,
            condition_flags: 11,
            price_flags: 12,
            volume_type: 13,
            records_back: 14,
            date: 20260617,
            received_at_ns: 15,
        })
    }

    fn make_market_value(contract: Arc<Contract>) -> StreamEvent {
        StreamEvent::Data(StreamData::MarketValue {
            contract,
            ms_of_day: 1,
            market_bid: 2.5,
            market_ask: 3.5,
            market_price: 3.0,
            date: 20260617,
            received_at_ns: 4,
        })
    }

    /// The WS Trade frame carries exactly the columns
    /// `EventSerializer.serializeTrade` emits — no more. The terminal's
    /// trade frame is `ms_of_day, sequence, size, condition, price,
    /// exchange, date`; the extended-condition columns, the flag bitsets,
    /// `volume_type`, `records_back`, and `received_at_ns` stay on the
    /// REST / Arrow surface and must NOT appear, so a strict terminal WS
    /// client sees only the field set it parses.
    #[test]
    fn trade_frame_matches_terminal_field_set() {
        let contract = Arc::new(Contract::stock("AAPL"));
        let event = make_trade(Arc::clone(&contract));
        let json = fpss_event_to_ws_json(&event, Some(&contract))
            .expect("Trade serialization must succeed");

        for key in [
            "ms_of_day",
            "sequence",
            "size",
            "condition",
            "price",
            "exchange",
            "date",
        ] {
            assert!(
                json.contains(&format!("\"{key}\":")),
                "Trade frame must carry the `{key}` column: {json}"
            );
        }
        // Columns the terminal's trade frame never emits.
        for key in [
            "ext_condition1",
            "ext_condition2",
            "ext_condition3",
            "ext_condition4",
            "condition_flags",
            "price_flags",
            "volume_type",
            "records_back",
            "received_at_ns",
        ] {
            assert!(
                !json.contains(&format!("\"{key}\":")),
                "Trade frame must NOT carry the our-only `{key}` column: {json}"
            );
        }
        assert!(
            json.contains("\"header\":{\"type\":\"TRADE\"}"),
            "Trade frame header type: {json}"
        );
    }

    /// MARKET_VALUE is a first-class per-contract stream: its tick must
    /// serialize with the calculated market bid/ask/price columns rather
    /// than being swallowed by the catch-all `None` arm.
    #[test]
    fn market_value_frame_serializes_calculated_columns() {
        let contract = Arc::new(Contract::stock("SPX"));
        let event = make_market_value(Arc::clone(&contract));
        let json = fpss_event_to_ws_json(&event, Some(&contract))
            .expect("MARKET_VALUE serialization must succeed");

        for key in [
            "ms_of_day",
            "market_bid",
            "market_ask",
            "market_price",
            "date",
        ] {
            assert!(
                json.contains(&format!("\"{key}\":")),
                "MARKET_VALUE frame must carry the `{key}` column: {json}"
            );
        }
        assert!(
            !json.contains("\"received_at_ns\":"),
            "MARKET_VALUE frame must NOT carry the our-only `received_at_ns`: {json}"
        );
        assert!(
            json.contains("\"header\":{\"type\":\"MARKET_VALUE\"}"),
            "MARKET_VALUE frame header type: {json}"
        );
        // The terminal names the contract key `root`, not `symbol`.
        assert!(
            json.contains("\"root\":\"SPX\""),
            "MARKET_VALUE frame must carry its contract under `root`: {json}"
        );
    }

    /// The callback thread resolves the contract directly from the
    /// event's `Arc<Contract>` — the previous `contract_id ->
    /// Arc<Contract>` map and the TOCTOU race that came with it are
    /// gone. Serialisation still succeeds with the contract symbol
    /// embedded in the JSON payload.
    #[test]
    fn contract_snapshot_rides_on_event() {
        let contract = Arc::new(Contract::stock("AAPL"));
        let event = make_quote(Arc::clone(&contract));
        let peeked = lookup_event_contract(&event).expect("event carries its contract");
        assert_eq!(&*peeked.symbol, "AAPL");
        let json = fpss_event_to_ws_json(&event, Some(&peeked))
            .expect("serialization must succeed with the event's contract");
        assert!(
            json.contains("\"root\":\"AAPL\""),
            "serialized JSON must include the event's contract under `root`: {json}"
        );
    }

    /// Pre-`ContractAssigned` events carry the empty-contract sentinel.
    /// The serialised payload still parses; the `contract` envelope
    /// degrades to an empty object rather than embedding the
    /// wire-internal numeric id.
    #[test]
    fn empty_contract_sentinel_serializes_without_id_leak() {
        let sentinel = Arc::new(Contract::stock(""));
        let event = make_quote(Arc::clone(&sentinel));
        let json = fpss_event_to_ws_json(&event, None)
            .expect("serialization must succeed with no resolved contract");
        assert!(
            !json.contains("\"id\":"),
            "no wire-internal contract id may leak into the WS payload: {json}"
        );
    }

    /// Pre-`ContractAssigned` ticks carry the unresolved-contract
    /// sentinel whose `symbol` is
    /// `__pending:<id>`. The WS payload must surface
    /// `unresolved_contract_id: <id>` (top-level integer) and emit
    /// `contract: {"status": "pending"}` so operators can correlate
    /// the pre-ContractAssigned tick with the matching control frame
    /// downstream.
    #[test]
    fn unresolved_sentinel_surfaces_wire_id_to_ws_payload() {
        let unresolved = Arc::new(Contract::pending(42));
        let event = make_quote(Arc::clone(&unresolved));
        let json = fpss_event_to_ws_json(&event, Some(&unresolved))
            .expect("serialization must succeed for an unresolved sentinel");

        assert!(
            json.contains("\"unresolved_contract_id\":42"),
            "WS payload must surface the wire id from the `__pending:` \
             prefix as a top-level integer: {json}"
        );
        assert!(
            json.contains("\"status\":\"pending\""),
            "WS payload must mark the contract envelope as pending: {json}"
        );
        // The `__pending:` prefix MUST NOT leak into the JSON as a
        // symbol — that's a diagnostic payload, not a real ticker.
        assert!(
            !json.contains("__pending:"),
            "WS payload must NOT echo the diagnostic prefix: {json}"
        );
        // The resolved-contract path's `id` field stays absent.
        assert!(
            !json.contains("\"id\":"),
            "the resolved-contract `id` field is for `ContractAssigned` \
             control frames only: {json}"
        );
    }

    /// Every stream-request outcome serialises as its documented
    /// verification token, never the Rust variant identifier. A Debug
    /// rendering here would leak `Subscribed` / `MaxStreamsReached` /
    /// `InvalidPerms` onto the wire and break clients matching on the
    /// `SUBSCRIBED` / `MAX_STREAMS_REACHED` / `INVALID_PERMS` vocabulary.
    #[test]
    fn req_response_outcomes_serialize_to_wire_tokens() {
        use thetadatadx::StreamResponseType;

        for (outcome, token) in [
            (StreamResponseType::Subscribed, "SUBSCRIBED"),
            (StreamResponseType::Error, "ERROR"),
            (StreamResponseType::MaxStreamsReached, "MAX_STREAMS_REACHED"),
            (StreamResponseType::InvalidPerms, "INVALID_PERMS"),
        ] {
            assert_eq!(outcome.as_wire_str(), token, "{outcome:?}");

            let event = StreamEvent::Control(thetadatadx::fpss::StreamControl::ReqResponse {
                req_id: 7,
                result: outcome,
            });
            let json = fpss_event_to_ws_json(&event, None)
                .expect("REQ_RESPONSE control frame must serialise");
            assert!(
                json.contains(&format!("\"response\":\"{token}\"")),
                "REQ_RESPONSE must publish the wire token {token}: {json}"
            );
            // The Rust variant identifier must never reach the payload.
            assert!(
                !json.contains(&format!("{outcome:?}")),
                "the Rust variant name must not leak onto the wire: {json}"
            );
        }
    }

    /// A disconnect surfaces as the terminal's bare `STATUS:DISCONNECTED`
    /// frame (`serializeStatus`) — `header.type=STATUS`,
    /// `header.status=DISCONNECTED`, and no `reason` field, since the
    /// terminal's status frame carries only the status string. The Rust
    /// `RemoveReason` variant must never leak onto the wire.
    #[test]
    fn disconnect_serializes_as_terminal_status_frame() {
        use thetadatadx::RemoveReason;

        for reason in [
            RemoveReason::Unspecified,
            RemoveReason::InvalidCredentials,
            RemoveReason::TooManyRequests,
            RemoveReason::ServerRestarting,
            RemoveReason::InvalidCredentialsNullUser,
        ] {
            let event =
                StreamEvent::Control(thetadatadx::fpss::StreamControl::Disconnected { reason });
            let json = fpss_event_to_ws_json(&event, None)
                .expect("DISCONNECTED control frame must serialise");
            // Key order inside the header object is serializer-defined;
            // assert on content, not byte-exact key ordering.
            assert!(
                json.contains("\"type\":\"STATUS\"")
                    && json.contains("\"status\":\"DISCONNECTED\""),
                "disconnect must be a bare STATUS:DISCONNECTED frame: {json}"
            );
            // The terminal's STATUS frame carries no `reason` field.
            assert!(
                !json.contains("\"reason\":"),
                "STATUS:DISCONNECTED must not carry an our-only `reason` field: {json}"
            );
            assert!(
                !json.contains(&format!("{reason:?}")),
                "the Rust variant name must not leak onto the wire: {json}"
            );
        }
    }

    /// The contract envelope uses the terminal's key names (`security_type`
    /// / `root`, not `sec_type` / `symbol`) and emits `strike` in dollars
    /// (a `$550.00` strike serializes as `550.0`) — the same unit every
    /// other public surface speaks.
    #[test]
    fn option_contract_envelope_uses_terminal_keys_and_dollar_strike() {
        let contract = Arc::new(Contract::option_raw("SPY", 20_260_417, true, 550_000));
        let event = make_quote(Arc::clone(&contract));
        let json = fpss_event_to_ws_json(&event, Some(&contract))
            .expect("Quote serialization must succeed");

        assert!(
            json.contains("\"security_type\":\"OPTION\""),
            "contract must carry `security_type`, not `sec_type`: {json}"
        );
        assert!(
            json.contains("\"root\":\"SPY\""),
            "contract must carry `root`, not `symbol`: {json}"
        );
        // Dollars float (`550.0`), NOT the raw thousandths integer.
        assert!(
            json.contains("\"strike\":550.0"),
            "strike must serialize as a dollars float: {json}"
        );
        assert!(
            !json.contains("\"strike\":550000"),
            "strike must NOT serialize as the raw thousandths integer: {json}"
        );
        assert!(json.contains("\"expiration\":20260417"), "{json}");
        assert!(json.contains("\"right\":\"C\""), "{json}");
        // The old key names must be gone entirely.
        assert!(!json.contains("\"sec_type\":"), "{json}");
        assert!(!json.contains("\"symbol\":"), "{json}");
    }

    /// The FPSS START / STOP lifecycle frames (decoded to `MarketOpen` /
    /// `MarketClose`) serialize as the terminal's `STATE` frame
    /// (`serializeState`), `state` = `START` / `STOP`. They must NOT
    /// serialize as the bespoke `STATUS:MARKET_OPEN` / `MARKET_CLOSE`
    /// values the terminal never emits.
    #[test]
    fn market_lifecycle_serializes_as_state_frame() {
        for (ctrl, state) in [
            (thetadatadx::fpss::StreamControl::MarketOpen, "START"),
            (thetadatadx::fpss::StreamControl::MarketClose, "STOP"),
        ] {
            let event = StreamEvent::Control(ctrl);
            let json =
                fpss_event_to_ws_json(&event, None).expect("STATE control frame must serialise");
            // Key order inside the header object is serializer-defined;
            // assert on content, not byte-exact key ordering.
            assert!(
                json.contains("\"type\":\"STATE\"")
                    && json.contains(&format!("\"state\":\"{state}\"")),
                "lifecycle must be a STATE/{state} frame: {json}"
            );
            assert!(
                !json.contains("MARKET_OPEN") && !json.contains("MARKET_CLOSE"),
                "must not emit the our-only STATUS:MARKET_* value: {json}"
            );
        }
    }

    /// The terminal's `EventSerializer` emits no `CONTRACT`, `ERROR`, or
    /// `OPEN_INTEREST` `header.type`. Those control / data events must
    /// serialize to nothing here so a strict terminal client never
    /// receives a `header.type` it cannot recognize.
    #[test]
    fn our_only_frame_types_are_not_emitted() {
        // ServerError -> no ERROR frame.
        let server_error = StreamEvent::Control(thetadatadx::fpss::StreamControl::ServerError {
            message: "boom".to_string(),
        });
        assert!(
            fpss_event_to_ws_json(&server_error, None).is_none(),
            "ServerError must not emit a header.type=ERROR frame"
        );

        // ContractAssigned -> no standalone CONTRACT frame.
        let assigned = StreamEvent::Control(thetadatadx::fpss::StreamControl::ContractAssigned {
            id: 7,
            contract: Arc::new(Contract::stock("AAPL")),
        });
        assert!(
            fpss_event_to_ws_json(&assigned, None).is_none(),
            "ContractAssigned must not emit a standalone CONTRACT frame"
        );

        // OpenInterest tick -> no OPEN_INTEREST frame.
        let oi = StreamEvent::Data(StreamData::OpenInterest {
            contract: Arc::new(Contract::option_raw("SPY", 20_260_417, true, 550_000)),
            ms_of_day: 1,
            open_interest: 123,
            date: 20_260_417,
            received_at_ns: 0,
        });
        assert!(
            fpss_event_to_ws_json(
                &oi,
                Some(&Contract::option_raw("SPY", 20_260_417, true, 550_000))
            )
            .is_none(),
            "OpenInterest must not emit an OPEN_INTEREST frame"
        );
    }

    /// A resolved Quote whose `peeked_contract` is the live mapped
    /// contract continues to serialise the same way it always did —
    /// MED-001 only changes the unresolved-sentinel branch.
    #[test]
    fn resolved_contract_path_unchanged_by_med_001_fix() {
        let resolved = Arc::new(Contract::stock("SPY"));
        let event = make_quote(Arc::clone(&resolved));
        let json = fpss_event_to_ws_json(&event, Some(&resolved))
            .expect("serialization must succeed for a resolved contract");

        assert!(json.contains("\"root\":\"SPY\""));
        assert!(
            !json.contains("\"unresolved_contract_id\""),
            "resolved-contract path must NOT emit the unresolved diagnostic: {json}"
        );
        assert!(
            !json.contains("\"status\":\"pending\""),
            "resolved-contract path must NOT mark the envelope as pending: {json}"
        );
    }
}
