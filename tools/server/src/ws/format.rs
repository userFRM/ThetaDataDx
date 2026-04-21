//! JSON payload shaping for outgoing WebSocket frames.

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssControl, FpssData, FpssEvent};

/// Convert an `FpssEvent` to the Java terminal's WebSocket JSON format.
///
/// `peeked_contract` should be the contract already looked up from the shared
/// map (see [`super::contract_map::lookup_event_contract`]). Passing it in
/// lets the caller release the map lock before serialization, so the FPSS
/// callback thread and the broadcast task never contend on the lock during
/// JSON encoding.
pub(super) fn fpss_event_to_ws_json(
    event: &FpssEvent,
    peeked_contract: Option<&Contract>,
) -> Option<String> {
    match event {
        FpssEvent::Data(data) => {
            let (event_type, contract_id, body) = match data {
                FpssData::Quote {
                    contract_id,
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
                    *contract_id,
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
                    contract_id,
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
                    *contract_id,
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
                    contract_id,
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
                    *contract_id,
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
                    contract_id,
                    ms_of_day,
                    open_interest,
                    date,
                    received_at_ns,
                    ..
                } => (
                    "OPEN_INTEREST",
                    *contract_id,
                    sonic_rs::json!({
                        "ms_of_day": ms_of_day,
                        "open_interest": open_interest,
                        "date": date,
                        "received_at_ns": received_at_ns,
                    }),
                ),
                _ => return None,
            };

            let contract_json = peeked_contract
                .map(contract_to_json)
                .unwrap_or_else(|| sonic_rs::json!({"id": contract_id}));

            let lc_type = event_type.to_ascii_lowercase();
            let msg = sonic_rs::json!({
                "header": { "type": event_type },
                "contract": contract_json,
                lc_type: body,
            });
            sonic_rs::to_string(&msg).ok()
        }

        FpssEvent::Control(ctrl) => match ctrl {
            FpssControl::ContractAssigned { id, contract } => {
                let msg = sonic_rs::json!({
                    "header": { "type": "CONTRACT" },
                    "contract": contract_to_json(contract),
                    "id": id,
                });
                sonic_rs::to_string(&msg).ok()
            }
            FpssControl::ReqResponse { req_id, result } => {
                let msg = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": format!("{result:?}"),
                        "req_id": req_id,
                    }
                });
                sonic_rs::to_string(&msg).ok()
            }
            FpssControl::MarketOpen => {
                let msg = sonic_rs::json!({
                    "header": { "type": "STATUS", "status": "MARKET_OPEN" }
                });
                sonic_rs::to_string(&msg).ok()
            }
            FpssControl::MarketClose => {
                let msg = sonic_rs::json!({
                    "header": { "type": "STATUS", "status": "MARKET_CLOSE" }
                });
                sonic_rs::to_string(&msg).ok()
            }
            FpssControl::ServerError { message } => {
                let msg = sonic_rs::json!({
                    "header": { "type": "ERROR" },
                    "error": message.as_str(),
                });
                sonic_rs::to_string(&msg).ok()
            }
            FpssControl::Disconnected { reason } => {
                let msg = sonic_rs::json!({
                    "header": { "type": "STATUS", "status": "DISCONNECTED" },
                    "reason": format!("{reason:?}"),
                });
                sonic_rs::to_string(&msg).ok()
            }
            _ => None,
        },

        _ => None,
    }
}

/// Convert a `Contract` to the JSON format the Java terminal uses.
fn contract_to_json(c: &Contract) -> sonic_rs::Value {
    let sec_type_str = format!("{:?}", c.sec_type).to_uppercase();
    let mut obj = sonic_rs::Object::new();
    obj.insert("root", sonic_rs::Value::from(c.root.as_str()));
    obj.insert("sec_type", sonic_rs::Value::from(sec_type_str.as_str()));
    if let Some(exp) = c.exp_date {
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
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::super::contract_map::lookup_event_contract;
    use super::*;

    fn make_quote(contract_id: i32) -> FpssEvent {
        FpssEvent::Data(FpssData::Quote {
            contract_id,
            symbol: Arc::from(""),
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

    // -----------------------------------------------------------------------
    //  F1 — TOCTOU fix: channel tuple carries the peeked Contract
    // -----------------------------------------------------------------------

    #[test]
    fn contract_snapshot_survives_concurrent_map_clear() {
        let map: Arc<Mutex<HashMap<i32, Arc<Contract>>>> = Arc::new(Mutex::new(HashMap::new()));

        // Pre-populate the map with a contract the event will reference.
        {
            let contract = Contract::stock("AAPL");
            map.lock().unwrap().insert(42, Arc::new(contract));
        }

        // Construct an event whose contract_id matches the pre-populated
        // entry.
        let event = make_quote(42);

        // Peek under the lock — mirrors what the callback thread does.
        let peeked = lookup_event_contract(&event, &map);
        assert!(peeked.is_some(), "pre-peek must find the contract");
        assert_eq!(peeked.as_ref().unwrap().root, "AAPL");

        // Simulate a reconnect / market-close clearing the shared map
        // AFTER the callback has peeked but BEFORE the broadcast task
        // serializes. Under the pre-fix code, a subsequent re-lookup
        // would now return None and produce `{"id": 42}` with no
        // root/strike/right.
        map.lock().unwrap().clear();

        // With the fix, the peeked snapshot still carries the full
        // contract via the Arc refcount — serialization must succeed
        // with root = "AAPL".
        let json = fpss_event_to_ws_json(&event, peeked.as_deref())
            .expect("serialization must succeed with peeked contract");
        assert!(
            json.contains("\"root\":\"AAPL\""),
            "serialized JSON must retain the peeked root after map clear: {json}"
        );
    }

    #[test]
    fn contract_snapshot_id_fallback_when_no_peek() {
        // If the map never had the contract, peek returns None and
        // fpss_event_to_ws_json falls back to `{"id": N}`. Verifies the
        // fallback path still works so events without ContractAssigned
        // don't get lost entirely.
        let event = make_quote(99);
        let json = fpss_event_to_ws_json(&event, None)
            .expect("serialization must succeed with no contract");
        assert!(json.contains("\"id\":99"));
    }
}
