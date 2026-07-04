//! Generic endpoint handler and shared-runtime dispatch for all registry endpoints.
//!
//! A single handler function receives endpoint metadata via closure capture,
//! validates query params, invokes the shared endpoint runtime in
//! `thetadatadx`, and returns the JVM terminal JSON envelope (or CSV when
//! `format=csv`).

use std::collections::HashMap;

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
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

/// Content type for the v3 plain-text error body. v3 returns the HTTP
/// status with the error description as the body (no JSON envelope), so the
/// data / registry error path emits `text/plain`.
pub(crate) const TEXT_PLAIN_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// Build a v3 plain-text error response: the HTTP status carrying `msg` as
/// a `text/plain` body, with no JSON envelope.
///
/// This is the v3 contract for the registry / data error path (`status` +
/// plain description). The JSON-enveloped [`error_response`] is retained for
/// the extractor-boundary and rate-limit rejections that other route layers
/// (the `BoundedQuery` extractor, the rate-limit path in `router`, the
/// shutdown-token guard) hand-write against; converting those is out of
/// scope here because they live in files owned elsewhere.
pub(crate) fn plain_error_response(status: StatusCode, msg: &str) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, TEXT_PLAIN_CONTENT_TYPE)],
        msg.to_owned(),
    )
        .into_response()
}

/// Deprecated v2 query parameters and their v3 replacements.
///
/// The JVM terminal's pre-matching filter rejects any request carrying one
/// of these legacy v2 parameter names with `410 Gone` and a body naming the
/// v3 replacement, rather than silently ignoring it (these names are not v3
/// parameters, so a request using them would otherwise fail later with an
/// opaque "missing required parameter" or simply drop the value). Replicating
/// the terminal's mapping means a v2 client gets the same actionable upgrade
/// message from this server.
///
/// `"*unnecessary*"` marks a v2 knob with no v3 equivalent (the behaviour is
/// now always-on or removed); `"*path parameter*"` marks a value that moved
/// from the query string into the URL path (`sec`). The names and their
/// replacements match the JVM terminal's v2-parameter rejection map.
const DEPRECATED_V2_PARAMS: &[(&str, &str)] = &[
    ("annual_div", "annual_dividend"),
    ("exp", "expiration"),
    ("ivl", "interval"),
    ("perf_boost", "*unnecessary*"),
    ("pretty_time", "*unnecessary*"),
    ("rate", "rate_type"),
    ("root", "symbol"),
    ("rth", "*unnecessary*"),
    ("sec", "*path parameter*"),
    ("under_price", "stock_price"),
    ("use_csv", "format"),
];

/// Reject a request carrying any deprecated v2 query parameter with `410
/// Gone`, mirroring the JVM terminal's pre-matching filter.
///
/// Returns `Some(response)` — a `410` whose `text/plain` body lists each
/// offending parameter and its v3 replacement — when at least one v2
/// parameter name is present, and `None` when the request is clean (the
/// caller then proceeds to normal dispatch). The check is a cheap scan over
/// the already-parsed query keys; `DEPRECATED_V2_PARAMS` has eleven entries,
/// so the per-request cost is negligible.
///
/// Splitting the decision (pure, over a `&HashMap`) from the middleware glue
/// keeps it unit-testable without driving a live request through the router.
fn deprecated_v2_param_response(params: &HashMap<String, String>) -> Option<Response> {
    let mut hits: Vec<(&str, &str)> = DEPRECATED_V2_PARAMS
        .iter()
        .filter(|(name, _)| params.contains_key(*name))
        .map(|(name, replacement)| (*name, *replacement))
        .collect();
    if hits.is_empty() {
        return None;
    }
    // Stable, deterministic ordering so the body (and tests) don't depend on
    // HashMap iteration order.
    hits.sort_unstable_by(|a, b| a.0.cmp(b.0));

    let mut body = String::from(
        "We have upgraded to API v3. Please use API v3 query parameters instead.\n\
         Deprecated query parameters:\n",
    );
    for (name, replacement) in hits {
        body.push('\t');
        body.push_str(name);
        body.push_str(" -> ");
        body.push_str(replacement);
        body.push('\n');
    }
    body.push_str("Consult API v3 documentation for more information: https://docs.thetadata.us/");
    Some(plain_error_response(StatusCode::GONE, &body))
}

