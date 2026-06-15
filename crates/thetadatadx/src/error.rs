//! The crate's public [`enum@Error`] type and its category enums.
//!
//! [`enum@Error`] is the single error surface returned across the SDK. Every
//! variant carries a typed category (`kind`) alongside a human-readable
//! message so callers can pattern-match on the failure class without
//! parsing `Display` strings, and the `Display` shapes stay stable for
//! bindings that read `to_string()`.

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

/// Classification of gRPC transport-level failures.
///
/// Mirrors the transport's `ChannelError` variants so callers can
/// pattern-match on the concrete transport fault (TLS handshake,
/// connection-level death, stream-level reset, etc.) without parsing
/// `Display` strings. Each variant is `#[non_exhaustive]` at the enum
/// level so future transport failure modes can be added without
/// breaking exhaustive matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransportErrorKind {
    /// TCP connect failed (DNS, refused, network unreachable, etc.).
    Tcp,
    /// TLS handshake failed (cert chain rejection, ALPN mismatch, etc.).
    Tls,
    /// The host string was not a valid DNS name for rustls.
    InvalidServerName,
    /// HTTP/2 session establishment failed over an already-connected
    /// transport.
    H2Handshake,
    /// h2 stream-level error scoped to a single RPC.
    H2Stream,
    /// Connection-level death (GOAWAY, IO failure, open-phase drop).
    ConnectionClosed,
    /// The request URI or `:path` for the RPC could not be built.
    InvalidPath,
}

impl TransportErrorKind {
    /// Stable string identifier for the variant — used in [`Error::Transport`]
    /// Display so bindings parsing `to_string()` see a stable token before
    /// the human-readable message.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Tls => "tls",
            Self::InvalidServerName => "invalid_server_name",
            Self::H2Handshake => "h2_handshake",
            Self::H2Stream => "h2_stream",
            Self::ConnectionClosed => "connection_closed",
            Self::InvalidPath => "invalid_path",
        }
    }
}

impl std::fmt::Display for TransportErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
/// Constructed by [`Error::Decode`] and surfaced when per-cell type
/// mismatches are detected after the table is decoded.
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum DecodeErrorKind {
    /// A FIE/FIT-encoded row contained fewer columns than the schema
    /// declares (or more, in the truncation-detection sense — the
    /// shape did not match).
    #[error("truncated row at index {row_idx}: expected {expected_columns}, got {actual_columns}")]
    TruncatedRow {
        /// Zero-based index of the offending row.
        row_idx: usize,
        /// Number of columns the schema declares.
        expected_columns: usize,
        /// Number of columns actually present in the row.
        actual_columns: usize,
    },
    /// A column declared as one Arrow / FlatFile value variant carried a
    /// value of a different variant.
    #[error(
        "type mismatch at row {row_idx} column {column_name:?}: expected {expected:?}, got {actual:?}"
    )]
    ColumnTypeMismatch {
        /// Zero-based index of the row whose cell failed type validation.
        row_idx: usize,
        /// Name of the column whose value type did not match the schema.
        column_name: String,
        /// Value type the schema declared for the column.
        expected: String,
        /// Value type actually carried by the cell.
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
    UnknownAlgorithm {
        /// Raw compression algorithm discriminant received on the wire.
        algo: i32,
    },
    /// The peer-advertised decompressed size exceeded the
    /// `max_message_size` ceiling threaded from
    /// [`crate::config::HistoricalConfig::max_message_size`]. A hostile peer
    /// that sets `ResponseData.original_size = i32::MAX` (≈ 2 GiB) is
    /// rejected at this variant before any `Vec::resize` runs, so the
    /// decoder cannot be coerced into a runaway allocation.
    #[error("decompressed payload size {size} exceeds max_message_size {max}")]
    MessageTooLarge {
        /// Advertised decompressed size on the wire (`original_size`
        /// for zstd; `compressed_data.len()` for the no-compress
        /// path).
        size: usize,
        /// Configured ceiling — mirrors `HistoricalConfig::max_message_size`.
        max: usize,
    },
}

