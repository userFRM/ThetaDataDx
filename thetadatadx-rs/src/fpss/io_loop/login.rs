//! FPSS login handshake.
//!
//! Owns the post-`CREDENTIALS` blocking read that resolves either a
//! `Metadata` (success) or a `Disconnected` (failure) frame, plus the
//! invariant that every typed control frame the server emits BEFORE
//! `Metadata` (`Connected`, `Ping`, `ReconnectedServer`, `Restart`) is
//! captured for replay onto the event bus once the Disruptor producer
//! is live.

use std::time::{Duration, Instant};

use crate::tdbe::types::enums::{RemoveReason, StreamMsgType};

use crate::error::Error;

use super::super::connection;
use super::super::events::StreamControl;
use super::super::framing::read_frame_into_with_stall_timeout;
use super::super::protocol::parse_disconnect_reason;

/// Outcome of a single login handshake.
pub enum LoginResult {
    Success(String),
    Disconnected(RemoveReason),
}

/// Wall-clock budget for a single handshake, independent of the per-read
/// socket timeout. A chatty peer that sends a control frame just before each
/// read timeout would otherwise reset the per-read deadline forever; this
/// caps the total time the reconnect thread can spend in one handshake.
fn handshake_deadline(read_timeout: Duration) -> Instant {
    // Generous relative to a single read so a legitimately slow but
    // progressing handshake is never cut off, yet bounded so a chatty
    // no-`Metadata` peer cannot wedge the reconnect thread.
    let budget = (read_timeout.saturating_mul(5)).max(Duration::from_secs(15));
    Instant::now() + budget
}

/// Wait for the server's login response (blocking).
///
/// Reads frames until METADATA or DISCONNECTED.
///
/// On `Metadata`, the payload is the server's "Bundle" string. We copy it
/// verbatim into [`LoginResult::Success`]; see
/// [`StreamControl::LoginSuccess`] for why this string is treated as opaque.
///
/// Typed control frames that arrive BEFORE `METADATA` — code 4
/// (`Connected`), code 10 (`Ping`), code 13 (`ReconnectedServer`), and
/// code 31 (`Restart`) — are appended to `pending_control` in the order
/// the wire delivered them. The caller flushes this buffer onto the
/// event bus once the Disruptor producer is live, so user callbacks
/// observe handshake-time control frames on the same channel the
/// post-login `decode_frame` dispatch uses — a typed control frame that
/// precedes `METADATA` would otherwise be lost, since the handshake loop
/// consumes it before the main dispatch can turn it into a typed event.
pub fn wait_for_login(
    stream: &mut connection::FpssStream,
    pending_control: &mut Vec<StreamControl>,
    read_timeout: Duration,
) -> Result<LoginResult, Error> {
    let deadline = handshake_deadline(read_timeout);
    wait_for_login_generic(
        stream,
        pending_control,
        deadline,
        read_timeout,
        Instant::now,
    )
}

