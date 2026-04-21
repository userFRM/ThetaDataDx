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

/// Wall-clock cap on the mid-frame retry budget.
///
/// The I/O loop drains outbound commands (ping heartbeats, subscribe /
/// unsubscribe writes, shutdown) between `read_frame_into` returns. The
/// 50 ms blocking-read timeout on the TLS socket is the primary drain
/// cadence. A partial-frame arrival used to loop internally for up to
/// `READ_TIMEOUT_MS` (10 s) — and indefinitely when bytes kept
/// trickling in before each per-stall deadline — which stalled the
/// command drain for seconds at a time.
///
/// This cap bounds the total wall-clock time the mid-frame reader will
/// spend retrying before it yields control back to the I/O loop via
/// `FpssErrorKind::ProtocolError` with a `DRAIN_YIELD_MARKER` substring.
/// At 200 ms it is 4× the 50 ms drain cadence, generous enough for
/// normal TCP gaps yet tight enough that a pathological trickler cannot
/// block heartbeats past the 2 s Java-side ping grace.
pub const MID_FRAME_DRAIN_WINDOW_MS: u64 = 200;

/// Marker substring the I/O loop matches to recognise a drain-yield
/// error. The full error is a regular `ProtocolError` so it fits the
/// existing error taxonomy; the substring lets the loop pattern-match
/// without plumbing a new enum variant through every downstream
/// consumer (FFI / Python bindings / SDK surfaces).
pub(crate) const DRAIN_YIELD_MARKER: &str = "drain-yield: bounded retry budget exceeded";

/// Per-frame read resumption state.
///
/// Threaded through `read_frame_into` so the I/O loop can yield to
/// drain outbound commands mid-frame and then re-enter the reader
/// without losing the bytes already delivered by the TLS socket. Each
/// call to `read_frame_into` reads zero or more bytes, updates the
/// state in place, and either returns a fully decoded frame
/// (`Ok(Some(...))`) or yields (`Err(ProtocolError)` carrying the
/// `DRAIN_YIELD_MARKER` substring).
///
/// The state is sized with the wire layout: 2 header bytes then up to
/// `MAX_PAYLOAD_LEN` payload bytes. No heap allocations on the hot
/// path — the embedded `header_buf` is a fixed `[u8; 2]` and the
/// payload bytes live in the caller-owned `Vec<u8>`.
#[derive(Debug, Default, Clone)]
pub struct FrameReadState {
    /// Bytes read so far into the 2-byte header. 0, 1, or 2.
    header_read: u8,
    /// Decoded header bytes. Meaningful only once `header_read == 2`.
    header_buf: [u8; 2],
    /// `true` once the header is complete and we're reading payload.
    payload_phase: bool,
    /// Bytes read so far into the payload. 0..=payload_len.
    payload_read: usize,
    /// Expected payload length. Meaningful only during `payload_phase`.
    payload_len: usize,
    /// Count of consecutive unknown codes observed during this
    /// resumption chain. Resets when a known frame is returned.
    consecutive_unknown: usize,
}

impl FrameReadState {
    /// Build a fresh resumption state for the start of a new frame.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when no bytes of the current frame have been read yet.
    /// The I/O loop uses this to skip the drain-yield error re-entry
    /// when the reader is idle (fresh frame boundary) -- in that
    /// case the caller can treat the error as "try again later"
    /// without worrying about desync.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.header_read == 0 && !self.payload_phase
    }
}

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
/// and the production [`MID_FRAME_DRAIN_WINDOW_MS`] drain budget into
/// [`read_header_with_timeout`]. Splitting the function lets tests
/// exercise the timeout/retry contract with a short deadline and a
/// short drain budget independently.
fn read_header<R: Read>(
    reader: &mut R,
    state: &mut FrameReadState,
) -> Result<Option<[u8; 2]>, crate::error::Error> {
    read_header_with_timeout(
        reader,
        state,
        Duration::from_millis(READ_TIMEOUT_MS),
        Duration::from_millis(MID_FRAME_DRAIN_WINDOW_MS),
    )
}

