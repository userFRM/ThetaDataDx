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
/// `StreamErrorKind::ProtocolError` with a `DRAIN_YIELD_MARKER` substring.
/// At 200 ms it is 4× the 50 ms drain cadence, generous enough for
/// normal TCP gaps yet tight enough that a pathological trickler cannot
/// block heartbeats past the 2 s ping grace.
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
    /// Hard wall-clock deadline for the in-progress frame, armed when
    /// the first byte of the frame is read and carried across every
    /// resumed `read_frame_into*` / drain-yield re-entry. `None` until
    /// the first byte lands; cleared (with the rest of the state) when
    /// the frame completes.
    ///
    /// The per-stall and per-call drain budgets both re-arm on progress
    /// (stall) or per invocation (drain), so neither bounds the *total*
    /// time a single frame may occupy: a peer trickling one byte per
    /// resume keeps both clocks fresh forever. This deadline is the one
    /// budget that accumulates across resumptions, so a partial frame
    /// can never be held past the configured cap — once it expires the
    /// reader fails fatally and the I/O loop's reconnect/watchdog fires.
    frame_deadline: Option<Instant>,
}

impl FrameReadState {
    /// Build a fresh resumption state for the start of a new frame.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Read the 2-byte FPSS header with configurable per-stall timeout
/// and drain-yield budget.
///
/// Byte-for-byte match with the JVM terminal's framed read under a
/// 10_000 ms per-stall socket timeout; a `drain_budget` yield on top.
/// Contract:
/// - EOF before any byte → `Ok(None)` (graceful server close).
/// - Pre-header `WouldBlock` / `TimedOut` (n == 0) → propagate as
///   `Error::Io` so `io_loop::is_read_timeout` drains pings +
///   command queue.
/// - Mid-header `WouldBlock` / `TimedOut` (n > 0) → retry. The
///   stall deadline is **re-armed on every successful byte**,
///   matching the JVM terminal's per-`read()` socket-timeout
///   semantics. Fatal
///   if `stall_timeout` elapses without any forward progress.
/// - Mid-header retries exceeding `drain_budget` of aggregate
///   wall-clock time → return a `ProtocolError` carrying the
///   [`DRAIN_YIELD_MARKER`] so the I/O loop recognizes the yield,
///   drains outbound commands, and re-enters the reader. The
///   partial header bytes are preserved on `state` so the next
///   call resumes from byte `state.header_read` without desync.
/// - The in-progress frame exceeding `frame_cap` of total wall-clock
///   time (measured from the first byte of the frame, persisted on
///   `state.frame_deadline` so it accumulates across resumed calls) →
///   fatal `ProtocolError`. The per-stall and drain budgets both
///   re-arm on progress / per call, so only this cap stops a peer that
///   trickles one byte per resume from holding a single header forever.
///
/// Earlier revisions treated the first mid-header `WouldBlock` as
/// fatal desync. Captured raw-byte dumps on dev + prod showed the
/// bytes were valid frames that arrived 50-76 ms after the first
/// `WouldBlock`; aggressive escalation caused a reconnect storm
/// whose downstream effects accounted for the spurious "unknown
/// message code" reports.
fn read_header_with_timeout<R: Read>(
    reader: &mut R,
    state: &mut FrameReadState,
    stall_timeout: Duration,
    drain_budget: Duration,
    frame_cap: Duration,
) -> Result<Option<[u8; 2]>, crate::error::Error> {
    let mut stall_deadline: Option<Instant> = None;
    // Arm the drain budget the moment ANY header byte is in flight —
    // whether it was delivered by a prior call (`state.header_read > 0`
    // on entry) or by the first successful read inside this call. A
    // fresh invocation with zero bytes in flight keeps the pre-header
    // WouldBlock semantics unchanged (the caller drains pings + commands
    // on `Error::Io`); but once the first byte lands, a subsequent
    // mid-header stall must be bounded by the yield cap exactly like the
    // mid-payload path, so it cannot block the command drain for the
    // full `stall_timeout`.
    let mut drain_deadline: Option<Instant> = if state.header_read > 0 {
        Some(Instant::now() + drain_budget)
    } else {
        None
    };
    // Arm the per-frame hard cap if a prior call already banked header
    // bytes for this frame; otherwise it arms below when the first byte
    // of the frame lands. Persisted on `state` so it survives both
    // drain-yields and the header→payload phase transition.
    if state.header_read > 0 {
        state
            .frame_deadline
            .get_or_insert_with(|| Instant::now() + frame_cap);
    }
    loop {
        let n = state.header_read as usize;
        match reader.read(&mut state.header_buf[n..]) {
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
                let new_n = n + read;
                state.header_read = u8::try_from(new_n).unwrap_or(2);
                if new_n >= 2 {
                    return Ok(Some(state.header_buf));
                }
                // First header byte just landed inside this call: arm the
                // drain budget so a stall on the second byte yields at the
                // cap rather than blocking for the full `stall_timeout`,
                // and arm the per-frame hard cap from the same instant so
                // the whole frame is bounded against a trickler.
                drain_deadline.get_or_insert_with(|| Instant::now() + drain_budget);
                state
                    .frame_deadline
                    .get_or_insert_with(|| Instant::now() + frame_cap);
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && n == 0 => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(crate::error::Error::Stream {
                    kind: crate::error::StreamErrorKind::ProtocolError,
                    message: format!("truncated FPSS header: got {n} byte(s), expected 2"),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if n > 0 && is_transient_read(&e) => {
                // Drain-yield: the aggregate wall-clock cap exists so
                // the command drain cannot be starved by a trickling
                // sender. The partial header bytes are preserved on
                // `state` so the next call resumes at byte
                // `state.header_read`.
                let now = Instant::now();
                // Per-frame hard cap first: unlike the drain-yield (which
                // is recoverable and re-armed per call) an exhausted frame
                // deadline is fatal, so the I/O loop's reconnect/watchdog
                // tears down a peer that has trickled a single header past
                // the cap instead of resuming it forever.
                if let Some(fd) = state.frame_deadline {
                    if now >= fd {
                        return Err(crate::error::Error::Stream {
                            kind: crate::error::StreamErrorKind::ProtocolError,
                            message: format!(
                                "mid-header frame deadline exceeded after {n} of 2 byte(s) \
                                 (per-frame cap {} ms): {e}",
                                frame_cap.as_millis()
                            ),
                        });
                    }
                }
                if let Some(db) = drain_deadline {
                    if now >= db {
                        return Err(crate::error::Error::Stream {
                            kind: crate::error::StreamErrorKind::ProtocolError,
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

/// Read exactly `buf.len()` bytes of payload with configurable
/// per-stall timeout and drain-yield budget.
///
/// Matches the JVM terminal's read-N-bytes under a 10_000 ms per-stall
/// socket timeout: every `WouldBlock` /
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
/// `frame_cap` is the per-frame hard cap (persisted on
/// `state.frame_deadline`). Unlike `drain_budget` it accumulates across
/// resumed calls and across the header→payload phase transition, so a
/// peer trickling one byte per resume — which keeps both the per-stall
/// and per-call drain clocks fresh — is still cut off once the whole
/// frame exceeds the cap. Exhausting it is fatal (not a drain-yield) so
/// the I/O loop's reconnect/watchdog fires instead of resuming forever.
///
/// Earlier revisions treated the first `WouldBlock` as a fatal
/// desync. Raw-byte tap captures on dev + prod showed every
/// "corruption" event was a valid frame that finished arriving
/// 50-76 ms after the first `WouldBlock`. The JVM terminal tolerates
/// that gap silently; so do we now, bounded by the drain-yield budget.
///
/// `Interrupted` is retried (POSIX signal wakeups are benign).
/// `EOF` and going `stall_timeout` without progress are still fatal.
fn read_exact_payload_with_timeout<R: Read>(
    reader: &mut R,
    buf: &mut [u8],
    state: &mut FrameReadState,
    stall_timeout: Duration,
    drain_budget: Duration,
    frame_cap: Duration,
) -> Result<(), crate::error::Error> {
    let mut stall_deadline: Option<Instant> = None;
    let drain_deadline: Instant = Instant::now() + drain_budget;
    // Continue (or, for a zero-byte-header frame that somehow reaches
    // here, arm) the per-frame hard cap. In the normal flow the header
    // phase already armed it on the first byte, so this preserves the
    // elapsed header time rather than resetting the budget.
    let frame_deadline: Instant = *state
        .frame_deadline
        .get_or_insert_with(|| Instant::now() + frame_cap);
    while state.payload_read < buf.len() {
        let n = state.payload_read;
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
                state.payload_read = n + k;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if is_transient_read(&e) => {
                // Drain-yield: the aggregate wall-clock cap exists so
                // the command drain cannot be starved by a trickling
                // sender. The partial payload bytes are preserved
                // on `state.payload_read` so the next call resumes
                // from the exact byte offset.
                let now = Instant::now();
                // Per-frame hard cap first: an exhausted frame deadline is
                // fatal (the drain-yield is recoverable and re-armed per
                // call), so a peer trickling one byte per resume is torn
                // down by the I/O loop's reconnect/watchdog once the whole
                // frame exceeds the cap instead of resuming forever.
                if now >= frame_deadline {
                    return Err(crate::error::Error::Stream {
                        kind: crate::error::StreamErrorKind::ProtocolError,
                        message: format!(
                            "mid-payload frame deadline exceeded after {n} of {} byte(s) \
                             (per-frame cap {} ms): {e}",
                            buf.len(),
                            frame_cap.as_millis()
                        ),
                    });
                }
                if now >= drain_deadline {
                    return Err(crate::error::Error::Stream {
                        kind: crate::error::StreamErrorKind::ProtocolError,
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
    state: &mut FrameReadState,
) -> Result<Option<(StreamMsgType, usize)>, crate::error::Error> {
    read_frame_into_with_stall_timeout(reader, buf, state, Duration::from_millis(READ_TIMEOUT_MS))
}

/// Takes the per-stall mid-frame timeout from the caller instead of the
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
    state: &mut FrameReadState,
    stall_timeout: Duration,
) -> Result<Option<(StreamMsgType, usize)>, crate::error::Error> {
    loop {
        // Header phase: read bytes into `state.header_buf` until we
        // have both. A drain-yield here preserves partial progress
        // via `state.header_read`.
        if !state.payload_phase {
            // The per-frame hard cap is tied to `stall_timeout`: a single FPSS
            // frame (≤ 257 bytes on the wire) never legitimately takes longer
            // than the read deadline to fully arrive, so the same budget that
            // bounds one stall also bounds the whole frame against a trickler.
            let header = read_header_with_timeout(
                reader,
                state,
                stall_timeout,
                Duration::from_millis(MID_FRAME_DRAIN_WINDOW_MS),
                stall_timeout,
            )?;
            let Some(header) = header else {
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
            // Per-frame hard cap tied to `stall_timeout`; see the header phase
            // above. The deadline persisted on `state` already carries the
            // header-phase elapsed time, so the payload phase continues the
            // same per-frame budget rather than restarting it.
            read_exact_payload_with_timeout(
                reader,
                &mut buf[..payload_len],
                state,
                stall_timeout,
                Duration::from_millis(MID_FRAME_DRAIN_WINDOW_MS),
                stall_timeout,
            )?;
        }

        // Frame complete — decide how to return based on the code.
        // `state` is reset AFTER the code classification so a
        // drain-yield above (pre-classification) would have carried
        // the partial state through unchanged.
        let reset_state = FrameReadState::new();
        *state = reset_state;

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

/// `true` if an error is a drain-yield from a mid-frame reader. The
/// I/O loop catches this in the read branch, drains outbound commands,
/// and re-enters `read_frame_into` with the same `FrameReadState` so
/// the partial frame resumes byte-for-byte.
pub fn is_drain_yield(err: &crate::error::Error) -> bool {
    matches!(
        err,
        crate::error::Error::Stream {
            kind: crate::error::StreamErrorKind::ProtocolError,
            message,
        } if message.contains(DRAIN_YIELD_MARKER)
    )
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
        // generous drain budget (1 s) and per-frame cap (60 s) so the
        // stall-deadline escalation fires first -- this test asserts the
        // per-stall failure path.
        let err = read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_millis(20),
            Duration::from_secs(1),
            Duration::from_secs(60),
        )
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

    /// A slow-trickling stream whose gaps are each well under the deadline
    /// but whose aggregate duration exceeds it must still succeed. Proves
    /// the deadline re-arms on every successful byte (the JVM terminal's
    /// per-read socket-timeout semantics) rather than running from function
    /// entry.
    /// Delivers one byte per `Ok`, with each byte preceded by a single
    /// `WouldBlock` that sleeps for `sleep_per_stall` wall-clock time.
    /// Used to verify the stall-deadline actually resets on progress —
    /// cumulative sleep exceeds the configured deadline, but each
    /// individual gap is under it. If the deadline did not reset, the
    /// second gap would trip it.
    ///
    /// The per-stall sleep is kept far below the per-stall deadline so CPU
    /// contention (a 5 ms sleep ballooning under load) cannot stretch a
    /// single gap across the deadline and trip a spurious fatal; the
    /// non-vacuousness instead comes from the *number* of gaps, whose real
    /// cumulative sleep is guaranteed to exceed the deadline.
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

    /// Many small gaps under a generous per-stall deadline:
    /// - `PAYLOAD_LEN` bytes, one `WouldBlock` gap before each.
    /// - Per-stall sleep `GAP_MS` (5 ms) is 40× below the per-stall
    ///   deadline `STALL_MS` (200 ms), so even a heavily load-stretched
    ///   gap stays well under the deadline — no spurious "without
    ///   progress" fatal.
    /// - Cumulative real sleep `PAYLOAD_LEN × GAP_MS` (80 × 5 ms = 400 ms)
    ///   structurally exceeds the per-stall deadline, so the success is
    ///   not vacuous: a fixed deadline armed from function entry (the
    ///   prior, broken style) would fire once cumulative sleep crossed
    ///   `STALL_MS`. Only the reset-on-progress semantics let it complete.
    ///
    /// Behavioral assertions preserved: the read succeeds (no byte loss,
    /// payload arrives in order) under reset-on-progress, and the
    /// lower-bound wall-time check stays non-vacuous because it is backed
    /// by `PAYLOAD_LEN` real sleeps that sum past the deadline.
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
        let mut state = FrameReadState::new();
        let started = Instant::now();
        // Drain budget and per-frame cap far above the aggregate wall
        // time so this test pins the per-stall reset semantics
        // independently of both the drain-yield and per-frame-cap paths
        // (generous upper bounds, not tight ones).
        read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_millis(STALL_MS),
            Duration::from_secs(5),
            Duration::from_secs(60),
        )
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
        let mut state = FrameReadState::new();
        // Drain budget (1 s) and per-frame cap (60 s) are generous so the
        // stall-deadline wins.
        let err = read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_millis(20),
            Duration::from_secs(1),
            Duration::from_secs(60),
        )
        .unwrap_err();
        match err {
            crate::error::Error::Stream { kind, message } => {
                assert_eq!(kind, crate::error::StreamErrorKind::ProtocolError);
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
        // Drain budget (1 s) and per-frame cap (60 s) generous so the
        // per-stall deadline trips first.
        let err = read_header_with_timeout(
            &mut reader,
            &mut state,
            Duration::from_millis(20),
            Duration::from_secs(1),
            Duration::from_secs(60),
        )
        .unwrap_err();
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

    /// Finding #2: a peer that delivers the FIRST header byte and then
    /// stalls must be cut off at the drain-yield cap, NOT held for the
    /// full per-stall `stall_timeout`. The drain budget is armed on the
    /// first successful header byte read inside the call, so a mid-header
    /// stall is bounded exactly like the mid-payload path; otherwise the
    /// reader could block the command drain (heartbeats, subscribes) for
    /// the whole `stall_timeout`.
    ///
    /// The deciding signature: with `drain_budget` (40 ms) far below
    /// `stall_timeout` (5 s), the error must carry [`DRAIN_YIELD_MARKER`]
    /// and return well inside the stall timeout. Before the fix the first
    /// byte left `drain_deadline` un-armed, so this call fell through to
    /// the per-stall path and the test would block for ~5 s before a
    /// "without progress" fatal.
    #[test]
    fn mid_header_first_byte_then_stall_yields_at_cap() {
        let mut reader = AlwaysStalledAfter {
            // One header byte (LEN=5), then an unbroken stall.
            prefix: vec![0x05],
            pos: 0,
            err_kind: std::io::ErrorKind::WouldBlock,
        };
        let mut state = FrameReadState::new();
        let started = Instant::now();
        let err = read_header_with_timeout(
            &mut reader,
            &mut state,
            // Generous per-stall timeout: if the drain budget were not
            // armed on the first byte, the call would block this long.
            Duration::from_secs(5),
            // Tight drain budget — the cap that must fire instead.
            Duration::from_millis(40),
            // Generous per-frame cap so the drain-yield fires first.
            Duration::from_secs(60),
        )
        .unwrap_err();
        let elapsed = started.elapsed();

        match &err {
            crate::error::Error::Stream { kind, message } => {
                assert_eq!(*kind, crate::error::StreamErrorKind::ProtocolError);
                assert!(
                    message.contains(DRAIN_YIELD_MARKER),
                    "first-byte-then-stall must yield at the drain cap, got: {message}"
                );
                assert!(
                    message.contains("mid-header"),
                    "expected mid-header context, got: {message}"
                );
            }
            other => panic!("expected mid-header drain-yield, got {other:?}"),
        }
        assert!(
            is_drain_yield(&err),
            "is_drain_yield must match the produced error"
        );
        // Must return at the cap, nowhere near the 5 s stall timeout.
        assert!(
            elapsed < Duration::from_secs(1),
            "drain-yield must fire at the ~40 ms cap, not the 5 s stall timeout; \
             elapsed = {elapsed:?}"
        );
        // The single delivered byte must be preserved for resumption.
        assert_eq!(
            state.header_read, 1,
            "the first header byte must be retained on state for the next call"
        );
    }

    /// Reader that delivers the first header byte IMMEDIATELY (no
    /// pre-stall), then emits sleeping `WouldBlock` errors before the
    /// second byte. Models the exact mid-header stall finding #2 bounds:
    /// the first byte lands inside the call (arming the drain budget),
    /// and the gap before the second byte must be cut off at the cap.
    struct FirstByteThenSleepingStall {
        byte_one: Option<u8>,
        byte_two: Option<u8>,
        sleep_per_stall: Duration,
        stalls_before_byte_two: usize,
    }

    impl std::io::Read for FirstByteThenSleepingStall {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if let Some(b) = self.byte_one.take() {
                buf[0] = b;
                return Ok(1);
            }
            if self.stalls_before_byte_two > 0 {
                self.stalls_before_byte_two -= 1;
                std::thread::sleep(self.sleep_per_stall);
                return Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, "gap"));
            }
            if let Some(b) = self.byte_two.take() {
                buf[0] = b;
                return Ok(1);
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "exhausted",
            ))
        }
    }

    /// Finding #2: after the first-byte drain-yield, the next call with
    /// the SAME state resumes at byte 1 and completes the header once the
    /// peer delivers the second byte. No byte is lost or re-read — the
    /// invariant that makes the mid-header yield safe for command-drain
    /// re-entry.
    #[test]
    fn mid_header_drain_yield_resumes_without_byte_loss() {
        let mut reader = FirstByteThenSleepingStall {
            byte_one: Some(0x01),
            byte_two: Some(StreamMsgType::Ping as u8),
            sleep_per_stall: Duration::from_millis(25),
            // Enough sleeping stalls (25 ms each) to overrun the 10 ms
            // budget on the first call.
            stalls_before_byte_two: 10,
        };
        let mut state = FrameReadState::new();

        // First call: byte 1 lands immediately and arms the 10 ms budget;
        // the first 25 ms stall then trips the cap, so the call yields
        // with exactly one byte banked. The per-frame cap (60 s) is far
        // above the test's runtime so the drain-yield fires first.
        let err = read_header_with_timeout(
            &mut reader,
            &mut state,
            Duration::from_secs(5),
            Duration::from_millis(10),
            Duration::from_secs(60),
        )
        .unwrap_err();
        assert!(is_drain_yield(&err), "first call must drain-yield");
        assert_eq!(state.header_read, 1, "one byte banked across the yield");

        // Drain the remaining inter-byte stalls so byte 2 is reachable,
        // then the second call completes the header from the preserved
        // offset with a generous budget. No byte is re-read. The per-frame
        // deadline carried on `state` from the first call is far from
        // expiry, so it does not preempt completion.
        reader.stalls_before_byte_two = 0;
        let header = read_header_with_timeout(
            &mut reader,
            &mut state,
            Duration::from_secs(5),
            Duration::from_secs(1),
            Duration::from_secs(60),
        )
        .expect("second call completes the header")
        .expect("a full 2-byte header");
        assert_eq!(
            header,
            [0x01, StreamMsgType::Ping as u8],
            "both header bytes must be present and in order after resumption"
        );
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

    /// Finding #3: a bytes-at-a-time producer with a small stall per byte
    /// must NOT keep the mid-payload reader occupied past the drain-yield
    /// budget. The reader must yield before consuming the whole payload,
    /// preserving partial state. This is the exact scenario the command
    /// drain needs to service heartbeats.
    ///
    /// Load-robust parameter choice (no tight wall-clock margins):
    /// - Per-stall sleep `GAP_MS` (3 ms) is three orders of magnitude
    ///   below the per-stall deadline (30 s), so the per-stall "without
    ///   progress" path can never preempt the drain-yield even when a
    ///   sleep balloons under CPU contention. Only the drain-yield path
    ///   can fire — which is the property under test.
    /// - The drain budget (150 ms) is ~50× a single gap, so at least one
    ///   byte is guaranteed to land before the budget trips (no spurious
    ///   `payload_read == 0`), yet far below the aggregate
    ///   `PAYLOAD_LEN × GAP_MS` (200 × 3 ms = 600 ms), so the yield is
    ///   guaranteed to happen strictly before the full payload arrives.
    ///
    /// Behavioral assertions preserved: the error is a drain-yield
    /// (`DRAIN_YIELD_MARKER`, mid-payload context), `is_drain_yield`
    /// agrees, and partial progress is banked (`0 < payload_read <
    /// PAYLOAD_LEN`). The wall-time bound stays only as a generous
    /// "didn't hang" backstop; the before-completion property is pinned
    /// deterministically by the `payload_read < PAYLOAD_LEN` count check.
    #[test]
    fn mid_payload_bytes_at_a_time_yields_before_full_payload() {
        const PAYLOAD_LEN: usize = 200;
        const GAP_MS: u64 = 3;
        const BUDGET_MS: u64 = 150;
        // Budget must sit between one gap (so a byte lands) and the
        // aggregate (so the yield precedes completion).
        const _: () = assert!(BUDGET_MS > GAP_MS);
        const _: () = assert!(BUDGET_MS < PAYLOAD_LEN as u64 * GAP_MS);

        // Byte values are irrelevant here (the test asserts counts, not
        // contents); fill with a wrapping pattern so any PAYLOAD_LEN is
        // valid without an overflow on the `u8` range.
        let mut reader = OneByteAtATime {
            remaining: (0..PAYLOAD_LEN).map(|i| (i % 256) as u8).collect(),
            sleep_per_stall: Duration::from_millis(GAP_MS),
            stalled_since_last_byte: false,
        };
        let mut payload = vec![0u8; PAYLOAD_LEN];
        let mut state = FrameReadState::new();
        let started = Instant::now();
        // Per-stall deadline (30 s) and per-frame cap (120 s) are enormous
        // so neither can preempt the drain-yield; the drain budget is what
        // must fire.
        let err = read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_secs(30),
            Duration::from_millis(BUDGET_MS),
            Duration::from_secs(120),
        )
        .unwrap_err();
        let elapsed = started.elapsed();

        // The error must be a drain-yield, not a per-stall fatal.
        match &err {
            crate::error::Error::Stream { kind, message } => {
                assert_eq!(*kind, crate::error::StreamErrorKind::ProtocolError);
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

        // Generous "didn't hang" backstop only — the meaningful
        // before-completion property is the deterministic count check
        // below, not a tight wall-clock margin.
        assert!(
            elapsed < Duration::from_secs(5),
            "drain-yield must fire well before the per-stall deadline; \
             elapsed = {elapsed:?}"
        );

        // Partial progress must be preserved: at least SOME bytes
        // should have landed (the budget is far above a single gap), and
        // strictly less than the full payload (the yield MUST happen
        // before completion — pinned deterministically by the budget
        // sitting below the aggregate sleep).
        assert!(
            state.payload_read > 0,
            "partial payload bytes must have landed before the yield"
        );
        assert!(
            state.payload_read < PAYLOAD_LEN,
            "yield must happen BEFORE the full payload arrives; got \
             {} of {}",
            state.payload_read,
            PAYLOAD_LEN
        );
    }

    /// Finding #3: after a drain-yield, the next call to
    /// `read_exact_payload_with_timeout` with the SAME state must
    /// resume from the exact byte offset. No bytes are lost, none
    /// are re-read. This is the invariant that makes the yield
    /// safe for the command-drain re-entry pattern.
    ///
    /// Load-robust parameters (same shape as the yield-before-completion
    /// test): tiny per-stall sleep under an enormous per-stall deadline so
    /// the per-stall path can never preempt the drain-yield, and a drain
    /// budget that sits comfortably between a single gap (so byte 1 lands)
    /// and the aggregate sleep (so the first call yields mid-payload).
    #[test]
    fn drain_yield_preserves_payload_read_for_resumption() {
        const PAYLOAD_LEN: usize = 40;
        const GAP_MS: u64 = 5;
        const BUDGET_MS: u64 = 60;
        const _: () = assert!(BUDGET_MS > GAP_MS);
        const _: () = assert!(BUDGET_MS < PAYLOAD_LEN as u64 * GAP_MS);

        // Deterministic, distinct-ish byte pattern so the final equality
        // check proves both no-loss AND correct ordering.
        let expected: Vec<u8> = (0..PAYLOAD_LEN)
            .map(|i| (i.wrapping_mul(7) % 256) as u8)
            .collect();
        let mut reader = OneByteAtATime {
            remaining: expected.clone(),
            sleep_per_stall: Duration::from_millis(GAP_MS),
            stalled_since_last_byte: false,
        };
        let mut payload = vec![0u8; PAYLOAD_LEN];
        let mut state = FrameReadState::new();

        // First call: per-stall deadline 30 s (never trips), drain budget
        // 60 ms -> yields after a handful of bytes, never the whole
        // payload (aggregate sleep is PAYLOAD_LEN × GAP_MS = 200 ms). The
        // per-frame cap (120 s) is far above the aggregate so it never
        // preempts the drain-yield this test pins.
        let err = read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_secs(30),
            Duration::from_millis(BUDGET_MS),
            Duration::from_secs(120),
        )
        .unwrap_err();
        assert!(is_drain_yield(&err), "first call must drain-yield");
        let first_read = state.payload_read;
        assert!(
            first_read > 0 && first_read < PAYLOAD_LEN,
            "first call must land some but not all bytes; got {first_read}"
        );

        // Second call: finish the payload with a generous budget. The
        // state carries the first-call progress (including the per-frame
        // deadline) so no bytes are re-read and the deadline does not
        // preempt completion.
        read_exact_payload_with_timeout(
            &mut reader,
            &mut payload,
            &mut state,
            Duration::from_secs(30),
            Duration::from_secs(5),
            Duration::from_secs(120),
        )
        .expect("second call must complete the payload without error");
        assert_eq!(
            state.payload_read, PAYLOAD_LEN,
            "state must reflect full payload completion"
        );
        assert_eq!(
            payload, expected,
            "every byte of the payload must land exactly once, in order"
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
        // Tiny per-stall sleep: this test does not depend on the yield
        // actually firing (it tolerates zero yields), only on any yield
        // being followed by clean resumption and the frame completing.
        // Keeping the sleep small means the aggregate wall time stays
        // orders of magnitude below the 5 s guard and the retry-count
        // bound even under heavy CPU contention.
        let mut reader = OneByteAtATime {
            remaining: wire,
            sleep_per_stall: Duration::from_millis(2),
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

    /// Finding #2: a peer trickling one byte per resume must be cut off
    /// once the in-progress frame exceeds the per-frame hard cap — it
    /// must NOT be held forever. This drives the exact I/O-loop re-entry
    /// pattern: each `read_exact_payload_with_timeout` call yields to the
    /// command drain (small `drain_budget`), then the loop re-enters with
    /// the SAME `state`. The per-frame deadline persisted on `state`
    /// accumulates across every resume, so after the cap elapses the read
    /// fails FATALLY (not a drain-yield) and the watchdog/reconnect can
    /// fire.
    ///
    /// Before the fix the drain deadline was a stack-local re-armed on
    /// every resumed call, so this loop would yield-and-resume forever
    /// and a single partial frame could pin the connection indefinitely.
    ///
    /// Load-robust: the per-byte gap (3 ms) is far below the per-stall
    /// timeout (5 s, never trips) and the payload is large enough that
    /// the trickle cannot complete within the per-frame cap (120 ms) even
    /// when the scheduler is loose. The non-vacuousness is pinned by the
    /// structural inequality below, not a tight wall-clock margin.
    #[test]
    fn trickling_peer_cut_off_at_per_frame_deadline() {
        const PAYLOAD_LEN: usize = 200;
        const GAP_MS: u64 = 3;
        const DRAIN_BUDGET_MS: u64 = 20;
        const FRAME_CAP_MS: u64 = 120;
        // A byte lands per (gap + Ok); the trickle needs far longer than
        // the per-frame cap to finish, so the cap MUST fire mid-frame.
        const _: () = assert!(PAYLOAD_LEN as u64 * GAP_MS > FRAME_CAP_MS);
        // The drain budget is well under the per-frame cap so yields fire
        // and the loop genuinely re-enters across the deadline.
        const _: () = assert!(DRAIN_BUDGET_MS < FRAME_CAP_MS);

        let mut reader = OneByteAtATime {
            remaining: (0..PAYLOAD_LEN).map(|i| (i % 256) as u8).collect(),
            sleep_per_stall: Duration::from_millis(GAP_MS),
            stalled_since_last_byte: false,
        };
        let mut payload = vec![0u8; PAYLOAD_LEN];
        let mut state = FrameReadState::new();

        let started = Instant::now();
        let mut resumes = 0u32;
        let fatal = loop {
            match read_exact_payload_with_timeout(
                &mut reader,
                &mut payload,
                &mut state,
                Duration::from_secs(5),
                Duration::from_millis(DRAIN_BUDGET_MS),
                Duration::from_millis(FRAME_CAP_MS),
            ) {
                Ok(()) => panic!(
                    "the trickle must never complete within the per-frame cap; \
                     read {} of {PAYLOAD_LEN} bytes",
                    state.payload_read
                ),
                Err(ref e) if is_drain_yield(e) => {
                    // Service the (simulated) command drain and re-enter
                    // with the SAME state — the per-frame deadline carries
                    // across this boundary.
                    resumes += 1;
                    assert!(
                        started.elapsed() < Duration::from_secs(5),
                        "the per-frame cap must fire long before this guard; \
                         the trickle is being held instead of cut off"
                    );
                    continue;
                }
                Err(e) => break e,
            }
        };

        // The terminating error must be the FATAL per-frame deadline, not
        // a drain-yield: only a fatal read makes the I/O loop reconnect.
        assert!(
            !is_drain_yield(&fatal),
            "the cut-off must be a fatal read, not a recoverable drain-yield"
        );
        match &fatal {
            crate::error::Error::Stream { kind, message } => {
                assert_eq!(*kind, crate::error::StreamErrorKind::ProtocolError);
                assert!(
                    message.contains("frame deadline exceeded") && message.contains("mid-payload"),
                    "expected a per-frame-deadline fatal, got: {message}"
                );
            }
            other => panic!("expected a fatal Fpss ProtocolError, got {other:?}"),
        }
        // The frame was genuinely partial when cut off (held, then
        // bounded — not completed), and the loop really did re-enter
        // across the deadline rather than failing on the first call.
        assert!(
            state.payload_read < PAYLOAD_LEN,
            "frame must be cut off mid-payload, not completed; got {} of {PAYLOAD_LEN}",
            state.payload_read
        );
        assert!(
            resumes >= 1,
            "the per-frame deadline must persist across at least one \
             drain-yield re-entry; resumes = {resumes}"
        );
    }

    /// Finding #2: the same per-frame bound applies in the header phase.
    /// A peer that delivers the first header byte and then trickles
    /// (never the second byte) must be cut off once the per-frame cap
    /// elapses, across resumed calls, with a FATAL error.
    #[test]
    fn trickling_header_cut_off_at_per_frame_deadline() {
        const GAP_MS: u64 = 3;
        const DRAIN_BUDGET_MS: u64 = 15;
        const FRAME_CAP_MS: u64 = 90;

        // Delivers byte one, then an unbounded run of short sleeping
        // stalls before byte two — byte two never actually arrives within
        // the cap, so the header can never complete.
        let mut reader = FirstByteThenSleepingStall {
            byte_one: Some(0x05),
            byte_two: Some(StreamMsgType::Ping as u8),
            sleep_per_stall: Duration::from_millis(GAP_MS),
            // Far more stalls than the cap admits at GAP_MS each.
            stalls_before_byte_two: (FRAME_CAP_MS / GAP_MS) as usize * 4,
        };
        let mut state = FrameReadState::new();

        let started = Instant::now();
        let mut resumes = 0u32;
        let fatal = loop {
            match read_header_with_timeout(
                &mut reader,
                &mut state,
                Duration::from_secs(5),
                Duration::from_millis(DRAIN_BUDGET_MS),
                Duration::from_millis(FRAME_CAP_MS),
            ) {
                Ok(Some(_)) => panic!("header must not complete within the cap"),
                Ok(None) => panic!("unexpected EOF"),
                Err(ref e) if is_drain_yield(e) => {
                    resumes += 1;
                    assert!(
                        started.elapsed() < Duration::from_secs(5),
                        "per-frame cap must fire well before this guard"
                    );
                    continue;
                }
                Err(e) => break e,
            }
        };

        assert!(
            !is_drain_yield(&fatal),
            "header cut-off must be fatal, not a drain-yield"
        );
        match &fatal {
            crate::error::Error::Stream { kind, message } => {
                assert_eq!(*kind, crate::error::StreamErrorKind::ProtocolError);
                assert!(
                    message.contains("frame deadline exceeded") && message.contains("mid-header"),
                    "expected a mid-header per-frame-deadline fatal, got: {message}"
                );
            }
            other => panic!("expected a fatal Fpss ProtocolError, got {other:?}"),
        }
        assert_eq!(
            state.header_read, 1,
            "the single delivered header byte stays banked at cut-off"
        );
        assert!(
            resumes >= 1,
            "the per-frame deadline must persist across header re-entries"
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
        let dy = crate::error::Error::Stream {
            kind: crate::error::StreamErrorKind::ProtocolError,
            message: format!("{DRAIN_YIELD_MARKER}: mid-payload after 3 of 8 byte(s)"),
        };
        assert!(is_drain_yield(&dy));

        // Unrelated protocol error -> false.
        let other = crate::error::Error::Stream {
            kind: crate::error::StreamErrorKind::ProtocolError,
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

    /// A mid-frame disconnect leaves `FrameReadState` parked in the
    /// payload phase with a partial `payload_read`. Reusing that state to
    /// read a brand-new session (the reconnect path) skips the header and
    /// consumes the new session's leading bytes as the tail of a phantom
    /// frame, classifying them under the previous frame's code byte and
    /// desyncing every frame after it. Resetting the state to the
    /// header-phase initial value (`FrameReadState::new()`) before the
    /// reborn reader runs restores the frame-boundary invariant.
    ///
    /// This pins the contract the io_loop reconnect block relies on when
    /// it resets `frame_state` after establishing a new connection.
    #[test]
    fn reset_frame_state_recovers_frame_boundary_after_mid_frame_disconnect() {
        // Build a state that looks exactly like the residue of a frame
        // whose payload was cut mid-transmission: header decoded for a
        // 4-byte ERROR frame, payload phase entered, 2 of 4 bytes read.
        let mut frame_state = FrameReadState::new();
        frame_state.header_buf = [0x04, StreamMsgType::Error as u8];
        frame_state.header_read = 2;
        frame_state.payload_phase = true;
        frame_state.payload_len = 4;
        frame_state.payload_read = 2;

        // The reborn reader's first bytes are a fresh, well-formed PING
        // frame from the new session: LEN=1, CODE=PING, one payload byte.
        let fresh_session = encode_manual(StreamMsgType::Ping as u8, &[0xAA]);

        // Without a reset, the stale payload phase swallows the first two
        // bytes of the new session as the phantom frame's tail and
        // classifies them under the stale ERROR code -- a desync.
        {
            let mut desynced_state = frame_state.clone();
            // The io_loop reuses one `frame_buf` across frames; on the
            // stale path it is still sized to the interrupted frame's
            // payload because the resize only runs at header completion,
            // which the skipped header phase never reaches.
            let mut buf = vec![0u8; desynced_state.payload_len];
            let mut cursor = Cursor::new(fresh_session.clone());
            let (code, _len) = read_frame_into(&mut cursor, &mut buf, &mut desynced_state)
                .unwrap()
                .unwrap();
            // The phantom frame is mis-attributed to the previous code,
            // not the PING that actually began the new session.
            assert_eq!(
                code,
                StreamMsgType::Error,
                "without reset, the stale code byte mislabels the new session's bytes"
            );
        }

        // With the reset the io_loop applies on reconnect, the reader
        // starts at the new session's header boundary and decodes the
        // PING frame correctly.
        frame_state = FrameReadState::new();
        assert!(!frame_state.payload_phase);
        assert_eq!(frame_state.header_read, 0);
        assert_eq!(frame_state.payload_read, 0);

        let mut buf = Vec::new();
        let mut cursor = Cursor::new(fresh_session);
        let (code, len) = read_frame_into(&mut cursor, &mut buf, &mut frame_state)
            .unwrap()
            .unwrap();
        assert_eq!(code, StreamMsgType::Ping);
        assert_eq!(&buf[..len], &[0xAA]);
    }
}
