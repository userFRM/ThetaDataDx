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

/// Categorized decode failures. Each variant carries enough context for
/// programmatic recovery without parsing strings.
///
/// Constructed by [`Error::Decode`] and surfaced through the
/// `From<crate::decode::DecodeError>` impl when per-cell type mismatches
/// are detected after the table is decoded.
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum DecodeErrorKind {
    /// A FIE/FIT-encoded row contained fewer columns than the schema
    /// declares (or more, in the truncation-detection sense — the
    /// shape did not match).
    #[error("truncated row at index {row_idx}: expected {expected_columns}, got {actual_columns}")]
    TruncatedRow {
        row_idx: usize,
        expected_columns: usize,
        actual_columns: usize,
    },
    /// A column declared as one Arrow / FlatFile value variant carried a
    /// value of a different variant.
    #[error(
        "type mismatch at row {row_idx} column {column_name:?}: expected {expected:?}, got {actual:?}"
    )]
    ColumnTypeMismatch {
        row_idx: usize,
        column_name: String,
        expected: String,
        actual: String,
    },
    /// Protobuf deserialization failure.
    #[error("protobuf decode: {0}")]
    Protobuf(String),
    /// FIE/FIT codec failure (e.g., invalid nibble sequence,
    /// per-cell value type mismatch).
    #[error("codec: {0}")]
    Codec(String),
    /// Arrow `RecordBatch` construction failure (downstream of the
    /// decode step — schema/length mismatch surfaced by the Arrow
    /// builder layer).
    #[error("arrow: {0}")]
    Arrow(String),
    /// Generic decode failure that hasn't been categorized yet.
    #[error("other: {0}")]
    Other(String),
}

/// Categorized decompression failures.
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum DecompressErrorKind {
    /// zstd decompression failure (codec error, output buffer
    /// undersized, corrupt stream).
    #[error("zstd: {0}")]
    Zstd(String),
    /// Compression algorithm value did not map to a known
    /// `proto::CompressionAlgo` discriminant.
    #[error("unknown algorithm: {algo}")]
    UnknownAlgorithm { algo: i32 },
    /// Generic decompression failure that hasn't been categorized.
    #[error("other: {0}")]
    Other(String),
}

/// Categorized configuration / input-validation failures.
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum ConfigErrorKind {
    /// A user-supplied numeric value was outside the validated range.
    #[error("{field}: value {value} outside range [{min}, {max}]")]
    OutOfRange {
        field: String,
        value: i64,
        min: i64,
        max: i64,
    },
    /// A required field was missing.
    #[error("missing required field: {0}")]
    MissingField(String),
    /// A field's value was syntactically invalid (e.g., bad URL,
    /// bad host:port, bad date format).
    #[error("{field}: {message}")]
    InvalidValue { field: String, message: String },
    /// I/O error reading a config file.
    #[error("config file I/O: {0}")]
    Io(String),
    /// TOML parse error.
    #[error("TOML parse: {0}")]
    TomlParse(String),
    /// Internal invariant violated (e.g., semaphore closed, retry loop
    /// exited without producing a result). These represent SDK bugs
    /// surfacing as configuration errors, not user input errors.
    #[error("internal: {0}")]
    Internal(String),
    /// Other configuration failure that hasn't been categorized.
    #[error("other: {0}")]
    Other(String),
}

/// Typed mapping of [`tonic::Code`].
///
/// Folding `tonic::Status` through this enum lets callers pattern-match
/// on a stable Rust enum instead of stringly-typed status codes. The
/// numeric discriminants match the gRPC wire codes one-for-one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(i32)]
pub enum GrpcStatusKind {
    Ok = 0,
    Cancelled = 1,
    Unknown = 2,
    InvalidArgument = 3,
    DeadlineExceeded = 4,
    NotFound = 5,
    AlreadyExists = 6,
    PermissionDenied = 7,
    ResourceExhausted = 8,
    FailedPrecondition = 9,
    Aborted = 10,
    OutOfRange = 11,
    Unimplemented = 12,
    Internal = 13,
    Unavailable = 14,
    DataLoss = 15,
    Unauthenticated = 16,
}

