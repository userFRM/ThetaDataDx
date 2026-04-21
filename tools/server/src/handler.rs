//! Generic endpoint handler and shared-runtime dispatch for all registry endpoints.
//!
//! A single handler function receives endpoint metadata via closure capture,
//! validates query params, invokes the shared endpoint runtime in
//! `thetadatadx`, and returns the Java terminal JSON envelope (or CSV when
//! `format=csv`).

use std::collections::HashMap;

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use sonic_rs::prelude::*;

use thetadatadx::endpoint::{invoke_endpoint, EndpointArgs, EndpointError};
use thetadatadx::registry::EndpointMeta;

use crate::format;
use crate::state::AppState;
use crate::validation;

// ---------------------------------------------------------------------------
//  Helpers
// ---------------------------------------------------------------------------

/// Build a JSON error response with the Java terminal error envelope format.
fn error_response(status: StatusCode, error_type: &str, msg: &str) -> Response {
    let body = format::error_envelope(error_type, msg);
    let json_bytes = sonic_rs::to_string(&body).unwrap_or_default();
    (
        status,
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        )],
        json_bytes,
    )
        .into_response()
}

/// Serialize a `sonic_rs::Value` to an axum JSON response body.
fn json_response(val: &sonic_rs::Value) -> Response {
    let json_bytes = sonic_rs::to_string(val).unwrap_or_default();
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        )],
        json_bytes,
    )
        .into_response()
}

/// Max query parameter count per request. The widest legitimate endpoint in
/// the registry takes ~10 params (`option_history_trade_quote` + format /
/// pagination knobs); 32 leaves generous headroom without letting a caller
/// allocate ~MB of `HashMap` slack via `?a=1&b=2&...` with thousands of
/// unique 60-byte keys that each pass the 64-byte per-param cap.
pub(crate) const MAX_QUERY_PARAMS: usize = 32;

// ---------------------------------------------------------------------------
//  BoundedQuery — count params BEFORE allocating the HashMap
// ---------------------------------------------------------------------------

/// Axum extractor that parses the raw URI query string into a
/// `HashMap<String, String>` while enforcing [`MAX_QUERY_PARAMS`] **during**
/// parsing — not after `serde_urlencoded` has already populated the full
/// HashMap.
///
/// # Why a custom extractor
///
/// `axum::extract::Query<HashMap<String, String>>` defers to
/// `serde_urlencoded::from_str`, which parses EVERY `&`-delimited pair into
/// the `HashMap` before returning. A subsequent `params.len() > 32` check
/// runs after the allocation and rehashing; it surfaces a 400 but does NOT
/// bound the memory footprint during parse, which defeats the memory-DoS
/// goal the cap was introduced to achieve.
///
/// `BoundedQuery` counts `&`-delimited pairs on the raw query string
/// **before** invoking `serde_urlencoded`. Any request with more than 32
/// pairs is rejected with 400 Bad Request the moment the 33rd `&` is
/// counted — no per-key `String` allocation, no HashMap rehashing.
///
/// Memory bound during parse: at most `MAX_QUERY_PARAMS` capacity on the
/// HashMap, independent of how long the attacker's query string was. The
/// body / URI limits stay in place via axum's `DefaultBodyLimit` and the
/// URI length limit in `hyper`.
#[derive(Debug)]
pub(crate) struct BoundedQuery<const N: usize>(pub HashMap<String, String>);

/// Error surfaced when a client sends more than `N` query parameters.
///
/// Rendered as `400 Bad Request` by `IntoResponse`. Keeping a dedicated
/// error type (rather than reusing `EndpointError`) so the rejection
/// happens at the extractor boundary — the generic handler never runs
/// when the cap trips, so no work is done with the over-limit input.
#[derive(Debug)]
pub(crate) struct BoundedQueryError {
    status: StatusCode,
    message: String,
}

impl IntoResponse for BoundedQueryError {
    fn into_response(self) -> Response {
        error_response(self.status, "bad_request", &self.message)
    }
}

