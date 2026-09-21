//! HTTP -> WebSocket upgrade handshake.

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::response::IntoResponse;

use crate::state::AppState;

use super::session;
use super::subscribe::WS_MAX_TEXT_BYTES;

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
    // Bound the frame and message size at the protocol layer so an
    // oversize frame is rejected by the codec before its bytes are
    // buffered. The codec's default limits (64 MiB message, 16 MiB
    // frame) let a client force ~64 MiB allocations per frame even
    // though the only message this server reads from a client is a
    // subscribe / stop envelope of a few hundred bytes. Capping at
    // `WS_MAX_TEXT_BYTES` makes the protocol layer reject anything
    // larger than a legitimate command before it reaches the buffer.
    // The application-level check in `subscribe::handle_client_message`
    // stays as defense-in-depth.
    ws.max_message_size(PROTOCOL_FRAME_CAP)
        .max_frame_size(PROTOCOL_FRAME_CAP)
        .on_upgrade(move |socket| session::handle_socket(socket, state))
        .into_response()
}

/// Protocol-layer ceiling for a client frame and message, in bytes.
///
/// Pinned to the application-level `WS_MAX_TEXT_BYTES` cap so the codec
/// rejects an oversize frame before buffering it, and so the two layers
/// can never drift apart. The only message this server reads from a
/// client is a subscribe / stop envelope, which the application cap
/// already bounds; raising this above that cap would re-open the
/// large-allocation vector the protocol bound exists to close.
const PROTOCOL_FRAME_CAP: usize = WS_MAX_TEXT_BYTES;
// The cap must stay far below the codec defaults (64 MiB message / 16 MiB
// frame) it overrides: at or above them it is a no-op and re-opens the
// large-allocation vector it exists to close. 64 KiB is a ceiling any
// legitimate subscribe envelope clears with room to spare. This is the
// whole check — a widening fails the build. It needs no test around it,
// and the two caps cannot drift because one is defined as the other.
const _: () = assert!(
    PROTOCOL_FRAME_CAP <= 64 * 1024,
    "protocol cap is not tight enough to bound allocations"
);
