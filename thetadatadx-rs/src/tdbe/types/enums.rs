//! Wire enum taxonomy for the FPSS data layer.
//!
//! Each enum mirrors a numeric (or short-text) code carried on the wire and
//! pairs a `from_code` resolver with an `as_str` symbolic form for the dynamic
//! bindings. `from_code` constructors stay strict — unknown codes return
//! `None` (or a documented sentinel) so the decoder fails loudly on schema
//! drift rather than mis-classifying.

/// Security type identifier.
///
/// `Unknown` is a sentinel for contracts whose shape has not yet been resolved.
/// The FPSS decoder uses it for the empty-contract placeholder that flows on
/// data events arriving before their `ContractAssigned` frame — downstream
/// consumers can pattern-match `sec_type == SecType::Unknown` instead of
/// relying on `contract.symbol.is_empty()`. `Unknown` has no wire-protocol
/// representation: [`SecType::from_code`] never returns it, and it is not
/// serialized in subscribe payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SecType {
    /// Equity / common stock contract.
    Stock = 0,
    /// Listed option contract (call or put on an underlying).
    Option = 1,
    /// Index instrument (e.g. SPX, VIX).
    Index = 2,
    /// Interest-rate instrument.
    Rate = 3,
    /// Unresolved contract shape (client-side sentinel, never sent over the wire).
    Unknown = -1,
}

impl SecType {
    /// Resolve a wire security-type code to its variant; `None` for unknown
    /// codes (including the `Unknown` sentinel, which has no wire form).
    #[must_use]
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Stock),
            1 => Some(Self::Option),
            2 => Some(Self::Index),
            3 => Some(Self::Rate),
            // `Unknown` has no wire representation — it is synthesized
            // client-side only. Returning `None` keeps the wire-protocol
            // parser strict.
            _ => None,
        }
    }

    /// Upper-case symbolic name (`"STOCK"`, `"OPTION"`, …) for binding surfaces.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stock => "STOCK",
            Self::Option => "OPTION",
            Self::Index => "INDEX",
            Self::Rate => "RATE",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Streaming message types for real-time data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum StreamMsgType {
    /// Client-to-server credentials handshake message.
    Credentials = 0,
    /// Session-token handshake message.
    SessionToken = 1,
    /// Informational server message.
    Info = 2,
    /// Stream metadata message.
    Metadata = 3,
    /// Connection-established acknowledgement.
    Connected = 4,
    /// Keep-alive ping.
    Ping = 10,
    /// Server error notification.
    Error = 11,
    /// Connection-lost notification.
    Disconnected = 12,
    /// Connection-reestablished notification.
    Reconnected = 13,
    /// Contract definition message.
    Contract = 20,
    /// Quote update message.
    Quote = 21,
    /// Trade update message.
    Trade = 22,
    /// Open-interest update message.
    OpenInterest = 23,
    /// OHLCVC bar message (open, high, low, close, volume, count).
    Ohlcvc = 24,
    /// Calculated market-value update message.
    MarketValue = 25,
    /// Stream-start control message.
    Start = 30,
    /// Stream-restart control message.
    Restart = 31,
    /// Stream-stop control message.
    Stop = 32,
    /// Request-response message.
    ReqResponse = 40,
    /// Quote-subscription removal message.
    RemoveQuote = 51,
    /// Trade-subscription removal message.
    RemoveTrade = 52,
    /// Open-interest-subscription removal message.
    RemoveOpenInterest = 53,
    /// Market-value-subscription removal message.
    RemoveMarketValue = 54,
}

impl StreamMsgType {
    /// Resolve a wire stream-message code to its variant; `None` for unknown
    /// codes.
    #[inline]
    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Credentials),
            1 => Some(Self::SessionToken),
            2 => Some(Self::Info),
            3 => Some(Self::Metadata),
            4 => Some(Self::Connected),
            10 => Some(Self::Ping),
            11 => Some(Self::Error),
            12 => Some(Self::Disconnected),
            13 => Some(Self::Reconnected),
            20 => Some(Self::Contract),
            21 => Some(Self::Quote),
            22 => Some(Self::Trade),
            23 => Some(Self::OpenInterest),
            24 => Some(Self::Ohlcvc),
            25 => Some(Self::MarketValue),
            30 => Some(Self::Start),
            31 => Some(Self::Restart),
            32 => Some(Self::Stop),
            40 => Some(Self::ReqResponse),
            51 => Some(Self::RemoveQuote),
            52 => Some(Self::RemoveTrade),
            53 => Some(Self::RemoveOpenInterest),
            54 => Some(Self::RemoveMarketValue),
            _ => None,
        }
    }
}

