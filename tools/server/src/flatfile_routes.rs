//! Hand-written FLATFILES route surface for the REST server (issue #432).
//!
//! Flat files are server-pre-built whole-universe daily blobs (CSV /
//! JSONL). They are a one-shot batch download — NOT a WebSocket
//! subscription stream — so the route surface is HTTP-only by design.
//! Streaming the bytes back to the client uses `axum::body::Body` over
//! a tokio file reader so the server doesn't pin a multi-hundred-MB
//! response in RAM.
//!
//! Routes:
//!
//! - `GET /v3/flatfile/{sec_type}/{req_type}` — convenience path. Path
//!   segments parse case-insensitively to the matching `SecType` /
//!   `ReqType`. Query params: `date=YYYYMMDD&format=csv|jsonl`.
//! - `POST /v3/flatfile/request` — generic endpoint. JSON body:
//!   `{ "sec_type": "OPTION", "req_type": "QUOTE", "date": "20260428",
//!      "format": "csv" }`.
//!
//! Response:
//! - `Content-Type: text/csv` (csv) or `application/x-ndjson` (jsonl).
//! - Body is the file bytes streamed via `tokio_util::io::ReaderStream`.
//! - On failure: standard error envelope (`error_type`, `error_msg`).
//!
//! Security note: the server requires authenticated access to ThetaData
//! servers. The MDDS-flat-file path inherits the same `AppState`
//! credentials as the per-endpoint surface; per-IP rate-limiting from
//! `router::build` applies here too.

use std::path::PathBuf;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};
use tokio_util::io::ReaderStream;

use crate::format;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct FlatfileQuery {
    /// Trading date in `YYYYMMDD` form.
    pub date: String,
    /// On-disk format: `csv` (default) or `jsonl`.
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FlatfileRequestBody {
    pub sec_type: String,
    pub req_type: String,
    pub date: String,
    #[serde(default)]
    pub format: Option<String>,
}

// ── Wire-friendly error response ─────────────────────────────────────────

fn error_response(status: StatusCode, error_type: &str, msg: &str) -> Response {
    let mut body = format::error_envelope(error_type, msg);
    let json_bytes =
        tdbe::json_canon::canonicalize_and_serialize(&mut body).unwrap_or_else(|err| {
            tracing::error!(error = %err, "flatfile error envelope serialise failed");
            format!(
                "{{\"header\":{{\"error_type\":\"serialization_error\",\
                 \"error_msg\":\"flatfile error envelope failed: {err}\"}},\
                 \"response\":[]}}"
            )
        });
    (
        status,
        [(
            header::CONTENT_TYPE,
            "application/json; charset=utf-8".to_string(),
        )],
        json_bytes,
    )
        .into_response()
}

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
        "jsonl" | "json" => Ok(FlatFileFormat::Jsonl),
        other => Err(format!(
            "unknown flat-file format: {other:?} (expected csv or jsonl)"
        )),
    }
}

fn content_type_for(format: FlatFileFormat) -> &'static str {
    match format {
        FlatFileFormat::Csv => "text/csv; charset=utf-8",
        // application/x-ndjson is the standard MIME for JSON Lines blobs.
        FlatFileFormat::Jsonl => "application/x-ndjson; charset=utf-8",
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────

async fn handle_get(
    state: State<AppState>,
    Path((sec_type_s, req_type_s)): Path<(String, String)>,
    Query(params): Query<FlatfileQuery>,
) -> Response {
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
    serve_flatfile(state, sec_type, req_type, &params.date, format).await
}

async fn handle_post(
    state: State<AppState>,
    body: axum::Json<FlatfileRequestBody>,
) -> Response {
    let sec_type = match parse_sec_type(&body.sec_type) {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "bad_request", &e),
    };
    let req_type = match parse_req_type(&body.req_type) {
        Ok(v) => v,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "bad_request", &e),
    };
    let format = match parse_format(body.format.as_deref()) {
        Ok(f) => f,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "bad_request", &e),
    };
    serve_flatfile(state, sec_type, req_type, &body.date, format).await
}

async fn serve_flatfile(
    state: State<AppState>,
    sec_type: SecType,
    req_type: ReqType,
    date: &str,
    format: FlatFileFormat,
) -> Response {
    // Pull and decode straight into a temp file. The SDK writes bytes
    // to disk during decode; we then stream the file back to the
    // client via tokio_util's ReaderStream so even ~hundred-MB blobs
    // don't pin server memory.
    //
    // The temp file is named deterministically per
    // (sec_type, req_type, date, format) so concurrent requests for
    // the same slice share a single download — the second request
    // will overwrite the same path while the first still has the
    // bytes open for streaming, which is harmless on POSIX (the
    // streaming reader's open fd holds the inode).
    let tmp_path: PathBuf = std::env::temp_dir().join(format!(
        "tdx_server_flatfile_{sec_type}_{}_{date}.{}",
        req_type as u32,
        format.extension(),
    ));

    let written = match state
        .tdx()
        .flatfile_request(sec_type, req_type, date, &tmp_path, format)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                "flatfiles_unavailable",
                &e.to_string(),
            );
        }
    };

    let file = match tokio::fs::File::open(&written).await {
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

    let filename = written
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("flatfile");

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

/// Add the FLATFILES routes onto an existing axum router.
pub(crate) fn add_flatfile_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route("/v3/flatfile/{sec_type}/{req_type}", get(handle_get))
        .route("/v3/flatfile/request", post(handle_post))
}
