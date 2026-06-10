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
        [(header::CONTENT_TYPE, crate::handler::JSON_CONTENT_TYPE)],
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

async fn handle_post(state: State<AppState>, body: axum::Json<FlatfileRequestBody>) -> Response {
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
    // Pull and decode into a per-request scratch path, then
    // atomically rename onto the deterministic final path. The SDK
    // writes bytes to disk during decode; we then stream the file
    // back to the client via tokio_util's ReaderStream so even
    // ~hundred-MB blobs don't pin server memory.
    //
    // The final path is named deterministically per (sec_type,
    // req_type, date, format) so callers can recognise the artefact,
    // but writes never target it directly. Two concurrent requests
    // for the same slice each write a fresh `{final}.{uuid}.partial`
    // scratch file and `rename` it into place on success. The rename
    // is atomic on POSIX — a reader that has already opened the final
    // path keeps streaming the old inode while a second writer
    // installs a new one under the same path — so we never truncate
    // bytes out from under an in-flight client.
    let (scratch_path, final_path) = flatfile_paths(sec_type, req_type, date, format);

    let written_scratch = match state
        .tdx()
        .flatfile_request(sec_type, req_type, date, &scratch_path, format)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            let _ = tokio::fs::remove_file(&scratch_path).await;
            return error_response(
                StatusCode::BAD_GATEWAY,
                "flatfiles_unavailable",
                &e.to_string(),
            );
        }
    };

    // The SDK may auto-append the format extension; honour whatever
    // path it returned and atomic-rename it onto `final_path`.
    if let Err(e) = tokio::fs::rename(&written_scratch, &final_path).await {
        let _ = tokio::fs::remove_file(&written_scratch).await;
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "io_error",
            &format!("failed to install flatfile artifact: {e}"),
        );
    }

    let file = match tokio::fs::File::open(&final_path).await {
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

    let filename = final_path
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

/// Compute the `(scratch, final)` path pair for a flatfile request.
///
/// `final_path` is deterministic per `(sec_type, req_type, date, format)`
/// so callers can recognise the cached artefact. `scratch_path` is
/// per-request unique via a UUID4 suffix so two concurrent identical
/// requests can never share a write target — each writes its own
/// scratch file, then an atomic `rename` installs it onto the final
/// path. Exposed `pub(crate)` so the in-crate race-regression tests
/// can exercise the rename-on-success contract without spinning up a
/// live SDK.
pub(crate) fn flatfile_paths(
    sec_type: SecType,
    req_type: ReqType,
    date: &str,
    format: FlatFileFormat,
) -> (PathBuf, PathBuf) {
    let final_path = std::env::temp_dir().join(format!(
        "tdx_server_flatfile_{sec_type}_{}_{date}.{}",
        req_type as u32,
        format.extension(),
    ));
    let scratch_path = std::env::temp_dir().join(format!(
        "tdx_server_flatfile_{sec_type}_{}_{date}.{}.{}.partial",
        req_type as u32,
        format.extension(),
        {
            let bytes: [u8; 16] = rand::random();
            bytes.iter().fold(String::with_capacity(32), |mut s, b| {
                use std::fmt::Write;
                let _ = write!(s, "{b:02x}");
                s
            })
        },
    ));
    (scratch_path, final_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: concurrent identical requests must never share a
    // scratch path. Before the race fix two callers wrote into
    // the same deterministic temp path, racing `File::create()`
    // against each other while the first reader's open fd still
    // pointed at the old inode's bytes. The fix attaches a UUID4
    // suffix to the scratch leg so every caller writes its own file;
    // the final path stays deterministic so the rename target is
    // shared.
    #[test]
    fn scratch_paths_are_unique_per_request() {
        let (a_scratch, a_final) = flatfile_paths(
            SecType::Option,
            ReqType::Quote,
            "20260428",
            FlatFileFormat::Csv,
        );
        let (b_scratch, b_final) = flatfile_paths(
            SecType::Option,
            ReqType::Quote,
            "20260428",
            FlatFileFormat::Csv,
        );
        assert_eq!(
            a_final,
            b_final,
            "final path must be deterministic per (sec, req, date, format) — got `{}` vs `{}`",
            a_final.display(),
            b_final.display()
        );
        assert_ne!(
            a_scratch,
            b_scratch,
            "two concurrent identical requests share scratch path `{}` — race risk",
            a_scratch.display()
        );
        let a_str = a_scratch.to_string_lossy();
        let b_str = b_scratch.to_string_lossy();
        assert!(
            a_str.ends_with(".partial"),
            "scratch path must end in `.partial` so a crashed mid-write is recognisable; got `{a_str}`"
        );
        assert!(
            b_str.ends_with(".partial"),
            "scratch path must end in `.partial` so a crashed mid-write is recognisable; got `{b_str}`"
        );
    }

    // Regression: concurrent renames of distinct scratch files onto a
    // shared final path must each deliver complete bytes to whoever
    // opened the final path first. `std::fs::rename` is atomic on
    // POSIX — an in-flight reader's file handle continues serving the
    // OLD inode's bytes while the new inode is installed under the
    // same path. This test fires N concurrent writers + readers
    // against the same final path, asserts every reader sees exactly
    // one of the written payloads in full (no truncation, no
    // zero-length, no mid-rename tear).
    #[test]
    fn atomic_rename_never_truncates_an_in_flight_reader() {
        use std::io::{Read, Write};
        use std::sync::Arc;
        use std::thread;

        const N: usize = 16;
        const PAYLOAD_LEN: usize = 1 << 16;

        let dir = std::env::temp_dir().join(format!("tdx_server_flatfile_race_{}", {
            let bytes: [u8; 16] = rand::random();
            bytes.iter().fold(String::with_capacity(32), |mut s, b| {
                use std::fmt::Write;
                let _ = write!(s, "{b:02x}");
                s
            })
        }));
        std::fs::create_dir_all(&dir).unwrap();
        let final_path = Arc::new(dir.join("artifact.csv"));

        // Pre-stage an initial artifact so the first round of readers
        // always has an inode to open even if the writers haven't
        // landed yet. Writers race to replace it with their own
        // per-thread payload.
        {
            let mut f = std::fs::File::create(&*final_path).unwrap();
            f.write_all(&vec![0u8; PAYLOAD_LEN]).unwrap();
        }

        let mut writers = Vec::with_capacity(N);
        for tid in 0..N {
            let final_path = Arc::clone(&final_path);
            writers.push(thread::spawn(move || {
                // Each writer's payload is a distinct fixed-length
                // byte pattern so a reader can spot a mid-rename tear
                // (mixed bytes from two payloads in the same buffer).
                let payload = vec![tid as u8 + 1; PAYLOAD_LEN];
                let scratch = final_path
                    .parent()
                    .unwrap()
                    .join(format!("artifact.csv.{tid}.partial"));
                {
                    let mut f = std::fs::File::create(&scratch).unwrap();
                    f.write_all(&payload).unwrap();
                    f.sync_all().unwrap();
                }
                std::fs::rename(&scratch, &*final_path).unwrap();
            }));
        }

        let mut readers = Vec::with_capacity(N);
        for _ in 0..N {
            let final_path = Arc::clone(&final_path);
            readers.push(thread::spawn(move || {
                // Open + drain the file. The inode held by `f` is
                // fixed at open time, so a concurrent `rename` over
                // the path does not affect the bytes this reader
                // sees.
                let mut f = std::fs::File::open(&*final_path).unwrap();
                let mut buf = Vec::with_capacity(PAYLOAD_LEN);
                f.read_to_end(&mut buf).unwrap();
                buf
            }));
        }

        for w in writers {
            w.join().expect("writer thread panicked");
        }
        for r in readers {
            let bytes = r.join().expect("reader thread panicked");
            assert_eq!(
                bytes.len(),
                PAYLOAD_LEN,
                "reader observed truncated payload: got {} bytes, expected {}",
                bytes.len(),
                PAYLOAD_LEN
            );
            // Every byte must equal the first byte — proves no
            // mid-rename tear (no mixed bytes from two payloads).
            let first = bytes[0];
            assert!(
                bytes.iter().all(|&b| b == first),
                "reader observed mid-rename byte tear (first byte = {first}, mismatch present)"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }
}
