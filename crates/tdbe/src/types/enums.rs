/// Security type identifier.
///
/// `Unknown` is a sentinel for contracts whose shape has not yet been resolved.
/// The FPSS decoder uses it for the empty-contract placeholder that flows on
/// data events arriving before their `ContractAssigned` frame — downstream
/// consumers can pattern-match `sec_type == SecType::Unknown` instead of
/// relying on `contract.root.is_empty()`. `Unknown` has no wire-protocol
/// representation: [`SecType::from_code`] never returns it, and it is not
/// serialized in subscribe payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SecType {
    Stock = 0,
    Option = 1,
    Index = 2,
    Rate = 3,
    /// Unresolved contract shape (client-side sentinel, never sent over the wire).
    Unknown = -1,
}

impl SecType {
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
    Date = 0,
    MsOfDay = 1,
    Correction = 2,
    PriceType = 4,
    MsOfDay2 = 5,
    Undefined = 6,
    // Quote fields
    BidSize = 101,
    BidExchange = 102,
    Bid = 103,
    BidCondition = 104,
    AskSize = 105,
    AskExchange = 106,
    Ask = 107,
    AskCondition = 108,
    Midpoint = 111,
    Vwap = 112,
    Qwap = 113,
    Wap = 114,
    // Open interest
    OpenInterest = 121,
    // Trade fields
    Sequence = 131,
    Size = 132,
    Condition = 133,
    Price = 134,
    Exchange = 135,
    ConditionFlags = 136,
    PriceFlags = 137,
    VolumeType = 138,
    RecordsBack = 139,
    Volume = 141,
    Count = 142,
    // First-order Greeks
    Theta = 151,
    Vega = 152,
    Delta = 153,
    Rho = 154,
    Epsilon = 155,
    Lambda = 156,
    // Second-order Greeks
    Gamma = 161,
    Vanna = 162,
    Charm = 163,
    Vomma = 164,
    Veta = 165,
    Vera = 166,
    Sopdk = 167,
    // Third-order Greeks
    Speed = 171,
    Zomma = 172,
    Color = 173,
    Ultima = 174,
    // Black-Scholes internals
    D1 = 181,
    D2 = 182,
    DualDelta = 183,
    DualGamma = 184,
    // OHLC
    Open = 191,
    High = 192,
    Low = 193,
    Close = 194,
    NetChange = 195,
    // Implied volatility
    ImpliedVol = 201,
    BidImpliedVol = 202,
    AskImpliedVol = 203,
    UnderlyingPrice = 204,
    IvError = 205,
    // Ratios
    Ratio = 211,
    Rating = 212,
    // Dividends
    ExDate = 221,
    RecordDate = 222,
    PaymentDate = 223,
    AnnDate = 224,
    DividendAmount = 225,
    LessAmount = 226,
    Rate = 230,
    // Extended conditions
    ExtCondition1 = 241,
    ExtCondition2 = 242,
    ExtCondition3 = 243,
    ExtCondition4 = 244,
    // Splits
    SplitDate = 251,
    BeforeShares = 252,
    AfterShares = 253,
    // Fundamentals
    OutstandingShares = 261,
    ShortShares = 262,
    InstitutionalInterest = 263,
    LastFiscalQuarter = 264,
    LastFiscalYear = 265,
    Assets = 266,
    Liabilities = 267,
    LongTermDebt = 268,
    EpsMrq = 269,
    EpsMry = 270,
    EpsDiluted = 271,
    SymbolChangeDate = 272,
    SymbolChangeType = 273,
    Symbol = 274,
}

impl DataType {
    #[must_use]
    pub fn from_code(code: i32) -> Option<Self> {
        // Generated from the Java enum
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
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Eod => "EOD",
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
    Credentials = 0,
    SessionToken = 1,
    Info = 2,
    Metadata = 3,
    Connected = 4,
    Ping = 10,
    Error = 11,
    Disconnected = 12,
    Reconnected = 13,
    Contract = 20,
    Quote = 21,
    Trade = 22,
    OpenInterest = 23,
    Ohlcvc = 24,
    Start = 30,
    Restart = 31,
    Stop = 32,
    ReqResponse = 40,
    RemoveQuote = 51,
    RemoveTrade = 52,
    RemoveOpenInterest = 53,
}

impl StreamMsgType {
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
            30 => Some(Self::Start),
            31 => Some(Self::Restart),
            32 => Some(Self::Stop),
            40 => Some(Self::ReqResponse),
            51 => Some(Self::RemoveQuote),
            52 => Some(Self::RemoveTrade),
            53 => Some(Self::RemoveOpenInterest),
            _ => None,
        }
    }
}