impl GrpcStatusKind {
    /// Map a [`tonic::Code`] into the matching `GrpcStatusKind` variant.
    #[must_use]
    pub fn from_code(code: tonic::Code) -> Self {
        match code {
            tonic::Code::Ok => Self::Ok,
            tonic::Code::Cancelled => Self::Cancelled,
            tonic::Code::Unknown => Self::Unknown,
            tonic::Code::InvalidArgument => Self::InvalidArgument,
            tonic::Code::DeadlineExceeded => Self::DeadlineExceeded,
            tonic::Code::NotFound => Self::NotFound,
            tonic::Code::AlreadyExists => Self::AlreadyExists,
            tonic::Code::PermissionDenied => Self::PermissionDenied,
            tonic::Code::ResourceExhausted => Self::ResourceExhausted,
            tonic::Code::FailedPrecondition => Self::FailedPrecondition,
            tonic::Code::Aborted => Self::Aborted,
            tonic::Code::OutOfRange => Self::OutOfRange,
            tonic::Code::Unimplemented => Self::Unimplemented,
            tonic::Code::Internal => Self::Internal,
            tonic::Code::Unavailable => Self::Unavailable,
            tonic::Code::DataLoss => Self::DataLoss,
            tonic::Code::Unauthenticated => Self::Unauthenticated,
        }
    }
}

impl std::fmt::Display for GrpcStatusKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

/// Structured error type for `thetadatadx`.
///
/// All error variants carry enough context for callers to programmatically
/// match on the failure category (`kind`) without parsing error messages.
///
/// # Pattern matching
///
/// ```ignore
/// use thetadatadx::error::{Error, ConfigErrorKind, DecodeErrorKind, GrpcStatusKind};
///
/// match err {
///     Error::Decode { kind: DecodeErrorKind::TruncatedRow { row_idx, .. }, .. } => {
///         tracing::warn!(row_idx, "row was truncated; retrying");
///     }
///     Error::Config { kind: ConfigErrorKind::OutOfRange { field, value, min, max }, .. } => {
///         eprintln!("config field {field} value {value} must be in [{min}, {max}]");
///     }
///     Error::Grpc { kind: GrpcStatusKind::DeadlineExceeded, .. } => {
///         // retry with longer timeout
///     }
///     _ => {}
/// }
/// ```
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    /// gRPC transport-level error (TLS handshake, connection refused, etc.).
    #[error("gRPC transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    /// gRPC status error from the upstream MDDS server.
    #[error("gRPC status {kind}: {message}")]
    Grpc {
        kind: GrpcStatusKind,
        message: String,
    },

    /// Decompression failure (zstd, gzip, etc.).
    #[error("decompression failed ({kind}): {message}")]
    Decompress {
        kind: DecompressErrorKind,
        message: String,
    },

    /// Decode failure — covers both protobuf deserialization errors and
    /// per-cell type-mismatch failures produced after the table is decoded.
    #[error("decode failed ({kind}): {message}")]
    Decode {
        kind: DecodeErrorKind,
        message: String,
    },

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
    #[error("configuration error ({kind}): {message}")]
    Config {
        kind: ConfigErrorKind,
        message: String,
    },

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

    /// FLATFILES request, stream, or decode failed for the requested
    /// flat-file format.
    ///
    /// Returned by [`crate::ThetaDataDxClient::flatfile_request`] and the
    /// per-data-type convenience methods when the FLATFILES surface is
    /// unavailable or cannot complete the request. This may reflect
    /// authentication rejection, request rejection, stream interruption
    /// or truncation, or decode failure for any supported
    /// [`crate::flatfiles::FlatFileFormat`] (CSV or JSONL).
    /// Carries a structured [`crate::flatfiles::FlatFilesUnavailableReason`]
    /// so the caller can decide whether to retry, fall back, or surface
    /// the underlying server error to the user.
    #[error("FLATFILES unavailable: {0}")]
    FlatFilesUnavailable(crate::flatfiles::FlatFilesUnavailableReason),

    /// `reconnect_streaming` succeeded in re-establishing the FPSS session
    /// but failed to restore one or more of the previously active
    /// subscriptions. The streaming connection itself is healthy; the listed
    /// subscriptions need to be re-issued by the caller (or the caller may
    /// choose to retry the whole `reconnect_streaming` call).
    ///
    /// Each entry is `(SubscriptionKind, Contract)` describing the
    /// subscription that could not be restored. The original per-failure
    /// error has already been logged at `warn` level via `tracing` so
    /// operators can see the underlying cause; the caller-facing surface is
    /// the structured list so programmatic recovery is possible without
    /// log scraping.
    #[error("partial reconnect: {} subscription(s) failed to restore", .failed.len())]
    PartialReconnect {
        /// The subscriptions that failed to restore.
        failed: Vec<(
            crate::fpss::protocol::SubscriptionKind,
            crate::fpss::protocol::Contract,
        )>,
    },
}

impl Error {
    // ─── Config constructors ────────────────────────────────────────────

