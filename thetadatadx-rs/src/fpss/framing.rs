//! FPSS wire frame reader and writer.
//!
//! # Wire format
//!
//! Every FPSS message (both client-to-server and server-to-client) uses the same
//! 2-byte header followed by a variable-length payload:
//!
//! ```text
//! [LEN: u8] [CODE: u8] [PAYLOAD: LEN bytes]
//! ```
//!
//! - `LEN` -- payload length (0..255). Does NOT include the 2-byte header itself.
//! - `CODE` -- message type, maps to [`StreamMsgType`].
//! - `PAYLOAD` -- `LEN` bytes of message-specific data.
//!
//! Total bytes on the wire per message = `LEN + 2`.
//!
//! # Design
//!
//! The reader and writer operate on `std::io::Read` / `std::io::Write` traits,
//! making them testable with in-memory buffers (no real socket needed).
//! Fully synchronous -- no tokio, no async.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use crate::tdbe::types::enums::StreamMsgType;

#[cfg(any(test, feature = "__test-helpers"))]
use super::protocol::READ_TIMEOUT_MS;

/// Windows `ERROR_IO_PENDING` raw OS error code.
///
/// On Windows the overlapped socket layer surfaces in-flight reads as
/// `ERROR_IO_PENDING` (Win32 error 997) instead of `WSAEWOULDBLOCK`. Rust
/// `std` maps 997 to `ErrorKind::Uncategorized`, so a `kind()` match on
/// `WouldBlock | TimedOut` misses it and a benign in-flight read appears as
/// a fatal I/O error. Callers must check the `raw_os_error()` to recognise
/// it as transient.
///
/// Reference: <https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--500-999->
pub(crate) const ERROR_IO_PENDING: i32 = 997;

/// Classify a raw `std::io::Error` returned by `read()` as a transient
/// "no data right now, try again" condition.
///
/// Returns `true` for the three cases the FPSS framing and I/O loops must
/// retry / drain on rather than escalate to a fatal disconnect:
///
/// - `ErrorKind::WouldBlock` — Linux, macOS `SO_RCVTIMEO` on a non-blocking
///   socket.
/// - `ErrorKind::TimedOut` — macOS `SO_RCVTIMEO` on a blocking socket.
/// - `raw_os_error() == Some(997)` — Windows `ERROR_IO_PENDING` from the
///   overlapped I/O layer. Maps to `ErrorKind::Uncategorized` in `std`,
///   so a `kind()` match alone misses it.
#[must_use]
pub fn is_transient_read(io_err: &std::io::Error) -> bool {
    matches!(
        io_err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ) || io_err.raw_os_error() == Some(ERROR_IO_PENDING)
}

/// Maximum payload length (single unsigned byte).
///
/// The framing length field is a single byte, capping the payload at 255.
pub const MAX_PAYLOAD_LEN: usize = 255;