/// Build a JSON error response with the JVM terminal error envelope format.
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
            // An absent optional param that the JVM terminal injects an
            // upstream default for is forwarded with that exact literal so
            // a request omitting it sees the same data window / venue /
            // interval the terminal would have produced (see
            // [`terminal_param_default`]). Defaults that fail to parse for
            // their declared `ParamType` are a server bug, not a client
            // fault, so the parse error propagates as-is.
            None if !param.required => {
                if let Some(default) = terminal_param_default(param.name) {
                    args.insert_raw(param.name, param.param_type, default)?;
                }
            }
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

/// The default literal the JVM terminal injects upstream for `param_name`
/// when the client omits it, or `None` for a param the terminal forwards
/// only when present.
///
/// The JVM terminal's REST layer substitutes a fixed literal for each of
/// these params when the request omits it (and always sets `interval` to
/// `1s` when it is absent), before forwarding the request to the upstream
/// gRPC service. A request that omits one of these params therefore lands on
/// the upstream with the terminal's literal, not an empty field. This server
/// forwards only what the client sent, so an omitted param previously
/// reached the upstream as unset — a different data window / venue /
/// interval / pairing rule than the terminal. The literals below match the
/// terminal's so the two front ends produce identical upstream calls.
///
/// Injection is name-gated AND guarded by the endpoint declaring the param:
/// `build_endpoint_args` only consults this table for a param the endpoint
/// actually has in `ep.params`, so a snapshot endpoint (no `start_time`)
/// never gains a time window and an `eod` endpoint (no `interval`) never
/// gains a bar size. Every current and future endpoint that declares one of
/// these optional params inherits the terminal-matching default
/// automatically.
///
/// `exclusive` intentionally defaults to `true` here even though the
/// registry's SDK-facing metadata documents `false`: the JVM terminal's REST
/// front end injects `true` for an omitted `exclusive`, and this server's job
/// is to reproduce the terminal's wire behaviour for a request that omits the
/// param. The registry default governs the typed SDK builders, a separate
/// surface.
fn terminal_param_default(param_name: &str) -> Option<&'static str> {
    match param_name {
        "venue" => Some("nqb"),
        "start_time" => Some("09:30:00"),
        "end_time" => Some("16:00:00"),
        "interval" => Some("1s"),
        "exclusive" => Some("true"),
        _ => None,
    }
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
    // v3 error bodies are the HTTP status plus a plain-text description (no
    // JSON envelope), so every arm here returns `plain_error_response`.
    match error {
        EndpointError::InvalidParams(message) => {
            plain_error_response(StatusCode::BAD_REQUEST, &message)
        }
        EndpointError::UnknownEndpoint(message) => {
            plain_error_response(StatusCode::NOT_FOUND, &message)
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
            let mut resp = plain_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
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
            plain_error_response(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string())
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
    /// JSON envelope.
    Json,
    /// RFC 4180 CSV with a header row.
    Csv,
    /// One JSON object per row, `\n`-delimited.
    Ndjson,
}

/// Parse the `format` query parameter. Absent means CSV — the v3 spec
/// defaults `format` to `csv` on every path (the vendor terminal's
/// `ResultsFormat.fromString` returns `CSV` for an absent / blank value).
fn parse_response_format(
    params: &HashMap<String, String>,
) -> Result<ResponseFormat, EndpointError> {
    let Some(raw) = params.get("format") else {
        return Ok(ResponseFormat::Csv);
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
/// 4. Negotiates the response format (`csv` default, `json`,
///    `ndjson`/`jsonl`); unknown `format` values are a 400.
/// 5. Invokes the shared endpoint runtime.
/// 6. Renders the negotiated format.
pub async fn generic(
    State(state): State<AppState>,
    BoundedQuery(params): BoundedQuery<MAX_QUERY_PARAMS>,
    ep: &EndpointMeta,
) -> Response {
    generic_with_overrides(State(state), BoundedQuery(params), ep, &[]).await
}

/// Generic handler with caller-supplied parameter overrides folded into the
/// query map before dispatch.
///
/// The JVM terminal serves `request_type` as a URL **path** segment on the
/// list-dates / list-contracts routes
/// (`/v3/stock/list/dates/{request_type}`), then sets it on the upstream
/// request as if it were the `request_type` query param. This server models
/// `request_type` as a registry query param, so the path-segment routes pass
/// the captured segment here as `("request_type", segment)` and it is folded
/// into the query map exactly as though the client had sent
/// `?request_type=<segment>`. An override always wins over a query value of
/// the same name, matching the terminal where the path binding is
/// authoritative.
///
/// `overrides` is a tiny fixed slice (one entry today), so the linear scan
/// and per-entry `insert` cost nothing measurable against the per-request
/// work that follows.
pub async fn generic_with_overrides(
    State(state): State<AppState>,
    BoundedQuery(mut params): BoundedQuery<MAX_QUERY_PARAMS>,
    ep: &EndpointMeta,
    overrides: &[(&str, String)],
) -> Response {
    for (key, value) in overrides {
        params.insert((*key).to_string(), value.clone());
    }

    // v2 → v3 parameter migration gate. A request carrying a deprecated v2
    // parameter name is rejected with `410 Gone` naming the v3 replacement,
    // matching the terminal's pre-matching filter. Runs before any
    // validation / dispatch so a v2 client gets the upgrade message rather
    // than an opaque missing-parameter error from a name v3 does not know.
    if let Some(response) = deprecated_v2_param_response(&params) {
        return response;
    }

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

    let output = match invoke_endpoint(state.client().historical(), ep.name, &args).await {
        Ok(output) => output,
        Err(error) => return endpoint_error_response(ep, error),
    };

    // Build the flat v3 rows once. The CSV / NDJSON renderers consume them
    // directly (contract identity inline per row); the JSON renderer groups
    // option rows under their `contract`. The request contract params are the
    // source of the per-row / contract identity — wildcard responses echo the
    // contract columns per row (those win), but the wire never carries
    // `symbol` and a single-contract response carries no contract columns, so
    // they are threaded from here where the query is known.
    let contract = format::ContractParams {
        symbol: params.get("symbol").map(String::as_str),
        expiration: params.get("expiration").map(String::as_str),
        strike: params.get("strike").map(String::as_str),
        right: params.get("right").map(String::as_str),
    };
    let rows = format::response_rows(ep, &contract, &output);
    match response_format {
        ResponseFormat::Json => {
            let mut json_val = format::json_envelope(ep, rows);
            json_response(&mut json_val)
        }
        ResponseFormat::Ndjson => {
            // NDJSON stays flat (one contract-inline row per line) — only the
            // JSON envelope groups under `contract`.
            let mut json_val = format::ok_envelope(rows);
            ndjson_response(&mut json_val)
        }
        ResponseFormat::Csv => {
            let disposition = format!(
                "attachment; filename=\"{}\"",
                csv_attachment_filename(ep, &params)
            );
            let body = format::json_to_csv(ep, &rows).unwrap_or_default();
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

/// Dispatch a `{request_type}` path-segment route to its registry endpoint.
///
/// The JVM terminal exposes three list routes with `request_type` as a
/// **path** segment rather than a query param:
/// `/v3/stock/list/dates/{request_type}`,
/// `/v3/option/list/dates/{request_type}`, and
/// `/v3/option/list/contracts/{request_type}` (the v3 OpenAPI marks the
/// segment `required: true`). The matching registry endpoints
/// (`stock_list_dates`, `option_list_dates`, `option_list_contracts`) model
/// `request_type` as a required query param, so a request to the terminal's
/// path form would otherwise 404. This handler captures the segment and
/// folds it into the query map as `request_type` before delegating to the
/// shared [`generic`] pipeline, so the path form and the query form resolve
/// to the same upstream call.
///
/// `endpoint_name` is resolved against the registry once per request; it is
/// always one of the three names wired in `router::build`, so the lookup
/// cannot legitimately miss. A miss is treated as a server misconfiguration
/// (500) rather than a client fault.
pub async fn list_by_request_type(
    state: State<AppState>,
    axum::extract::Path(request_type): axum::extract::Path<String>,
    query: BoundedQuery<MAX_QUERY_PARAMS>,
    endpoint_name: &'static str,
) -> Response {
    let Some(ep) = thetadatadx::find(endpoint_name) else {
        tracing::error!(
            endpoint = endpoint_name,
            "request_type path route is wired to an endpoint missing from the registry"
        );
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "route is wired to an unknown endpoint",
        );
    };
    generic_with_overrides(state, query, ep, &[("request_type", request_type)]).await
}

// ---------------------------------------------------------------------------
//  Terminal system endpoints
// ---------------------------------------------------------------------------

/// GET /v3/terminal/shutdown -- kills the server process, matching the JVM
/// terminal. Unauthenticated, returns the plain-text body `OK`, exactly as
/// the terminal documents it.
pub async fn terminal_shutdown(State(state): State<AppState>) -> Response {
    tracing::info!("shutdown requested via /v3/terminal/shutdown");
    state.shutdown();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, TEXT_PLAIN_CONTENT_TYPE)],
        "OK",
    )
        .into_response()
}

/// GET /v3/terminal/fpss/status -- FPSS (streaming) channel health as the
/// terminal's one-word `text/plain` body (`CONNECTED` / `DISCONNECTED`).
pub async fn terminal_streaming_status(State(state): State<AppState>) -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, TEXT_PLAIN_CONTENT_TYPE)],
        state.fpss_status(),
    )
        .into_response()
}

