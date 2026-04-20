//! FPSS wire frame reader and writer.
//!
//! # Wire format (from `PacketStream.java`)
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
//! Source: `PacketStream.readFrame()` and `PacketStream.writeFrame()` in the
//! decompiled Java terminal.
//!
//! # Design
//!
//! The reader and writer operate on `std::io::Read` / `std::io::Write` traits,
//! making them testable with in-memory buffers (no real socket needed).
//! Fully synchronous -- no tokio, no async.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use tdbe::types::enums::StreamMsgType;

use super::protocol::READ_TIMEOUT_MS;

/// Maximum payload length (single unsigned byte).
///
/// Source: `PacketStream.java` -- the length field is one byte.
pub const MAX_PAYLOAD_LEN: usize = 255;

/// A decoded FPSS frame: message code + payload bytes.
///
/// The `code` is the raw `StreamMsgType` enum value. Payload is a `Vec<u8>`
/// of length 0..255 as specified by the wire length byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Message type code (maps to [`StreamMsgType`]).
    pub code: StreamMsgType,
    /// Raw payload bytes.
    pub payload: Vec<u8>,
}

impl Frame {
    /// Create a new frame with the given message type and payload.
    ///
    /// # Panics
    ///
    /// Panics if `payload.len() > 255` (FPSS protocol limit).
    #[must_use]
    pub fn new(code: StreamMsgType, payload: Vec<u8>) -> Self {
        assert!(
            payload.len() <= MAX_PAYLOAD_LEN,
            "FPSS frame payload exceeds 255 bytes: {}",
            payload.len()
        );
        Self { code, payload }
    }
}

const MAX_CONSECUTIVE_UNKNOWN_CODES: usize = 5;

/// Read the 2-byte FPSS header.
///
/// Thin wrapper that plugs the production [`READ_TIMEOUT_MS`] deadline
/// into [`read_header_with_timeout`]. Splitting the function lets tests
/// exercise the timeout/retry contract with a short deadline.
fn read_header<R: Read>(reader: &mut R) -> Result<Option<[u8; 2]>, crate::error::Error> {
    read_header_with_timeout(reader, Duration::from_millis(READ_TIMEOUT_MS))
}

