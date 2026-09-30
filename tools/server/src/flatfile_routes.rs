//! Hand-written flat-file route surface for the REST server.
//!
//! Flat files are server-pre-built whole-universe daily blobs. They are a
//! one-shot batch download — NOT a WebSocket subscription stream — so the
//! route surface is HTTP-only by design. Streaming the bytes back to the
//! client uses `axum::body::Body` over a tokio file reader so the server
//! doesn't pin a multi-hundred-MB response in RAM.
//!
//! Route:
//!
//! - `GET /v3/{sec_type}/flat_file/{req_type}` — the terminal's flat-file
//!   path form. Path segments parse case-insensitively to the matching
//!   `SecType` / `ReqType`. Query params:
//!   `date=YYYY-MM-DD|YYYYMMDD&format=csv|json|ndjson|jsonl|html`. The pair
//!   must be a served flat-file dataset — option `trade_quote` /
//!   `open_interest` / `eod`, stock `trade_quote` / `eod`; any
//!   other `(sec_type, req_type)` pair returns `400 bad_request`.
//!
//! Every format is written row-by-row by its `RowSink`, so even a JSON
//! array or an HTML table streams without buffering the whole blob: `csv`,
//! `json` (a single JSON array), `ndjson` / `jsonl` (newline-delimited
//! JSON), and `html` (an HTML table).
//!
//! Response:
//! - `Content-Type: text/csv` (csv), `application/json` (json),
//!   `application/x-ndjson` (ndjson / jsonl), or `text/html` (html).
//! - Body is the file bytes streamed via `tokio_util::io::ReaderStream`.
//! - On failure: standard error envelope (`error_type`, `error_msg`).
//!
//! Security note: the server requires authenticated access to ThetaData
//! servers. The MDDS-flat-file path inherits the same `AppState`
//! credentials as the per-endpoint surface; per-IP rate-limiting from
//! `router::build` applies here too.

use std::path::PathBuf;

use axum::body::Body;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use thetadatadx::flatfiles::{
    flat_file_serves, FlatFileFormat, FlatFilesUnavailableReason, ReqType, SecType,
};
use tokio_util::io::ReaderStream;

use crate::handler::error_response;
use crate::state::AppState;

/// Query parameters for the flat-file route
/// (`GET /v3/{sec_type}/flat_file/{req_type}`).
#[derive(Debug, Deserialize)]
pub(crate) struct FlatfileQuery {
    /// Trading date in `YYYY-MM-DD` or `YYYYMMDD` form.
    pub date: String,
    /// On-disk format: `csv` (default), `json`, `ndjson` (or its `jsonl`
    /// alias), or `html`.
    #[serde(default)]
    pub format: Option<String>,
}

/// Define a `fn $name(&$rej) -> Response` that maps an axum extractor
/// rejection onto the server's canonical error envelope.
///
/// Every rejection carries its own status (a malformed body is `400`, a
/// missing `Content-Type` is `415`, a bad path segment is `400`) and a
/// diagnostic `body_text()`; `bad_request` is the canonical `error_type`
/// for any client-supplied input the server could not accept. Each mapper
/// is pure over the rejection so the wire shape is testable without driving
/// a live request through the router. `status()` / `body_text()` are inherent
/// methods on each axum rejection (no shared trait), so this generates one
/// named mapper per rejection type rather than a single generic function.
macro_rules! rejection_response_fn {
    ($name:ident, $rej:ty) => {
        fn $name(rejection: &$rej) -> Response {
            error_response(rejection.status(), "bad_request", &rejection.body_text())
        }
    };
}

// ── Query extractor with a canonical-envelope rejection ──────────────────

/// `axum::Query` wrapper whose extraction failure renders the server's
/// canonical error envelope instead of axum's default plain-text 400.
///
/// The convenience GET route carries its `date` (required) and `format`
/// in the query string. A request with a missing `date` or an otherwise
/// malformed query string fails deserialization in the extractor, before
/// the handler body runs. With the stock `Query` extractor that surfaces
/// as a bare text `400`, diverging from the canonical
/// `{"header":{"error_type":...,"error_msg":...},"response":[]}` envelope
/// that the POST sibling and every other route family return. Clients
/// drive retry / backoff off `header.error_type`, so the GET path must
/// fail the same shape as the POST path. This extractor maps the
/// `QueryRejection` onto `error_response` and otherwise behaves exactly
/// like `axum::Query<T>`.
pub(crate) struct FlatfileQueryExtractor<T>(pub(crate) T);

