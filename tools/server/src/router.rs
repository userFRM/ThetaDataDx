//! Route generation from the endpoint registry.
//!
//! Iterates `ENDPOINTS` at startup and registers a handler for every
//! registry endpoint, plus system routes. Each endpoint is mapped to a REST
//! path following the ThetaData v3 API convention. Paths are generated in the
//! core registry so the REST server does not re-derive them heuristically.
//!
//! # Hardening layers
//!
//! `build()` composes two generic admission layers on top of the registry
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
//!
//! The terminal this server replaces does no per-IP rate limiting, and
//! neither does this server — real request limits are enforced upstream by
//! the data service. The system routes mirror the terminal 1:1, including its
//! unauthenticated `GET /v3/terminal/shutdown`. The server binds to `0.0.0.0`
//! by default (all interfaces, matching the JVM terminal it replaces); pass
//! `--bind 127.0.0.1` for loopback-only exposure.

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::get;
use axum::Router;
use tower::limit::ConcurrencyLimitLayer;
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
pub(crate) const GLOBAL_CONCURRENCY_LIMIT: usize = 256;

/// Max request body size. 64 KB comfortably covers any realistic query
/// string + headers for this API; anything larger is DoS or a broken
/// client.
pub(crate) const BODY_LIMIT_BYTES: usize = 64 * 1024;

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

    // v3 path-form routes that the core registry does not generate, mounted
    // at the server level so the core codegen (and its drift guard) stays
    // untouched. Each route dispatches to the same registry endpoint / SDK
    // call as its query-form sibling.
    app = register_v3_path_routes(app);

    // Terminal system routes, mirrored 1:1 from the JVM terminal this server
    // replaces. The terminal serves exactly three unauthenticated GET routes
    // under `/v3/terminal/`: `shutdown` (kills the process, returns the plain
    // text `OK`), plus one-word channel-health probes for the FPSS (streaming)
    // and MDDS (market-data) transports. The transport codenames are the vendor
    // terminal's own public wire paths, not client-facing prose. Bodies are the
    // terminal's bare `text/plain` shape so operator tooling that scrapes the
    // terminal's mgmt surface works unchanged.
    app = app
        .route("/v3/terminal/shutdown", get(handler::terminal_shutdown))
        .route(
            "/v3/terminal/fpss/status",
            get(handler::terminal_streaming_status),
        )
        .route(
            "/v3/terminal/mdds/status",
            get(handler::terminal_market_data_status),
        );

    // Flat-file routes — whole-universe daily blobs over
    // HTTP. Not a WebSocket subscription stream; flat files are batch
    // downloads and the bytes ride a streaming response body so the
    // server doesn't pin multi-hundred-MB blobs in RAM.
    app = crate::flatfile_routes::add_flatfile_routes(app);

    // `.layer(X).layer(Y)` in axum/tower makes Y wrap X (outer wraps inner),
    // so the LAST `.layer(...)` call is the outermost request wrapper.
    let app = app
        .layer(ConcurrencyLimitLayer::new(GLOBAL_CONCURRENCY_LIMIT))
        .layer(DefaultBodyLimit::max(BODY_LIMIT_BYTES));

    // Per-request access log: one INFO line per request with method +
    // URI (span fields) and status + latency (event fields). Outermost
    // layer so it captures every response. Operators silence
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

