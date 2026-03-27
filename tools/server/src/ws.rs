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
use tokio::sync::broadcast;

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssControl, FpssData, FpssEvent};
use thetadatadx::types::price::Price;

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
/// 2. FPSS broadcast events -> forward to client
/// 3. Client messages -> process subscription commands
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut ws_rx = state.subscribe_ws();
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

            result = ws_rx.recv() => {
                match result {
                    Ok(event_json) => {
                        if socket.send(Message::Text(event_json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(lagged = n, "WebSocket client lagged, dropped events");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
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
        Contract::option(root, exp, is_call, strike)
    } else {
        Contract::stock(root)
    };

    let tdx = state.tdx();
    if tdx.is_streaming() {
        let result = if is_add {
            match req_type.as_str() {
                "QUOTE" => tdx.subscribe_quotes(&contract),
                "TRADE" => tdx.subscribe_trades(&contract),
                _ => {
                    tracing::warn!(req_type = %req_type, "unknown req_type for subscription");
                    Ok(0)
                }
            }
        } else {
            match req_type.as_str() {
                "QUOTE" => tdx.unsubscribe_quotes(&contract),
                "TRADE" => tdx.unsubscribe_trades(&contract),
                _ => Ok(0),
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

/// Convert an `FpssEvent` to the Java terminal's WebSocket JSON format.
fn fpss_event_to_ws_json(
    event: &FpssEvent,
    contract_map: &HashMap<i32, Contract>,
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
                    price_type,
                    date,
                } => (
                    "QUOTE",
                    *contract_id,
                    sonic_rs::json!({
                        "ms_of_day": ms_of_day,
                        "bid_size": bid_size,
                        "bid_exchange": bid_exchange,
                        "bid": Price::new(*bid, *price_type).to_f64(),
                        "bid_condition": bid_condition,
                        "ask_size": ask_size,
                        "ask_exchange": ask_exchange,
                        "ask": Price::new(*ask, *price_type).to_f64(),
                        "ask_condition": ask_condition,
                        "date": date,
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
                    price_type,
                    date,
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
                        "price": Price::new(*price, *price_type).to_f64(),
                        "date": date,
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
                    price_type,
                    date,
                } => (
                    "OHLC",
                    *contract_id,
                    sonic_rs::json!({
                        "ms_of_day": ms_of_day,
                        "open": Price::new(*open, *price_type).to_f64(),
                        "high": Price::new(*high, *price_type).to_f64(),
                        "low": Price::new(*low, *price_type).to_f64(),
                        "close": Price::new(*close, *price_type).to_f64(),
                        "volume": volume,
                        "count": count,
                        "date": date,
                    }),
                ),
                FpssData::OpenInterest {
                    contract_id,
                    ms_of_day,
                    open_interest,
                    date,
                } => (
                    "OPEN_INTEREST",
                    *contract_id,
                    sonic_rs::json!({
                        "ms_of_day": ms_of_day,
                        "open_interest": open_interest,
                        "date": date,
                    }),
                ),
                _ => return None,
            };

            let contract_json = contract_map
                .get(&contract_id)
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
/// Registers a callback that broadcasts events to WS clients.
pub fn start_fpss_bridge(state: AppState) -> Result<(), thetadatadx::Error> {
    let contract_map: Arc<Mutex<HashMap<i32, Contract>>> = state.contract_map();
    let map_clone = Arc::clone(&contract_map);
    let state_clone = state.clone();

    state.tdx().start_streaming(move |event: &FpssEvent| {
        // Track contract assignments.
        if let FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) = event {
            if let Ok(mut map) = map_clone.lock() {
                map.insert(*id, contract.clone());
            }
        }

        // Update connection status.
        match event {
            FpssEvent::Control(FpssControl::LoginSuccess { .. }) => {
                state_clone.set_fpss_connected(true);
            }
            FpssEvent::Control(FpssControl::Disconnected { .. }) => {
                state_clone.set_fpss_connected(false);
            }
            _ => {}
        }

        // Convert to WS JSON and broadcast.
        let map = map_clone.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ws_json) = fpss_event_to_ws_json(event, &map) {
            state_clone.broadcast_ws(ws_json);
        }
    })?;

    state.set_fpss_connected(true);
    Ok(())
}