/// Read the 2-byte FPSS header with configurable per-stall timeout
/// and drain-yield budget.
///
/// Byte-for-byte match with Java `PacketStream2.readCopy` +
/// `socket.setSoTimeout(10_000)` on the per-stall half; a
/// `drain_budget` yield on top. Contract:
/// - EOF before any byte → `Ok(None)` (graceful server close).
/// - Pre-header `WouldBlock` / `TimedOut` (n == 0) → propagate as
///   `Error::Io` so `io_loop::is_read_timeout` drains pings +
///   command queue.
/// - Mid-header `WouldBlock` / `TimedOut` (n > 0) → retry. The
///   stall deadline is **re-armed on every successful byte**,
///   matching Java's per-`read()` `setSoTimeout` semantics. Fatal
///   if `stall_timeout` elapses without any forward progress.
/// - Mid-header retries exceeding `drain_budget` of aggregate
///   wall-clock time → return a `ProtocolError` carrying the
///   [`DRAIN_YIELD_MARKER`] so the I/O loop recognizes the yield,
///   drains outbound commands, and re-enters the reader. The
///   partial header bytes are preserved on `state` so the next
///   call resumes from byte `state.header_read` without desync.
///
/// Earlier revisions treated the first mid-header `WouldBlock` as
/// fatal desync. Captured raw-byte dumps on dev + prod showed the
/// bytes were valid frames that arrived 50-76 ms after the first
/// `WouldBlock`; aggressive escalation caused a reconnect storm
/// whose downstream effects accounted for the spurious "unknown
/// message code" reports (#192, #369).
fn read_header_with_timeout<R: Read>(
    reader: &mut R,
    state: &mut FrameReadState,
    stall_timeout: Duration,
    drain_budget: Duration,
) -> Result<Option<[u8; 2]>, crate::error::Error> {
    let mut stall_deadline: Option<Instant> = None;
    let drain_deadline: Option<Instant> =
        // Only arm the drain budget on the first call that makes any
        // progress: a fresh invocation with zero bytes in flight must
        // keep the pre-header WouldBlock semantics unchanged (the
        // caller drains pings + commands on `Error::Io`).
        if state.header_read > 0 {
            Some(Instant::now() + drain_budget)
        } else {
            None
        };
    loop {
        let n = state.header_read as usize;
        match reader.read(&mut state.header_buf[n..]) {
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
                let new_n = n + read;
                state.header_read = u8::try_from(new_n).unwrap_or(2);
                if new_n >= 2 {
                    return Ok(Some(state.header_buf));
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
                // Drain-yield: the aggregate wall-clock cap exists so
                // the command drain cannot be starved by a trickling
                // sender. The partial header bytes are preserved on
                // `state` so the next call resumes at byte
                // `state.header_read`.
                let now = Instant::now();
                if let Some(db) = drain_deadline {
                    if now >= db {
                        return Err(crate::error::Error::Fpss {
                            kind: crate::error::FpssErrorKind::ProtocolError,
                            message: format!(
                                "{DRAIN_YIELD_MARKER}: mid-header after {n} of 2 byte(s) \
                                 (budget {} ms): {e}",
                                drain_budget.as_millis()
                            ),
                        });
                    }
                }
                let deadline = *stall_deadline.get_or_insert(now + stall_timeout);
                if now >= deadline {
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
/// and the [`MID_FRAME_DRAIN_WINDOW_MS`] drain budget into
/// [`read_exact_payload_with_timeout`]. Splitting the function lets
/// tests exercise the timeout/retry contract with a short deadline.
fn read_exact_payload<R: Read>(
    reader: &mut R,
    buf: &mut [u8],
    state: &mut FrameReadState,
) -> Result<(), crate::error::Error> {
    read_exact_payload_with_timeout(
        reader,
        buf,
        state,
        Duration::from_millis(READ_TIMEOUT_MS),
        Duration::from_millis(MID_FRAME_DRAIN_WINDOW_MS),
    )
}

/// Read exactly `buf.len()` bytes of payload with configurable
/// per-stall timeout and drain-yield budget.
///
/// Matches Java `DataInputStream.readNBytes` +
/// `setSoTimeout(10_000)` on the per-stall half: every `WouldBlock` /
/// `TimedOut` re-arms a fresh `stall_timeout` deadline from the last
/// successful byte of progress, not from function entry. A stream
/// that dribbles data in slowly but steadily is fine; only
/// `stall_timeout` of total silence fails.
///
/// `drain_budget` is the aggregate wall-clock cap that bounds the
/// total time this function can spend retrying before yielding
/// control to the I/O loop. On yield the function returns a
/// `ProtocolError` carrying [`DRAIN_YIELD_MARKER`] and updates
/// `state.payload_read` so the next call resumes from the exact byte
/// offset. Without this budget, a pathological sender dribbling one
/// byte every `stall_timeout - 1 ms` could block the command drain
/// for up to `stall_timeout * payload_len / byte` seconds -- the
/// ping / subscribe / shutdown queue would stall behind it.
///
/// Earlier revisions treated the first `WouldBlock` as a fatal
/// desync. Raw-byte tap captures on dev + prod showed every
/// "corruption" event was a valid frame that finished arriving
/// 50-76 ms after the first `WouldBlock`. Java tolerates that gap
/// silently; so do we now, bounded by the drain-yield budget.
///
/// `Interrupted` is retried (POSIX signal wakeups are benign).
/// `EOF` and going `stall_timeout` without progress are still fatal.
fn read_exact_payload_with_timeout<R: Read>(
    reader: &mut R,
    buf: &mut [u8],
    state: &mut FrameReadState,
    stall_timeout: Duration,
    drain_budget: Duration,
) -> Result<(), crate::error::Error> {
    let mut stall_deadline: Option<Instant> = None;
    let drain_deadline: Instant = Instant::now() + drain_budget;
    while state.payload_read < buf.len() {
        let n = state.payload_read;
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
                state.payload_read = n + k;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // Drain-yield: the aggregate wall-clock cap exists so
                // the command drain cannot be starved by a trickling
                // sender. The partial payload bytes are preserved
                // on `state.payload_read` so the next call resumes
                // from the exact byte offset.
                let now = Instant::now();
                if now >= drain_deadline {
                    return Err(crate::error::Error::Fpss {
                        kind: crate::error::FpssErrorKind::ProtocolError,
                        message: format!(
                            "{DRAIN_YIELD_MARKER}: mid-payload after {n} of {} byte(s) \
                             (budget {} ms): {e}",
                            buf.len(),
                            drain_budget.as_millis()
                        ),
                    });
                }
                let deadline = *stall_deadline.get_or_insert(now + stall_timeout);
                if now >= deadline {
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
/// # Resumable reads
///
/// The `state` argument carries partial-frame progress across calls.
/// On a drain-yield error the state is updated to record the exact
/// byte offset in the header or payload, so the next call resumes
/// without desync. When a full frame is returned, the state is
/// reset to idle for the next frame.
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
    state: &mut FrameReadState,
) -> Result<Option<(StreamMsgType, usize)>, crate::error::Error> {
    loop {
        // Header phase: read bytes into `state.header_buf` until we
        // have both. A drain-yield here preserves partial progress
        // via `state.header_read`.
        if !state.payload_phase {
            let Some(header) = read_header(reader, state)? else {
                // Clean EOF before any byte of this frame — reset the
                // state to idle so the next caller-driven frame starts
                // fresh if the stream reopens (reconnect path).
                *state = FrameReadState::new();
                return Ok(None);
            };

            let payload_len = header[0] as usize;
            state.payload_len = payload_len;
            state.payload_phase = true;
            state.payload_read = 0;

            // Always consume the payload to keep the stream aligned.
            // The `buf.clear()` + `buf.resize()` is done here, at
            // header completion, so we only allocate once per frame.
            buf.clear();
            buf.resize(payload_len, 0);
        }

        // Payload phase: read bytes until `payload_len` are in buf.
        // A drain-yield here preserves `state.payload_read`.
        let payload_len = state.payload_len;
        let code_byte = state.header_buf[1];
        if payload_len > 0 {
            read_exact_payload(reader, &mut buf[..payload_len], state)?;
        }

        // Frame complete — decide how to return based on the code.
        // `state` is reset AFTER the code classification so a
        // drain-yield above (pre-classification) would have carried
        // the partial state through unchanged.
        let consecutive_unknown = state.consecutive_unknown;
        let reset_state = FrameReadState::new();
        *state = reset_state;

        if let Some(code) = StreamMsgType::from_code(code_byte) {
            return Ok(Some((code, payload_len)));
        }
        let new_consecutive = consecutive_unknown + 1;
        if new_consecutive >= MAX_CONSECUTIVE_UNKNOWN_CODES {
            return Err(crate::error::Error::Fpss {
                kind: crate::error::FpssErrorKind::ProtocolError,
                message: format!(
                    "framing corruption: {new_consecutive} consecutive \
                     unknown message codes (last code: {code_byte})"
                ),
            });
        }
        state.consecutive_unknown = new_consecutive;
        tracing::debug!(
            code = code_byte,
            payload_len,
            consecutive_unknown = new_consecutive,
            "skipping unknown FPSS message code"
        );
    }
}

/// Read a single FPSS frame from a blocking reader.
///
/// Convenience wrapper that allocates a fresh `Vec<u8>` and
/// `FrameReadState` per call, and **transparently retries on
/// drain-yield**: the handshake and test paths that call this helper
/// do not have a command drain to service, so the yield budget is
/// not meaningful for them. A drain-yield here loops back into the
/// reader with the same state so partial progress is preserved.
///
/// Prefer `read_frame_into` on the hot path where the caller reuses
/// the buffer and state across frames AND owns a command drain that
/// needs to service yields.
///
/// Returns `None` on clean EOF (reader closed). Returns `Err` on
/// partial reads, stall-timeouts, or unknown message codes.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Frame>, crate::error::Error> {
    let mut buf = Vec::new();
    let mut state = FrameReadState::new();
    loop {
        match read_frame_into(reader, &mut buf, &mut state) {
            Ok(Some((code, _len))) => {
                return Ok(Some(Frame { code, payload: buf }));
            }
            Ok(None) => return Ok(None),
            Err(ref e) if is_drain_yield(e) => {
                // No command drain to service on this path -- loop
                // back immediately with the same resumption state so
                // partial bytes are not lost.
                continue;
            }
            Err(e) => return Err(e),
        }
    }
}

/// `true` if an error is a drain-yield from a mid-frame reader. The
/// I/O loop catches this in the read branch, drains outbound commands,
/// and re-enters `read_frame_into` with the same `FrameReadState` so
/// the partial frame resumes byte-for-byte.
pub fn is_drain_yield(err: &crate::error::Error) -> bool {
    matches!(
        err,
        crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ProtocolError,
            message,
        } if message.contains(DRAIN_YIELD_MARKER)
    )
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
        let mut state = FrameReadState::new();
        // First two payload bytes land; next reads stall forever. Use a
        // generous drain budget (1 s) so the stall-deadline escalation
        // fires first -- this test asserts the per-stall failure path.
        let err = read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_millis(20),
            Duration::from_secs(1),
        )
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
        let mut state = FrameReadState::new();
        let started = Instant::now();
        // Use a drain budget (500 ms) that's larger than the aggregate
        // wall time (120 ms) so this test pins the per-stall reset
        // semantics independently of the drain-yield path.
        read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_millis(40),
            Duration::from_millis(500),
        )
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
        let mut state = FrameReadState::new();
        // Drain budget is generous (1 s) so the stall-deadline wins.
        let err = read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_millis(20),
            Duration::from_secs(1),
        )
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
        let mut state = FrameReadState::new();
        // Drain budget generous (1 s) so the per-stall deadline trips first.
        let err = read_header_with_timeout(
            &mut reader,
            &mut state,
            Duration::from_millis(20),
            Duration::from_secs(1),
        )
        .unwrap_err();
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

    // -- Finding #3 coverage: drain-yield + bounded retry budget -----------
    //
    // The mid-frame reader must not block the command drain for more than
    // a small multiple of the 50 ms command-drain cadence. Previously a
    // trickling sender could loop internally for up to 10 s (the full
    // READ_TIMEOUT_MS) because the per-stall deadline reset on every
    // successful byte. Now the aggregate drain-yield budget caps the total
    // wall time in the reader; exceeding it returns a
    // `ProtocolError` tagged with `DRAIN_YIELD_MARKER` so the I/O loop
    // drains commands and re-enters with the preserved `FrameReadState`.

    /// Bytes-at-a-time producer: delivers ONE byte, then WouldBlock
    /// forever, then one more byte, etc. Each WouldBlock sleeps for
    /// `sleep_per_stall` so the test can measure wall-clock behaviour
    /// deterministically.
    struct OneByteAtATime {
        remaining: Vec<u8>,
        sleep_per_stall: Duration,
        stalled_since_last_byte: bool,
    }

    impl std::io::Read for OneByteAtATime {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.remaining.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "exhausted; idle stall",
                ));
            }
            if !self.stalled_since_last_byte {
                self.stalled_since_last_byte = true;
                std::thread::sleep(self.sleep_per_stall);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "pre-byte stall",
                ));
            }
            buf[0] = self.remaining.remove(0);
            self.stalled_since_last_byte = false;
            Ok(1)
        }
    }

    /// Finding #3: a bytes-at-a-time producer with a 30 ms stall per
    /// byte must NOT keep the mid-payload reader occupied past the
    /// drain-yield budget. With an 80-byte payload and a 100 ms
    /// drain budget, the reader must yield before consuming the
    /// whole payload, preserving partial state. This is the exact
    /// scenario the command drain needs to service heartbeats.
    #[test]
    fn mid_payload_bytes_at_a_time_yields_before_full_payload() {
        let payload_size = 80;
        let mut reader = OneByteAtATime {
            remaining: (0..u8::try_from(payload_size).unwrap()).collect(),
            sleep_per_stall: Duration::from_millis(30),
            stalled_since_last_byte: false,
        };
        let mut payload = vec![0u8; payload_size];
        let mut state = FrameReadState::new();
        let started = Instant::now();
        // Stall per-byte deadline = 60 ms (longer than the per-byte
        // sleep so the stall-deadline alone would NOT trip).
        // Drain budget = 100 ms (shorter than the aggregate 30 × 80 =
        // 2400 ms wall time the trickler would otherwise burn).
        let err = read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_millis(60),
            Duration::from_millis(100),
        )
        .unwrap_err();
        let elapsed = started.elapsed();

        // The error must be a drain-yield, not a per-stall fatal.
        match &err {
            crate::error::Error::Fpss { kind, message } => {
                assert_eq!(*kind, crate::error::FpssErrorKind::ProtocolError);
                assert!(
                    message.contains(DRAIN_YIELD_MARKER),
                    "expected drain-yield marker, got: {message}"
                );
                assert!(
                    message.contains("mid-payload"),
                    "expected mid-payload context, got: {message}"
                );
            }
            other => panic!("expected drain-yield Fpss error, got {other:?}"),
        }
        assert!(
            is_drain_yield(&err),
            "is_drain_yield helper must match the produced error"
        );

        // Wall time bound: must not exceed the budget by more than
        // one stall (allow 30 ms slack for the per-byte sleep that
        // was already in flight when the deadline tripped).
        assert!(
            elapsed < Duration::from_millis(160),
            "drain-yield must fire promptly; elapsed = {elapsed:?}, \
             budget was 100 ms + 30 ms stall slack"
        );

        // Partial progress must be preserved: at least SOME bytes
        // should have landed (the stall completed at least one
        // iteration before the deadline fired), and strictly less
        // than the full payload (the yield MUST happen before
        // completion).
        assert!(
            state.payload_read > 0,
            "partial payload bytes must have landed before the yield"
        );
        assert!(
            state.payload_read < payload_size,
            "yield must happen BEFORE the full payload arrives; got \
             {} of {}",
            state.payload_read,
            payload_size
        );
    }

    /// Finding #3: after a drain-yield, the next call to
    /// `read_exact_payload_with_timeout` with the SAME state must
    /// resume from the exact byte offset. No bytes are lost, none
    /// are re-read. This is the invariant that makes the yield
    /// safe for the command-drain re-entry pattern.
    #[test]
    fn drain_yield_preserves_payload_read_for_resumption() {
        // 4-byte payload delivered one byte at a time. First call
        // yields after the drain budget; second call completes
        // the payload without losing any bytes.
        let mut reader = OneByteAtATime {
            remaining: vec![0x11, 0x22, 0x33, 0x44],
            sleep_per_stall: Duration::from_millis(25),
            stalled_since_last_byte: false,
        };
        let mut payload = vec![0u8; 4];
        let mut state = FrameReadState::new();

        // First call: 60 ms drain budget -> yields somewhere
        // between byte 2 and byte 3.
        let err = read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_millis(100),
            Duration::from_millis(60),
        )
        .unwrap_err();
        assert!(is_drain_yield(&err), "first call must drain-yield");
        let first_read = state.payload_read;
        assert!(
            first_read > 0 && first_read < 4,
            "first call must land some but not all bytes; got {first_read}"
        );

        // Second call: finish the payload. The state carries the
        // first-call progress so no bytes are re-read.
        read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_millis(100),
            Duration::from_secs(1),
        )
        .expect("second call must complete the payload without error");
        assert_eq!(
            state.payload_read, 4,
            "state must reflect full payload completion"
        );
        assert_eq!(
            payload,
            vec![0x11, 0x22, 0x33, 0x44],
            "every byte of the payload must land in the correct order"
        );
    }

    /// Finding #3: the public `read_frame_into` entry point must
    /// propagate drain-yields out to the caller (so the I/O loop
    /// can service the command drain) and then resume cleanly on
    /// the next call with the same state + buffer. This exercises
    /// the full public API end-to-end.
    ///
    /// `OneByteAtATime` emits a `WouldBlock` before every byte; the
    /// very first `WouldBlock` is pre-header (zero bytes in flight)
    /// and surfaces as `Error::Io(WouldBlock)` -- the io_loop
    /// treats this as a benign drain-cadence tick. Every subsequent
    /// `WouldBlock` is mid-frame and may surface as a drain-yield
    /// once the aggregate budget trips. The retry loop here accepts
    /// both classes and spins until the full frame lands.
    #[test]
    fn read_frame_into_drain_yield_round_trip() {
        // Build a 6-byte payload frame: [LEN=6, CODE=PING, 6 bytes...]
        let mut wire = vec![0x06, StreamMsgType::Ping as u8];
        wire.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let mut reader = OneByteAtATime {
            remaining: wire,
            sleep_per_stall: Duration::from_millis(25),
            stalled_since_last_byte: false,
        };
        let mut buf: Vec<u8> = Vec::new();
        let mut state = FrameReadState::new();

        // Spin until the frame completes, treating both drain-yields
        // (mid-frame yield to caller) and pre-header IO WouldBlocks
        // (caller-drain cadence) as benign retries. This mirrors the
        // production io_loop's read branch.
        let mut yields = 0u32;
        let mut idle_ticks = 0u32;
        let started = Instant::now();
        let frame = loop {
            match read_frame_into(&mut reader, &mut buf, &mut state) {
                Ok(Some(frame)) => break frame,
                Ok(None) => panic!("unexpected EOF"),
                Err(ref e) if is_drain_yield(e) => {
                    yields += 1;
                    if started.elapsed() > Duration::from_secs(5) {
                        panic!("test timeout waiting for frame");
                    }
                    continue;
                }
                Err(crate::error::Error::Io(ref io_err))
                    if io_err.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    idle_ticks += 1;
                    if started.elapsed() > Duration::from_secs(5) {
                        panic!("test timeout waiting for frame");
                    }
                    continue;
                }
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        };

        assert_eq!(frame.0, StreamMsgType::Ping);
        assert_eq!(frame.1, 6);
        assert_eq!(buf, vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        // Under production drain budget the yield may or may not
        // fire depending on timer granularity; the key property
        // the test pins is that ANY yield is followed by a clean
        // resumption. So tolerate zero yields but assert the frame
        // completed correctly.
        assert!(
            yields + idle_ticks <= 32,
            "retries should not loop forever; yields={yields}, \
             idle_ticks={idle_ticks}"
        );
    }

    /// Finding #3: the `is_drain_yield` helper must correctly
    /// classify every drain-yield error shape the readers produce,
    /// and reject unrelated errors. The I/O loop relies on this
    /// classifier to decide whether to drain-and-resume or
    /// escalate to reconnect.
    #[test]
    fn is_drain_yield_classifier_is_precise() {
        // Drain-yield with the expected marker -> true.
        let dy = crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ProtocolError,
            message: format!("{DRAIN_YIELD_MARKER}: mid-payload after 3 of 8 byte(s)"),
        };
        assert!(is_drain_yield(&dy));

        // Unrelated protocol error -> false.
        let other = crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ProtocolError,
            message: "truncated FPSS header: got 1 byte(s), expected 2".to_string(),
        };
        assert!(!is_drain_yield(&other));

        // IO error -> false (drain-yield is Fpss-kind only).
        let io = crate::error::Error::Io(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "pre-header",
        ));
        assert!(!is_drain_yield(&io));
    }
}