/// Mount the v3 REST path forms the core registry does not generate.
///
/// Two families of route live here rather than in the core's
/// `endpoint_surface.toml` so the core codegen and its drift guard stay
/// untouched (server-level overrides are the cheaper, lower-risk change):
///
/// 1. `request_type` **path-segment** routes (`C6`). The terminal serves
///    `request_type` as a path segment on three list routes; the registry
///    models it as a query param, so the terminal's path form would 404.
///    Each route captures the segment and folds it into the query map via
///    [`handler::list_by_request_type`], dispatching to the same registry
///    endpoint as the query form.
/// 2. **Renamed** routes (`H1`). The terminal's `calendar/today`,
///    `calendar/year_holidays`, and `interest_rate/history/eod` map to the
///    registry's `calendar/open_today`, `calendar/year`, and
///    `rate/history/eod`. The registry already mounts the latter (legacy
///    aliases stay live); these add the v3 names pointing at the same
///    handlers / SDK calls.
///
/// A renamed route resolves its registry endpoint by name at build time;
/// the `expect` documents that these three endpoints are part of the
/// compiled-in registry and a miss is a build-time invariant violation, not
/// a runtime condition.
fn register_v3_path_routes(app: Router<AppState>) -> Router<AppState> {
    // --- request_type path-segment routes (C6) ---
    // Each closure pins the registry endpoint name its path form dispatches
    // to; `list_by_request_type` injects the captured segment as the
    // `request_type` query param before delegating to the generic pipeline.
    let app = app
        .route(
            "/v3/stock/list/dates/{request_type}",
            get(
                |s: axum::extract::State<AppState>,
                 p: axum::extract::Path<String>,
                 q: handler::BoundedQuery<{ handler::MAX_QUERY_PARAMS }>| async move {
                    handler::list_by_request_type(s, p, q, "stock_list_dates").await
                },
            ),
        )
        .route(
            "/v3/option/list/dates/{request_type}",
            get(
                |s: axum::extract::State<AppState>,
                 p: axum::extract::Path<String>,
                 q: handler::BoundedQuery<{ handler::MAX_QUERY_PARAMS }>| async move {
                    handler::list_by_request_type(s, p, q, "option_list_dates").await
                },
            ),
        )
        .route(
            "/v3/option/list/contracts/{request_type}",
            get(
                |s: axum::extract::State<AppState>,
                 p: axum::extract::Path<String>,
                 q: handler::BoundedQuery<{ handler::MAX_QUERY_PARAMS }>| async move {
                    handler::list_by_request_type(s, p, q, "option_list_contracts").await
                },
            ),
        );

    // --- renamed routes (H1) ---
    // The v3 path name routes to the same registry endpoint the legacy path
    // already serves. The legacy paths stay mounted by the registry loop, so
    // both names work.
    app.route(
        "/v3/calendar/today",
        get(registry_route_handler("calendar_open_today")),
    )
    .route(
        "/v3/calendar/year_holidays",
        get(registry_route_handler("calendar_year")),
    )
    .route(
        "/v3/interest_rate/history/eod",
        get(registry_route_handler("interest_rate_history_eod")),
    )
}

