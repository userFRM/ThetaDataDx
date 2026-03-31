//! FPSS message types, contract serialization, and subscription protocol.
//!
//! # Wire protocol (from decompiled Java)
//!
//! ## Message codes (`StreamMsgType` in Java)
//!
//! Source: `StreamMsgType.java` -- enum with byte codes for each message direction.
//! See [`crate::types::enums::StreamMsgType`] for the Rust enum.
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
//!   - 0 = OK, 1 = ERROR, 2 = MAX_STREAMS, 3 = INVALID_PERMS
//!
//! Source: `PacketStream.addQuote()`, `PacketStream.removeQuote()`,
//!         `FPSSClient.onReqResponse()` in decompiled terminal.

pub mod framing;

use crate::types::enums::{RemoveReason, SecType, StreamMsgType, StreamResponseType};

/// Maximum payload size for a single FPSS frame (1-byte length field).
///
/// Source: `PacketStream.java` — `LEN` field is a single unsigned byte.
pub const MAX_PAYLOAD: usize = 255;

/// Ping interval in milliseconds.
///
/// Source: `FPSSClient.java` — heartbeat thread sends PING every 100ms after login.
pub const PING_INTERVAL_MS: u64 = 100;

/// Reconnect delay in milliseconds after IOException.
///
/// Source: `FPSSClient.java` — `RECONNECT_DELAY_MS = 2000`.
pub const RECONNECT_DELAY_MS: u64 = 2_000;

/// Delay before reconnecting after TOO_MANY_REQUESTS disconnect (milliseconds).
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

/// FPSS server endpoints.
///
/// Source: `FPSSClient.java` — `SERVERS` array, four entries across two sites.
pub const SERVERS: &[(&str, u16)] = &[
    ("nj-a.thetadata.us", 20000),
    ("nj-a.thetadata.us", 20001),
    ("nj-b.thetadata.us", 20000),
    ("nj-b.thetadata.us", 20001),
];

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// A contract identifier for FPSS subscriptions.
///
/// Matches the wire format from `Contract.java`:
/// - Stock/Index/Rate: root ticker + security type
/// - Option: root ticker + security type + expiration + call/put + strike
///
/// Source: `Contract.java` — `toBytes()`, `fromBytes()`, constructor overloads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Contract {
    /// Root ticker symbol (ASCII, max ~6 chars in practice).
    pub root: String,
    /// Security type.
    pub sec_type: SecType,
    /// Expiration date as YYYYMMDD integer (options only).
    pub exp_date: Option<i32>,
    /// True = call, false = put (options only).
    pub is_call: Option<bool>,
    /// Strike price in fixed-point (options only). The encoding matches
    /// ThetaData's integer strike representation.
    pub strike: Option<i32>,
}

