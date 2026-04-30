//! Vendor `DataType` codes and their wire-format properties.
//!
//! The first FLATFILES chunk header encodes the per-column schema as an
//! array of `i32` codes — one per column — describing the FIT-decoded row
//! layout. The codes are stable identifiers from the vendor's
//! `net.thetadata.enums.DataType` enum (e.g. `MS_OF_DAY=1`, `OPEN_INTEREST=121`).
//!
//! For each code we need three pieces of information when emitting CSV
//! or JSONL:
//!
//! - `name` — lowercase column header in the vendor's CSV (e.g. `ms_of_day`).
//! - `is_price` — whether the integer field is divided by `10^price_type`
//!   to reconstruct a fractional price. The PRICE_TYPE column itself
//!   carries the exponent N for the row.
//! - `is_str` / `is_date` — informational; date columns are emitted as
//!   plain integers (e.g. `20260428`), the way the vendor jar does.
//!
//! No constants beyond the public-facing enum surface are reproduced here.
//! The full vendor enum is large (~90 entries); this module ships the
//! subset that appears in the four sec-types × five req-types FLATFILES
//! produces, plus a forward-compat fallback for unknown codes.

/// Wire-format DataType identifier.
///
/// `code()` is the i32 the server emits in the per-chunk header.
/// Unknown codes are tolerated as `Unknown(code)` — a forward-compat
/// hatch so a server-side schema extension does not break decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DataType {
    Date,
    MsOfDay,
    Correction,
    PriceType,
    MsOfDay2,
    BidSize,
    BidExchange,
    Bid,
    BidCondition,
    AskSize,
    AskExchange,
    Ask,
    AskCondition,
    Midpoint,
    Vwap,
    Qwap,
    Wap,
    OpenInterest,
    Sequence,
    Size,
    Condition,
    Price,
    Exchange,
    ConditionFlags,
    PriceFlags,
    VolumeType,
    RecordsBack,
    Volume,
    Count,
    Open,
    High,
    Low,
    Close,
    NetChange,
    ImpliedVol,
    BidImpliedVol,
    AskImpliedVol,
    UnderlyingPrice,
    IvError,
    ExtCondition1,
    ExtCondition2,
    ExtCondition3,
    ExtCondition4,
    /// Forward-compat fallback. Renders as `unknown_<code>` in CSV, with
    /// `is_price = false` so we never accidentally divide by PRICE_TYPE.
    Unknown(i32),
}