/// Build a GET handler closure that dispatches to the registry endpoint
/// named `endpoint_name` through the shared [`handler::generic`] pipeline.
///
/// Used to mount a v3 path alias at the server level for an endpoint the
/// core registry already exposes under a different path. The endpoint is
/// resolved against the compiled-in registry once at build time; these names
/// are part of the registry, so a miss is a build-time invariant violation
/// surfaced via `expect`, never a runtime branch.
fn registry_route_handler(
    endpoint_name: &'static str,
) -> impl Fn(
    axum::extract::State<AppState>,
    handler::BoundedQuery<{ handler::MAX_QUERY_PARAMS }>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = axum::response::Response> + Send>>
       + Clone {
    let ep: &'static EndpointMeta = thetadatadx::find(endpoint_name)
        .expect("v3 alias route is wired to an endpoint compiled into the registry");
    let ep_shared = Arc::new(ep);
    move |s: axum::extract::State<AppState>,
          q: handler::BoundedQuery<{ handler::MAX_QUERY_PARAMS }>| {
        let ep = Arc::clone(&ep_shared);
        Box::pin(async move { handler::generic(s, q, &ep).await })
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    // -----------------------------------------------------------------------
    //  v3 path-form routes (C6 / H1) — registry wiring + matchit coexistence
    // -----------------------------------------------------------------------

    /// Every registry endpoint name referenced by a server-level v3 route
    /// (the `{request_type}` path routes and the renamed-route aliases) must
    /// resolve in the compiled-in registry. `register_v3_path_routes` /
    /// `registry_route_handler` panic via `expect` on a typo'd or removed
    /// name; this surfaces that as a fast unit-test failure instead of a
    /// boot-time panic.
    #[test]
    fn v3_path_routes_reference_real_registry_endpoints() {
        for name in [
            // request_type path-segment routes (C6)
            "stock_list_dates",
            "option_list_dates",
            "option_list_contracts",
            // renamed-route aliases (H1)
            "calendar_open_today",
            "calendar_year",
            "interest_rate_history_eod",
        ] {
            assert!(
                thetadatadx::find(name).is_some(),
                "v3 route is wired to `{name}`, which is missing from the registry"
            );
        }
    }

    /// `registry_route_handler` resolves its endpoint at construction; this
    /// proves the three H1 alias names build a handler without panicking.
    #[test]
    fn registry_route_handler_builds_for_renamed_aliases() {
        let _ = registry_route_handler("calendar_open_today");
        let _ = registry_route_handler("calendar_year");
        let _ = registry_route_handler("interest_rate_history_eod");
    }

    /// The v3 wildcard flat-file route (`/v3/{sec_type}/flat_file/...`) and
    /// the `{request_type}` path-segment routes must coexist with the
    /// static `/v3/{stock,option,index}/...` registry routes WITHOUT a
    /// matchit conflict panic, AND must not shadow the static routes. This
    /// builds a bare `Router` carrying every real registry `rest_path` plus
    /// the new server-level path patterns, then drives requests to confirm
    /// the precedence axum/matchit actually applies.
    #[tokio::test]
    async fn v3_path_routes_coexist_with_registry_routes_without_shadowing() {
        async fn registry_marker() -> &'static str {
            "REGISTRY"
        }
        async fn flat_marker() -> &'static str {
            "FLAT"
        }
        async fn rt_marker(axum::extract::Path(rt): axum::extract::Path<String>) -> String {
            format!("RT:{rt}")
        }

        // Mount every real registry path so the test mirrors the production
        // routing table's static segments at positions 2 and 3.
        let mut app: Router<()> = Router::new();
        for ep in ENDPOINTS {
            app = app.route(ep.rest_path, get(registry_marker));
        }
        // The new server-level path forms (same patterns as the production
        // wiring, minus the live handlers).
        app = app
            .route("/v3/{sec_type}/flat_file/{req_type}", get(flat_marker))
            .route("/v3/stock/list/dates/{request_type}", get(rt_marker))
            .route("/v3/option/list/dates/{request_type}", get(rt_marker))
            .route("/v3/option/list/contracts/{request_type}", get(rt_marker));

        async fn get_body(app: &Router<()>, uri: &str) -> (StatusCode, String) {
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .expect("router responds");
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .expect("body collect");
            (status, String::from_utf8(bytes.to_vec()).unwrap())
        }

        // Flat-file v3 path resolves to the wildcard route.
        let (st, body) = get_body(&app, "/v3/option/flat_file/trade_quote").await;
        assert_eq!((st, body.as_str()), (StatusCode::OK, "FLAT"));

        // A static registry data route is NOT shadowed by the `{sec_type}`
        // wildcard.
        let (st, body) = get_body(&app, "/v3/option/snapshot/ohlc").await;
        assert_eq!((st, body.as_str()), (StatusCode::OK, "REGISTRY"));

        // The query-form list route still hits its registry handler...
        let (st, body) = get_body(&app, "/v3/option/list/contracts").await;
        assert_eq!((st, body.as_str()), (StatusCode::OK, "REGISTRY"));

        // ...while the path-segment form captures `request_type`.
        let (st, body) = get_body(&app, "/v3/option/list/contracts/trade").await;
        assert_eq!((st, body.as_str()), (StatusCode::OK, "RT:trade"));

        let (st, body) = get_body(&app, "/v3/stock/list/dates/quote").await;
        assert_eq!((st, body.as_str()), (StatusCode::OK, "RT:quote"));
    }
}