impl Contract {
    /// Create a stock contract.
    ///
    /// Source: `Contract(String root)` constructor in `Contract.java` — defaults to STOCK.
    pub fn stock(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            sec_type: SecType::Stock,
            exp_date: None,
            is_call: None,
            strike: None,
        }
    }

    /// Create an index contract.
    pub fn index(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            sec_type: SecType::Index,
            exp_date: None,
            is_call: None,
            strike: None,
        }
    }

    /// Create a rate contract.
    pub fn rate(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            sec_type: SecType::Rate,
            exp_date: None,
            is_call: None,
            strike: None,
        }
    }

    /// Create an option contract.
    ///
    /// Source: `Contract(String root, int expDate, boolean isCall, int strike)`
    /// constructor in `Contract.java`.
    ///
    /// # Arguments
    /// - `root`: Underlying ticker (e.g., "AAPL")
    /// - `exp_date`: Expiration as YYYYMMDD integer (e.g., 20260320)
    /// - `is_call`: true for call, false for put
    /// - `strike`: Strike price in ThetaData's integer encoding
    pub fn option(root: impl Into<String>, exp_date: i32, is_call: bool, strike: i32) -> Self {
        Self {
            root: root.into(),
            sec_type: SecType::Option,
            exp_date: Some(exp_date),
            is_call: Some(is_call),
            strike: Some(strike),
        }
    }

    /// Serialize to the wire format used in FPSS subscription messages.
    ///
    /// # Wire format (from `Contract.toBytes()`)
    ///
    /// Stock/Index/Rate:
    /// ```text
    /// [total_size: u8] [root_len: u8] [root ASCII bytes] [sec_type: u8]
    /// ```
    ///
    /// Option:
    /// ```text
    /// [total_size: u8] [root_len: u8] [root ASCII bytes] [sec_type: u8]
    /// [exp_date: i32 BE] [is_call: u8] [strike: i32 BE]
    /// ```
    ///
    /// `total_size` counts the entire buffer including itself, matching Java's
    /// `Contract.toBytes()` exactly:
    ///   - Stock: `3 + root.length()` = size(1) + root_len(1) + root(N) + sec_type(1)
    ///   - Option: `12 + root.length()` = size(1) + root_len(1) + root(N) + sec_type(1) + exp(4) + is_call(1) + strike(4)
    ///
    /// Java's `fromBytes()` validates `len == size`, confirming the size byte
    /// counts itself.
    pub fn to_bytes(&self) -> Vec<u8> {
        let root_bytes = self.root.as_bytes();
        assert!(
            root_bytes.len() <= 16,
            "contract root too long: {} bytes (max 16 to match Java Contract.toBytes())",
            root_bytes.len()
        );
        let root_len = root_bytes.len() as u8;

        let is_option = self.sec_type == SecType::Option;

        // Java: `3 + root.length()` for non-option, `12 + root.length()` for option.
        // The size byte counts itself: size(1) + root_len(1) + root(N) + sec_type(1) [+ option fields(9)]
        let total_size = if is_option {
            12 + root_bytes.len()
        } else {
            3 + root_bytes.len()
        };

        let mut buf = Vec::with_capacity(total_size);

        // total_size byte (includes itself — matches Java's Contract.toBytes())
        buf.push(total_size as u8);
        // root_len
        buf.push(root_len);
        // root ASCII
        buf.extend_from_slice(root_bytes);
        // sec_type
        buf.push(self.sec_type as u8);

        if is_option {
            // exp_date: i32 big-endian
            buf.extend_from_slice(&self.exp_date.unwrap_or(0).to_be_bytes());
            // is_call: u8 (1 = call, 0 = put)
            buf.push(if self.is_call.unwrap_or(false) { 1 } else { 0 });
            // strike: i32 big-endian
            buf.extend_from_slice(&self.strike.unwrap_or(0).to_be_bytes());
        }

        buf
    }

    /// Deserialize from the wire format.
    ///
    /// Input starts at the `total_size` byte (the first byte of `Contract.toBytes()` output).
    ///
    /// Source: `Contract.fromBytes()` in `Contract.java`.
    pub fn from_bytes(data: &[u8]) -> Result<(Self, usize), ContractParseError> {
        if data.is_empty() {
            return Err(ContractParseError::TooShort);
        }

        // Java's size byte counts itself: the total buffer length equals the size byte value.
        // Java fromBytes: `if (len != size) throw ...` where len is the total span including size.
        let total_size = data[0] as usize;
        if data.len() < total_size {
            return Err(ContractParseError::TooShort);
        }

        // Minimum: size(1) + root_len(1) + root(>=0) + sec_type(1) = 3
        if total_size < 3 {
            return Err(ContractParseError::InvalidSize(total_size));
        }

        let root_len = data[1] as usize;
        // Validate: size(1) + root_len(1) + root(N) + sec_type(1) <= total_size
        if total_size < 2 + root_len + 1 {
            return Err(ContractParseError::InvalidSize(total_size));
        }

        let root_start = 2;
        let root_end = root_start + root_len;
        let root = std::str::from_utf8(&data[root_start..root_end])
            .map_err(|_| ContractParseError::InvalidUtf8)?
            .to_string();

        let sec_type_byte = data[root_end];
        let sec_type = SecType::from_code(sec_type_byte as i32)
            .ok_or(ContractParseError::UnknownSecType(sec_type_byte))?;

        if sec_type == SecType::Option {
            // Need 9 more bytes after sec_type: exp_date(4) + is_call(1) + strike(4)
            let opt_start = root_end + 1;
            if data.len() < opt_start + 9 {
                return Err(ContractParseError::TooShort);
            }

            let exp_date = i32::from_be_bytes([
                data[opt_start],
                data[opt_start + 1],
                data[opt_start + 2],
                data[opt_start + 3],
            ]);
            let is_call = data[opt_start + 4] != 0;
            let strike = i32::from_be_bytes([
                data[opt_start + 5],
                data[opt_start + 6],
                data[opt_start + 7],
                data[opt_start + 8],
            ]);

            Ok((
                Contract {
                    root,
                    sec_type,
                    exp_date: Some(exp_date),
                    is_call: Some(is_call),
                    strike: Some(strike),
                },
                total_size,
            ))
        } else {
            Ok((
                Contract {
                    root,
                    sec_type,
                    exp_date: None,
                    is_call: None,
                    strike: None,
                },
                total_size,
            ))
        }
    }
}

