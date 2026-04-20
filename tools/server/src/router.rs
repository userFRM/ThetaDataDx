//! Route generation from the endpoint registry.
//!
//! Iterates `ENDPOINTS` at startup and registers a handler for every one of
//! the 61 endpoints, plus system routes. Each endpoint is mapped to a REST
//! path following the ThetaData v3 API convention. Paths are generated in the
//! core registry so the REST server does not re-derive them heuristically.
//!
//! # Hardening layers
//!
//! `build()` composes three security-relevant layers on top of the registry
//! routes:
//!
//! 1. `DefaultBodyLimit` (64 KB) caps total request-body size -- primary
//!    defense against memory-DoS on POSTs / multipart. Query-string length
//!    is enforced per-field in `handler::build_endpoint_args` via the
//!    `validation` module (axum doesn't expose a URL-length knob cleanly).
//! 2. `ConcurrencyLimitLayer` (256) caps the number of in-flight requests,
//!    preventing a flood of slow clients from exhausting the tokio
//!    runtime's task slots.
//! 3. `tower_governor::GovernorLayer` enforces a per-IP rate limit
//!    (20 rps, burst 40) on every route. The shutdown endpoint gets an
//!    additional, tighter `route_layer` (~3 attempts per hour per IP) so
//!    token-guessing costs real wall-clock time.
//!
//! Layer order matters: `GovernorLayer` (outer) sees the request first and
//! rejects with 429 before `ConcurrencyLimitLayer` acquires a runtime slot.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use tower::limit::ConcurrencyLimitLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::SmartIpKeyExtractor;
use tower_governor::GovernorLayer;

use thetadatadx::registry::{EndpointMeta, ENDPOINTS};

use crate::handler;
use crate::state::AppState;

/// Total concurrent in-flight requests across all routes.
///
/// At 256 simultaneous slow-client connections the server still has headroom
/// for bursty health-check polling; beyond that, the tokio task pool is the
/// next bottleneck and we'd rather return pressure at the edge.
const GLOBAL_CONCURRENCY_LIMIT: usize = 256;

/// Max request body size. 64 KB comfortably covers any realistic query
/// string + headers for this API; anything larger is DoS or a broken
/// client.
const BODY_LIMIT_BYTES: usize = 64 * 1024;

/// General per-IP quota: 20 requests per second with a burst of 40. Tuned
/// for backtest fetches (bursty metadata pulls followed by idle time).
const GENERAL_PER_SECOND: u64 = 20;
const GENERAL_BURST_SIZE: u32 = 40;

/// Shutdown endpoint quota: ~3 attempts per IP per hour.
///
/// `tower_governor::GovernorConfigBuilder::per_second(n)` treats `n` as
/// "requests allowed per second" — passing 3600 would allow 3600 rps,
/// which is the opposite of what we want. The crate's `period(Duration)`
/// setter instead sets the INTERVAL between token replenishments: one
/// token per `SHUTDOWN_REPLENISH_PERIOD`. Combined with `burst_size(3)`,
/// a single IP can issue at most three attempts before the bucket empties
/// and must wait one full hour for each subsequent slot. UUID-v4 entropy
/// already makes brute-force infeasible, but this pins an upper bound on
/// guess rate regardless of token length.
const SHUTDOWN_REPLENISH_PERIOD: Duration = Duration::from_secs(3600);
const SHUTDOWN_BURST_SIZE: u32 = 3;

// ---------------------------------------------------------------------------
//  Router construction
// ---------------------------------------------------------------------------

/// Build the full REST API router with routes dynamically generated from
/// the endpoint registry plus hand-written system routes.
///
/// Default port: 25503 (matching ThetaData v3 terminal).
pub fn build(state: AppState) -> Router {
    let mut app = Router::new();
    let mut registered = 0usize;

    for ep in ENDPOINTS {
        let ep_arc: &'static EndpointMeta = ep;
        let ep_shared = Arc::new(ep_arc);
        let handler_fn =
            move |s: axum::extract::State<AppState>,
                  q: axum::extract::Query<std::collections::HashMap<String, String>>| {
                let ep = Arc::clone(&ep_shared);
                async move { handler::generic(s, q, &ep).await }
            };
        app = app.route(ep.rest_path, get(handler_fn));
        registered += 1;
        tracing::debug!(endpoint = ep.name, path = ep.rest_path, "registered route");
    }

    tracing::info!(
        count = registered,
        "registered endpoint routes from registry"
    );

    // Build the per-route governor for the shutdown endpoint. Must be
    // attached with `route_layer` so it only applies to `/v3/system/shutdown`
    // and not to the sibling system-status routes.
    let shutdown_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(SmartIpKeyExtractor)
            .period(SHUTDOWN_REPLENISH_PERIOD)
            .burst_size(SHUTDOWN_BURST_SIZE)
            .finish()
            .expect("shutdown governor config invariants hold at build time"),
    );

    // System routes. The shutdown route gets a tighter, route-scoped
    // governor; the rest fall under the global governor attached below.
    app = app
        .route("/v3/system/status", get(handler::system_status))
        .route("/v3/system/mdds/status", get(handler::system_mdds_status))
        .route("/v3/system/fpss/status", get(handler::system_fpss_status))
        .route(
            "/v3/system/shutdown",
            post(handler::system_shutdown).route_layer(GovernorLayer::new(shutdown_governor)),
        );

    // Global per-IP governor (outermost rate limit). All routes inherit
    // this; the shutdown route additionally enforces the per-route governor
    // above.
    let global_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(SmartIpKeyExtractor)
            .per_second(GENERAL_PER_SECOND)
            .burst_size(GENERAL_BURST_SIZE)
            .finish()
            .expect("general governor config invariants hold at build time"),
    );

    // `GovernorLayer` spawns a background task that periodically purges
    // stale per-IP buckets to keep the map from growing unbounded under
    // churn. One task is enough; share the same Arc across both layers.
    let global_cleanup = Arc::clone(&global_governor);
    tokio::spawn(async move {
        let interval = Duration::from_secs(60);
        loop {
            tokio::time::sleep(interval).await;
            global_cleanup.limiter().retain_recent();
        }
    });

    // `.layer(X).layer(Y)` in axum/tower makes Y wrap X (outer wraps inner),
    // so the LAST `.layer(...)` call is the outermost request wrapper. We
    // want the governor to reject rate-exceeded requests BEFORE they
    // consume a concurrency permit or body-limit slot, so it lives last.
    app.layer(ConcurrencyLimitLayer::new(GLOBAL_CONCURRENCY_LIMIT))
        .layer(DefaultBodyLimit::max(BODY_LIMIT_BYTES))
        .layer(GovernorLayer::new(global_governor))
        .with_state(state)
}