/// Categorized configuration / input-validation failures.
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum ConfigErrorKind {
    /// A user-supplied numeric value was outside the validated range.
    #[error("{field}: value {value} outside range [{min}, {max}]")]
    OutOfRange {
        /// Name of the field that carried the out-of-range value.
        field: String,
        /// The supplied value that fell outside the valid range.
        value: i64,
        /// Lower bound of the valid range (inclusive).
        min: i64,
        /// Upper bound of the valid range (inclusive).
        max: i64,
    },
    /// A required field was missing.
    #[error("missing required field: {0}")]
    MissingField(String),
    /// A field's value was syntactically invalid (e.g., bad URL,
    /// bad host:port, bad date format).
    #[error("{field}: {message}")]
    InvalidValue {
        /// Name of the field whose value was syntactically invalid.
        field: String,
        /// Human-readable explanation of why the value was rejected.
        message: String,
    },
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
}

impl ConfigErrorKind {
    /// True when this kind reflects a rejected user-supplied parameter
    /// (a bad value, an out-of-range number, a missing required field)
    /// rather than an environmental fault (config-file I/O, TOML parse)
    /// or an internal invariant violation.
    ///
    /// The bindings route the user-input kinds to a dedicated
    /// invalid-parameter error class so a caller can distinguish a
    /// malformed-but-rejected argument from an unrelated configuration
    /// failure by class, while `Io` / `TomlParse` / `Internal` stay on
    /// the root error class.
    #[must_use]
    pub fn is_invalid_parameter(&self) -> bool {
        matches!(
            self,
            ConfigErrorKind::OutOfRange { .. }
                | ConfigErrorKind::MissingField(_)
                | ConfigErrorKind::InvalidValue { .. }
        )
    }
}

/// Canonical gRPC status codes.
///
/// Numeric discriminants match the gRPC wire codes one-for-one (see
/// <https://grpc.github.io/grpc/core/md_doc_statuscodes.html>).
/// Pattern-match on this enum instead of comparing raw `u32` codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u32)]
pub enum GrpcStatusKind {
    /// The operation completed successfully.
    Ok = 0,
    /// The operation was cancelled, typically by the caller.
    Cancelled = 1,
    /// An unknown error occurred, often a fault not surfaced with a more
    /// specific status code.
    Unknown = 2,
    /// The caller specified an invalid argument, independent of the
    /// system state.
    InvalidArgument = 3,
    /// The deadline expired before the operation could complete.
    DeadlineExceeded = 4,
    /// The requested entity was not found.
    NotFound = 5,
    /// The entity a caller attempted to create already exists.
    AlreadyExists = 6,
    /// The caller does not have permission to execute the operation.
    PermissionDenied = 7,
    /// A resource has been exhausted, such as a quota or rate limit.
    ResourceExhausted = 8,
    /// The system is not in a state required for the operation's
    /// execution.
    FailedPrecondition = 9,
    /// The operation was aborted, typically due to a concurrency conflict.
    Aborted = 10,
    /// The operation was attempted past the valid range.
    OutOfRange = 11,
    /// The operation is not implemented or not supported.
    Unimplemented = 12,
    /// An internal error occurred, signalling a broken invariant.
    Internal = 13,
    /// The service is currently unavailable, usually a transient
    /// condition that may be resolved by retrying.
    Unavailable = 14,
    /// Unrecoverable data loss or corruption.
    DataLoss = 15,
    /// The request does not have valid authentication credentials.
    Unauthenticated = 16,
}

