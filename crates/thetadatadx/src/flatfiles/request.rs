//! High-level FLAT_FILE request driver.
//!
//! Given a `(SecType, ReqType, date)` tuple plus an output path, this module
//! drives one full request/response round-trip:
//!
//! 1. Open a TLS connection to the first reachable MDDS legacy host.
//! 2. Authenticate (CREDENTIALS + VERSION).
//! 3. Send a FLAT_FILE request frame.
//! 4. Stream every chunk to a local file until FLAT_FILE_END.
//! 5. Surface the local path back to the caller.
//!
//! This module owns the raw FLAT_FILE download step. The higher-level
//! INDEX walking, FIT decoding, and on-disk / in-memory output paths
//! live in [`crate::flatfiles`] under the `index`, `decode`, `writer`,
//! and `decoded` modules. Callers that want decoded vendor-format
//! output use the higher-level [`crate::ThetaDataDx::flatfile_request`]
//! entry point; callers that want the raw binary stream for a custom
//! pipeline use this entry point directly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};

use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::auth::Credentials;
use crate::error::Error;
use crate::flatfiles::framing::{msg, read_frame};
use crate::flatfiles::mdds_spki::{ALLOWED_MDDS_HOSTS, MDDS_PORTS};
use crate::flatfiles::session::{connect_and_login, MddsHost};
use crate::flatfiles::types::{FlatFilesUnavailableReason, ReqType, SecType};

/// Process-wide monotonic id generator. The server treats id as opaque; we
/// use an `AtomicI64` so concurrent `flatfile_request_raw` calls cannot
/// collide on the same request id within a single process.
static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn next_id() -> i64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Build the FLAT_FILE request payload.
///
/// Verified live against `nj-a.thetadata.us:12000` on `2026-04-29`:
/// ```text
/// MSG_CODE=217&id=<id>&start_date=YYYYMMDD&REQ=<reqcode>&SEC=<sectype>&rth=true&ivl=0
/// ```
fn build_flat_file_payload(id: i64, sec: SecType, req: ReqType, date: &str) -> Vec<u8> {
    format!(
        "MSG_CODE={}&id={}&start_date={}&REQ={}&SEC={}&rth=true&ivl=0",
        msg::FLAT_FILE,
        id,
        date,
        req.as_wire(),
        sec.as_wire(),
    )
    .into_bytes()
}

/// Validate that `date` is exactly 8 ASCII digits in YYYYMMDD shape.
fn validate_date(date: &str) -> Result<(), Error> {
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Config(format!(
            "flatfiles: date {date:?} must be YYYYMMDD digits"
        )));
    }
    Ok(())
}

