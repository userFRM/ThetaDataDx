//! Generic endpoint handler and shared-runtime dispatch for all registry endpoints.
//!
//! A single handler function receives endpoint metadata via closure capture,
//! validates query params, calls the endpoint through the shared runtime in
//! `thetadatadx`, and writes the negotiated format from the vendor's table as
//! its chunks arrive.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ops::ControlFlow;

use axum::body::{Body, Bytes};
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use thetadatadx::endpoint::{EndpointArgs, EndpointError, invoke_endpoint_tables};
use thetadatadx::wire::DataValueList;
use thetadatadx::{EndpointMeta, MarketDataClient, ParamType};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;

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
pub(crate) fn deprecated_v2_param_response(params: &HashMap<String, String>) -> Option<Response> {
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
    let body = format::error_envelope(error_type, msg);
    let json_bytes = sonic_rs::to_string(&body).unwrap_or_else(|err| {
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

/// Max query parameter count per request. The widest legitimate endpoint in
/// the registry takes ~10 params (`option_history_trade_quote` + format /
/// pagination knobs); 32 leaves generous headroom without letting a caller
/// allocate ~MB of `HashMap` slack via `?a=1&b=2&...` with thousands of
/// unique 60-byte keys that each pass the 64-byte per-param cap.
pub(crate) const MAX_QUERY_PARAMS: usize = 32;

// ---------------------------------------------------------------------------
//  BoundedQuery — count params BEFORE allocating the HashMap
// ---------------------------------------------------------------------------

/// Axum extractor that parses the raw URI query string into its
/// `(key, value)` pairs while enforcing [`MAX_QUERY_PARAMS`] **during**
/// parsing, not after `serde_urlencoded` has already allocated every pair.
///
/// The pairs keep every occurrence of a repeated key, in request order;
/// [`fold_query_pairs`] resolves repeats against the endpoint's declared
/// parameter types.
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
/// Memory bound during parse: at most `MAX_QUERY_PARAMS` pairs, independent
/// of how long the attacker's query string was. The body / URI limits stay in
/// place via axum's `DefaultBodyLimit` and the URI length limit in `hyper`.
#[derive(Debug)]
pub(crate) struct BoundedQuery<const N: usize>(pub Vec<(String, String)>);

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

        // Now it's safe to parse: the pair count is at most N.
        let params: Vec<(String, String)> =
            serde_urlencoded::from_str(query).map_err(|e| BoundedQueryError {
                status: StatusCode::BAD_REQUEST,
                message: format!("invalid query string: {e}"),
            })?;

        Ok(BoundedQuery(params))
    }
}

/// Resolve the raw query pairs into one value per key, the way the vendor
/// terminal reads a repeated key.
///
/// A list-typed parameter (`symbol` on the snapshot routes) collects every
/// occurrence: `symbol=AAPL&symbol=MSFT` becomes the comma-separated
/// `AAPL,MSFT` the list parser already accepts, which is also how an OpenAPI
/// client serialises an array parameter by default. Any other parameter keeps
/// its first occurrence. The list-typed set is the registry's own
/// `ParamType::Symbols` declaration for this endpoint.
pub(crate) fn fold_query_pairs(
    ep: &EndpointMeta,
    pairs: Vec<(String, String)>,
) -> HashMap<String, String> {
    let mut params: HashMap<String, String> = HashMap::with_capacity(pairs.len());
    for (key, value) in pairs {
        match params.entry(key) {
            Entry::Vacant(slot) => {
                slot.insert(value);
            }
            Entry::Occupied(mut slot) => {
                let is_list = ep.params.iter().any(|param| {
                    param.name == slot.key() && param.param_type == ParamType::Symbols
                });
                if is_list {
                    let joined = slot.get_mut();
                    joined.push(',');
                    joined.push_str(&value);
                }
            }
        }
    }
    params
}

pub(crate) fn build_endpoint_args(
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
    // (`symbol` is a single 16-byte ticker on market-data endpoints but
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
            // The builder sends the endpoint table's default, the terminal's value.
            None => {}
        }
    }
    Ok(args)
}