impl<S, T> FromRequestParts<S> for FlatfileQueryExtractor<T>
where
    axum::extract::Query<T>: FromRequestParts<S, Rejection = QueryRejection>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(value)) => Ok(Self(value)),
            Err(rejection) => Err(query_rejection_response(&rejection)),
        }
    }
}

// A query string the server could not deserialize (missing required `date`,
// malformed key/value) is a client-supplied input fault: `400`,
// `error_type="bad_request"`.
rejection_response_fn!(query_rejection_response, QueryRejection);

// The brace route matches a non-UTF-8 percent-encoded segment (`%ff`);
// `Path::<(String, String)>` then fails `decode_utf8()` and rejects. Without
// this the GET path answers axum's default plain-text 400 (`Invalid UTF-8 in
// \`sec_type\``), diverging from the canonical envelope the query and POST
// siblings return — the contract clients drive retry / backoff off.
rejection_response_fn!(path_rejection_response, PathRejection);

// ── Enum parsing ─────────────────────────────────────────────────────────

fn parse_sec_type(s: &str) -> Result<SecType, String> {
    match s.to_ascii_uppercase().as_str() {
        "OPTION" => Ok(SecType::Option),
        "STOCK" => Ok(SecType::Stock),
        "INDEX" => Ok(SecType::Index),
        other => Err(format!("unknown sec_type: {other}")),
    }
}

fn parse_req_type(s: &str) -> Result<ReqType, String> {
    match s.to_ascii_uppercase().as_str() {
        "EOD" => Ok(ReqType::Eod),
        "QUOTE" => Ok(ReqType::Quote),
        "OPEN_INTEREST" | "OPENINTEREST" => Ok(ReqType::OpenInterest),
        "OHLC" => Ok(ReqType::Ohlc),
        "TRADE" => Ok(ReqType::Trade),
        "TRADE_QUOTE" | "TRADEQUOTE" => Ok(ReqType::TradeQuote),
        other => Err(format!("unknown req_type: {other}")),
    }
}

fn parse_format(value: Option<&str>) -> Result<FlatFileFormat, String> {
    match value.unwrap_or("csv").to_ascii_lowercase().as_str() {
        "csv" => Ok(FlatFileFormat::Csv),
        // `ndjson` and `jsonl` are the same line-delimited framing under two
        // names; both stream as `application/x-ndjson`.
        "ndjson" | "jsonl" => Ok(FlatFileFormat::Jsonl),
        // `json` streams a single JSON array; `html` an HTML table. Both are
        // written row-by-row by their `RowSink`, so neither buffers the whole
        // daily blob.
        "json" => Ok(FlatFileFormat::Json),
        "html" => Ok(FlatFileFormat::Html),
        other => Err(format!(
            "unknown flat-file format: {other:?} (supported: csv, json, ndjson, jsonl, html)"
        )),
    }
}

/// Reject an `(sec_type, req_type)` pair the flat-file distribution does not
/// serve, at the route boundary, before any temp-path or upstream work.
///
/// The route surface parses the security and request types case-insensitively
/// to their variants; this guard then constrains the pair to the served matrix
/// (option `trade_quote` / `open_interest` / `eod`, stock `trade_quote` /
/// `eod`) so an impossible combination — e.g. `INDEX`, or stock
/// `open_interest` — fails as a `400 bad_request` here rather than reaching the
/// SDK gate. The served matrix is the single source of truth.
fn reject_unserved_dataset(sec_type: SecType, req_type: ReqType) -> Result<(), String> {
    if flat_file_serves(sec_type, req_type) {
        Ok(())
    } else {
        Err(format!(
            "flat-file service does not serve {sec_type} {}",
            req_type.as_str()
        ))
    }
}

