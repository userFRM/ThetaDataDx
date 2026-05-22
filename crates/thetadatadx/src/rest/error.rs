//! REST-transport error type.
//!
//! Distinct from [`crate::error::Error`] only at the conversion
//! boundary -- `From<RestError> for Error` lifts every variant into a
//! structured `crate::Error` so call sites consuming both gRPC and
//! REST results uniformly via `?` keep the same error vocabulary.

use std::fmt;

use crate::error::{Error, TransportErrorKind};

/// REST-transport error.
#[derive(Debug)]
pub enum RestError {
    /// Underlying HTTP transport failed (connect refused, TLS error,
    /// connection reset, timeout, ...). Wraps [`reqwest::Error`].
    Http(reqwest::Error),
    /// The Terminal returned a non-2xx status; the body (truncated to
    /// 4 KiB to keep crash dumps bounded) is captured for triage.
    HttpStatus {
        /// HTTP status code returned.
        status: u16,
        /// First 4 KiB of the response body.
        body: String,
    },
    /// CSV decode failed: the response body did not parse as a valid
    /// `<header>\n<row1>\n<row2>...` table, or a column referenced by
    /// the row decoder was missing.
    CsvDecode {
        /// Human-readable reason for the failure.
        reason: String,
        /// 0-based row index in the response that surfaced the
        /// failure (`usize::MAX` if the header row itself was
        /// malformed).
        row: usize,
    },
    /// A required column was missing from the CSV header on a non-empty
    /// response. Distinct from [`Self::CsvDecode`] so a forward-compat
    /// schema change (server adds / renames a column) is recoverable
    /// up the stack without a string match.
    MissingColumn {
        /// Schema-side name of the absent column.
        column: &'static str,
        /// Comma-separated list of headers the response actually
        /// carried; useful in tracing diagnostics.
        available: String,
    },
}

impl fmt::Display for RestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "REST HTTP transport error: {e}"),
            Self::HttpStatus { status, body } => write!(
                f,
                "REST HTTP {status} from local Terminal: {}",
                body.lines().next().unwrap_or("")
            ),
            Self::CsvDecode { reason, row } => {
                write!(f, "REST CSV decode failed at row {row}: {reason}")
            }
            Self::MissingColumn { column, available } => write!(
                f,
                "REST CSV header missing required column {column:?} (available: {available})"
            ),
        }
    }
}

impl std::error::Error for RestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for RestError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl From<RestError> for Error {
    fn from(e: RestError) -> Self {
        match e {
            RestError::Http(err) => Self::Transport {
                kind: TransportErrorKind::ConnectionClosed,
                message: format!("REST transport: {err}"),
            },
            RestError::HttpStatus { status, body } => Self::Transport {
                kind: TransportErrorKind::UnexpectedHttpStatus,
                message: format!(
                    "REST returned HTTP {status}: {}",
                    body.lines().next().unwrap_or("")
                ),
            },
            RestError::CsvDecode { reason, row } => Self::Transport {
                kind: TransportErrorKind::Codec,
                message: format!("REST CSV decode at row {row}: {reason}"),
            },
            RestError::MissingColumn { column, available } => Self::Transport {
                kind: TransportErrorKind::Codec,
                message: format!(
                    "REST CSV header missing column {column:?} (available: {available})"
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_decode_lifts_into_transport_codec() {
        let lifted: Error = RestError::CsvDecode {
            reason: "bad i32 at col 1 row 7: 'xyz'".to_owned(),
            row: 7,
        }
        .into();
        match lifted {
            Error::Transport { kind, message } => {
                assert_eq!(kind, TransportErrorKind::Codec);
                assert!(message.contains("REST CSV decode"), "message: {message}");
                assert!(message.contains("row 7"), "message: {message}");
            }
            other => panic!("expected Transport::Codec, got {other:?}"),
        }
    }

    #[test]
    fn missing_column_lifts_into_transport_codec() {
        let lifted: Error = RestError::MissingColumn {
            column: "ms_of_day",
            available: "bid,ask,date".to_owned(),
        }
        .into();
        match lifted {
            Error::Transport { kind, message } => {
                assert_eq!(kind, TransportErrorKind::Codec);
                assert!(
                    message.contains("ms_of_day"),
                    "message did not name missing column: {message}"
                );
                assert!(
                    message.contains("bid,ask,date"),
                    "message did not list available columns: {message}"
                );
            }
            other => panic!("expected Transport::Codec, got {other:?}"),
        }
    }

    #[test]
    fn http_status_still_lifts_into_unexpected_http_status() {
        let lifted: Error = RestError::HttpStatus {
            status: 503,
            body: "service unavailable\nretry later".to_owned(),
        }
        .into();
        match lifted {
            Error::Transport { kind, .. } => {
                assert_eq!(kind, TransportErrorKind::UnexpectedHttpStatus);
            }
            other => panic!("expected Transport::UnexpectedHttpStatus, got {other:?}"),
        }
    }
}
