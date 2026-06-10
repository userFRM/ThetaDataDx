//! JSON payload shaping for outgoing WebSocket frames.

use std::sync::atomic::{AtomicU64, Ordering};

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssControl, FpssData, FpssEvent, UNRESOLVED_CONTRACT_SYMBOL_PREFIX};

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

/// Convert an `FpssEvent` to the Java terminal's WebSocket JSON format.
///
/// `peeked_contract` should be the `Arc<Contract>` carried on the event
/// (see [`super::contract_map::lookup_event_contract`]). Passing it in
/// keeps the broadcast task contract-aware without re-deriving the
/// reference from the event on every serialisation.
pub(super) fn fpss_event_to_ws_json(
    event: &FpssEvent,
    peeked_contract: Option<&Contract>,
) -> Option<String> {
    match event {
        FpssEvent::Data(data) => {
            let (event_type, body) = match data {
                FpssData::Quote {
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
                    received_at_ns,
                    ..
                } => (
                    "QUOTE",
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
                        "received_at_ns": received_at_ns,
                    }),
                ),
                FpssData::Trade {
                    ms_of_day,
                    sequence,
                    condition,
                    size,
                    exchange,
                    price,
                    date,
                    received_at_ns,
                    ..
                } => (
                    "TRADE",
                    sonic_rs::json!({
                        "ms_of_day": ms_of_day,
                        "sequence": sequence,
                        "condition": condition,
                        "size": size,
                        "exchange": exchange,
                        "price": price,
                        "date": date,
                        "received_at_ns": received_at_ns,
                    }),
                ),
                FpssData::Ohlcvc {
                    ms_of_day,
                    open,
                    high,
                    low,
                    close,
                    volume,
                    count,
                    date,
                    received_at_ns,
                    ..
                } => (
                    "OHLC",
                    sonic_rs::json!({
                        "ms_of_day": ms_of_day,
                        "open": open,
                        "high": high,
                        "low": low,
                        "close": close,
                        "volume": volume,
                        "count": count,
                        "date": date,
                        "received_at_ns": received_at_ns,
                    }),
                ),
                FpssData::OpenInterest {
                    ms_of_day,
                    open_interest,
                    date,
                    received_at_ns,
                    ..
                } => (
                    "OPEN_INTEREST",
                    sonic_rs::json!({
                        "ms_of_day": ms_of_day,
                        "open_interest": open_interest,
                        "date": date,
                        "received_at_ns": received_at_ns,
                    }),
                ),
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

        FpssEvent::Control(ctrl) => match ctrl {
            FpssControl::ContractAssigned { id, contract } => {
                let msg = sonic_rs::json!({
                    "header": { "type": "CONTRACT" },
                    "contract": contract_to_json(contract),
                    "id": id,
                });
                try_serialize(&msg)
            }
            FpssControl::ReqResponse { req_id, result } => {
                let msg = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": format!("{result:?}"),
                        "req_id": req_id,
                    }
                });
                try_serialize(&msg)
            }
            FpssControl::MarketOpen => {
                let msg = sonic_rs::json!({
                    "header": { "type": "STATUS", "status": "MARKET_OPEN" }
                });
                try_serialize(&msg)
            }
            FpssControl::MarketClose => {
                let msg = sonic_rs::json!({
                    "header": { "type": "STATUS", "status": "MARKET_CLOSE" }
                });
                try_serialize(&msg)
            }
            FpssControl::ServerError { message } => {
                let msg = sonic_rs::json!({
                    "header": { "type": "ERROR" },
                    "error": message.as_str(),
                });
                try_serialize(&msg)
            }
            FpssControl::Disconnected { reason } => {
                let msg = sonic_rs::json!({
                    "header": { "type": "STATUS", "status": "DISCONNECTED" },
                    "reason": format!("{reason:?}"),
                });
                try_serialize(&msg)
            }
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
    c.sec_type == tdbe::types::enums::SecType::Unknown
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

/// Convert a `Contract` to the JSON format the Java terminal uses.
fn contract_to_json(c: &Contract) -> sonic_rs::Value {
    let sec_type_str = format!("{:?}", c.sec_type).to_uppercase();
    let mut obj = sonic_rs::Object::new();
    obj.insert("symbol", sonic_rs::Value::from(&*c.symbol));
    obj.insert("sec_type", sonic_rs::Value::from(sec_type_str.as_str()));
    if let Some(exp) = c.expiration {
        obj.insert("expiration", sonic_rs::Value::from(exp));
    }
    if let Some(strike) = c.strike {
        obj.insert("strike", sonic_rs::Value::from(strike));
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

    fn make_quote(contract: Arc<Contract>) -> FpssEvent {
        FpssEvent::Data(FpssData::Quote {
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
            json.contains("\"symbol\":\"AAPL\""),
            "serialized JSON must include the event's contract symbol: {json}"
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

    /// A resolved Quote whose `peeked_contract` is the live mapped
    /// contract continues to serialise the same way it always did —
    /// MED-001 only changes the unresolved-sentinel branch.
    #[test]
    fn resolved_contract_path_unchanged_by_med_001_fix() {
        let resolved = Arc::new(Contract::stock("SPY"));
        let event = make_quote(Arc::clone(&resolved));
        let json = fpss_event_to_ws_json(&event, Some(&resolved))
            .expect("serialization must succeed for a resolved contract");

        assert!(json.contains("\"symbol\":\"SPY\""));
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
