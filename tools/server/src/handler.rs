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
use thetadatadx::EndpointMeta;

use crate::format;
use crate::state::AppState;
use crate::validation;

// ---------------------------------------------------------------------------
//  Helpers
// ---------------------------------------------------------------------------

/// Content type for JSON envelope responses.
///
/// Deliberately the bare media type without a `charset` parameter: RFC 8259
/// specifies UTF-8 as the only encoding for `application/json`, and the Java
/// terminal emits the bare form. Strict HTTP clients that exact-match the
/// content-type string during negotiation break on a `; charset=utf-8`
/// suffix, so the suffix must never come back.
pub(crate) const JSON_CONTENT_TYPE: &str = "application/json";

/// Build a JSON error response with the Java terminal error envelope format.
///
/// The envelope is hand-built from string + array primitives, so
/// serialisation cannot legitimately fail. If it does anyway, fall back to a
/// minimal hard-coded envelope rather than an empty body so callers always
/// see structured JSON they can parse.
///
/// `pub(crate)` so the rate-limit rejection path in `router` emits the
/// same canonical envelope as every other error.
pub(crate) fn error_response(status: StatusCode, error_type: &str, msg: &str) -> Response {
    let mut body = format::error_envelope(error_type, msg);
    let json_bytes =
        thetadatadx::json_canon::canonicalize_and_serialize(&mut body).unwrap_or_else(|err| {
            tracing::error!(
                error = %err,
                "error envelope failed to serialise; emitting minimal fallback"
            );
            format!(
                "{{\"header\":{{\"error_type\":\"serialization_error\",\
             \"error_msg\":\"failed to serialise error envelope: {err}\"}},\
             \"response\":[]}}"
            )
        });
    (
        status,
        [(axum::http::header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
        json_bytes,
    )
        .into_response()
}

/// Serialize a `sonic_rs::Value` to an axum JSON response body.
///
/// The value tree is canonicalised in place (non-finite f64 -> JSON `null`)
/// before serialisation so cross-language SDK agreement holds. If
/// serialisation still fails — a logic bug, not a data bug — surface it as a
/// structured `500` carrying the underlying error message rather than an
/// empty `200 OK` body.
fn json_response(val: &mut sonic_rs::Value) -> Response {
    match thetadatadx::json_canon::canonicalize_and_serialize(val) {
        Ok(json_bytes) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
            json_bytes,
        )
            .into_response(),
        Err(err) => {
            tracing::error!(error = %err, "response serialisation failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialization_error",
                &err.to_string(),
            )
        }
    }
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
    // Params declared in `ep.params` dispatch on the registry
    // `ParamType` — the param NAME alone cannot pick the right cap
    // (`symbol` is a single 16-byte ticker on historical endpoints but
    // a 512-byte comma-separated list on the snapshot endpoints).
    // Params outside the registry metadata (`format`, unknown keys)
    // fall back to the name-based table, whose generic 64-byte cap
    // blocks `?format=<megabytes>` style floods.
    for (name, raw) in params {
        match ep.params.iter().find(|param| param.name == name) {
            Some(param) => validation::validate_param_value(param, raw)?,
            None => validation::validate_query_param(name, raw)?,
        }
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

/// Resolve the endpoint a request should actually dispatch to.
///
/// The legacy terminal serves both the single-date and the date-range
/// shape on the single-date URL, dispatching on which params are
/// populated. The registry models the range shape as a dedicated
/// `<stem>_range` sibling endpoint, so a query that carries
/// `start_date` + `end_date` without `date` on a path whose endpoint
/// requires `date` re-dispatches to the sibling instead of failing
/// with `missing required parameter: 'date'`. Every current and future
/// single-date endpoint with a `_range` sibling inherits the bridge
/// automatically; endpoints without a sibling keep their existing
/// missing-param diagnostics.
fn resolve_range_sibling<'a>(
    ep: &'a EndpointMeta,
    params: &HashMap<String, String>,
) -> &'a EndpointMeta {
    let wants_range = params.contains_key("start_date")
        && params.contains_key("end_date")
        && !params.contains_key("date");
    if !wants_range {
        return ep;
    }
    let requires_date = ep
        .params
        .iter()
        .any(|param| param.name == "date" && param.required);
    if !requires_date {
        return ep;
    }
    match thetadatadx::find(&format!("{}_range", ep.name)) {
        Some(sibling) => {
            tracing::debug!(
                from = ep.name,
                to = sibling.name,
                "date-range query on single-date path; dispatching to range sibling"
            );
            sibling
        }
        None => ep,
    }
}

/// `Retry-After` seconds advertised when the upstream reports
/// `ResourceExhausted` after the SDK's retry budget is spent. Upstream
/// tier slots free on a millisecond-to-second cadence, so one second is
/// the honest "immediately retryable" hint without inviting a tight
/// hammer loop.
const UPSTREAM_EXHAUSTED_RETRY_AFTER_SECS: u64 = 1;

fn endpoint_error_response(ep: &EndpointMeta, error: EndpointError) -> Response {
    match error {
        EndpointError::InvalidParams(message) => {
            error_response(StatusCode::BAD_REQUEST, "bad_request", &message)
        }
        EndpointError::UnknownEndpoint(message) => {
            error_response(StatusCode::NOT_FOUND, "not_found", &message)
        }
        // Upstream capacity rejection that survived the SDK's retry
        // budget (`ResourceExhausted` is classified transient and
        // retried with backoff before it ever reaches this handler).
        // 503 + Retry-After is the honest shape: the service is
        // temporarily out of upstream slots and the request is safely
        // retryable. 500 would imply a server fault; 429 would imply
        // the CLIENT exceeded a local quota, which it did not.
        EndpointError::Server(thetadatadx::Error::Grpc {
            kind: thetadatadx::GrpcStatusKind::ResourceExhausted,
            message,
            retry_after,
        }) => {
            tracing::warn!(
                endpoint = ep.name,
                error = %message,
                "upstream capacity exhausted after retries"
            );
            let mut resp = error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "upstream_exhausted",
                &format!("upstream is at capacity; retry shortly: {message}"),
            );
            // Prefer the server-advertised cooldown when the upstream
            // attached one; fall back to the static default otherwise.
            let retry_secs =
                retry_after.map_or(UPSTREAM_EXHAUSTED_RETRY_AFTER_SECS, |d| d.as_secs().max(1));
            if let Ok(value) = axum::http::HeaderValue::from_str(&retry_secs.to_string()) {
                resp.headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, value);
            }
            resp
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

// ---------------------------------------------------------------------------
//  Response format negotiation
// ---------------------------------------------------------------------------

/// Content type for NDJSON (newline-delimited JSON) responses. The
/// `charset` parameter mirrors the flat-file route surface, which has
/// always emitted it on JSONL bodies.
pub(crate) const NDJSON_CONTENT_TYPE: &str = "application/x-ndjson; charset=utf-8";

/// Wire formats a registry endpoint can render its response in.
///
/// Parsed from the `format` query parameter; unknown values are a 400 —
/// silently downgrading `format=parquet` to the JSON envelope made the
/// caller's pipeline fail far from the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseFormat {
    /// Java terminal JSON envelope (default).
    Json,
    /// RFC 4180 CSV with a header row.
    Csv,
    /// One JSON object per row, `\n`-delimited.
    Ndjson,
}

/// Parse the `format` query parameter. Absent means JSON.
fn parse_response_format(
    params: &HashMap<String, String>,
) -> Result<ResponseFormat, EndpointError> {
    let Some(raw) = params.get("format") else {
        return Ok(ResponseFormat::Json);
    };
    match raw.to_ascii_lowercase().as_str() {
        "json" => Ok(ResponseFormat::Json),
        "csv" => Ok(ResponseFormat::Csv),
        // `ndjson` and `jsonl` are the same line-delimited framing under
        // two community names; accept both like the flat-file routes do.
        "ndjson" | "jsonl" => Ok(ResponseFormat::Ndjson),
        other => Err(EndpointError::InvalidParams(format!(
            "unknown format: '{other}' (supported: json, csv, ndjson, jsonl)"
        ))),
    }
}

/// Build the `Content-Disposition` attachment filename for a CSV
/// response: `<endpoint>_<date>.csv`, or `<endpoint>_<start>_<end>.csv`
/// for range queries, or `<endpoint>.csv` when no date param is present.
///
/// Browser downloads (`<a download>`) fall back to the URL path's last
/// segment without this header, saving files as the bare endpoint stem
/// (`ohlc`, no extension); the legacy terminal sends a date-stamped
/// filename. Date values are filtered to `[A-Za-z0-9._-]` before being
/// embedded — they are validated upstream, but a header value must never
/// depend on a validator elsewhere staying tight.
fn csv_attachment_filename(ep: &EndpointMeta, params: &HashMap<String, String>) -> String {
    fn sanitize(value: &str) -> String {
        value
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            .collect()
    }

    if let Some(date) = params.get("date") {
        return format!("{}_{}.csv", ep.name, sanitize(date));
    }
    if let (Some(start), Some(end)) = (params.get("start_date"), params.get("end_date")) {
        return format!("{}_{}_{}.csv", ep.name, sanitize(start), sanitize(end));
    }
    format!("{}.csv", ep.name)
}

/// Render the `response` rows of a canonicalised envelope as NDJSON.
///
/// One JSON object per row, `\n`-delimited — the line-at-a-time framing
/// Pandas / Polars / DuckDB ingest natively. An empty response renders
/// as an empty body (zero lines), mirroring the CSV branch.
fn ndjson_response(json_val: &mut sonic_rs::Value) -> Response {
    // Collapse non-finite leaves once across the whole tree, then
    // serialise row-by-row; per-row serialisation cannot reintroduce
    // non-canonical cells.
    thetadatadx::json_canon::canonicalize(json_val);
    let rows = json_val
        .get("response")
        .and_then(|v: &sonic_rs::Value| v.as_array());
    let Some(rows) = rows else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            "response envelope is missing the response array",
        );
    };

    let mut body = String::with_capacity(rows.len() * 128);
    for row in rows.iter() {
        match sonic_rs::to_string(row) {
            Ok(line) => {
                body.push_str(&line);
                body.push('\n');
            }
            Err(err) => {
                tracing::error!(error = %err, "NDJSON row serialisation failed");
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "serialization_error",
                    &err.to_string(),
                );
            }
        }
    }

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, NDJSON_CONTENT_TYPE)],
        body,
    )
        .into_response()
}

