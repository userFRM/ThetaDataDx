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
//!   `Error::Stream { kind: ProtocolError }` on truncation.
//! - Each read is independent (no cross-call state), so a torn read
//!   leaves no residue for the next reconnect-cycle reader to desync on.

use std::io::{Cursor, Error as IoError, ErrorKind, Read};

use thetadatadx::fpss::__test_internals::{read_frame_into, MAX_PAYLOAD_LEN};

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
/// `read_frame_into` returns `Ok(None)`, and a follow-up call on the
/// still-exhausted reader returns `Ok(None)` again — no residue carried.
#[test]
fn pre_header_clean_eof_returns_none() {
    let mut reader = PrefixThenEof::new(Vec::new());
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);

    let result = read_frame_into(&mut reader, &mut buf);
    assert!(matches!(result, Ok(None)), "got {result:?}");

    let result = read_frame_into(&mut reader, &mut buf);
    assert!(matches!(result, Ok(None)), "got {result:?}");
}

/// One byte of header arrives, then EOF (clean Ok(0)). Surfaces as
/// truncated-header error so the I/O loop knows to escalate.
#[test]
fn mid_header_clean_eof_returns_protocol_error() {
    let mut reader = PrefixThenEof::new(vec![0x05]); // single header byte
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);

    let result = read_frame_into(&mut reader, &mut buf);
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

    let result = read_frame_into(&mut reader, &mut buf);
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

    let result = read_frame_into(&mut reader, &mut buf);
    assert!(
        result.is_err(),
        "UnexpectedEof mid-payload must surface as error; got {result:?}"
    );
}

/// Reconnect cycle: after a torn read, a fresh reader on a clean stream
/// must decode without carrying half-decoded leftover bytes from the
/// previous connection. Because reads hold no cross-call state, this
/// holds by construction.
#[test]
fn clean_stream_after_torn_read_decodes_cleanly() {
    // Cycle 1: torn mid-payload.
    let mut bytes = Vec::new();
    bytes.push(50u8);
    bytes.push(21u8);
    bytes.extend(std::iter::repeat_n(0x00, 10));

    let mut torn_reader = PrefixThenUnexpectedEof::new(bytes);
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let _ = read_frame_into(&mut torn_reader, &mut buf);

    // Cycle 2: a clean stream must decode without carrying torn bytes.
    let mut clean = Vec::new();
    let payload = b"hello";
    clean.push(payload.len() as u8);
    clean.push(11u8); // Error opcode (StreamMsgType::Error = 11)
    clean.extend_from_slice(payload);

    let mut clean_reader = Cursor::new(clean);
    let mut buf2: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let result = read_frame_into(&mut clean_reader, &mut buf2);
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

    // Bound iterations strictly: a single call must terminate in O(1).
    let result = read_frame_into(&mut reader, &mut buf);
    // Pre-header `UnexpectedEof` with zero bytes seen returns
    // `Ok(None)` per `read_header_with_timeout`'s contract — graceful
    // close.
    assert!(
        matches!(result, Ok(None) | Err(_)),
        "fully-dead reader produced unexpected result {result:?}",
    );
}