impl std::fmt::Display for Contract {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.sec_type {
            SecType::Option => {
                let right = if self.is_call.unwrap_or(false) {
                    "C"
                } else {
                    "P"
                };
                write!(
                    f,
                    "{} {} {} {} {}",
                    self.root,
                    self.sec_type.as_str(),
                    self.exp_date.unwrap_or(0),
                    right,
                    self.strike.unwrap_or(0),
                )
            }
            _ => write!(f, "{} {}", self.root, self.sec_type.as_str()),
        }
    }
}

/// Errors that can occur when parsing a contract from bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractParseError {
    TooShort,
    InvalidSize(usize),
    InvalidUtf8,
    UnknownSecType(u8),
}

impl std::fmt::Display for ContractParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "contract data too short"),
            Self::InvalidSize(s) => write!(f, "invalid contract total_size: {s}"),
            Self::InvalidUtf8 => write!(f, "contract root is not valid UTF-8"),
            Self::UnknownSecType(c) => write!(f, "unknown sec_type code: {c}"),
        }
    }
}

impl std::error::Error for ContractParseError {}

// ---------------------------------------------------------------------------
// Credentials payload
// ---------------------------------------------------------------------------

/// Build the CREDENTIALS (code 0) message payload.
///
/// # Wire format (from `FPSSClient.sendCredentials()`)
///
/// ```text
/// [0x00] [username_len: u16 BE] [username bytes] [password bytes]
/// ```
///
/// The leading 0x00 byte is a version/flag byte present in the Java source.
/// `username_len` is the byte-length of the username (email), as a big-endian u16.
/// Password bytes follow immediately with no length prefix — the server infers
/// password length from `payload_len - 3 - username_len`.
pub fn build_credentials_payload(username: &str, password: &str) -> Vec<u8> {
    let user_bytes = username.as_bytes();
    let pass_bytes = password.as_bytes();

    // Match Java's `putShort((byte)len)` behavior: the length is first narrowed
    // to a byte (i8), then sign-extended to a short (i16). For lengths 0-127
    // this is identical to a plain u16 cast. For lengths 128-255 the sign
    // extension sets the high byte to 0xFF. In practice usernames are always
    // <128 bytes, but we match the exact wire encoding for correctness.
    let user_len = user_bytes.len() as i8 as i16;

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
/// # Wire format (from `PacketStream.addQuote()` / `PacketStream.addTrade()`)
///
/// ```text
/// [req_id: i32 BE] [contract bytes]
/// ```
///
/// The message code (21=QUOTE, 22=TRADE, 23=OPEN_INTEREST) is set by the caller
/// in the frame header; this function only builds the payload.
pub fn build_subscribe_payload(req_id: i32, contract: &Contract) -> Vec<u8> {
    let contract_bytes = contract.to_bytes();
    let mut buf = Vec::with_capacity(4 + contract_bytes.len());
    buf.extend_from_slice(&req_id.to_be_bytes());
    buf.extend_from_slice(&contract_bytes);
    buf
}

/// Build a full-type subscription payload (subscribe to all contracts of a security type).
///
/// # Wire format (from `PacketStream.java`)
///
/// ```text
/// [req_id: i32 BE] [sec_type: u8]
/// ```
///
/// Total 5 bytes. The server uses the 5-byte length to distinguish this from
/// a per-contract subscription (which is always longer).
pub fn build_full_type_subscribe_payload(req_id: i32, sec_type: SecType) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5);
    buf.extend_from_slice(&req_id.to_be_bytes());
    buf.push(sec_type as u8);
    buf
}

/// Build the PING (code 10) payload.
///
/// Source: `FPSSClient.java` — heartbeat sends 1-byte zero payload every 100ms.
pub fn build_ping_payload() -> Vec<u8> {
    vec![0x00]
}

/// Build the STOP (code 32) payload sent by the client on shutdown.
///
/// Source: `FPSSClient.java` — `sendStop()` sends empty-ish STOP message.
pub fn build_stop_payload() -> Vec<u8> {
    vec![0x00]
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Parse a REQ_RESPONSE (code 40) payload.
///
/// # Wire format (from `FPSSClient.onReqResponse()`)
///
/// ```text
/// [req_id: i32 BE] [resp_code: i32 BE]
/// ```
///
/// Returns `(req_id, response_type)`.
pub fn parse_req_response(
    payload: &[u8],
) -> Result<(i32, StreamResponseType), crate::error::Error> {
    if payload.len() < 8 {
        return Err(crate::error::Error::FpssProtocol(format!(
            "REQ_RESPONSE payload too short: {} bytes, expected 8",
            payload.len()
        )));
    }

    let req_id = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let resp_code = i32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);

    let resp_type = match resp_code {
        0 => StreamResponseType::Subscribed,
        1 => StreamResponseType::Error,
        2 => StreamResponseType::MaxStreamsReached,
        3 => StreamResponseType::InvalidPerms,
        _ => {
            return Err(crate::error::Error::FpssProtocol(format!(
                "unknown REQ_RESPONSE code: {resp_code}"
            )));
        }
    };

    Ok((req_id, resp_type))
}

