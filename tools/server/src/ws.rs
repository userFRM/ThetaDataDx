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
//!
//! # Hardening
//!
//! The WS router composes the same three layers as the REST router
//! (`router::build`): a 256-wide `ConcurrencyLimitLayer`, a 64 KiB
//! `DefaultBodyLimit`, and a per-peer-IP `GovernorLayer` (20 rps, burst 40).
//! On top of that, `handle_client_message` rejects any `Message::Text`
//! longer than [`WS_MAX_TEXT_BYTES`]. A legitimate subscribe / stop command
//! is well under 200 bytes; anything larger is attack-shaped and discarded
//! before `sonic_rs::from_str` touches it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use sonic_rs::prelude::*;
use tokio::sync::mpsc;
use tower::limit::ConcurrencyLimitLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::PeerIpKeyExtractor;
use tower_governor::GovernorLayer;

use tdbe::types::enums::SecType;
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssControl, FpssData, FpssEvent};

use crate::state::AppState;
use crate::validation;

/// Max accepted payload size for a single client `Message::Text` frame.
///
/// A legitimate subscribe / stop envelope is <200 bytes; 4 KiB leaves
/// comfortable headroom for long ticker lists or extra fields while
/// rejecting a multi-megabyte JSON bomb before `sonic_rs::from_str`
/// touches the bytes.
pub(crate) const WS_MAX_TEXT_BYTES: usize = 4 * 1024;

/// Inclusive lower bound on option expiration dates (YYYYMMDD). Any
/// earlier value is an attacker probing the contract keyspace.
pub(crate) const MIN_OPTION_EXP: i32 = 19000101;

/// Inclusive upper bound on option expiration dates (YYYYMMDD). ThetaData
/// supports LEAPS out a few decades; 2100 is a hard ceiling that the
/// underlying MDDS would reject anyway.
pub(crate) const MAX_OPTION_EXP: i32 = 21000101;

/// Return `true` iff `exp` is within the accepted YYYYMMDD range for an
/// option subscription. Cheap integer check — no allocations.
pub(crate) fn is_valid_yyyymmdd_range(exp: i32) -> bool {
    (MIN_OPTION_EXP..=MAX_OPTION_EXP).contains(&exp)
}

/// Mirrors `router::GLOBAL_CONCURRENCY_LIMIT` — single constant would cross
/// the module boundary gratuitously. 256 is chosen for the same reason:
/// enough headroom for bursty clients, tight enough to shed pressure at
/// the edge before it hits tokio task slots.
const WS_CONCURRENCY_LIMIT: usize = 256;

/// Mirrors `router::BODY_LIMIT_BYTES`. The WS upgrade request itself is
/// small; this cap prevents a malicious upgrade handshake from pushing a
/// multi-MB body through the axum extractor chain.
const WS_BODY_LIMIT_BYTES: usize = 64 * 1024;

/// Per-IP rate for the WS upgrade path. Matches the REST router.
const WS_GENERAL_PER_SECOND: u64 = 20;
const WS_GENERAL_BURST_SIZE: u32 = 40;

/// Build the WebSocket router (single route: `/v1/events`).
///
/// Applies the same hardening layers as `router::build`:
///
/// 1. `ConcurrencyLimitLayer` caps in-flight WS upgrades to
///    [`WS_CONCURRENCY_LIMIT`]; the single-client invariant is still
///    enforced downstream via `state.try_acquire_ws`, but this stops
///    attackers from queueing thousands of blocked upgrades.
/// 2. `DefaultBodyLimit` caps the upgrade request body at
///    [`WS_BODY_LIMIT_BYTES`].
/// 3. `GovernorLayer` keyed on the peer connect-info IP enforces
///    [`WS_GENERAL_PER_SECOND`] rps with a burst of [`WS_GENERAL_BURST_SIZE`].
///    Peer-IP-only — `X-Forwarded-For` is ignored (see `router.rs` for
///    rationale).
pub fn router(state: AppState) -> Router {
    let governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(PeerIpKeyExtractor)
            .per_second(WS_GENERAL_PER_SECOND)
            .burst_size(WS_GENERAL_BURST_SIZE)
            .finish()
            .expect("ws governor config invariants hold at build time"),
    );

    // Matches the REST router: periodically purge stale per-IP buckets so
    // the rate-limit map cannot grow unbounded under churn.
    let cleanup = Arc::clone(&governor);
    tokio::spawn(async move {
        let interval = Duration::from_secs(60);
        loop {
            tokio::time::sleep(interval).await;
            cleanup.limiter().retain_recent();
        }
    });

    Router::new()
        .route("/v1/events", get(ws_upgrade))
        .layer(ConcurrencyLimitLayer::new(WS_CONCURRENCY_LIMIT))
        .layer(DefaultBodyLimit::max(WS_BODY_LIMIT_BYTES))
        .layer(GovernorLayer::new(governor))
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
                // Never send an empty WS frame when serialization fails --
                // downstream clients have no way to distinguish an empty
                // string from a valid heartbeat and will silently drop it.
                // A serialization failure on the server-built heartbeat is
                // either a bug in sonic_rs or a panic elsewhere; treat it
                // as fatal for this connection and log so operators notice.
                let text = match sonic_rs::to_string(&msg) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "ws heartbeat serialize failed; closing socket");
                        break;
                    }
                };
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

