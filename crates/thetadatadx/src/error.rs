use thiserror::Error;

/// Classification of authentication failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthErrorKind {
    /// Wrong email/password or expired credentials.
    InvalidCredentials,
    /// Transient network error (DNS, timeout, connection refused).
    NetworkError,
    /// Upstream server returned a non-auth HTTP error.
    ServerError,
    /// Request timed out.
    Timeout,
}

impl std::fmt::Display for AuthErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCredentials => write!(f, "InvalidCredentials"),
            Self::NetworkError => write!(f, "NetworkError"),
            Self::ServerError => write!(f, "ServerError"),
            Self::Timeout => write!(f, "Timeout"),
        }
    }
}

/// Classification of FPSS streaming failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FpssErrorKind {
    /// Could not connect to any FPSS server.
    ConnectionRefused,
    /// Operation timed out.
    Timeout,
    /// Wire protocol violation (corrupt frame, unexpected payload).
    ProtocolError,
    /// Server disconnected the client.
    Disconnected,
    /// Server sent `TOO_MANY_REQUESTS` -- back off.
    TooManyRequests,
}

impl std::fmt::Display for FpssErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionRefused => write!(f, "ConnectionRefused"),
            Self::Timeout => write!(f, "Timeout"),
            Self::ProtocolError => write!(f, "ProtocolError"),
            Self::Disconnected => write!(f, "Disconnected"),
            Self::TooManyRequests => write!(f, "TooManyRequests"),
        }
    }
}

/// Structured error type for `thetadatadx`.
///
/// All error variants carry enough context for callers to programmatically
/// match on the failure category (`kind`) without parsing error messages.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// gRPC transport-level error (TLS handshake, connection refused, etc.).
    #[error("gRPC transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    /// gRPC status error from the upstream MDDS server.
    #[error("gRPC status: {status} -- {message}")]
    Grpc { status: String, message: String },

    /// Decompression failure (zstd, gzip, etc.).
    #[error("Decompression failed: {0}")]
    Decompress(String),

    /// Protobuf decode failure.
    #[error("Protobuf decode failed: {0}")]
    Decode(String),

    /// Query returned no data rows.
    #[error("No data returned")]
    NoData,

    /// Authentication error.
    #[error("Authentication error ({kind}): {message}")]
    Auth {
        kind: AuthErrorKind,
        message: String,
    },

    /// FPSS streaming error.
    #[error("FPSS error ({kind}): {message}")]
    Fpss {
        kind: FpssErrorKind,
        message: String,
    },

    /// Configuration / input validation error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// HTTP error (reqwest).
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// I/O error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// TLS error.
    #[error("TLS error: {0}")]
    Tls(#[from] rustls::Error),

    /// Per-request deadline elapsed.
    ///
    /// Returned when a `with_deadline(d)` (Rust builder) or `timeout_ms`
    /// (FFI / Python / Go / C++) elapses while the gRPC call was in flight.
    /// The in-flight future is dropped before this error is returned, so the
    /// underlying `tonic::transport::Channel` cancels the stream and the
    /// request-semaphore permit is released; subsequent calls on the same
    /// `DirectClient` succeed.
    #[error("Request deadline exceeded after {duration_ms} ms")]
    Timeout {
        /// Configured budget in milliseconds.
        duration_ms: u64,
    },
}

impl From<tdbe::error::Error> for Error {
    fn from(err: tdbe::error::Error) -> Self {
        // The pure-data crate carries a small error enum; fold its variants
        // into the closest `thetadatadx::Error` variant so callers can use
        // `?` when invoking `tdbe` APIs (e.g. `tdbe::right::parse_right`)
        // from a `Result<_, thetadatadx::Error>` context.
        match err {
            tdbe::error::Error::Config(msg) => Self::Config(msg),
            tdbe::error::Error::Io(e) => Self::Io(e),
            other => Self::Config(other.to_string()),
        }
    }
}

impl From<tonic::Status> for Error {
    fn from(s: tonic::Status) -> Self {
        // Extract http_status_code from gRPC metadata and enrich the error
        // message with the ThetaData error name when available.
        let metadata_str = format!("{:?}", s.metadata());
        if let Some(td_err) = tdbe::errors::error_from_grpc_metadata(&metadata_str) {
            Self::Grpc {
                status: format!("{:?}", s.code()),
                message: format!(
                    "{} (ThetaData: {} -- {})",
                    s.message(),
                    td_err.name,
                    td_err.description
                ),
            }
        } else {
            Self::Grpc {
                status: format!("{:?}", s.code()),
                message: s.message().to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_without_metadata_passes_through() {
        let status = tonic::Status::internal("something went wrong");
        let err = Error::from(status);
        let msg = format!("{}", err);
        assert!(msg.contains("something went wrong"));
    }

    #[test]
    fn auth_error_display_includes_kind() {
        let err = Error::Auth {
            kind: AuthErrorKind::InvalidCredentials,
            message: "bad password".to_string(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("InvalidCredentials"));
        assert!(msg.contains("bad password"));
    }

    #[test]
    fn fpss_error_display_includes_kind() {
        let err = Error::Fpss {
            kind: FpssErrorKind::Disconnected,
            message: "server rejected login".to_string(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("Disconnected"));
        assert!(msg.contains("server rejected login"));
    }

    #[test]
    fn timeout_error_carries_duration() {
        let err = Error::Timeout {
            duration_ms: 60_000,
        };
        let msg = format!("{err}");
        assert!(msg.contains("60000"));
        assert!(msg.contains("deadline"));
    }
}