/// Parse a DISCONNECTED (code 12) payload.
///
/// # Wire format (from `FPSSClient.java`)
///
/// ```text
/// [reason: i16 BE]
/// ```
///
/// 2-byte big-endian `RemoveReason` code.
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
/// # Wire format (from `FPSSClient.onContract()`)
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
pub fn parse_contract_message(payload: &[u8]) -> Result<(i32, Contract), crate::error::Error> {
    if payload.len() < 5 {
        return Err(crate::error::Error::FpssProtocol(format!(
            "CONTRACT payload too short: {} bytes",
            payload.len()
        )));
    }

    let contract_id = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let (contract, _consumed) = Contract::from_bytes(&payload[4..])
        .map_err(|e| crate::error::Error::FpssProtocol(format!("failed to parse contract: {e}")))?;

    Ok((contract_id, contract))
}

// ---------------------------------------------------------------------------
// Which message code to use for subscribe/unsubscribe
// ---------------------------------------------------------------------------

/// Returns the `StreamMsgType` code for subscribing to a given data type.
///
/// Source: `PacketStream.addQuote()` uses code 21, `addTrade()` uses 22,
/// `addOpenInterest()` uses 23.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionKind {
    Quote,
    Trade,
    OpenInterest,
}

impl SubscriptionKind {
    /// Message code for subscribing (Client->Server).
    pub fn subscribe_code(self) -> StreamMsgType {
        match self {
            Self::Quote => StreamMsgType::Quote,
            Self::Trade => StreamMsgType::Trade,
            Self::OpenInterest => StreamMsgType::OpenInterest,
        }
    }

