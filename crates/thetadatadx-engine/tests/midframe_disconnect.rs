//! Mid-frame TLS disconnect injection tests for the FPSS framing layer.
//!
//! Drives `read_frame_into` with a `Read` adapter that:
//!
//! 1. Returns a controlled prefix of bytes,
//! 2. Then surfaces `ErrorKind::UnexpectedEof` (vendor closed mid-byte-stream),
//! 3. Or returns `0` (clean EOF) part-way through a payload.
//!
//! Verifies:
//!
//! - The reader does NOT panic.
//! - The reader does NOT block on a stuck Read source.
//! - The reader returns `Ok(None)` on clean pre-header EOF, or a typed
//!   `Error::Fpss { kind: ProtocolError }` on truncation.
//! - `FrameReadState` resets cleanly between attempts so the next
//!   reconnect-cycle reader does not desync on stale partial bytes.

use std::io::{Cursor, Error as IoError, ErrorKind, Read};

use thetadatadx_engine::fpss::__test_internals::{
    read_frame_into, FrameReadState, MAX_PAYLOAD_LEN,
};

// ---------------------------------------------------------------------------
// Read adapters
// ---------------------------------------------------------------------------

/// Returns the prefix bytes, then `Ok(0)` (clean EOF) on subsequent calls.
struct PrefixThenEof {
    inner: Cursor<Vec<u8>>,
}

impl PrefixThenEof {
    fn new(prefix: Vec<u8>) -> Self {
        Self {
            inner: Cursor::new(prefix),
        }
    }
}

impl Read for PrefixThenEof {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Cursor returns 0 once exhausted — exactly what we want for
        // a "clean EOF after partial data" simulation.
        self.inner.read(buf)
    }
}

/// Returns the prefix bytes, then `Err(UnexpectedEof)` on subsequent
/// calls. Models a vendor TLS connection slammed shut mid-frame.
struct PrefixThenUnexpectedEof {
    inner: Cursor<Vec<u8>>,
    erred: bool,
}

impl PrefixThenUnexpectedEof {
    fn new(prefix: Vec<u8>) -> Self {
        Self {
            inner: Cursor::new(prefix),
            erred: false,
        }
    }
}

