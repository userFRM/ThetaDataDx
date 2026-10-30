//! Endpoint registry -- single source of truth for all `MarketDataClient` endpoints.
//!
//! Used by the CLI and MCP server to auto-generate commands and tool
//! definitions. `ENDPOINTS` expands from the endpoint table, so an endpoint
//! added there is registered with the same parameters and descriptions the
//! client methods carry.
//!
//! # Design
//!
//! Each entry is a `const` descriptor (`EndpointMeta`) that captures:
//! - Method name on `MarketDataClient` (e.g. `"stock_history_eod"`)
//! - Human description
//! - Category / subcategory for grouping
//! - Parameter list with types
//! - Return type discriminant
//!
//! Streaming endpoints (`*_stream`) are excluded because they use a callback
//! API (`FnMut(&Ticks<T>)`) that does not map to CLI/MCP output semantics. They
//! remain available on `MarketDataClient` for programmatic use.

// Items in this module are split into two groups:
//
// 1. Always-compiled: `ParamType`. Used by `endpoint_args.rs`'s `parse_raw_arg_value`
//    and `EndpointArgs::insert_raw`, both of which are `#[cfg(feature = "__internal")]`-gated.
//    `ParamType` must therefore also be `#[cfg(feature = "__internal")]`.
//
// 2. `#[cfg(feature = "__internal")]`: everything else. `EndpointMeta`, `ParamMeta`,
//    `ReturnType`, `ENDPOINTS`, `CATEGORIES`, `find`, `by_category`, and
//    `param_type_to_json_type` are only reachable from workspace tools.

/// Metadata for a single parameter.
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
#[derive(Debug, Clone)]
pub struct ParamMeta {
    /// Parameter name as accepted on the call (e.g. `"symbol"`).
    pub name: &'static str,
    /// Human-readable description surfaced in CLI/MCP help.
    pub description: &'static str,
    /// Scalar type the value is parsed into.
    pub param_type: ParamType,
    /// Whether the parameter must be supplied by the caller.
    pub required: bool,
}

/// Metadata for a single endpoint.
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
#[derive(Debug, Clone)]
pub struct EndpointMeta {
    /// Method name on `MarketDataClient` (e.g. `"stock_history_eod"`).
    pub name: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Category: `"stock"`, `"option"`, `"index"`, `"rate"`, `"calendar"`.
    pub category: &'static str,
    /// Subcategory: `"list"`, `"snapshot"`, `"history"`, `"at_time"`,
    /// `"snapshot_greeks"`, `"history_greeks"`, etc.
    pub subcategory: &'static str,
    #[doc(hidden)]
    pub rest_path: &'static str,
    /// Parameters in call order.
    pub params: &'static [ParamMeta],
    /// Return type discriminant.
    pub returns: ReturnType,
}

// ═══════════════════════════════════════════════════════════════════════════
//  Types and endpoints (only when `__internal` is enabled)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "__internal")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    RequestType,
    Symbol,
    Symbols,
    Venue,
    Str,
    Bool,
    Date,
    Interval,
    Expiration,
    Strike,
    Right,
    Int,
    Float,
    RateType,
    Version,
    Year,
}

#[cfg(feature = "__internal")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnType {
    StringList,
    OhlcTicks,
    TradeTicks,
    QuoteTicks,
    MarketValueTicks,
    EodTicks,
    TradeQuoteTicks,
    IntervalQuoteTicks,
    OptionContracts,
    OpenInterestTicks,
    IvTicks,
    GreeksAllTicks,
    GreeksFirstOrderTicks,
    GreeksSecondOrderTicks,
    GreeksThirdOrderTicks,
    GreeksEodTicks,
    TradeGreeksAllTicks,
    TradeGreeksFirstOrderTicks,
    TradeGreeksSecondOrderTicks,
    TradeGreeksThirdOrderTicks,
    TradeGreeksImpliedVolatilityTicks,
    PriceTicks,
    IndexPriceAtTimeTicks,
    CalendarDays,
    InterestRateTicks,
}

#[cfg(feature = "__internal")]
pub const CATEGORIES: &[&str] = &["stock", "option", "index", "calendar", "rate"];

