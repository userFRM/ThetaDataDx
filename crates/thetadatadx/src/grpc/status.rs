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
use thiserror::Error;

/// `grpc-status` trailer name.
pub(crate) const GRPC_STATUS: &str = "grpc-status";
/// `grpc-message` trailer name (RFC 3986 percent-encoded UTF-8 per spec).
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
#[derive(Debug, Error, PartialEq, Eq)]
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
    /// `grpc-message` was present but not a valid UTF-8 string.
    #[error("`grpc-message` trailer is not valid UTF-8")]
    MessageNotUtf8,
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

        let message = match trailers.get(GRPC_MESSAGE) {
            None => String::new(),
            Some(raw_msg) => header_value_to_str(raw_msg)
                .ok_or(StatusParseError::MessageNotUtf8)?
                .to_string(),
        };

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
    fn non_utf8_message_is_error() {
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_static("grpc-status"),
            HeaderValue::from_static("13"),
        );
        let v = HeaderValue::from_bytes(&[0xff]).unwrap();
        h.insert(HeaderName::from_static("grpc-message"), v);
        let err = Status::from_trailers(&h).expect_err("non-utf8 message rejected");
        assert_eq!(err, StatusParseError::MessageNotUtf8);
    }

    #[test]
    fn display_omits_empty_message() {
        let s = Status::new(0, "");
        assert_eq!(s.to_string(), "grpc-status=0");
    }
}
