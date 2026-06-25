//! Public enums for the FLATFILES surface.
//!
//! Defines the security and request types a caller selects, plus the typed
//! reason an unavailable FLATFILES response carries. The reason classifier
//! ([`FlatFilesUnavailableReason::is_transient`]) drives the request
//! driver's retry-vs-surface decision.

use std::fmt;

use crate::tdbe::types::enums::RemoveReason;

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
/// datasets — option `trade_quote` / `open_interest` / `eod`, stock
/// `trade_quote` / `eod`, and index `eod`. Every other request type
/// (per-tick quotes, trades, OHLC bars) is served by the historical
/// endpoints, not as a flat file. Sending an unserved pair yields a server
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
            | (SecType::Index, ReqType::Eod)
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
    (SecType::Index, ReqType::Eod),
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
///   fail the same way. A `DISCONNECTED` carrying a terminal
///   `RemoveReason` (a permanent credential/account code, or a no-data /
///   validation code such as `NoStartDate`), and `RequestRejected` from a
///   malformed request, both fall here. The flatfile driver gives up
///   immediately; no automatic retry.
/// * **Transient** — the request might succeed on a fresh connection
///   (server hop, momentary network blip, mid-stream truncation). The
///   flatfile driver retries with exponential backoff up to the
///   [`crate::config::FlatFilesConfig::max_attempts`] budget before
///   surfacing the error.
///
/// Within the terminal class, [`FlatFilesUnavailableReason::is_no_data`]
/// further distinguishes a *no-data* `DISCONNECTED` (the requested slice
/// simply does not exist for this account/date — the daily snapshot is not
/// yet generated, or the date is outside the entitlement window) from a
/// genuine auth/transport fault. The server maps the former to `404` and
/// the latter to `502`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlatFilesUnavailableReason {
    /// Server sent a `DISCONNECTED` carrying a `RemoveReason` ordinal —
    /// either during login (e.g. `INVALID_CREDENTIALS=0`,
    /// `ACCOUNT_ALREADY_CONNECTED=6`) or mid-stream, when it declines to
    /// serve a requested slice (e.g. `NO_START_DATE=13` for an
    /// out-of-window date whose snapshot does not exist). The retry class
    /// and the no-data-vs-fault distinction are decoded from the ordinal —
    /// see [`FlatFilesUnavailableReason::is_transient`] and
    /// [`FlatFilesUnavailableReason::is_no_data`].
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
    /// `AuthRejected` decodes its `RemoveReason` ordinal through
    /// [`disconnect_reason_class`]: only the genuinely-transient reasons
    /// (timeouts, `ServerRestarting`, rate-limit) retry; permanent
    /// credential/account reasons and the no-data / validation reasons
    /// (`NoStartDate`, `GeneralValidationError`) are terminal. This
    /// classifier is shared by the login and mid-stream `DISCONNECTED`
    /// paths — a mid-stream no-data `DISCONNECTED` must NOT be retried,
    /// unlike a login-phase transient. `RequestRejected` is always
    /// terminal — bad params will not fix themselves on retry.
    /// `StreamTruncated` is always transient.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        match self {
            Self::StreamTruncated { .. } => true,
            Self::RequestRejected { .. } => false,
            Self::AuthRejected { reason_code } => {
                disconnect_reason_class(*reason_code) == DisconnectReasonClass::Transient
            }
        }
    }

    /// Inverse of [`Self::is_transient`]. Provided so callers can keep
    /// the intent line-by-line readable in classifier chains.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        !self.is_transient()
    }

    /// Returns `true` when this is a terminal *no-data* condition: the
    /// upstream answered but no flat file exists for the requested
    /// account/date, rather than a credential/transport fault. The server
    /// maps this to `404 flatfiles_no_data`; every other terminal reason
    /// stays `502`.
    ///
    /// Only an `AuthRejected` whose `RemoveReason` ordinal classifies as
    /// [`DisconnectReasonClass::TerminalNoData`] (`NoStartDate`,
    /// `GeneralValidationError`) is no-data. A `RequestRejected` no-data
    /// condition is carried in the server diagnostic string instead and is
    /// recognised by the server's message classifier, not here.
    #[must_use]
    pub fn is_no_data(&self) -> bool {
        matches!(
            self,
            Self::AuthRejected { reason_code }
                if disconnect_reason_class(*reason_code) == DisconnectReasonClass::TerminalNoData
        )
    }
}

