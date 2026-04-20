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

fn build_endpoint_args(
    ep: &EndpointMeta,
    params: &HashMap<String, String>,
) -> Result<EndpointArgs, EndpointError> {
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

/// GET /v3/system/shutdown -- requires `X-Shutdown-Token` header.
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
