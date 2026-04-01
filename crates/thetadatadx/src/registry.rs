//! Endpoint registry -- single source of truth for all DirectClient endpoints.
//!
//! Used by the CLI and MCP server to auto-generate commands and tool definitions.
//! When ThetaData adds a new proto RPC, the build script parses
//! `v3_endpoints.proto` and regenerates the registry automatically.
//!
//! # Design
//!
//! Each entry is a `const` descriptor (`EndpointMeta`) that captures:
//! - Method name on `DirectClient` (e.g. `"stock_history_eod"`)
//! - Human description
//! - Category / subcategory for grouping
//! - Parameter list with types
//! - Return type discriminant
//!
//! Streaming endpoints (`*_stream`) are excluded because they use a callback
//! API (`FnMut(&[T])`) that does not map to CLI/MCP output semantics. They
//! remain available on `DirectClient` for programmatic use.

/// Parameter type for endpoint arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    /// Single ticker symbol (e.g. "AAPL")
    Symbol,
    /// Comma-separated symbols (e.g. "AAPL,MSFT,GOOGL")
    Symbols,
    /// Date in YYYYMMDD format
    Date,
    /// Accepts milliseconds ("60000") or shorthand ("1m"). Presets: 100ms, 500ms, 1s, 5s, 10s, 15s, 30s, 1m, 5m, 10m, 15m, 30m, 1h.
    Interval,
    /// Option right: C or P
    Right,
    /// Strike price as string
    Strike,
    /// Expiration date as string
    Expiration,
    /// Request type string (e.g. "EOD", "TRADE")
    RequestType,
    /// Free-form string
    Str,
    /// Year string (e.g. "2024")
    Year,
    /// Floating-point number (e.g. annual_dividend)
    Float,
    /// Integer (e.g. max_dte, strike_range)
    Int,
    /// Boolean flag
    Bool,
}

/// What the endpoint returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnType {
    /// `Vec<String>` extracted from a column
    StringList,
    /// `Vec<EodTick>`
    EodTicks,
    /// `Vec<OhlcTick>`
    OhlcTicks,
    /// `Vec<TradeTick>`
    TradeTicks,
    /// `Vec<QuoteTick>`
    QuoteTicks,
    /// Raw `proto::DataTable` (Greeks, calendar, rates, etc.)
    DataTable,
}

/// Metadata for a single parameter.
#[derive(Debug, Clone)]
pub struct ParamMeta {
    pub name: &'static str,
    pub description: &'static str,
    pub param_type: ParamType,
    pub required: bool,
}

/// Metadata for a single endpoint.
#[derive(Debug, Clone)]
pub struct EndpointMeta {
    /// Method name on `DirectClient` (e.g. `"stock_history_eod"`).
    pub name: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Category: `"stock"`, `"option"`, `"index"`, `"rate"`, `"calendar"`.
    pub category: &'static str,
    /// Subcategory: `"list"`, `"snapshot"`, `"history"`, `"at_time"`,
    /// `"snapshot_greeks"`, `"history_greeks"`, etc.
    pub subcategory: &'static str,
    /// Parameters in call order.
    pub params: &'static [ParamMeta],
    /// Return type discriminant.
    pub returns: ReturnType,
}

// ═══════════════════════════════════════════════════════════════════════════
//  Generated from v3_endpoints.proto by build.rs
// ═══════════════════════════════════════════════════════════════════════════

include!(concat!(env!("OUT_DIR"), "/registry_generated.rs"));

// ═══════════════════════════════════════════════════════════════════════════
//  Lookup helpers
// ═══════════════════════════════════════════════════════════════════════════

/// All category names in display order.
pub const CATEGORIES: &[&str] = &["stock", "option", "index", "rate", "calendar"];

/// Find an endpoint by its method name.
pub fn find(name: &str) -> Option<&'static EndpointMeta> {
    ENDPOINTS.iter().find(|e| e.name == name)
}

/// All endpoints in a category.
pub fn by_category(cat: &str) -> Vec<&'static EndpointMeta> {
    ENDPOINTS.iter().filter(|e| e.category == cat).collect()
}

/// Map a `ParamType` to a JSON Schema type string.
pub fn param_type_to_json_type(pt: ParamType) -> &'static str {
    match pt {
        ParamType::Symbol
        | ParamType::Symbols
        | ParamType::Date
        | ParamType::Interval
        | ParamType::Right
        | ParamType::Strike
        | ParamType::Expiration
        | ParamType::RequestType
        | ParamType::Str
        | ParamType::Year => "string",
        ParamType::Float => "number",
        ParamType::Int => "integer",
        ParamType::Bool => "boolean",
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_count_is_61() {
        assert_eq!(
            ENDPOINTS.len(),
            61,
            "expected 61 non-streaming endpoints, got {}",
            ENDPOINTS.len()
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
    fn by_category_stock() {
        let stock = by_category("stock");
        // 13 from proto + 1 manual (stock_history_ohlc_range) = 14
        assert_eq!(stock.len(), 14);
    }

    #[test]
    fn by_category_option() {
        let option = by_category("option");
        assert_eq!(option.len(), 34);
    }

    #[test]
    fn by_category_index() {
        let index = by_category("index");
        assert_eq!(index.len(), 9);
    }

    #[test]
    fn by_category_calendar() {
        let cal = by_category("calendar");
        assert_eq!(cal.len(), 3);
    }

    #[test]
    fn by_category_rate() {
        let rate = by_category("rate");
        assert_eq!(rate.len(), 1);
    }

    #[test]
    fn categories_sum_to_total() {
        let total: usize = CATEGORIES.iter().map(|c| by_category(c).len()).sum();
        assert_eq!(total, ENDPOINTS.len());
    }

    #[test]
    fn all_params_have_names() {
        for ep in ENDPOINTS {
            for p in ep.params {
                assert!(!p.name.is_empty(), "empty param name in {}", ep.name);
                assert!(
                    !p.description.is_empty(),
                    "empty param description in {}::{}",
                    ep.name,
                    p.name
                );
            }
        }
    }

    #[test]
    fn stock_history_ohlc_range_exists() {
        assert!(
            find("stock_history_ohlc_range").is_some(),
            "manual stock_history_ohlc_range entry must be present"
        );
    }
}