#[cfg(feature = "__internal")]
#[must_use]
pub fn param_type_to_json_type(pt: ParamType) -> &'static str {
    match pt {
        ParamType::RequestType => "string",
        ParamType::Symbol => "string",
        ParamType::Symbols => "string",
        ParamType::Venue => "string",
        ParamType::Str => "string",
        ParamType::Bool => "boolean",
        ParamType::Date => "string",
        ParamType::Interval => "string",
        ParamType::Expiration => "string",
        ParamType::Strike => "string",
        ParamType::Right => "string",
        ParamType::Int => "integer",
        ParamType::Float => "number",
        ParamType::RateType => "string",
        ParamType::Version => "string",
        ParamType::Year => "string",
    }
}

/// One registry entry from a resolved endpoint. The description is the first
/// line of the endpoint's documentation.
#[cfg(feature = "__internal")]
macro_rules! registry_entry {
    ($name:ident $cat:ident $sub:ident $rest:literal $ret:ident $desc:literal params {
        all [$({ $p:ident $a:ident $b:ident $t:ident $v:tt $d:literal })*] $($more:tt)*
    }) => {
        EndpointMeta {
            name: stringify!($name),
            description: $desc,
            category: stringify!($cat),
            subcategory: stringify!($sub),
            rest_path: $rest,
            params: &[$(ParamMeta {
                name: stringify!($p),
                description: $d,
                param_type: registry_entry!(@type $t),
                required: registry_entry!(@required $b),
            }),*],
            returns: registry_entry!(@returns $ret),
        }
    };
    // A time of day is a string on the wire and in the registry.
    (@type Time) => { ParamType::Str };
    (@type $t:ident) => { ParamType::$t };
    (@required req) => { true };
    (@required opt) => { false };
    (@returns StringList) => { ReturnType::StringList };
    (@returns $item:ident) => { pastey::paste!(ReturnType::[<$item s>]) };
}

/// Expands the endpoint table into `ENDPOINTS`, in table order.
#[cfg(feature = "__internal")]
macro_rules! registry_endpoints {
    ($( $(#[$attr:meta])* $shape:ident $name:ident {
        group: $cat:ident / $sub:ident,
        rest: $rest:literal,
        returns: $ret:ident $(($col:literal))?,
        params: [$($key:tt)*],
        doc: [$desc:literal $(, $doc:literal)* $(,)?],
    } )*) => {
        pub static ENDPOINTS: &[EndpointMeta] = &[$(
            crate::__resolve!([$($key)*] => registry_entry { $name $cat $sub $rest $ret $desc }),
        )*];
    };
}

#[cfg(feature = "__internal")]
crate::__endpoints! { registry_endpoints {} }

// ═══════════════════════════════════════════════════════════════════════════
//  Lookup helpers (only when `__internal` is enabled)
// ═══════════════════════════════════════════════════════════════════════════

/// Find an endpoint by its method name.
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
#[must_use]
pub fn find(name: &str) -> Option<&'static EndpointMeta> {
    ENDPOINTS.iter().find(|e| e.name == name)
}

/// All endpoints in a category.
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
#[must_use]
pub fn by_category(cat: &str) -> Vec<&'static EndpointMeta> {
    ENDPOINTS.iter().filter(|e| e.category == cat).collect()
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(all(test, feature = "__internal"))]
mod tests {
    use super::*;

    #[test]
    fn registry_is_not_empty() {
        assert!(
            !ENDPOINTS.is_empty(),
            "generated registry unexpectedly contains no endpoints"
        );
    }

    #[test]
    fn all_names_unique() {
        let mut names: Vec<&str> = ENDPOINTS.iter().map(|e| e.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), ENDPOINTS.len(), "duplicate endpoint names");
    }

    #[test]
    fn find_works() {
        assert!(find("stock_history_eod").is_some());
        assert!(find("nonexistent").is_none());
    }

    #[test]
    fn every_category_has_endpoints() {
        for category in CATEGORIES {
            assert!(
                !by_category(category).is_empty(),
                "category {category} unexpectedly has no endpoints"
            );
        }
    }

    #[test]
    fn categories_sum_to_total() {
        let total: usize = CATEGORIES.iter().map(|c| by_category(c).len()).sum();
        assert_eq!(total, ENDPOINTS.len());
    }
}
