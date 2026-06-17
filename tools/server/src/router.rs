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
//! 3. `tower_governor::GovernorLayer` enforces a per-IP rate limit on
//!    every route — **only when the operator opts in**. The terminal this
//!    server replaces does no per-IP rate limiting, so the default must
//!    not either: with neither rate-limit env var set, the general
//!    governor layer is never attached, regardless of bind address. An
//!    operator exposing the server as a relay opts in by setting
//!    `THETADATADX_RATE_LIMIT_PER_SECOND` and/or
//!    `THETADATADX_RATE_LIMIT_BURST_SIZE`. The shutdown endpoint keeps an
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

/// Fallback general per-IP quota applied only when the operator opts into
/// rate limiting by setting one of the two rate-limit env vars but not the
/// other: 20 requests per second with a burst of 40. Tuned for backtest
/// fetches (bursty metadata pulls followed by idle time). When NEITHER env
/// var is set, no general governor is attached at all — the terminal this
/// server replaces does no per-IP rate limiting, so the default must not
/// either.
pub(crate) const GENERAL_PER_SECOND: u64 = 20;
pub(crate) const GENERAL_BURST_SIZE: u32 = 40;

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
/// natively. Shared by the REST shutdown limiter, the opt-in general
/// per-IP limiter, and the WS upgrade limiter.
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

/// Opt-in per-IP rate-limit setting: `(per_second, burst_size)`.
///
/// The general `GovernorLayer` (on both the HTTP general routes and the WS
/// upgrade) is attached only when this is `Some`. It is `Some` only when the
/// operator sets at least one of the two rate-limit env vars; otherwise the
/// server runs with no general per-IP limit, matching the terminal it
/// replaces.
pub type RateLimit = (u64, u32);

/// Environment variable names for the opt-in per-IP rate limit.
pub(crate) const ENV_RATE_LIMIT_PER_SECOND: &str = "THETADATADX_RATE_LIMIT_PER_SECOND";
pub(crate) const ENV_RATE_LIMIT_BURST_SIZE: &str = "THETADATADX_RATE_LIMIT_BURST_SIZE";

/// Resolve the opt-in per-IP rate limit from the process environment.
///
/// The terminal this server replaces does no per-IP rate limiting, so the
/// default is OFF: with NEITHER `THETADATADX_RATE_LIMIT_PER_SECOND` nor
/// `THETADATADX_RATE_LIMIT_BURST_SIZE` set, this returns `None` and no
/// general governor is attached anywhere. Operators exposing the server as
/// a relay opt in by setting one or both.
///
/// When at least one is set the limiter is enabled; a value that is absent
/// or unparseable falls back to the documented default constant for that
/// field ([`GENERAL_PER_SECOND`] / [`GENERAL_BURST_SIZE`]), so a single env
/// var is enough to turn the limiter on with sane companion settings.
pub fn resolve_rate_limit() -> Option<RateLimit> {
    resolve_rate_limit_from(
        std::env::var(ENV_RATE_LIMIT_PER_SECOND).ok().as_deref(),
        std::env::var(ENV_RATE_LIMIT_BURST_SIZE).ok().as_deref(),
    )
}

/// Pure core of [`resolve_rate_limit`], split out so the opt-in / partial-set
/// semantics can be tested without mutating process-global env state.
pub(crate) fn resolve_rate_limit_from(
    per_second: Option<&str>,
    burst_size: Option<&str>,
) -> Option<RateLimit> {
    // Default-OFF: neither knob present means no governor anywhere.
    if per_second.is_none() && burst_size.is_none() {
        return None;
    }
    let per_second = per_second
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(GENERAL_PER_SECOND);
    let burst_size = burst_size
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(GENERAL_BURST_SIZE);
    Some((per_second, burst_size))
}

