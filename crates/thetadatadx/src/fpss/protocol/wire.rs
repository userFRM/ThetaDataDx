//! Wire-payload builders and parsers for FPSS messages.
//!
//! Builders cover the client->server direction (credentials, subscribe,
//! full-type subscribe, ping, stop). Parsers cover the server->client
//! responses (REQ_RESPONSE, DISCONNECTED, CONTRACT).
//!
//! Behaviour conforms to the ThetaData FPSS wire protocol.

use crate::tdbe::types::enums::{RemoveReason, SecType, StreamResponseType};

use super::contract::Contract;
use crate::error::Error;

// ---------------------------------------------------------------------------
// Credentials payload
// ---------------------------------------------------------------------------

/// Build the CREDENTIALS (code 0) message payload.
///
/// # Wire format
///
/// ```text
/// [0x00] [username_len: u16 BE] [username bytes] [password bytes]
/// ```
///
/// The leading 0x00 byte is a version/flag byte.
/// `username_len` is the byte-length of the username (email), as a big-endian u16.
/// Password bytes follow immediately with no length prefix — the server infers
/// password length from `payload_len - 3 - username_len`.
#[must_use]
pub fn build_credentials_payload(username: &str, password: &str) -> Vec<u8> {
    let user_bytes = username.as_bytes();
    let pass_bytes = password.as_bytes();

    // Match the wire's `putShort((byte)len)` behavior: the length is first
    // narrowed to a byte (i8), then sign-extended to a short (i16). For
    // lengths 0-127 this is identical to a plain u16 cast. For lengths
    // 128-255 the sign extension sets the high byte to 0xFF. In practice
    // usernames are always <128 bytes, but we match the exact wire
    // encoding for correctness. The truncation to i8 is intentional.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let user_len = i16::from(user_bytes.len() as i8);

    // 1 (version) + 2 (user_len) + user + pass
    let mut buf = Vec::with_capacity(3 + user_bytes.len() + pass_bytes.len());
    buf.push(0x00); // version/flag byte
    buf.extend_from_slice(&user_len.to_be_bytes());
    buf.extend_from_slice(user_bytes);
    buf.extend_from_slice(pass_bytes);
    buf
}

// ---------------------------------------------------------------------------
// Subscription payloads
// ---------------------------------------------------------------------------

/// Build a subscription payload for a specific contract.
///
/// # Wire format
///
/// ```text
/// [req_id: i32 BE] [contract bytes]
/// ```
///
/// The message code (21=QUOTE, 22=TRADE, `23=OPEN_INTEREST`) is set by the caller
/// in the frame header; this function only builds the payload.
///
/// # Errors
///
/// Returns [`Error::Config`] if the contract root is empty or longer
/// than 16 bytes, surfacing the wire-protocol invariant from
/// [`Contract::try_to_bytes`].
pub fn build_subscribe_payload(req_id: i32, contract: &Contract) -> Result<Vec<u8>, Error> {
    let contract_bytes = contract.try_to_bytes()?;
    let mut buf = Vec::with_capacity(4 + contract_bytes.len());
    buf.extend_from_slice(&req_id.to_be_bytes());
    buf.extend_from_slice(&contract_bytes);
    Ok(buf)
}

/// Build a full-type subscription payload (subscribe to all contracts of a security type).
///
/// # Wire format
///
/// ```text
/// [req_id: i32 BE] [sec_type: u8]
/// ```
///
/// Total 5 bytes. The server uses the 5-byte length to distinguish this from
/// a per-contract subscription (which is always longer).
#[must_use]
pub fn build_full_type_subscribe_payload(req_id: i32, sec_type: SecType) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5);
    buf.extend_from_slice(&req_id.to_be_bytes());
    buf.push(sec_type as u8);
    buf
}

/// Build the PING (code 10) payload.
///
/// Heartbeat sends a 1-byte zero payload every 100ms.
#[must_use]
pub fn build_ping_payload() -> Vec<u8> {
    vec![0x00]
}