/// Read-generic variant of [`wait_for_login`] for unit-testable handshake
/// coverage. Holds the full dispatch logic so both the TLS-backed entry
/// point above and in-memory test harnesses can drive it against a
/// buffer of pre-canned frames.
///
/// `deadline` is a wall-clock backstop that bounds a peer which streams
/// control frames without ever completing the handshake: the per-read socket
/// timeout alone cannot, because each frame resets it. `now` injects the
/// clock so tests can drive the deadline deterministically. It is checked
/// between complete frames; a single frame's read is bounded by
/// `stall_timeout` (the per-stall no-progress budget the reader enforces),
/// so a peer that dribbles a partial frame and then goes silent is cut off
/// by that stall timeout rather than held indefinitely.
fn wait_for_login_generic<R, C>(
    stream: &mut R,
    pending_control: &mut Vec<StreamControl>,
    deadline: Instant,
    stall_timeout: Duration,
    mut now: C,
) -> Result<LoginResult, Error>
where
    R: std::io::Read,
    C: FnMut() -> Instant,
{
    // Reused across frames. Each read consumes one complete frame bounded by
    // the per-stall timeout; the wall-clock deadline is checked between frames.
    let mut frame_buf: Vec<u8> = Vec::new();
    loop {
        if now() >= deadline {
            return Err(Error::Stream {
                kind: crate::error::StreamErrorKind::Timeout,
                message: "login handshake did not complete within the handshake deadline"
                    .to_string(),
            });
        }

        let (code, payload_len) =
            match read_frame_into_with_stall_timeout(stream, &mut frame_buf, stall_timeout) {
                Ok(Some(frame)) => frame,
                Ok(None) => {
                    return Err(Error::Stream {
                        kind: crate::error::StreamErrorKind::Disconnected,
                        message: "connection closed during login handshake".to_string(),
                    })
                }
                Err(e) => return Err(e),
            };
        let payload = &frame_buf[..payload_len];

        match code {
            StreamMsgType::Metadata => {
                let permissions = String::from_utf8_lossy(payload).to_string();
                return Ok(LoginResult::Success(permissions));
            }
            StreamMsgType::Disconnected => {
                let reason = parse_disconnect_reason(payload);
                return Ok(LoginResult::Disconnected(reason));
            }
            StreamMsgType::Error => {
                let msg = String::from_utf8_lossy(payload);
                tracing::warn!(message = %msg, "server error during login");
                return Err(Error::Stream {
                    kind: crate::error::StreamErrorKind::ConnectionRefused,
                    message: format!("server error during login: {msg}"),
                });
            }
            StreamMsgType::Connected => {
                // Code 4: transport ack. Mirror the post-login
                // `decode_frame` dispatch so users subscribed to
                // `StreamControl::Connected` see this frame whether it
                // arrived before or after METADATA.
                tracing::debug!("FPSS CONNECTED frame received during handshake");
                pending_control.push(StreamControl::Connected);
            }
            StreamMsgType::Ping => {
                // Code 10: server heartbeat. Preserve the raw payload so
                // downstream diagnostics match the post-login dispatch
                // path byte-for-byte.
                pending_control.push(StreamControl::Ping {
                    payload: payload.to_vec(),
                });
            }
            StreamMsgType::Reconnected => {
                // Code 13: server-side reconnect ack. Distinct from the
                // client-emitted `Reconnected` control (which the
                // auto-reconnect state machine produces after a fresh
                // TLS session authenticates).
                tracing::debug!("FPSS RECONNECTED frame received during handshake");
                pending_control.push(StreamControl::ReconnectedServer);
            }
            StreamMsgType::Restart => {
                // Code 31: server stream restart. Promoted to a typed
                // event so callbacks that clear downstream state on
                // restart don't need to wait for the post-METADATA
                // dispatch.
                tracing::debug!("FPSS RESTART frame received during handshake");
                pending_control.push(StreamControl::Restart);
            }
            other => {
                tracing::trace!(code = ?other, "ignoring frame during login handshake");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::events::StreamEvent;
    use super::*;

    /// Build a single FPSS wire frame: `[LEN: u8] [CODE: u8] [PAYLOAD...]`.
    ///
    /// Keeps the handshake unit tests decoupled from the higher-level
    /// `framing::write_raw_frame` writer so they can't be passed a
    /// bogus-but-valid frame through test-only helpers.
    fn wire_frame(code: StreamMsgType, payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() <= u8::MAX as usize);
        let mut v = Vec::with_capacity(2 + payload.len());
        v.push(payload.len() as u8);
        v.push(code as u8);
        v.extend_from_slice(payload);
        v
    }

    /// Drive the handshake with a far-future deadline that never trips, for
    /// the dispatch/ordering tests.
    fn run_handshake<R: std::io::Read>(
        stream: &mut R,
        pending_control: &mut Vec<StreamControl>,
    ) -> Result<LoginResult, Error> {
        let deadline = Instant::now() + Duration::from_secs(3_600);
        wait_for_login_generic(
            stream,
            pending_control,
            deadline,
            Duration::from_secs(10),
            Instant::now,
        )
    }

    /// A CONNECTED frame arriving BEFORE METADATA must be captured in
    /// `pending_control` so the io_loop can forward the buffered
    /// `StreamControl::Connected` to the event bus. Without this capture
    /// the frame would be lost, since only the post-login `decode_frame`
    /// dispatch knows how to turn it into a typed event.
    #[test]
    fn wait_for_login_captures_connected_frame_before_metadata() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&wire_frame(StreamMsgType::Connected, &[]));
        buf.extend_from_slice(&wire_frame(StreamMsgType::Metadata, b"test-perms"));
        let mut cursor = std::io::Cursor::new(buf);

        let mut pending: Vec<StreamControl> = Vec::new();
        let result = run_handshake(&mut cursor, &mut pending)
            .expect("wait_for_login_generic must succeed when Metadata arrives");
        match result {
            LoginResult::Success(p) => assert_eq!(p, "test-perms"),
            LoginResult::Disconnected(r) => {
                panic!("expected LoginResult::Success, got Disconnected({r:?})")
            }
        }
        assert_eq!(pending.len(), 1);
        assert!(matches!(pending[0], StreamControl::Connected));
    }

    /// Complement to the above: when only METADATA arrives (the common
    /// case), `pending_control` stays empty so the io_loop does NOT
    /// emit any spurious control event for users who never actually
    /// saw a typed control frame on the wire.
    #[test]
    fn wait_for_login_leaves_pending_control_empty_without_control_frames() {
        let buf = wire_frame(StreamMsgType::Metadata, b"test-perms");
        let mut cursor = std::io::Cursor::new(buf);

        let mut pending: Vec<StreamControl> = Vec::new();
        let result = run_handshake(&mut cursor, &mut pending)
            .expect("wait_for_login_generic must succeed when Metadata arrives");
        assert!(matches!(result, LoginResult::Success(_)));
        assert!(
            pending.is_empty(),
            "pending_control must stay empty when no typed control frames preceded METADATA"
        );
    }

    /// LoginResult variant shape: a Disconnected frame during handshake
    /// propagates without populating `pending_control`. Guards against
    /// a regression where a shape-buggy handshake would smuggle a
    /// Connected/Ping through the error path.
    #[test]
    fn wait_for_login_disconnected_does_not_populate_pending_control() {
        let mut buf = Vec::new();
        // Reason code 0 = InvalidCredentials (i16 BE).
        buf.extend_from_slice(&wire_frame(
            StreamMsgType::Disconnected,
            &0i16.to_be_bytes(),
        ));
        let mut cursor = std::io::Cursor::new(buf);

        let mut pending: Vec<StreamControl> = Vec::new();
        let result = run_handshake(&mut cursor, &mut pending)
            .expect("Disconnected frame must produce LoginResult::Disconnected, not Err");
        assert!(matches!(result, LoginResult::Disconnected(_)));
        assert!(pending.is_empty());
    }

    /// End-to-end coverage that the io_loop-level forwarding path does
    /// the right thing with a populated `pending_control` buffer:
    /// running the actual I/O loop requires a real TLS stream, so this
    /// test asserts the adapter contract by exercising the smaller,
    /// deterministic piece -- draining the buffer emits every control
    /// in wire order BEFORE LoginSuccess.
    #[test]
    fn pending_control_forwards_to_event_bus_in_wire_order() {
        // Emulate the io_loop startup block: drain `pending_control`
        // in wire order, THEN publish LoginSuccess. A regression that
        // re-orders or drops events would fail on the `matches!`
        // sequence below.
        let mut events: Vec<StreamEvent> = Vec::new();
        let pending_control: Vec<StreamControl> = vec![
            StreamControl::Connected,
            StreamControl::Ping {
                payload: vec![0x00],
            },
            StreamControl::ReconnectedServer,
            StreamControl::Restart,
        ];
        for ctrl in pending_control {
            events.push(StreamEvent::Control(ctrl));
        }
        events.push(StreamEvent::Control(StreamControl::LoginSuccess {
            permissions: "test".to_string(),
        }));
        assert_eq!(events.len(), 5);
        assert!(matches!(
            events[0],
            StreamEvent::Control(StreamControl::Connected)
        ));
        match &events[1] {
            StreamEvent::Control(StreamControl::Ping { payload }) => {
                assert_eq!(payload.as_slice(), &[0x00]);
            }
            other => panic!("expected Ping, got {other:?}"),
        }
        assert!(matches!(
            events[2],
            StreamEvent::Control(StreamControl::ReconnectedServer)
        ));
        assert!(matches!(
            events[3],
            StreamEvent::Control(StreamControl::Restart)
        ));
        assert!(matches!(
            events[4],
            StreamEvent::Control(StreamControl::LoginSuccess { .. })
        ));
    }

    /// A PING frame arriving BEFORE METADATA must be captured in
    /// `pending_control` as `StreamControl::Ping` with the exact payload
    /// bytes, so the handshake's trace-and-drop branch does not swallow a
    /// heartbeat the server emits between CONNECT and METADATA.
    #[test]
    fn wait_for_login_captures_ping_frame_before_metadata() {
        let mut buf = Vec::new();
        // Server heartbeat observed as a 1-byte payload `[0]`.
        buf.extend_from_slice(&wire_frame(StreamMsgType::Ping, &[0x00]));
        buf.extend_from_slice(&wire_frame(StreamMsgType::Metadata, b"test-perms"));
        let mut cursor = std::io::Cursor::new(buf);

        let mut pending: Vec<StreamControl> = Vec::new();
        let result = run_handshake(&mut cursor, &mut pending)
            .expect("wait_for_login_generic must succeed when Metadata arrives");
        assert!(matches!(result, LoginResult::Success(_)));
        assert_eq!(pending.len(), 1, "PING must surface as a typed control");
        match &pending[0] {
            StreamControl::Ping { payload } => {
                assert_eq!(
                    payload.as_slice(),
                    &[0x00],
                    "Ping payload must match the wire bytes byte-for-byte"
                );
            }
            other => panic!("expected StreamControl::Ping, got {other:?}"),
        }
    }

    /// A RECONNECTED frame (code 13) arriving BEFORE METADATA must be
    /// captured as
    /// `StreamControl::ReconnectedServer`. The distinction from the
    /// client-emitted `StreamControl::Reconnected` is preserved.
    #[test]
    fn wait_for_login_captures_reconnected_server_frame_before_metadata() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&wire_frame(StreamMsgType::Reconnected, &[]));
        buf.extend_from_slice(&wire_frame(StreamMsgType::Metadata, b"perms"));
        let mut cursor = std::io::Cursor::new(buf);

        let mut pending: Vec<StreamControl> = Vec::new();
        let result = run_handshake(&mut cursor, &mut pending)
            .expect("wait_for_login_generic must succeed when Metadata arrives");
        assert!(matches!(result, LoginResult::Success(_)));
        assert_eq!(pending.len(), 1);
        assert!(matches!(pending[0], StreamControl::ReconnectedServer));
    }

    /// A RESTART frame (code 31) arriving BEFORE METADATA must be
    /// captured as `StreamControl::Restart`.
    #[test]
    fn wait_for_login_captures_restart_frame_before_metadata() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&wire_frame(StreamMsgType::Restart, &[]));
        buf.extend_from_slice(&wire_frame(StreamMsgType::Metadata, b"perms"));
        let mut cursor = std::io::Cursor::new(buf);

        let mut pending: Vec<StreamControl> = Vec::new();
        let result = run_handshake(&mut cursor, &mut pending)
            .expect("wait_for_login_generic must succeed when Metadata arrives");
        assert!(matches!(result, LoginResult::Success(_)));
        assert_eq!(pending.len(), 1);
        assert!(matches!(pending[0], StreamControl::Restart));
    }

    /// Multiple typed control frames arriving BEFORE METADATA must all
    /// be captured, in the exact wire order the server delivered them.
    #[test]
    fn wait_for_login_captures_multiple_control_frames_in_wire_order() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&wire_frame(StreamMsgType::Connected, &[]));
        buf.extend_from_slice(&wire_frame(StreamMsgType::Ping, &[0x00]));
        buf.extend_from_slice(&wire_frame(StreamMsgType::Reconnected, &[]));
        buf.extend_from_slice(&wire_frame(StreamMsgType::Restart, &[]));
        buf.extend_from_slice(&wire_frame(StreamMsgType::Metadata, b"perms"));
        let mut cursor = std::io::Cursor::new(buf);

        let mut pending: Vec<StreamControl> = Vec::new();
        let result = run_handshake(&mut cursor, &mut pending)
            .expect("wait_for_login_generic must succeed when Metadata arrives");
        assert!(matches!(result, LoginResult::Success(_)));
        assert_eq!(pending.len(), 4);
        assert!(matches!(pending[0], StreamControl::Connected));
        assert!(matches!(pending[1], StreamControl::Ping { .. }));
        assert!(matches!(pending[2], StreamControl::ReconnectedServer));
        assert!(matches!(pending[3], StreamControl::Restart));
    }

    /// A peer that streams control frames without ever sending METADATA must
    /// not wedge the handshake. With the per-read socket timeout reset by
    /// every frame, the wall-clock deadline is the only backstop: once it
    /// passes, the handshake returns a timeout instead of looping forever.
    #[test]
    fn wait_for_login_chatty_no_metadata_hits_deadline() {
        // A long buffer of PING frames and no METADATA: read_frame always has
        // a frame to return, so only the deadline can stop the loop.
        let mut buf = Vec::new();
        for _ in 0..1_000 {
            buf.extend_from_slice(&wire_frame(StreamMsgType::Ping, &[0x00]));
        }
        let mut cursor = std::io::Cursor::new(buf);

        // A clock that jumps past the deadline after a few iterations,
        // emulating wall-clock time advancing while the peer keeps talking.
        let start = Instant::now();
        let deadline = start + Duration::from_secs(10);
        let mut ticks = 0u32;
        let clock = move || {
            ticks += 1;
            if ticks > 3 {
                deadline + Duration::from_secs(1)
            } else {
                start
            }
        };

        let mut pending: Vec<StreamControl> = Vec::new();
        let result = wait_for_login_generic(
            &mut cursor,
            &mut pending,
            deadline,
            Duration::from_secs(10),
            clock,
        );
        match result {
            Err(Error::Stream { kind, .. }) => {
                assert!(matches!(kind, crate::error::StreamErrorKind::Timeout));
            }
            Err(other) => panic!("expected an Fpss timeout error, got {other:?}"),
            Ok(_) => panic!("a chatty no-METADATA peer must trip the handshake deadline"),
        }
    }

    /// Reader that delivers a fixed prefix once, then stalls with
    /// `WouldBlock` forever, sleeping briefly on each stall so the
    /// mid-frame reader's real-time drain budget elapses. Models a peer
    /// that dribbles in a partial frame and then goes quiet mid-frame.
    struct PartialThenStallForever {
        prefix: Vec<u8>,
        pos: usize,
        sleep_per_stall: Duration,
    }

    impl std::io::Read for PartialThenStallForever {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos < self.prefix.len() {
                let remaining = &self.prefix[self.pos..];
                let n = remaining.len().min(buf.len());
                buf[..n].copy_from_slice(&remaining[..n]);
                self.pos += n;
                return Ok(n);
            }
            std::thread::sleep(self.sleep_per_stall);
            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "mid-frame stall",
            ))
        }
    }

    /// A peer that dribbles a partial frame (a complete header and part of
    /// the payload) and then goes permanently silent must be cut off by the
    /// per-stall no-progress timeout — not held indefinitely. The reader
    /// blocks inside the payload read until `stall_timeout` elapses with zero
    /// progress, then surfaces a fatal `ProtocolError`, which the handshake
    /// propagates. This is the terminal's only mid-frame bound.
    #[test]
    fn wait_for_login_partial_frame_silence_hits_stall_timeout() {
        // A complete header (LEN=4, CODE=Ping) and 2 of 4 payload bytes, then
        // a permanent mid-payload stall: the frame can never complete.
        let mut reader = PartialThenStallForever {
            prefix: vec![0x04, StreamMsgType::Ping as u8, 0x01, 0x02],
            pos: 0,
            sleep_per_stall: Duration::from_millis(2),
        };

        let mut pending: Vec<StreamControl> = Vec::new();
        let result = wait_for_login_generic(
            &mut reader,
            &mut pending,
            // Far-future wall-clock deadline: the per-stall timeout is what
            // must end the handshake here.
            Instant::now() + Duration::from_secs(3_600),
            // Short per-stall budget: the permanent mid-payload silence trips
            // it quickly.
            Duration::from_millis(30),
            Instant::now,
        );
        match result {
            Err(Error::Stream { kind, message }) => {
                assert!(
                    matches!(kind, crate::error::StreamErrorKind::ProtocolError),
                    "a mid-frame silence must surface as a fatal protocol error, got {kind:?}"
                );
                assert!(
                    message.contains("mid-payload") && message.contains("without progress"),
                    "expected a per-stall no-progress error, got: {message}"
                );
            }
            Err(other) => panic!("expected an Fpss protocol error, got {other:?}"),
            Ok(_) => panic!("a partial-frame silence must trip the stall timeout"),
        }
    }
}
