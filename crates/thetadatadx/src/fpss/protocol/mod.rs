//! FPSS message types, contract serialization, and subscription protocol.
//!
//! # Wire protocol (from decompiled Java)
//!
//! ## Message codes (`StreamMsgType` in Java)
//!
//! Source: `StreamMsgType.java` — enum with byte codes for each message direction.
//! See [`tdbe::types::enums::StreamMsgType`] for the Rust enum.
//!
//! ## Contract serialization (`Contract.java`)
//!
//! Contracts are serialized as a compact binary format on the wire:
//!
//! - **Stock/Index**: `[total_size: u8] [root_len: u8] [root ASCII] [sec_type: u8]`
//! - **Option**:      `[total_size: u8] [root_len: u8] [root ASCII] [sec_type: u8]
//!                      [exp_date: i32 BE] [is_call: u8] [strike: i32 BE]`
//!
//! Source: `Contract.toBytes()` and `Contract.fromBytes()` in decompiled terminal.
//!
//! ## Authentication (`FPSSClient.java`)
//!
//! CREDENTIALS message (code 0) payload:
//! ```text
//! [0x00] [username_len: u16 BE] [username bytes] [password bytes]
//! ```
//!
//! Source: `FPSSClient.sendCredentials()` in decompiled terminal.
//!
//! ## Subscription (`FPSSClient.java`, `PacketStream.java`)
//!
//! Subscribe payload: `[req_id: i32 BE] [contract bytes]`
//! Full-type subscribe: `[req_id: i32 BE] [sec_type: u8]` (5 bytes, subscribes all of that type)
//! Unsubscribe payload: same format as subscribe, using REMOVE_* codes.
//!
//! Response (code 40): `[req_id: i32 BE] [resp_code: i32 BE]`
//!   - 0 = OK, 1 = ERROR, 2 = `MAX_STREAMS`, 3 = `INVALID_PERMS`
//!
//! Source: `PacketStream.addQuote()`, `PacketStream.removeQuote()`,
//!         `FPSSClient.onReqResponse()` in decompiled terminal.
//!
//! # Sub-modules
//!
//! - [`contract`] — `Contract` struct, OCC-21 parser, wire codec.
//! - [`wire`] — payload builders / parsers (credentials, subscribe, ping, stop, REQ_RESPONSE, CONTRACT, DISCONNECTED).
//! - [`subscription`] — `SubscriptionKind` enum (Quote / Trade / OpenInterest).

pub mod contract;
pub mod subscription;
pub mod wire;

pub use self::contract::{Contract, ContractParseError};
pub use self::subscription::SubscriptionKind;
pub use self::wire::{
    build_credentials_payload, build_full_type_subscribe_payload, build_ping_payload,
    build_stop_payload, build_subscribe_payload, parse_contract_message, parse_disconnect_reason,
    parse_req_response,
};

/// Maximum payload size for a single FPSS frame (1-byte length field).
///
/// Source: `PacketStream.java` — `LEN` field is a single unsigned byte.
pub const MAX_PAYLOAD: usize = 255;

/// Ping interval in milliseconds.
///
/// Source: `FPSSClient.java` — heartbeat thread sends PING every 100ms after login.
pub const PING_INTERVAL_MS: u64 = 100;

/// Reconnect delay in milliseconds after `IOException`.
///
/// Source: `FPSSClient.java` — `RECONNECT_DELAY_MS = 2000`.
pub const RECONNECT_DELAY_MS: u64 = 2_000;

/// Delay before reconnecting after `TOO_MANY_REQUESTS` disconnect (milliseconds).
///
/// Source: `FPSSClient.java` — waits 130 seconds on `RemoveReason.TOO_MANY_REQUESTS`.
pub const TOO_MANY_REQUESTS_DELAY_MS: u64 = 130_000;

/// Socket connect timeout in milliseconds.
///
/// Source: `FPSSClient.java` — `socket.connect(addr, 2000)`.
pub const CONNECT_TIMEOUT_MS: u64 = 2_000;

/// Socket read timeout in milliseconds.
///
/// Source: `FPSSClient.java` — `socket.setSoTimeout(10000)`.
pub const READ_TIMEOUT_MS: u64 = 10_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_payload_cap_is_one_byte() {
        // PacketStream.java uses a single-byte LEN field; cap MUST be 255.
        assert_eq!(MAX_PAYLOAD, 255);
    }

    #[test]
    fn ping_interval_matches_java_heartbeat() {
        // FPSSClient.java sends PING every 100ms after login.
        assert_eq!(PING_INTERVAL_MS, 100);
    }

    #[test]
    fn reconnect_delays_match_java() {
        // 2000ms general reconnect, 130s TOO_MANY_REQUESTS cooldown.
        assert_eq!(RECONNECT_DELAY_MS, 2_000);
        assert_eq!(TOO_MANY_REQUESTS_DELAY_MS, 130_000);
    }

    #[test]
    fn socket_timeouts_match_java() {
        // FPSSClient.java: socket.connect(addr, 2000), setSoTimeout(10000).
        assert_eq!(CONNECT_TIMEOUT_MS, 2_000);
        assert_eq!(READ_TIMEOUT_MS, 10_000);
    }
}
