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

/// Data field types returned in responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum DataType {
    // Core fields
    /// Trading date of the record, encoded `YYYYMMDD`.
    Date = 0,
    /// Timestamp as milliseconds since midnight (exchange time).
    MsOfDay = 1,
    /// Correction indicator flagging revised or cancelled records.
    Correction = 2,
    /// Price-type qualifier for the carried price field.
    PriceType = 4,
    /// Secondary timestamp as milliseconds since midnight (e.g. quote time on a trade record).
    MsOfDay2 = 5,
    /// Reserved / unspecified field.
    Undefined = 6,
    // Quote fields
    /// Number of contracts or shares at the bid.
    BidSize = 101,
    /// Exchange code posting the bid.
    BidExchange = 102,
    /// Bid price.
    Bid = 103,
    /// Quote condition code for the bid.
    BidCondition = 104,
    /// Number of contracts or shares at the ask.
    AskSize = 105,
    /// Exchange code posting the ask.
    AskExchange = 106,
    /// Ask price.
    Ask = 107,
    /// Quote condition code for the ask.
    AskCondition = 108,
    /// Bid/ask midpoint price.
    Midpoint = 111,
    /// Volume-weighted average price.
    Vwap = 112,
    /// Quote-weighted average price.
    Qwap = 113,
    /// Weighted average price.
    Wap = 114,
    // Open interest
    /// Open interest (outstanding contracts).
    OpenInterest = 121,
    // Trade fields
    /// Exchange sequence number of the trade.
    Sequence = 131,
    /// Trade size (contracts or shares).
    Size = 132,
    /// Trade condition code.
    Condition = 133,
    /// Trade price.
    Price = 134,
    /// Exchange code reporting the trade.
    Exchange = 135,
    /// Packed trade condition flags.
    ConditionFlags = 136,
    /// Packed price-qualifier flags.
    PriceFlags = 137,
    /// Volume-type qualifier.
    VolumeType = 138,
    /// Number of records back from the most recent (request offset).
    RecordsBack = 139,
    /// Aggregate volume.
    Volume = 141,
    /// Trade count.
    Count = 142,
    // First-order Greeks
    /// Theta — option price sensitivity to the passage of time.
    Theta = 151,
    /// Vega — option price sensitivity to implied volatility.
    Vega = 152,
    /// Delta — option price sensitivity to the underlying price.
    Delta = 153,
    /// Rho — option price sensitivity to the risk-free interest rate.
    Rho = 154,
    /// Epsilon — option price sensitivity to the dividend yield.
    Epsilon = 155,
    /// Lambda — elasticity (percentage change in option value per percentage change in the underlying).
    Lambda = 156,
    // Second-order Greeks
    /// Gamma — rate of change of delta with respect to the underlying price.
    Gamma = 161,
    /// Vanna — sensitivity of delta to implied volatility (and of vega to the underlying price).
    Vanna = 162,
    /// Charm — rate of change of delta with respect to the passage of time.
    Charm = 163,
    /// Vomma — rate of change of vega with respect to implied volatility.
    Vomma = 164,
    /// Veta — rate of change of vega with respect to the passage of time.
    Veta = 165,
    /// Vera — rate of change of rho with respect to implied volatility.
    Vera = 166,
    /// Second-order partial derivative of option price with respect to strike (dual-strike convexity).
    Sopdk = 167,
    // Third-order Greeks
    /// Speed — rate of change of gamma with respect to the underlying price.
    Speed = 171,
    /// Zomma — rate of change of gamma with respect to implied volatility.
    Zomma = 172,
    /// Color — rate of change of gamma with respect to the passage of time.
    Color = 173,
    /// Ultima — third-order sensitivity of option price to implied volatility.
    Ultima = 174,
    // Black-Scholes internals
    /// Black-Scholes `d1` term.
    D1 = 181,
    /// Black-Scholes `d2` term.
    D2 = 182,
    /// Dual delta — sensitivity of option price to the strike price.
    DualDelta = 183,
    /// Dual gamma — rate of change of dual delta with respect to the strike price.
    DualGamma = 184,
    // OHLC
    /// Opening price of the interval.
    Open = 191,
    /// Highest price of the interval.
    High = 192,
    /// Lowest price of the interval.
    Low = 193,
    /// Closing price of the interval.
    Close = 194,
    /// Net change versus the prior close.
    NetChange = 195,
    // Implied volatility
    /// Implied volatility derived from the trade or mid price.
    ImpliedVol = 201,
    /// Implied volatility derived from the bid price.
    BidImpliedVol = 202,
    /// Implied volatility derived from the ask price.
    AskImpliedVol = 203,
    /// Underlying instrument price used in the calculation.
    UnderlyingPrice = 204,
    /// Implied-volatility solver error / residual.
    IvError = 205,
    // Ratios
    /// Ratio value (e.g. split or adjustment ratio).
    Ratio = 211,
    /// Rating value.
    Rating = 212,
    // Dividends
    /// Ex-dividend date, encoded `YYYYMMDD`.
    ExDate = 221,
    /// Record date, encoded `YYYYMMDD`.
    RecordDate = 222,
    /// Dividend payment date, encoded `YYYYMMDD`.
    PaymentDate = 223,
    /// Dividend announcement date, encoded `YYYYMMDD`.
    AnnDate = 224,
    /// Dividend amount per share.
    DividendAmount = 225,
    /// Reduction amount applied to the dividend.
    LessAmount = 226,
    /// Dividend rate.
    Rate = 230,
    // Extended conditions
    /// First extended trade/quote condition code.
    ExtCondition1 = 241,
    /// Second extended trade/quote condition code.
    ExtCondition2 = 242,
    /// Third extended trade/quote condition code.
    ExtCondition3 = 243,
    /// Fourth extended trade/quote condition code.
    ExtCondition4 = 244,
    // Splits
    /// Split effective date, encoded `YYYYMMDD`.
    SplitDate = 251,
    /// Share count before the split.
    BeforeShares = 252,
    /// Share count after the split.
    AfterShares = 253,
    // Fundamentals
    /// Total outstanding shares.
    OutstandingShares = 261,
    /// Shares held short.
    ShortShares = 262,
    /// Institutional ownership interest.
    InstitutionalInterest = 263,
    /// Last fiscal quarter, encoded `YYYYMMDD`.
    LastFiscalQuarter = 264,
    /// Last fiscal year, encoded `YYYYMMDD`.
    LastFiscalYear = 265,
    /// Total assets.
    Assets = 266,
    /// Total liabilities.
    Liabilities = 267,
    /// Long-term debt.
    LongTermDebt = 268,
    /// Earnings per share, most recent quarter.
    EpsMrq = 269,
    /// Earnings per share, most recent year.
    EpsMry = 270,
    /// Diluted earnings per share.
    EpsDiluted = 271,
    /// Symbol-change effective date, encoded `YYYYMMDD`.
    SymbolChangeDate = 272,
    /// Symbol-change type code.
    SymbolChangeType = 273,
    /// Instrument symbol.
    Symbol = 274,
}