    /// Build a `Config` error categorized as `InvalidValue`.
    #[must_use]
    pub fn config_invalid(field: impl Into<String>, message: impl Into<String>) -> Self {
        let field = field.into();
        let message = message.into();
        let display = format!("{field}: {message}");
        Self::Config {
            kind: ConfigErrorKind::InvalidValue { field, message },
            message: display,
        }
    }

    /// Build a `Config` error categorized as `MissingField`.
    #[must_use]
    pub fn config_missing(field: impl Into<String>) -> Self {
        let field = field.into();
        let display = format!("missing required field: {field}");
        Self::Config {
            kind: ConfigErrorKind::MissingField(field),
            message: display,
        }
    }

    /// Build a `Config` error categorized as `OutOfRange`.
    #[must_use]
    pub fn config_out_of_range(field: impl Into<String>, value: i64, min: i64, max: i64) -> Self {
        let field = field.into();
        let display = format!("{field}: value {value} outside range [{min}, {max}]");
        Self::Config {
            kind: ConfigErrorKind::OutOfRange {
                field,
                value,
                min,
                max,
            },
            message: display,
        }
    }

    /// Build a `Config` error categorized as `Io` (config-file I/O failure).
    #[must_use]
    pub fn config_io(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Config {
            kind: ConfigErrorKind::Io(message.clone()),
            message,
        }
    }

    /// Build a `Config` error categorized as `TomlParse`.
    #[must_use]
    pub fn config_toml(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Config {
            kind: ConfigErrorKind::TomlParse(message.clone()),
            message,
        }
    }

    /// Build a `Config` error categorized as `Internal` (SDK invariant).
    #[must_use]
    pub fn config_internal(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Config {
            kind: ConfigErrorKind::Internal(message.clone()),
            message,
        }
    }

    /// Build a `Config` error with an unspecified kind.
    #[must_use]
    pub fn config_other(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Config {
            kind: ConfigErrorKind::Other(message.clone()),
            message,
        }
    }

    // ─── Decode constructors ────────────────────────────────────────────

    /// Build a `Decode` error categorized as a protobuf deserialization
    /// failure.
    #[must_use]
    pub fn decode_protobuf(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Decode {
            kind: DecodeErrorKind::Protobuf(message.clone()),
            message,
        }
    }

    /// Build a `Decode` error categorized as an FIE/FIT codec failure.
    #[must_use]
    pub fn decode_codec(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Decode {
            kind: DecodeErrorKind::Codec(message.clone()),
            message,
        }
    }

    /// Build a `Decode` error categorized as an Arrow construction
    /// failure.
    #[must_use]
    pub fn decode_arrow(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Decode {
            kind: DecodeErrorKind::Arrow(message.clone()),
            message,
        }
    }

    /// Build a `Decode` error categorized as a truncated row.
    #[must_use]
    pub fn decode_truncated_row(
        row_idx: usize,
        expected_columns: usize,
        actual_columns: usize,
    ) -> Self {
        let kind = DecodeErrorKind::TruncatedRow {
            row_idx,
            expected_columns,
            actual_columns,
        };
        let message = kind.to_string();
        Self::Decode { kind, message }
    }

    /// Build a `Decode` error categorized as a column type mismatch.
    #[must_use]
    pub fn decode_column_type_mismatch(
        row_idx: usize,
        column_name: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        let kind = DecodeErrorKind::ColumnTypeMismatch {
            row_idx,
            column_name: column_name.into(),
            expected: expected.into(),
            actual: actual.into(),
        };
        let message = kind.to_string();
        Self::Decode { kind, message }
    }

    /// Build a `Decode` error with an unspecified kind.
    #[must_use]
    pub fn decode_other(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Decode {
            kind: DecodeErrorKind::Other(message.clone()),
            message,
        }
    }

    // ─── Decompress constructors ────────────────────────────────────────

    /// Build a `Decompress` error categorized as a zstd codec failure.
    #[must_use]
    pub fn decompress_zstd(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Decompress {
            kind: DecompressErrorKind::Zstd(message.clone()),
            message,
        }
    }

    /// Build a `Decompress` error for an unrecognised compression
    /// algorithm.
    #[must_use]
    pub fn decompress_unknown_algorithm(algo: i32) -> Self {
        let kind = DecompressErrorKind::UnknownAlgorithm { algo };
        let message = kind.to_string();
        Self::Decompress { kind, message }
    }

    /// Build a `Decompress` error with an unspecified kind.
    #[must_use]
    pub fn decompress_other(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Decompress {
            kind: DecompressErrorKind::Other(message.clone()),
            message,
        }
    }
}