impl<const N: usize, S> FromRequestParts<S> for BoundedQuery<N>
where
    S: Send + Sync,
{
    type Rejection = BoundedQueryError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or("");

        // Count `&`-delimited pairs. An empty query produces zero pairs
        // (we don't want `?` alone to count as 1). Iterate without
        // allocating anything — pure byte-slice scan — so we can bail
        // BEFORE `serde_urlencoded` allocates any `String`.
        if !query.is_empty() {
            let pair_count = query.split('&').filter(|s| !s.is_empty()).count();
            if pair_count > N {
                return Err(BoundedQueryError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("request has {pair_count} query parameters; max is {N}"),
                });
            }
        }

        // Now it's safe to parse into a HashMap — the pair count is at
        // most N, so the HashMap capacity is bounded by the cap.
        let params: HashMap<String, String> =
            serde_urlencoded::from_str(query).map_err(|e| BoundedQueryError {
                status: StatusCode::BAD_REQUEST,
                message: format!("invalid query string: {e}"),
            })?;

        Ok(BoundedQuery(params))
    }
}

fn build_endpoint_args(
    ep: &EndpointMeta,
    params: &HashMap<String, String>,
) -> Result<EndpointArgs, EndpointError> {
    // Defensive belt-and-braces: the `BoundedQuery` extractor already
    // enforced the cap during URL parsing, but keeping this check means a
    // future caller that bypasses the extractor (programmatic test, direct
    // `build_endpoint_args` call) still gets the same rejection.
    if params.len() > MAX_QUERY_PARAMS {
        return Err(EndpointError::InvalidParams(format!(
            "request has {} query parameters; max is {MAX_QUERY_PARAMS}",
            params.len()
        )));
    }

    // Length-cap every incoming query-param BEFORE parsing. This bounds
    // memory-DoS on malicious inputs (`?root=<1 MB string>`) at the edge
    // and keeps format errors below (`?right=garbage`) surfaced as 400
    // instead of 500.
    //
    // Length checks are server-side and orthogonal to the semantic
    // validators in `thetadatadx::validate`, which enforce *format*
    // (digit-count, right vocabulary, strike wildcard) but accept
    // arbitrarily long strings.
    //
    // The generic fallback (`MAX_GENERIC_LEN`) also blocks unknown params
    // like `?format=<megabytes>` that don't appear in `ep.params` but
    // could still be passed through `HashMap<String, String>`.
    for (name, raw) in params {
        validation::validate_query_param(name, raw)?;
    }

    let mut args = EndpointArgs::new();
    for param in ep.params {
        match params.get(param.name) {
            Some(raw) => args.insert_raw(param.name, param.param_type, raw)?,
            None if param.required => {
                return Err(EndpointError::InvalidParams(format!(
                    "missing required parameter: '{}' ({})",
                    param.name, param.description
                )));
            }
            None => {}
        }
    }
    Ok(args)
}

