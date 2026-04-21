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

use tdbe::types::enums::{RemoveReason, SecType, StreamMsgType, StreamResponseType};

use crate::error::Error;

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
    /// `ThetaData`'s integer strike representation.
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
    /// # Arguments
    /// - `root`: Underlying ticker (e.g., `"AAPL"`)
    /// - `exp_date`: Expiration as `"YYYYMMDD"` (e.g., `"20260320"`)
    /// - `strike`: Strike price in dollars as string (e.g., `"550"`)
    /// - `right`: option right — accepts `"call"`/`"put"`/`"C"`/`"P"`
    ///   (case-insensitive). FPSS per-contract subscriptions cannot carry
    ///   the `both` / `*` wildcard, so those values are rejected.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if `exp_date` is not a valid integer date,
    /// if `right` cannot be parsed to a single side, if `strike` is not a
    /// valid f64, or if `strike * 1000` would overflow `i32`.
    pub fn option(
        root: impl Into<String>,
        exp_date: &str,
        strike: &str,
        right: &str,
    ) -> Result<Self, Error> {
        let exp: i32 = exp_date
            .replace('-', "")
            .parse()
            .map_err(|e| Error::Config(format!("invalid expiration date {exp_date:?}: {e}")))?;
        let is_call = crate::right::parse_right_strict(right)?
            .as_is_call()
            .ok_or_else(|| {
                Error::Config("parse_right_strict returned Both despite strict mode".to_string())
            })?;
        let strike_dollars: f64 = strike
            .parse()
            .map_err(|e| Error::Config(format!("invalid strike price {strike:?}: {e}")))?;
        let strike_scaled = (strike_dollars * 1000.0).round();
        if !strike_scaled.is_finite()
            || strike_scaled < f64::from(i32::MIN)
            || strike_scaled > f64::from(i32::MAX)
        {
            return Err(Error::Config(format!(
                "strike {strike_dollars} out of i32 range after *1000 scaling"
            )));
        }
        // Reason: bounds checked above.
        #[allow(clippy::cast_possible_truncation)]
        let strike_raw = strike_scaled as i32;
        Ok(Self {
            root: root.into(),
            sec_type: SecType::Option,
            exp_date: Some(exp),
            is_call: Some(is_call),
            strike: Some(strike_raw),
        })
    }

    /// Construct from raw wire-format values (integer expiration, bool call/put, raw strike).
    ///
    /// Prefer [`Contract::option`] for user-facing code. This constructor is for the
    /// drop-in REST/WS server which must match the Java terminal's contract format.
    pub fn option_raw(root: impl Into<String>, exp_date: i32, is_call: bool, strike: i32) -> Self {
        Self {
            root: root.into(),
            sec_type: SecType::Option,
            exp_date: Some(exp_date),
            is_call: Some(is_call),
            strike: Some(strike),
        }
    }

    /// OCC-21 option identifier length (6 root + 6 YYMMDD + 1 side + 8 strike).
    const OCC21_LEN: usize = 21;

    /// Validate that a candidate root ticker is 1..=6 ASCII uppercase letters
    /// optionally containing a single `.` (e.g. `"BRK.B"`). Multiple dots
    /// are rejected — industry-practice single-dot compound tickers like
    /// `"BRK.A"` / `"BRK.B"` / `"RDS.A"` never carry more than one dot,
    /// and allowing `"A..B"` would let callers sneak past the shape check
    /// with unconventional symbols that fail downstream exchange lookups.
    /// Returns an `Error::Config` with the offending input on failure.
    fn validate_root(input: &str, root: &str) -> Result<(), Error> {
        if root.is_empty() || root.len() > 6 {
            return Err(Error::Config(format!(
                "Contract::from_str: root must be 1..=6 chars, got {} chars in {input:?}",
                root.len()
            )));
        }
        let mut dot_count = 0usize;
        for ch in root.chars() {
            if ch == '.' {
                dot_count += 1;
                if dot_count > 1 {
                    return Err(Error::Config(format!(
                        "Contract::from_str: root must contain at most one '.', got {root:?} in {input:?}"
                    )));
                }
            } else if !ch.is_ascii_uppercase() {
                return Err(Error::Config(format!(
                    "Contract::from_str: root must be ASCII A-Z (or '.'), got {ch:?} in {input:?}",
                )));
            }
        }
        Ok(())
    }

    /// Parse an OCC-21 option identifier (`"AAPL  260417C00550000"`).
    ///
    /// Layout (ASCII, exactly 21 bytes):
    /// - bytes `0..6`: root, right-padded with spaces.
    /// - bytes `6..12`: `YYMMDD` expiration; the returned `exp_date` is
    ///   `20000000 + YYMMDD` (e.g. `260417` -> `20260417`).
    /// - byte `12`: `'C'` or `'P'`.
    /// - bytes `13..21`: zero-padded strike in thousandths of a dollar.
    ///
    /// # Scope
    ///
    /// This parser is scoped to OCC symbols in the 2000-2099 range. Any
    /// two-digit YY maps to `2000 + YY`, so `"AAPL  990101C00100000"`
    /// parses to `exp_date = 20990101` (2099-01-01). The OCC roster
    /// re-uses YY codes every century; callers transitioning past 2099
    /// must introduce an explicit century argument. The live FPSS feed
    /// will never surface a pre-2000 OCC symbol (the market-data channel
    /// only ships live contracts), so no roll-over heuristic is
    /// encoded.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] naming the specific failure (wrong length,
    /// empty root, non-uppercase root character, bad YYMMDD digits, right
    /// outside `C`/`P`, bad strike digits) with the full offending input
    /// included.
    fn parse_occ21(input: &str) -> Result<Self, Error> {
        if input.len() != Self::OCC21_LEN {
            return Err(Error::Config(format!(
                "Contract::from_str: OCC-21 format must be exactly {} chars, got {} in {input:?}",
                Self::OCC21_LEN,
                input.len()
            )));
        }
        if !input.is_ascii() {
            return Err(Error::Config(format!(
                "Contract::from_str: OCC-21 identifier must be ASCII, got {input:?}"
            )));
        }
        let bytes = input.as_bytes();
        // Root: bytes 0..6, right-padded with spaces.
        let root_raw = &input[0..6];
        let root = root_raw.trim_end_matches(' ').to_string();
        Self::validate_root(input, &root)?;

        // YYMMDD -> integer, then centuried to YYYYMMDD.
        let yymmdd_raw = &input[6..12];
        let yymmdd: i32 = yymmdd_raw.parse().map_err(|e| {
            Error::Config(format!(
                "Contract::from_str: expiration YYMMDD not numeric ({yymmdd_raw:?}, {e}) in {input:?}"
            ))
        })?;
        if !(0..1_000_000).contains(&yymmdd) {
            return Err(Error::Config(format!(
                "Contract::from_str: expiration YYMMDD out of range ({yymmdd}) in {input:?}"
            )));
        }
        // Scope: 2000-2099. See the doc-comment `# Scope` section.
        // Any two-digit YY is interpreted as `2000 + YY`, so `99`
        // maps to 2099 rather than 1999. The FPSS live feed only
        // ships contracts with future expirations, so pre-2000 OCC
        // symbols cannot reach this parser over the wire.
        let exp_date: i32 = 20_000_000 + yymmdd;

        // Right byte.
        let right_byte = bytes[12];
        let is_call = match right_byte {
            b'C' => true,
            b'P' => false,
            other => {
                return Err(Error::Config(format!(
                    "Contract::from_str: expected 'C' or 'P' at position 12, got {:?} in {input:?}",
                    other as char
                )));
            }
        };

        // Strike: thousandths of a dollar, zero-padded integer of 8 digits.
        let strike_raw = &input[13..21];
        for ch in strike_raw.chars() {
            if !ch.is_ascii_digit() {
                return Err(Error::Config(format!(
                    "Contract::from_str: strike must be 8 ASCII digits, got {strike_raw:?} in {input:?}"
                )));
            }
        }
        let strike: i32 = strike_raw.parse().map_err(|e| {
            Error::Config(format!(
                "Contract::from_str: strike not numeric ({strike_raw:?}, {e}) in {input:?}"
            ))
        })?;

        Ok(Self {
            root,
            sec_type: SecType::Option,
            exp_date: Some(exp_date),
            is_call: Some(is_call),
            strike: Some(strike),
        })
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
    ///   - Stock: `3 + root.length()` = size(1) + `root_len(1)` + root(N) + `sec_type(1)`
    ///   - Option: `12 + root.length()` = size(1) + `root_len(1)` + root(N) + `sec_type(1)` + exp(4) + `is_call(1)` + strike(4)
    ///
    /// Java's `fromBytes()` validates `len == size`, confirming the size byte
    /// counts itself.
    /// # Panics
    ///
    /// Panics if the contract root symbol exceeds 16 bytes (the maximum
    /// length accepted by Java's `Contract.toBytes()`).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let root_bytes = self.root.as_bytes();
        assert!(
            root_bytes.len() <= 16,
            "contract root too long: {} bytes (max 16 to match Java Contract.toBytes())",
            root_bytes.len()
        );
        let root_len = u8::try_from(root_bytes.len()).expect("root length validated <= 16");

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
        // Max total_size = 12 + 16 = 28, safely fits u8.
        buf.push(u8::try_from(total_size).expect("total_size <= 28"));
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
            buf.push(u8::from(self.is_call.unwrap_or(false)));
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
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
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
        let sec_type = SecType::from_code(i32::from(sec_type_byte))
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

impl std::str::FromStr for Contract {
    type Err = Error;

    /// Parse a [`Contract`] from a string.
    ///
    /// Two formats are accepted:
    ///
    /// 1. **Bare root** (stock contract). 1..=6 ASCII uppercase letters,
    ///    optionally including a single `.`:
    ///    ```
    ///    # use std::str::FromStr;
    ///    # use thetadatadx::fpss::protocol::Contract;
    ///    let c = "AAPL".parse::<Contract>().unwrap();
    ///    assert_eq!(c.root, "AAPL");
    ///    ```
    /// 2. **OCC-21 option identifier**. 21 ASCII characters:
    ///    `[root (6, space-padded)] [YYMMDD (6)] [C|P (1)] [strike (8, 1/1000$)]`.
    ///    Both 21-char (canonical) and 20-char (one space inside the
    ///    root-padding region) inputs are accepted. The 20-char form is
    ///    repaired by peeling off the fixed 15-char `YYMMDD+right+strike`
    ///    suffix and right-padding the leading root slice to 6 chars —
    ///    byte-for-byte equivalent to the 21-char form for the same
    ///    contract, matching the tolerant upstream feed that occasionally
    ///    ships the 20-char variant.
    ///    ```
    ///    # use std::str::FromStr;
    ///    # use thetadatadx::fpss::protocol::Contract;
    ///    let c = "SPY   260417C00550000".parse::<Contract>().unwrap();
    ///    assert_eq!(c.root, "SPY");
    ///    assert_eq!(c.exp_date, Some(20_260_417));
    ///    assert_eq!(c.is_call, Some(true));
    ///    assert_eq!(c.strike, Some(550_000));
    ///    ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] whose message names the specific failure
    /// (wrong length, non-ASCII, bad YYMMDD digits, right outside `C`/`P`,
    /// bad strike digits, empty root, non-uppercase root character) and
    /// includes the original input.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        // Strategy: the two formats are unambiguous by length after trim.
        // `"AAPL"` trimmed is 4 chars; `"SPY   260417C00550000"` trimmed
        // is 21 chars. A middle-length string is garbage — we still
        // route through the bare-root validator so the error message
        // names the exact constraint that failed.
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(Error::Config(format!(
                "Contract::from_str: empty or whitespace-only input in {input:?}"
            )));
        }
        if trimmed.len() == Self::OCC21_LEN {
            return Self::parse_occ21(trimmed);
        }
        // 20-char tolerance: some upstream sources drop a single space
        // INSIDE the root-padding region (e.g. `"SPY 260417C00550000"`
        // has "SPY" + one pad space + 15-char suffix = 20 chars). The
        // OCC-21 layout is [6-char root][15-char YYMMDD+right+strike],
        // so repair by peeling the trailing 15 chars as the fixed
        // suffix and right-padding the leading root slice to 6 chars.
        //
        // This is strictly a repair heuristic. The strict 21-char
        // parser runs on the repaired string, so all other constraints
        // (digits, right byte, strike width) still surface their exact
        // errors with the repaired input in the message.
        const OCC21_SUFFIX_LEN: usize = 15; // YYMMDD(6) + right(1) + strike(8)
        if trimmed.len() == Self::OCC21_LEN - 1 && trimmed.len() > OCC21_SUFFIX_LEN {
            let split = trimmed.len() - OCC21_SUFFIX_LEN;
            let root_slice = trimmed[..split].trim_end_matches(' ');
            let suffix = &trimmed[split..];
            if !root_slice.is_empty() && root_slice.len() <= 6 {
                // Rebuild with the root re-padded to 6 chars, then the
                // 15-char suffix — byte-for-byte equivalent to a
                // correctly-formatted OCC-21 string for the same
                // contract.
                let mut repaired = String::with_capacity(Self::OCC21_LEN);
                repaired.push_str(root_slice);
                for _ in root_slice.len()..6 {
                    repaired.push(' ');
                }
                repaired.push_str(suffix);
                if repaired.len() == Self::OCC21_LEN {
                    return Self::parse_occ21(&repaired);
                }
            }
        }

        // Bare root fallback.
        Self::validate_root(input, trimmed)?;
        Ok(Self::stock(trimmed))
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
#[must_use]
pub fn build_credentials_payload(username: &str, password: &str) -> Vec<u8> {
    let user_bytes = username.as_bytes();
    let pass_bytes = password.as_bytes();

    // Match Java's `putShort((byte)len)` behavior: the length is first narrowed
    // to a byte (i8), then sign-extended to a short (i16). For lengths 0-127
    // this is identical to a plain u16 cast. For lengths 128-255 the sign
    // extension sets the high byte to 0xFF. In practice usernames are always
    // <128 bytes, but we match the exact wire encoding for correctness.
    // Truncation to i8 is intentional: matches Java putShort((byte)len) wire encoding.
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
/// # Wire format (from `PacketStream.addQuote()` / `PacketStream.addTrade()`)
///
/// ```text
/// [req_id: i32 BE] [contract bytes]
/// ```
///
/// The message code (21=QUOTE, 22=TRADE, `23=OPEN_INTEREST`) is set by the caller
/// in the frame header; this function only builds the payload.
#[must_use]
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
#[must_use]
pub fn build_full_type_subscribe_payload(req_id: i32, sec_type: SecType) -> Vec<u8> {
    let mut buf = Vec::with_capacity(5);
    buf.extend_from_slice(&req_id.to_be_bytes());
    buf.push(sec_type as u8);
    buf
}

/// Build the PING (code 10) payload.
///
/// Source: `FPSSClient.java` — heartbeat sends 1-byte zero payload every 100ms.
#[must_use]
pub fn build_ping_payload() -> Vec<u8> {
    vec![0x00]
}

/// Build the STOP (code 32) payload sent by the client on shutdown.
///
/// Source: `FPSSClient.java` — `sendStop()` sends empty-ish STOP message.
#[must_use]
pub fn build_stop_payload() -> Vec<u8> {
    vec![0x00]
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Parse a `REQ_RESPONSE` (code 40) payload.
///
/// # Wire format (from `FPSSClient.onReqResponse()`)
///
/// ```text
/// [req_id: i32 BE] [resp_code: i32 BE]
/// ```
///
/// Returns `(req_id, response_type)`.
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn parse_req_response(
    payload: &[u8],
) -> Result<(i32, StreamResponseType), crate::error::Error> {
    if payload.len() < 8 {
        return Err(crate::error::Error::Fpss {
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
            return Err(crate::error::Error::Fpss {
                kind: crate::error::FpssErrorKind::ProtocolError,
                message: format!("unknown REQ_RESPONSE code: {resp_code}"),
            });
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
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn parse_contract_message(payload: &[u8]) -> Result<(i32, Contract), crate::error::Error> {
    if payload.len() < 5 {
        return Err(crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ProtocolError,
            message: format!("CONTRACT payload too short: {} bytes", payload.len()),
        });
    }

    let contract_id = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let (contract, _consumed) =
        Contract::from_bytes(&payload[4..]).map_err(|e| crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ProtocolError,
            message: format!("failed to parse contract: {e}"),
        })?;

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
    #[must_use]
    pub fn subscribe_code(self) -> StreamMsgType {
        match self {
            Self::Quote => StreamMsgType::Quote,
            Self::Trade => StreamMsgType::Trade,
            Self::OpenInterest => StreamMsgType::OpenInterest,
        }
    }

    /// Message code for unsubscribing (Client->Server).
    #[must_use]
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
        let c = Contract::option("SPY", "20261218", "60", "C").unwrap();
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
        let c = Contract::option("SPY", "20261218", "45", "P").unwrap();
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
        let c = Contract::option("SPY", "20261218", "60", "C").unwrap();
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
    fn option_rejects_invalid_strike() {
        // Garbage strike string -- must return Err, not panic.
        assert!(Contract::option("SPY", "20261218", "not-a-number", "C").is_err());
    }

    #[test]
    fn option_rejects_overflowing_strike() {
        // Strike * 1000 exceeds i32::MAX. Must return Err, not wrap silently.
        assert!(Contract::option("SPY", "20261218", "3000000", "C").is_err());
    }

    #[test]
    fn option_rejects_invalid_expiration() {
        // Non-numeric expiration -- must return Err, not panic.
        assert!(Contract::option("SPY", "not-a-date", "60", "C").is_err());
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

    // -- FromStr tests ---------------------------------------------------------

    #[test]
    fn from_str_bare_root_stock() {
        use std::str::FromStr;
        let c = Contract::from_str("AAPL").unwrap();
        assert_eq!(c.root, "AAPL");
        assert_eq!(c.sec_type, SecType::Stock);
        assert!(c.exp_date.is_none());
        assert!(c.is_call.is_none());
        assert!(c.strike.is_none());
    }

    #[test]
    fn from_str_bare_root_short_ticker() {
        use std::str::FromStr;
        let c = Contract::from_str("A").unwrap();
        assert_eq!(c.root, "A");
        assert_eq!(c.sec_type, SecType::Stock);
    }

    #[test]
    fn from_str_bare_root_with_dot() {
        use std::str::FromStr;
        // BRK.A style tickers must parse as stock roots.
        let c = Contract::from_str("BRK.A").unwrap();
        assert_eq!(c.root, "BRK.A");
        assert_eq!(c.sec_type, SecType::Stock);
    }

    #[test]
    fn from_str_bare_root_trims_surrounding_whitespace() {
        use std::str::FromStr;
        let c = Contract::from_str("  SPY  ").unwrap();
        assert_eq!(c.root, "SPY");
    }

    #[test]
    fn from_str_occ21_call() {
        use std::str::FromStr;
        // SPY  (4 chars -> 6 chars padded) 26-04-17 Call 550.00.
        let c = Contract::from_str("SPY   260417C00550000").unwrap();
        assert_eq!(c.root, "SPY");
        assert_eq!(c.sec_type, SecType::Option);
        assert_eq!(c.exp_date, Some(20_260_417));
        assert_eq!(c.is_call, Some(true));
        assert_eq!(c.strike, Some(550_000));
    }

    #[test]
    fn from_str_occ21_put() {
        use std::str::FromStr;
        // QQQ 26-06-20 Put 350.00.
        let c = Contract::from_str("QQQ   260620P00350000").unwrap();
        assert_eq!(c.root, "QQQ");
        assert_eq!(c.is_call, Some(false));
        assert_eq!(c.exp_date, Some(20_260_620));
        assert_eq!(c.strike, Some(350_000));
    }

    #[test]
    fn from_str_occ21_aapl_documented_example() {
        use std::str::FromStr;
        // The exact example from the spec.
        let c = Contract::from_str("AAPL  260417C00550000").unwrap();
        assert_eq!(c.root, "AAPL");
        assert_eq!(c.exp_date, Some(20_260_417));
        assert_eq!(c.is_call, Some(true));
        assert_eq!(c.strike, Some(550_000));
    }

    #[test]
    fn from_str_occ21_six_char_root() {
        use std::str::FromStr;
        // Full six-char root: no spaces in the root field.
        let c = Contract::from_str("ABCDEF260417C00550000").unwrap();
        assert_eq!(c.root, "ABCDEF");
        assert_eq!(c.exp_date, Some(20_260_417));
        assert_eq!(c.strike, Some(550_000));
    }

    #[test]
    fn from_str_occ21_malformed_strike() {
        use std::str::FromStr;
        // Strike contains non-digit characters.
        let err = Contract::from_str("SPY   260417C0055000X")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("strike"),
            "expected error to name strike failure, got: {err}"
        );
        assert!(
            err.contains("SPY   260417C0055000X"),
            "expected error to include the offending input, got: {err}"
        );
    }

    #[test]
    fn from_str_occ21_wrong_right_byte() {
        use std::str::FromStr;
        // Byte 12 is 'X', not 'C' or 'P'.
        let err = Contract::from_str("SPY   260417X00550000")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("'C' or 'P'"),
            "expected error to name the right constraint, got: {err}"
        );
    }

    #[test]
    fn from_str_occ21_bad_expiration_digits() {
        use std::str::FromStr;
        let err = Contract::from_str("SPY   2X0417C00550000")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("expiration"),
            "expected error to name expiration failure, got: {err}"
        );
    }

    #[test]
    fn from_str_wrong_length_garbage() {
        use std::str::FromStr;
        // 10 chars — neither a root (<=6) nor OCC-21 (21). The parser
        // pads to 21 and the strict parse fails at the right-byte slot.
        let err = Contract::from_str("APPLETREES").unwrap_err().to_string();
        // Any specific failure is fine; spec says the error must name a
        // specific failure, so assert it's not a generic pass-through.
        assert!(
            err.contains("Contract::from_str"),
            "expected Contract::from_str-prefixed error, got: {err}"
        );
        assert!(
            err.contains("APPLETREES"),
            "expected error to include the offending input, got: {err}"
        );
    }

    #[test]
    fn from_str_empty_input() {
        use std::str::FromStr;
        let err = Contract::from_str("").unwrap_err().to_string();
        assert!(
            err.contains("empty"),
            "expected error to mention empty input, got: {err}"
        );
    }

    #[test]
    fn from_str_lowercase_root_rejected() {
        use std::str::FromStr;
        let err = Contract::from_str("aapl").unwrap_err().to_string();
        assert!(
            err.contains("ASCII A-Z"),
            "expected error to describe root charset, got: {err}"
        );
    }

    #[test]
    fn from_str_root_too_long() {
        use std::str::FromStr;
        // 7-char non-space root: not a valid bare root and not OCC-21 length.
        let err = Contract::from_str("ABCDEFG").unwrap_err().to_string();
        assert!(
            err.contains("Contract::from_str"),
            "expected Contract::from_str-prefixed error, got: {err}"
        );
    }

    // -- OCC-21 20-char tolerance -------------------------------------------
    //
    // Upstream feeds occasionally ship OCC symbols one space short of the
    // canonical 21 bytes because the missing space sits INSIDE the root
    // padding region. The repair MUST peel the fixed 15-char suffix, then
    // re-pad the leading root slice to 6 chars so the strict 21-char
    // parser sees the exact same bytes it would have on a correctly
    // formatted input.

    #[test]
    fn from_str_occ21_20char_single_space_padded_root() {
        use std::str::FromStr;
        // 20-char form: "SPY" + two pad spaces (one short) + 15-char
        // suffix. Canonical OCC-21 has three pad spaces after SPY.
        let twenty = "SPY  260417C00550000";
        assert_eq!(twenty.len(), 20);
        let c20 = Contract::from_str(twenty).expect("20-char OCC-21 must repair");

        // 21-char canonical: "SPY" + 3 pad spaces + 15-char suffix.
        let twentyone = "SPY   260417C00550000";
        assert_eq!(twentyone.len(), 21);
        let c21 = Contract::from_str(twentyone).expect("21-char OCC-21 must parse");

        // Both strings MUST produce the same Contract. This pins the
        // repair behaviour — previously the 20-char form was padded
        // with a trailing space, which shifted the right-byte into a
        // digit slot and either errored or decoded a different contract.
        assert_eq!(c20, c21, "20-char and 21-char forms must parse identically");
        assert_eq!(c20.root, "SPY");
        assert_eq!(c20.exp_date, Some(20_260_417));
        assert_eq!(c20.is_call, Some(true));
        assert_eq!(c20.strike, Some(550_000));
    }

    #[test]
    fn from_str_occ21_20char_two_char_root() {
        use std::str::FromStr;
        // Root "T" (1 char) + 4 pad spaces (5-char root field) + suffix
        // = 20 chars. Canonical has 5 pad spaces after "T".
        let twenty = "T    260417C00150000";
        assert_eq!(twenty.len(), 20);
        let c = Contract::from_str(twenty).expect("20-char OCC-21 with short root must repair");
        assert_eq!(c.root, "T");
        assert_eq!(c.is_call, Some(true));
        assert_eq!(c.strike, Some(150_000));
    }

    // -- Century scope (2000-2099) -------------------------------------------

    #[test]
    fn from_str_occ21_yy_99_maps_to_2099() {
        use std::str::FromStr;
        // YY=99 maps to 2099 under the documented 2000-2099 scope.
        // Any post-2099 expansion MUST add an explicit century argument;
        // the live FPSS feed ships only live contracts so a pre-2000
        // OCC symbol cannot reach this parser over the wire.
        let c = Contract::from_str("AAPL  990101C00100000").expect("YY=99 must parse");
        assert_eq!(c.exp_date, Some(20_990_101), "YY=99 must map to 2099-01-01");
        assert_eq!(c.root, "AAPL");
        assert_eq!(c.is_call, Some(true));
        assert_eq!(c.strike, Some(100_000));
    }

    // -- validate_root: at most one dot --------------------------------------

    #[test]
    fn from_str_bare_root_rejects_multiple_dots() {
        use std::str::FromStr;
        // "A..B" has two dots and must be rejected — no real ticker
        // has more than one dot in its root.
        let err = Contract::from_str("A..B").unwrap_err().to_string();
        assert!(
            err.contains("at most one '.'"),
            "expected single-dot error, got: {err}"
        );
        assert!(
            err.contains("A..B"),
            "expected error to include the offending input, got: {err}"
        );
    }

    #[test]
    fn from_str_bare_root_allows_exactly_one_dot() {
        use std::str::FromStr;
        // Industry-practice single-dot compound tickers MUST still
        // parse — keeping BRK.A / BRK.B / RDS.A reachable.
        let c = Contract::from_str("BRK.B").expect("single-dot root must parse");
        assert_eq!(c.root, "BRK.B");
    }
}