/// Build the STOP (code 32) payload sent by the client on shutdown.
///
/// `sendStop()` sends an empty-ish STOP message.
#[must_use]
pub fn build_stop_payload() -> Vec<u8> {
    vec![0x00]
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Parse a `REQ_RESPONSE` (code 40) payload.
///
/// # Wire format
///
/// ```text
/// [req_id: i32 BE] [resp_code: i32 BE]
/// ```
///
/// Returns `(req_id, response_type)`.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn parse_req_response(payload: &[u8]) -> Result<(i32, StreamResponseType), Error> {
    if payload.len() < 8 {
        return Err(Error::Fpss {
            kind: crate::error::FpssErrorKind::ProtocolError,
            message: format!(
                "REQ_RESPONSE payload too short: {} bytes, expected 8",
                payload.len()
            ),
        });
    }

    let req_id = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let resp_code = i32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);

    let resp_type = match resp_code {
        0 => StreamResponseType::Subscribed,
        1 => StreamResponseType::Error,
        2 => StreamResponseType::MaxStreamsReached,
        3 => StreamResponseType::InvalidPerms,
        _ => {
            return Err(Error::Fpss {
                kind: crate::error::FpssErrorKind::ProtocolError,
                message: format!("unknown REQ_RESPONSE code: {resp_code}"),
            });
        }
    };

    Ok((req_id, resp_type))
}

/// Parse a DISCONNECTED (code 12) payload.
///
/// # Wire format
///
/// ```text
/// [reason: i16 BE]
/// ```
///
/// 2-byte big-endian `RemoveReason` code.
#[must_use]
pub fn parse_disconnect_reason(payload: &[u8]) -> RemoveReason {
    if payload.len() < 2 {
        return RemoveReason::Unspecified;
    }
    let code = i16::from_be_bytes([payload[0], payload[1]]);
    match code {
        0 => RemoveReason::InvalidCredentials,
        1 => RemoveReason::InvalidLoginValues,
        2 => RemoveReason::InvalidLoginSize,
        3 => RemoveReason::GeneralValidationError,
        4 => RemoveReason::TimedOut,
        5 => RemoveReason::ClientForcedDisconnect,
        6 => RemoveReason::AccountAlreadyConnected,
        7 => RemoveReason::SessionTokenExpired,
        8 => RemoveReason::InvalidSessionToken,
        9 => RemoveReason::FreeAccount,
        12 => RemoveReason::TooManyRequests,
        13 => RemoveReason::NoStartDate,
        14 => RemoveReason::LoginTimedOut,
        15 => RemoveReason::ServerRestarting,
        16 => RemoveReason::SessionTokenNotFound,
        17 => RemoveReason::ServerUserDoesNotExist,
        18 => RemoveReason::InvalidCredentialsNullUser,
        _ => RemoveReason::Unspecified,
    }
}