/// Build the full REST API router with routes dynamically generated from
/// the endpoint registry plus hand-written system routes.
///
/// `rate_limit` mounts the per-IP general rate limiter only when `Some`
/// (see [`resolve_rate_limit`]); the terminal this server replaces does no
/// per-IP rate limiting, so the default `None` attaches no general governor.
/// The tighter shutdown-route limiter is mounted unconditionally.
///
/// Default port: 25503 (matching ThetaData v3 terminal).
pub fn build(state: AppState, rate_limit: Option<RateLimit>) -> Router {
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

    // Global per-IP governor (outermost rate limit), an opt-in DoS guard.
    // The terminal this server replaces does no per-IP rate limiting, so
    // the default attaches no general governor; an operator exposing the
    // server as a relay opts in via the rate-limit env vars. The shutdown
    // route's tighter governor above stays active on every bind regardless.
    if let Some((per_second, burst_size)) = rate_limit {
        let global_governor = Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(PeerIpKeyExtractor)
                .per_second(per_second)
                .burst_size(burst_size)
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

    use std::net::SocketAddr;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    async fn body_string(resp: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body collect");
        String::from_utf8(bytes.to_vec()).expect("body utf8")
    }

    /// Build a minimal router that mounts the general governor exactly as
    /// `build()` does — same `GovernorConfigBuilder`, `PeerIpKeyExtractor`,
    /// and `governor_error_response` — over a trivial `200 OK` route, so the
    /// opt-in / default-off wiring can be exercised at the wire level without
    /// constructing a live `AppState`.
    fn governor_probe_router(rate_limit: Option<RateLimit>) -> Router {
        let app = Router::new().route("/probe", get(|| async { StatusCode::OK }));
        let Some((per_second, burst_size)) = rate_limit else {
            return app;
        };
        let governor = Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(PeerIpKeyExtractor)
                .per_second(per_second)
                .burst_size(burst_size)
                .finish()
                .expect("general governor config invariants hold at build time"),
        );
        app.layer(GovernorLayer::new(governor).error_handler(governor_error_response))
    }

    /// Fire one `/probe` request carrying a peer `ConnectInfo` so the
    /// `PeerIpKeyExtractor` resolves a key, returning the response status.
    async fn probe_once(app: &Router, peer: SocketAddr) -> StatusCode {
        let mut req = Request::builder()
            .uri("/probe")
            .body(Body::empty())
            .expect("request builds");
        req.extensions_mut().insert(ConnectInfo(peer));
        app.clone()
            .oneshot(req)
            .await
            .expect("router responds")
            .status()
    }

    /// Default-OFF: with no rate limit resolved, a burst far exceeding the
    /// old 20 rps / burst 40 ceiling is never `429`'d — the server imposes
    /// no per-IP limit, matching the terminal it replaces.
    #[tokio::test]
    async fn no_governor_when_rate_limit_unset_never_429s_a_burst() {
        let app = governor_probe_router(None);
        let peer: SocketAddr = "203.0.113.7:9000".parse().unwrap();
        for _ in 0..100 {
            assert_eq!(
                probe_once(&app, peer).await,
                StatusCode::OK,
                "default-off server must not rate-limit any request"
            );
        }
    }

    /// Opt-in: with a tuned ceiling, traffic up to the burst passes and the
    /// next same-IP request in the same instant is rejected with `429`.
    #[tokio::test]
    async fn governor_enforces_tuned_ceiling_when_rate_limit_set() {
        // burst_size 3 with a low refill: the 4th immediate request from the
        // same peer must be shed.
        let app = governor_probe_router(Some((1, 3)));
        let peer: SocketAddr = "203.0.113.8:9000".parse().unwrap();

        for n in 0..3 {
            assert_eq!(
                probe_once(&app, peer).await,
                StatusCode::OK,
                "request {n} within the burst must pass"
            );
        }
        assert_eq!(
            probe_once(&app, peer).await,
            StatusCode::TOO_MANY_REQUESTS,
            "request past the tuned burst must be rate-limited"
        );
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

    /// Default-OFF: with neither env var set the limiter resolves to
    /// `None`, so no general governor is attached on any bind — the
    /// terminal this server replaces imposes no per-IP rate limit.
    #[test]
    fn rate_limit_is_off_when_neither_env_var_is_set() {
        assert_eq!(resolve_rate_limit_from(None, None), None);
    }

    /// Both env vars set: the operator's tuned pair is used verbatim.
    #[test]
    fn rate_limit_uses_both_operator_values_when_set() {
        assert_eq!(resolve_rate_limit_from(Some("5"), Some("9")), Some((5, 9)));
    }

    /// Only one of the pair set still turns the limiter ON; the absent
    /// field falls back to its documented default constant.
    #[test]
    fn rate_limit_partial_set_falls_back_to_defaults() {
        assert_eq!(
            resolve_rate_limit_from(Some("7"), None),
            Some((7, GENERAL_BURST_SIZE))
        );
        assert_eq!(
            resolve_rate_limit_from(None, Some("11")),
            Some((GENERAL_PER_SECOND, 11))
        );
    }

    /// A present-but-unparseable value falls back to its default rather
    /// than disabling the limiter the operator asked for.
    #[test]
    fn rate_limit_unparseable_value_falls_back_to_default() {
        assert_eq!(
            resolve_rate_limit_from(Some("not-a-number"), Some("9")),
            Some((GENERAL_PER_SECOND, 9))
        );
        assert_eq!(
            resolve_rate_limit_from(Some("5"), Some("oops")),
            Some((5, GENERAL_BURST_SIZE))
        );
    }
}