/// Generic handler invoked for all registry endpoints.
///
/// 1. The [`BoundedQuery`] extractor enforces [`MAX_QUERY_PARAMS`] DURING
///    URL parse so `?a=1&b=2&...` flood attacks can't force a multi-MB
///    HashMap allocation before the cap trips.
/// 2. Re-dispatches `start_date`+`end_date` queries on a single-date path
///    to the `_range` registry sibling (see [`resolve_range_sibling`]).
/// 3. Validates query params against `EndpointMeta.params` (length caps
///    included — `format` falls under the generic 64-byte cap here).
/// 4. Negotiates the response format (`json` default, `csv`,
///    `ndjson`/`jsonl`); unknown `format` values are a 400.
/// 5. Invokes the shared endpoint runtime.
/// 6. Renders the negotiated format.
pub async fn generic(
    State(state): State<AppState>,
    BoundedQuery(params): BoundedQuery<MAX_QUERY_PARAMS>,
    ep: &EndpointMeta,
) -> Response {
    let ep = resolve_range_sibling(ep, &params);

    let args = match build_endpoint_args(ep, &params) {
        Ok(args) => args,
        Err(error) => return endpoint_error_response(ep, error),
    };

    // Format negotiation runs AFTER `build_endpoint_args` on purpose:
    // the length validators in there cap `format` at the generic
    // 64-byte bound, so the lowercase copy and the echoed-value 400
    // below can never amplify an oversized query value.
    let response_format = match parse_response_format(&params) {
        Ok(f) => f,
        Err(error) => return endpoint_error_response(ep, error),
    };

    let output = match invoke_endpoint(state.tdx(), ep.name, &args).await {
        Ok(output) => output,
        Err(error) => return endpoint_error_response(ep, error),
    };

    let mut json_val = format::output_envelope(&output);
    match response_format {
        ResponseFormat::Json => json_response(&mut json_val),
        ResponseFormat::Ndjson => ndjson_response(&mut json_val),
        ResponseFormat::Csv => {
            let disposition = format!(
                "attachment; filename=\"{}\"",
                csv_attachment_filename(ep, &params)
            );
            let body = json_val
                .get("response")
                .and_then(|v: &sonic_rs::Value| v.as_array())
                .and_then(|arr| {
                    let items: Vec<sonic_rs::Value> = arr.iter().cloned().collect();
                    format::json_to_csv(&items)
                })
                .unwrap_or_default();
            (
                StatusCode::OK,
                [
                    ("content-type", "text/csv; charset=utf-8"),
                    ("content-disposition", disposition.as_str()),
                ],
                body,
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
//  System endpoints
// ---------------------------------------------------------------------------

/// GET /v3/system/status -- matches Java terminal system status.
pub async fn system_status(State(state): State<AppState>) -> Response {
    let mut body = sonic_rs::json!({
        "status": state.mdds_status(),
        "version": env!("CARGO_PKG_VERSION")
    });
    json_response(&mut body)
}

/// GET /v3/system/mdds/status
pub async fn system_mdds_status(State(state): State<AppState>) -> Response {
    let mut body = format::ok_envelope(vec![sonic_rs::Value::from(state.mdds_status())]);
    json_response(&mut body)
}

/// GET /v3/system/fpss/status
pub async fn system_fpss_status(State(state): State<AppState>) -> Response {
    let mut body = sonic_rs::json!({
        "status": state.fpss_status(),
        "version": env!("CARGO_PKG_VERSION"),
        // Expose the bounded callback->broadcast drop counter so operators
        // can scrape one number to detect WS-side back-pressure without
        // tailing logs. Mirrors the FPSS SDK's `dropped_events()` surface.
        "broadcast_dropped": state.fpss_broadcast_dropped(),
        // M1 fix: surface the JSON-serialization-failure counter so a
        // sonic_rs::to_string regression on the hot path is visible
        // alongside the broadcast drop counter rather than swallowed.
        "json_serialize_failures": crate::ws::json_serialize_failure_count(),
    });
    json_response(&mut body)
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
    let mut body = format::ok_envelope(vec![sonic_rs::Value::from("OK")]);
    json_response(&mut body)
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use sonic_rs::Value;
    use thetadatadx::ENDPOINTS;

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

    /// Registry-declared params dispatch on `ParamType`, so the
    /// multi-symbol snapshot endpoints (param NAME `symbol`, registry
    /// type `Symbols`) accept comma-separated lists past the 16-byte
    /// single-ticker cap. Name-based dispatch used to reject any list
    /// with more than ~3 tickers.
    #[test]
    fn build_endpoint_args_accepts_multi_symbol_snapshot_lists() {
        let ep = thetadatadx::find("stock_snapshot_ohlc")
            .expect("snapshot endpoint must exist in the registry");
        let mut params: HashMap<String, String> = HashMap::new();
        params.insert(
            "symbol".to_string(),
            "AAPL,MSFT,TSLA,GOOG,AMZN,NVDA".to_string(),
        );

        let args = build_endpoint_args(ep, &params)
            .expect("six-ticker list must pass the Symbols-typed cap");
        assert_eq!(
            args.required_str("symbol").unwrap(),
            "AAPL,MSFT,TSLA,GOOG,AMZN,NVDA"
        );
    }

    /// Single-ticker endpoints keep the tight 16-byte cap — the typed
    /// dispatch must not loosen the historical surface.
    #[test]
    fn build_endpoint_args_keeps_single_symbol_cap_on_historical_endpoints() {
        let ep = thetadatadx::find("stock_history_eod")
            .expect("historical endpoint must exist in the registry");
        let mut params: HashMap<String, String> = HashMap::new();
        params.insert(
            "symbol".to_string(),
            "AAPL,MSFT,TSLA,GOOG,AMZN,NVDA".to_string(),
        );
        params.insert("start_date".to_string(), "20260101".to_string());
        params.insert("end_date".to_string(), "20260301".to_string());

        let err = build_endpoint_args(ep, &params)
            .expect_err("comma list must overflow the single-ticker cap");
        match err {
            EndpointError::InvalidParams(msg) => {
                assert!(
                    msg.contains("'symbol'") && msg.contains("16"),
                    "expected the 16-byte single-ticker rejection: {msg}"
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
    //  Range-sibling auto-routing
    // -----------------------------------------------------------------------

    fn string_params(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// `start_date`+`end_date` (without `date`) on a single-date path
    /// dispatches to the registry's `_range` sibling, matching the
    /// legacy terminal which accepts both shapes on one URL.
    #[test]
    fn range_query_on_single_date_path_routes_to_range_sibling() {
        let ep = thetadatadx::find("stock_history_ohlc").expect("endpoint exists");
        let params = string_params(&[
            ("symbol", "AMZN"),
            ("start_date", "20260603"),
            ("end_date", "20260605"),
            ("interval", "1m"),
        ]);
        let resolved = resolve_range_sibling(ep, &params);
        assert_eq!(resolved.name, "stock_history_ohlc_range");

        // ...and the resolved endpoint accepts the params end-to-end.
        build_endpoint_args(resolved, &params)
            .expect("range sibling must accept the range-shaped query");
    }

    #[test]
    fn single_date_query_stays_on_single_date_endpoint() {
        let ep = thetadatadx::find("stock_history_ohlc").expect("endpoint exists");
        let params = string_params(&[("symbol", "AMZN"), ("date", "20260603")]);
        let resolved = resolve_range_sibling(ep, &params);
        assert_eq!(resolved.name, "stock_history_ohlc");
    }

    /// An explicit `date` wins even when the range pair is also present:
    /// the single-date endpoint already accepts optional `start_date` /
    /// `end_date` pass-through, so no re-dispatch happens.
    #[test]
    fn explicit_date_disables_range_routing() {
        let ep = thetadatadx::find("stock_history_ohlc").expect("endpoint exists");
        let params = string_params(&[
            ("symbol", "AMZN"),
            ("date", "20260603"),
            ("start_date", "20260603"),
            ("end_date", "20260605"),
        ]);
        assert_eq!(
            resolve_range_sibling(ep, &params).name,
            "stock_history_ohlc"
        );
    }

    /// Endpoints without a `_range` sibling keep their existing
    /// missing-param diagnostics — no silent re-dispatch to nowhere.
    #[test]
    fn endpoints_without_range_sibling_keep_missing_date_diagnostic() {
        let ep = thetadatadx::find("stock_history_trade").expect("endpoint exists");
        let params = string_params(&[
            ("symbol", "AMZN"),
            ("start_date", "20260603"),
            ("end_date", "20260605"),
        ]);
        let resolved = resolve_range_sibling(ep, &params);
        assert_eq!(resolved.name, "stock_history_trade");
        let err = build_endpoint_args(resolved, &params)
            .expect_err("missing required date must still surface");
        match err {
            EndpointError::InvalidParams(msg) => {
                assert!(msg.contains("'date'"), "diagnostic names the param: {msg}");
            }
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    /// Endpoints whose range shape is native (required `start_date` /
    /// `end_date`, no `date` param) never re-dispatch.
    #[test]
    fn native_range_endpoints_are_untouched() {
        let ep = thetadatadx::find("stock_history_eod").expect("endpoint exists");
        let params = string_params(&[
            ("symbol", "AAPL"),
            ("start_date", "20260101"),
            ("end_date", "20260301"),
        ]);
        assert_eq!(resolve_range_sibling(ep, &params).name, "stock_history_eod");
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

    // -----------------------------------------------------------------------
    //  non-finite f64 must not collapse to an empty body
    // -----------------------------------------------------------------------

    /// Read the body of an axum `Response` to a `String` synchronously inside
    /// a tokio runtime. Used by the NaN-cell regression tests below.
    async fn read_body(resp: Response) -> String {
        use axum::body::to_bytes;
        let body = resp.into_body();
        let bytes = to_bytes(body, usize::MAX).await.expect("body collect");
        String::from_utf8(bytes.to_vec()).expect("body utf8")
    }

    // -----------------------------------------------------------------------
    //  Response-format negotiation + NDJSON rendering
    // -----------------------------------------------------------------------

    #[test]
    fn response_format_defaults_to_json_and_parses_known_values() {
        assert_eq!(
            parse_response_format(&HashMap::new()).unwrap(),
            ResponseFormat::Json
        );
        for (raw, expected) in [
            ("json", ResponseFormat::Json),
            ("csv", ResponseFormat::Csv),
            ("CSV", ResponseFormat::Csv),
            ("ndjson", ResponseFormat::Ndjson),
            ("NDJSON", ResponseFormat::Ndjson),
            ("jsonl", ResponseFormat::Ndjson),
            ("JSONL", ResponseFormat::Ndjson),
        ] {
            let params = string_params(&[("format", raw)]);
            assert_eq!(parse_response_format(&params).unwrap(), expected, "{raw}");
        }
    }

    /// Unknown `format` values are a 400 listing the supported set —
    /// silently downgrading to the JSON envelope made the caller's
    /// pipeline fail far from the cause.
    #[test]
    fn response_format_rejects_unknown_values() {
        for bad in ["parquet", "arrow", "xml", "nd-json"] {
            let params = string_params(&[("format", bad)]);
            let err = parse_response_format(&params).expect_err(bad);
            match err {
                EndpointError::InvalidParams(msg) => {
                    assert!(msg.contains(bad), "message echoes the value: {msg}");
                    for supported in ["json", "csv", "ndjson", "jsonl"] {
                        assert!(
                            msg.contains(supported),
                            "message lists '{supported}': {msg}"
                        );
                    }
                }
                other => panic!("expected InvalidParams, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn ndjson_response_emits_one_object_per_row() {
        let mut envelope = format::ok_envelope(vec![
            sonic_rs::json!({"symbol": "AAPL", "close": 200.5}),
            sonic_rs::json!({"symbol": "MSFT", "close": 470.0}),
        ]);
        let resp = ndjson_response(&mut envelope);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/x-ndjson; charset=utf-8")
        );

        let body = read_body(resp).await;
        assert!(body.ends_with('\n'), "every row line is newline-terminated");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2, "one line per response row: {body}");
        for line in &lines {
            let row: Value = sonic_rs::from_str(line).expect("each line is standalone JSON");
            assert!(
                row.get("symbol").is_some(),
                "row carries its fields: {line}"
            );
        }
        assert!(
            !body.contains("\"header\""),
            "NDJSON body must not carry the envelope wrapper: {body}"
        );
    }

    #[tokio::test]
    async fn ndjson_response_renders_empty_response_as_empty_body() {
        let mut envelope = format::ok_envelope(vec![]);
        let resp = ndjson_response(&mut envelope);
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body(resp).await;
        assert!(body.is_empty(), "zero rows render zero lines, got {body:?}");
    }

    #[tokio::test]
    async fn ndjson_response_collapses_non_finite_cells_to_null() {
        let mut row = sonic_rs::json!({"symbol": "AAPL", "vega": Value::new_null()});
        if let Some(o) = row.as_object_mut() {
            o.insert(&"vega", thetadatadx::json_canon::finite_or_null(f64::NAN));
        }
        let mut envelope = format::ok_envelope(vec![row]);
        let resp = ndjson_response(&mut envelope);
        let body = read_body(resp).await;
        assert!(
            body.contains("\"vega\":null"),
            "non-finite cells collapse to null on the NDJSON path too: {body}"
        );
    }

    // -----------------------------------------------------------------------
    //  CSV attachment filename
    // -----------------------------------------------------------------------

    #[test]
    fn csv_filename_stamps_endpoint_and_date() {
        let ep = thetadatadx::find("stock_history_ohlc").expect("endpoint exists");
        let params = string_params(&[("symbol", "AAPL"), ("date", "20260603")]);
        assert_eq!(
            csv_attachment_filename(ep, &params),
            "stock_history_ohlc_20260603.csv"
        );
    }

    #[test]
    fn csv_filename_stamps_date_range() {
        let ep = thetadatadx::find("stock_history_eod").expect("endpoint exists");
        let params = string_params(&[
            ("symbol", "AAPL"),
            ("start_date", "20260101"),
            ("end_date", "20260301"),
        ]);
        assert_eq!(
            csv_attachment_filename(ep, &params),
            "stock_history_eod_20260101_20260301.csv"
        );
    }

    #[test]
    fn csv_filename_falls_back_to_endpoint_stem() {
        let ep = thetadatadx::find("stock_list_symbols").expect("endpoint exists");
        assert_eq!(
            csv_attachment_filename(ep, &HashMap::new()),
            "stock_list_symbols.csv"
        );
    }

    /// Header values must never carry quote / control bytes even if a
    /// validator elsewhere loosens — the filename filter is the last
    /// line of defence against header splitting.
    #[test]
    fn csv_filename_strips_header_unsafe_bytes() {
        let ep = thetadatadx::find("stock_history_ohlc").expect("endpoint exists");
        let params = string_params(&[("date", "2026\"06\r\n03;")]);
        assert_eq!(
            csv_attachment_filename(ep, &params),
            "stock_history_ohlc_20260603.csv"
        );
    }

    // -----------------------------------------------------------------------
    //  Canonical error envelope + content type
    // -----------------------------------------------------------------------

    /// Every error response must serialise the canonical envelope
    /// (`header.error_type` + `header.error_msg` + empty `response`) with
    /// the bare `application/json` content type. Clients write one error
    /// parser against this shape; the nested `error.message` variant and
    /// the `; charset=utf-8` suffix must never reappear.
    #[tokio::test]
    async fn error_response_emits_canonical_envelope_and_bare_content_type() {
        let resp = error_response(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "missing required parameter: 'date'",
        );
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/json"),
            "content type must be the bare media type"
        );

        let body = read_body(resp).await;
        let parsed: Value = sonic_rs::from_str(&body).expect("error body must be valid JSON");
        assert_eq!(
            parsed
                .get("header")
                .and_then(|h| h.get("error_type"))
                .and_then(Value::as_str),
            Some("bad_request")
        );
        assert_eq!(
            parsed
                .get("header")
                .and_then(|h| h.get("error_msg"))
                .and_then(Value::as_str),
            Some("missing required parameter: 'date'")
        );
        assert!(
            parsed
                .get("response")
                .and_then(|r: &Value| r.as_array())
                .is_some_and(|rows| rows.is_empty()),
            "error envelope must carry an empty response array: {body}"
        );
        assert!(
            parsed.get("error").is_none(),
            "nested error.message form must not be emitted: {body}"
        );
    }

    /// Upstream `ResourceExhausted` that survives the SDK retry budget
    /// maps to 503 + `Retry-After`, not an opaque 500: the request is
    /// safely retryable and the fault is capacity, not the server.
    #[tokio::test]
    async fn upstream_resource_exhausted_maps_to_503_with_retry_after() {
        let ep = any_endpoint();
        let resp = endpoint_error_response(
            ep,
            EndpointError::Server(thetadatadx::Error::Grpc {
                kind: thetadatadx::GrpcStatusKind::ResourceExhausted,
                message: "stream quota exceeded".to_string(),
                retry_after: None,
            }),
        );
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("1")
        );
        let body = read_body(resp).await;
        assert!(
            body.contains("\"error_type\":\"upstream_exhausted\""),
            "envelope names the capacity condition: {body}"
        );
        assert!(
            body.contains("stream quota exceeded"),
            "upstream detail is preserved: {body}"
        );
    }

    /// Other gRPC faults keep the 500 server_error shape — only the
    /// capacity condition is retry-hinted.
    #[tokio::test]
    async fn other_grpc_faults_stay_500() {
        let ep = any_endpoint();
        let resp = endpoint_error_response(
            ep,
            EndpointError::Server(thetadatadx::Error::Grpc {
                kind: thetadatadx::GrpcStatusKind::Internal,
                message: "decode fault".to_string(),
                retry_after: None,
            }),
        );
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(resp
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .is_none());
    }

    #[tokio::test]
    async fn json_response_uses_bare_json_content_type() {
        let mut envelope = format::ok_envelope(vec![Value::from("AAPL")]);
        let resp = json_response(&mut envelope);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/json"),
            "JSON envelope responses must use the bare media type"
        );
    }

    #[tokio::test]
    async fn json_response_emits_non_empty_body_for_nan_payload() {
        // Construct a payload that mimics a Greeks-style row where one cell
        // came back non-finite from the upstream solver. A naive
        // `sonic_rs::to_string(...).unwrap_or_default()` round-trip would hand
        // the client a 200 OK with an empty body; canonicalisation must not.
        let mut row = sonic_rs::json!({
            "symbol": "AAPL",
            "delta": 0.5_f64,
            // `vega` slot is filled in below with a pre-collapsed NaN sentinel
            // so the canonicaliser walk still has work to do on a real leaf.
            "vega": Value::new_null(),
        });
        if let Some(o) = row.as_object_mut() {
            o.insert(&"vega", thetadatadx::json_canon::finite_or_null(f64::NAN));
        }
        let mut envelope = format::ok_envelope(vec![row]);

        let resp = json_response(&mut envelope);
        assert_eq!(resp.status(), StatusCode::OK);

        let body = read_body(resp).await;
        assert!(
            !body.is_empty(),
            "issue #459: NaN cell must not collapse the response to an empty body"
        );
        // Non-finite cell must serialise as JSON null — exact byte assertion.
        assert!(
            body.contains("\"vega\":null"),
            "vega must canonicalise to null in the wire body, got {body}"
        );
        // Sibling finite cells must round-trip unchanged.
        assert!(
            body.contains("\"delta\":0.5"),
            "delta must round-trip unchanged, got {body}"
        );
        assert!(
            body.contains("\"symbol\":\"AAPL\""),
            "symbol must round-trip unchanged, got {body}"
        );
        // The full envelope shape — `header` + `response` array — must be
        // intact, so clients that pattern-match on `header.error_type` to
        // distinguish success from failure still work.
        assert!(
            body.contains("\"header\""),
            "envelope header missing: {body}"
        );
        assert!(
            body.contains("\"response\""),
            "envelope response missing: {body}"
        );
    }
}
