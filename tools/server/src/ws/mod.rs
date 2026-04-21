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

mod broadcast;
mod contract_map;
mod format;
mod session;
mod subscribe;
mod upgrade;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::routing::get;
use axum::Router;
use tower::limit::ConcurrencyLimitLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::PeerIpKeyExtractor;
use tower_governor::GovernorLayer;

use crate::state::AppState;

pub use broadcast::start_fpss_bridge;

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
        .route("/v1/events", get(upgrade::ws_upgrade))
        .layer(ConcurrencyLimitLayer::new(WS_CONCURRENCY_LIMIT))
        .layer(DefaultBodyLimit::max(WS_BODY_LIMIT_BYTES))
        .layer(GovernorLayer::new(governor))
        .with_state(state)
}
