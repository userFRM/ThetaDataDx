//! WebSocket server with full FPSS bridge.
//!
//! Replicates the Java terminal's WebSocket behavior:
//!
//! - Single WebSocket endpoint at `/v1/events`
//! - Only one WebSocket client at a time (enforced via `AtomicBool`)
//! - Clients receive JSON events: QUOTE, TRADE, OHLC, STATUS
//! - STATUS heartbeat every 1 second with FPSS connection state
//! - Client commands: subscribe/unsubscribe via JSON messages
//!
//! # FPSS Bridge
//!
//! `start_fpss_bridge()` connects an `FpssClient` whose callback converts
//! each `FpssEvent` to JSON and broadcasts it to all WS clients.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use sonic_rs::prelude::*;
use tokio::sync::mpsc;

use tdbe::types::enums::SecType;
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssControl, FpssData, FpssEvent};

use crate::state::AppState;

/// Build the WebSocket router (single route: `/v1/events`).
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/events", get(ws_upgrade))
        .with_state(state)
}

/// Handle the HTTP -> WebSocket upgrade.
async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    tracing::debug!("WebSocket upgrade request");

    if !state.try_acquire_ws() {
        tracing::warn!("WebSocket connection rejected: another client is already connected");
        return (
            axum::http::StatusCode::CONFLICT,
            "only one WebSocket client allowed at a time",
        )
            .into_response();
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state))
        .into_response()
}

/// Main WebSocket connection handler.
///
/// Multiplexes three event sources in `tokio::select!`:
/// 1. Heartbeat tick (1s) -> send STATUS
/// 2. Per-client mpsc events -> forward to client (zero-copy `Arc<str>`)
/// 3. Client messages -> process subscription commands
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut ws_rx: mpsc::Receiver<Arc<str>> = state.register_ws_client().await;
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(1));

    tracing::debug!("WebSocket client connected");

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let status = state.fpss_status();
                let msg = sonic_rs::json!({
                    "header": {
                        "type": "STATUS",
                        "status": status
                    }
                });
                let text = sonic_rs::to_string(&msg).unwrap_or_default();
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }

            event = ws_rx.recv() => {
                match event {
                    Some(event_json) => {
                        if socket.send(Message::Text(event_json.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    None => {
                        // Sender side dropped -- server shutting down.
                        break;
                    }
                }
            }

            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        tracing::debug!(msg = %text, "WebSocket client message");
                        handle_client_message(&state, &text, &mut socket).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::debug!("WebSocket client disconnected");
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "WebSocket recv error");
                        break;
                    }
                    _ => {} // Ignore binary/ping/pong.
                }
            }
        }
    }

    // Close the receiver so the sender side sees is_closed() = true.
    ws_rx.close();
    // Clean up our entry from the client list.
    state.cleanup_ws_clients().await;
    state.release_ws();
    tracing::debug!("WebSocket connection closed");
}

