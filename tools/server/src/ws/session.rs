//! Per-connection WebSocket session: main loop and response helper.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use tokio::sync::mpsc;

use crate::state::AppState;

use super::subscribe;

/// Main WebSocket connection handler.
///
/// Multiplexes four event sources in `tokio::select!`:
/// 1. Session close signal (a newer client replaced this session)
///    -> send a Close frame and exit
/// 2. Heartbeat tick (1s) -> send STATUS
/// 3. Per-client mpsc events -> forward to client (zero-copy `Arc<str>`)
/// 4. Client messages -> process subscription commands
pub(super) async fn handle_socket(mut socket: WebSocket, state: AppState) {
    // Beginning the session fires the close signal of any session that
    // was already active — single-client semantics with replacement,
    // matching the legacy terminal.
    let close_signal = state.begin_ws_session();
    let mut ws_rx: mpsc::Receiver<Arc<str>> = state.register_ws_client().await;
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(1));

    tracing::debug!("WebSocket client connected");

    loop {
        tokio::select! {
            _ = close_signal.notified() => {
                tracing::info!("WebSocket session replaced by a new client; closing");
                // Best-effort Close frame so the displaced client learns
                // WHY instead of seeing a bare TCP reset.
                let frame = axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::NORMAL,
                    reason: "replaced by a new client connection".into(),
                };
                let _ = socket.send(Message::Close(Some(frame))).await;
                break;
            }

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
                        subscribe::handle_client_message(&state, &text, &mut socket).await;
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
    // Clears the active-session slot only if this session still owns it —
    // a replaced session exiting late must not evict its replacement.
    state.end_ws_session(&close_signal);
    tracing::debug!("WebSocket connection closed");
}

/// Serialize a response envelope and send it to the client.
///
/// Never sends an empty WS frame on serialization failure -- logs the
/// error instead. The socket is left open so the client can retry the
/// command.
pub(super) async fn send_response(socket: &mut WebSocket, resp: &sonic_rs::Value, ctx: &str) {
    match sonic_rs::to_string(resp) {
        Ok(s) => {
            let _ = socket.send(Message::Text(s.into())).await;
        }
        Err(e) => {
            tracing::error!(error = %e, context = %ctx, "ws response serialize failed; dropping");
        }
    }
}