impl From<tdbe::error::Error> for Error {
    fn from(err: tdbe::error::Error) -> Self {
        // The pure-data crate carries a small error enum; fold its variants
        // into the closest `thetadatadx::Error` variant so callers can use
        // `?` when invoking `tdbe` APIs (e.g. `tdbe::right::parse_right`)
        // from a `Result<_, thetadatadx::Error>` context.
        match err {
            tdbe::error::Error::Config(msg) => Self::config_other(msg),
            tdbe::error::Error::Io(e) => Self::Io(e),
            other => Self::config_other(other.to_string()),
        }
    }
}

impl From<crate::decode::DecodeError> for Error {
    fn from(err: crate::decode::DecodeError) -> Self {
        // Per-cell decode failures surface through the same channel as
        // protobuf decode failures so callers pattern-match a single
        // `Error::Decode` variant. The `Codec` kind is the closest fit
        // for FIE/FIT-cell type-mismatch failures.
        Self::decode_codec(err.to_string())
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
        let kind = GrpcStatusKind::from_code(s.code());
        let message = match td_err {
            Some(td) => format!(
                "{} (ThetaData: {} -- {})",
                s.message(),
                td.name,
                td.description
            ),
            None => s.message().to_string(),
        };
        Self::Grpc { kind, message }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_is_send_sync_static() {
        fn assert_bounds<T: Send + Sync + 'static + std::error::Error>() {}
        assert_bounds::<Error>();
    }

    #[test]
    fn decode_truncated_row_roundtrip() {
        let err = Error::decode_truncated_row(7, 5, 3);
        match err {
            Error::Decode {
                kind:
                    DecodeErrorKind::TruncatedRow {
                        row_idx,
                        expected_columns,
                        actual_columns,
                    },
                ..
            } => {
                assert_eq!(row_idx, 7);
                assert_eq!(expected_columns, 5);
                assert_eq!(actual_columns, 3);
            }
            other => panic!("expected TruncatedRow, got {other:?}"),
        }
    }

    #[test]
    fn decode_column_type_mismatch_roundtrip() {
        let err = Error::decode_column_type_mismatch(2, "price", "Float64", "Utf8");
        match err {
            Error::Decode {
                kind:
                    DecodeErrorKind::ColumnTypeMismatch {
                        row_idx,
                        column_name,
                        expected,
                        actual,
                    },
                ..
            } => {
                assert_eq!(row_idx, 2);
                assert_eq!(column_name, "price");
                assert_eq!(expected, "Float64");
                assert_eq!(actual, "Utf8");
            }
            other => panic!("expected ColumnTypeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn decode_protobuf_kind_carried() {
        let err = Error::decode_protobuf("invalid wire tag");
        assert!(matches!(
            err,
            Error::Decode {
                kind: DecodeErrorKind::Protobuf(_),
                ..
            }
        ));
    }

    #[test]
    fn decode_codec_kind_carried() {
        let err = Error::decode_codec("bad nibble");
        assert!(matches!(
            err,
            Error::Decode {
                kind: DecodeErrorKind::Codec(_),
                ..
            }
        ));
    }

    #[test]
    fn decode_arrow_kind_carried() {
        let err = Error::decode_arrow("schema length mismatch");
        assert!(matches!(
            err,
            Error::Decode {
                kind: DecodeErrorKind::Arrow(_),
                ..
            }
        ));
    }

    #[test]
    fn decode_other_kind_carried() {
        let err = Error::decode_other("unspecified");
        assert!(matches!(
            err,
            Error::Decode {
                kind: DecodeErrorKind::Other(_),
                ..
            }
        ));
    }

    #[test]
    fn decompress_zstd_kind_carried() {
        let err = Error::decompress_zstd("input corrupted");
        assert!(matches!(
            err,
            Error::Decompress {
                kind: DecompressErrorKind::Zstd(_),
                ..
            }
        ));
    }

    #[test]
    fn decompress_unknown_algorithm_kind_carried() {
        let err = Error::decompress_unknown_algorithm(99);
        match err {
            Error::Decompress {
                kind: DecompressErrorKind::UnknownAlgorithm { algo },
                ..
            } => assert_eq!(algo, 99),
            other => panic!("expected UnknownAlgorithm, got {other:?}"),
        }
    }

    #[test]
    fn decompress_other_kind_carried() {
        let err = Error::decompress_other("unspecified");
        assert!(matches!(
            err,
            Error::Decompress {
                kind: DecompressErrorKind::Other(_),
                ..
            }
        ));
    }

    #[test]
    fn config_out_of_range_roundtrip() {
        let err = Error::config_out_of_range("fpss.timeout_ms", 0, 100, 60_000);
        match err {
            Error::Config {
                kind:
                    ConfigErrorKind::OutOfRange {
                        field,
                        value,
                        min,
                        max,
                    },
                ..
            } => {
                assert_eq!(field, "fpss.timeout_ms");
                assert_eq!(value, 0);
                assert_eq!(min, 100);
                assert_eq!(max, 60_000);
            }
            other => panic!("expected OutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn config_invalid_kind_carried() {
        let err = Error::config_invalid("mdds.uri", "not a URI");
        assert!(matches!(
            err,
            Error::Config {
                kind: ConfigErrorKind::InvalidValue { .. },
                ..
            }
        ));
    }

    #[test]
    fn config_missing_kind_carried() {
        let err = Error::config_missing("auth.email");
        assert!(matches!(
            err,
            Error::Config {
                kind: ConfigErrorKind::MissingField(_),
                ..
            }
        ));
    }

    #[test]
    fn config_io_kind_carried() {
        let err = Error::config_io("file not found");
        assert!(matches!(
            err,
            Error::Config {
                kind: ConfigErrorKind::Io(_),
                ..
            }
        ));
    }

    #[test]
    fn config_toml_kind_carried() {
        let err = Error::config_toml("expected `]`");
        assert!(matches!(
            err,
            Error::Config {
                kind: ConfigErrorKind::TomlParse(_),
                ..
            }
        ));
    }

    #[test]
    fn config_internal_kind_carried() {
        let err = Error::config_internal("semaphore closed");
        assert!(matches!(
            err,
            Error::Config {
                kind: ConfigErrorKind::Internal(_),
                ..
            }
        ));
    }

    #[test]
    fn config_other_kind_carried() {
        let err = Error::config_other("unspecified");
        assert!(matches!(
            err,
            Error::Config {
                kind: ConfigErrorKind::Other(_),
                ..
            }
        ));
    }

    #[test]
    fn grpc_status_kind_from_code_round_trip() {
        let cases = [
            (tonic::Code::Ok, GrpcStatusKind::Ok),
            (tonic::Code::Cancelled, GrpcStatusKind::Cancelled),
            (tonic::Code::Unknown, GrpcStatusKind::Unknown),
            (
                tonic::Code::InvalidArgument,
                GrpcStatusKind::InvalidArgument,
            ),
            (
                tonic::Code::DeadlineExceeded,
                GrpcStatusKind::DeadlineExceeded,
            ),
            (tonic::Code::NotFound, GrpcStatusKind::NotFound),
            (tonic::Code::AlreadyExists, GrpcStatusKind::AlreadyExists),
            (
                tonic::Code::PermissionDenied,
                GrpcStatusKind::PermissionDenied,
            ),
            (
                tonic::Code::ResourceExhausted,
                GrpcStatusKind::ResourceExhausted,
            ),
            (
                tonic::Code::FailedPrecondition,
                GrpcStatusKind::FailedPrecondition,
            ),
            (tonic::Code::Aborted, GrpcStatusKind::Aborted),
            (tonic::Code::OutOfRange, GrpcStatusKind::OutOfRange),
            (tonic::Code::Unimplemented, GrpcStatusKind::Unimplemented),
            (tonic::Code::Internal, GrpcStatusKind::Internal),
            (tonic::Code::Unavailable, GrpcStatusKind::Unavailable),
            (tonic::Code::DataLoss, GrpcStatusKind::DataLoss),
            (
                tonic::Code::Unauthenticated,
                GrpcStatusKind::Unauthenticated,
            ),
        ];
        for (code, expected) in cases {
            assert_eq!(
                GrpcStatusKind::from_code(code),
                expected,
                "mapping mismatch for {code:?}"
            );
        }
    }

    #[test]
    fn from_tonic_status_carries_kind() {
        let status = tonic::Status::new(tonic::Code::PermissionDenied, "tier insufficient");
        let err: Error = status.into();
        match err {
            Error::Grpc { kind, message } => {
                assert_eq!(kind, GrpcStatusKind::PermissionDenied);
                assert!(message.contains("tier insufficient"));
            }
            other => panic!("expected Error::Grpc, got {other:?}"),
        }
    }

    #[test]
    fn from_tonic_status_unauthenticated_kind() {
        let status = tonic::Status::new(tonic::Code::Unauthenticated, "expired token");
        let err: Error = status.into();
        match err {
            Error::Grpc { kind, .. } => assert_eq!(kind, GrpcStatusKind::Unauthenticated),
            other => panic!("expected Error::Grpc, got {other:?}"),
        }
    }
}
