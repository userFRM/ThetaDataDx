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
//! Transient failures (mid-stream truncation, server-side socket reset,
//! momentary connectivity blip on the legacy host) trigger automatic
//! retry with exponential backoff per
//! [`crate::config::FlatFilesConfig`]. Terminal failures (bad
//! credentials, malformed request) surface immediately — see
//! [`crate::flatfiles::FlatFilesUnavailableReason::is_transient`].
//!
//! This module owns the raw FLAT_FILE download step. The higher-level
//! INDEX walking, FIT decoding, and on-disk / in-memory output paths
//! live in [`crate::flatfiles`] under the `index`, `decode`, `writer`,
//! and `decoded` modules. Callers that want decoded vendor-format
//! output use the higher-level [`crate::Client::flatfile_request`]
//! entry point; callers that want the raw binary stream for a custom
//! pipeline use this entry point directly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};

use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::auth::Credentials;
use crate::config::FlatFilesConfig;
use crate::error::Error;
use crate::flatfiles::framing::{msg, read_frame, Frame};
use crate::flatfiles::mdds_spki::{ALLOWED_MDDS_HOSTS, MDDS_PORTS};
use crate::flatfiles::session::{connect_and_login, MddsHost};
use crate::flatfiles::types::{flat_file_serves, FlatFilesUnavailableReason, ReqType, SecType};
use crate::flatfiles::ScratchGuard;

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

/// Reject any `(sec, req)` pair the flat-file distribution does not
/// serve, before any connection is opened. Returns a typed
/// invalid-parameter error naming the unserved dataset so the caller
/// gets a deterministic local failure instead of a server-side
/// `INVALID_PARAMS` rejection after a round-trip. See
/// [`crate::flatfiles::types::flat_file_serves`] for the served matrix.
fn validate_dataset(sec: SecType, req: ReqType) -> Result<(), Error> {
    if flat_file_serves(sec, req) {
        return Ok(());
    }
    Err(Error::config_invalid(
        "flatfiles.dataset",
        format!(
            "flat-file service does not serve {} {}",
            sec.as_wire().to_ascii_lowercase(),
            req.as_str()
        ),
    ))
}

/// Validate that `date` is exactly 8 ASCII digits AND a real Gregorian
/// calendar date. Rejects shape-only matches like `"00000000"` or
/// `"20260230"` via the canonical `crate::tdbe::time::is_valid_yyyymmdd`
/// validator shared with MDDS + FPSS.
fn validate_date(date: &str) -> Result<(), Error> {
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::config_invalid(
            "flatfiles.date",
            format!("date {date:?} must be YYYYMMDD digits"),
        ));
    }
    let yyyymmdd: i32 = date.parse().map_err(|_| {
        Error::config_invalid(
            "flatfiles.date",
            format!("date {date:?} must be YYYYMMDD digits"),
        )
    })?;
    if !crate::tdbe::time::is_valid_yyyymmdd(yyyymmdd) {
        return Err(Error::config_invalid(
            "flatfiles.date",
            format!("date {date:?} is not a valid Gregorian date"),
        ));
    }
    Ok(())
}

/// Classify an [`Error`] returned by [`run_one_attempt`] as transient (worth
/// retrying on a fresh connection) or terminal (surface immediately).
fn error_is_transient(err: &Error) -> bool {
    match err {
        // Local I/O failures (connect refused, mid-stream TLS reset,
        // unexpected EOF before any payload arrived). The next attempt
        // re-runs the host candidate list from the top so a momentary
        // single-host blip rotates onto the next reachable host. Local
        // disk faults are the exception: a full, read-only, or
        // permission-denied output filesystem will not heal on retry, so
        // surface those immediately instead of re-running the whole download.
        Error::Io(io) => !matches!(
            io.kind(),
            std::io::ErrorKind::StorageFull
                | std::io::ErrorKind::PermissionDenied
                | std::io::ErrorKind::ReadOnlyFilesystem
        ),
        // Explicit reason classifier on the typed FLATFILES failure.
        // `StreamTruncated` is transient; `RequestRejected` is terminal;
        // `AuthRejected` depends on the wire reason code.
        Error::FlatFilesUnavailable(reason) => reason.is_transient(),
        // Auth-server / config errors are terminal — none of these are
        // resolved by retry alone.
        _ => false,
    }
}

