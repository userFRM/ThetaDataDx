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
use crate::fpss::framing::MAX_PAYLOAD_LEN;
use zeroize::Zeroizing;

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
///
/// # Errors
///
/// Returns [`Error::Config`] (`InvalidValue`) if the username is 128 bytes or
/// longer: the on-wire length field is a single byte widened to a short, so it
/// can only faithfully represent 0..=127 bytes, and a larger length would emit
/// a frame whose length field disagrees with the bytes that follow.
///
/// Also returns [`Error::Config`] (`InvalidValue`) if the assembled payload
/// would exceed the [`MAX_PAYLOAD_LEN`] (255-byte) single-frame limit. The
/// payload length is `3 + username.len() + password.len()`; oversized
/// credentials are rejected here so the caller gets a typed configuration
/// error instead of a frame-construction panic.
///
/// # Secret handling
///
/// The returned buffer holds the cleartext password and is wrapped in
/// [`zeroize::Zeroizing`] so its backing allocation is wiped on drop. This
/// keeps the credentials buffer on the same zeroize discipline as the rest of
/// the auth path ([`crate::auth`] holds the password in `Zeroizing`), so no
/// transient between the credentials struct and the socket leaves cleartext in
/// freed heap memory.
pub fn build_credentials_payload(
    username: &str,
    password: &str,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let user_bytes = username.as_bytes();
    let pass_bytes = password.as_bytes();

    // 1 (version) + 2 (user_len) + username + password must fit the single
    // length byte on the wire. Reject oversized credentials up front so the
    // connect path returns a typed error rather than panicking inside
    // `Frame::new`.
    let payload_len = 3 + user_bytes.len() + pass_bytes.len();
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(Error::config_invalid(
            "auth.credentials",
            format!(
                "credentials payload is {payload_len} bytes, exceeding the FPSS \
                 {MAX_PAYLOAD_LEN}-byte frame limit (username {} bytes + password \
                 {} bytes + 3-byte header); shorten the email or password",
                user_bytes.len(),
                pass_bytes.len()
            ),
        ));
    }

    // The username length travels the wire as a single byte that is then
    // widened to a short, so it can only faithfully represent lengths
    // 0..=127. A length of 128 or more would set the sign bit and widen to a
    // 0xFF.. short, producing a length field that disagrees with the bytes
    // that follow. Real usernames (emails) are far under this bound, so we
    // fail closed on the unrepresentable range rather than emit a frame whose
    // length field is wrong.
    if user_bytes.len() >= 128 {
        return Err(Error::config_invalid(
            "auth.credentials",
            format!(
                "username is {} bytes; the FPSS credentials length field is a \
                 single byte and can only represent 0..=127 bytes; shorten the \
                 email",
                user_bytes.len()
            ),
        ));
    }

    // With the 0..=127 invariant enforced above, the byte and short encodings
    // coincide: narrowing to i8 then widening to i16 yields the same value as
    // a plain u16 cast, so this reproduces the exact wire bytes.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let user_len = i16::from(user_bytes.len() as i8);

    // 1 (version) + 2 (user_len) + user + pass
    let mut buf = Vec::with_capacity(3 + user_bytes.len() + pass_bytes.len());
    buf.push(0x00); // version/flag byte
    buf.extend_from_slice(&user_len.to_be_bytes());
    buf.extend_from_slice(user_bytes);
    buf.extend_from_slice(pass_bytes);
    Ok(Zeroizing::new(buf))
}

/// Login-type byte for an API-key login in the API-key credentials
/// payload. The password login uses `0`; an API key uses `2`.
const LOGIN_TYPE_API_KEY: u8 = 2;