/// `Retry-After` seconds advertised when the upstream reports
/// `ResourceExhausted` after the SDK's retry budget is spent. Upstream
/// slots free on a millisecond-to-second cadence, so one second is
/// the honest "immediately retryable" hint without inviting a tight
/// hammer loop.
const UPSTREAM_EXHAUSTED_RETRY_AFTER_SECS: u64 = 1;

pub(crate) fn endpoint_error_response(ep: &EndpointMeta, error: EndpointError) -> Response {
    // v3 error bodies are the HTTP status plus a plain-text description (no
    // JSON envelope), so every arm here returns `plain_error_response`.
    match error {
        EndpointError::InvalidParams(message) => {
            plain_error_response(StatusCode::BAD_REQUEST, &message)
        }
        EndpointError::UnknownEndpoint(message) => {
            plain_error_response(StatusCode::NOT_FOUND, &message)
        }
        // The SDK refused the request's own input before or after asking
        // upstream (for example a list request naming several symbols on an
        // endpoint whose plain list cannot attribute its rows): a bad
        // request, not a server fault.
        EndpointError::Server(thetadatadx::Error::Config {
            kind: thetadatadx::ConfigErrorKind::InvalidValue { message, .. },
            ..
        }) => plain_error_response(StatusCode::BAD_REQUEST, &message),
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
            ..
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
        // Any other upstream status is answered the way the vendor terminal
        // answers it: with the HTTP status the service attached in its
        // `http_status_code` trailer (for example its no-data status for a
        // query with no rows), falling back to 500 when there is none, and
        // with the service's description as the body.
        EndpointError::Server(thetadatadx::Error::Grpc {
            kind,
            message,
            http_status_code,
            ..
        }) => {
            let status = http_status_code
                .and_then(|code| StatusCode::from_u16(code).ok())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            if status.is_server_error() {
                tracing::warn!(endpoint = ep.name, %kind, %status, error = %message, "request failed");
            } else {
                tracing::debug!(endpoint = ep.name, %kind, %status, error = %message, "request rejected upstream");
            }
            plain_error_response(status, &message)
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
pub(crate) enum ResponseFormat {
    /// JSON envelope.
    Json,
    /// One array per column, keyed by column name. The shape the vendor
    /// serves accounts opened before its JSON cutover.
    JsonLegacy,
    /// RFC 4180 CSV with a header row.
    Csv,
    /// One JSON object per row, `\n`-delimited.
    Ndjson,
    /// The terminal's HTML page, rendered inline for browser viewing.
    Html,
}

/// Parse the `format` query parameter. Absent means CSV — the v3 spec
/// defaults `format` to `csv` on every path (the vendor terminal's
/// `ResultsFormat.fromString` returns `CSV` for an absent / blank value).
pub(crate) fn parse_response_format(
    params: &HashMap<String, String>,
) -> Result<ResponseFormat, EndpointError> {
    let Some(raw) = params.get("format") else {
        return Ok(ResponseFormat::Csv);
    };
    match raw.to_ascii_lowercase().as_str() {
        // `json_new` is the vendor's name for the current JSON shape and
        // routes to the same serialiser `json` does; `json_legacy` is the
        // column-oriented shape it still serves to accounts opened before
        // the cutover.
        "json" | "json_new" => Ok(ResponseFormat::Json),
        "json_legacy" => Ok(ResponseFormat::JsonLegacy),
        "csv" => Ok(ResponseFormat::Csv),
        // `ndjson` and `jsonl` are the same line-delimited framing under
        // two community names; accept both like the flat-file routes do.
        "ndjson" | "jsonl" => Ok(ResponseFormat::Ndjson),
        // `html` renders the flat rows as a browser-viewable table, inline
        // (no attachment) — the terminal's `&format=html` behaviour.
        "html" => Ok(ResponseFormat::Html),
        other => Err(EndpointError::InvalidParams(format!(
            "unknown format: '{other}' (supported: json, json_new, json_legacy, csv, ndjson, jsonl, html)"
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
pub(crate) fn csv_attachment_filename(
    ep: &EndpointMeta,
    params: &HashMap<String, String>,
) -> String {
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

/// Generic handler invoked for all registry endpoints.
///
/// 1. The [`BoundedQuery`] extractor enforces [`MAX_QUERY_PARAMS`] DURING
///    URL parse so `?a=1&b=2&...` flood attacks can't force a multi-MB
///    HashMap allocation before the cap trips.
/// 2. Validates query params against `EndpointMeta.params` (length caps
///    included — `format` falls under the generic 64-byte cap here).
/// 3. Negotiates the response format (`csv` default, `json`,
///    `ndjson`/`jsonl`, `html`); unknown `format` values are a 400.
/// 4. Invokes the shared endpoint runtime.
/// 5. Renders the negotiated format.
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
    BoundedQuery(pairs): BoundedQuery<MAX_QUERY_PARAMS>,
    ep: &EndpointMeta,
    overrides: &[(&str, String)],
) -> Response {
    let mut params = fold_query_pairs(ep, pairs);
    for (key, value) in overrides {
        params.insert((*key).to_string(), value.clone());
    }
    respond(state, ep, &params).await
}

/// A response that ends within this many bytes is sent whole, with its length.
/// A longer one is sent as it is written, from the moment it outgrows this, so
/// its first rows leave before its last arrive. The terminal frames its
/// responses the same way, at the same size.
const WHOLE_RESPONSE_LIMIT: usize = 8 * 1024;

/// Written chunks held between the upstream and a slower client. One entry is
/// one upstream chunk, so this bounds what a response holds in memory.
pub(crate) const CHUNKS_IN_FLIGHT: usize = 4;

pub(crate) type Chunk = Result<Bytes, std::io::Error>;

/// How a response begins.
enum Start {
    /// The whole body, which ended within [`WHOLE_RESPONSE_LIMIT`].
    Whole(Vec<u8>),
    /// The body follows as it is written.
    Streamed,
}

/// Answer one registry request from its folded query parameters: validate
/// them, call the endpoint through `client`, and write the negotiated format
/// from the vendor's table as its chunks arrive.
///
/// The status waits until the body has either ended or outgrown
/// [`WHOLE_RESPONSE_LIMIT`], so a refusal or a failure before then answers
/// with its own status. A failure after the body has begun aborts it rather
/// than ending it, so a client cannot mistake a truncated response for a
/// complete one.
pub(crate) async fn respond<C>(
    client: C,
    ep: &EndpointMeta,
    params: &HashMap<String, String>,
) -> Response
where
    C: AsRef<MarketDataClient> + Send + Sync + 'static,
{
    // v2 → v3 parameter migration gate. A request carrying a deprecated v2
    // parameter name is rejected with `410 Gone` naming the v3 replacement,
    // matching the terminal's pre-matching filter. Runs before any
    // validation / dispatch so a v2 client gets the upgrade message rather
    // than an opaque missing-parameter error from a name v3 does not know.
    if let Some(response) = deprecated_v2_param_response(params) {
        return response;
    }

    let args = match build_endpoint_args(ep, params) {
        Ok(args) => args,
        Err(error) => return endpoint_error_response(ep, error),
    };

    // Format negotiation runs AFTER `build_endpoint_args` on purpose:
    // the length validators in there cap `format` at the generic
    // 64-byte bound, so the lowercase copy and the echoed-value 400
    // below can never amplify an oversized query value.
    let response_format = match parse_response_format(params) {
        Ok(f) => f,
        Err(error) => return endpoint_error_response(ep, error),
    };

    let table = format::TableWriter::new(
        ep,
        response_format,
        params.get("symbol").map(String::as_str),
    );
    let (chunks, body) = mpsc::channel(CHUNKS_IN_FLIGHT);
    let (started, start) = oneshot::channel();
    tokio::spawn(write_body(client, ep.name, args, table, chunks, started));
    let body = match start.await {
        Ok(Ok(Start::Whole(body))) => Body::from(body),
        Ok(Ok(Start::Streamed)) => Body::from_stream(ReceiverStream::new(body)),
        Ok(Err(error)) => return endpoint_error_response(ep, error),
        Err(_) => {
            return plain_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the response ended before it began",
            );
        }
    };
    match response_format {
        ResponseFormat::Csv => {
            let disposition = format!(
                "attachment; filename=\"{}\"",
                csv_attachment_filename(ep, params)
            );
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                    (header::CONTENT_DISPOSITION, disposition.as_str()),
                ],
                body,
            )
                .into_response()
        }
        ResponseFormat::Json | ResponseFormat::JsonLegacy => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, JSON_CONTENT_TYPE)],
            body,
        )
            .into_response(),
        ResponseFormat::Ndjson => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, NDJSON_CONTENT_TYPE)],
            body,
        )
            .into_response(),
        // Inline, browser-viewable: no content-disposition, so the table
        // renders in the tab rather than downloading.
        ResponseFormat::Html => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response(),
    }
}

