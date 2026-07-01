//! Vendor schema drift coverage.
//!
//! Injects an unknown `StreamMsgType` value mid-session (`code = 99`)
//! after several normal frames. Verifies:
//!
//! - The reader silently skips the unknown opcode (mirrors the
//!   terminal's `case default: continue` frame skip).
//! - Subsequent normal frames after the unknown opcode are consumed
//!   without desync — the reader tracks payload bytes correctly so
//!   the next 2-byte header lands on the right offset.
//! - An unbroken run of unknown opcodes is skipped in full with no
//!   ceiling, matching the terminal, which imposes no reject limit.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use thetadatadx::StreamMsgType;

use thetadatadx::fpss::__test_internals::{
    decode_frame, read_frame_into, DeltaState, FrameReadState, MAX_PAYLOAD_LEN,
};
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{StreamControl, StreamEvent};

fn push_frame(out: &mut Vec<u8>, code: u8, payload: &[u8]) {
    out.push(payload.len() as u8);
    out.push(code);
    out.extend_from_slice(payload);
}

/// One unknown opcode mid-session must NOT abort the reader and the
/// post-unknown CONNECTED + LoginSuccess + ContractAssigned frames
/// must decode normally — proving the unknown-opcode payload was
/// fully consumed and the reader landed on the next header byte.
#[test]
fn single_unknown_opcode_skipped_without_desync() {
    let mut bytes = Vec::new();
    push_frame(&mut bytes, StreamMsgType::Connected as u8, &[]);
    push_frame(&mut bytes, StreamMsgType::Metadata as u8, b"perms");

    // Unknown opcode 99 with a 16-byte payload.
    let unknown_payload: Vec<u8> = (0..16u8).collect();
    push_frame(&mut bytes, 99, &unknown_payload);

    // Normal frame after the unknown — must decode.
    let c = Contract::stock("AAPL");
    let cb = c.to_bytes();
    let mut payload = Vec::new();
    payload.extend_from_slice(&7i32.to_be_bytes());
    payload.extend_from_slice(&cb);
    push_frame(&mut bytes, StreamMsgType::Contract as u8, &payload);

    let mut cursor = Cursor::new(bytes);
    let authenticated = AtomicBool::new(true);
    let shutdown = AtomicBool::new(false);
    let mut local: HashMap<i32, Arc<Contract>> = HashMap::new();
    let mut delta = DeltaState::new();
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let mut state = FrameReadState::new();
    let mut events: Vec<StreamEvent> = Vec::new();

    for _ in 0..16 {
        match read_frame_into(&mut cursor, &mut buf, &mut state) {
            Ok(Some((code, n))) => {
                let (p, s) = decode_frame(
                    code,
                    &buf[..n],
                    &authenticated,
                    &mut local,
                    &shutdown,
                    &mut delta,
                );
                if let Some(e) = p {
                    if let Some(public) = e.as_public() {
                        events.push(public.clone());
                    }
                }
                if let Some(e) = s {
                    if let Some(public) = e.as_public() {
                        events.push(public.clone());
                    }
                }
            }
            Ok(None) => break,
            Err(e) => panic!("schema-drift fixture must not error on a single unknown opcode: {e}"),
        }
    }

    // Required landmarks: we MUST have seen CONNECTED, LoginSuccess,
    // and the post-unknown ContractAssigned (id 7). The unknown
    // opcode itself does NOT surface in the read_frame_into return
    // — it is silently skipped at the framing layer (matches
    // PacketStream2.readFrame default-skip).
    let mut saw_login = false;
    let mut saw_contract_7 = false;
    for e in &events {
        if let StreamEvent::Control(StreamControl::LoginSuccess { .. }) = e {
            saw_login = true;
        }
        if let StreamEvent::Control(StreamControl::ContractAssigned { id, .. }) = e {
            if *id == 7 {
                saw_contract_7 = true;
            }
        }
    }
    assert!(saw_login, "LoginSuccess missing; events: {events:?}");
    assert!(
        saw_contract_7,
        "Post-unknown ContractAssigned id=7 missing — reader desynced; events: {events:?}",
    );
}

/// A long unbroken run of unknown opcodes is skipped in full — the
/// framing layer imposes no reject ceiling, matching the terminal. The
/// reader consumes every unknown frame's payload and reaches clean EOF
/// without ever surfacing a typed frame or a framing error.
#[test]
fn consecutive_unknown_opcodes_are_all_skipped() {
    let mut bytes = Vec::new();
    // 32 unbroken unknown opcodes — far past any historical ceiling.
    for i in 0..32u8 {
        push_frame(&mut bytes, 90u8.wrapping_add(i % 10), &[0xAA]);
    }

    let mut cursor = Cursor::new(bytes);
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let mut state = FrameReadState::new();

    // A single read consumes and skips the entire run of unknown frames,
    // returning clean EOF — never a typed frame, never an error.
    match read_frame_into(&mut cursor, &mut buf, &mut state) {
        Ok(None) => {}
        Ok(Some(_)) => panic!("unknown opcode must not yield a typed frame"),
        Err(e) => panic!("a run of unknown opcodes must not escalate to an error: {e}"),
    }
}

/// Mixed stream: known frame, unknown frame, known frame, unknown frame...
/// Every known frame is returned and every unknown frame is silently
/// consumed, so a sparse drift is delivered intact with no escalation.
#[test]
fn alternating_known_and_unknown_does_not_escalate() {
    let mut bytes = Vec::new();
    push_frame(&mut bytes, StreamMsgType::Connected as u8, &[]);
    for _ in 0..10 {
        push_frame(&mut bytes, 99, &[0x42]); // unknown
        push_frame(&mut bytes, StreamMsgType::Ping as u8, &[0x00]); // known reset
    }

    let mut cursor = Cursor::new(bytes);
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_PAYLOAD_LEN);
    let mut state = FrameReadState::new();

    let mut frames = 0;
    for _ in 0..64 {
        match read_frame_into(&mut cursor, &mut buf, &mut state) {
            Ok(Some(_)) => frames += 1,
            Ok(None) => break,
            Err(e) => panic!("alternating sparse drift must not escalate: {e}"),
        }
    }
    // 1 Connected + 10 Pings = 11 known frames; the 10 unknowns are
    // silently consumed.
    assert_eq!(frames, 11, "expected 11 known frames after sparse drift");
}