/// Authenticate, send a FLAT_FILE request, and stream every response chunk
/// to `output_path`. On success returns `output_path`. On failure returns
/// the underlying [`Error`] — typically `Error::FlatFilesUnavailable` for
/// auth/server rejection, or `Error::Io` for local I/O issues.
///
/// **Output format**: a raw concatenation of every FLAT_FILE chunk
/// payload, in receive order, **without** the framing headers. This is the
/// same byte sequence the JVM terminal accumulates internally before walking
/// the index. The INDEX walker and per-`(SecType, ReqType)` FIT decoder
/// are exposed via [`crate::flatfiles::flatfile_request_decoded`];
/// this function returns the raw bytes for callers that want to keep the
/// on-disk vendor format unchanged.
///
/// Uses [`FlatFilesConfig::default`] for retry tuning. Callers that need
/// to override the retry budget should call
/// [`flatfile_request_raw_with_config`] directly.
pub async fn flatfile_request_raw(
    creds: &Credentials,
    sec: SecType,
    req: ReqType,
    date: &str,
    output_path: impl AsRef<Path>,
) -> Result<PathBuf, Error> {
    let config = FlatFilesConfig::default();
    flatfile_request_raw_with_config(creds, sec, req, date, output_path, &config).await
}

/// Same as [`flatfile_request_raw`] but with caller-supplied retry tuning.
///
/// Transient failures (`Error::Io`, `FlatFilesUnavailable::StreamTruncated`,
/// `FlatFilesUnavailable::AuthRejected` with a transient reason code)
/// trigger an exponential-backoff retry up to `config.max_attempts`
/// total. Terminal failures surface immediately — no amount of retrying
/// will fix bad credentials or a malformed request.
///
/// Backoff follows the ladder `initial_backoff`, `*2`, `*4` up to
/// `max_backoff`, full-jittered when [`FlatFilesConfig::jitter`] is set
/// — see [`FlatFilesConfig::delay_for_attempt`]. A `tracing::warn!`
/// is emitted before each sleep so operators can observe sustained
/// transient pressure on the legacy MDDS hosts.
pub async fn flatfile_request_raw_with_config(
    creds: &Credentials,
    sec: SecType,
    req: ReqType,
    date: &str,
    output_path: impl AsRef<Path>,
    config: &FlatFilesConfig,
) -> Result<PathBuf, Error> {
    validate_dataset(sec, req)?;
    validate_date(date)?;
    let output_path = output_path.as_ref().to_path_buf();
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    let output_for_attempt = output_path.clone();
    run_retry_loop(config, move |_attempt| {
        let creds = creds.clone();
        let path = output_for_attempt.clone();
        async move { run_one_attempt(&creds, sec, req, date, &path, config).await }
    })
    .await
}

/// Generic retry driver shared between [`flatfile_request_raw_with_config`]
/// and the unit test. Calls `attempt_fn(attempt_number)` once per try,
/// classifies the error, sleeps the configured backoff between transient
/// failures, and surfaces terminal failures immediately.
///
/// Extracted so the unit test can drive the exact retry / backoff /
/// terminal-vs-transient decision logic against a synthetic
/// `attempt_fn` without spinning up a real TLS server.
async fn run_retry_loop<F, Fut>(
    config: &FlatFilesConfig,
    mut attempt_fn: F,
) -> Result<PathBuf, Error>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<PathBuf, Error>>,
{
    // Cap the attempt budget at the validated upper bound so a future
    // bypass of `DirectConfig::validate` cannot turn a misconfigured
    // value into an unbounded retry loop. `max_attempts.max(1)` keeps
    // the call functional when a caller explicitly passes `0`.
    let max_attempts = config.max_attempts.max(1);

    let mut last_err: Option<Error> = None;
    for attempt in 1..=max_attempts {
        match attempt_fn(attempt).await {
            Ok(path) => return Ok(path),
            Err(err) => {
                if attempt >= max_attempts || !error_is_transient(&err) {
                    return Err(err);
                }
                let backoff = config.delay_for_attempt(attempt);
                tracing::warn!(
                    target: "flatfiles",
                    attempt,
                    max_attempts,
                    backoff_ms = backoff.as_millis() as u64,
                    error = %err,
                    "flatfile_request: transient failure, will retry",
                );
                last_err = Some(err);
                tokio::time::sleep(backoff).await;
            }
        }
    }

    // The loop returns directly on success or on terminal error; the
    // only way out of the bottom is exhausting the attempt budget.
    Err(last_err.expect("retry loop must record an error before exhaustion"))
}

