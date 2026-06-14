//! Route generation from the endpoint registry.
//!
//! Iterates `ENDPOINTS` at startup and registers a handler for every
//! registry endpoint, plus system routes. Each endpoint is mapped to a REST
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
//!    runtime's task slots. Requests past the cap QUEUE on the layer's
//!    semaphore (they are not rejected) — see the doc on
//!    [`GLOBAL_CONCURRENCY_LIMIT`].
//! 3. `tower_governor::GovernorLayer` enforces a per-IP rate limit
//!    (20 rps, burst 40) on every route — **only on non-loopback binds**.
//!    On `127.0.0.1` / `::1` every local client shares one bucket, so a
//!    parallel backtest or bulk pull rate-limits itself as a group; the
//!    legacy terminal imposes no such limit, and a DoS guard has no
//!    purpose against the local machine. The shutdown endpoint keeps an
//!    independent, tighter `route_layer` (~3 attempts per hour per IP) on
//!    every bind so token-guessing costs real wall-clock time.
//!
//! Layer order matters: `GovernorLayer` (outer) sees the request first and
//! rejects with 429 before `ConcurrencyLimitLayer` acquires a runtime slot.
//!
//! # Trust model
//!
//! The server is NOT deployed behind a trusted reverse proxy. It binds to
//! `127.0.0.1` by default and accepts connections directly from local
//! clients. The rate-limit key extractor uses the peer connect-info IP
//! only: `X-Forwarded-For`, `X-Real-IP`, and `Forwarded` headers are
//! **explicitly ignored**. Trusting any of them would let a malicious
//! local process rotate a synthetic IP on every request to obtain a fresh
//! token bucket, defeating both the general 20 rps cap and the shutdown-
//! token guessing limit. Revisit only if a deployment introduces a
//! validated proxy in front of this server.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;
use tower::limit::ConcurrencyLimitLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::PeerIpKeyExtractor;
use tower_governor::{GovernorError, GovernorLayer};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tower_http::LatencyUnit;
use tracing::Level;

use thetadatadx::{EndpointMeta, ENDPOINTS};

use crate::handler;
use crate::state::AppState;

/// Total concurrent in-flight requests across all routes.
///
/// At 256 simultaneous slow-client connections the server still has headroom
/// for bursty health-check polling; beyond that, the tokio task pool is the
/// next bottleneck and we'd rather return pressure at the edge.
///
/// # Semantics: queue, not reject
///
/// `tower::limit::ConcurrencyLimit` acquires a shared-semaphore permit in
/// `poll_ready` — request 257 WAITS for a slot, it is never rejected, and
/// the layer introduces no error of its own (its error type is the inner
/// service's, `Infallible` under axum). Two queues therefore compose on
/// every request path: this 256-wide admission queue at the HTTP edge,
/// then the SDK's tier-sized request semaphore (`Semaphore::new(pool_size)`
/// in the MDDS client) which serialises dispatch across the upstream gRPC
/// channel pool. A burst larger than the upstream tier cap queues FIFO and
/// drains as slots free; the caller's only deadline is its own client-side
/// timeout (the future drops, releasing both permits). The full model is
/// documented in `docs-site/docs/server/http.md` under "Concurrency
/// model".
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

/// Map a `tower_governor` rejection onto the canonical error envelope.
///
/// The crate's default responder emits `429` with a plain-text body and
/// a non-standard `x-ratelimit-after` header; clients that parse
/// `header.error_type` to drive retry logic broke on the only path that
/// did not emit JSON. This handler emits the same
/// `{"header":{"error_type","error_msg"},"response":[]}` envelope as
/// every other failure, plus a standards-compliant `Retry-After` header
/// in seconds (RFC 9110) — the field every HTTP retry helper reads
/// natively. Shared by the REST shutdown limiter, the general
/// non-loopback limiter, and the WS upgrade limiter.
pub(crate) fn governor_error_response(error: GovernorError) -> axum::response::Response {
    match error {
        GovernorError::TooManyRequests { wait_time, .. } => {
            let mut resp = handler::error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                &format!("too many requests; retry after {wait_time}s"),
            );
            if let Ok(value) = axum::http::HeaderValue::from_str(&wait_time.to_string()) {
                resp.headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, value);
            }
            resp
        }
        GovernorError::UnableToExtractKey => handler::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "rate limiter could not determine the client address",
        ),
        GovernorError::Other { code, msg, .. } => handler::error_response(
            code,
            "server_error",
            msg.as_deref()
                .unwrap_or("rate limiter rejected the request"),
        ),
    }
}

