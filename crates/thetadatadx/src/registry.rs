//! Endpoint registry -- single source of truth for all `MddsClient` endpoints.
//!
//! Used by the CLI and MCP server to auto-generate commands and tool definitions.
//! When `ThetaData` adds a new proto RPC, the build script parses
//! `mdds.proto` and regenerates the registry automatically.
//!
//! # Design
//!
//! Each entry is a `const` descriptor (`EndpointMeta`) that captures:
//! - Method name on `MddsClient` (e.g. `"stock_history_eod"`)
//! - Human description
//! - Category / subcategory for grouping
//! - Canonical REST path for terminal-compatible HTTP routing
//! - Parameter list with types
//! - Return type discriminant
//!
//! Streaming endpoints (`*_stream`) are excluded because they use a callback
//! API (`FnMut(&[T])`) that does not map to CLI/MCP output semantics. They
//! remain available on `MddsClient` for programmatic use.

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
    /// Method name on `MddsClient` (e.g. `"stock_history_eod"`).
    pub name: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Category: `"stock"`, `"option"`, `"index"`, `"rate"`, `"calendar"`.
    pub category: &'static str,
    /// Subcategory: `"list"`, `"snapshot"`, `"history"`, `"at_time"`,
    /// `"snapshot_greeks"`, `"history_greeks"`, etc.
    pub subcategory: &'static str,
    /// Canonical terminal-compatible REST path (for example `/v3/stock/history/eod`).
    pub rest_path: &'static str,
    /// Parameters in call order.
    pub params: &'static [ParamMeta],
    /// Return type discriminant.
    pub returns: ReturnType,
}

// ═══════════════════════════════════════════════════════════════════════════
//  Generated from mdds.proto by build.rs
// ═══════════════════════════════════════════════════════════════════════════

include!(concat!(env!("OUT_DIR"), "/registry_generated.rs"));

// ═══════════════════════════════════════════════════════════════════════════
//  Lookup helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Find an endpoint by its method name.
#[must_use]
pub fn find(name: &str) -> Option<&'static EndpointMeta> {
    ENDPOINTS.iter().find(|e| e.name == name)
}

/// All endpoints in a category.
#[must_use]
pub fn by_category(cat: &str) -> Vec<&'static EndpointMeta> {
    ENDPOINTS.iter().filter(|e| e.category == cat).collect()
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
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