/// Read the 2-byte FPSS header under a per-stall no-progress timeout.
///
/// Byte-for-byte match with the terminal's framed read under a per-stall
/// socket timeout. Contract:
/// - EOF before any byte → `Ok(None)` (graceful server close).
/// - Pre-header `WouldBlock` / `TimedOut` (n == 0) → propagate as
///   `Error::Io` so `io_loop::is_read_timeout` drains pings +
///   command queue.
/// - Mid-header `WouldBlock` / `TimedOut` (n > 0) → retry. The stall
///   deadline is **re-armed on every successful byte**, matching the
///   terminal's per-`read()` socket-timeout semantics. Fatal only if
///   `stall_timeout` elapses without any forward progress.
///
/// Earlier revisions treated the first mid-header `WouldBlock` as
/// fatal desync. Captured raw-byte dumps showed the bytes were valid
/// frames that arrived 50-76 ms after the first `WouldBlock`;
/// aggressive escalation caused a reconnect storm whose downstream
/// effects accounted for the spurious "unknown message code" reports.
fn read_header_with_timeout<R: Read>(
    reader: &mut R,
    stall_timeout: Duration,
) -> Result<Option<[u8; 2]>, crate::error::Error> {
    let mut header_buf = [0u8; 2];
    let mut header_read = 0usize;
    let mut stall_deadline: Option<Instant> = None;
    loop {
        let n = header_read;
        match reader.read(&mut header_buf[n..]) {
            Ok(0) if n == 0 => return Ok(None),
            Ok(0) => {
                return Err(crate::error::Error::Stream {
                    kind: crate::error::StreamErrorKind::ProtocolError,
                    message: format!("truncated FPSS header: got {n} byte(s), expected 2"),
                })
            }
            Ok(read) => {
                // Forward progress — re-arm the stall clock for the next gap.
                stall_deadline = None;
                header_read = n + read;
                if header_read >= 2 {
                    return Ok(Some(header_buf));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && n == 0 => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(crate::error::Error::Stream {
                    kind: crate::error::StreamErrorKind::ProtocolError,
                    message: format!("truncated FPSS header: got {n} byte(s), expected 2"),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if n == 0 && is_transient_read(&e) => {
                // Pre-header transient: no bytes in flight. Surface as
                // `Error::Io` so the I/O loop drains pings + commands and
                // re-enters, exactly as the terminal's read-slice does.
                return Err(e.into());
            }
            Err(e) if is_transient_read(&e) => {
                // Mid-header transient with bytes already in flight. Retry
                // under the stall clock, re-armed on every byte of forward
                // progress; only `stall_timeout` of total silence is fatal.
                let now = Instant::now();
                let deadline = *stall_deadline.get_or_insert(now + stall_timeout);
                if now >= deadline {
                    return Err(crate::error::Error::Stream {
                        kind: crate::error::StreamErrorKind::ProtocolError,
                        message: format!(
                            "mid-header read timeout after {n} of 2 byte(s) without progress for {} ms: {e}",
                            stall_timeout.as_millis()
                        ),
                    });
                }
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Read exactly `buf.len()` bytes of payload under a per-stall
/// no-progress timeout.
///
/// Matches the terminal's read-N-bytes under a per-stall socket timeout:
/// every `WouldBlock` / `TimedOut` re-arms a fresh `stall_timeout`
/// deadline from the last successful byte of progress, not from function
/// entry. A stream that dribbles data in slowly but steadily is fine;
/// only `stall_timeout` of total silence fails.
///
/// Earlier revisions treated the first `WouldBlock` as a fatal desync.
/// Raw-byte tap captures showed every "corruption" event was a valid
/// frame that finished arriving 50-76 ms after the first `WouldBlock`.
/// The terminal tolerates that gap silently; so do we now.
///
/// `Interrupted` is retried (POSIX signal wakeups are benign).
/// `EOF` and going `stall_timeout` without progress are still fatal.
fn read_exact_payload_with_timeout<R: Read>(
    reader: &mut R,
    buf: &mut [u8],
    stall_timeout: Duration,
) -> Result<(), crate::error::Error> {
    let mut payload_read = 0usize;
    let mut stall_deadline: Option<Instant> = None;
    while payload_read < buf.len() {
        let n = payload_read;
        match reader.read(&mut buf[n..]) {
            Ok(0) => {
                return Err(crate::error::Error::Stream {
                    kind: crate::error::StreamErrorKind::ProtocolError,
                    message: format!("EOF mid-payload: got {n} of {} bytes", buf.len()),
                })
            }
            Ok(k) => {
                // Forward progress — re-arm the stall clock for the next gap.
                stall_deadline = None;
                payload_read = n + k;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if is_transient_read(&e) => {
                // Retry under the stall clock, re-armed on every byte of
                // forward progress; only `stall_timeout` of total silence
                // is fatal — the terminal's per-`read()` socket-timeout.
                let now = Instant::now();
                let deadline = *stall_deadline.get_or_insert(now + stall_timeout);
                if now >= deadline {
                    return Err(crate::error::Error::Stream {
                        kind: crate::error::StreamErrorKind::ProtocolError,
                        message: format!(
                            "mid-payload read timeout after {n} of {} byte(s) without progress for {} ms: {e}",
                            buf.len(),
                            stall_timeout.as_millis()
                        ),
                    });
                }
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Returns true if the payload contains non-printable control bytes,
/// indicating binary data rather than a human-readable error message.
pub(crate) fn is_binary_payload(payload: &[u8]) -> bool {
    payload.iter().any(|&b| b < 0x09 || (b > 0x0D && b < 0x20))
}

/// Read a single FPSS frame into a caller-owned buffer, avoiding per-frame
/// heap allocation on the hot path.
///
/// On success returns `Some((code, payload_len))` where `buf[..payload_len]`
/// holds the payload bytes. Returns `None` on clean EOF (reader closed).
///
/// # Buffer reuse
///
/// The caller passes in a reusable `Vec<u8>` that is `.clear()`ed and
/// `.resize()`d each call. Because `Vec` retains its capacity, repeated
/// calls with similarly-sized frames hit zero allocator calls after the
/// first frame.
///
/// # Single-call frame read
///
/// Each call reads one complete frame (header + payload) before
/// returning, bounded solely by the per-stall no-progress timeout — the
/// same bound the terminal's read loop applies. There is no mid-frame
/// resumption: a frame whose bytes span multiple read slices simply
/// blocks here until it completes or the stall timeout fires.
///
/// # Unknown message codes
///
/// Frames with unrecognized codes are silently skipped (payload consumed
/// to keep the stream aligned), with no ceiling on how many may be
/// skipped — the FIT decoder tolerates codes it does not recognize.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
///
/// Production reads go through [`read_frame_into_with_stall_timeout`] directly
/// (the I/O loop owns the deadline); this default-timeout wrapper exists for
/// the frame-pipeline integration tests, so it is gated to those builds.
#[cfg(any(test, feature = "__test-helpers"))]
pub fn read_frame_into<R: Read>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> Result<Option<(StreamMsgType, usize)>, crate::error::Error> {
    read_frame_into_with_stall_timeout(reader, buf, Duration::from_millis(READ_TIMEOUT_MS))
}

/// Takes the per-stall read timeout from the caller instead of the
/// parity-reference `READ_TIMEOUT_MS` default. The I/O loop threads the
/// user-supplied [`crate::config::StreamingConfig::timeout_ms`] through this
/// entry point so the public knob actually controls the framing stall budget.
///
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn read_frame_into_with_stall_timeout<R: Read>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    stall_timeout: Duration,
) -> Result<Option<(StreamMsgType, usize)>, crate::error::Error> {
    loop {
        // Header phase: read both header bytes, bounded by the per-stall
        // no-progress timeout.
        let Some(header) = read_header_with_timeout(reader, stall_timeout)? else {
            // Clean EOF before any byte of this frame.
            return Ok(None);
        };

        let payload_len = header[0] as usize;
        let code_byte = header[1];

        // Always consume the payload to keep the stream aligned. The
        // `buf.clear()` + `buf.resize()` allocates at most once per frame.
        buf.clear();
        buf.resize(payload_len, 0);
        if payload_len > 0 {
            read_exact_payload_with_timeout(reader, &mut buf[..payload_len], stall_timeout)?;
        }

        if let Some(code) = StreamMsgType::from_code(code_byte) {
            return Ok(Some((code, payload_len)));
        }
        // Unrecognized code: the payload was consumed above to keep the
        // stream aligned; skip and continue with no ceiling on how many may
        // be skipped. The FIT decoder tolerates codes it does not recognize,
        // matching the terminal.
        tracing::debug!(
            code = code_byte,
            payload_len,
            "skipping unknown FPSS message code"
        );
    }
}

/// Write a frame from raw parts without constructing a `Frame` struct.
///
/// Convenience function for hot paths (e.g., ping heartbeat) where we want
/// to avoid allocation. Always flushes after writing.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn write_raw_frame<W: Write>(
    writer: &mut W,
    code: StreamMsgType,
    payload: &[u8],
) -> Result<(), crate::error::Error> {
    write_raw_frame_no_flush(writer, code, payload)?;
    writer.flush()?;
    Ok(())
}

/// Write a frame from raw parts without flushing.
///
/// Use this when batching multiple writes. Caller is responsible for
/// flushing at the appropriate time (e.g., after PING frames only).
///
/// Flushes only on ping frames, letting `BufWriter` batch other writes
/// for better throughput.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn write_raw_frame_no_flush<W: Write>(
    writer: &mut W,
    code: StreamMsgType,
    payload: &[u8],
) -> Result<(), crate::error::Error> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(crate::error::Error::Stream {
            kind: crate::error::StreamErrorKind::ProtocolError,
            message: format!(
                "frame payload too large: {} bytes (max {})",
                payload.len(),
                MAX_PAYLOAD_LEN
            ),
        });
    }

    // Length already validated <= MAX_PAYLOAD_LEN (255).
    let len_byte = u8::try_from(payload.len()).map_err(|_| crate::error::Error::Stream {
        kind: crate::error::StreamErrorKind::ProtocolError,
        message: format!("frame payload length overflow: {}", payload.len()),
    })?;
    let header = [len_byte, code as u8];
    writer.write_all(&header)?;
    if !payload.is_empty() {
        writer.write_all(payload)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests -- all use in-memory cursors, no real sockets
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Helper: encode a frame manually and return the raw bytes.
    fn encode_manual(code: u8, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(2 + payload.len());
        buf.push(payload.len() as u8);
        buf.push(code);
        buf.extend_from_slice(payload);
        buf
    }

    /// Reader that returns a fixed prefix then always `WouldBlock` (or a
    /// caller-chosen kind). Models a stream where some bytes arrived but
    /// the rest of the frame never does.
    struct AlwaysStalledAfter {
        prefix: Vec<u8>,
        pos: usize,
        err_kind: std::io::ErrorKind,
    }

    impl std::io::Read for AlwaysStalledAfter {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos < self.prefix.len() {
                let remaining = &self.prefix[self.pos..];
                let n = remaining.len().min(buf.len());
                buf[..n].copy_from_slice(&remaining[..n]);
                self.pos += n;
                Ok(n)
            } else {
                Err(std::io::Error::new(self.err_kind, "simulated stall"))
            }
        }
    }

    /// A prolonged mid-payload stall (no progress past the stall deadline)
    /// must escalate to a fatal `ProtocolError`. The reader never delivers
    /// the remaining payload bytes.
    #[test]
    fn mid_payload_stall_past_deadline_escalates_to_fatal() {
        // LEN=4, CODE=PING; 2 of 4 payload bytes arrive; then infinite stall.
        let mut reader = AlwaysStalledAfter {
            prefix: vec![0x04, StreamMsgType::Ping as u8, 0x01, 0x02],
            pos: 0,
            err_kind: std::io::ErrorKind::WouldBlock,
        };
        // Consume the header via read_exact, then the payload reader must
        // bail after 20 ms of no progress.
        let mut header = [0u8; 2];
        std::io::Read::read_exact(&mut reader, &mut header).unwrap();
        assert_eq!(header, [0x04, StreamMsgType::Ping as u8]);
        let mut payload = [0u8; 4];
        let err =
            read_exact_payload_with_timeout(&mut reader, &mut payload, Duration::from_millis(20))
                .unwrap_err();
        match err {
            crate::error::Error::Stream { kind, message } => {
                assert_eq!(kind, crate::error::StreamErrorKind::ProtocolError);
                assert!(
                    message.contains("mid-payload")
                        && message.contains("without progress for 20 ms"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected fatal ProtocolError, got {other:?}"),
        }
    }

    /// Delivers one byte per `Ok`, each preceded by a single `WouldBlock`
    /// that sleeps for `sleep_per_stall`. Verifies the stall deadline
    /// resets on progress: cumulative sleep exceeds the deadline, but each
    /// individual gap is under it, so only reset-on-progress lets it pass.
    struct SleepingTrickle {
        remaining: Vec<u8>,
        sleep_per_stall: Duration,
        stalled_since_last_byte: bool,
    }

    impl std::io::Read for SleepingTrickle {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.remaining.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "done",
                ));
            }
            if !self.stalled_since_last_byte {
                self.stalled_since_last_byte = true;
                std::thread::sleep(self.sleep_per_stall);
                return Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, "tick"));
            }
            buf[0] = self.remaining.remove(0);
            self.stalled_since_last_byte = false;
            Ok(1)
        }
    }

    /// Many small gaps under a generous per-stall deadline must succeed:
    /// - `PAYLOAD_LEN` bytes, one `WouldBlock` gap before each.
    /// - Per-stall sleep `GAP_MS` (5 ms) is 40× below the per-stall
    ///   deadline `STALL_MS` (200 ms), so even a heavily load-stretched
    ///   gap stays well under the deadline.
    /// - Cumulative real sleep `PAYLOAD_LEN × GAP_MS` (80 × 5 ms = 400 ms)
    ///   structurally exceeds the per-stall deadline, so a fixed deadline
    ///   armed from function entry (the broken style) would fire; only
    ///   reset-on-progress lets the read complete.
    #[test]
    fn mid_payload_progress_resets_deadline() {
        const PAYLOAD_LEN: usize = 80;
        const GAP_MS: u64 = 5;
        const STALL_MS: u64 = 200;
        // The real cumulative sleep must exceed the per-stall deadline so
        // the reset-on-progress property is exercised, not vacuously met.
        const _: () = assert!(PAYLOAD_LEN as u64 * GAP_MS > STALL_MS);

        let mut reader = SleepingTrickle {
            remaining: (0..u8::try_from(PAYLOAD_LEN).unwrap()).collect::<Vec<u8>>(),
            sleep_per_stall: Duration::from_millis(GAP_MS),
            stalled_since_last_byte: false,
        };
        let mut payload = vec![0u8; PAYLOAD_LEN];
        let started = Instant::now();
        read_exact_payload_with_timeout(&mut reader, &mut payload, Duration::from_millis(STALL_MS))
            .unwrap();
        let elapsed = started.elapsed();
        let expected: Vec<u8> = (0..u8::try_from(PAYLOAD_LEN).unwrap()).collect();
        assert_eq!(payload, expected, "every byte must land in order");
        assert!(
            elapsed >= Duration::from_millis(STALL_MS),
            "cumulative wall time {elapsed:?} must exceed the {STALL_MS} ms per-stall \
             deadline — otherwise the reset-on-progress property is vacuously true"
        );
    }

    /// `TimedOut` must be classified identically to `WouldBlock`. macOS
    /// produces `TimedOut` on `SO_RCVTIMEO`; Linux produces `WouldBlock`.
    #[test]
    fn mid_payload_timed_out_treated_like_would_block() {
        let mut reader = AlwaysStalledAfter {
            prefix: vec![0x03, StreamMsgType::Ping as u8, 0xAA],
            pos: 0,
            err_kind: std::io::ErrorKind::TimedOut,
        };
        let mut header = [0u8; 2];
        std::io::Read::read_exact(&mut reader, &mut header).unwrap();
        let mut payload = [0u8; 3];
        let err =
            read_exact_payload_with_timeout(&mut reader, &mut payload, Duration::from_millis(20))
                .unwrap_err();
        match err {
            crate::error::Error::Stream { kind, message } => {
                assert_eq!(kind, crate::error::StreamErrorKind::ProtocolError);
                assert!(message.contains("mid-payload"), "got: {message}");
            }
            other => panic!("expected fatal ProtocolError, got {other:?}"),
        }
    }

    /// Same stall-escalation property for the header path: the first
    /// header byte arrives, then the reader stalls without progress past
    /// the deadline, and the header read must fail fatally.
    #[test]
    fn mid_header_stall_past_deadline_escalates_to_fatal() {
        let mut reader = AlwaysStalledAfter {
            prefix: vec![0x05],
            pos: 0,
            err_kind: std::io::ErrorKind::WouldBlock,
        };
        let err = read_header_with_timeout(&mut reader, Duration::from_millis(20)).unwrap_err();
        match err {
            crate::error::Error::Stream { kind, message } => {
                assert_eq!(kind, crate::error::StreamErrorKind::ProtocolError);
                assert!(
                    message.contains("mid-header")
                        && message.contains("without progress for 20 ms"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected fatal ProtocolError, got {other:?}"),
        }
    }

    /// A pre-header transient (zero bytes in flight) must surface as
    /// `Error::Io`, not a fatal `ProtocolError`: the I/O loop treats this
    /// as a benign drain-cadence tick and drains pings + commands. This is
    /// the boundary that keeps the command drain alive between frames.
    #[test]
    fn pre_header_would_block_surfaces_as_io_error() {
        let mut reader = AlwaysStalledAfter {
            prefix: Vec::new(),
            pos: 0,
            err_kind: std::io::ErrorKind::WouldBlock,
        };
        let err = read_header_with_timeout(&mut reader, Duration::from_millis(20)).unwrap_err();
        match err {
            crate::error::Error::Io(io_err) => {
                assert!(is_transient_read(&io_err), "must be a transient read");
            }
            other => panic!("pre-header WouldBlock must be Error::Io, got {other:?}"),
        }
    }

    /// Windows `ERROR_IO_PENDING` (raw OS error 997) must classify as a
    /// transient read. Rust `std` maps 997 to `ErrorKind::Uncategorized`,
    /// so a plain `kind()` match would miss it and treat the in-flight
    /// overlapped read as a fatal disconnect — which is exactly what the
    /// Python user reported in issue #469.
    #[test]
    fn is_transient_read_recognises_windows_error_io_pending() {
        let err = std::io::Error::from_raw_os_error(ERROR_IO_PENDING);
        // Sanity: confirm the precondition that motivates this fix —
        // `std` does not map 997 to a recognisable kind on any platform.
        assert_ne!(err.kind(), std::io::ErrorKind::WouldBlock);
        assert_ne!(err.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(err.raw_os_error(), Some(997));
        assert!(
            is_transient_read(&err),
            "ERROR_IO_PENDING (os error 997) must be classified as transient"
        );

        // Other raw OS errors (e.g. ECONNRESET on Linux) must NOT be
        // classified as transient — they are real disconnects.
        let real_err = std::io::Error::from_raw_os_error(104); // ECONNRESET
        assert!(
            !is_transient_read(&real_err),
            "ECONNRESET must not be classified as transient"
        );

        // The classic kinds still match.
        let wb = std::io::Error::new(std::io::ErrorKind::WouldBlock, "x");
        let to = std::io::Error::new(std::io::ErrorKind::TimedOut, "x");
        assert!(is_transient_read(&wb));
        assert!(is_transient_read(&to));
    }

    /// A full frame that arrives in one shot decodes to the right code and
    /// payload, and a subsequent independent call decodes the next frame:
    /// consecutive single-call reads never bleed state into each other (the
    /// frame-boundary invariant the reconnect path relies on).
    #[test]
    fn read_frame_into_decodes_consecutive_frames() {
        let mut bytes = encode_manual(StreamMsgType::Error as u8, b"hi");
        bytes.extend(encode_manual(StreamMsgType::Ping as u8, &[0xAA]));
        let mut cursor = Cursor::new(bytes);
        let mut buf = Vec::new();

        let (code, n) = read_frame_into(&mut cursor, &mut buf).unwrap().unwrap();
        assert_eq!(code, StreamMsgType::Error);
        assert_eq!(&buf[..n], b"hi");

        let (code, n) = read_frame_into(&mut cursor, &mut buf).unwrap().unwrap();
        assert_eq!(code, StreamMsgType::Ping);
        assert_eq!(&buf[..n], &[0xAA]);

        assert!(matches!(read_frame_into(&mut cursor, &mut buf), Ok(None)));
    }

    /// An unrecognized code is skipped (payload consumed to stay aligned)
    /// and the reader continues to the next well-formed frame in the same
    /// call, matching the terminal's tolerance for unknown codes.
    #[test]
    fn read_frame_into_skips_unknown_code() {
        let mut bytes = encode_manual(0xFE, &[0x00, 0x01, 0x02]); // no such code
        bytes.extend(encode_manual(StreamMsgType::Ping as u8, &[0xBB]));
        let mut cursor = Cursor::new(bytes);
        let mut buf = Vec::new();

        let (code, n) = read_frame_into(&mut cursor, &mut buf).unwrap().unwrap();
        assert_eq!(code, StreamMsgType::Ping);
        assert_eq!(&buf[..n], &[0xBB]);
    }
}