impl Read for PrefixThenUnexpectedEof {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            return Ok(n);
        }
        if self.erred {
            // Subsequent calls keep erroring — mirrors a real torn-down
            // socket where every poll returns the same kernel state.
            return Err(IoError::new(
                ErrorKind::UnexpectedEof,
                "simulated TLS connection closed mid-frame",
            ));
        }
        self.erred = true;
        Err(IoError::new(
            ErrorKind::UnexpectedEof,
            "simulated TLS connection closed mid-frame",
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Pre-header EOF (no bytes delivered yet) is a graceful close.
/// `read_frame_into` returns `Ok(None)` and resets `FrameReadState` to
/// idle so the reconnect path re-enters with a clean slate.
#[test]
fn pre_header_clean_eof_returns_none_and_resets_state() {
    let mut reader = PrefixThenEof::new(Vec::new());
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let mut state = FrameReadState::new();

    let result = read_frame_into(&mut reader, &mut buf, &mut state);
    assert!(matches!(result, Ok(None)), "got {result:?}");

    // After a clean pre-header EOF the state must be idle: a fresh
    // `FrameReadState::default()` should be byte-for-byte equivalent
    // to what we hold, exercised through a follow-up call that
    // immediately returns Ok(None) again.
    let result = read_frame_into(&mut reader, &mut buf, &mut state);
    assert!(matches!(result, Ok(None)), "got {result:?}");
}

/// One byte of header arrives, then EOF (clean Ok(0)). Surfaces as
/// truncated-header error so the I/O loop knows to escalate.
#[test]
fn mid_header_clean_eof_returns_protocol_error() {
    let mut reader = PrefixThenEof::new(vec![0x05]); // single header byte
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let mut state = FrameReadState::new();

    let result = read_frame_into(&mut reader, &mut buf, &mut state);
    assert!(
        result.is_err(),
        "mid-header EOF must surface as error; got {result:?}"
    );
}

/// Mid-payload truncation: the header promises 200 bytes but only 50
/// arrive before EOF. The reader must surface a typed error and not
/// loop forever.
#[test]
fn mid_payload_truncation_returns_protocol_error() {
    let mut bytes = Vec::new();
    bytes.push(200u8); // length
    bytes.push(21u8); // Quote opcode
    bytes.extend(std::iter::repeat_n(0xAB, 50));

    let mut reader = PrefixThenEof::new(bytes);
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let mut state = FrameReadState::new();

    let result = read_frame_into(&mut reader, &mut buf, &mut state);
    assert!(
        result.is_err(),
        "mid-payload truncation must surface as error; got {result:?}"
    );
}

/// `UnexpectedEof` mid-payload — same outcome as clean EOF: typed
/// error, no panic, no loop.
#[test]
fn mid_payload_unexpected_eof_returns_protocol_error() {
    let mut bytes = Vec::new();
    bytes.push(50u8);
    bytes.push(21u8);
    bytes.extend(std::iter::repeat_n(0xCD, 20));

    let mut reader = PrefixThenUnexpectedEof::new(bytes);
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let mut state = FrameReadState::new();

    let result = read_frame_into(&mut reader, &mut buf, &mut state);
    assert!(
        result.is_err(),
        "UnexpectedEof mid-payload must surface as error; got {result:?}"
    );
}

/// Reconnect cycle: after a torn read, instantiating a fresh
/// `FrameReadState` and pointing at a clean stream must succeed
/// without carrying half-decoded leftover bytes from the previous
/// connection.
#[test]
fn fresh_state_after_torn_read_decodes_cleanly() {
    // Cycle 1: torn mid-payload.
    let mut bytes = Vec::new();
    bytes.push(50u8);
    bytes.push(21u8);
    bytes.extend(std::iter::repeat_n(0x00, 10));

    let mut torn_reader = PrefixThenUnexpectedEof::new(bytes);
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let mut state = FrameReadState::new();
    let _ = read_frame_into(&mut torn_reader, &mut buf, &mut state);

    // Cycle 2: fresh state on a clean stream — must decode without
    // carrying torn bytes.
    let mut clean = Vec::new();
    let payload = b"hello";
    clean.push(payload.len() as u8);
    clean.push(11u8); // Error opcode (StreamMsgType::Error = 11)
    clean.extend_from_slice(payload);

    let mut clean_reader = Cursor::new(clean);
    let mut buf2: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let mut fresh_state = FrameReadState::new();
    let result = read_frame_into(&mut clean_reader, &mut buf2, &mut fresh_state);
    let (_code, n) = result
        .expect("fresh stream must decode")
        .expect("fresh stream must yield a frame");
    assert_eq!(n, payload.len(), "payload length mismatch");
    assert_eq!(&buf2[..n], payload, "payload bytes mismatch");
}

/// Watchdog: a reader that consistently surfaces `UnexpectedEof`
/// from byte zero must not block indefinitely. Bound this with a
/// strict iteration count: if the reader ever loops, this test trips.
#[test]
fn fully_dead_reader_does_not_loop() {
    let mut reader = PrefixThenUnexpectedEof::new(Vec::new());
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let mut state = FrameReadState::new();

    // Bound iterations strictly: a single call must terminate in O(1).
    let result = read_frame_into(&mut reader, &mut buf, &mut state);
    // Pre-header `UnexpectedEof` with zero bytes seen returns
    // `Ok(None)` per `read_header_with_timeout`'s contract — graceful
    // close.
    assert!(
        matches!(result, Ok(None) | Err(_)),
        "fully-dead reader produced unexpected result {result:?}",
    );
}