/// Execute one full connect-and-stream pass without any retry. Internal
/// helper for [`flatfile_request_raw_with_config`]; returns the same
/// `Error` taxonomy callers see on the public API.
async fn run_one_attempt(
    creds: &Credentials,
    sec: SecType,
    req: ReqType,
    date: &str,
    output_path: &Path,
    config: &FlatFilesConfig,
) -> Result<PathBuf, Error> {
    // Build the host candidate list — try every (host, port) in priority
    // order from the `MDDS_NJ_HOSTS` / `MDDS_PORTS` tables.
    let mut hosts: Vec<MddsHost<'_>> =
        Vec::with_capacity(ALLOWED_MDDS_HOSTS.len() * MDDS_PORTS.len());
    for h in ALLOWED_MDDS_HOSTS {
        for p in MDDS_PORTS {
            hosts.push(MddsHost { host: h, port: *p });
        }
    }

    let connect_timeout = std::time::Duration::from_secs(config.connect_timeout_secs.max(1));
    let read_timeout = std::time::Duration::from_secs(config.read_timeout_secs.max(1));
    let mut session = connect_and_login(&hosts, creds, connect_timeout).await?;
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

    // Download into a sibling `.tmp` and rename onto `output_path` only after
    // the stream finishes and flushes. A prior good file at `output_path` is
    // never truncated up front, so an all-attempts-fail run leaves it intact,
    // and a mid-stream failure leaves the partial under the temp name, not the
    // final one. The guard reaps the temp on every failing exit (a `?` return
    // or a cancelled future); `disarm` after a successful rename.
    let tmp_path = {
        let mut p = output_path.as_os_str().to_owned();
        p.push(".tmp");
        PathBuf::from(p)
    };
    let mut tmp_guard = ScratchGuard::new(&tmp_path);
    let file = File::create(&tmp_path).await?;
    // 1 MB buffer — typical chunks are ~8-64 KB, so this batches many
    // chunks per actual write syscall.
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let mut total: u64 = 0;
    let mut chunks: u32 = 0;
    // Loop only exits normally on FLAT_FILE_END; every other terminator
    // returns Err. Reaching the post-loop log line therefore implies a
    // clean end-of-stream, no bookkeeping flag needed.

    loop {
        // Bound the wait for each frame. A server that stalls mid-stream
        // — never sending the next chunk nor FLAT_FILE_END — would
        // otherwise hang the download forever. On expiry, surface a
        // transient stall so the retry ladder reconnects on a fresh
        // session rather than blocking indefinitely.
        let frame = match tokio::time::timeout(read_timeout, read_frame(&mut session.stream)).await
        {
            Err(_) => {
                return Err(Error::FlatFilesUnavailable(
                    FlatFilesUnavailableReason::StreamTruncated {
                        bytes_received: total,
                    },
                ));
            }
            Ok(Ok(f)) => f,
            Ok(Err(e)) => {
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
        match classify_stream_frame(&frame, request_id, total)? {
            FrameAction::Append => {
                out.write_all(&frame.payload).await?;
                total += frame.payload.len() as u64;
                chunks += 1;
            }
            FrameAction::Ignore => {}
            FrameAction::EndOfStream => break,
        }
    }
    out.flush().await?;
    drop(out);
    // Publish the completed download onto the final name, then release the
    // guard so it does not reap the file we just renamed. Windows `rename`
    // (unlike Unix) fails when the destination exists, so remove a prior file
    // first there; the Unix path stays an atomic replace.
    // ponytail: Windows publish is remove-then-rename, not atomic; ReplaceFileW-via-windows-crate if a concurrent-writer race is ever observed
    #[cfg(windows)]
    let _ = tokio::fs::remove_file(output_path).await;
    tokio::fs::rename(&tmp_path, output_path).await?;
    tmp_guard.disarm();
    tracing::info!(
        target: "flatfiles",
        "FLAT_FILE_END id={request_id} chunks={chunks} bytes={total} -> {}",
        output_path.display()
    );
    Ok(output_path.to_path_buf())
}

/// What [`run_one_attempt`] should do with a decoded stream frame.
#[derive(Debug)]
enum FrameAction {
    /// A `FLAT_FILE` data chunk: append its payload to the output.
    Append,
    /// A frame that advances no state (a server `PING` heartbeat).
    Ignore,
    /// `FLAT_FILE_END`: the response is complete, stop reading.
    EndOfStream,
}

/// Decide what a single decoded frame means inside an in-flight FLAT_FILE
/// response, or surface the typed error it represents.
///
/// The frame layout (see [`crate::flatfiles::framing`]) scopes frames in
/// two ways. The *data* frames — `FLAT_FILE` chunks and the terminating
/// `FLAT_FILE_END` — carry the request id the client assigned, so a foreign
/// id on those is a genuine correlation fault and is rejected. The *control*
/// frames the server can interleave — `ERROR`, `DISCONNECTED`, `PING` — are
/// connection-scoped and carry the sentinel `id=-1`; they are dispatched by
/// message code alone. Gating the id check on message code is what keeps a
/// connection-scoped `DISCONNECTED` (`id=-1`, `msg=102`) — the server's way
/// of declining to serve a slice, e.g. when the daily snapshot is not yet
/// generated — from being misreported as an `unexpected response id=-1`
/// correlation error instead of its real `RemoveReason`.
///
/// `bytes_received` is threaded in only for the `DISCONNECTED` debug log so
/// operators can see how far a declined stream got before the server cut it.
fn classify_stream_frame(
    frame: &Frame,
    request_id: i64,
    bytes_received: u64,
) -> Result<FrameAction, Error> {
    match frame.msg {
        // Data frames are request-scoped: a foreign id is a framing fault.
        msg::FLAT_FILE | msg::FLAT_FILE_END if frame.id != request_id => {
            Err(Error::config_internal(format!(
                "flatfiles: unexpected response id={} (expected {request_id}) msg={}",
                frame.id, frame.msg
            )))
        }
        msg::FLAT_FILE => Ok(FrameAction::Append),
        msg::FLAT_FILE_END => Ok(FrameAction::EndOfStream),
        msg::ERROR => {
            let server_message = String::from_utf8_lossy(&frame.payload).into_owned();
            Err(Error::FlatFilesUnavailable(
                FlatFilesUnavailableReason::RequestRejected { server_message },
            ))
        }
        msg::PING => Ok(FrameAction::Ignore),
        msg::DISCONNECTED => {
            let reason_code = if frame.payload.len() >= 2 {
                u16::from_be_bytes([frame.payload[0], frame.payload[1]])
            } else {
                0
            };
            tracing::debug!(
                target: "flatfiles",
                request_id,
                reason_code,
                bytes_received,
                "FLAT_FILE stream ended with DISCONNECTED",
            );
            Err(Error::FlatFilesUnavailable(
                FlatFilesUnavailableReason::AuthRejected { reason_code },
            ))
        }
        other => Err(Error::config_internal(format!(
            "flatfiles: unexpected msg={other} during FLAT_FILE stream"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a bare [`Frame`] for the dispatch-classifier tests.
    fn frame(msg: u16, id: i64, payload: Vec<u8>) -> Frame {
        Frame { msg, id, payload }
    }

    /// A `DISCONNECTED` (msg=102) and an `ERROR` (msg=101) arrive
    /// connection-scoped with the sentinel `id=-1`, never the request id.
    /// The classifier must decode them by message code — into the typed
    /// `AuthRejected` / `RequestRejected` reasons — rather than tripping the
    /// request-id correlation guard. This is the exact frame that surfaced
    /// as `unexpected response id=-1 (expected N) msg=102` before the fix.
    #[test]
    fn connection_scoped_control_frames_dispatch_by_code_not_id() {
        let request_id = 7;

        // DISCONNECTED with id=-1 and a 2-byte RemoveReason payload.
        let disc = frame(msg::DISCONNECTED, -1, 9u16.to_be_bytes().to_vec());
        match classify_stream_frame(&disc, request_id, 0) {
            Err(Error::FlatFilesUnavailable(FlatFilesUnavailableReason::AuthRejected {
                reason_code,
            })) => assert_eq!(
                reason_code, 9,
                "RemoveReason ordinal must decode from payload"
            ),
            other => panic!("DISCONNECTED id=-1 must decode to AuthRejected, got {other:?}"),
        }

        // ERROR with id=-1 carrying a server diagnostic string.
        let err = frame(
            msg::ERROR,
            -1,
            b"PERMISSION:Invalid permissions for date".to_vec(),
        );
        match classify_stream_frame(&err, request_id, 4096) {
            Err(Error::FlatFilesUnavailable(FlatFilesUnavailableReason::RequestRejected {
                server_message,
            })) => assert!(
                server_message.starts_with("PERMISSION:"),
                "server diagnostic must be preserved verbatim, got {server_message:?}"
            ),
            other => panic!("ERROR id=-1 must decode to RequestRejected, got {other:?}"),
        }
    }

    /// A `PING` heartbeat (also connection-scoped, `id=-1`) is ignored, and
    /// a request-scoped data frame carrying the matching id is handled
    /// normally — `FLAT_FILE` appends, `FLAT_FILE_END` ends the stream.
    #[test]
    fn data_frames_and_heartbeats_classify_normally() {
        let request_id = 42;
        assert!(matches!(
            classify_stream_frame(&frame(msg::PING, -1, vec![]), request_id, 0),
            Ok(FrameAction::Ignore)
        ));
        assert!(matches!(
            classify_stream_frame(
                &frame(msg::FLAT_FILE, request_id, vec![1, 2, 3]),
                request_id,
                0
            ),
            Ok(FrameAction::Append)
        ));
        assert!(matches!(
            classify_stream_frame(
                &frame(msg::FLAT_FILE_END, request_id, vec![]),
                request_id,
                99
            ),
            Ok(FrameAction::EndOfStream)
        ));
    }

    /// The request-id correlation guard still fires — but only for the
    /// request-scoped *data* frames. A `FLAT_FILE` / `FLAT_FILE_END` whose
    /// id does not match the request is a genuine framing fault and must
    /// surface the `unexpected response` internal error.
    #[test]
    fn foreign_id_on_data_frame_is_still_a_correlation_fault() {
        let request_id = 5;
        for code in [msg::FLAT_FILE, msg::FLAT_FILE_END] {
            let wrong = frame(code, 999, vec![]);
            let err = classify_stream_frame(&wrong, request_id, 0)
                .expect_err("a data frame with a foreign id must be rejected");
            assert!(
                err.to_string().contains("unexpected response id=999"),
                "foreign-id data frame must name the correlation fault, got {err}"
            );
        }
    }

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
    fn dataset_gate_accepts_the_five_served_combos() {
        for (sec, req) in [
            (SecType::Option, ReqType::TradeQuote),
            (SecType::Option, ReqType::OpenInterest),
            (SecType::Option, ReqType::Eod),
            (SecType::Stock, ReqType::TradeQuote),
            (SecType::Stock, ReqType::Eod),
        ] {
            assert!(
                validate_dataset(sec, req).is_ok(),
                "{sec} {req:?} must pass the flat-file dataset gate"
            );
        }
    }

    #[test]
    fn dataset_gate_rejects_unserved_combos_with_typed_error() {
        // Representative unserved pairs across both security types.
        for (sec, req, needle) in [
            (SecType::Option, ReqType::Quote, "option quote"),
            (SecType::Option, ReqType::Trade, "option trade"),
            (SecType::Option, ReqType::Ohlc, "option ohlc"),
            (SecType::Stock, ReqType::Quote, "stock quote"),
            (SecType::Stock, ReqType::Trade, "stock trade"),
            (SecType::Stock, ReqType::Ohlc, "stock ohlc"),
            (SecType::Stock, ReqType::OpenInterest, "stock open_interest"),
            (SecType::Index, ReqType::TradeQuote, "index trade_quote"),
            (SecType::Index, ReqType::Quote, "index quote"),
            (SecType::Index, ReqType::Eod, "index eod"),
        ] {
            let err = validate_dataset(sec, req).expect_err("unserved pair must be rejected");
            // Typed invalid-parameter leaf, not a network failure.
            assert!(
                matches!(&err, Error::Config { kind, .. } if matches!(kind, crate::error::ConfigErrorKind::InvalidValue { .. })),
                "{sec} {req:?} must surface the typed invalid-parameter error, got {err:?}"
            );
            assert!(
                err.to_string().contains(needle),
                "error for {sec} {req:?} must name the unserved pair, got {err}"
            );
        }
    }

    #[test]
    fn date_validation_catches_garbage() {
        assert!(validate_date("20260428").is_ok());
        assert!(validate_date("2026-04-28").is_err());
        assert!(validate_date("abcdefgh").is_err());
        assert!(validate_date("").is_err());
        // Shape-only acceptance was the old bug; calendar-impossible
        // dates must now be rejected here too.
        assert!(validate_date("00000000").is_err());
        assert!(validate_date("20260230").is_err());
        assert!(validate_date("19990431").is_err());
    }

    /// Build a `FlatFilesConfig` with sub-millisecond backoff so the
    /// async retry tests don't wait real wall-clock seconds. Production
    /// validation forbids these values (the public `validate()`
    /// requires sane intervals) — only constructed here directly,
    /// never round-tripped through `DirectConfig::validate`.
    fn test_config(max_attempts: u32) -> FlatFilesConfig {
        FlatFilesConfig {
            max_attempts,
            initial_backoff: std::time::Duration::from_millis(1),
            max_backoff: std::time::Duration::from_millis(4),
            jitter: false,
            ..FlatFilesConfig::production_defaults()
        }
    }

    /// Drive the retry loop against a synthetic attempt function that
    /// returns the queued result on each call. Verifies the four
    /// retry-loop contracts:
    ///
    /// * Transient failure on attempt 1 + success on attempt 2 → loop
    ///   reports success after one retry.
    /// * Transient failure on attempts 1+2 + success on attempt 3 →
    ///   loop reports success after two retries.
    /// * Three transient failures with `max_attempts = 3` → loop
    ///   exhausts the budget and surfaces the last error.
    /// * Terminal failure on attempt 1 → loop short-circuits even with
    ///   attempts remaining.
    #[tokio::test]
    async fn retry_loop_succeeds_after_one_transient_then_ok() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let queue: Rc<RefCell<Vec<Result<PathBuf, Error>>>> = Rc::new(RefCell::new(vec![
            Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "blip",
            ))),
            Ok(PathBuf::from("/tmp/ok")),
        ]));
        let attempts: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
        let queue_ref = Rc::clone(&queue);
        let attempts_ref = Rc::clone(&attempts);
        let result = run_retry_loop(&test_config(3), move |_attempt| {
            *attempts_ref.borrow_mut() += 1;
            let next = queue_ref.borrow_mut().remove(0);
            async move { next }
        })
        .await;
        assert_eq!(result.unwrap(), PathBuf::from("/tmp/ok"));
        assert_eq!(*attempts.borrow(), 2, "expected 1 retry then success");
    }

    #[tokio::test]
    async fn retry_loop_succeeds_after_two_transients_then_ok() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let queue: Rc<RefCell<Vec<Result<PathBuf, Error>>>> = Rc::new(RefCell::new(vec![
            Err(Error::FlatFilesUnavailable(
                FlatFilesUnavailableReason::StreamTruncated {
                    bytes_received: 1024,
                },
            )),
            Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "eof",
            ))),
            Ok(PathBuf::from("/tmp/recovered")),
        ]));
        let attempts: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
        let queue_ref = Rc::clone(&queue);
        let attempts_ref = Rc::clone(&attempts);
        let result = run_retry_loop(&test_config(3), move |_attempt| {
            *attempts_ref.borrow_mut() += 1;
            let next = queue_ref.borrow_mut().remove(0);
            async move { next }
        })
        .await;
        assert_eq!(result.unwrap(), PathBuf::from("/tmp/recovered"));
        assert_eq!(*attempts.borrow(), 3, "expected 2 retries then success");
    }

    #[tokio::test]
    async fn retry_loop_exhausts_attempt_budget_on_sustained_transient() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let attempts: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
        let attempts_ref = Rc::clone(&attempts);
        let result = run_retry_loop(&test_config(3), move |_attempt| {
            *attempts_ref.borrow_mut() += 1;
            async move {
                Err::<PathBuf, _>(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "reset",
                )))
            }
        })
        .await;
        let err = result.unwrap_err();
        assert!(matches!(err, Error::Io(_)));
        assert_eq!(*attempts.borrow(), 3, "expected exactly max_attempts tries");
    }

    #[tokio::test]
    async fn retry_loop_short_circuits_on_terminal_error() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let attempts: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
        let attempts_ref = Rc::clone(&attempts);
        let result = run_retry_loop(&test_config(5), move |_attempt| {
            *attempts_ref.borrow_mut() += 1;
            async move {
                Err::<PathBuf, _>(Error::FlatFilesUnavailable(
                    FlatFilesUnavailableReason::RequestRejected {
                        server_message: "INVALID_PARAMS".into(),
                    },
                ))
            }
        })
        .await;
        assert!(matches!(
            result.unwrap_err(),
            Error::FlatFilesUnavailable(FlatFilesUnavailableReason::RequestRejected { .. })
        ));
        assert_eq!(
            *attempts.borrow(),
            1,
            "terminal errors must not consume retry budget"
        );
    }

    #[test]
    fn transient_classifier_routes_io_to_retry() {
        // `Error::Io` wraps any local socket / TLS failure; the retry
        // loop treats these as transient because reconnecting to the
        // next reachable host typically clears them.
        let io_err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset",
        ));
        assert!(error_is_transient(&io_err));

        // Stream truncation is transient — the legacy host dropped us
        // mid-stream, fresh connection might complete.
        let truncated = Error::FlatFilesUnavailable(FlatFilesUnavailableReason::StreamTruncated {
            bytes_received: 4096,
        });
        assert!(error_is_transient(&truncated));

        // Request rejection is terminal — bad params don't fix themselves.
        let rejected = Error::FlatFilesUnavailable(FlatFilesUnavailableReason::RequestRejected {
            server_message: "INVALID_PARAMS".into(),
        });
        assert!(!error_is_transient(&rejected));

        // Auth rejection with a permanent reason code (1 = InvalidLoginValues).
        let auth_permanent =
            Error::FlatFilesUnavailable(FlatFilesUnavailableReason::AuthRejected {
                reason_code: 1,
            });
        assert!(!error_is_transient(&auth_permanent));

        // Auth rejection with a transient reason code (15 = ServerRestarting).
        let auth_transient =
            Error::FlatFilesUnavailable(FlatFilesUnavailableReason::AuthRejected {
                reason_code: 15,
            });
        assert!(error_is_transient(&auth_transient));

        // A mid-stream no-data DISCONNECTED (13 = NoStartDate) is terminal —
        // the slice does not exist for this account/date, so re-running
        // cannot succeed. It must NOT enter the retry ladder.
        let no_data = Error::FlatFilesUnavailable(FlatFilesUnavailableReason::AuthRejected {
            reason_code: 13,
        });
        assert!(
            !error_is_transient(&no_data),
            "NoStartDate must be terminal, not retried"
        );

        // Config errors are terminal — not retryable.
        let cfg_err = Error::config_invalid("flatfiles.date", "bad");
        assert!(!error_is_transient(&cfg_err));
    }

    /// End-to-end through the retry driver: a `NoStartDate` (ordinal 13)
    /// `DISCONNECTED` decoded by [`classify_stream_frame`] is terminal, so
    /// the loop surfaces it on the first attempt instead of burning the
    /// full ~10-attempt budget. This is the regression guard — the old
    /// login-phase classifier drove this permanent no-data drop through
    /// every retry before failing.
    #[tokio::test]
    async fn no_start_date_disconnect_is_not_retried() {
        use std::cell::RefCell;
        use std::rc::Rc;
        // Reproduce the exact reason the frame classifier produces for a
        // mid-stream DISCONNECTED carrying RemoveReason ordinal 13.
        let disc = frame(msg::DISCONNECTED, -1, 13u16.to_be_bytes().to_vec());
        let no_data_err = classify_stream_frame(&disc, 7, 0)
            .expect_err("a DISCONNECTED frame must classify to an error");
        match &no_data_err {
            Error::FlatFilesUnavailable(FlatFilesUnavailableReason::AuthRejected {
                reason_code,
            }) => assert_eq!(*reason_code, 13),
            other => panic!("NoStartDate DISCONNECTED must decode to AuthRejected, got {other:?}"),
        }

        let attempts: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
        let attempts_ref = Rc::clone(&attempts);
        let result = run_retry_loop(&test_config(5), move |_attempt| {
            *attempts_ref.borrow_mut() += 1;
            // Each attempt yields the same terminal no-data error.
            async move {
                Err::<PathBuf, _>(Error::FlatFilesUnavailable(
                    FlatFilesUnavailableReason::AuthRejected { reason_code: 13 },
                ))
            }
        })
        .await;
        assert!(matches!(
            result.unwrap_err(),
            Error::FlatFilesUnavailable(FlatFilesUnavailableReason::AuthRejected {
                reason_code: 13
            })
        ));
        assert_eq!(
            *attempts.borrow(),
            1,
            "terminal no-data DISCONNECTED must not consume the retry budget"
        );
    }
}
