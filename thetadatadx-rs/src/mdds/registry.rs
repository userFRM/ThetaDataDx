//! Endpoint registry -- single source of truth for all `MarketDataClient` endpoints.
//!
//! Used by the CLI and MCP server to auto-generate commands and tool
//! definitions. When `ThetaData` adds a new proto RPC, the build script parses
//! `mdds.proto` and regenerates the registry automatically.
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
//! API (`FnMut(&[T])`) that does not map to CLI/MCP output semantics. They
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
//  Generated from mdds.proto by build.rs
//
//  Gated on `__internal`: the generated file defines `ParamType`, `ReturnType`,
//  `ENDPOINTS`, `CATEGORIES`, and `param_type_to_json_type`. All of these are
//  exclusively for workspace tools — not needed in default crate builds.
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "__internal")]
include!(concat!(env!("OUT_DIR"), "/registry_generated.rs"));

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