/// Build the API-key CREDENTIALS (code 0) message payload.
///
/// This is the API-key counterpart to [`build_credentials_payload`]. It
/// authenticates with an API key instead of a password and carries a
/// short build-identifier the SDK supplies for the login handshake.
///
/// # Wire format
///
/// ```text
/// [0x03] [commit_len: u8] [commit bytes] [login_type: u8]
/// [user_len: u16 BE] [user bytes] [credential bytes]
/// ```
///
/// - The leading `0x03` byte is the payload version.
/// - `commit_len` / `commit` carry a short build-identifier string the
///   SDK supplies (informational only).
/// - `login_type` is `2` for an API-key login.
/// - `user_len` is the byte-length of the user field (the account email,
///   or empty when none is available), as a big-endian u16.
/// - `credential` (the API key) follows immediately with no length
///   prefix — the server infers its length from the remaining bytes.
///
/// # Errors
///
/// Returns [`Error::Config`] (`InvalidValue`) if the user field is 128
/// bytes or longer (the same bound the password payload enforces), if the
/// commit string exceeds 255 bytes (the single-byte `commit_len` field),
/// or if the assembled payload would exceed the [`MAX_PAYLOAD_LEN`]
/// (255-byte) single-frame limit.
///
/// # Secret handling
///
/// The returned buffer holds the cleartext API key and is wrapped in
/// [`zeroize::Zeroizing`] so its backing allocation is wiped on drop,
/// matching the password payload's secret discipline.
pub fn build_apikey_credentials_payload(
    user: &str,
    api_key: &str,
    commit: &str,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let user_bytes = user.as_bytes();
    let key_bytes = api_key.as_bytes();
    let commit_bytes = commit.as_bytes();

    // The commit length travels the wire as a single byte, so it can only
    // represent 0..=255 bytes. The build identifier is short, so reject an
    // over-long value up front rather than emitting a length field that
    // disagrees with the bytes that follow.
    if commit_bytes.len() > u8::MAX as usize {
        return Err(Error::config_invalid(
            "auth.credentials",
            format!(
                "build identifier is {} bytes; the API-key credentials commit \
                 length field is a single byte and can only represent 0..=255 \
                 bytes",
                commit_bytes.len()
            ),
        ));
    }

    // The user-length field can faithfully represent 0..=127 bytes — the
    // same bound the password payload enforces. Real account emails are far
    // under this, so fail closed on the unrepresentable range.
    if user_bytes.len() >= 128 {
        return Err(Error::config_invalid(
            "auth.credentials",
            format!(
                "user field is {} bytes; the FPSS credentials length field can \
                 only represent 0..=127 bytes; shorten the email",
                user_bytes.len()
            ),
        ));
    }

    // 1 (version) + 1 (commit_len) + commit + 1 (login_type)
    // + 2 (user_len) + user + credential must fit the single-frame cap.
    let payload_len = 1 + 1 + commit_bytes.len() + 1 + 2 + user_bytes.len() + key_bytes.len();
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(Error::config_invalid(
            "auth.credentials",
            format!(
                "API-key credentials payload is {payload_len} bytes, exceeding the \
                 FPSS {MAX_PAYLOAD_LEN}-byte frame limit (build id {} bytes + user \
                 {} bytes + key {} bytes + 5-byte header); shorten the API key",
                commit_bytes.len(),
                user_bytes.len(),
                key_bytes.len()
            ),
        ));
    }

    let mut buf = Vec::with_capacity(payload_len);
    buf.push(0x03); // version byte
    #[allow(clippy::cast_possible_truncation)]
    buf.push(commit_bytes.len() as u8);
    buf.extend_from_slice(commit_bytes);
    buf.push(LOGIN_TYPE_API_KEY);
    #[allow(clippy::cast_possible_truncation)]
    buf.extend_from_slice(&(user_bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(user_bytes);
    buf.extend_from_slice(key_bytes);
    Ok(Zeroizing::new(buf))
}

/// Short build-identifier the SDK supplies in the API-key login
/// handshake. Informational only; it tags the login with the SDK build.
#[must_use]
pub fn login_build_id() -> String {
    format!("thetadatadx/{}", env!("CARGO_PKG_VERSION"))
}

/// Build the CREDENTIALS (code 0) login payload for a set of credentials,
/// selecting the wire format from the authentication method.
///
/// - Email + password credentials produce the legacy `[0x00 ...]` payload
///   via [`build_credentials_payload`] — byte-identical to the original
///   login frame.
/// - API-key credentials produce the `[0x03 ...]` payload via
///   [`build_apikey_credentials_payload`], carrying the account email (or
///   an empty user field when none is available) and the SDK build id.
///
/// # Errors
///
/// Propagates the length / frame-cap errors from whichever underlying
/// builder runs.
pub fn build_login_payload(creds: &crate::auth::Credentials) -> Result<Zeroizing<Vec<u8>>, Error> {
    if let Some(key) = creds.api_key_secret() {
        // For an API-key credential the user field carries the account email
        // when the credential was paired with one, and is empty (userLen 0)
        // otherwise. The streaming endpoint accepts the empty-user form for
        // key-only credentials.
        let user = creds.email().unwrap_or("");
        build_apikey_credentials_payload(user, key, &login_build_id())
    } else {
        // The email + password path is unchanged and byte-identical to the
        // original login frame. A password credential always carries both
        // fields; treat a missing one as empty rather than failing here.
        build_credentials_payload(creds.email().unwrap_or(""), creds.password().unwrap_or(""))
    }
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
            kind: crate::error::StreamErrorKind::ProtocolError,
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
                kind: crate::error::StreamErrorKind::ProtocolError,
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
            kind: crate::error::StreamErrorKind::ProtocolError,
            message: format!("CONTRACT payload too short: {} bytes", payload.len()),
        });
    }

    let contract_id = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let (contract, _consumed) = Contract::from_bytes(&payload[4..]).map_err(|e| Error::Fpss {
        kind: crate::error::StreamErrorKind::ProtocolError,
        message: format!("failed to parse contract: {e}"),
    })?;

    Ok((contract_id, contract))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_payload_format() {
        let payload = build_credentials_payload("user@test.com", "pass123").expect("valid creds");
        assert_eq!(payload[0], 0x00); // version byte
        let user_len = u16::from_be_bytes([payload[1], payload[2]]);
        assert_eq!(user_len, 13); // "user@test.com".len()
        assert_eq!(&payload[3..16], b"user@test.com");
        assert_eq!(&payload[16..], b"pass123");
    }

    /// Pin the EXACT bytes of the password (`0x00`) payload so any drift
    /// in the live-proven frame is caught. This frame format must never
    /// change: `[0x00][user_len: u16 BE][user][password]`.
    #[test]
    fn password_payload_exact_bytes_unchanged() {
        let payload = build_credentials_payload("a@b.co", "pw1").expect("valid creds");
        let expected: Vec<u8> = {
            let mut v = vec![0x00];
            v.extend_from_slice(&6u16.to_be_bytes()); // "a@b.co".len() == 6
            v.extend_from_slice(b"a@b.co");
            v.extend_from_slice(b"pw1");
            v
        };
        assert_eq!(
            &payload[..],
            &expected[..],
            "the password (0x00) payload bytes must never change"
        );
    }

    /// Pin the EXACT bytes of the API-key (`0x03`) payload:
    /// `[0x03][commit_len: u8][commit][login_type: u8][user_len: u16 BE][user][key]`
    /// with `login_type == 2`.
    #[test]
    fn apikey_payload_exact_bytes() {
        let payload = build_apikey_credentials_payload("a@b.co", "key123", "build/1")
            .expect("valid api-key creds");
        let expected: Vec<u8> = {
            let mut v = vec![0x03]; // version
            v.push(7); // commit_len == "build/1".len()
            v.extend_from_slice(b"build/1");
            v.push(2); // login_type == API key
            v.extend_from_slice(&6u16.to_be_bytes()); // user_len == "a@b.co".len()
            v.extend_from_slice(b"a@b.co");
            v.extend_from_slice(b"key123");
            v
        };
        assert_eq!(&payload[..], &expected[..]);
        // Spot-check the load-bearing framing bytes explicitly.
        assert_eq!(payload[0], 0x03, "version byte");
        assert_eq!(payload[1], 7, "commit length");
        assert_eq!(&payload[2..9], b"build/1", "commit bytes");
        assert_eq!(payload[9], 2, "login type must be 2 for api-key");
        let user_len = u16::from_be_bytes([payload[10], payload[11]]);
        assert_eq!(user_len, 6);
        assert_eq!(&payload[12..18], b"a@b.co");
        assert_eq!(&payload[18..], b"key123");
    }

    /// An API-key login with no account email emits `user_len == 0` and no
    /// user bytes.
    #[test]
    fn apikey_payload_empty_user() {
        let payload =
            build_apikey_credentials_payload("", "k", "c").expect("empty user must build");
        // [0x03][1][c][2][0x00 0x00][k]
        assert_eq!(payload[0], 0x03);
        assert_eq!(payload[1], 1);
        assert_eq!(&payload[2..3], b"c");
        assert_eq!(payload[3], 2);
        let user_len = u16::from_be_bytes([payload[4], payload[5]]);
        assert_eq!(user_len, 0, "empty user must encode user_len 0");
        assert_eq!(&payload[6..], b"k");
    }

    #[test]
    fn apikey_payload_rejects_128_byte_user() {
        let user = "u".repeat(128);
        let err = build_apikey_credentials_payload(&user, "k", "c")
            .expect_err("128-byte user must error");
        match err {
            Error::Config {
                kind: crate::error::ConfigErrorKind::InvalidValue { field, .. },
                message,
                ..
            } => {
                assert_eq!(field, "auth.credentials");
                assert!(
                    message.contains("127"),
                    "message names the bound: {message}"
                );
            }
            other => panic!("expected Error::Config InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn apikey_payload_rejects_oversize() {
        // Push the key long enough to blow the 255-byte cap.
        let key = "k".repeat(260);
        let err = build_apikey_credentials_payload("a@b.co", &key, "c")
            .expect_err("oversized api-key creds must error");
        match err {
            Error::Config {
                kind: crate::error::ConfigErrorKind::InvalidValue { field, .. },
                message,
                ..
            } => {
                assert_eq!(field, "auth.credentials");
                assert!(
                    message.contains("255"),
                    "message names the frame limit: {message}"
                );
            }
            other => panic!("expected Error::Config InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn credentials_payload_rejects_oversize() {
        // 3-byte header + username + password must fit 255 bytes. Push the
        // combined size just over the limit and expect a typed config error,
        // not a panic in `Frame::new`.
        let username = "a".repeat(200);
        let password = "b".repeat(60); // 3 + 200 + 60 = 263 > 255
        let err = build_credentials_payload(&username, &password)
            .expect_err("oversized creds must error");
        match err {
            Error::Config {
                kind: crate::error::ConfigErrorKind::InvalidValue { field, .. },
                message,
                ..
            } => {
                assert_eq!(field, "auth.credentials");
                assert!(
                    message.contains("255"),
                    "message should name the limit: {message}"
                );
                assert!(
                    message.contains("263"),
                    "message should name the size: {message}"
                );
            }
            other => panic!("expected Error::Config InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn credentials_payload_accepts_at_limit() {
        // Exactly 255 bytes total: 3-byte header + 100-byte username + 152-byte
        // password. A long-but-valid credential near the limit still builds.
        let username = "u".repeat(100);
        let password = "p".repeat(MAX_PAYLOAD_LEN - 3 - 100);
        let payload =
            build_credentials_payload(&username, &password).expect("at-limit creds must build");
        assert_eq!(payload.len(), MAX_PAYLOAD_LEN);
    }

    #[test]
    fn credentials_payload_accepts_127_byte_username() {
        // 127 is the largest username the single-byte length field can encode
        // without the sign bit flipping. The length byte must read back as the
        // true length, proving the encoding for the realistic (<128) range is
        // unchanged.
        let username = "u".repeat(127);
        let password = "p"; // 3 + 127 + 1 = 131 bytes, well under the limit
        let payload =
            build_credentials_payload(&username, password).expect("127-byte username must build");
        let user_len = u16::from_be_bytes([payload[1], payload[2]]);
        assert_eq!(user_len, 127);
        assert_eq!(&payload[3..130], username.as_bytes());
        assert_eq!(&payload[130..], b"p");
    }

    #[test]
    fn credentials_payload_rejects_128_byte_username() {
        // 128 bytes would set the sign bit of the single-byte length field and
        // widen to a 0xFF.. short, so it is rejected up front.
        let username = "u".repeat(128);
        let err =
            build_credentials_payload(&username, "p").expect_err("128-byte username must error");
        match err {
            Error::Config {
                kind: crate::error::ConfigErrorKind::InvalidValue { field, .. },
                message,
                ..
            } => {
                assert_eq!(field, "auth.credentials");
                assert!(
                    message.contains("127"),
                    "message should name the representable bound: {message}"
                );
                assert!(
                    message.contains("128"),
                    "message should name the offending length: {message}"
                );
            }
            other => panic!("expected Error::Config InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn credentials_payload_rejects_long_username() {
        // A clearly oversized username (200 bytes) is rejected on the same
        // single-byte length invariant, ahead of the total-payload check.
        let username = "u".repeat(200);
        let err =
            build_credentials_payload(&username, "p").expect_err("200-byte username must error");
        match err {
            Error::Config {
                kind: crate::error::ConfigErrorKind::InvalidValue { field, .. },
                message,
                ..
            } => {
                assert_eq!(field, "auth.credentials");
                assert!(
                    message.contains("200"),
                    "message should name the offending length: {message}"
                );
            }
            other => panic!("expected Error::Config InvalidValue, got {other:?}"),
        }
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
