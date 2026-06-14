//! Public enums for the FLATFILES surface.
//!
//! Defines the security and request types a caller selects, plus the typed
//! reason an unavailable FLATFILES response carries. The reason classifier
//! ([`FlatFilesUnavailableReason::is_transient`]) drives the request
//! driver's retry-vs-surface decision.

use std::fmt;

/// Security types accepted by the FLATFILES route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecType {
    /// Listed options.
    Option,
    /// Equities.
    Stock,
    /// Index instruments.
    Index,
}

impl SecType {
    /// Wire string the server expects in the `SEC=` query field.
    pub(crate) fn as_wire(self) -> &'static str {
        match self {
            Self::Option => "OPTION",
            Self::Stock => "STOCK",
            Self::Index => "INDEX",
        }
    }
}

impl fmt::Display for SecType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire())
    }
}

/// FLATFILES request types and their V2 wire codes.
///
/// These integers go into the `REQ=` field of the FLAT_FILE request payload.
/// They are the V2 server's `ReqType.code()` values, captured from the
/// vendor's bundled enum. They are **not** the same as ordinal positions; the
/// V2 enum maps `OPEN_INTEREST → 103`, `TRADE → 201`, `TRADE_QUOTE → 207`, etc.
/// Sending the ordinal instead of the code yields
/// `INVALID_PARAMS:Invalid request type` from the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ReqType {
    /// End-of-day summary.
    Eod = 1,
    /// Best bid/offer quotes.
    Quote = 101,
    /// Open interest.
    OpenInterest = 103,
    /// Open/high/low/close bars.
    Ohlc = 104,
    /// Trades.
    Trade = 201,
    /// Trades interleaved with the prevailing quote.
    TradeQuote = 207,
}

impl ReqType {
    /// Returns the V2 server `ReqType.code()` value for the `REQ=` field.
    pub(crate) fn as_wire(self) -> u32 {
        self as u32
    }
}

/// Reason a [`ThetaDataDxClient::flatfile_request`](crate::ThetaDataDxClient::flatfile_request)
/// call cannot return CSV.
///
/// Returned inside `Error::FlatFilesUnavailable` so callers can decide
/// whether to fall back to the V3 fan-out path or to retry later.
///
/// The variants partition into two retry classes consumed by
/// [`FlatFilesUnavailableReason::is_transient`]:
///
/// * **Terminal** — re-running the request with identical inputs will
///   fail the same way. Auth rejection on a permanent credential reason
///   code, and `RequestRejected` from a malformed request, both fall
///   here. The flatfile driver gives up immediately; no automatic retry.
/// * **Transient** — the request might succeed on a fresh connection
///   (server hop, momentary network blip, mid-stream truncation). The
///   flatfile driver retries with exponential backoff up to the
///   [`crate::config::FlatFilesConfig::max_attempts`] budget before
///   surfacing the error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlatFilesUnavailableReason {
    /// Server returned a `RemoveReason` ordinal during auth (e.g.
    /// `INVALID_CREDENTIALS=1`, `ACCOUNT_ALREADY_CONNECTED=7`).
    AuthRejected {
        /// Server-supplied removal-reason ordinal explaining the rejection.
        reason_code: u16,
    },
    /// Server replied with an `ERROR` frame to the FLAT_FILE request itself
    /// (e.g. `INVALID_PARAMS:Invalid request type`).
    RequestRejected {
        /// Diagnostic message text returned by the server.
        server_message: String,
    },
    /// Connection dropped before the response completed.
    StreamTruncated {
        /// Number of payload bytes received before the stream was cut short.
        bytes_received: u64,
    },
}

impl FlatFilesUnavailableReason {
    /// Returns `true` when the same request issued on a fresh connection
    /// might succeed (network blip, mid-stream drop). Drives the
    /// flatfile retry loop's terminal-vs-retryable decision.
    ///
    /// `AuthRejected` is treated as terminal for every credential
    /// reason code in the permanent set
    /// ([`crate::fpss::reconnect_delay`] returns `None` for these); the
    /// transient auth reasons (e.g. `ServerRestarting`) are surfaced as
    /// retryable. `RequestRejected` is always terminal — bad params
    /// will not fix themselves on retry. `StreamTruncated` is always
    /// transient.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::StreamTruncated { .. } => true,
            Self::RequestRejected { .. } => false,
            Self::AuthRejected { reason_code } => auth_reason_is_transient(*reason_code),
        }
    }

    /// Inverse of [`Self::is_transient`]. Provided so callers can keep
    /// the intent line-by-line readable in classifier chains.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        !self.is_transient()
    }
}

/// Classify a `RemoveReason` ordinal received during MDDS legacy login
/// as transient (retry on a fresh connection) vs terminal (no amount of
/// retrying will fix it).
///
/// Mirrors [`crate::fpss::reconnect_delay`]: the same permanent set
/// applies on both surfaces — `InvalidCredentials`, `InvalidLoginValues`,
/// `InvalidLoginSize`, `AccountAlreadyConnected`, `FreeAccount`,
/// `ServerUserDoesNotExist`, `InvalidCredentialsNullUser`. Every other
/// reason code (and the `0` sentinel emitted when the payload is too
/// short) routes through the retry path.
fn auth_reason_is_transient(reason_code: u16) -> bool {
    // Wire ordinals match `crate::fpss::protocol::wire::remove_reason_from_code`.
    //   0 InvalidCredentials, 1 InvalidLoginValues, 2 InvalidLoginSize,
    //   6 AccountAlreadyConnected, 9 FreeAccount,
    //   17 ServerUserDoesNotExist, 18 InvalidCredentialsNullUser.
    !matches!(reason_code, 0 | 1 | 2 | 6 | 9 | 17 | 18)
}

impl fmt::Display for FlatFilesUnavailableReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthRejected { reason_code } => {
                write!(f, "MDDS auth rejected (RemoveReason ord={reason_code})")
            }
            Self::RequestRejected { server_message } => {
                write!(f, "FLAT_FILE request rejected: {server_message}")
            }
            Self::StreamTruncated { bytes_received } => {
                write!(f, "stream truncated after {bytes_received} bytes")
            }
        }
    }
}