fn endpoint_error_response(ep: &EndpointMeta, error: EndpointError) -> Response {
    match error {
        EndpointError::InvalidParams(message) => {
            error_response(StatusCode::BAD_REQUEST, "bad_request", &message)
        }
        EndpointError::UnknownEndpoint(message) => {
            error_response(StatusCode::NOT_FOUND, "not_found", &message)
        }
        EndpointError::Server(error) => {
            tracing::warn!(endpoint = ep.name, error = %error, "request failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &error.to_string(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
//  Generic endpoint handler
// ---------------------------------------------------------------------------

/// Generic handler invoked for all registry endpoints.
///
/// 1. The [`BoundedQuery`] extractor enforces [`MAX_QUERY_PARAMS`] DURING
///    URL parse so `?a=1&b=2&...` flood attacks can't force a multi-MB
///    HashMap allocation before the cap trips.
/// 2. Validates required query params against `EndpointMeta.params`.
/// 3. Invokes the shared endpoint runtime.
/// 4. Returns JSON envelope or CSV depending on `format=csv`.
pub async fn generic(
    State(state): State<AppState>,
    BoundedQuery(params): BoundedQuery<MAX_QUERY_PARAMS>,
    ep: &EndpointMeta,
) -> Response {
    let use_csv = params
        .get("format")
        .is_some_and(|v| v.eq_ignore_ascii_case("csv"));

    let args = match build_endpoint_args(ep, &params) {
        Ok(args) => args,
        Err(error) => return endpoint_error_response(ep, error),
    };

    let output = match invoke_endpoint(state.tdx(), ep.name, &args).await {
        Ok(output) => output,
        Err(error) => return endpoint_error_response(ep, error),
    };

    let json_val = format::output_envelope(&output);
    if use_csv {
        if let Some(arr) = json_val
            .get("response")
            .and_then(|v: &sonic_rs::Value| v.as_array())
        {
            let items: Vec<sonic_rs::Value> = arr.iter().cloned().collect();
            if let Some(csv) = format::json_to_csv(&items) {
                return (
                    StatusCode::OK,
                    [("content-type", "text/csv; charset=utf-8")],
                    csv,
                )
                    .into_response();
            }
        }
        return (
            StatusCode::OK,
            [("content-type", "text/csv; charset=utf-8")],
            String::new(),
        )
            .into_response();
    }

    json_response(&json_val)
}

// ---------------------------------------------------------------------------
//  System endpoints
// ---------------------------------------------------------------------------

/// GET /v3/system/status -- matches Java terminal system status.
pub async fn system_status(State(state): State<AppState>) -> Response {
    let body = sonic_rs::json!({
        "status": state.mdds_status(),
        "version": env!("CARGO_PKG_VERSION")
    });
    json_response(&body)
}

/// GET /v3/system/mdds/status
pub async fn system_mdds_status(State(state): State<AppState>) -> Response {
    let body = format::ok_envelope(vec![sonic_rs::Value::from(state.mdds_status())]);
    json_response(&body)
}

/// GET /v3/system/fpss/status
pub async fn system_fpss_status(State(state): State<AppState>) -> Response {
    let body = sonic_rs::json!({
        "status": state.fpss_status(),
        "version": env!("CARGO_PKG_VERSION")
    });
    json_response(&body)
}

/// POST /v3/system/shutdown -- requires `X-Shutdown-Token` header.
/// Changed from GET in #377 review: shutdown mutates server state, so it
/// belongs on an idempotent non-cacheable verb. GET would be cached /
/// prefetched / CSRF-triggered; POST requires an explicit, intentional
/// client action.
pub async fn system_shutdown(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = headers
        .get("X-Shutdown-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !state.validate_shutdown_token(token) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid or missing X-Shutdown-Token header",
        );
    }

    tracing::info!("shutdown requested via REST API with valid token");
    state.shutdown();
    let body = format::ok_envelope(vec![sonic_rs::Value::from("OK")]);
    json_response(&body)
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use thetadatadx::registry::ENDPOINTS;

    /// Grab any registered endpoint as a stand-in for the per-endpoint
    /// arg-building path. The param cap lives outside `ep.params` so any
    /// endpoint exercises the same code path.
    fn any_endpoint() -> &'static EndpointMeta {
        ENDPOINTS
            .first()
            .expect("registry must have at least one endpoint")
    }

    #[test]
    fn build_endpoint_args_rejects_too_many_params() {
        let ep = any_endpoint();
        let mut params: HashMap<String, String> = HashMap::with_capacity(MAX_QUERY_PARAMS + 1);
        for i in 0..=MAX_QUERY_PARAMS {
            params.insert(format!("k{i}"), format!("v{i}"));
        }
        assert_eq!(params.len(), MAX_QUERY_PARAMS + 1);

        let err = build_endpoint_args(ep, &params)
            .expect_err("over-limit query-param count must be rejected");
        match err {
            EndpointError::InvalidParams(msg) => {
                assert!(
                    msg.contains("query parameters"),
                    "error message should mention query parameters: {msg}"
                );
                assert!(
                    msg.contains(&MAX_QUERY_PARAMS.to_string()),
                    "error message should mention the cap: {msg}"
                );
            }
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn build_endpoint_args_accepts_limit_boundary() {
        // At exactly MAX_QUERY_PARAMS the count check must not fire; any
        // failure must come from downstream validators (missing required
        // params etc.), NOT from the count cap.
        let ep = any_endpoint();
        let mut params: HashMap<String, String> = HashMap::with_capacity(MAX_QUERY_PARAMS);
        for i in 0..MAX_QUERY_PARAMS {
            params.insert(format!("k{i}"), format!("v{i}"));
        }
        // We don't assert Ok here -- missing required params still fail --
        // but the error must never be the count-cap message.
        if let Err(EndpointError::InvalidParams(msg)) = build_endpoint_args(ep, &params) {
            assert!(
                !msg.contains("query parameters; max is"),
                "count cap tripped at the boundary: {msg}"
            );
        }
    }

    // -----------------------------------------------------------------------
    //  BoundedQuery — cap enforced DURING parse, not after HashMap alloc
    // -----------------------------------------------------------------------

    async fn run_bounded_query<const N: usize>(
        query: &str,
    ) -> Result<HashMap<String, String>, BoundedQueryError> {
        let uri = format!("http://example.test/v3/foo?{query}");
        let req = Request::builder()
            .uri(uri)
            .body(())
            .expect("request build must succeed");
        let (mut parts, _body) = req.into_parts();
        BoundedQuery::<N>::from_request_parts(&mut parts, &())
            .await
            .map(|BoundedQuery(p)| p)
    }

    #[tokio::test]
    async fn bounded_query_rejects_over_limit() {
        // 33 distinct key=value pairs with N=32 must fail BEFORE axum
        // would allocate the HashMap from serde_urlencoded. This mirrors
        // the memory-DoS attack shape (`?a=1&b=2&...&z=26&aa=27&...`).
        let mut qs = String::new();
        for i in 0..(MAX_QUERY_PARAMS + 1) {
            if i > 0 {
                qs.push('&');
            }
            qs.push_str(&format!("k{i}=v{i}"));
        }

        let err = run_bounded_query::<{ MAX_QUERY_PARAMS }>(&qs)
            .await
            .expect_err("33 params must be rejected");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(
            err.message.contains("query parameters"),
            "rejection message must name the failure: {}",
            err.message
        );
        assert!(
            err.message.contains(&MAX_QUERY_PARAMS.to_string()),
            "rejection message must name the cap: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn bounded_query_accepts_exactly_limit() {
        // 32 distinct pairs must succeed at N=32; the boundary value is
        // legal, not an off-by-one rejection.
        let mut qs = String::new();
        for i in 0..MAX_QUERY_PARAMS {
            if i > 0 {
                qs.push('&');
            }
            qs.push_str(&format!("k{i}=v{i}"));
        }

        let params = run_bounded_query::<{ MAX_QUERY_PARAMS }>(&qs)
            .await
            .expect("32 params is at the cap, not over it");
        assert_eq!(params.len(), MAX_QUERY_PARAMS);
    }

    #[tokio::test]
    async fn bounded_query_accepts_empty_query() {
        // No query string, no pairs. Extractor must return an empty map
        // instead of failing — plenty of registry endpoints take zero
        // required params from the URL (health probes, `open_today`).
        let params = run_bounded_query::<{ MAX_QUERY_PARAMS }>("")
            .await
            .expect("empty query must parse to empty map");
        assert!(params.is_empty());
    }

    #[tokio::test]
    async fn bounded_query_parses_normal_request() {
        // Realistic 4-param request: must parse into the HashMap exactly.
        let params = run_bounded_query::<{ MAX_QUERY_PARAMS }>(
            "symbol=AAPL&start_date=20240101&end_date=20240201&format=json",
        )
        .await
        .expect("normal query must parse");
        assert_eq!(params.get("symbol").map(String::as_str), Some("AAPL"));
        assert_eq!(
            params.get("start_date").map(String::as_str),
            Some("20240101")
        );
        assert_eq!(params.get("end_date").map(String::as_str), Some("20240201"));
        assert_eq!(params.get("format").map(String::as_str), Some("json"));
    }
}
