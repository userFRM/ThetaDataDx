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
        std::fmt::Debug::fmt(self, f)
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
        std::fmt::Debug::fmt(self, f)
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

    /// Decode failure — covers both protobuf deserialization errors and
    /// per-cell type-mismatch failures produced after the table is decoded.
    #[error("Decode failed: {0}")]
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
    /// `MddsClient` succeed.
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

impl From<crate::decode::DecodeError> for Error {
    fn from(err: crate::decode::DecodeError) -> Self {
        // Per-cell decode failures surface through the same channel as
        // protobuf decode failures so callers pattern-match a single
        // `Error::Decode` variant.
        Self::Decode(err.to_string())
    }
}

impl From<tonic::Status> for Error {
    fn from(s: tonic::Status) -> Self {
        // Extract http_status_code from gRPC metadata and enrich the error
        // message with the ThetaData error name when available.
        let td_err = s
            .metadata()
            .get(tdbe::error::HTTP_STATUS_CODE_KEY)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u16>().ok())
            .and_then(tdbe::error::error_from_http_code);
        let status = format!("{:?}", s.code());
        let message = match td_err {
            Some(td) => format!(
                "{} (ThetaData: {} -- {})",
                s.message(),
                td.name,
                td.description
            ),
            None => s.message().to_string(),
        };
        Self::Grpc { status, message }
    }
}
