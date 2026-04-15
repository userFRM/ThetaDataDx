//! Data types used during endpoint surface generation.
//!
//! Split into three layers:
//!   * **Surface**: the raw `endpoint_surface.toml` shape (including templates
//!     and parameter groups, before any inheritance/expansion).
//!   * **Resolved**: the concrete endpoint after template + param-group
//!     expansion, but before cross-validation against the wire contract.
//!   * **Generated**: the merged model consumed by emitters. `GeneratedEndpoint`
//!     is the SSOT every renderer iterates over.

use std::collections::HashMap;

use serde::Deserialize;

/// A checked-in endpoint surface specification file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceSpec {
    pub(super) version: u32,
    #[serde(default)]
    pub(super) param_groups: HashMap<String, SurfaceParamGroup>,
    #[serde(default)]
    pub(super) templates: HashMap<String, SurfaceTemplate>,
    pub(super) endpoints: Vec<SurfaceEndpoint>,
    pub(super) test_fixtures: SurfaceTestFixtures,
}

/// Representative fixture values feeding the live-validator parameter-mode
/// matrix. Split so every hardcoded value in the Rust generator maps to a
/// single TOML row.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceTestFixtures {
    /// Anchor symbol per endpoint category. Feeds `Symbol`/`Symbols` fixtures.
    pub(super) category_symbol: HashMap<String, String>,
    /// Concrete (no-wildcard) fixture keyed on the wire `param_type`. Covers
    /// everything except `Symbol`/`Symbols`, which route through
    /// `category_symbol`.
    pub(super) concrete_by_type: HashMap<String, String>,
    /// Per-param-name overrides that beat `concrete_by_type` matching (e.g.
    /// the compressed `end_date` that keeps bulk cells inside the 60s
    /// per-cell timeout). See issue #290.
    #[serde(default)]
    pub(super) concrete_overrides: HashMap<String, String>,
    /// Per-mode param-name overrides for option ContractSpec variants
    /// (`concrete_iso`, `all_strikes_one_exp`, wildcard/zero-sentinel cells).
    #[serde(default)]
    pub(super) mode_overrides: HashMap<String, HashMap<String, String>>,
    /// Representative values for builder-bound optional params. Drives
    /// `with_<name>` and `all_optionals` modes.
    pub(super) optional_defaults: HashMap<String, String>,
}

/// A reusable parameter group declared in `endpoint_surface.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceParamGroup {
    #[serde(default)]
    pub(super) params: Vec<SurfaceParamEntry>,
}

/// A reusable endpoint template declared in `endpoint_surface.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceTemplate {
    #[serde(default)]
    pub(super) extends: Option<String>,
    #[serde(default)]
    pub(super) wire_name: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) category: Option<String>,
    #[serde(default)]
    pub(super) subcategory: Option<String>,
    #[serde(default)]
    pub(super) rest_path: Option<String>,
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) returns: Option<String>,
    #[serde(default)]
    pub(super) list_column: Option<String>,
    #[serde(default)]
    pub(super) params: Vec<SurfaceParamEntry>,
}

/// A normalized endpoint surface entry loaded from `endpoint_surface.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceEndpoint {
    pub(super) name: String,
    #[serde(default)]
    pub(super) template: Option<String>,
    #[serde(default)]
    pub(super) wire_name: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) category: Option<String>,
    #[serde(default)]
    pub(super) subcategory: Option<String>,
    #[serde(default)]
    pub(super) rest_path: Option<String>,
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) returns: Option<String>,
    #[serde(default)]
    pub(super) list_column: Option<String>,
    #[serde(default)]
    pub(super) params: Vec<SurfaceParamEntry>,
}

/// A normalized endpoint parameter entry loaded from `endpoint_surface.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceParam {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) param_type: String,
    pub(super) required: bool,
    pub(super) binding: String,
    #[serde(default)]
    pub(super) arg_name: Option<String>,
    #[serde(default)]
    pub(super) default: Option<String>,
}

/// A single parameter entry or reference inside a parameter group, template, or endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum SurfaceParamEntry {
    Use(SurfaceParamUse),
    Param(SurfaceParam),
}

/// A reference to a reusable parameter group.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceParamUse {
    #[serde(rename = "use")]
    pub(super) group: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ResolvedTemplate {
    pub(super) wire_name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) category: Option<String>,
    pub(super) subcategory: Option<String>,
    pub(super) rest_path: Option<String>,
    pub(super) kind: Option<String>,
    pub(super) returns: Option<String>,
    pub(super) list_column: Option<String>,
    pub(super) params: Vec<SurfaceParam>,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedSurfaceEndpoint {
    pub(super) name: String,
    pub(super) wire_name: Option<String>,
    pub(super) description: String,
    pub(super) category: String,
    pub(super) subcategory: String,
    pub(super) rest_path: String,
    pub(super) kind: String,
    pub(super) returns: String,
    pub(super) list_column: Option<String>,
    pub(super) params: Vec<SurfaceParam>,
}

/// A parsed proto field.
#[derive(Debug, Clone)]
pub(super) struct ProtoField {
    pub(super) name: String,
    pub(super) proto_type: String, // "string", "int32", "double", "bool", or "ContractSpec"
    pub(super) is_optional: bool,
    pub(super) is_repeated: bool,
}

/// A parsed RPC entry.
#[derive(Debug)]
pub(super) struct Rpc {
    pub(super) rpc_name: String,     // e.g. "GetStockHistoryEod"
    pub(super) request_type: String, // e.g. "StockHistoryEodRequest"
}

#[derive(Debug, Clone)]
pub(super) struct GeneratedParam {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) param_type: String,
    pub(super) required: bool,
    pub(super) binding: String,
    pub(super) arg_name: Option<String>,
    pub(super) default: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct GeneratedEndpoint {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) category: String,
    pub(super) subcategory: String,
    pub(super) rest_path: String,
    pub(super) grpc_name: String,
    pub(super) request_type: String,
    pub(super) query_type: String,
    pub(super) fields: Vec<ProtoField>,
    pub(super) params: Vec<GeneratedParam>,
    pub(super) return_type: String,
    pub(super) kind: String,
    pub(super) list_column: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ParsedEndpoints {
    pub(super) endpoints: Vec<GeneratedEndpoint>,
    pub(super) fixtures: TestFixtures,
}

/// Resolved fixture tables consumed by `modes.rs`. Wire-compatible 1:1 with
/// `SurfaceTestFixtures` after TOML load — the indirection keeps the build-
/// support shape separate from the proto-parser output type.
#[derive(Debug, Clone, Default)]
pub(super) struct TestFixtures {
    pub(super) category_symbol: HashMap<String, String>,
    pub(super) concrete_by_type: HashMap<String, String>,
    pub(super) concrete_overrides: HashMap<String, String>,
    pub(super) mode_overrides: HashMap<String, HashMap<String, String>>,
    pub(super) optional_defaults: HashMap<String, String>,
}