impl GrpcStatusKind {
    /// Map a raw gRPC numeric code into the matching variant.
    ///
    /// Codes outside the canonical 0..=16 range fold into
    /// [`GrpcStatusKind::Unknown`] — the wire is what it is, and the
    /// caller already lost structured information by the time an
    /// out-of-range code arrived.
    #[must_use]
    pub fn from_u32(code: u32) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::Cancelled,
            3 => Self::InvalidArgument,
            4 => Self::DeadlineExceeded,
            5 => Self::NotFound,
            6 => Self::AlreadyExists,
            7 => Self::PermissionDenied,
            8 => Self::ResourceExhausted,
            9 => Self::FailedPrecondition,
            10 => Self::Aborted,
            11 => Self::OutOfRange,
            12 => Self::Unimplemented,
            13 => Self::Internal,
            14 => Self::Unavailable,
            15 => Self::DataLoss,
            16 => Self::Unauthenticated,
            // 2 (Unknown) and anything else fold to Unknown.
            _ => Self::Unknown,
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
    /// gRPC transport-level error (TLS handshake, connection refused,
    /// h2 protocol failure, GOAWAY from the server, etc.).
    ///
    /// Carries a typed [`TransportErrorKind`] so retry classifiers and
    /// bindings can dispatch on the concrete fault category without
    /// regexing the `Display` string. The Display shape stays stable
    /// (`transport error (<kind>): <message>`) so binding consumers
    /// that parse `to_string()` keep working across upgrades.
    #[error("transport error ({kind}): {message}")]
    Transport {
        /// Concrete transport failure category.
        kind: TransportErrorKind,
        /// Human-readable detail for logs and `Display`.
        message: String,
    },

    /// gRPC status error from the ThetaData historical data service.
    #[error("gRPC status {kind}: {message}")]
    Grpc {
        /// Canonical gRPC status code returned by the service.
        kind: GrpcStatusKind,
        /// Human-readable detail for logs and `Display`.
        message: String,
        /// Server-supplied minimum backoff before the next retry,
        /// decoded from the `google.rpc.RetryInfo` status detail when
        /// the server attached one. The retry loop raises its computed
        /// delay to at least this value so a server-instructed
        /// cooldown is honoured in full; `None` (the common case)
        /// leaves the client-side backoff schedule unchanged.
        retry_after: Option<std::time::Duration>,
    },

    /// Decompression failure (zstd, gzip, etc.).
    #[error("decompression failed ({kind}): {message}")]
    Decompress {
        /// Concrete decompression failure category.
        kind: DecompressErrorKind,
        /// Human-readable detail for logs and `Display`.
        message: String,
    },

    /// Decode failure — covers both protobuf deserialization errors and
    /// per-cell type-mismatch failures produced after the table is decoded.
    #[error("decode failed ({kind}): {message}")]
    Decode {
        /// Concrete decode failure category.
        kind: DecodeErrorKind,
        /// Human-readable detail for logs and `Display`.
        message: String,
    },

    /// Query returned no data rows.
    #[error("No data returned")]
    NoData,

    /// Authentication error.
    #[error("Authentication error ({kind}): {message}")]
    Auth {
        /// Concrete authentication failure category.
        kind: AuthErrorKind,
        /// Human-readable detail for logs and `Display`.
        message: String,
    },

    /// FPSS streaming error.
    #[error("FPSS error ({kind}): {message}")]
    Fpss {
        /// Concrete FPSS streaming failure category.
        kind: FpssErrorKind,
        /// Human-readable detail for logs and `Display`.
        message: String,
    },