/// Call `endpoint` and write its response, reporting on `started` how the
/// response begins and sending the rest of a streamed body into `chunks`.
async fn write_body(
    client: impl AsRef<MarketDataClient>,
    endpoint: &'static str,
    args: EndpointArgs,
    table: format::TableWriter,
    chunks: mpsc::Sender<Chunk>,
    started: oneshot::Sender<Result<Start, EndpointError>>,
) {
    let mut body = BodyWriter {
        endpoint,
        table,
        held: Vec::new(),
        chunks: chunks.clone(),
        started: Some(started),
        failed: false,
    };
    let call = invoke_endpoint_tables(client.as_ref(), endpoint, &args, |headers, rows| {
        body.chunk(&headers, &rows)
    });
    // The call ends, and is cancelled upstream, as soon as the client stops
    // waiting for the response or reading it.
    let outcome = tokio::select! {
        outcome = call => outcome,
        () = chunks.closed() => Ok(()),
    };
    body.finish(outcome).await;
}

/// Writes one response's body: held while it may still end within
/// [`WHOLE_RESPONSE_LIMIT`], sent on chunk by chunk once it cannot.
struct BodyWriter {
    endpoint: &'static str,
    table: format::TableWriter,
    /// The body so far, until the response begins.
    held: Vec<u8>,
    chunks: mpsc::Sender<Chunk>,
    /// Present until the response begins.
    started: Option<oneshot::Sender<Result<Start, EndpointError>>>,
    failed: bool,
}