/// Streaming subscription response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StreamResponseType {
    /// Subscription accepted and active.
    Subscribed = 0,
    /// Subscription failed with an error.
    Error = 1,
    /// Subscription rejected: the account's maximum concurrent stream count was reached.
    MaxStreamsReached = 2,
    /// Subscription rejected: the account lacks permission for the requested data.
    InvalidPerms = 3,
}

impl StreamResponseType {
    /// Stream-request verification token this outcome is published as on
    /// the WebSocket wire.
    ///
    /// The wire vocabulary is fixed by the stream-verification contract
    /// every client matches on: `SUBSCRIBED` / `ERROR` /
    /// `MAX_STREAMS_REACHED` / `INVALID_PERMS`. This mapping is the single
    /// source of those tokens, so the Rust variant identifier can never
    /// reach the wire — emitting the debug form of the variant would break
    /// every client written against the documented vocabulary.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Subscribed => "SUBSCRIBED",
            Self::Error => "ERROR",
            Self::MaxStreamsReached => "MAX_STREAMS_REACHED",
            Self::InvalidPerms => "INVALID_PERMS",
        }
    }
}

impl core::fmt::Display for StreamResponseType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_wire_str())
    }
}

/// Disconnect reason codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum RemoveReason {
    /// No reason supplied; also the fallback for unrecognized wire codes.
    Unspecified = -1,
    /// Supplied credentials were invalid.
    InvalidCredentials = 0,
    /// Login payload contained invalid values.
    InvalidLoginValues = 1,
    /// Login payload had an invalid size.
    InvalidLoginSize = 2,
    /// Login failed general server-side validation.
    GeneralValidationError = 3,
    /// Connection timed out.
    TimedOut = 4,
    /// Client requested the disconnect.
    ClientForcedDisconnect = 5,
    /// The account is already connected on another session.
    AccountAlreadyConnected = 6,
    /// The session token has expired.
    SessionTokenExpired = 7,
    /// The supplied session token was invalid.
    InvalidSessionToken = 8,
    /// The account is a free account without access to this stream.
    FreeAccount = 9,
    /// The account exceeded its request-rate limit.
    TooManyRequests = 12,
    /// The request omitted a required start date.
    NoStartDate = 13,
    /// The login handshake timed out.
    LoginTimedOut = 14,
    /// The server is restarting.
    ServerRestarting = 15,
    /// The session token was not found server-side.
    SessionTokenNotFound = 16,
    /// The user account does not exist on the server.
    ServerUserDoesNotExist = 17,
    /// Invalid credentials: the user was null.
    InvalidCredentialsNullUser = 18,
}

impl RemoveReason {
    /// Resolve a wire-level i16 code to the typed variant. Returns
    /// `RemoveReason::Unspecified` for unknown codes so callers stay
    /// total without having to handle `Option`.
    #[must_use]
    pub fn from_code(code: i16) -> Self {
        match code {
            0 => Self::InvalidCredentials,
            1 => Self::InvalidLoginValues,
            2 => Self::InvalidLoginSize,
            3 => Self::GeneralValidationError,
            4 => Self::TimedOut,
            5 => Self::ClientForcedDisconnect,
            6 => Self::AccountAlreadyConnected,
            7 => Self::SessionTokenExpired,
            8 => Self::InvalidSessionToken,
            9 => Self::FreeAccount,
            12 => Self::TooManyRequests,
            13 => Self::NoStartDate,
            14 => Self::LoginTimedOut,
            15 => Self::ServerRestarting,
            16 => Self::SessionTokenNotFound,
            17 => Self::ServerUserDoesNotExist,
            18 => Self::InvalidCredentialsNullUser,
            _ => Self::Unspecified,
        }
    }

