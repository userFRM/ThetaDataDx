//! Generic endpoint handler and shared-runtime dispatch for all registry endpoints.
//!
//! A single handler function receives endpoint metadata via closure capture,
//! validates query params, invokes the shared endpoint runtime in
//! `thetadatadx`, and returns the Java terminal JSON envelope (or CSV when
//! `format=csv`).

use std::collections::HashMap;

use axum::extract::{Query, State};
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

fn build_endpoint_args(
    ep: &EndpointMeta,
    params: &HashMap<String, String>,
) -> Result<EndpointArgs, EndpointError> {
    // Cap the TOTAL number of query parameters before iterating. Per-param
    // length caps alone don't defend against `?a=1&b=2&...` with 10 000
    // unique 60-byte keys: each one passes the 64-byte validator yet the
    // `HashMap` backing store rehashes up into the megabyte range. 32 is
    // strictly above the widest endpoint in the registry.
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
/// 1. Validates required query params against `EndpointMeta.params`.
/// 2. Invokes the shared endpoint runtime.
/// 3. Returns JSON envelope or CSV depending on `format=csv`.
pub async fn generic(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
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
}