impl BodyWriter {
    /// Write one chunk's rows and pass them on. Breaks the call once the
    /// client has gone, or on a cell the response cannot carry.
    fn chunk(
        &mut self,
        headers: &[String],
        rows: &[DataValueList],
    ) -> impl Future<Output = ControlFlow<()>> + Send + 'static + use<> {
        let mut piece = Vec::new();
        let out = if self.started.is_some() {
            &mut self.held
        } else {
            &mut piece
        };
        let chunk = match self.table.write(headers, rows, out) {
            Ok(()) => self.pass_on(piece, false),
            Err(error) => self.fail(thetadatadx::Error::from(error).into()),
        };
        send_chunk(&self.chunks, chunk, self.failed)
    }

    /// Close the body once the call has ended with `outcome`.
    async fn finish(mut self, outcome: Result<(), EndpointError>) {
        if self.failed {
            return;
        }
        let chunk = match outcome {
            Ok(()) => {
                let mut piece = Vec::new();
                let out = if self.started.is_some() {
                    &mut self.held
                } else {
                    &mut piece
                };
                self.table.finish(out);
                self.pass_on(piece, true)
            }
            Err(error) => self.fail(error),
        };
        if let Some(chunk) = chunk {
            let _ = self.chunks.send(chunk).await;
        }
    }

    /// What to send of the body just written, to `piece` once the response
    /// has begun and to the held body before. The response begins streamed
    /// once the held body outgrows [`WHOLE_RESPONSE_LIMIT`], and whole when it
    /// is `last` within it.
    fn pass_on(&mut self, piece: Vec<u8>, last: bool) -> Option<Chunk> {
        let outgrown = self.held.len() > WHOLE_RESPONSE_LIMIT;
        match self.started.take() {
            None => (!piece.is_empty()).then(|| Ok(Bytes::from(piece))),
            Some(started) if outgrown => {
                let _ = started.send(Ok(Start::Streamed));
                Some(Ok(Bytes::from(std::mem::take(&mut self.held))))
            }
            Some(started) if last => {
                let _ = started.send(Ok(Start::Whole(std::mem::take(&mut self.held))));
                None
            }
            started => {
                self.started = started;
                None
            }
        }
    }

    /// Report `error`: as the response itself while it has not begun, by
    /// aborting the body once it has.
    fn fail(&mut self, error: EndpointError) -> Option<Chunk> {
        self.failed = true;
        match self.started.take() {
            Some(started) => {
                let _ = started.send(Err(error));
                None
            }
            None => {
                tracing::warn!(endpoint = self.endpoint, %error, "response aborted");
                Some(Err(std::io::Error::other(error.to_string())))
            }
        }
    }
}