/// Streaming subscription response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StreamResponseType {
    Subscribed = 0,
    Error = 1,
    MaxStreamsReached = 2,
    InvalidPerms = 3,
}

/// Disconnect reason codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum RemoveReason {
    Unspecified = -1,
    InvalidCredentials = 0,
    InvalidLoginValues = 1,
    InvalidLoginSize = 2,
    GeneralValidationError = 3,
    TimedOut = 4,
    ClientForcedDisconnect = 5,
    AccountAlreadyConnected = 6,
    SessionTokenExpired = 7,
    InvalidSessionToken = 8,
    FreeAccount = 9,
    TooManyRequests = 12,
    NoStartDate = 13,
    LoginTimedOut = 14,
    ServerRestarting = 15,
    SessionTokenNotFound = 16,
    ServerUserDoesNotExist = 17,
    InvalidCredentialsNullUser = 18,
}

/// Option right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Right {
    Call,
    Put,
    Both,
}

impl Right {
    #[must_use]
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'C' | 'c' => Some(Self::Call),
            'P' | 'p' => Some(Self::Put),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_char(&self) -> char {
        match self {
            Self::Call => 'C',
            Self::Put => 'P',
            Self::Both => '*',
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Put => "put",
            Self::Both => "both",
        }
    }
}

/// Data venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Venue {
    Nqb,
    UtpCta,
}

impl Venue {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nqb => "NQB",
            Self::UtpCta => "UTP_CTA",
        }
    }
}

/// Interest rate type for Greeks calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateType {
    Sofr,
    TreasuryM1,
    TreasuryM3,
    TreasuryM6,
    TreasuryY1,
    TreasuryY2,
    TreasuryY3,
    TreasuryY5,
    TreasuryY7,
    TreasuryY10,
    TreasuryY20,
    TreasuryY30,
}

impl RateType {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sofr => "SOFR",
            Self::TreasuryM1 => "TREASURY_M1",
            Self::TreasuryM3 => "TREASURY_M3",
            Self::TreasuryM6 => "TREASURY_M6",
            Self::TreasuryY1 => "TREASURY_Y1",
            Self::TreasuryY2 => "TREASURY_Y2",
            Self::TreasuryY3 => "TREASURY_Y3",
            Self::TreasuryY5 => "TREASURY_Y5",
            Self::TreasuryY7 => "TREASURY_Y7",
            Self::TreasuryY10 => "TREASURY_Y10",
            Self::TreasuryY20 => "TREASURY_Y20",
            Self::TreasuryY30 => "TREASURY_Y30",
        }
    }
}

/// Historical request type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestType {
    Trade,
    Quote,
    Eod,
    Ohlc,
}

impl RequestType {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Trade => "trade",
            Self::Quote => "quote",
            Self::Eod => "eod",
            Self::Ohlc => "ohlc",
        }
    }
}

/// Historical interval preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Interval {
    Tick,
    Ms10,
    Ms100,
    Ms500,
    S1,
    S5,
    S10,
    S15,
    S30,
    M1,
    M5,
    M10,
    M15,
    M30,
    H1,
}

impl Interval {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tick => "tick",
            Self::Ms10 => "10ms",
            Self::Ms100 => "100ms",
            Self::Ms500 => "500ms",
            Self::S1 => "1s",
            Self::S5 => "5s",
            Self::S10 => "10s",
            Self::S15 => "15s",
            Self::S30 => "30s",
            Self::M1 => "1m",
            Self::M5 => "5m",
            Self::M10 => "10m",
            Self::M15 => "15m",
            Self::M30 => "30m",
            Self::H1 => "1h",
        }
    }
}

/// Endpoint version selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Version {
    Latest,
    V1,
}

impl Version {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Latest => "latest",
            Self::V1 => "1",
        }
    }
}