impl DataType {
    /// Resolve a wire field-type code to its variant; `None` for codes outside
    /// the known taxonomy.
    #[must_use]
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Date),
            1 => Some(Self::MsOfDay),
            2 => Some(Self::Correction),
            4 => Some(Self::PriceType),
            5 => Some(Self::MsOfDay2),
            6 => Some(Self::Undefined),
            101 => Some(Self::BidSize),
            102 => Some(Self::BidExchange),
            103 => Some(Self::Bid),
            104 => Some(Self::BidCondition),
            105 => Some(Self::AskSize),
            106 => Some(Self::AskExchange),
            107 => Some(Self::Ask),
            108 => Some(Self::AskCondition),
            111 => Some(Self::Midpoint),
            112 => Some(Self::Vwap),
            113 => Some(Self::Qwap),
            114 => Some(Self::Wap),
            121 => Some(Self::OpenInterest),
            131 => Some(Self::Sequence),
            132 => Some(Self::Size),
            133 => Some(Self::Condition),
            134 => Some(Self::Price),
            135 => Some(Self::Exchange),
            136 => Some(Self::ConditionFlags),
            137 => Some(Self::PriceFlags),
            138 => Some(Self::VolumeType),
            139 => Some(Self::RecordsBack),
            141 => Some(Self::Volume),
            142 => Some(Self::Count),
            151 => Some(Self::Theta),
            152 => Some(Self::Vega),
            153 => Some(Self::Delta),
            154 => Some(Self::Rho),
            155 => Some(Self::Epsilon),
            156 => Some(Self::Lambda),
            161 => Some(Self::Gamma),
            162 => Some(Self::Vanna),
            163 => Some(Self::Charm),
            164 => Some(Self::Vomma),
            165 => Some(Self::Veta),
            166 => Some(Self::Vera),
            167 => Some(Self::Sopdk),
            171 => Some(Self::Speed),
            172 => Some(Self::Zomma),
            173 => Some(Self::Color),
            174 => Some(Self::Ultima),
            181 => Some(Self::D1),
            182 => Some(Self::D2),
            183 => Some(Self::DualDelta),
            184 => Some(Self::DualGamma),
            191 => Some(Self::Open),
            192 => Some(Self::High),
            193 => Some(Self::Low),
            194 => Some(Self::Close),
            195 => Some(Self::NetChange),
            201 => Some(Self::ImpliedVol),
            202 => Some(Self::BidImpliedVol),
            203 => Some(Self::AskImpliedVol),
            204 => Some(Self::UnderlyingPrice),
            205 => Some(Self::IvError),
            211 => Some(Self::Ratio),
            212 => Some(Self::Rating),
            221 => Some(Self::ExDate),
            222 => Some(Self::RecordDate),
            223 => Some(Self::PaymentDate),
            224 => Some(Self::AnnDate),
            225 => Some(Self::DividendAmount),
            226 => Some(Self::LessAmount),
            230 => Some(Self::Rate),
            241 => Some(Self::ExtCondition1),
            242 => Some(Self::ExtCondition2),
            243 => Some(Self::ExtCondition3),
            244 => Some(Self::ExtCondition4),
            251 => Some(Self::SplitDate),
            252 => Some(Self::BeforeShares),
            253 => Some(Self::AfterShares),
            261 => Some(Self::OutstandingShares),
            262 => Some(Self::ShortShares),
            263 => Some(Self::InstitutionalInterest),
            264 => Some(Self::LastFiscalQuarter),
            265 => Some(Self::LastFiscalYear),
            266 => Some(Self::Assets),
            267 => Some(Self::Liabilities),
            268 => Some(Self::LongTermDebt),
            269 => Some(Self::EpsMrq),
            270 => Some(Self::EpsMry),
            271 => Some(Self::EpsDiluted),
            272 => Some(Self::SymbolChangeDate),
            273 => Some(Self::SymbolChangeType),
            274 => Some(Self::Symbol),
            _ => None,
        }
    }

    /// Whether this data type represents a price value (needs Price decoding).
    #[must_use]
    pub fn is_price(&self) -> bool {
        matches!(
            self,
            Self::Bid
                | Self::Ask
                | Self::Midpoint
                | Self::Vwap
                | Self::Qwap
                | Self::Wap
                | Self::Price
                | Self::Theta
                | Self::Vega
                | Self::Delta
                | Self::Rho
                | Self::Epsilon
                | Self::Lambda
                | Self::Gamma
                | Self::Vanna
                | Self::Charm
                | Self::Vomma
                | Self::Veta
                | Self::Vera
                | Self::Sopdk
                | Self::Speed
                | Self::Zomma
                | Self::Color
                | Self::Ultima
                | Self::D1
                | Self::D2
                | Self::DualDelta
                | Self::DualGamma
                | Self::Open
                | Self::High
                | Self::Low
                | Self::Close
                | Self::NetChange
                | Self::ImpliedVol
                | Self::BidImpliedVol
                | Self::AskImpliedVol
                | Self::UnderlyingPrice
                | Self::IvError
                | Self::Ratio
                | Self::Rating
                | Self::DividendAmount
                | Self::LessAmount
                | Self::Rate
                | Self::InstitutionalInterest
                | Self::Assets
                | Self::Liabilities
                | Self::LongTermDebt
                | Self::EpsMrq
                | Self::EpsMry
                | Self::EpsDiluted
        )
    }
}