/// Parse a CONTRACT (code 20) payload.
///
/// # Wire format
///
/// ```text
/// [contract_id: i32 BE] [contract bytes...]
/// ```
///
/// The server assigns a numeric `contract_id` used to identify this contract
/// in subsequent QUOTE/TRADE/OHLCVC data messages. The contract bytes use the
/// same serialization as `Contract::to_bytes()`.
///
/// Returns `(server_assigned_id, contract)`.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn parse_contract_message(payload: &[u8]) -> Result<(i32, Contract), Error> {
    if payload.len() < 5 {
        return Err(Error::Fpss {
            kind: crate::error::FpssErrorKind::ProtocolError,
            message: format!("CONTRACT payload too short: {} bytes", payload.len()),
        });
    }

    let contract_id = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let (contract, _consumed) = Contract::from_bytes(&payload[4..]).map_err(|e| Error::Fpss {
        kind: crate::error::FpssErrorKind::ProtocolError,
        message: format!("failed to parse contract: {e}"),
    })?;

    Ok((contract_id, contract))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_payload_format() {
        let payload = build_credentials_payload("user@test.com", "pass123");
        assert_eq!(payload[0], 0x00); // version byte
        let user_len = u16::from_be_bytes([payload[1], payload[2]]);
        assert_eq!(user_len, 13); // "user@test.com".len()
        assert_eq!(&payload[3..16], b"user@test.com");
        assert_eq!(&payload[16..], b"pass123");
    }

    #[test]
    fn subscribe_payload_with_stock() {
        let contract = Contract::stock("MSFT");
        let payload = build_subscribe_payload(42, &contract).expect("valid root");
        // req_id + contract header (frame is 11 bytes).
        assert_eq!(payload.len(), 11);
        let req_id = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
        assert_eq!(req_id, 42);
        // Rest is the contract bytes
        let (parsed, _) = Contract::from_bytes(&payload[4..]).unwrap();
        assert_eq!(parsed, contract);
    }

    #[test]
    fn build_subscribe_payload_rejects_oversize_root() {
        let contract = Contract::stock("ABCDEFGHIJKLMNOPQ"); // 17 chars
        let err = build_subscribe_payload(1, &contract).expect_err("too-long root must error");
        match err {
            Error::Config { message, .. } => assert!(message.contains("too long")),
            other => panic!("expected Error::Config, got {other:?}"),
        }
    }

    #[test]
    fn build_subscribe_payload_rejects_empty_root() {
        let contract = Contract::stock("");
        let err = build_subscribe_payload(1, &contract).expect_err("empty root must error");
        match err {
            Error::Config { kind, .. } => assert!(matches!(
                kind,
                crate::error::ConfigErrorKind::MissingField(_)
            )),
            other => panic!("expected Error::Config, got {other:?}"),
        }
    }

    #[test]
    fn full_type_subscribe_payload() {
        let payload = build_full_type_subscribe_payload(99, SecType::Stock);
        assert_eq!(payload.len(), 5);
        let req_id = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
        assert_eq!(req_id, 99);
        assert_eq!(payload[4], SecType::Stock as u8);
    }

    #[test]
    fn parse_req_response_ok() {
        let mut data = Vec::new();
        data.extend_from_slice(&42i32.to_be_bytes());
        data.extend_from_slice(&0i32.to_be_bytes()); // Subscribed
        let (req_id, resp) = parse_req_response(&data).unwrap();
        assert_eq!(req_id, 42);
        assert_eq!(resp, StreamResponseType::Subscribed);
    }

    #[test]
    fn parse_req_response_max_streams() {
        let mut data = Vec::new();
        data.extend_from_slice(&1i32.to_be_bytes());
        data.extend_from_slice(&2i32.to_be_bytes()); // MaxStreamsReached
        let (req_id, resp) = parse_req_response(&data).unwrap();
        assert_eq!(req_id, 1);
        assert_eq!(resp, StreamResponseType::MaxStreamsReached);
    }

    #[test]
    fn parse_req_response_too_short() {
        let data = [0u8; 7];
        let err = parse_req_response(&data).unwrap_err();
        assert!(err.to_string().contains("too short"));
    }

    #[test]
    fn parse_disconnect_reasons() {
        let make = |code: i16| {
            let bytes = code.to_be_bytes();
            parse_disconnect_reason(&bytes)
        };

        assert_eq!(make(0), RemoveReason::InvalidCredentials);
        assert_eq!(make(6), RemoveReason::AccountAlreadyConnected);
        assert_eq!(make(12), RemoveReason::TooManyRequests);
        assert_eq!(make(15), RemoveReason::ServerRestarting);
        assert_eq!(make(-99), RemoveReason::Unspecified);
    }

    #[test]
    fn parse_disconnect_reason_empty() {
        assert_eq!(parse_disconnect_reason(&[]), RemoveReason::Unspecified);
    }

    #[test]
    fn parse_contract_message_stock() {
        // Build a CONTRACT payload: 4-byte id + contract bytes
        let contract = Contract::stock("TSLA");
        let contract_bytes = contract.to_bytes();
        let mut payload = Vec::new();
        payload.extend_from_slice(&7i32.to_be_bytes());
        payload.extend_from_slice(&contract_bytes);

        let (id, parsed) = parse_contract_message(&payload).unwrap();
        assert_eq!(id, 7);
        assert_eq!(parsed, contract);
    }

    #[test]
    fn ping_payload() {
        let p = build_ping_payload();
        assert_eq!(p, vec![0x00]);
    }
}
