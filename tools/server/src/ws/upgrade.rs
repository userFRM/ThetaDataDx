//! HTTP -> WebSocket upgrade handshake.

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::response::IntoResponse;

use crate::state::AppState;

use super::session;

/// Handle the HTTP -> WebSocket upgrade.
pub(super) async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    tracing::debug!("WebSocket upgrade request");

    if !state.try_acquire_ws() {
        tracing::warn!("WebSocket connection rejected: another client is already connected");
        return (
            axum::http::StatusCode::CONFLICT,
            "only one WebSocket client allowed at a time",
        )
            .into_response();
    }

    ws.on_upgrade(move |socket| session::handle_socket(socket, state))
        .into_response()
}