/// Request type for historical data queries.
///
/// The full wire request taxonomy retained in the data layer. The
/// flat-file path carries its own `crate::flatfiles::ReqType`, so this
/// reference enum has no in-crate caller today; it allows dead code.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ReqType {
    TrailingDiv = 0,
    Eod = 1,
    Rate = 2,
    EodCta = 3,
    EodUtp = 4,
    EodOpra = 5,
    EodOtc = 6,
    EodOtcbb = 7,
    EodTd = 8,
    /// Calculated market value (per-contract): a theoretical price
    /// derived from the real-time bid/ask via a size-imbalance +
    /// spread-aware nudge. This is the request type the snapshot and
    /// stream paths name.
    MarketValue = 9,
    Default = 100,
    Quote = 101,
    Volume = 102,
    OpenInterest = 103,
    Ohlc = 104,
    OhlcQuote = 105,
    Price = 106,
    Fundamental = 107,
    Dividend = 108,
    Quote1Min = 109,
    Trade = 201,
    ImpliedVolatility = 202,
    Greeks = 203,
    Liquidity = 204,
    LiquidityPlus = 205,
    ImpliedVolatilityVerbose = 206,
    TradeQuote = 207,
    EodQuoteGreeks = 208,
    EodTradeGreeks = 209,
    Split = 210,
    EodGreeks = 211,
    SymbolHistory = 212,
    TradeGreeks = 301,
    GreeksSecondOrder = 302,
    GreeksThirdOrder = 303,
    AltCalcs = 304,
    TradeGreeksSecondOrder = 305,
    TradeGreeksThirdOrder = 306,
    AllGreeks = 307,
    AllTradeGreeks = 308,
}

impl ReqType {
    /// Upper-snake symbolic name for the request type (`"QUOTE"`, `"TRADE"`,
    /// …), falling back to `"DEFAULT"` for unmapped variants.
    #[must_use]
    #[allow(dead_code)] // Reason: paired with the reference `ReqType` enum above; no in-crate caller today.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Eod => "EOD",
            Self::MarketValue => "MARKET_VALUE",
            Self::Quote => "QUOTE",
            Self::Trade => "TRADE",
            Self::Ohlc => "OHLC",
            Self::Greeks => "GREEKS",
            Self::OpenInterest => "OPEN_INTEREST",
            Self::ImpliedVolatility => "IMPLIED_VOLATILITY",
            Self::TradeQuote => "TRADE_QUOTE",
            Self::TradeGreeks => "TRADE_GREEKS",
            Self::AllGreeks => "ALL_GREEKS",
            Self::AllTradeGreeks => "ALL_TRADE_GREEKS",
            _ => "DEFAULT",
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
    use super::{RemoveReason, StreamResponseType};

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
