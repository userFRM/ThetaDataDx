//! WebSocket server with full FPSS bridge.
//!
//! Replicates the JVM terminal's WebSocket behavior:
//!
//! - Single WebSocket endpoint at `/v1/events`
//! - Only one WebSocket client at a time (enforced via `AtomicBool`)
//! - Clients receive JSON events: QUOTE, TRADE, OHLC, STATUS
//! - STATUS heartbeat every 1 second with FPSS connection state
//! - Client commands: subscribe/unsubscribe via JSON messages
//!
//! # FPSS Bridge
//!
//! `start_fpss_bridge()` connects an `StreamingClient` whose callback converts
//! each `StreamEvent` to JSON and broadcasts it to all WS clients.
//!
//! # Hardening
//!
//! The WS router composes the same generic admission layers as the REST
//! router (`router::build`): a 256-wide `ConcurrencyLimitLayer` and a 64 KiB
//! `DefaultBodyLimit`. On top of that, `handle_client_message` rejects any
//! `Message::Text` longer than [`WS_MAX_TEXT_BYTES`]. A legitimate subscribe
//! / stop command is well under 200 bytes; anything larger is attack-shaped
//! and discarded before `sonic_rs::from_str` touches it.

mod broadcast;
mod contract_map;
mod format;
mod session;
mod subscribe;
mod upgrade;

use axum::extract::DefaultBodyLimit;
use axum::routing::get;
use axum::Router;
use tower::limit::ConcurrencyLimitLayer;

use crate::state::AppState;

pub use broadcast::start_fpss_bridge;
pub use format::json_serialize_failure_count;

// The WS router shares the HTTP router's admission caps verbatim so the two
// surfaces shed pressure identically; import the originals rather than mirror
// their values.
use crate::router::{BODY_LIMIT_BYTES, GLOBAL_CONCURRENCY_LIMIT};

/// Build the WebSocket router (single route: `/v1/events`).
///
/// Applies the same generic admission layers as `router::build`:
///
/// 1. `ConcurrencyLimitLayer` caps in-flight WS upgrades to
///    [`GLOBAL_CONCURRENCY_LIMIT`]; the single-client invariant is still
///    enforced downstream via `state.try_acquire_ws`, but this stops
///    attackers from queueing thousands of blocked upgrades.
/// 2. `DefaultBodyLimit` caps the upgrade request body at
///    [`BODY_LIMIT_BYTES`].
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/events", get(upgrade::ws_upgrade))
        .layer(ConcurrencyLimitLayer::new(GLOBAL_CONCURRENCY_LIMIT))
        .layer(DefaultBodyLimit::max(BODY_LIMIT_BYTES))
        .with_state(state)
}