/// Parse and handle a client subscription command.
async fn handle_client_message(state: &AppState, text: &str, socket: &mut WebSocket) {
    let obj: sonic_rs::Value = match sonic_rs::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("invalid WebSocket JSON: {}", text);
            let resp = sonic_rs::json!({
                "header": {
                    "type": "REQ_RESPONSE",
                    "response": "ERROR",
                    "req_id": 0
                }
            });
            let _ = socket
                .send(Message::Text(
                    sonic_rs::to_string(&resp).unwrap_or_default().into(),
                ))
                .await;
            return;
        }
    };

    let null_val = sonic_rs::Value::default();
    let msg_type_val = obj.get("msg_type").unwrap_or(&null_val);
    let msg_type = msg_type_val.as_str().unwrap_or("").to_uppercase();

    let id_val = obj.get("id").unwrap_or(&null_val);
    let req_id = id_val.as_i64().unwrap_or(0);

    if msg_type == "STOP" {
        tracing::info!("WebSocket client requested STOP");
        let resp = sonic_rs::json!({
            "header": { "type": "REQ_RESPONSE", "response": "OK", "req_id": req_id }
        });
        let _ = socket
            .send(Message::Text(
                sonic_rs::to_string(&resp).unwrap_or_default().into(),
            ))
            .await;
        return;
    }

    let add_val = obj.get("add").unwrap_or(&null_val);
    let is_add = add_val.as_bool().unwrap_or(true);

    let sec_type_val = obj.get("sec_type").unwrap_or(&null_val);
    let sec_type = sec_type_val.as_str().unwrap_or("").to_uppercase();

    let req_type_val = obj.get("req_type").unwrap_or(&null_val);
    let req_type = req_type_val.as_str().unwrap_or("").to_uppercase();

    let contract_obj = obj.get("contract").unwrap_or(&null_val);
    let root_val = contract_obj.get("root").unwrap_or(&null_val);
    let root = root_val.as_str().unwrap_or("");

    tracing::info!(
        msg_type = %msg_type,
        sec_type = %sec_type,
        req_type = %req_type,
        req_id = req_id,
        root = %root,
        add = is_add,
        "WebSocket subscription command"
    );

    let contract = if sec_type == "OPTION" {
        let exp_val = contract_obj.get("expiration").unwrap_or(&null_val);
        let exp = exp_val.as_i64().unwrap_or(0) as i32;
        let strike_val = contract_obj.get("strike").unwrap_or(&null_val);
        let strike = strike_val.as_i64().unwrap_or(0) as i32;
        let right_val = contract_obj.get("right").unwrap_or(&null_val);
        let is_call = right_val
            .as_str()
            .is_some_and(|r| r.eq_ignore_ascii_case("C") || r.eq_ignore_ascii_case("CALL"));
        Contract::option_raw(root, exp, is_call, strike)
    } else {
        Contract::stock(root)
    };

    let tdx = state.tdx();
    if tdx.is_streaming() {
        let result = if is_add {
            match req_type.as_str() {
                "QUOTE" => tdx.subscribe_quotes(&contract),
                "TRADE" => tdx.subscribe_trades(&contract),
                "OPEN_INTEREST" => tdx.subscribe_open_interest(&contract),
                "FULL_TRADES" => {
                    let st = match sec_type.as_str() {
                        "OPTION" => SecType::Option,
                        "INDEX" => SecType::Index,
                        _ => SecType::Stock,
                    };
                    tdx.subscribe_full_trades(st)
                }
                _ => {
                    tracing::warn!(req_type = %req_type, "unknown req_type for subscription");
                    Ok(())
                }
            }
        } else {
            match req_type.as_str() {
                "QUOTE" => tdx.unsubscribe_quotes(&contract),
                "TRADE" => tdx.unsubscribe_trades(&contract),
                "OPEN_INTEREST" => tdx.unsubscribe_open_interest(&contract),
                _ => Ok(()),
            }
        };

        let resp = match result {
            Ok(_) => sonic_rs::json!({
                "header": { "type": "REQ_RESPONSE", "response": "OK", "req_id": req_id }
            }),
            Err(e) => {
                tracing::warn!(error = %e, "FPSS subscription failed");
                let err_msg = e.to_string();
                sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": err_msg.as_str()
                    }
                })
            }
        };
        let _ = socket
            .send(Message::Text(
                sonic_rs::to_string(&resp).unwrap_or_default().into(),
            ))
            .await;
    } else {
        tracing::warn!("FPSS streaming not started, subscription command ignored");
        let resp = sonic_rs::json!({
            "header": { "type": "REQ_RESPONSE", "response": "OK", "req_id": req_id }
        });
        let _ = socket
            .send(Message::Text(
                sonic_rs::to_string(&resp).unwrap_or_default().into(),
            ))
            .await;
    }
}

// ---------------------------------------------------------------------------
//  FPSS -> WebSocket bridge
// ---------------------------------------------------------------------------

