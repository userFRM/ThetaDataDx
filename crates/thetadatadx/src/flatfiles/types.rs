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

    /// Client-facing dataset token (`"trade_quote"`, `"open_interest"`,
    /// `"eod"`, …) for the request type.
    ///
    /// This is the canonical spelling the public surface accepts and emits:
    /// the request-type segment of the flat-file route, the tokens
    /// user-facing error text names, and the value rendered on response
    /// payloads. It is the single source of those tokens so the Rust
    /// variant identifier can never reach a client surface — emitting the
    /// debug form of the variant would diverge from the documented
    /// vocabulary callers parse against.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Eod => "eod",
            Self::Quote => "quote",
            Self::OpenInterest => "open_interest",
            Self::Ohlc => "ohlc",
            Self::Trade => "trade",
            Self::TradeQuote => "trade_quote",
        }
    }
}

impl fmt::Display for ReqType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Single source of truth for the `(SecType, ReqType)` pairs the flat-file
/// distribution actually serves.
///
/// The flat-file service publishes a fixed matrix of daily snapshot
/// datasets — option `trade_quote` / `open_interest` / `eod` and stock
/// `trade_quote` / `eod`. Every other request type (per-tick quotes,
/// trades, OHLC bars) is served by the historical endpoints, not as a
/// flat file. Sending an unserved pair yields a server
/// `INVALID_PARAMS:Invalid request type` rejection; this predicate lets
/// the request entry points reject the pair locally, before any network
/// round-trip, so callers see a typed invalid-parameter error instead.
#[must_use]
pub fn flat_file_serves(sec: SecType, req: ReqType) -> bool {
    matches!(
        (sec, req),
        (
            SecType::Option,
            ReqType::TradeQuote | ReqType::OpenInterest | ReqType::Eod
        ) | (SecType::Stock, ReqType::TradeQuote | ReqType::Eod)
    )
}

/// Lower-case dataset name for `req` as it appears in user-facing error
/// text (e.g. `open_interest`). Matches the request-type tokens the
/// public surface accepts.
pub(crate) fn req_dataset_name(req: ReqType) -> &'static str {
    req.as_str()
}

/// Every `(SecType, ReqType)` pair the flat-file distribution serves, in a
/// stable order suitable for advertising on a tool surface.
///
/// This is the single enumeration backing [`flat_file_serves`]: the predicate
/// answers membership, this constant lists the members. Tool surfaces (CLI
/// `flatfile`, the MCP flat-file tools, the REST flat-file routes) derive their
/// advertised `(sec_type, req_type)` combinations from this list so they can
/// never offer a pair the service rejects. Adding a served dataset here — and
/// to the `flat_file_serves` match — is the only change needed for every
/// surface to expose it.
pub const SERVED_DATASETS: &[(SecType, ReqType)] = &[
    (SecType::Option, ReqType::TradeQuote),
    (SecType::Option, ReqType::OpenInterest),
    (SecType::Option, ReqType::Eod),
    (SecType::Stock, ReqType::TradeQuote),
    (SecType::Stock, ReqType::Eod),
];

/// Reason a [`Client::flatfile_request`](crate::Client::flatfile_request)
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
    /// `INVALID_CREDENTIALS=0`, `ACCOUNT_ALREADY_CONNECTED=6`).
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
                write!(
                    f,
                    "historical auth rejected (RemoveReason ord={reason_code})"
                )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every flat-file request type maps to its exact client-facing dataset
    /// token. `as_str`/`Display` is the single source of these tokens on the
    /// public surface, so a drift here would diverge from the documented
    /// vocabulary callers parse against.
    #[test]
    fn req_type_tokens_are_exact() {
        for (req, token) in [
            (ReqType::Eod, "eod"),
            (ReqType::Quote, "quote"),
            (ReqType::OpenInterest, "open_interest"),
            (ReqType::Ohlc, "ohlc"),
            (ReqType::Trade, "trade"),
            (ReqType::TradeQuote, "trade_quote"),
        ] {
            assert_eq!(req.as_str(), token, "{req:?}");
            // `Display` routes through the same mapping.
            assert_eq!(req.to_string(), token, "{req:?}");
            // The Rust variant identifier must never reach a client surface.
            assert_ne!(token, format!("{req:?}"), "{req:?}");
        }
    }

    /// `SERVED_DATASETS` and [`flat_file_serves`] are the list and the
    /// membership predicate over the same set: every listed pair must be
    /// served, and every served pair (over the full enum cross-product) must
    /// be listed. A drift between the two would let a tool surface advertise a
    /// pair the request layer rejects, or hide one it accepts.
    #[test]
    fn served_datasets_and_predicate_agree() {
        let secs = [SecType::Option, SecType::Stock, SecType::Index];
        let reqs = [
            ReqType::Eod,
            ReqType::Quote,
            ReqType::OpenInterest,
            ReqType::Ohlc,
            ReqType::Trade,
            ReqType::TradeQuote,
        ];
        for sec in secs {
            for req in reqs {
                let listed = SERVED_DATASETS.contains(&(sec, req));
                assert_eq!(
                    listed,
                    flat_file_serves(sec, req),
                    "SERVED_DATASETS and flat_file_serves disagree on {sec} {}",
                    req.as_str()
                );
            }
        }
        // The served set is exactly the documented five datasets.
        assert_eq!(SERVED_DATASETS.len(), 5);
    }

    /// Every security type maps to its exact upper-case wire token; the
    /// Rust variant identifier must never reach the `SEC=` field.
    #[test]
    fn sec_type_tokens_are_exact() {
        for (sec, token) in [
            (SecType::Option, "OPTION"),
            (SecType::Stock, "STOCK"),
            (SecType::Index, "INDEX"),
        ] {
            assert_eq!(sec.to_string(), token, "{sec:?}");
        }
    }
}