    /// Symbolic UpperCamelCase name (`"TooManyRequests"`,
    /// `"InvalidCredentials"`, …). Used by Python and TypeScript
    /// bindings to surface the wire i16 as a readable string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unspecified => "Unspecified",
            Self::InvalidCredentials => "InvalidCredentials",
            Self::InvalidLoginValues => "InvalidLoginValues",
            Self::InvalidLoginSize => "InvalidLoginSize",
            Self::GeneralValidationError => "GeneralValidationError",
            Self::TimedOut => "TimedOut",
            Self::ClientForcedDisconnect => "ClientForcedDisconnect",
            Self::AccountAlreadyConnected => "AccountAlreadyConnected",
            Self::SessionTokenExpired => "SessionTokenExpired",
            Self::InvalidSessionToken => "InvalidSessionToken",
            Self::FreeAccount => "FreeAccount",
            Self::TooManyRequests => "TooManyRequests",
            Self::NoStartDate => "NoStartDate",
            Self::LoginTimedOut => "LoginTimedOut",
            Self::ServerRestarting => "ServerRestarting",
            Self::SessionTokenNotFound => "SessionTokenNotFound",
            Self::ServerUserDoesNotExist => "ServerUserDoesNotExist",
            Self::InvalidCredentialsNullUser => "InvalidCredentialsNullUser",
        }
    }

    /// Stable SCREAMING_SNAKE_CASE token published as the disconnect
    /// `reason` on the WebSocket wire.
    ///
    /// This mapping is the single source of the wire reason vocabulary, so
    /// the Rust variant identifier can never reach a client: emitting the
    /// debug form of the variant would leak an internal type name in place
    /// of the documented status token.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Unspecified => "UNSPECIFIED",
            Self::InvalidCredentials => "INVALID_CREDENTIALS",
            Self::InvalidLoginValues => "INVALID_LOGIN_VALUES",
            Self::InvalidLoginSize => "INVALID_LOGIN_SIZE",
            Self::GeneralValidationError => "GENERAL_VALIDATION_ERROR",
            Self::TimedOut => "TIMED_OUT",
            Self::ClientForcedDisconnect => "CLIENT_FORCED_DISCONNECT",
            Self::AccountAlreadyConnected => "ACCOUNT_ALREADY_CONNECTED",
            Self::SessionTokenExpired => "SESSION_TOKEN_EXPIRED",
            Self::InvalidSessionToken => "INVALID_SESSION_TOKEN",
            Self::FreeAccount => "FREE_ACCOUNT",
            Self::TooManyRequests => "TOO_MANY_REQUESTS",
            Self::NoStartDate => "NO_START_DATE",
            Self::LoginTimedOut => "LOGIN_TIMED_OUT",
            Self::ServerRestarting => "SERVER_RESTARTING",
            Self::SessionTokenNotFound => "SESSION_TOKEN_NOT_FOUND",
            Self::ServerUserDoesNotExist => "SERVER_USER_DOES_NOT_EXIST",
            Self::InvalidCredentialsNullUser => "INVALID_CREDENTIALS_NULL_USER",
        }
    }
}

/// Market-calendar day classification.
///
/// Carries the vendor's own day-type vocabulary: the calendar wire
/// sends a text `type` column with exactly these four values, and the
/// decoder maps them one-for-one onto this enum (unknown text fails
/// decode loudly, so the enum is total over the wire vocabulary).
/// `#[repr(i32)]` keeps the C ABI field a plain `int32_t`; the
/// discriminants are the stable cross-binding codes.
///
/// String forms (via [`CalendarStatus::as_str`]) are the values the
/// Python / TypeScript bindings, Arrow columns, and the HTTP server
/// surface: `"open"`, `"early_close"`, `"full_close"`, `"weekend"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum CalendarStatus {
    /// Normal trading day.
    Open = 0,
    /// Trading day with an early close (e.g. day after Thanksgiving).
    EarlyClose = 1,
    /// Market closed for a holiday.
    FullClose = 2,
    /// Weekend.
    Weekend = 3,
}

impl CalendarStatus {
    /// Vendor vocabulary for this day type — the exact text the
    /// calendar wire sends (`"open"` / `"early_close"` / `"full_close"`
    /// / `"weekend"`). This is the string form every dynamic binding
    /// and the HTTP server emit.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::EarlyClose => "early_close",
            Self::FullClose => "full_close",
            Self::Weekend => "weekend",
        }
    }

    /// Parse the vendor's day-type text. Returns `None` for any value
    /// outside the documented vocabulary so decoders can fail loudly
    /// instead of mis-classifying schema drift.
    #[must_use]
    pub fn from_wire_text(text: &str) -> Option<Self> {
        match text {
            "open" => Some(Self::Open),
            "early_close" => Some(Self::EarlyClose),
            "full_close" => Some(Self::FullClose),
            "weekend" => Some(Self::Weekend),
            _ => None,
        }
    }

    /// Resolve a wire-level integer code to the typed variant. Returns
    /// `None` for codes outside `0..=3`.
    #[must_use]
    pub const fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Open),
            1 => Some(Self::EarlyClose),
            2 => Some(Self::FullClose),
            3 => Some(Self::Weekend),
            _ => None,
        }
    }

    /// `true` when the market trades at all on this day type (`Open`
    /// or `EarlyClose`).
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open | Self::EarlyClose)
    }
}