    /// Message code for unsubscribing (Client->Server).
    pub fn unsubscribe_code(self) -> StreamMsgType {
        match self {
            Self::Quote => StreamMsgType::RemoveQuote,
            Self::Trade => StreamMsgType::RemoveTrade,
            Self::OpenInterest => StreamMsgType::RemoveOpenInterest,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_contract_roundtrip() {
        let c = Contract::stock("AAPL");
        let bytes = c.to_bytes();
        // Java: 3 + root.length() = 3 + 4 = 7 total bytes, size byte = 7
        assert_eq!(bytes.len(), 7);
        assert_eq!(bytes[0], 7); // total_size includes itself (Java: `3 + root.length()`)

        let (parsed, consumed) = Contract::from_bytes(&bytes).unwrap();
        assert_eq!(consumed, 7);
        assert_eq!(parsed, c);
    }

    #[test]
    fn option_contract_roundtrip() {
        let c = Contract::option("SPY", 20261218, true, 60000);
        let bytes = c.to_bytes();
        // Java: 12 + root.length() = 12 + 3 = 15 total bytes, size byte = 15
        assert_eq!(bytes.len(), 15);
        assert_eq!(bytes[0], 15); // total_size includes itself (Java: `12 + root.length()`)

        let (parsed, consumed) = Contract::from_bytes(&bytes).unwrap();
        assert_eq!(consumed, 15);
        assert_eq!(parsed, c);
        assert_eq!(parsed.exp_date, Some(20261218));
        assert_eq!(parsed.is_call, Some(true));
        assert_eq!(parsed.strike, Some(60000));
    }

    #[test]
    fn index_contract_roundtrip() {
        let c = Contract::index("SPX");
        let bytes = c.to_bytes();
        let (parsed, _) = Contract::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.root, "SPX");
        assert_eq!(parsed.sec_type, SecType::Index);
    }

    #[test]
    fn contract_from_bytes_too_short() {
        let err = Contract::from_bytes(&[]).unwrap_err();
        assert_eq!(err, ContractParseError::TooShort);
    }

    #[test]
    fn contract_from_bytes_invalid_size() {
        // total_size = 2, but minimum valid is 3 (size + root_len + sec_type with root_len=0)
        let err = Contract::from_bytes(&[2, 0]).unwrap_err();
        assert_eq!(err, ContractParseError::InvalidSize(2));
    }

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
        let payload = build_subscribe_payload(42, &contract);
        // req_id(4) + contract(1+1+4+1 = 7) = 11
        assert_eq!(payload.len(), 11);
        let req_id = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
        assert_eq!(req_id, 42);
        // Rest is the contract bytes
        let (parsed, _) = Contract::from_bytes(&payload[4..]).unwrap();
        assert_eq!(parsed, contract);
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
    fn contract_display_stock() {
        assert_eq!(Contract::stock("AAPL").to_string(), "AAPL STOCK");
    }

    #[test]
    fn contract_display_option() {
        let c = Contract::option("SPY", 20261218, false, 45000);
        assert_eq!(c.to_string(), "SPY OPTION 20261218 P 45000");
    }

    #[test]
    fn ping_payload() {
        let p = build_ping_payload();
        assert_eq!(p, vec![0x00]);
    }

    #[test]
    fn subscription_kind_codes() {
        assert_eq!(
            SubscriptionKind::Quote.subscribe_code(),
            StreamMsgType::Quote
        );
        assert_eq!(
            SubscriptionKind::Quote.unsubscribe_code(),
            StreamMsgType::RemoveQuote
        );
        assert_eq!(
            SubscriptionKind::Trade.subscribe_code(),
            StreamMsgType::Trade
        );
        assert_eq!(
            SubscriptionKind::Trade.unsubscribe_code(),
            StreamMsgType::RemoveTrade
        );
        assert_eq!(
            SubscriptionKind::OpenInterest.subscribe_code(),
            StreamMsgType::OpenInterest
        );
        assert_eq!(
            SubscriptionKind::OpenInterest.unsubscribe_code(),
            StreamMsgType::RemoveOpenInterest
        );
    }

    // -- Java wire-format parity tests -----------------------------------------
    // These verify byte-for-byte compatibility with Java's Contract.toBytes().

    #[test]
    fn java_parity_stock_aapl() {
        // Java: root="AAPL" (4 bytes), sec=STOCK
        // Java allocates: 3 + 4 = 7 bytes
        // Wire: [7, 4, 'A', 'A', 'P', 'L', sec_type_code]
        let c = Contract::stock("AAPL");
        let bytes = c.to_bytes();
        assert_eq!(bytes[0], 7); // size byte = 3 + root.length()
        assert_eq!(bytes[1], 4); // root_len
        assert_eq!(&bytes[2..6], b"AAPL");
        assert_eq!(bytes[6], SecType::Stock as u8);
        assert_eq!(bytes.len(), 7);
    }

    #[test]
    fn java_parity_option_spy() {
        // Java: root="SPY" (3 bytes), sec=OPTION, exp=20261218, isCall=true, strike=60000
        // Java allocates: 12 + 3 = 15 bytes
        // Wire: [15, 3, 'S','P','Y', sec_type, exp(4), is_call(1), strike(4)]
        let c = Contract::option("SPY", 20261218, true, 60000);
        let bytes = c.to_bytes();
        assert_eq!(bytes[0], 15); // size byte = 12 + root.length()
        assert_eq!(bytes[1], 3); // root_len
        assert_eq!(&bytes[2..5], b"SPY");
        assert_eq!(bytes[5], SecType::Option as u8);
        // exp_date = 20261218 big-endian
        assert_eq!(&bytes[6..10], &20261218i32.to_be_bytes());
        assert_eq!(bytes[10], 1); // is_call = true
                                  // strike = 60000 big-endian
        assert_eq!(&bytes[11..15], &60000i32.to_be_bytes());
    }

    #[test]
    fn java_parity_index_spx() {
        // Java: root="SPX" (3 bytes), sec=INDEX
        // Java allocates: 3 + 3 = 6 bytes
        let c = Contract::index("SPX");
        let bytes = c.to_bytes();
        assert_eq!(bytes[0], 6);
        assert_eq!(bytes[1], 3);
        assert_eq!(&bytes[2..5], b"SPX");
        assert_eq!(bytes[5], SecType::Index as u8);
        assert_eq!(bytes.len(), 6);
    }

    #[test]
    fn java_parity_single_char_root() {
        // Edge case: root="A" (1 byte), sec=STOCK
        // Java allocates: 3 + 1 = 4 bytes
        let c = Contract::stock("A");
        let bytes = c.to_bytes();
        assert_eq!(bytes[0], 4);
        assert_eq!(bytes[1], 1);
        assert_eq!(bytes[2], b'A');
        assert_eq!(bytes.len(), 4);
    }
}