/// Send `chunk`, if there is one, into the response body. The call goes on
/// while the client still reads and nothing has `failed`.
pub(crate) fn send_chunk(
    chunks: &mpsc::Sender<Chunk>,
    chunk: Option<Chunk>,
    failed: bool,
) -> impl Future<Output = ControlFlow<()>> + Send + 'static + use<> {
    let chunks = chunks.clone();
    async move {
        let sent = match chunk {
            Some(chunk) => chunks.send(chunk).await.is_ok(),
            None => true,
        };
        if sent && !failed {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
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
/// terminal's one-word `text/plain` body (`CONNECTED` / `UNVERIFIED` /
/// `DISCONNECTED` / `ERROR`).
pub async fn terminal_streaming_status(State(state): State<AppState>) -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, TEXT_PLAIN_CONTENT_TYPE)],
        state.fpss_status(),
    )
        .into_response()
}

/// GET /v3/terminal/mdds/status -- MDDS (market-data) channel health as the
/// terminal's one-word `text/plain` body (`CONNECTED` / `DISCONNECTED`).
pub async fn terminal_market_data_status(State(state): State<AppState>) -> Response {
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
    use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
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
    /// dispatch must not loosen the market-data surface.
    #[test]
    fn build_endpoint_args_keeps_single_symbol_cap_on_market_data_endpoints() {
        let ep = thetadatadx::find("stock_history_eod")
            .expect("market-data endpoint must exist in the registry");
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

    /// Intraday history endpoints carry an optional `date` plus optional
    /// `start_date`/`end_date`, exactly as the terminal serves them: a
    /// single URL accepts either a single-day query or a range query.
    #[test]
    fn intraday_history_accepts_single_date_query() {
        let ep = thetadatadx::find("stock_history_ohlc").expect("endpoint exists");
        let params = string_params(&[("symbol", "AMZN"), ("date", "20260603"), ("interval", "1m")]);
        build_endpoint_args(ep, &params).expect("single-date query must be accepted");
    }

    #[test]
    fn intraday_history_accepts_range_query_on_base_route() {
        // No `date`, only `start_date`+`end_date` — the terminal serves
        // this on the base route; there is no separate `_range` route.
        let ep = thetadatadx::find("stock_history_ohlc").expect("endpoint exists");
        let params = string_params(&[
            ("symbol", "AMZN"),
            ("start_date", "20260603"),
            ("end_date", "20260605"),
            ("interval", "1m"),
        ]);
        build_endpoint_args(ep, &params).expect("range query must be accepted on the base route");
    }

    /// The same holds for every intraday history endpoint, not just OHLC —
    /// `trade` carries the same optional-date/optional-range shape.
    #[test]
    fn intraday_trade_history_accepts_range_query() {
        let ep = thetadatadx::find("stock_history_trade").expect("endpoint exists");
        let params = string_params(&[
            ("symbol", "AMZN"),
            ("start_date", "20260603"),
            ("end_date", "20260605"),
        ]);
        build_endpoint_args(ep, &params).expect("range query must be accepted");
    }

    /// The fabricated `_range` route no longer exists in the registry.
    #[test]
    fn no_fabricated_ohlc_range_endpoint() {
        assert!(
            thetadatadx::find("stock_history_ohlc_range").is_none(),
            "the terminal has no ohlc_range route; it must not exist in the registry"
        );
    }

    /// EOD endpoints require an explicit range (`start_date`/`end_date`),
    /// matching the terminal — there is no single-`date` shortcut there.
    #[test]
    fn eod_history_requires_range() {
        let ep = thetadatadx::find("stock_history_eod").expect("endpoint exists");
        let params = string_params(&[
            ("symbol", "AAPL"),
            ("start_date", "20260101"),
            ("end_date", "20260301"),
        ]);
        build_endpoint_args(ep, &params).expect("range query must be accepted");
    }

    // -----------------------------------------------------------------------
    //  BoundedQuery — cap enforced DURING parse, not after HashMap alloc
    // -----------------------------------------------------------------------

    async fn run_bounded_query<const N: usize>(
        query: &str,
    ) -> Result<Vec<(String, String)>, BoundedQueryError> {
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
        // Realistic 4-param request: must parse into its pairs exactly.
        let params = run_bounded_query::<{ MAX_QUERY_PARAMS }>(
            "symbol=AAPL&start_date=20240101&end_date=20240201&format=json",
        )
        .await
        .expect("normal query must parse");
        let expected = [
            ("symbol", "AAPL"),
            ("start_date", "20240101"),
            ("end_date", "20240201"),
            ("format", "json"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        assert_eq!(params, expected);
    }

    /// A repeated key resolves the way the terminal reads it: the snapshot
    /// routes' list-typed `symbol` collects every occurrence (the default
    /// OpenAPI serialisation of an array parameter), and any other parameter
    /// keeps its first occurrence. A last-wins map silently dropped `AAPL`.
    #[tokio::test]
    async fn repeated_query_keys_fold_like_the_terminal() {
        let ep = thetadatadx::find("stock_snapshot_quote").expect("endpoint exists");
        let pairs = run_bounded_query::<{ MAX_QUERY_PARAMS }>(
            "symbol=AAPL&venue=nqb&symbol=MSFT&format=json&venue=utp_cta&format=csv",
        )
        .await
        .expect("repeated keys must parse");
        let params = fold_query_pairs(ep, pairs);
        assert_eq!(params["symbol"], "AAPL,MSFT");
        assert_eq!(params["venue"], "nqb");
        assert_eq!(params["format"], "json");
    }

    /// Read the body of an axum `Response` to a `String`.
    async fn read_body(resp: Response) -> String {
        use axum::body::to_bytes;
        let body = resp.into_body();
        let bytes = to_bytes(body, usize::MAX).await.expect("body collect");
        String::from_utf8(bytes.to_vec()).expect("body utf8")
    }

    // -----------------------------------------------------------------------
    //  Response-format negotiation
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
            // The vendor's two JSON names: `json_new` is the current shape
            // under its own spelling, `json_legacy` the column-oriented one
            // it still serves. Rejecting either turned a request the vendor
            // answers into a 400.
            ("json_new", ResponseFormat::Json),
            ("JSON_NEW", ResponseFormat::Json),
            ("json_legacy", ResponseFormat::JsonLegacy),
            ("JSON_LEGACY", ResponseFormat::JsonLegacy),
            ("csv", ResponseFormat::Csv),
            ("CSV", ResponseFormat::Csv),
            ("ndjson", ResponseFormat::Ndjson),
            ("NDJSON", ResponseFormat::Ndjson),
            ("jsonl", ResponseFormat::Ndjson),
            ("JSONL", ResponseFormat::Ndjson),
            ("html", ResponseFormat::Html),
            ("HTML", ResponseFormat::Html),
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
                    for supported in [
                        "json",
                        "json_new",
                        "json_legacy",
                        "csv",
                        "ndjson",
                        "jsonl",
                        "html",
                    ] {
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
                http_status_code: None,
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

    /// Every other gRPC status is answered with the HTTP status the service
    /// attached in its `http_status_code` trailer, as the terminal does, and
    /// with the service's description verbatim as the body. Without a usable
    /// trailer the answer is 500. Only the capacity condition is
    /// retry-hinted.
    #[tokio::test]
    async fn grpc_faults_answer_with_the_upstream_http_status() {
        use thetadatadx::GrpcStatusKind;
        for (kind, http_status_code, expected) in [
            (GrpcStatusKind::NotFound, Some(472), 472),
            (GrpcStatusKind::InvalidArgument, Some(400), 400),
            (GrpcStatusKind::Internal, None, 500),
            (GrpcStatusKind::PermissionDenied, Some(0), 500),
        ] {
            let resp = endpoint_error_response(
                any_endpoint(),
                EndpointError::Server(thetadatadx::Error::Grpc {
                    kind,
                    message: "upstream description".to_string(),
                    retry_after: None,
                    http_status_code,
                }),
            );
            assert_eq!(resp.status().as_u16(), expected, "{kind:?}");
            assert!(
                resp.headers()
                    .get(axum::http::header::RETRY_AFTER)
                    .is_none()
            );
            assert_eq!(read_body(resp).await, "upstream description", "{kind:?}");
        }
    }

    /// v3 registry / data errors are a plain-text body at the right status —
    /// no JSON envelope, `text/plain` content type, the message verbatim.
    /// A request the SDK itself refuses is a 400 like one the registry
    /// refuses, not a server fault.
    #[tokio::test]
    async fn endpoint_invalid_params_emits_plain_text_body() {
        for error in [
            EndpointError::InvalidParams("missing required parameter: 'date'".to_string()),
            EndpointError::Server(thetadatadx::Error::config_invalid(
                "endpoint.params",
                "missing required parameter: 'date'",
            )),
        ] {
            let resp = endpoint_error_response(any_endpoint(), error);
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

    /// The status waits until the body has ended or outgrown
    /// `WHOLE_RESPONSE_LIMIT`: an upstream failure before then answers with
    /// its own status, and one after it aborts the body already begun, so a
    /// truncated response never reads as a complete one.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failure_answers_with_its_status_until_the_body_has_begun() {
        use crate::golden_responses::mock;
        use prost::Message as _;
        use std::sync::Arc;
        use thetadatadx::grpc::{Channel, ChannelPool};
        use thetadatadx::wire::{
            CompressionAlgo, CompressionDescription, DataTable, DataValue, ResponseData,
            data_value::DataType,
        };
        use tokio_stream::StreamExt as _;

        let ep = thetadatadx::find("stock_list_symbols").expect("registered endpoint");
        // `symbol\r\n` and seven bytes a row: 78 bytes, then 14,008.
        for (symbols, begun) in [(10, false), (2_000, true)] {
            let table = DataTable {
                headers: vec!["symbol".into()],
                data_table: (0..symbols)
                    .map(|i| DataValueList {
                        values: vec![DataValue {
                            data_type: Some(DataType::Text(format!("S{i:04}"))),
                        }],
                    })
                    .collect(),
            };
            let compressed_data = table.encode_to_vec();
            let chunk = ResponseData {
                compression_description: Some(CompressionDescription {
                    algo: CompressionAlgo::None as i32,
                    level: 0,
                }),
                original_size: i32::try_from(compressed_data.len()).expect("small table"),
                compressed_data,
            };
            // The rows, then a failure with no status of the service's own.
            let upstream =
                mock::MockServer::spawn_with_message(vec![chunk], 13, "upstream fault".into())
                    .await;
            let channel = Channel::connect_h2c("127.0.0.1", upstream.addr.port())
                .await
                .expect("connect to the mock");
            let client = Arc::new(MarketDataClient::for_endpoint_routing_test(
                thetadatadx::DirectConfig::production(),
                ChannelPool::from_channels(vec![channel]),
                Arc::new(tokio::sync::Semaphore::new(1)),
            ));

            let response = respond(client, ep, &HashMap::new()).await;
            if !begun {
                assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(read_body(response).await, "upstream fault");
                continue;
            }
            assert_eq!(response.status(), StatusCode::OK);
            let mut body = response.into_body().into_data_stream();
            let mut received = Vec::new();
            let aborted = loop {
                match body.next().await {
                    Some(Ok(bytes)) => received.extend_from_slice(&bytes),
                    Some(Err(_)) => break true,
                    None => break false,
                }
            };
            assert!(aborted, "the body ended as though complete");
            assert!(
                received.starts_with(b"symbol\r\nS0000\r\n"),
                "the rows written before the failure are sent"
            );
        }
    }
}