    /// Configuration / input validation error.
    #[error("configuration error ({kind}): {message}")]
    Config {
        /// Concrete configuration / input-validation failure category.
        kind: ConfigErrorKind,
        /// Human-readable detail for logs and `Display`.
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
    /// underlying gRPC channel sends `RST_STREAM` and the
    /// request-semaphore permit is released; subsequent calls on the same
    /// client succeed.
    #[error("Request deadline exceeded after {duration_ms} ms")]
    Timeout {
        /// Configured budget in milliseconds.
        duration_ms: u64,
    },

    /// FLATFILES request, stream, or decode failed for the requested
    /// flat-file format.
    ///
    /// Returned by [`crate::Client::flatfile_request`] and the
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

    /// Build a `Decompress` error for a payload whose advertised
    /// decompressed size exceeds the channel's `max_message_size`
    /// ceiling. The decode path validates the advertised size before
    /// any allocation, refusing an oversized payload without a `Vec::resize`.
    #[must_use]
    pub fn decompress_message_too_large(size: usize, max: usize) -> Self {
        let kind = DecompressErrorKind::MessageTooLarge { size, max };
        let message = kind.to_string();
        Self::Decompress { kind, message }
    }

    // ─── Structured accessors ───────────────────────────────────────────

    /// Server-supplied minimum backoff before the next retry, when the
    /// upstream attached a `google.rpc.RetryInfo` detail to a
    /// rate-limit status.
    ///
    /// Returns `Some(duration)` only for a [`Error::Grpc`] carrying the
    /// hint; every other variant (and a rate-limit status without the
    /// detail) returns `None`. The language bindings surface this as a
    /// typed field on their rate-limit error class so a caller can read
    /// the back-off interval as a value instead of parsing it out of the
    /// message text.
    #[must_use]
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Error::Grpc { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

impl From<crate::tdbe::error::Error> for Error {
    fn from(err: crate::tdbe::error::Error) -> Self {
        // The data-format layer carries a small error enum; fold its
        // variants into the closest typed `thetadatadx::Error` variant so
        // callers can use `?` when invoking the offline analytics surface
        // (e.g. `greeks::all_greeks`, `greeks::parse_right`) from a
        // `Result<_, thetadatadx::Error>` context. Every bridge routes to
        // a typed `ConfigErrorKind` variant (`InvalidValue` for parse /
        // domain failures, `Io` for I/O surfaces) so retry classifiers can
        // dispatch on the structured kind. The field label is `"input"`:
        // these failures reflect a bad caller-supplied value, never an
        // SDK-internal name.
        match err {
            crate::tdbe::error::Error::Config(msg) => Self::config_invalid("input", msg),
            crate::tdbe::error::Error::Io(e) => Self::Io(e),
            other => Self::config_invalid("input", other.to_string()),
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

impl From<crate::grpc::Status> for Error {
    fn from(s: crate::grpc::Status) -> Self {
        // The transport carries the canonical `grpc-status` /
        // `grpc-message` pair plus the decoded `google.rpc.RetryInfo`
        // hint. Surface the numeric code + UTF-8 message as-is so
        // callers get `GrpcStatusKind` pattern-matching.
        let kind = GrpcStatusKind::from_u32(s.code());
        Self::Grpc {
            kind,
            message: s.message().to_string(),
            retry_after: s.retry_delay(),
        }
    }
}

impl From<crate::grpc::ChannelError> for Error {
    fn from(err: crate::grpc::ChannelError) -> Self {
        use crate::grpc::ChannelError;
        // Rpc / DeadlineExceeded route to their own variants — everything
        // else folds into a typed `Transport { kind, message }` so retry
        // classifiers downstream can dispatch on the structured fault
        // without parsing `Display`.
        match err {
            ChannelError::Rpc { status } => Self::from(status),
            ChannelError::DeadlineExceeded { duration_ms } => Self::Timeout { duration_ms },
            other => {
                let kind = match &other {
                    ChannelError::Tcp { .. } => TransportErrorKind::Tcp,
                    ChannelError::Tls { .. } => TransportErrorKind::Tls,
                    ChannelError::InvalidServerName { .. } => TransportErrorKind::InvalidServerName,
                    ChannelError::H2Handshake(_) => TransportErrorKind::H2Handshake,
                    ChannelError::H2Stream(_) => TransportErrorKind::H2Stream,
                    ChannelError::InvalidPath { .. } => TransportErrorKind::InvalidPath,
                    ChannelError::ConnectionClosed(_) => TransportErrorKind::ConnectionClosed,
                    // Rpc / DeadlineExceeded handled above — keep compiler
                    // exhaustiveness happy without a runtime branch.
                    ChannelError::Rpc { .. } | ChannelError::DeadlineExceeded { .. } => {
                        TransportErrorKind::ConnectionClosed
                    }
                };
                Self::Transport {
                    kind,
                    message: other.to_string(),
                }
            }
        }
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
    fn retry_after_exposes_grpc_hint() {
        let with_hint = Error::Grpc {
            kind: GrpcStatusKind::ResourceExhausted,
            message: "429".into(),
            retry_after: Some(std::time::Duration::from_millis(1500)),
        };
        assert_eq!(
            with_hint.retry_after(),
            Some(std::time::Duration::from_millis(1500))
        );

        let no_hint = Error::Grpc {
            kind: GrpcStatusKind::ResourceExhausted,
            message: "429".into(),
            retry_after: None,
        };
        assert_eq!(no_hint.retry_after(), None);

        // Non-gRPC variants never carry a retry hint.
        assert_eq!(Error::NoData.retry_after(), None);
        assert_eq!(Error::Timeout { duration_ms: 500 }.retry_after(), None);
    }

    #[test]
    fn config_kind_invalid_parameter_partition() {
        // User-input rejections are invalid-parameter kinds.
        assert!(ConfigErrorKind::OutOfRange {
            field: "fpss.timeout_ms".into(),
            value: 0,
            min: 100,
            max: 60_000,
        }
        .is_invalid_parameter());
        assert!(ConfigErrorKind::MissingField("auth.email".into()).is_invalid_parameter());
        assert!(ConfigErrorKind::InvalidValue {
            field: "mdds.uri".into(),
            message: "not a URI".into(),
        }
        .is_invalid_parameter());

        // Environmental / internal faults are not.
        assert!(!ConfigErrorKind::Io("file not found".into()).is_invalid_parameter());
        assert!(!ConfigErrorKind::TomlParse("expected `]`".into()).is_invalid_parameter());
        assert!(!ConfigErrorKind::Internal("semaphore closed".into()).is_invalid_parameter());
    }

    #[test]
    fn grpc_status_kind_from_u32_round_trip() {
        let cases = [
            (0u32, GrpcStatusKind::Ok),
            (1, GrpcStatusKind::Cancelled),
            (2, GrpcStatusKind::Unknown),
            (3, GrpcStatusKind::InvalidArgument),
            (4, GrpcStatusKind::DeadlineExceeded),
            (5, GrpcStatusKind::NotFound),
            (6, GrpcStatusKind::AlreadyExists),
            (7, GrpcStatusKind::PermissionDenied),
            (8, GrpcStatusKind::ResourceExhausted),
            (9, GrpcStatusKind::FailedPrecondition),
            (10, GrpcStatusKind::Aborted),
            (11, GrpcStatusKind::OutOfRange),
            (12, GrpcStatusKind::Unimplemented),
            (13, GrpcStatusKind::Internal),
            (14, GrpcStatusKind::Unavailable),
            (15, GrpcStatusKind::DataLoss),
            (16, GrpcStatusKind::Unauthenticated),
        ];
        for (code, expected) in cases {
            assert_eq!(
                GrpcStatusKind::from_u32(code),
                expected,
                "mapping mismatch for code={code}"
            );
        }
        // Out-of-range codes fold to Unknown.
        assert_eq!(GrpcStatusKind::from_u32(99), GrpcStatusKind::Unknown);
        assert_eq!(GrpcStatusKind::from_u32(u32::MAX), GrpcStatusKind::Unknown);
    }

    #[test]
    fn from_grpc_status_carries_kind() {
        let status = crate::grpc::Status::new(7, "tier insufficient");
        let err: Error = status.into();
        match err {
            Error::Grpc {
                kind,
                message,
                retry_after,
            } => {
                assert_eq!(kind, GrpcStatusKind::PermissionDenied);
                assert!(message.contains("tier insufficient"));
                assert_eq!(retry_after, None, "no RetryInfo detail on this status");
            }
            other => panic!("expected Error::Grpc, got {other:?}"),
        }
    }

    #[test]
    fn from_grpc_status_unauthenticated_kind() {
        let status = crate::grpc::Status::new(16, "expired token");
        let err: Error = status.into();
        match err {
            Error::Grpc { kind, .. } => assert_eq!(kind, GrpcStatusKind::Unauthenticated),
            other => panic!("expected Error::Grpc, got {other:?}"),
        }
    }

    #[test]
    fn from_channel_error_routes_deadline_to_timeout() {
        let err: Error = crate::grpc::ChannelError::DeadlineExceeded { duration_ms: 123 }.into();
        match err {
            Error::Timeout { duration_ms } => assert_eq!(duration_ms, 123),
            other => panic!("expected Error::Timeout, got {other:?}"),
        }
    }

    #[test]
    fn from_channel_error_routes_rpc_to_grpc() {
        let status = crate::grpc::Status::new(13, "internal");
        let err: Error = crate::grpc::ChannelError::Rpc { status }.into();
        match err {
            Error::Grpc { kind, message, .. } => {
                assert_eq!(kind, GrpcStatusKind::Internal);
                assert!(message.contains("internal"));
            }
            other => panic!("expected Error::Grpc, got {other:?}"),
        }
    }

    /// Every non-Rpc / non-DeadlineExceeded `ChannelError` variant must
    /// round-trip through `From<ChannelError> for Error` with a typed
    /// [`TransportErrorKind`] that mirrors the variant. Pins the
    /// structured payload promise the binding layer relies on.
    #[test]
    fn from_channel_error_routes_every_transport_variant_to_typed_kind() {
        use crate::grpc::ChannelError;

        let cases: Vec<(ChannelError, TransportErrorKind)> = vec![
            (
                ChannelError::Tcp {
                    host: "h".into(),
                    port: 1,
                    source: std::io::Error::other("e"),
                },
                TransportErrorKind::Tcp,
            ),
            (
                ChannelError::Tls {
                    host: "h".into(),
                    source: std::io::Error::other("e"),
                },
                TransportErrorKind::Tls,
            ),
            (
                ChannelError::InvalidServerName { host: "h".into() },
                TransportErrorKind::InvalidServerName,
            ),
            (
                ChannelError::H2Handshake("e".into()),
                TransportErrorKind::H2Handshake,
            ),
            (
                ChannelError::H2Stream("e".into()),
                TransportErrorKind::H2Stream,
            ),
            (
                ChannelError::InvalidPath {
                    path: "/".into(),
                    message: "e".into(),
                },
                TransportErrorKind::InvalidPath,
            ),
            (
                ChannelError::ConnectionClosed("e".into()),
                TransportErrorKind::ConnectionClosed,
            ),
        ];

        for (input, expected) in cases {
            let err: Error = input.into();
            match err {
                Error::Transport { kind, message } => {
                    assert_eq!(kind, expected, "kind mismatch (display={message})");
                    assert!(
                        !message.is_empty(),
                        "transport error message must not be empty"
                    );
                }
                other => panic!("expected Error::Transport, got {other:?}"),
            }
        }
    }

    /// Display shape is part of the binding contract — assert the
    /// `transport error (<kind>): <message>` skeleton is preserved.
    #[test]
    fn transport_display_carries_kind_token() {
        let err = Error::Transport {
            kind: TransportErrorKind::H2Stream,
            message: "test message".into(),
        };
        let display = err.to_string();
        assert!(
            display.contains("h2_stream"),
            "transport display must carry kind token, got {display:?}"
        );
        assert!(
            display.contains("test message"),
            "transport display must carry message, got {display:?}"
        );
    }
}