/// GET /v3/terminal/mdds/status -- MDDS (historical) channel health as the
/// terminal's one-word `text/plain` body (`CONNECTED` / `DISCONNECTED`).
pub async fn terminal_historical_status(State(state): State<AppState>) -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, TEXT_PLAIN_CONTENT_TYPE)],
        state.mdds_status(),
    )
        .into_response()
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

    // -----------------------------------------------------------------------
    //  C8 — server-side parameter defaults match the JVM terminal
    // -----------------------------------------------------------------------

    /// The default literals match the JVM terminal's absent-param
    /// substitutions exactly.
    #[test]
    fn terminal_param_defaults_match_the_terminal_literals() {
        assert_eq!(terminal_param_default("venue"), Some("nqb"));
        assert_eq!(terminal_param_default("start_time"), Some("09:30:00"));
        assert_eq!(terminal_param_default("end_time"), Some("16:00:00"));
        assert_eq!(terminal_param_default("interval"), Some("1s"));
        // The terminal injects `exclusive=true` even though the SDK-facing
        // registry metadata documents `false`; the REST front end is what we
        // reproduce here.
        assert_eq!(terminal_param_default("exclusive"), Some("true"));
        // Params the terminal forwards only when present get no default.
        assert_eq!(terminal_param_default("symbol"), None);
        assert_eq!(terminal_param_default("date"), None);
        assert_eq!(terminal_param_default("strike"), None);
    }

    /// An intraday OHLC request that omits the windowing params lands on the
    /// upstream with the terminal's `09:30:00`..`16:00:00` window, `1s`
    /// interval, and `nqb` venue — not an empty field per param.
    #[test]
    fn build_endpoint_args_injects_window_interval_and_venue_defaults() {
        let ep = thetadatadx::find("stock_history_ohlc").expect("endpoint exists");
        let params = string_params(&[("symbol", "AAPL"), ("date", "20260603")]);
        let args = build_endpoint_args(ep, &params).expect("args build");
        assert_eq!(args.optional_str("interval").unwrap(), Some("1s"));
        assert_eq!(args.optional_str("start_time").unwrap(), Some("09:30:00"));
        assert_eq!(args.optional_str("end_time").unwrap(), Some("16:00:00"));
        assert_eq!(args.optional_str("venue").unwrap(), Some("nqb"));
    }

    /// A client value always wins over the injected default — the default is
    /// the absent-param fallback, never an override.
    #[test]
    fn build_endpoint_args_default_yields_to_client_value() {
        let ep = thetadatadx::find("stock_history_ohlc").expect("endpoint exists");
        let params = string_params(&[
            ("symbol", "AAPL"),
            ("date", "20260603"),
            ("interval", "1m"),
            ("venue", "utp_cta"),
        ]);
        let args = build_endpoint_args(ep, &params).expect("args build");
        assert_eq!(args.optional_str("interval").unwrap(), Some("1m"));
        assert_eq!(args.optional_str("venue").unwrap(), Some("utp_cta"));
        // The untouched window params still pick up the terminal default.
        assert_eq!(args.optional_str("start_time").unwrap(), Some("09:30:00"));
    }

    /// The `trade_quote` endpoint declares `exclusive` (and no `interval`):
    /// the `exclusive=true` default is injected and NO interval is added,
    /// because the endpoint does not declare one.
    #[test]
    fn build_endpoint_args_injects_exclusive_true_and_no_interval_for_trade_quote() {
        let ep = thetadatadx::find("stock_history_trade_quote").expect("endpoint exists");
        let params = string_params(&[("symbol", "AAPL"), ("date", "20260603")]);
        let args = build_endpoint_args(ep, &params).expect("args build");
        assert_eq!(args.optional_bool("exclusive").unwrap(), Some(true));
        assert_eq!(args.optional_str("start_time").unwrap(), Some("09:30:00"));
        assert_eq!(args.optional_str("venue").unwrap(), Some("nqb"));
        // The endpoint has no `interval` param, so none is injected.
        assert_eq!(args.optional_str("interval").unwrap(), None);
    }

    /// Defaults are injected ONLY for params the endpoint declares: an EOD
    /// range endpoint that declares none of the window/venue/interval params
    /// gains none of them.
    #[test]
    fn build_endpoint_args_injects_no_defaults_for_endpoints_without_those_params() {
        let ep = thetadatadx::find("stock_history_eod").expect("endpoint exists");
        let params = string_params(&[
            ("symbol", "AAPL"),
            ("start_date", "20260101"),
            ("end_date", "20260301"),
        ]);
        let args = build_endpoint_args(ep, &params).expect("args build");
        assert_eq!(args.optional_str("interval").unwrap(), None);
        assert_eq!(args.optional_str("start_time").unwrap(), None);
        assert_eq!(args.optional_str("end_time").unwrap(), None);
        assert_eq!(args.optional_str("venue").unwrap(), None);
    }

    /// A snapshot endpoint declares `venue` but no time window: only the
    /// venue default is injected.
    #[test]
    fn build_endpoint_args_injects_only_venue_for_snapshot() {
        let ep = thetadatadx::find("stock_snapshot_ohlc").expect("endpoint exists");
        let params = string_params(&[("symbol", "AAPL")]);
        let args = build_endpoint_args(ep, &params).expect("args build");
        assert_eq!(args.optional_str("venue").unwrap(), Some("nqb"));
        assert_eq!(args.optional_str("start_time").unwrap(), None);
        assert_eq!(args.optional_str("interval").unwrap(), None);
    }

    // -----------------------------------------------------------------------
    //  H8 — deprecated v2 query parameters are rejected with 410
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn deprecated_v2_param_returns_410_naming_the_replacement() {
        let params = string_params(&[("root", "AAPL"), ("date", "20260603")]);
        let resp =
            deprecated_v2_param_response(&params).expect("a v2 param must trigger the 410 gate");
        assert_eq!(resp.status(), StatusCode::GONE);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; charset=utf-8")
        );
        let body = read_body(resp).await;
        assert!(
            body.contains("root -> symbol"),
            "body must map the deprecated param to its v3 name: {body}"
        );
        assert!(
            body.contains("API v3"),
            "body must carry the upgrade message: {body}"
        );
    }

    #[tokio::test]
    async fn deprecated_v2_params_list_every_offender_sorted() {
        let params = string_params(&[("ivl", "1m"), ("exp", "20260101"), ("use_csv", "true")]);
        let resp = deprecated_v2_param_response(&params).expect("v2 params present");
        let body = read_body(resp).await;
        // Deterministic, sorted order regardless of HashMap iteration.
        let exp_at = body.find("exp -> expiration").expect("exp mapped");
        let ivl_at = body.find("ivl -> interval").expect("ivl mapped");
        let csv_at = body.find("use_csv -> format").expect("use_csv mapped");
        assert!(
            exp_at < ivl_at && ivl_at < csv_at,
            "entries must be sorted: {body}"
        );
    }

    #[test]
    fn clean_request_does_not_trigger_the_v2_gate() {
        let params = string_params(&[("symbol", "AAPL"), ("interval", "1m"), ("format", "json")]);
        assert!(
            deprecated_v2_param_response(&params).is_none(),
            "a v3-only request must pass the gate untouched"
        );
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
    fn response_format_defaults_to_csv_and_parses_known_values() {
        // v3 defaults `format` to `csv` when the param is absent.
        assert_eq!(
            parse_response_format(&HashMap::new()).unwrap(),
            ResponseFormat::Csv
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
        // v3 error body is plain text (no JSON envelope) carrying the
        // description.
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; charset=utf-8")
        );
        let body = read_body(resp).await;
        assert!(
            !body.contains("\"header\"") && !body.contains("\"error_type\""),
            "v3 error body must not carry a JSON envelope: {body}"
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

    /// v3 registry / data errors are a plain-text body at the right status —
    /// no JSON envelope, `text/plain` content type, the message verbatim.
    #[tokio::test]
    async fn endpoint_invalid_params_emits_plain_text_body() {
        let ep = any_endpoint();
        let resp = endpoint_error_response(
            ep,
            EndpointError::InvalidParams("missing required parameter: 'date'".to_string()),
        );
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; charset=utf-8")
        );
        let body = read_body(resp).await;
        assert_eq!(body, "missing required parameter: 'date'");
        assert!(
            !body.contains('{') && !body.contains("header"),
            "v3 error body must not be a JSON envelope: {body}"
        );
    }

    /// An unknown endpoint maps to a 404 plain-text body.
    #[tokio::test]
    async fn endpoint_unknown_emits_plain_text_404() {
        let ep = any_endpoint();
        let resp = endpoint_error_response(
            ep,
            EndpointError::UnknownEndpoint("no such endpoint: 'frobnicate'".to_string()),
        );
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; charset=utf-8")
        );
        let body = read_body(resp).await;
        assert_eq!(body, "no such endpoint: 'frobnicate'");
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
        // The v3 success envelope is `{"response": [...]}` with NO `header`
        // key (v3 carries no header on any path). Assert the real shape: no
        // header, a `response` array, and the row's actual values inside it.
        let parsed: Value = sonic_rs::from_str(&body).expect("body must be valid JSON");
        assert!(
            parsed.get("header").is_none(),
            "v3 success envelope must not carry a header: {body}"
        );
        let rows = parsed
            .get("response")
            .and_then(|r: &Value| r.as_array())
            .expect("v3 envelope must carry a response array");
        assert_eq!(rows.len(), 1, "exactly one row was supplied: {body}");
        let row = &rows[0];
        assert_eq!(
            row.get("symbol").and_then(Value::as_str),
            Some("AAPL"),
            "symbol must round-trip in the response row: {body}"
        );
        assert_eq!(
            row.get("delta").and_then(Value::as_f64),
            Some(0.5),
            "delta must round-trip in the response row: {body}"
        );
        assert!(
            row.get("vega").is_some_and(Value::is_null),
            "non-finite vega must canonicalise to JSON null in the row: {body}"
        );
    }
}