/// Read the 2-byte FPSS header with a configurable per-stall timeout.
///
/// Byte-for-byte match with Java `PacketStream2.readCopy` +
/// `socket.setSoTimeout(10_000)`:
/// - EOF before any byte → `Ok(None)` (graceful server close).
/// - Pre-header `WouldBlock` / `TimedOut` (n == 0) → propagate as `Error::Io`
///   so `io_loop::is_read_timeout` drains pings + command queue.
/// - Mid-header `WouldBlock` / `TimedOut` (n > 0) → retry. The stall
///   deadline is **re-armed on every successful byte**, matching Java's
///   per-`read()` `setSoTimeout` semantics. Only fatal if we go
///   `stall_timeout` without any forward progress.
///
/// Earlier revisions treated the first mid-header `WouldBlock` as a fatal
/// desync. Captured raw-byte dumps on both dev and prod showed the bytes
/// were valid frames that arrived 50-76 ms after the first `WouldBlock`;
/// aggressive escalation caused a reconnect storm whose downstream effects
/// accounted for the spurious "unknown message code" reports (#192, #369).
fn read_header_with_timeout<R: Read>(
    reader: &mut R,
    stall_timeout: Duration,
) -> Result<Option<[u8; 2]>, crate::error::Error> {
    let mut header = [0u8; 2];
    let mut n = 0usize;
    let mut stall_deadline: Option<Instant> = None;
    loop {
        match reader.read(&mut header[n..]) {
            Ok(0) if n == 0 => return Ok(None),
            Ok(0) => {
                return Err(crate::error::Error::Fpss {
                    kind: crate::error::FpssErrorKind::ProtocolError,
                    message: format!("truncated FPSS header: got {n} byte(s), expected 2"),
                })
            }
            Ok(read) => {
                // Forward progress — re-arm the stall clock for the next gap.
                stall_deadline = None;
                n += read;
                if n >= 2 {
                    return Ok(Some(header));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && n == 0 => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(crate::error::Error::Fpss {
                    kind: crate::error::FpssErrorKind::ProtocolError,
                    message: format!("truncated FPSS header: got {n} byte(s), expected 2"),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e)
                if n > 0
                    && matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
            {
                let deadline =
                    *stall_deadline.get_or_insert_with(|| Instant::now() + stall_timeout);
                if Instant::now() >= deadline {
                    return Err(crate::error::Error::Fpss {
                        kind: crate::error::FpssErrorKind::ProtocolError,
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

/// Read exactly `buf.len()` bytes of payload.
///
/// Thin wrapper that plugs the production [`READ_TIMEOUT_MS`] deadline
/// into [`read_exact_payload_with_timeout`]. Splitting the function lets
/// tests exercise the timeout/retry contract with a short deadline.
fn read_exact_payload<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<(), crate::error::Error> {
    read_exact_payload_with_timeout(reader, buf, Duration::from_millis(READ_TIMEOUT_MS))
}

/// Read exactly `buf.len()` bytes of payload with a configurable per-stall timeout.
///
/// Matches Java `DataInputStream.readNBytes` + `setSoTimeout(10_000)`:
/// every `WouldBlock` / `TimedOut` re-arms a fresh `stall_timeout` deadline
/// **from the last successful byte of progress**, not from function entry.
/// A stream that dribbles data in slowly but steadily is fine; only
/// `stall_timeout` of total silence fails.
///
/// Earlier revisions treated the first `WouldBlock` as a fatal desync.
/// Raw-byte tap captures on dev and prod showed every "corruption" event
/// was a valid frame that finished arriving 50-76 ms after the first
/// `WouldBlock`. Java tolerates that gap silently; so do we now.
///
/// `Interrupted` is retried (POSIX signal wakeups are benign). `EOF` and
/// going `stall_timeout` without progress are still fatal.
fn read_exact_payload_with_timeout<R: Read>(
    reader: &mut R,
    buf: &mut [u8],
    stall_timeout: Duration,
) -> Result<(), crate::error::Error> {
    let mut n = 0usize;
    let mut stall_deadline: Option<Instant> = None;
    while n < buf.len() {
        match reader.read(&mut buf[n..]) {
            Ok(0) => {
                return Err(crate::error::Error::Fpss {
                    kind: crate::error::FpssErrorKind::ProtocolError,
                    message: format!("EOF mid-payload: got {n} of {} bytes", buf.len()),
                })
            }
            Ok(k) => {
                // Forward progress — re-arm the stall clock for the next gap.
                stall_deadline = None;
                n += k;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                let deadline =
                    *stall_deadline.get_or_insert_with(|| Instant::now() + stall_timeout);
                if Instant::now() >= deadline {
                    return Err(crate::error::Error::Fpss {
                        kind: crate::error::FpssErrorKind::ProtocolError,
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
/// # Unknown message codes
///
/// Frames with unrecognized codes are silently skipped (payload consumed
/// to keep the stream aligned). After [`MAX_CONSECUTIVE_UNKNOWN_CODES`]
/// consecutive unknown codes, returns an error to trigger reconnection.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn read_frame_into<R: Read>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> Result<Option<(StreamMsgType, usize)>, crate::error::Error> {
    let mut consecutive_unknown = 0usize;

    loop {
        buf.clear();

        let Some(header) = read_header(reader)? else {
            return Ok(None);
        };

        let payload_len = header[0] as usize;
        let code_byte = header[1];

        // Always consume the payload to keep the stream aligned. A mid-payload
        // timeout is fatal: the cursor is desynced and io_loop must reconnect
        // (matches Java's PacketStream → IOException → reconnect path).
        buf.resize(payload_len, 0);
        if payload_len > 0 {
            read_exact_payload(reader, buf)?;
        }

        if let Some(code) = StreamMsgType::from_code(code_byte) {
            return Ok(Some((code, payload_len)));
        }
        consecutive_unknown += 1;
        if consecutive_unknown >= MAX_CONSECUTIVE_UNKNOWN_CODES {
            return Err(crate::error::Error::Fpss {
                kind: crate::error::FpssErrorKind::ProtocolError,
                message: format!(
                    "framing corruption: {consecutive_unknown} consecutive \
                     unknown message codes (last code: {code_byte})"
                ),
            });
        }
        tracing::debug!(
            code = code_byte,
            payload_len,
            consecutive_unknown,
            "skipping unknown FPSS message code"
        );
    }
}

/// Read a single FPSS frame from a blocking reader.
///
/// Convenience wrapper around [`read_frame_into`] that allocates a fresh
/// `Vec<u8>` per call. Prefer `read_frame_into` on the hot path.
///
/// Returns `None` on clean EOF (reader closed). Returns `Err` on partial reads
/// or unknown message codes.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Frame>, crate::error::Error> {
    let mut buf = Vec::new();
    match read_frame_into(reader, &mut buf)? {
        Some((code, _len)) => Ok(Some(Frame { code, payload: buf })),
        None => Ok(None),
    }
}

/// Write a single FPSS frame to a blocking writer.
///
/// # Wire format (from `PacketStream.writeFrame()`)
///
/// Writes `[LEN: u8] [CODE: u8] [PAYLOAD: LEN bytes]` and flushes.
///
/// Returns `Err` if the payload exceeds 255 bytes.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn write_frame<W: Write>(writer: &mut W, frame: &Frame) -> Result<(), crate::error::Error> {
    if frame.payload.len() > MAX_PAYLOAD_LEN {
        return Err(crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ProtocolError,
            message: format!(
                "frame payload too large: {} bytes (max {})",
                frame.payload.len(),
                MAX_PAYLOAD_LEN
            ),
        });
    }

    // Length already validated <= MAX_PAYLOAD_LEN (255); code is a StreamMsgType u8 repr.
    let len_byte = u8::try_from(frame.payload.len()).map_err(|_| crate::error::Error::Fpss {
        kind: crate::error::FpssErrorKind::ProtocolError,
        message: format!("frame payload length overflow: {}", frame.payload.len()),
    })?;
    let header = [len_byte, frame.code as u8];
    writer.write_all(&header)?;
    if !frame.payload.is_empty() {
        writer.write_all(&frame.payload)?;
    }
    writer.flush()?;

    Ok(())
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
/// Source: Java terminal only flushes on ping frames, letting `BufWriter`
/// batch other writes for better throughput.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn write_raw_frame_no_flush<W: Write>(
    writer: &mut W,
    code: StreamMsgType,
    payload: &[u8],
) -> Result<(), crate::error::Error> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ProtocolError,
            message: format!(
                "frame payload too large: {} bytes (max {})",
                payload.len(),
                MAX_PAYLOAD_LEN
            ),
        });
    }

    // Length already validated <= MAX_PAYLOAD_LEN (255).
    let len_byte = u8::try_from(payload.len()).map_err(|_| crate::error::Error::Fpss {
        kind: crate::error::FpssErrorKind::ProtocolError,
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

    #[test]
    fn read_empty_frame() {
        let data = encode_manual(StreamMsgType::Ping as u8, &[0x00]);
        let mut cursor = Cursor::new(data);
        let frame = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(frame.code, StreamMsgType::Ping);
        assert_eq!(frame.payload, vec![0x00]);
    }

    #[test]
    fn read_frame_with_payload() {
        let payload = b"hello world";
        let data = encode_manual(StreamMsgType::Error as u8, payload);
        let mut cursor = Cursor::new(data);
        let frame = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(frame.code, StreamMsgType::Error);
        assert_eq!(frame.payload, b"hello world");
    }

    #[test]
    fn read_frame_eof() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let result = read_frame(&mut cursor).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn read_frame_unknown_code_skipped() {
        // Unknown codes are silently skipped (dev server sends them).
        // After skipping, the reader hits EOF and returns None.
        let data = encode_manual(0xFF, &[]);
        let mut cursor = Cursor::new(data);
        let result = read_frame(&mut cursor).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn write_and_read_roundtrip() {
        let original = Frame::new(StreamMsgType::Credentials, b"test_creds".to_vec());

        // Write
        let mut buf = Vec::new();
        write_frame(&mut buf, &original).unwrap();

        // Read back
        let mut cursor = Cursor::new(buf);
        let decoded = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn write_raw_and_read_roundtrip() {
        let mut buf = Vec::new();
        write_raw_frame(&mut buf, StreamMsgType::Quote, &[1, 2, 3, 4]).unwrap();

        let mut cursor = Cursor::new(buf);
        let frame = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(frame.code, StreamMsgType::Quote);
        assert_eq!(frame.payload, vec![1, 2, 3, 4]);
    }

    #[test]
    fn write_frame_too_large() {
        let big_payload = vec![0u8; 256];
        let frame = Frame {
            code: StreamMsgType::Ping,
            payload: big_payload,
        };
        let mut buf = Vec::new();
        let err = write_frame(&mut buf, &frame).unwrap_err();
        assert!(err.to_string().contains("payload too large"));
    }

    #[test]
    fn read_zero_length_payload() {
        let data = encode_manual(StreamMsgType::Start as u8, &[]);
        let mut cursor = Cursor::new(data);
        let frame = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(frame.code, StreamMsgType::Start);
        assert!(frame.payload.is_empty());
    }

    #[test]
    fn multiple_frames_in_sequence() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&encode_manual(StreamMsgType::Ping as u8, &[0x00]));
        wire.extend_from_slice(&encode_manual(StreamMsgType::Error as u8, b"bad request"));
        wire.extend_from_slice(&encode_manual(StreamMsgType::Start as u8, &[]));

        let mut cursor = Cursor::new(wire);

        let f1 = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(f1.code, StreamMsgType::Ping);

        let f2 = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(f2.code, StreamMsgType::Error);
        assert_eq!(f2.payload, b"bad request");

        let f3 = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(f3.code, StreamMsgType::Start);
        assert!(f3.payload.is_empty());

        // Next read should return None (EOF)
        let f4 = read_frame(&mut cursor).unwrap();
        assert!(f4.is_none());
    }

    #[test]
    fn metadata_frame_utf8_payload() {
        // METADATA (code 3) carries a UTF-8 permissions string
        let perms = "pro,options,indices";
        let data = encode_manual(StreamMsgType::Metadata as u8, perms.as_bytes());
        let mut cursor = Cursor::new(data);
        let frame = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(frame.code, StreamMsgType::Metadata);
        assert_eq!(
            std::str::from_utf8(&frame.payload).unwrap(),
            "pro,options,indices"
        );
    }

    /// Reader that returns a prefix of a buffer, then fails with a specified
    /// IO error. Simulates a socket that stalls after delivering `n_before_err`
    /// bytes — the class of failure the framing escalation path exists to
    /// handle.
    struct PrefixThenErr {
        prefix: Vec<u8>,
        pos: usize,
        err_kind: std::io::ErrorKind,
    }

    impl std::io::Read for PrefixThenErr {
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

    /// Pre-header `WouldBlock` (zero bytes delivered) must propagate as
    /// `Error::Io` so `io_loop::is_read_timeout` can drain queued commands
    /// and re-enter the poll. This is the benign-timeout housekeeping path.
    #[test]
    fn pre_header_would_block_propagates_as_io() {
        let mut reader = PrefixThenErr {
            prefix: Vec::new(),
            pos: 0,
            err_kind: std::io::ErrorKind::WouldBlock,
        };
        let err = read_frame(&mut reader).unwrap_err();
        match err {
            crate::error::Error::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock),
            other => panic!("expected Error::Io(WouldBlock), got {other:?}"),
        }
    }

    /// Reader that emits a prefix, then `err_count` `WouldBlock` errors,
    /// then resumes with the suffix. Models the TCP pause between the
    /// header bytes and the payload that we captured on dev + prod.
    struct PrefixThenStallThenResume {
        prefix: Vec<u8>,
        suffix: Vec<u8>,
        prefix_pos: usize,
        suffix_pos: usize,
        remaining_stalls: usize,
    }

    impl std::io::Read for PrefixThenStallThenResume {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.prefix_pos < self.prefix.len() {
                let remaining = &self.prefix[self.prefix_pos..];
                let n = remaining.len().min(buf.len());
                buf[..n].copy_from_slice(&remaining[..n]);
                self.prefix_pos += n;
                return Ok(n);
            }
            if self.remaining_stalls > 0 {
                self.remaining_stalls -= 1;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "simulated stall",
                ));
            }
            if self.suffix_pos < self.suffix.len() {
                let remaining = &self.suffix[self.suffix_pos..];
                let n = remaining.len().min(buf.len());
                buf[..n].copy_from_slice(&remaining[..n]);
                self.suffix_pos += n;
                return Ok(n);
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "reader exhausted",
            ))
        }
    }

    /// Mid-header `WouldBlock` (one byte delivered, second stalls briefly)
    /// must retry within the `READ_TIMEOUT_MS` aggregate deadline, matching
    /// Java `readByte` + `setSoTimeout(10_000)`. Capture evidence: real
    /// server pauses between LEN and CODE measured at 50-76 ms on dev; the
    /// surrounding bytes were valid.
    #[test]
    fn mid_header_would_block_retries_and_recovers() {
        // 1 byte (LEN=1), 3 WouldBlock stalls, then CODE=PING + 1 payload byte.
        let mut reader = PrefixThenStallThenResume {
            prefix: vec![0x01],
            suffix: vec![StreamMsgType::Ping as u8, 0xAA],
            prefix_pos: 0,
            suffix_pos: 0,
            remaining_stalls: 3,
        };
        let frame = read_frame(&mut reader).unwrap().unwrap();
        assert_eq!(frame.code, StreamMsgType::Ping);
        assert_eq!(frame.payload, vec![0xAA]);
    }

    /// Mid-payload `WouldBlock` (header + partial payload, brief stall,
    /// rest arrives) must retry and complete, matching Java `readNBytes`
    /// + `setSoTimeout(10_000)`. Overwhelmingly most common case in the field.
    #[test]
    fn mid_payload_would_block_retries_and_recovers() {
        // header: len=4, code=PING; 2 payload bytes; 3 stalls; 2 more payload bytes.
        let mut reader = PrefixThenStallThenResume {
            prefix: vec![0x04, StreamMsgType::Ping as u8, 0x01, 0x02],
            suffix: vec![0x03, 0x04],
            prefix_pos: 0,
            suffix_pos: 0,
            remaining_stalls: 3,
        };
        let frame = read_frame(&mut reader).unwrap().unwrap();
        assert_eq!(frame.code, StreamMsgType::Ping);
        assert_eq!(frame.payload, vec![0x01, 0x02, 0x03, 0x04]);
    }

    /// Reader that always returns `WouldBlock` (or a caller-chosen kind).
    /// Models a stream where the header arrived but the payload never does.
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

    /// A prolonged mid-payload stall (no progress past the short stall
    /// deadline) must escalate to a fatal `ProtocolError`. The reader
    /// never delivers the remaining payload bytes.
    #[test]
    fn mid_payload_stall_past_deadline_escalates_to_fatal() {
        // LEN=4, CODE=PING; 2 of 4 payload bytes arrive; then infinite stall.
        let mut reader = AlwaysStalledAfter {
            prefix: vec![0x04, StreamMsgType::Ping as u8, 0x01, 0x02],
            pos: 0,
            err_kind: std::io::ErrorKind::WouldBlock,
        };
        // Consume the header via the public entry point, then the payload
        // path (timeout-configurable variant) must bail after 20 ms of
        // no-progress. Build a minimal 2-byte buffer matching payload size.
        let mut header = [0u8; 2];
        std::io::Read::read_exact(&mut reader, &mut header).unwrap();
        assert_eq!(header, [0x04, StreamMsgType::Ping as u8]);
        let mut payload = [0u8; 4];
        // First two payload bytes land; next reads stall forever.
        let err =
            read_exact_payload_with_timeout(&mut reader, &mut payload, Duration::from_millis(20))
                .unwrap_err();
        match err {
            crate::error::Error::Fpss { kind, message } => {
                assert_eq!(kind, crate::error::FpssErrorKind::ProtocolError);
                assert!(
                    message.contains("mid-payload")
                        && message.contains("without progress for 20 ms"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected fatal ProtocolError, got {other:?}"),
        }
    }

    /// A slow-trickling stream whose gaps are each under the deadline but
    /// whose aggregate duration exceeds it must still succeed. Proves the
    /// deadline re-arms on every successful byte (Java `setSoTimeout`
    /// per-read semantics) rather than running from function entry.
    /// Delivers one byte per `Ok`, with each byte preceded by a single
    /// `WouldBlock` that sleeps for `sleep_per_stall` wall-clock time.
    /// Used to verify the stall-deadline actually resets on progress —
    /// cumulative sleep exceeds the configured deadline, but each
    /// individual gap is under it. If the deadline did not reset, the
    /// second gap would trip it.
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

    /// With an 8-byte payload + 15 ms of WouldBlock sleep per byte + a 40 ms
    /// stall deadline:
    /// - Cumulative wall time: 8 × 15 ms = 120 ms (> 40 ms)
    /// - Longest single gap: 15 ms (< 40 ms)
    ///
    /// Must succeed under the Java-parity "reset on progress" semantics.
    /// A fixed deadline from function entry (prior implementation style)
    /// would fail around cycle 3 once cumulative sleep crossed 40 ms.
    #[test]
    fn mid_payload_progress_resets_deadline() {
        let mut reader = SleepingTrickle {
            remaining: (1..=8).collect::<Vec<u8>>(),
            sleep_per_stall: Duration::from_millis(15),
            stalled_since_last_byte: false,
        };
        let mut payload = [0u8; 8];
        let started = Instant::now();
        read_exact_payload_with_timeout(&mut reader, &mut payload, Duration::from_millis(40))
            .unwrap();
        let elapsed = started.elapsed();
        assert_eq!(payload, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(
            elapsed >= Duration::from_millis(40),
            "cumulative wall time {elapsed:?} should exceed the 40 ms deadline — \
             otherwise the reset-on-progress property is vacuously true"
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
            crate::error::Error::Fpss { kind, message } => {
                assert_eq!(kind, crate::error::FpssErrorKind::ProtocolError);
                assert!(message.contains("mid-payload"), "got: {message}");
            }
            other => panic!("expected fatal ProtocolError, got {other:?}"),
        }
    }

    /// Same three-property set but for the header path.
    #[test]
    fn mid_header_stall_past_deadline_escalates_to_fatal() {
        let mut reader = AlwaysStalledAfter {
            prefix: vec![0x05],
            pos: 0,
            err_kind: std::io::ErrorKind::WouldBlock,
        };
        let err = read_header_with_timeout(&mut reader, Duration::from_millis(20)).unwrap_err();
        match err {
            crate::error::Error::Fpss { kind, message } => {
                assert_eq!(kind, crate::error::FpssErrorKind::ProtocolError);
                assert!(
                    message.contains("mid-header")
                        && message.contains("without progress for 20 ms"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected fatal ProtocolError, got {other:?}"),
        }
    }

    /// Truncated header (EOF after one byte) must be fatal — same desync
    /// semantics as a mid-header timeout, different IO class.
    #[test]
    fn mid_header_eof_escalates_to_fatal() {
        let data = vec![0x05]; // one byte, then cursor hits end
        let mut cursor = Cursor::new(data);
        let err = read_frame(&mut cursor).unwrap_err();
        match err {
            crate::error::Error::Fpss { kind, message } => {
                assert_eq!(kind, crate::error::FpssErrorKind::ProtocolError);
                assert!(message.contains("truncated FPSS header"));
            }
            other => panic!("expected fatal ProtocolError, got {other:?}"),
        }
    }

    #[test]
    fn disconnected_frame() {
        // DISCONNECTED (code 12) carries a 2-byte BE reason code
        let reason_bytes = 6i16.to_be_bytes(); // AccountAlreadyConnected
        let data = encode_manual(StreamMsgType::Disconnected as u8, &reason_bytes);
        let mut cursor = Cursor::new(data);
        let frame = read_frame(&mut cursor).unwrap().unwrap();
        assert_eq!(frame.code, StreamMsgType::Disconnected);
        assert_eq!(frame.payload.len(), 2);
        let reason = i16::from_be_bytes([frame.payload[0], frame.payload[1]]);
        assert_eq!(reason, 6);
    }
}