/// Peek the contract for an event's `contract_id`, if any, while briefly
/// holding the shared contract-map lock. Returns a cloned `Contract` so the
/// lock can be released before the (O(fields)) JSON serialization runs.
fn lookup_event_contract(
    event: &FpssEvent,
    contract_map: &Mutex<HashMap<i32, Contract>>,
) -> Option<Contract> {
    let cid = match event {
        FpssEvent::Data(FpssData::Quote { contract_id, .. })
        | FpssEvent::Data(FpssData::Trade { contract_id, .. })
        | FpssEvent::Data(FpssData::OpenInterest { contract_id, .. })
        | FpssEvent::Data(FpssData::Ohlcvc { contract_id, .. }) => *contract_id,
        _ => return None,
    };
    let map = contract_map.lock().unwrap_or_else(|e| e.into_inner());
    map.get(&cid).cloned()
}

/// Convert an `FpssEvent` to the Java terminal's WebSocket JSON format.
///
/// `peeked_contract` should be the contract already looked up from the shared
/// map (see [`lookup_event_contract`]). Passing it in lets the caller release
/// the map lock before serialization, so the FPSS callback thread and the
/// broadcast task never contend on the lock during JSON encoding.
fn fpss_event_to_ws_json(event: &FpssEvent, peeked_contract: Option<&Contract>) -> Option<String> {
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

/// Start the FPSS -> WebSocket bridge via `ThetaDataDx::start_streaming()`.
///
/// The Disruptor callback runs on a blocking consumer thread and must stay
/// cheap. It only: (1) updates the contract map and connection flags,
/// (2) hands a cloned event to an unbounded channel. A dedicated tokio task
/// locks the map, serializes the JSON, and fans out to every WS client.
pub fn start_fpss_bridge(state: AppState) -> Result<(), thetadatadx::Error> {
    let contract_map: Arc<Mutex<HashMap<i32, Contract>>> = state.contract_map();
    let map_for_cb = Arc::clone(&contract_map);
    let map_for_task = Arc::clone(&contract_map);
    let state_for_cb = state.clone();
    let state_for_task = state.clone();

    // Unbounded mpsc keeps the Disruptor callback non-blocking even if the
    // broadcast task is briefly slow. Memory is bounded by channel drain
    // rate; clients get bounded per-client backpressure inside broadcast_ws.
    //
    // Per-tick clone is intentionally cheap: `FpssData::{Quote,Trade,Ohlcvc,
    // OpenInterest}` carry only primitives plus `Arc<str>` for symbol, so
    // `event.clone()` is a field copy + refcount bump — not a heap allocation
    // on the hot path.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<FpssEvent>();

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            // Fetch the event's contract (if any) under the map lock, then
            // drop the lock BEFORE running the O(fields) JSON serialization.
            // This stops the broadcast task and the FPSS callback from
            // contending on the same Mutex during encoding.
            let peeked = lookup_event_contract(&event, &map_for_task);
            let json = fpss_event_to_ws_json(&event, peeked.as_ref());
            if let Some(ws_json) = json {
                let msg: Arc<str> = Arc::from(ws_json);
                state_for_task.broadcast_ws(msg);
            }
        }
    });

    state.tdx().start_streaming(move |event: &FpssEvent| {
        // Track contract assignments. Must happen on the callback thread so
        // the broadcast task sees the mapping before it serializes the next
        // event that references it.
        if let FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) = event {
            if let Ok(mut map) = map_for_cb.lock() {
                map.insert(*id, contract.clone());
            }
        }

        // Update connection status.
        match event {
            FpssEvent::Control(FpssControl::LoginSuccess { .. }) => {
                state_for_cb.set_fpss_connected(true);
            }
            FpssEvent::Control(FpssControl::Disconnected { .. }) => {
                state_for_cb.set_fpss_connected(false);
            }
            _ => {}
        }

        // Hand off for serialization + broadcast. Callback returns immediately.
        let _ = tx.send(event.clone());
    })?;

    state.set_fpss_connected(true);
    Ok(())
}
