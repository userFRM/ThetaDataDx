//! Per-connection WebSocket session: main loop and response helper.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use tokio::sync::mpsc;

use crate::state::AppState;

use super::subscribe;

/// Main WebSocket connection handler.
///
/// Multiplexes three event sources in `tokio::select!`:
/// 1. Heartbeat tick (1s) -> send STATUS
/// 2. Per-client mpsc events -> forward to client (zero-copy `Arc<str>`)
/// 3. Client messages -> process subscription commands
pub(super) async fn handle_socket(mut socket: WebSocket, state: AppState) {
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
    state.release_ws();
    tracing::debug!("WebSocket connection closed");
}

/// Serialize a response envelope and send it to the client.
///
/// Never sends an empty WS frame on serialization failure -- logs the
/// error instead. The socket is left open so the client can retry the
/// command. Callers that must close on serialize failure should inspect
/// the return value (`false` = not sent) and propagate.
pub(super) async fn send_response(
    socket: &mut WebSocket,
    resp: &sonic_rs::Value,
    ctx: &str,
) -> bool {
    match sonic_rs::to_string(resp) {
        Ok(s) => socket.send(Message::Text(s.into())).await.is_ok(),
        Err(e) => {
            tracing::error!(error = %e, context = %ctx, "ws response serialize failed; dropping");
            false
        }
    }
}