/// Whether `bind` is a loopback address (`127.0.0.0/8`, `::1`).
///
/// Drives the per-IP rate-limiter decision: loopback binds skip the
/// general `GovernorLayer` because every local client shares the same
/// peer IP and would throttle each other as a group. Unparseable values
/// (hostnames) conservatively count as non-loopback so the limiter stays
/// on when the reachability of the bind is unknown.
pub(crate) fn is_loopback_bind(bind: &str) -> bool {
    bind.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Build the full REST API router with routes dynamically generated from
/// the endpoint registry plus hand-written system routes.
///
/// `rate_limit_general` mounts the per-IP general rate limiter; pass
/// `false` for loopback binds (see [`is_loopback_bind`]). The tighter
/// shutdown-route limiter is mounted unconditionally.
///
/// Default port: 25503 (matching ThetaData v3 terminal).
pub fn build(state: AppState, rate_limit_general: bool) -> Router {
    let mut app = Router::new();
    let mut registered = 0usize;

    for ep in ENDPOINTS {
        let ep_arc: &'static EndpointMeta = ep;
        let ep_shared = Arc::new(ep_arc);
        let handler_fn =
            move |s: axum::extract::State<AppState>,
                  q: handler::BoundedQuery<{ handler::MAX_QUERY_PARAMS }>| {
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
            .key_extractor(PeerIpKeyExtractor)
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
            post(handler::system_shutdown).route_layer(
                GovernorLayer::new(shutdown_governor).error_handler(governor_error_response),
            ),
        );

    // Flat-file routes — whole-universe daily blobs over
    // HTTP. Not a WebSocket subscription stream; flat files are batch
    // downloads and the bytes ride a streaming response body so the
    // server doesn't pin multi-hundred-MB blobs in RAM.
    app = crate::flatfile_routes::add_flatfile_routes(app);

    // `.layer(X).layer(Y)` in axum/tower makes Y wrap X (outer wraps inner),
    // so the LAST `.layer(...)` call is the outermost request wrapper. We
    // want the governor to reject rate-exceeded requests BEFORE they
    // consume a concurrency permit or body-limit slot, so it lives last.
    let mut app = app
        .layer(ConcurrencyLimitLayer::new(GLOBAL_CONCURRENCY_LIMIT))
        .layer(DefaultBodyLimit::max(BODY_LIMIT_BYTES));

    // Global per-IP governor (outermost rate limit), DoS guard for
    // non-loopback binds only. On loopback every local client shares one
    // bucket and a parallel backtest throttles itself; the legacy
    // terminal imposes no per-IP limit there. The shutdown route's
    // tighter governor above stays active on every bind.
    if rate_limit_general {
        let global_governor = Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(PeerIpKeyExtractor)
                .per_second(GENERAL_PER_SECOND)
                .burst_size(GENERAL_BURST_SIZE)
                .finish()
                .expect("general governor config invariants hold at build time"),
        );

        // `GovernorLayer` needs a background task that periodically purges
        // stale per-IP buckets to keep the map from growing unbounded under
        // churn.
        let global_cleanup = Arc::clone(&global_governor);
        tokio::spawn(async move {
            let interval = Duration::from_secs(60);
            loop {
                tokio::time::sleep(interval).await;
                global_cleanup.limiter().retain_recent();
            }
        });

        app = app.layer(GovernorLayer::new(global_governor).error_handler(governor_error_response));
    }

    // Per-request access log: one INFO line per request with method +
    // URI (span fields) and status + latency (event fields). Outermost
    // layer so rate-limit rejections are logged too. Operators silence
    // it with `--log-level info,tower_http=off`.
    let app = app.layer(
        TraceLayer::new_for_http()
            .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
            .on_response(
                DefaultOnResponse::new()
                    .level(Level::INFO)
                    .latency_unit(LatencyUnit::Millis),
            ),
    );

    app.with_state(state)
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    async fn body_string(resp: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body collect");
        String::from_utf8(bytes.to_vec()).expect("body utf8")
    }

    /// 429 rejections carry the canonical envelope, the bare JSON
    /// content type, and an RFC 9110 `Retry-After` header in seconds —
    /// the shape every HTTP retry helper consumes natively.
    #[tokio::test]
    async fn rate_limit_rejection_is_canonical_envelope_with_retry_after() {
        let resp = governor_error_response(GovernorError::TooManyRequests {
            wait_time: 3,
            headers: None,
        });
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("3"),
            "Retry-After must carry the bucket-refill wait in seconds"
        );
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );

        let body = body_string(resp).await;
        assert!(
            body.contains("\"error_type\":\"rate_limited\""),
            "envelope must carry the rate_limited error type: {body}"
        );
        assert!(
            body.contains("retry after 3s"),
            "error_msg must state the wait: {body}"
        );
        assert!(
            body.contains("\"response\":[]"),
            "envelope must carry the empty response array: {body}"
        );
    }

    #[tokio::test]
    async fn key_extraction_failure_maps_to_structured_500() {
        let resp = governor_error_response(GovernorError::UnableToExtractKey);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_string(resp).await;
        assert!(body.contains("\"error_type\":\"server_error\""), "{body}");
    }

    #[tokio::test]
    async fn other_governor_errors_keep_their_status_and_envelope() {
        let resp = governor_error_response(GovernorError::Other {
            code: StatusCode::SERVICE_UNAVAILABLE,
            msg: Some("limiter offline".to_string()),
            headers: None,
        });
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_string(resp).await;
        assert!(body.contains("limiter offline"), "{body}");
    }

    #[test]
    fn loopback_binds_are_detected() {
        assert!(is_loopback_bind("127.0.0.1"));
        assert!(is_loopback_bind("127.0.0.53"));
        assert!(is_loopback_bind("::1"));
    }

    #[test]
    fn non_loopback_binds_keep_the_limiter() {
        assert!(!is_loopback_bind("0.0.0.0"));
        assert!(!is_loopback_bind("192.168.2.21"));
        assert!(!is_loopback_bind("10.0.0.7"));
        assert!(!is_loopback_bind("::"));
    }

    /// Unparseable binds (hostnames) conservatively count as
    /// non-loopback: the limiter stays on when the reachability of the
    /// bind cannot be determined from the string.
    #[test]
    fn unparseable_binds_default_to_limited() {
        assert!(!is_loopback_bind("localhost"));
        assert!(!is_loopback_bind(""));
        assert!(!is_loopback_bind("example.internal"));
    }
}
