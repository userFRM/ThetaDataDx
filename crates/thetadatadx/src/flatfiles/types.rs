//! Public enums for the FLATFILES surface.

use std::fmt;

/// Security types accepted by the FLATFILES route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecType {
    Option,
    Stock,
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
    Eod = 1,
    Quote = 101,
    OpenInterest = 103,
    Ohlc = 104,
    Trade = 201,
    TradeQuote = 207,
}

impl ReqType {
    pub(crate) fn as_wire(self) -> u32 {
        self as u32
    }
}

/// Reason a [`ThetaDataDx::flatfile_request`](crate::ThetaDataDx::flatfile_request)
/// call cannot return CSV.
///
/// Returned inside `Error::FlatFilesUnavailable` so callers can decide
/// whether to fall back to the V3 fan-out path or to retry later.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlatFilesUnavailableReason {
    /// Server returned a `RemoveReason` ordinal during auth (e.g.
    /// `INVALID_CREDENTIALS=1`, `ACCOUNT_ALREADY_CONNECTED=7`).
    AuthRejected { reason_code: u16 },
    /// Server replied with an `ERROR` frame to the FLAT_FILE request itself
    /// (e.g. `INVALID_PARAMS:Invalid request type`).
    RequestRejected { server_message: String },
    /// Connection dropped before the response completed.
    StreamTruncated { bytes_received: u64 },
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