fn content_type_for(format: FlatFileFormat) -> &'static str {
    match format {
        FlatFileFormat::Csv => "text/csv; charset=utf-8",
        // application/x-ndjson is the standard MIME for JSON Lines blobs.
        FlatFileFormat::Jsonl => "application/x-ndjson; charset=utf-8",
        FlatFileFormat::Json => "application/json; charset=utf-8",
        FlatFileFormat::Html => "text/html; charset=utf-8",
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────

async fn handle_get(
    state: State<AppState>,
    // A non-UTF-8 percent-encoded segment (`%ff`) matches the brace route but
    // fails `Path`'s `decode_utf8()`, so the rejection is reachable. Take the
    // `Result` and route the failure through `error_response` so it answers
    // the canonical JSON envelope, matching the query sibling — instead of
    // axum's default plain-text 400.
    path: Result<Path<(String, String)>, PathRejection>,
    FlatfileQueryExtractor(params): FlatfileQueryExtractor<FlatfileQuery>,
) -> Response {
    let Path((sec_type_s, req_type_s)) = match path {
        Ok(p) => p,
        Err(rej) => return path_rejection_response(&rej),
    };
    let sec_type = match parse_sec_type(&sec_type_s) {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "bad_request", &e),
    };
    let req_type = match parse_req_type(&req_type_s) {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "bad_request", &e),
    };
    let format = match parse_format(params.format.as_deref()) {
        Ok(f) => f,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "bad_request", &e),
    };
    // Validate `date` at the boundary, before it is interpolated into a
    // temp PathBuf in `flatfile_paths`. Path safety must not depend solely
    // on the downstream format validator rejecting non-YYYYMMDD first.
    if let Err(e) = crate::validation::validate_date(&params.date, "date") {
        return error_response(StatusCode::BAD_REQUEST, "bad_request", &e.message);
    }
    if let Err(e) = reject_unserved_dataset(sec_type, req_type) {
        return error_response(StatusCode::BAD_REQUEST, "bad_request", &e);
    }
    // The v3 `date` param is documented `format: date` (dashed `YYYY-MM-DD`),
    // but the driver requires exactly 8 digits — normalise a well-formed
    // dashed date to `YYYYMMDD` here so both spellings reach it. Any other
    // shape passes through for the driver to reject.
    let date = normalize_flatfile_date(&params.date);
    serve_flatfile(state, sec_type, req_type, &date, format).await
}

/// Normalise a well-formed `YYYY-MM-DD` date to the `YYYYMMDD` the flat-file
/// driver requires; any other input passes through unchanged for the driver
/// to accept or reject.
fn normalize_flatfile_date(date: &str) -> String {
    let b = date.as_bytes();
    let dashed = b.len() == 10
        && b.iter().enumerate().all(|(i, &c)| {
            if i == 4 || i == 7 {
                c == b'-'
            } else {
                c.is_ascii_digit()
            }
        });
    if dashed {
        format!("{}{}{}", &date[0..4], &date[5..7], &date[8..10])
    } else {
        date.to_owned()
    }
}

/// Map an SDK flat-file error onto the HTTP `(status, error_type)` the
/// route answers with.
///
/// Three classes, each with a distinct status so a client can tell them
/// apart from the response code alone rather than parsing the message:
///
/// * **400 `bad_request`** — the request never reached the upstream. The
///   SDK's local dataset gate rejected an unserved `(sec_type, req_type)`
///   pair (a typed invalid-parameter [`thetadatadx::Error::Config`]).
/// * **404 `flatfiles_no_data`** — the upstream answered, but no flat file
///   is available to this account for the requested `date`: the daily
///   snapshot has not been generated yet, or the account's history
///   entitlement does not cover that date (the upstream returns a
///   `PERMISSION` rejection naming the first accessible date). This is a
///   normal "nothing here for you" outcome, not a gateway failure — a 502
///   would wrongly signal the upstream is broken and invite a retry that
///   cannot succeed.
/// * **502 `flatfiles_unavailable`** — a genuine upstream/transport fault
///   (mid-stream truncation, auth rejection, connection drop). The upstream
///   is the failing dependency, so `Bad Gateway` is the honest status.
fn classify_flatfile_error(e: &thetadatadx::Error) -> (StatusCode, &'static str) {
    match e {
        thetadatadx::Error::Config { kind, .. } if kind.is_invalid_parameter() => {
            (StatusCode::BAD_REQUEST, "bad_request")
        }
        // No-data arrives two ways: an `ERROR` frame whose diagnostic names
        // the reason (`RequestRejected`, message-classified), or a
        // connection-scoped `DISCONNECTED` whose `RemoveReason` is a no-data
        // code such as `NoStartDate` (`AuthRejected`, reason-classified by
        // the SDK). Both are "nothing here for this account/date", not a
        // gateway fault, so both map to 404 — otherwise the same no-data
        // condition answers 404 via one frame type and 502 via the other.
        thetadatadx::Error::FlatFilesUnavailable(FlatFilesUnavailableReason::RequestRejected {
            server_message,
        }) if rejection_is_no_data(server_message) => (StatusCode::NOT_FOUND, "flatfiles_no_data"),
        thetadatadx::Error::FlatFilesUnavailable(reason) if reason.is_no_data() => {
            (StatusCode::NOT_FOUND, "flatfiles_no_data")
        }
        _ => (StatusCode::BAD_GATEWAY, "flatfiles_unavailable"),
    }
}