impl DataType {
    /// Resolve from the wire i32.
    pub(crate) fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Date,
            1 => Self::MsOfDay,
            2 => Self::Correction,
            4 => Self::PriceType,
            5 => Self::MsOfDay2,
            101 => Self::BidSize,
            102 => Self::BidExchange,
            103 => Self::Bid,
            104 => Self::BidCondition,
            105 => Self::AskSize,
            106 => Self::AskExchange,
            107 => Self::Ask,
            108 => Self::AskCondition,
            111 => Self::Midpoint,
            112 => Self::Vwap,
            113 => Self::Qwap,
            114 => Self::Wap,
            121 => Self::OpenInterest,
            131 => Self::Sequence,
            132 => Self::Size,
            133 => Self::Condition,
            134 => Self::Price,
            135 => Self::Exchange,
            136 => Self::ConditionFlags,
            137 => Self::PriceFlags,
            138 => Self::VolumeType,
            139 => Self::RecordsBack,
            141 => Self::Volume,
            142 => Self::Count,
            191 => Self::Open,
            192 => Self::High,
            193 => Self::Low,
            194 => Self::Close,
            195 => Self::NetChange,
            201 => Self::ImpliedVol,
            202 => Self::BidImpliedVol,
            203 => Self::AskImpliedVol,
            204 => Self::UnderlyingPrice,
            205 => Self::IvError,
            241 => Self::ExtCondition1,
            242 => Self::ExtCondition2,
            243 => Self::ExtCondition3,
            244 => Self::ExtCondition4,
            other => Self::Unknown(other),
        }
    }

    /// Lowercase column name used in the vendor's CSV header and in JSON
    /// keys. Matches the vendor's `Enum.name().toLowerCase()` exactly.
    pub(crate) fn name(self) -> std::borrow::Cow<'static, str> {
        use std::borrow::Cow;
        match self {
            Self::Date => Cow::Borrowed("date"),
            Self::MsOfDay => Cow::Borrowed("ms_of_day"),
            Self::Correction => Cow::Borrowed("correction"),
            Self::PriceType => Cow::Borrowed("price_type"),
            Self::MsOfDay2 => Cow::Borrowed("ms_of_day2"),
            Self::BidSize => Cow::Borrowed("bid_size"),
            Self::BidExchange => Cow::Borrowed("bid_exchange"),
            Self::Bid => Cow::Borrowed("bid"),
            Self::BidCondition => Cow::Borrowed("bid_condition"),
            Self::AskSize => Cow::Borrowed("ask_size"),
            Self::AskExchange => Cow::Borrowed("ask_exchange"),
            Self::Ask => Cow::Borrowed("ask"),
            Self::AskCondition => Cow::Borrowed("ask_condition"),
            Self::Midpoint => Cow::Borrowed("midpoint"),
            Self::Vwap => Cow::Borrowed("vwap"),
            Self::Qwap => Cow::Borrowed("qwap"),
            Self::Wap => Cow::Borrowed("wap"),
            Self::OpenInterest => Cow::Borrowed("open_interest"),
            Self::Sequence => Cow::Borrowed("sequence"),
            Self::Size => Cow::Borrowed("size"),
            Self::Condition => Cow::Borrowed("condition"),
            Self::Price => Cow::Borrowed("price"),
            Self::Exchange => Cow::Borrowed("exchange"),
            Self::ConditionFlags => Cow::Borrowed("condition_flags"),
            Self::PriceFlags => Cow::Borrowed("price_flags"),
            Self::VolumeType => Cow::Borrowed("volume_type"),
            Self::RecordsBack => Cow::Borrowed("records_back"),
            Self::Volume => Cow::Borrowed("volume"),
            Self::Count => Cow::Borrowed("count"),
            Self::Open => Cow::Borrowed("open"),
            Self::High => Cow::Borrowed("high"),
            Self::Low => Cow::Borrowed("low"),
            Self::Close => Cow::Borrowed("close"),
            Self::NetChange => Cow::Borrowed("net_change"),
            Self::ImpliedVol => Cow::Borrowed("implied_vol"),
            Self::BidImpliedVol => Cow::Borrowed("bid_implied_vol"),
            Self::AskImpliedVol => Cow::Borrowed("ask_implied_vol"),
            Self::UnderlyingPrice => Cow::Borrowed("underlying_price"),
            Self::IvError => Cow::Borrowed("iv_error"),
            Self::ExtCondition1 => Cow::Borrowed("ext_condition1"),
            Self::ExtCondition2 => Cow::Borrowed("ext_condition2"),
            Self::ExtCondition3 => Cow::Borrowed("ext_condition3"),
            Self::ExtCondition4 => Cow::Borrowed("ext_condition4"),
            Self::Unknown(c) => Cow::Owned(format!("unknown_{c}")),
        }
    }

    /// Whether the column carries a fractional price encoded as
    /// `int_value / 10^price_type`. Mirrors the vendor's
    /// `DataType.isPrice()` flag.
    pub(crate) fn is_price(self) -> bool {
        matches!(
            self,
            Self::Bid
                | Self::Ask
                | Self::Midpoint
                | Self::Vwap
                | Self::Qwap
                | Self::Wap
                | Self::Price
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
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_interest_schema_decodes_known_codes() {
        // Option open_interest schema observed live: ms_of_day, open_interest, date.
        assert_eq!(DataType::from_code(1), DataType::MsOfDay);
        assert_eq!(DataType::from_code(121), DataType::OpenInterest);
        assert_eq!(DataType::from_code(0), DataType::Date);
    }

    #[test]
    fn price_columns_flagged() {
        assert!(DataType::Bid.is_price());
        assert!(DataType::Ask.is_price());
        assert!(DataType::Price.is_price());
        assert!(!DataType::Size.is_price());
        assert!(!DataType::OpenInterest.is_price());
    }

    #[test]
    fn unknown_code_round_trips_with_named_fallback() {
        let dt = DataType::from_code(9999);
        assert_eq!(dt, DataType::Unknown(9999));
        assert_eq!(dt.name(), "unknown_9999");
        assert!(!dt.is_price());
    }

    #[test]
    fn vendor_lowercase_naming_holds() {
        // Sanity: every name is ASCII lowercase or underscore, never a Java
        // CamelCase leak.
        for code in [1, 121, 0, 134, 138] {
            let n = DataType::from_code(code).name();
            assert!(n.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }
}