impl std::fmt::Display for CalendarStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

include!("generated/enums_endpoint.rs");

#[cfg(test)]
mod wire_token_tests {
    use super::{RemoveReason, SecType, StreamResponseType};

    /// Every security type maps to its exact upper-case symbolic token.
    /// This is the token the WebSocket contract payload and the Python /
    /// TypeScript fluent surfaces publish, so a drift here would leak the
    /// Rust variant identifier onto a client surface.
    #[test]
    fn sec_type_wire_tokens_are_exact() {
        for (sec, token) in [
            (SecType::Stock, "STOCK"),
            (SecType::Option, "OPTION"),
            (SecType::Index, "INDEX"),
            (SecType::Rate, "RATE"),
            (SecType::Unknown, "UNKNOWN"),
        ] {
            assert_eq!(sec.as_str(), token, "{sec:?}");
        }
    }

    /// Every stream-response outcome maps to its exact stream-verification
    /// token. This is the single source the WebSocket surface publishes,
    /// so a drift here would leak the Rust variant name onto the wire and
    /// break clients matching on the documented vocabulary.
    #[test]
    fn stream_response_wire_tokens_are_exact() {
        assert_eq!(StreamResponseType::Subscribed.as_wire_str(), "SUBSCRIBED");
        assert_eq!(StreamResponseType::Error.as_wire_str(), "ERROR");
        assert_eq!(
            StreamResponseType::MaxStreamsReached.as_wire_str(),
            "MAX_STREAMS_REACHED"
        );
        assert_eq!(
            StreamResponseType::InvalidPerms.as_wire_str(),
            "INVALID_PERMS"
        );
        // `Display` routes through the same mapping.
        assert_eq!(StreamResponseType::Subscribed.to_string(), "SUBSCRIBED");
    }

    /// Every disconnect reason maps to a stable SCREAMING_SNAKE token; the
    /// Rust variant identifier must never reach the wire.
    #[test]
    fn remove_reason_wire_tokens_are_exact() {
        for (reason, token) in [
            (RemoveReason::Unspecified, "UNSPECIFIED"),
            (RemoveReason::InvalidCredentials, "INVALID_CREDENTIALS"),
            (RemoveReason::InvalidLoginValues, "INVALID_LOGIN_VALUES"),
            (RemoveReason::InvalidLoginSize, "INVALID_LOGIN_SIZE"),
            (
                RemoveReason::GeneralValidationError,
                "GENERAL_VALIDATION_ERROR",
            ),
            (RemoveReason::TimedOut, "TIMED_OUT"),
            (
                RemoveReason::ClientForcedDisconnect,
                "CLIENT_FORCED_DISCONNECT",
            ),
            (
                RemoveReason::AccountAlreadyConnected,
                "ACCOUNT_ALREADY_CONNECTED",
            ),
            (RemoveReason::SessionTokenExpired, "SESSION_TOKEN_EXPIRED"),
            (RemoveReason::InvalidSessionToken, "INVALID_SESSION_TOKEN"),
            (RemoveReason::FreeAccount, "FREE_ACCOUNT"),
            (RemoveReason::TooManyRequests, "TOO_MANY_REQUESTS"),
            (RemoveReason::NoStartDate, "NO_START_DATE"),
            (RemoveReason::LoginTimedOut, "LOGIN_TIMED_OUT"),
            (RemoveReason::ServerRestarting, "SERVER_RESTARTING"),
            (
                RemoveReason::SessionTokenNotFound,
                "SESSION_TOKEN_NOT_FOUND",
            ),
            (
                RemoveReason::ServerUserDoesNotExist,
                "SERVER_USER_DOES_NOT_EXIST",
            ),
            (
                RemoveReason::InvalidCredentialsNullUser,
                "INVALID_CREDENTIALS_NULL_USER",
            ),
        ] {
            assert_eq!(reason.as_wire_str(), token, "{reason:?}");
        }
    }
}
