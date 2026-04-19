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

use tdbe::types::enums::StreamMsgType;

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
/// Timeout semantics (Java-aligned, `FPSSClient.onEvent` → reconnect):
/// - If `reader.read` returns EOF before any byte is read, return `Ok(None)` —
///   graceful server close.
/// - If `WouldBlock` / `TimedOut` fires BEFORE any byte is read, propagate as
///   `std::io::Error` so `io_loop::is_read_timeout` can drive ping/command
///   housekeeping and resume.
/// - If either fires AFTER at least 1 byte has been read, we are mid-frame
///   and the cursor is desynced. Return a FATAL `FpssErrorKind::ProtocolError`
///   so `io_loop` treats it as involuntary disconnect and reconnects —
///   matches Java's `PacketStream.readCopy` → `SocketTimeoutException` →
///   `handleInvoluntaryDisconnect` path.
fn read_header<R: Read>(reader: &mut R) -> Result<Option<[u8; 2]>, crate::error::Error> {
    let mut header = [0u8; 2];
    let mut n = 0usize;
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
                // Mid-header timeout: stream cursor is desynced by `n` bytes.
                // Treat as fatal so io_loop reconnects instead of re-reading
                // payload bytes as a header.
                return Err(crate::error::Error::Fpss {
                    kind: crate::error::FpssErrorKind::ProtocolError,
                    message: format!("mid-header read timeout after {n} of 2 byte(s): {e}"),
                });
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Read exactly `buf.len()` bytes of payload, treating mid-frame stalls as
/// fatal. Once the header has committed us to N payload bytes, a timeout
/// before all N arrive means the reader cursor is desynced — abandon the
/// connection and reconnect rather than retry.
///
/// Only `Interrupted` is retried (POSIX signal wakeups are benign); any
/// `WouldBlock` / `TimedOut` mid-payload is escalated.
fn read_exact_payload<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<(), crate::error::Error> {
    let mut n = 0usize;
    while n < buf.len() {
        match reader.read(&mut buf[n..]) {
            Ok(0) => {
                return Err(crate::error::Error::Fpss {
                    kind: crate::error::FpssErrorKind::ProtocolError,
                    message: format!("EOF mid-payload: got {n} of {} bytes", buf.len()),
                })
            }
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(crate::error::Error::Fpss {
                    kind: crate::error::FpssErrorKind::ProtocolError,
                    message: format!(
                        "mid-payload read timeout after {n} of {} byte(s): {e}",
                        buf.len()
                    ),
                });
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

    /// Mid-header `WouldBlock` (one byte delivered, second byte stalls)
    /// must escalate to fatal `ProtocolError` — the cursor is desynced by
    /// one byte and retrying would read a payload byte as a frame header.
    #[test]
    fn mid_header_timeout_escalates_to_fatal() {
        let mut reader = PrefixThenErr {
            prefix: vec![0x05],
            pos: 0,
            err_kind: std::io::ErrorKind::WouldBlock,
        };
        let err = read_frame(&mut reader).unwrap_err();
        match err {
            crate::error::Error::Fpss { kind, message } => {
                assert_eq!(kind, crate::error::FpssErrorKind::ProtocolError);
                assert!(
                    message.contains("mid-header"),
                    "expected mid-header message, got: {message}"
                );
            }
            other => panic!("expected fatal ProtocolError, got {other:?}"),
        }
    }

    /// Mid-payload `TimedOut` (header + partial payload delivered, remaining
    /// payload stalls) must escalate to fatal `ProtocolError`. Retrying would
    /// leave `N - k` payload bytes in the stream that the next `read_frame`
    /// would parse as a bogus header.
    #[test]
    fn mid_payload_timeout_escalates_to_fatal() {
        // header: len=4, code=PING; then 2 of 4 payload bytes before the stall.
        let mut reader = PrefixThenErr {
            prefix: vec![0x04, StreamMsgType::Ping as u8, 0x01, 0x02],
            pos: 0,
            err_kind: std::io::ErrorKind::TimedOut,
        };
        let err = read_frame(&mut reader).unwrap_err();
        match err {
            crate::error::Error::Fpss { kind, message } => {
                assert_eq!(kind, crate::error::FpssErrorKind::ProtocolError);
                assert!(
                    message.contains("mid-payload"),
                    "expected mid-payload message, got: {message}"
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