/// Authenticate, send a FLAT_FILE request, and stream every response chunk
/// to `output_path`. On success returns `output_path`. On failure returns
/// the underlying [`Error`] — typically `Error::FlatFilesUnavailable` for
/// auth/server rejection, or `Error::Io` for local I/O issues.
///
/// **Output format**: a raw concatenation of every FLAT_FILE chunk
/// payload, in receive order, **without** the framing headers. This is the
/// same byte sequence the vendor jar accumulates internally before walking
/// the index — to convert it to CSV one must implement the INDEX walker
/// and per-data-type FIT decoder. Both are tracked as TODOs in
/// [`crate::flatfiles`].
pub async fn flatfile_request_raw(
    creds: &Credentials,
    sec: SecType,
    req: ReqType,
    date: &str,
    output_path: impl AsRef<Path>,
) -> Result<PathBuf, Error> {
    validate_date(date)?;

    // Build the host candidate list — try every (host, port) in priority
    // order, matching the vendor terminal's `MDDS_NJ_HOSTS` config.
    let mut hosts: Vec<MddsHost<'_>> =
        Vec::with_capacity(ALLOWED_MDDS_HOSTS.len() * MDDS_PORTS.len());
    for h in ALLOWED_MDDS_HOSTS {
        for p in MDDS_PORTS {
            hosts.push(MddsHost { host: h, port: *p });
        }
    }

    let mut session = connect_and_login(&hosts, creds).await?;
    tracing::debug!(target: "flatfiles", "authed against MDDS legacy: bundle={}", session.bundle);

    let request_id = next_id();
    let payload = build_flat_file_payload(request_id, sec, req, date);
    crate::flatfiles::framing::write_frame(
        &mut session.stream,
        msg::FLAT_FILE,
        request_id,
        &payload,
    )
    .await?;

    let output_path = output_path.as_ref().to_path_buf();
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    let file = File::create(&output_path).await?;
    // 1 MB buffer — typical chunks are ~8-64 KB, so this batches many
    // chunks per actual write syscall.
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let mut total: u64 = 0;
    let mut chunks: u32 = 0;
    // Loop only exits normally on FLAT_FILE_END; every other terminator
    // returns Err. Reaching the post-loop log line therefore implies a
    // clean end-of-stream, no bookkeeping flag needed.

    loop {
        let frame = match read_frame(&mut session.stream).await {
            Ok(f) => f,
            Err(e) => {
                // Differentiate between EOF-after-some-data (truncation)
                // and an outright protocol error.
                if total > 0
                    && matches!(&e, Error::Io(io) if io.kind() == std::io::ErrorKind::UnexpectedEof)
                {
                    return Err(Error::FlatFilesUnavailable(
                        FlatFilesUnavailableReason::StreamTruncated {
                            bytes_received: total,
                        },
                    ));
                }
                return Err(e);
            }
        };
        if frame.id != request_id && frame.msg != msg::PING {
            // The server may interleave heartbeats; everything else with a
            // foreign id is a protocol violation we want to surface.
            return Err(Error::Config(format!(
                "flatfiles: unexpected response id={} (expected {request_id}) msg={}",
                frame.id, frame.msg
            )));
        }
        match frame.msg {
            msg::FLAT_FILE => {
                out.write_all(&frame.payload).await?;
                total += frame.payload.len() as u64;
                chunks += 1;
            }
            msg::FLAT_FILE_END => {
                break;
            }
            msg::ERROR => {
                let server_message = String::from_utf8_lossy(&frame.payload).into_owned();
                return Err(Error::FlatFilesUnavailable(
                    FlatFilesUnavailableReason::RequestRejected { server_message },
                ));
            }
            msg::PING => {
                // Ignore — server-initiated heartbeat.
            }
            msg::DISCONNECTED => {
                let reason_code = if frame.payload.len() >= 2 {
                    u16::from_be_bytes([frame.payload[0], frame.payload[1]])
                } else {
                    0
                };
                return Err(Error::FlatFilesUnavailable(
                    FlatFilesUnavailableReason::AuthRejected { reason_code },
                ));
            }
            other => {
                return Err(Error::Config(format!(
                    "flatfiles: unexpected msg={other} during FLAT_FILE stream"
                )));
            }
        }
    }
    out.flush().await?;
    drop(out);
    tracing::info!(
        target: "flatfiles",
        "FLAT_FILE_END id={request_id} chunks={chunks} bytes={total} -> {}",
        output_path.display()
    );
    Ok(output_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_canonical_ascii() {
        let p = build_flat_file_payload(9001, SecType::Option, ReqType::OpenInterest, "20260428");
        let s = std::str::from_utf8(&p).unwrap();
        assert_eq!(
            s,
            "MSG_CODE=217&id=9001&start_date=20260428&REQ=103&SEC=OPTION&rth=true&ivl=0"
        );
    }

    #[test]
    fn date_validation_catches_garbage() {
        assert!(validate_date("20260428").is_ok());
        assert!(validate_date("2026-04-28").is_err());
        assert!(validate_date("abcdefgh").is_err());
        assert!(validate_date("").is_err());
    }
}
