//! HTTP -> WebSocket upgrade handshake.

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::response::IntoResponse;

use crate::state::AppState;

use super::session;

/// Handle the HTTP -> WebSocket upgrade.
///
/// The upgrade always succeeds; the single-client invariant is enforced
/// with REPLACEMENT semantics inside `session::handle_socket` — beginning
/// the new session fires the close signal of any session already active,
/// which sends a Close frame and exits. This matches the legacy terminal,
/// which drops the existing client to let the new one in (a stuck or
/// half-dead client must never lock out its replacement).
pub(super) async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    tracing::debug!("WebSocket upgrade request");
    ws.on_upgrade(move |socket| session::handle_socket(socket, state))
        .into_response()
}
