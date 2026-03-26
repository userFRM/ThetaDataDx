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

use crate::types::enums::StreamMsgType;

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
    pub fn new(code: StreamMsgType, payload: Vec<u8>) -> Self {
        assert!(
            payload.len() <= MAX_PAYLOAD_LEN,
            "FPSS frame payload exceeds 255 bytes: {}",
            payload.len()
        );
        Self { code, payload }
    }
}

/// Read a single FPSS frame from a blocking reader.
///
/// # Wire format (from `PacketStream.readFrame()`)
///
/// Reads exactly `[LEN: u8] [CODE: u8]`, then reads `LEN` bytes of payload.
///
/// Returns `None` on clean EOF (reader closed). Returns `Err` on partial reads
/// or unknown message codes.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Frame>, crate::error::Error> {
    // Read the 2-byte header.
    // Only treat as clean EOF if zero bytes were read (true connection close).
    // A partial header (1 byte read then EOF) indicates framing corruption.
    let mut header = [0u8; 2];
    let mut header_read = 0usize;
    loop {
        match reader.read(&mut header[header_read..]) {
            Ok(0) => {
                if header_read == 0 {
                    // Clean EOF -- no bytes read at all.
                    return Ok(None);
                }
                // Partial header -- got some bytes then EOF. This is framing corruption.
                return Err(crate::error::Error::FpssProtocol(format!(
                    "truncated FPSS header: got {header_read} byte(s), expected 2"
                )));
            }
            Ok(n) => {
                header_read += n;
                if header_read >= 2 {
                    break;
                }
                // Got 1 byte, loop to read the second.
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                if header_read == 0 {
                    return Ok(None);
                }
                return Err(crate::error::Error::FpssProtocol(format!(
                    "truncated FPSS header: got {header_read} byte(s), expected 2"
                )));
            }
            Err(e) => return Err(e.into()),
        }
    }

    let payload_len = header[0] as usize;
    let code_byte = header[1];

    let code = StreamMsgType::from_code(code_byte).ok_or_else(|| {
        crate::error::Error::FpssProtocol(format!("unknown message code: {code_byte}"))
    })?;

    // Read the payload
    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        reader.read_exact(&mut payload)?;
    }

    Ok(Some(Frame { code, payload }))
}

/// Write a single FPSS frame to a blocking writer.
///
/// # Wire format (from `PacketStream.writeFrame()`)
///
/// Writes `[LEN: u8] [CODE: u8] [PAYLOAD: LEN bytes]` and flushes.
///
/// Returns `Err` if the payload exceeds 255 bytes.
pub fn write_frame<W: Write>(writer: &mut W, frame: &Frame) -> Result<(), crate::error::Error> {
    if frame.payload.len() > MAX_PAYLOAD_LEN {
        return Err(crate::error::Error::FpssProtocol(format!(
            "frame payload too large: {} bytes (max {})",
            frame.payload.len(),
            MAX_PAYLOAD_LEN
        )));
    }

    let header = [frame.payload.len() as u8, frame.code as u8];
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
/// Source: Java terminal only flushes on ping frames, letting BufWriter
/// batch other writes for better throughput.
pub fn write_raw_frame_no_flush<W: Write>(
    writer: &mut W,
    code: StreamMsgType,
    payload: &[u8],
) -> Result<(), crate::error::Error> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(crate::error::Error::FpssProtocol(format!(
            "frame payload too large: {} bytes (max {})",
            payload.len(),
            MAX_PAYLOAD_LEN
        )));
    }

    let header = [payload.len() as u8, code as u8];
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
    fn read_frame_unknown_code() {
        let data = encode_manual(0xFF, &[]);
        let mut cursor = Cursor::new(data);
        let err = read_frame(&mut cursor).unwrap_err();
        assert!(err.to_string().contains("unknown message code: 255"));
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
