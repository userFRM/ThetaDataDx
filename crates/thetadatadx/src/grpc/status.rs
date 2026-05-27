//! gRPC status, parsed from HTTP/2 trailers.
//!
//! Every gRPC response — successful or not — ends with HTTP/2 trailers
//! carrying at minimum a `grpc-status` numeric code and optionally a
//! `grpc-message` human string and a `grpc-status-details-bin` payload.
//! See <https://grpc.github.io/grpc/core/md_doc_statuscodes.html> for the
//! canonical status code list and <https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md>
//! for the trailers contract.
//!
//! The parser here turns a [`http::HeaderMap`] into a typed [`Status`].
//! A missing `grpc-status` is an error rather than a panic — some
//! pathological peers ship no trailers at all (the response body ends
//! mid-stream), and this layer is the place that gets to refuse instead
//! of fall through.

use http::{HeaderMap, HeaderValue};
use percent_encoding::percent_decode;
use thiserror::Error;

/// `grpc-status` trailer name.
pub(crate) const GRPC_STATUS: &str = "grpc-status";
/// `grpc-message` trailer name. Per the gRPC HTTP/2 spec the value is
/// RFC 3986 percent-encoded UTF-8; the parser percent-decodes and
/// gracefully tolerates malformed values rather than invalidating the
/// `grpc-status` it travels with. See [`decode_grpc_message`].
pub(crate) const GRPC_MESSAGE: &str = "grpc-message";
/// `grpc-status: 0` — the `Ok` code.
pub(crate) const STATUS_OK: u32 = 0;

/// gRPC status carried in HTTP/2 trailers.
///
/// Stored as the raw numeric `code` so callers match against the gRPC
/// canonical codes directly. The module deliberately avoids a typed
/// status-code enum to keep the dependency surface narrow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Numeric gRPC status code, e.g. `0` for `Ok`, `13` for `Internal`.
    code: u32,
    /// Human-readable status message decoded from `grpc-message` (may
    /// be empty when the trailer is absent or the status is `Ok`).
    message: String,
}

impl Status {
    /// Build a status with the given code and (possibly empty) message.
    #[must_use]
    pub fn new(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Numeric status code.
    #[must_use]
    pub const fn code(&self) -> u32 {
        self.code
    }

    /// Status message; empty on `Ok` or when the trailer is absent.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// `true` iff the status code is `0` (gRPC `Ok`).
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.code == STATUS_OK
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "grpc-status={}", self.code)
        } else {
            write!(f, "grpc-status={}: {}", self.code, self.message)
        }
    }
}

/// Errors raised by [`Status::from_trailers`].
///
/// `#[non_exhaustive]` so downstream `match` arms must include a
/// wildcard; new variants land without breaking semver.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum StatusParseError {
    /// `grpc-status` trailer was absent entirely. Per the gRPC HTTP/2
    /// spec every response must carry one; its absence is a wire-level
    /// protocol violation.
    #[error("response trailers missing required `grpc-status`")]
    Missing,
    /// `grpc-status` was present but not a valid UTF-8 string.
    #[error("`grpc-status` trailer is not valid UTF-8")]
    StatusNotUtf8,
    /// `grpc-status` was present and UTF-8 but not a base-10 integer.
    #[error("`grpc-status` trailer is not a base-10 integer: {value:?}")]
    StatusNotNumeric {
        /// The raw value as received.
        value: String,
    },
}

impl Status {
    /// Parse a gRPC status from a [`HeaderMap`] of trailers.
    ///
    /// `grpc-status` is required; `grpc-message` is optional and
    /// missing-or-empty translates to an empty `message` field.
    ///
    /// Returns [`StatusParseError::Missing`] when the `grpc-status`
    /// trailer is entirely absent — callers may then choose to map this
    /// to their own "incomplete response" error rather than panicking
    /// at the boundary.
    ///
    /// # Errors
    ///
    /// Returns a [`StatusParseError`] when the trailers are malformed.
    pub fn from_trailers(trailers: &HeaderMap) -> Result<Self, StatusParseError> {
        let raw = trailers.get(GRPC_STATUS).ok_or(StatusParseError::Missing)?;
        let code_str = header_value_to_str(raw).ok_or(StatusParseError::StatusNotUtf8)?;
        let code: u32 = code_str
            .parse()
            .map_err(|_| StatusParseError::StatusNotNumeric {
                value: code_str.to_string(),
            })?;

        let message = trailers
            .get(GRPC_MESSAGE)
            .map(decode_grpc_message)
            .unwrap_or_default();

        Ok(Self { code, message })
    }
}