/// Serialize a response envelope and send it to the client.
///
/// Never sends an empty WS frame on serialization failure -- logs the
/// error instead. The socket is left open so the client can retry the
/// command. Callers that must close on serialize failure should inspect
/// the return value (`false` = not sent) and propagate.
async fn send_response(socket: &mut WebSocket, resp: &sonic_rs::Value, ctx: &str) -> bool {
    match sonic_rs::to_string(resp) {
        Ok(s) => socket.send(Message::Text(s.into())).await.is_ok(),
        Err(e) => {
            tracing::error!(error = %e, context = %ctx, "ws response serialize failed; dropping");
            false
        }
    }
}

/// Parse and handle a client subscription command.
async fn handle_client_message(state: &AppState, text: &str, socket: &mut WebSocket) {
    // Reject oversize Text frames BEFORE handing bytes to the JSON parser.
    // Without this, a 2 GB subscribe envelope would stream into
    // `sonic_rs::from_str` and could OOM the process long before
    // validation kicked in. 4 KiB is strictly above any legitimate
    // subscribe envelope emitted by the Python SDK / terminal clients
    // (observed ~120-200 bytes).
    if text.len() > WS_MAX_TEXT_BYTES {
        tracing::warn!(
            bytes = text.len(),
            limit = WS_MAX_TEXT_BYTES,
            "WS client Text frame exceeds cap; rejecting"
        );
        let err_msg = format!(
            "text frame exceeds maximum of {WS_MAX_TEXT_BYTES} bytes (got {})",
            text.len()
        );
        let resp = sonic_rs::json!({
            "header": {
                "type": "REQ_RESPONSE",
                "response": "ERROR",
                "req_id": 0,
                "error": err_msg.as_str(),
            }
        });
        send_response(socket, &resp, "oversize_text_reply").await;
        return;
    }

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
            send_response(socket, &resp, "invalid_json_reply").await;
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
        send_response(socket, &resp, "stop_reply").await;
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

    // Bound the client-supplied ticker root length BEFORE the string flows
    // into `Contract::stock(root)` / `Contract::option_raw(root, ...)`.
    // Without this a malicious client can send a multi-megabyte `"root"`
    // value in the JSON subscribe envelope, triggering allocation inside
    // the FPSS contract map keyed by that string. Mirrors the REST
    // validation performed in `handler::build_endpoint_args`.
    if let Err(e) = validation::validate_symbol(root, "root") {
        tracing::warn!(error = %e, "WS subscribe: root failed length validation");
        let resp = sonic_rs::json!({
            "header": {
                "type": "REQ_RESPONSE",
                "response": "ERROR",
                "req_id": req_id,
                "error": e.message.as_str(),
            }
        });
        send_response(socket, &resp, "bad_request_reply").await;
        return;
    }

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
        // Reject externally-sourced values that don't fit `i32`. Silent
        // narrowing (`as i32`) on client input is a principle violation:
        // a caller sending `strike = 9_000_000_000` would have wrapped
        // to a garbage ThetaData contract instead of surfacing the bad
        // request. Both `expiration` and `strike` are parsed fallibly.
        let exp_val = contract_obj.get("expiration").unwrap_or(&null_val);
        let exp_i64 = match exp_val.as_i64() {
            Some(v) => v,
            None => {
                tracing::warn!("WS subscribe: option expiration missing or not an integer");
                let resp = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": "expiration must be an integer",
                    }
                });
                send_response(socket, &resp, "bad_request_reply").await;
                return;
            }
        };
        let exp = match i32::try_from(exp_i64) {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    expiration = exp_i64,
                    "WS subscribe: option expiration out of i32 range"
                );
                let err_msg = format!("expiration {exp_i64} exceeds i32 range");
                let resp = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": err_msg.as_str(),
                    }
                });
                send_response(socket, &resp, "bad_request_reply").await;
                return;
            }
        };
        // Range-check expiration in the YYYYMMDD domain. Any value outside
        // 1900-01-01..=2100-01-01 is a format error or an attacker probing
        // the FPSS contract-map keyspace with garbage ids. `i32::try_from`
        // already fenced the width; this fences the semantic range before
        // it reaches `Contract::option_raw`.
        if !is_valid_yyyymmdd_range(exp) {
            tracing::warn!(
                expiration = exp,
                "WS subscribe: option expiration out of YYYYMMDD range"
            );
            let err_msg = format!(
                "'exp' out of range (expected YYYYMMDD {}..={}; got {exp})",
                MIN_OPTION_EXP, MAX_OPTION_EXP
            );
            let resp = sonic_rs::json!({
                "header": {
                    "type": "REQ_RESPONSE",
                    "response": "ERROR",
                    "req_id": req_id,
                    "error": err_msg.as_str(),
                }
            });
            send_response(socket, &resp, "bad_request_reply").await;
            return;
        }
        let strike_val = contract_obj.get("strike").unwrap_or(&null_val);
        let strike_i64 = match strike_val.as_i64() {
            Some(v) => v,
            None => {
                tracing::warn!("WS subscribe: option strike missing or not an integer");
                let resp = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": "strike must be an integer",
                    }
                });
                send_response(socket, &resp, "bad_request_reply").await;
                return;
            }
        };
        let strike = match i32::try_from(strike_i64) {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    strike = strike_i64,
                    "WS subscribe: option strike out of i32 range"
                );
                let err_msg = format!("strike {strike_i64} exceeds i32 range");
                let resp = sonic_rs::json!({
                    "header": {
                        "type": "REQ_RESPONSE",
                        "response": "ERROR",
                        "req_id": req_id,
                        "error": err_msg.as_str(),
                    }
                });
                send_response(socket, &resp, "bad_request_reply").await;
                return;
            }
        };
        // Strike is an OPRA-encoded price (thousandths of a dollar). A
        // non-positive value is never legal — zero is impossible and
        // negatives would wrap to a garbage key in `Contract::option_raw`.
        if strike <= 0 {
            tracing::warn!(
                strike = strike,
                "WS subscribe: option strike must be positive"
            );
            let err_msg = format!("'strike' must be positive (got {strike})");
            let resp = sonic_rs::json!({
                "header": {
                    "type": "REQ_RESPONSE",
                    "response": "ERROR",
                    "req_id": req_id,
                    "error": err_msg.as_str(),
                }
            });
            send_response(socket, &resp, "bad_request_reply").await;
            return;
        }
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
        send_response(socket, &resp, "subscription_reply").await;
    } else {
        tracing::warn!("FPSS streaming not started, subscription command ignored");
        let resp = sonic_rs::json!({
            "header": { "type": "REQ_RESPONSE", "response": "OK", "req_id": req_id }
        });
        send_response(socket, &resp, "streaming_off_reply").await;
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
/// (2) peeks the event's current contract under the map lock, and
/// (3) hands a cloned event + peeked contract snapshot to an unbounded
/// channel. A dedicated tokio task serializes the JSON and fans out to
/// every WS client.
///
/// # TOCTOU safety
///
/// The `(FpssEvent, Option<Contract>)` tuple pins the contract snapshot
/// captured **at the exact moment the callback thread saw the event**.
/// Before this change, the broadcast task re-looked up the contract just
/// before serialization — which meant a concurrent map `clear()` triggered
/// by a reconnect or market-close could race in between, erasing the
/// contract and silently producing `{"id": N}` JSON with no root / strike /
/// right. That silent degradation is unacceptable across market-close /
/// reconnect boundaries. Peeking-before-send removes the race entirely:
/// the cloned `Contract` value is immune to subsequent map mutations.
pub fn start_fpss_bridge(state: AppState) -> Result<(), thetadatadx::Error> {
    let contract_map: Arc<Mutex<HashMap<i32, Contract>>> = state.contract_map();
    let map_for_cb = Arc::clone(&contract_map);
    let state_for_cb = state.clone();
    let state_for_task = state.clone();

    // Unbounded mpsc keeps the Disruptor callback non-blocking even if the
    // broadcast task is briefly slow. Memory is bounded by channel drain
    // rate; clients get bounded per-client backpressure inside broadcast_ws.
    //
    // Per-tick clone is intentionally cheap: `FpssData::{Quote,Trade,Ohlcvc,
    // OpenInterest}` carry only primitives plus `Arc<str>` for symbol, so
    // `event.clone()` is a field copy + refcount bump — not a heap allocation
    // on the hot path. The `Option<Contract>` tail is an `Arc<str>` for the
    // root plus a few primitives, same cost profile.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(FpssEvent, Option<Contract>)>();

    // Observability counter for the `tx.send` drop path below. Lives on the
    // callback closure so it survives the Disruptor consumer thread's
    // lifetime; shared with broadcast diagnostics via `tracing::debug!`.
    let dropped_broadcast: Arc<std::sync::atomic::AtomicU64> =
        Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dropped_broadcast = Arc::clone(&dropped_broadcast);

    tokio::spawn(async move {
        while let Some((event, peeked)) = rx.recv().await {
            // Snapshot already taken on the callback thread — just encode
            // and broadcast. No map lock acquired in the broadcast task.
            let json = fpss_event_to_ws_json(&event, peeked.as_ref());
            if let Some(ws_json) = json {
                let msg: Arc<str> = Arc::from(ws_json);
                state_for_task.broadcast_ws(msg).await;
            }
        }
    });

    state.tdx().start_streaming(move |event: &FpssEvent| {
        // Track contract assignments. Must happen on the callback thread so
        // the broadcast task sees the mapping before it serializes the next
        // event that references it.
        if let FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) = event {
            // Recover from poisoning rather than silently dropping all
            // future ContractAssigned events. If a previous lock-holder
            // panicked, the map state may be partial but that is strictly
            // less bad than losing every subsequent symbol assignment.
            let mut map = map_for_cb.lock().unwrap_or_else(|e| e.into_inner());
            map.insert(*id, contract.clone());
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

        // Peek the contract for this event NOW, while the callback thread
        // still holds the causal ordering with `ContractAssigned` /
        // reconnect clears. Cloning the `Contract` value captures a
        // snapshot that no subsequent map mutation can invalidate — even
        // if a reconnect clears the map before the broadcast task wakes,
        // the cloned `Contract` travels with the event downstream. This
        // is the documented fix for the reconnect / market-close silent-
        // degradation race; see module docs for detail.
        let peeked = lookup_event_contract(event, &map_for_cb);

        // Hand off for serialization + broadcast. Callback returns
        // immediately. A `SendError` here means the broadcast task has
        // exited (shutdown, panic, receiver dropped) — route it to
        // `tracing::debug!` with a monotonically-increasing counter so
        // soak tests can detect back-pressure / task death, matching the
        // observability pattern used by the SDK streaming callbacks
        // (see `crates/thetadatadx/build_support/sdk_surface.rs`).
        if tx.send((event.clone(), peeked)).is_err() {
            let dropped = dropped_broadcast
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                .wrapping_add(1);
            tracing::debug!(
                target: "thetadatadx::server::ws",
                dropped_total = dropped,
                "fpss event dropped — broadcast task is gone"
            );
        }
    })?;

    state.set_fpss_connected(true);
    Ok(())
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    //  M4 — exp / strike bounds
    // -----------------------------------------------------------------------

    #[test]
    fn yyyymmdd_accepts_realistic_range() {
        assert!(is_valid_yyyymmdd_range(20260420));
        assert!(is_valid_yyyymmdd_range(20000101));
        assert!(is_valid_yyyymmdd_range(19500615));
    }

    #[test]
    fn yyyymmdd_accepts_boundaries() {
        assert!(is_valid_yyyymmdd_range(MIN_OPTION_EXP));
        assert!(is_valid_yyyymmdd_range(MAX_OPTION_EXP));
    }

    #[test]
    fn yyyymmdd_rejects_below_range() {
        assert!(!is_valid_yyyymmdd_range(0));
        assert!(!is_valid_yyyymmdd_range(18991231));
        assert!(!is_valid_yyyymmdd_range(MIN_OPTION_EXP - 1));
        assert!(!is_valid_yyyymmdd_range(-20260420));
    }

    #[test]
    fn yyyymmdd_rejects_above_range() {
        assert!(!is_valid_yyyymmdd_range(21000102));
        assert!(!is_valid_yyyymmdd_range(99999999));
        assert!(!is_valid_yyyymmdd_range(MAX_OPTION_EXP + 1));
        assert!(!is_valid_yyyymmdd_range(i32::MAX));
    }

    // -----------------------------------------------------------------------
    //  C1 — WS text frame cap
    // -----------------------------------------------------------------------

    #[test]
    fn ws_text_cap_is_tight() {
        // Sanity: the cap must be exactly 4 KiB. Hard number instead of
        // a self-referential assertion so a future typo in the constant
        // trips this test. A larger value would re-open the 2 GB text
        // frame OOM vector this cap was introduced to close.
        assert_eq!(WS_MAX_TEXT_BYTES, 4 * 1024);
    }

    // -----------------------------------------------------------------------
    //  F1 — TOCTOU fix: channel tuple carries the peeked Contract
    // -----------------------------------------------------------------------

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

    #[test]
    fn contract_snapshot_survives_concurrent_map_clear() {
        let map: Arc<Mutex<HashMap<i32, Contract>>> = Arc::new(Mutex::new(HashMap::new()));

        // Pre-populate the map with a contract the event will reference.
        {
            let contract = Contract::stock("AAPL");
            map.lock().unwrap().insert(42, contract);
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
        // contract — serialization must succeed with root = "AAPL".
        let json = fpss_event_to_ws_json(&event, peeked.as_ref())
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