/// Retry/severity class of a `DISCONNECTED` `RemoveReason` on the flat-file
/// path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisconnectReasonClass {
    /// A fresh connection might succeed: timeouts, `ServerRestarting`,
    /// rate-limit, session-token churn, or an unrecognised/sentinel code.
    /// Retried by the flat-file retry ladder.
    Transient,
    /// The requested slice does not exist for this account/date —
    /// `NoStartDate` (out-of-window / snapshot not yet generated) or
    /// `GeneralValidationError`. Not retried; surfaced as a `404` no-data
    /// outcome by the server.
    TerminalNoData,
    /// A permanent credential/account rejection — bad credentials, free
    /// account, account already connected. Not retried; surfaced as a
    /// `502` by the server (a genuine auth fault, not "nothing here").
    TerminalPermanent,
}

/// Classify a flat-file `DISCONNECTED` `RemoveReason` ordinal into its
/// retry/severity class.
///
/// This is the mid-stream / login flat-file classifier — deliberately
/// distinct from [`crate::fpss::session::reconnect_delay`], the streaming
/// reconnect policy. The streaming loop only needs transient-vs-permanent
/// (it never surfaces a no-data slice as `404`), and it treats every
/// non-credential reason as retryable; on the flat-file path that would
/// drive a permanent no-data `DISCONNECTED` (`NoStartDate`) through the
/// full retry ladder before failing, instead of surfacing it immediately
/// as no-data. So no-data / validation reasons are pulled out as their own
/// terminal class here.
///
/// The ordinal is decoded through [`RemoveReason::from_code`] — the single
/// source of the wire mapping — rather than matched as a bare integer, so
/// this stays correct if a wire ordinal ever moves. The `0` sentinel the
/// frame parser substitutes for a too-short payload decodes to
/// `InvalidCredentials` and is classed permanent, matching the prior
/// behaviour.
fn disconnect_reason_class(reason_code: u16) -> DisconnectReasonClass {
    // The flat-file frame parser reads the reason as a big-endian `u16`;
    // `RemoveReason::from_code` takes the canonical `i16`. Every defined
    // ordinal is in `0..=18`; `Unspecified` (`-1` / `0xFFFF`) round-trips
    // through the `as i16` cast and decodes to the transient default.
    match RemoveReason::from_code(reason_code as i16) {
        // No data for this account/date, or the request failed server-side
        // validation: re-running with identical inputs cannot succeed, and
        // it is a "nothing here" outcome rather than an outage.
        RemoveReason::NoStartDate | RemoveReason::GeneralValidationError => {
            DisconnectReasonClass::TerminalNoData
        }
        // Permanent credential / account rejections. Mirrors the permanent
        // set in `crate::fpss::session::reconnect_delay`.
        RemoveReason::InvalidCredentials
        | RemoveReason::InvalidLoginValues
        | RemoveReason::InvalidLoginSize
        | RemoveReason::AccountAlreadyConnected
        | RemoveReason::FreeAccount
        | RemoveReason::ServerUserDoesNotExist
        | RemoveReason::InvalidCredentialsNullUser => DisconnectReasonClass::TerminalPermanent,
        // Everything else is worth a fresh connection: timeouts
        // (`TimedOut`, `LoginTimedOut`), `ServerRestarting`,
        // `TooManyRequests`, session-token churn, client-forced drop, and
        // the `Unspecified` sentinel.
        _ => DisconnectReasonClass::Transient,
    }
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
        // The served set is exactly the documented six datasets.
        assert_eq!(SERVED_DATASETS.len(), 6);
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

    /// A `NoStartDate` (ordinal 13) `DISCONNECTED` is a terminal *no-data*
    /// condition: the requested slice does not exist for this account/date.
    /// It must NOT be retried (`is_transient` false / `is_terminal` true)
    /// and must report as no-data so the server answers `404`, not `502`.
    /// This is the exact regression: the old login-phase classifier treated
    /// every non-credential ordinal as transient, driving a permanent
    /// no-data drop through the full retry ladder to a `502`.
    #[test]
    fn no_start_date_disconnect_is_terminal_no_data() {
        // 13 == RemoveReason::NoStartDate (asserted against the canonical
        // wire mapping below so a moved ordinal fails loudly).
        assert_eq!(RemoveReason::from_code(13), RemoveReason::NoStartDate);
        let reason = FlatFilesUnavailableReason::AuthRejected { reason_code: 13 };
        assert!(!reason.is_transient(), "NoStartDate must not be retried");
        assert!(reason.is_terminal());
        assert!(
            reason.is_no_data(),
            "NoStartDate must surface as a no-data (404) condition"
        );
    }

    /// `GeneralValidationError` (ordinal 3) is the other terminal no-data /
    /// validation reason: re-running with identical inputs cannot succeed,
    /// and it is a "nothing here" outcome, so it is non-transient and
    /// no-data alongside `NoStartDate`.
    #[test]
    fn general_validation_error_disconnect_is_terminal_no_data() {
        assert_eq!(
            RemoveReason::from_code(3),
            RemoveReason::GeneralValidationError
        );
        let reason = FlatFilesUnavailableReason::AuthRejected { reason_code: 3 };
        assert!(!reason.is_transient());
        assert!(reason.is_no_data());
    }

    /// Genuinely-transient `DISCONNECTED` reasons stay retryable and are
    /// NOT no-data, so the retry ladder still reconnects on a fresh session
    /// (a `502` only surfaces once the budget is exhausted). `TimedOut`,
    /// `ServerRestarting`, `TooManyRequests`, and `LoginTimedOut` cover the
    /// timeout / outage / rate-limit classes.
    #[test]
    fn transient_disconnect_reasons_still_retry() {
        for (code, expect) in [
            (4u16, RemoveReason::TimedOut),
            (15, RemoveReason::ServerRestarting),
            (12, RemoveReason::TooManyRequests),
            (14, RemoveReason::LoginTimedOut),
        ] {
            assert_eq!(RemoveReason::from_code(code as i16), expect, "ordinal {code}");
            let reason = FlatFilesUnavailableReason::AuthRejected { reason_code: code };
            assert!(
                reason.is_transient(),
                "{expect:?} (ord {code}) must remain retryable"
            );
            assert!(
                !reason.is_no_data(),
                "{expect:?} (ord {code}) is a transport/outage fault, not no-data"
            );
        }
    }

    /// Permanent credential / account rejections are terminal but NOT
    /// no-data: they are a genuine auth fault, so the server keeps them at
    /// `502`, never `404`. They must not be retried either.
    #[test]
    fn permanent_credential_disconnect_is_terminal_not_no_data() {
        // 0 InvalidCredentials, 1 InvalidLoginValues, 2 InvalidLoginSize,
        // 6 AccountAlreadyConnected, 9 FreeAccount, 17 ServerUserDoesNotExist,
        // 18 InvalidCredentialsNullUser.
        for code in [0u16, 1, 2, 6, 9, 17, 18] {
            let reason = FlatFilesUnavailableReason::AuthRejected { reason_code: code };
            assert!(
                !reason.is_transient(),
                "credential reason {code} must not be retried"
            );
            assert!(
                !reason.is_no_data(),
                "credential reason {code} is an auth fault (502), not no-data (404)"
            );
        }
    }

    /// `is_no_data` is scoped to the `AuthRejected` reason-code path only:
    /// a `RequestRejected` (message-classified on the server) and a
    /// `StreamTruncated` are never reported as no-data here.
    #[test]
    fn is_no_data_is_scoped_to_auth_rejected() {
        assert!(!FlatFilesUnavailableReason::RequestRejected {
            server_message: "NO_START_DATE:...".into(),
        }
        .is_no_data());
        assert!(!FlatFilesUnavailableReason::StreamTruncated {
            bytes_received: 4096,
        }
        .is_no_data());
    }
}