/// Whether an upstream `RequestRejected` message describes a "no flat file
/// available for this account/date" condition rather than a transport
/// fault.
///
/// The upstream tags these with a leading reason token. `PERMISSION:`
/// covers the history-entitlement boundary (`"Invalid permissions for
/// date. Your first access date is: YYYYMMDD ..."`); `NO_START_DATE`
/// covers a date with no generated snapshot. Matching the tagged prefix
/// keeps the classifier from sweeping a genuinely malformed-request
/// rejection (`INVALID_PARAMS`) into the no-data bucket — that stays a 502
/// so it is not silently masked as an empty result.
///
/// The leading reason is a tagged prefix, so the match is anchored on the
/// prefix rather than a whitespace/`:`-split first token: a leading space
/// would split to an empty token (wrongly 502), and prose like
/// `"PERMISSION denied"` would match a bare `PERMISSION` token (wrongly
/// 404). `trim()` then `starts_with` the exact `PERMISSION:` /
/// `NO_START_DATE` tags avoids both.
fn rejection_is_no_data(server_message: &str) -> bool {
    let trimmed = server_message.trim();
    trimmed.starts_with("PERMISSION:") || trimmed.starts_with("NO_START_DATE")
}

async fn serve_flatfile(
    state: State<AppState>,
    sec_type: SecType,
    req_type: ReqType,
    date: &str,
    format: FlatFileFormat,
) -> Response {
    // Pull and decode into a per-request scratch file, open it, and
    // unlink it before streaming it back through tokio_util's
    // ReaderStream, so even multi-GB files never pin server memory. The
    // open handle keeps the data readable until the body finishes or the
    // client disconnects, and the OS reclaims the space then; nothing is
    // left in the temp directory. The scratch name carries a per-request
    // random suffix, so concurrent requests for the same slice never
    // share or remove each other's file.
    let (scratch_path, filename) = flatfile_paths(sec_type, req_type, date, format);

    let written_scratch = match state
        .client()
        .flatfile_request(sec_type, req_type, date, &scratch_path, format)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            let _ = tokio::fs::remove_file(&scratch_path).await;
            let (status, error_type) = classify_flatfile_error(&e);
            return error_response(status, error_type, &e.to_string());
        }
    };

    // Honour whatever path the SDK returned.
    let opened = tokio::fs::File::open(&written_scratch).await;
    let _ = tokio::fs::remove_file(&written_scratch).await;
    let file = match opened {
        Ok(f) => f,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                &format!("failed to open written flatfile: {e}"),
            );
        }
    };
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type_for(format))
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(body)
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "flatfile response build failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "response build failed",
            )
        })
}

/// Add the flat-file route onto an existing axum router.
///
/// `/v3/{sec_type}/flat_file/{req_type}` — the v3 form the JVM terminal serves
/// (e.g. `/v3/option/flat_file/trade_quote`). The terminal carries `date` +
/// `format` in the query string and `sec_type` / `req_type` in the path. An
/// unserved `(sec_type, req_type)` pair (stock `open_interest`, any non-EOD
/// `index`) fails the served-matrix gate as a `400 bad_request`.
pub(crate) fn add_flatfile_routes(router: Router<AppState>) -> Router<AppState> {
    router.route("/v3/{sec_type}/flat_file/{req_type}", get(handle_get))
}