/// Convert a [`HeaderValue`] to `&str`, or `None` if it is not UTF-8.
///
/// Kept private — callers should reach for [`Status::from_trailers`]
/// instead of poking at the headers themselves.
fn header_value_to_str(v: &HeaderValue) -> Option<&str> {
    v.to_str().ok()
}

/// Decode a `grpc-message` trailer per the gRPC HTTP/2 wire spec
/// (<https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md>).
///
/// The spec mandates RFC 3986 percent-decoding (`%HH` escapes only —
/// no `+`-as-space) of the message bytes, with the decoded bytes then
/// interpreted as UTF-8. Crucially, a malformed message MUST NOT
/// invalidate an otherwise-valid `grpc-status` — a higher-level
/// retry / auth handler that keys on the status code (Unauthenticated,
/// Unavailable, etc.) would otherwise break on any peer that ships a
/// non-canonical message.
///
/// Fallback chain, in order: percent-decode + UTF-8 → raw header as
/// UTF-8 → empty string. Every gRPC status parse returns a usable
/// `Status` even when the message side of the trailer pair is
/// malformed.
fn decode_grpc_message(raw: &HeaderValue) -> String {
    let raw_bytes = raw.as_bytes();
    if let Ok(decoded) = percent_decode(raw_bytes).decode_utf8() {
        return decoded.into_owned();
    }
    // Percent-decode failure or non-UTF-8 decoded bytes: fall back to
    // the raw header as UTF-8. If that also fails (opaque non-UTF-8
    // bytes), surface an empty message — the parsed `grpc-status`
    // remains valid so callers can still classify the RPC outcome.
    match raw.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderName;

    fn trailer(name: &'static str, value: &'static str) -> (HeaderName, HeaderValue) {
        (
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        )
    }

    #[test]
    fn parses_ok_status() {
        let mut h = HeaderMap::new();
        let (n, v) = trailer("grpc-status", "0");
        h.insert(n, v);
        let s = Status::from_trailers(&h).expect("status parsed");
        assert!(s.is_ok());
        assert_eq!(s.code(), 0);
        assert_eq!(s.message(), "");
    }

    #[test]
    fn parses_error_status_with_message() {
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("grpc-status"),
            HeaderValue::from_static("13"),
        );
        h.insert(
            HeaderName::from_static("grpc-message"),
            HeaderValue::from_static("internal"),
        );
        let s = Status::from_trailers(&h).expect("status parsed");
        assert!(!s.is_ok());
        assert_eq!(s.code(), 13);
        assert_eq!(s.message(), "internal");
        assert_eq!(s.to_string(), "grpc-status=13: internal");
    }

    #[test]
    fn missing_status_is_error_not_panic() {
        // No grpc-status at all — common pathological case where the
        // server resets the stream mid-response. Must surface as an
        // error so the caller can decide how to react, not panic.
        let h = HeaderMap::new();
        let err = Status::from_trailers(&h).expect_err("missing trailer rejected");
        assert_eq!(err, StatusParseError::Missing);
    }

    #[test]
    fn non_numeric_status_is_error() {
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("grpc-status"),
            HeaderValue::from_static("oops"),
        );
        let err = Status::from_trailers(&h).expect_err("non-numeric rejected");
        match err {
            StatusParseError::StatusNotNumeric { value } => {
                assert_eq!(value, "oops");
            }
            other => panic!("expected StatusNotNumeric, got {other:?}"),
        }
    }

    #[test]
    fn non_utf8_status_is_error() {
        let mut h = HeaderMap::new();
        // 0xff is not valid UTF-8 (continuation byte without lead).
        let v = HeaderValue::from_bytes(&[0xff]).expect("HeaderValue accepts opaque bytes");
        h.insert(HeaderName::from_static("grpc-status"), v);
        let err = Status::from_trailers(&h).expect_err("non-utf8 status rejected");
        assert_eq!(err, StatusParseError::StatusNotUtf8);
    }

    #[test]
    fn percent_decoded_message_round_trips() {
        // `%20` decodes to a space per RFC 3986. The gRPC HTTP/2 spec
        // mandates this decoding for `grpc-message` so the test
        // pins the contract from the wire.
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("grpc-status"),
            HeaderValue::from_static("13"),
        );
        h.insert(
            HeaderName::from_static("grpc-message"),
            HeaderValue::from_static("Hello%20world"),
        );
        let s = Status::from_trailers(&h).expect("status parsed");
        assert_eq!(s.code(), 13);
        assert_eq!(s.message(), "Hello world");
    }

    #[test]
    fn malformed_percent_escape_is_passed_through() {
        // `%2X` is not a valid `%HH` escape. The `percent-encoding`
        // crate passes invalid escapes through literally, so the
        // decoded UTF-8 still contains `%2X` verbatim. The spec
        // forbids a malformed message from invalidating a parsed
        // `grpc-status`; the test pins both the byte preservation
        // and the parse success.
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("grpc-status"),
            HeaderValue::from_static("13"),
        );
        h.insert(
            HeaderName::from_static("grpc-message"),
            HeaderValue::from_static("Bad%2Xescape"),
        );
        let s = Status::from_trailers(&h).expect("status parsed even with malformed escape");
        assert_eq!(s.code(), 13);
        assert_eq!(
            s.message(),
            "Bad%2Xescape",
            "invalid %HH escape preserved verbatim by percent-encoding"
        );
    }

    #[test]
    fn percent_escape_decoding_non_utf8_falls_back_to_raw() {
        // `%FF` decodes to byte 0xFF which is not valid UTF-8. The
        // parser must fall back to the raw header bytes (which here
        // are the ASCII string "%FF") rather than failing or
        // returning an empty message.
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("grpc-status"),
            HeaderValue::from_static("13"),
        );
        h.insert(
            HeaderName::from_static("grpc-message"),
            HeaderValue::from_static("%FF"),
        );
        let s = Status::from_trailers(&h).expect("status parsed despite non-utf8 percent decode");
        assert_eq!(s.code(), 13);
        assert_eq!(
            s.message(),
            "%FF",
            "non-utf8 decoded bytes fall back to raw header text"
        );
    }

    #[test]
    fn non_utf8_message_falls_back_to_empty() {
        // Opaque non-UTF-8 bytes in `grpc-message` must not invalidate
        // the parsed status. Per spec, the message is best-effort; we
        // surface an empty string and the caller still gets the
        // canonical `grpc-status` code.
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("grpc-status"),
            HeaderValue::from_static("13"),
        );
        let v = HeaderValue::from_bytes(&[0xff]).unwrap();
        h.insert(HeaderName::from_static("grpc-message"), v);
        let s = Status::from_trailers(&h).expect("status parsed despite non-utf8 message");
        assert_eq!(s.code(), 13);
        assert_eq!(
            s.message(),
            "",
            "non-utf8 message falls back to empty string"
        );
    }

    #[test]
    fn unauthenticated_surfaces_even_with_malformed_message() {
        // The motivating case: `grpc-status: 16` (Unauthenticated) with
        // a non-UTF-8 message used to invalidate the entire parse,
        // breaking auth / retry handlers that key on the status code.
        // The fixed parser surfaces a usable `Status` and lets the
        // higher layer classify Unauthenticated correctly.
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("grpc-status"),
            HeaderValue::from_static("16"),
        );
        let bad = HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap();
        h.insert(HeaderName::from_static("grpc-message"), bad);
        let s = Status::from_trailers(&h).expect("Unauthenticated parses despite bad message");
        assert_eq!(s.code(), 16, "Unauthenticated status code preserved");
        assert!(!s.is_ok());
    }

    #[test]
    fn display_omits_empty_message() {
        let s = Status::new(0, "");
        assert_eq!(s.to_string(), "grpc-status=0");
    }
}