/// Compute the `(scratch_path, filename)` pair for a flatfile request.
///
/// `filename` is the attachment name, deterministic per `(sec_type,
/// req_type, date, format)`. `scratch_path` is unique per request via a
/// random suffix, so a request only ever writes and unlinks its own file.
fn flatfile_paths(
    sec_type: SecType,
    req_type: ReqType,
    date: &str,
    format: FlatFileFormat,
) -> (PathBuf, String) {
    let filename = format!(
        "thetadatadx_server_flatfile_{sec_type}_{}_{date}.{}",
        req_type as u32,
        format.extension(),
    );
    let scratch_path =
        std::env::temp_dir().join(format!("{filename}.{}.partial", crate::random_hex_token()));
    (scratch_path, filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every documented flat-file `format` token maps to its variant and
    /// content type; an unknown token is a `400` whose message lists the
    /// supported set. `json` and `html` are first-class formats, not
    /// aliases — the JSON array and HTML table stream row-by-row.
    #[test]
    fn parse_format_accepts_every_documented_token() {
        for (token, want, ctype) in [
            ("csv", FlatFileFormat::Csv, "text/csv; charset=utf-8"),
            (
                "json",
                FlatFileFormat::Json,
                "application/json; charset=utf-8",
            ),
            (
                "ndjson",
                FlatFileFormat::Jsonl,
                "application/x-ndjson; charset=utf-8",
            ),
            (
                "jsonl",
                FlatFileFormat::Jsonl,
                "application/x-ndjson; charset=utf-8",
            ),
            ("html", FlatFileFormat::Html, "text/html; charset=utf-8"),
            ("HTML", FlatFileFormat::Html, "text/html; charset=utf-8"),
        ] {
            let got = parse_format(Some(token)).expect("documented token must parse");
            assert_eq!(got, want, "token {token:?} must map to {want:?}");
            assert_eq!(content_type_for(got), ctype, "content type for {token:?}");
        }
        // Absent `format` defaults to csv.
        assert_eq!(parse_format(None).unwrap(), FlatFileFormat::Csv);
        // Unknown token is a 400 that names the supported set.
        let err = parse_format(Some("parquet")).expect_err("unknown token must reject");
        assert!(
            err.contains("csv, json, ndjson, jsonl, html"),
            "rejection must list the supported formats; got {err:?}"
        );
    }

    /// An upstream history-entitlement / no-snapshot rejection is a normal
    /// "nothing here for this account/date" outcome, not a gateway failure.
    /// It must map to `404 flatfiles_no_data` so a client distinguishes it
    /// from a real upstream outage by status code alone — the boundary the
    /// confusing `502 unexpected response id=-1` previously erased.
    #[test]
    fn no_data_rejection_maps_to_404() {
        for msg in [
            "PERMISSION:Invalid permissions for date. Your first access date is: 20260618 \
             for flat files. Reach out to sales to inquire about deeper history.",
            "NO_START_DATE:no flat file generated for the requested date",
        ] {
            let e = thetadatadx::Error::FlatFilesUnavailable(
                FlatFilesUnavailableReason::RequestRejected {
                    server_message: msg.to_string(),
                },
            );
            assert_eq!(
                classify_flatfile_error(&e),
                (StatusCode::NOT_FOUND, "flatfiles_no_data"),
                "no-data rejection must be a 404, not a 502: {msg}"
            );
        }
    }

    /// A genuine upstream/transport or auth fault stays `502
    /// flatfiles_unavailable`, and a malformed-request rejection is NOT
    /// swept into the no-data bucket — masking a bad request as an empty
    /// result would hide the fault. All must surface as 502. `AuthRejected
    /// { 15 }` is `ServerRestarting` — a transient outage, not a no-data
    /// condition. `AuthRejected { 3 }` is `GeneralValidationError` — a
    /// login-phase credential/auth failure the upstream handles like
    /// `InvalidCredentials`; classing it no-data would mask an auth failure
    /// behind a `404`, so it must stay 502.
    #[test]
    fn transport_and_malformed_faults_stay_502() {
        let cases = [
            FlatFilesUnavailableReason::StreamTruncated {
                bytes_received: 4096,
            },
            FlatFilesUnavailableReason::AuthRejected { reason_code: 15 },
            FlatFilesUnavailableReason::AuthRejected { reason_code: 3 },
            FlatFilesUnavailableReason::RequestRejected {
                server_message: "INVALID_PARAMS:Invalid request type".to_string(),
            },
        ];
        for reason in cases {
            let e = thetadatadx::Error::FlatFilesUnavailable(reason.clone());
            assert_eq!(
                classify_flatfile_error(&e),
                (StatusCode::BAD_GATEWAY, "flatfiles_unavailable"),
                "transport / malformed fault must stay 502: {reason:?}"
            );
        }
    }

    /// A no-data `DISCONNECTED` arrives as `AuthRejected` carrying a
    /// no-data `RemoveReason` (13 = NoStartDate, the out-of-window /
    /// snapshot-not-generated condition). It is the same "nothing here for
    /// this account/date" outcome as the `ERROR`-frame no-data, so it must
    /// also map to `404 flatfiles_no_data` — not the `502` the default arm
    /// would give. This is the server half of the regression fix: before,
    /// a no-data `DISCONNECTED` ran the full retry ladder and then answered
    /// `502`, while the same no-data via an `ERROR` frame answered `404`.
    #[test]
    fn no_data_disconnect_maps_to_404() {
        let e =
            thetadatadx::Error::FlatFilesUnavailable(FlatFilesUnavailableReason::AuthRejected {
                reason_code: 13,
            });
        assert_eq!(
            classify_flatfile_error(&e),
            (StatusCode::NOT_FOUND, "flatfiles_no_data"),
            "NoStartDate DISCONNECTED must be a 404, not a 502",
        );
    }

    /// `rejection_is_no_data` matches the upstream's tagged reason prefix
    /// robustly against whitespace and prose. A leading space must not
    /// blank the match (regression: a whitespace/`:`-split first token went
    /// empty → wrong 502), and `"PERMISSION denied"` prose must NOT match
    /// (regression: a bare `PERMISSION` token → wrong 404). Only the exact
    /// `PERMISSION:` / `NO_START_DATE` tags count.
    #[test]
    fn rejection_is_no_data_is_whitespace_and_prose_robust() {
        // Tagged no-data reasons, including a leading-space variant.
        assert!(rejection_is_no_data(
            "PERMISSION:Invalid permissions for date. Your first access date is: 20260618"
        ));
        assert!(rejection_is_no_data("NO_START_DATE:no snapshot for date"));
        assert!(
            rejection_is_no_data("  PERMISSION:leading whitespace"),
            "a leading space must not blank the no-data match"
        );
        assert!(rejection_is_no_data("\tNO_START_DATE:tab-led"));

        // Prose / unrelated reasons must NOT be swept into no-data.
        assert!(
            !rejection_is_no_data("PERMISSION denied: contact support"),
            "`PERMISSION denied` prose must not match the `PERMISSION:` tag"
        );
        assert!(!rejection_is_no_data("INVALID_PARAMS:Invalid request type"));
        assert!(!rejection_is_no_data(""));
        assert!(!rejection_is_no_data("   "));
    }

    /// The SDK's local dataset gate rejects an unserved pair before any
    /// upstream call — a client request fault that stays `400 bad_request`.
    #[test]
    fn local_invalid_parameter_maps_to_400() {
        let e = thetadatadx::Error::config_invalid(
            "flatfiles.dataset",
            "flat-file service does not serve index trade_quote",
        );
        assert_eq!(
            classify_flatfile_error(&e),
            (StatusCode::BAD_REQUEST, "bad_request"),
        );
    }

    /// The served-matrix gate accepts every pair the distribution serves and
    /// rejects everything else — including a pair whose security and request
    /// types are each individually served but not as a pair (stock
    /// open_interest). No index pair is served, so every index dataset is
    /// rejected. The rejection names the dataset.
    #[test]
    fn unserved_dataset_is_rejected_at_the_boundary() {
        for (sec, req) in [
            (SecType::Option, ReqType::TradeQuote),
            (SecType::Option, ReqType::OpenInterest),
            (SecType::Option, ReqType::Eod),
            (SecType::Stock, ReqType::TradeQuote),
            (SecType::Stock, ReqType::Eod),
        ] {
            assert!(
                reject_unserved_dataset(sec, req).is_ok(),
                "served pair {sec} {} must be accepted",
                req.as_str()
            );
        }

        for (sec, req) in [
            (SecType::Stock, ReqType::OpenInterest),
            (SecType::Option, ReqType::Quote),
            (SecType::Option, ReqType::Trade),
            (SecType::Option, ReqType::Ohlc),
            (SecType::Index, ReqType::Eod),
            (SecType::Index, ReqType::TradeQuote),
            (SecType::Index, ReqType::OpenInterest),
        ] {
            let err = reject_unserved_dataset(sec, req).expect_err(&format!(
                "unserved pair {sec} {} must be rejected",
                req.as_str()
            ));
            assert!(
                err.contains("does not serve") && err.contains(req.as_str()),
                "rejection must name the unserved dataset; got {err:?}"
            );
        }
    }

    /// A well-formed `YYYY-MM-DD` date normalises to the `YYYYMMDD` the
    /// driver requires; a bare `YYYYMMDD` and any other shape pass through
    /// unchanged.
    #[test]
    fn dashed_date_is_normalised_to_yyyymmdd() {
        assert_eq!(normalize_flatfile_date("2026-04-28"), "20260428");
        assert_eq!(normalize_flatfile_date("20260428"), "20260428");
        // Not a well-formed dashed date: passed through for the driver to reject.
        assert_eq!(normalize_flatfile_date("2026/04/28"), "2026/04/28");
        assert_eq!(normalize_flatfile_date("2026-4-28"), "2026-4-28");
        assert_eq!(normalize_flatfile_date("garbage"), "garbage");
    }

    /// A GET whose query string omits the required `date` param must
    /// surface the server's canonical error envelope, not axum's default
    /// plain-text 400. The `Query<FlatfileQuery>` deserialize fails in the
    /// extractor before the handler body runs; the custom extractor routes
    /// that failure through the same `error_response` helper every other
    /// route family uses, so the GET path keys the same `header.error_type`
    /// contract clients drive retry / backoff off.
    #[tokio::test]
    async fn get_missing_date_returns_canonical_envelope() {
        use axum::body::to_bytes;
        use axum::extract::FromRequestParts;
        use axum::http::Request as HttpRequest;
        use sonic_rs::{JsonContainerTrait, JsonValueTrait};

        // No `date` key at all: `FlatfileQuery.date` is required, so the
        // deserialize fails before the handler runs.
        let request = HttpRequest::builder()
            .method("GET")
            .uri("/v3/option/flat_file/trade_quote?format=csv")
            .body(Body::empty())
            .unwrap();
        let (mut parts, _) = request.into_parts();

        let rejection =
            FlatfileQueryExtractor::<FlatfileQuery>::from_request_parts(&mut parts, &())
                .await
                .err()
                .expect("a query string missing the required `date` must be rejected");

        assert_eq!(
            rejection.status(),
            StatusCode::BAD_REQUEST,
            "a missing required query param is a client fault (400)"
        );
        assert_eq!(
            rejection
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some(crate::handler::JSON_CONTENT_TYPE),
            "the rejection must be served as JSON, not plain text"
        );

        let body = to_bytes(rejection.into_body(), usize::MAX).await.unwrap();
        let value: sonic_rs::Value =
            sonic_rs::from_slice(&body).expect("the rejection body must be the JSON envelope");

        let header = value.get("header").expect("envelope must carry a header");
        assert_eq!(
            header.get("error_type").as_str(),
            Some("bad_request"),
            "clients drive retry off header.error_type"
        );
        assert!(
            header
                .get("error_msg")
                .as_str()
                .is_some_and(|m| !m.is_empty()),
            "the envelope must carry a non-empty diagnostic error_msg"
        );
        assert_eq!(
            value
                .get("response")
                .and_then(|r| r.as_array())
                .map(sonic_rs::Array::len),
            Some(0),
            "the error envelope's response array must be empty"
        );
    }

    /// A GET with a malformed query string is rejected through the same
    /// canonical envelope. Here `date` is present but supplied twice, which
    /// the single-valued `String` field cannot deserialize.
    #[tokio::test]
    async fn get_malformed_query_string_returns_canonical_envelope() {
        use axum::body::to_bytes;
        use axum::extract::FromRequestParts;
        use axum::http::Request as HttpRequest;
        use sonic_rs::{JsonContainerTrait, JsonValueTrait};

        // `date` repeated: a single-valued `String` field cannot accept a
        // multi-value sequence, so the query deserialize fails.
        let request = HttpRequest::builder()
            .method("GET")
            .uri("/v3/option/flat_file/trade_quote?date=20260428&date=20260429")
            .body(Body::empty())
            .unwrap();
        let (mut parts, _) = request.into_parts();

        let rejection =
            FlatfileQueryExtractor::<FlatfileQuery>::from_request_parts(&mut parts, &())
                .await
                .err()
                .expect("a malformed query string must be rejected");

        assert_eq!(
            rejection.status(),
            StatusCode::BAD_REQUEST,
            "a malformed query string is a client fault (400)"
        );
        assert_eq!(
            rejection
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some(crate::handler::JSON_CONTENT_TYPE),
            "the rejection must be served as JSON, not plain text"
        );

        let body = to_bytes(rejection.into_body(), usize::MAX).await.unwrap();
        let value: sonic_rs::Value =
            sonic_rs::from_slice(&body).expect("the rejection body must be the JSON envelope");

        let header = value.get("header").expect("envelope must carry a header");
        assert_eq!(
            header.get("error_type").as_str(),
            Some("bad_request"),
            "clients drive retry off header.error_type"
        );
        assert!(
            header
                .get("error_msg")
                .as_str()
                .is_some_and(|m| !m.is_empty()),
            "the envelope must carry a non-empty diagnostic error_msg"
        );
        assert_eq!(
            value
                .get("response")
                .and_then(|r| r.as_array())
                .map(sonic_rs::Array::len),
            Some(0),
            "the error envelope's response array must be empty"
        );
    }

    /// A non-UTF-8 percent-encoded path segment (`%ff`) matches the brace
    /// route and reaches `Path::<(String, String)>`, whose `decode_utf8()`
    /// fails. The GET flat-file route must answer the canonical JSON
    /// envelope (`{"header":{"error_type":"bad_request",...},"response":[]}`,
    /// `Content-Type: application/json`, 400) rather than axum's default
    /// plain-text 400, matching the query sibling.
    ///
    /// Driven through a stateless probe router that mounts the same
    /// brace-route pattern over the same `path_rejection_response` mapping
    /// the live `handle_get` uses, so the reachability chain (route match →
    /// `Path` reject → envelope) is exercised end-to-end without a live
    /// client.
    #[tokio::test]
    async fn malformed_path_segment_returns_canonical_envelope() {
        use axum::body::to_bytes;
        use axum::routing::get;
        use sonic_rs::{JsonContainerTrait, JsonValueTrait};
        use tower::ServiceExt;

        async fn probe(path: Result<Path<(String, String)>, PathRejection>) -> Response {
            match path {
                Ok(_) => error_response(StatusCode::OK, "ok", ""),
                Err(rej) => path_rejection_response(&rej),
            }
        }

        let app: Router = Router::new().route("/v3/{sec_type}/flat_file/{req_type}", get(probe));

        // `%ff` decodes to the byte 0xFF, which is not valid UTF-8, so the
        // `Path` deserialize rejects after the route matches.
        let uri = "/v3/option/flat_file/%ff";
        let request = axum::http::Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.expect("router responds");

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "a non-UTF-8 path segment is a client fault (400): {uri}"
        );
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some(crate::handler::JSON_CONTENT_TYPE),
            "the rejection must be served as JSON, not plain text: {uri}"
        );

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: sonic_rs::Value = sonic_rs::from_slice(&body)
            .unwrap_or_else(|e| panic!("rejection body must be the JSON envelope ({uri}): {e}"));

        let envelope_header = value.get("header").expect("envelope must carry a header");
        assert_eq!(
            envelope_header.get("error_type").as_str(),
            Some("bad_request"),
            "clients drive retry off header.error_type: {uri}"
        );
        assert!(
            envelope_header
                .get("error_msg")
                .as_str()
                .is_some_and(|m| !m.is_empty()),
            "the envelope must carry a non-empty diagnostic error_msg: {uri}"
        );
        assert_eq!(
            value
                .get("response")
                .and_then(|r| r.as_array())
                .map(sonic_rs::Array::len),
            Some(0),
            "the error envelope's response array must be empty: {uri}"
        );
    }

    // Every request writes, and then unlinks, its own scratch file: two
    // concurrent identical requests must never share one, or the first to
    // finish would unlink the file the other is still writing.
    #[test]
    fn scratch_paths_are_unique_per_request() {
        let (a_scratch, a_name) = flatfile_paths(
            SecType::Option,
            ReqType::Quote,
            "20260428",
            FlatFileFormat::Csv,
        );
        let (b_scratch, b_name) = flatfile_paths(
            SecType::Option,
            ReqType::Quote,
            "20260428",
            FlatFileFormat::Csv,
        );
        assert_eq!(
            a_name, b_name,
            "the attachment name is deterministic per (sec, req, date, format)"
        );
        assert_ne!(
            a_scratch,
            b_scratch,
            "two concurrent identical requests share scratch path `{}`",
            a_scratch.display()
        );
    }
}
