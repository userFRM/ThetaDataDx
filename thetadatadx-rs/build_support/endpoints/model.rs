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
    pub(super) enums: Vec<SurfaceEnum>,
    #[serde(default)]
    pub(super) param_groups: HashMap<String, SurfaceParamGroup>,
    #[serde(default)]
    pub(super) templates: HashMap<String, SurfaceTemplate>,
    pub(super) endpoints: Vec<SurfaceEndpoint>,
    /// Live-validator fixture block. Schema-validation-only on the
    /// build-script side — the bin tree reparses the TOML into its own
    /// `TestFixtures` for emission. Declared here so
    /// `deny_unknown_fields` rejects fixture typos at TOML-load time.
    #[serde(rename = "test_fixtures")]
    _test_fixtures: SurfaceTestFixtures,
    /// Global request-level options that appear on every endpoint via
    /// `ThetaDataDxEndpointRequestOptions`. Declared here so the FFI struct layout
    /// stays in lock-step with the TOML (see scripts/ci/check_docs_consistency.py).
    /// Schema-validation-only: not consumed by the Rust generator today —
    /// only by the docs-consistency drift check — but codified in the
    /// surface so future generator passes can read from here rather than
    /// hardcoding timeout_ms. Leading underscore signals "shape-check
    /// only" to the compiler.
    #[serde(default, rename = "request_options_global")]
    _request_options_global: Vec<SurfaceGlobalRequestOption>,
}

/// A reusable wire string enum declared in `endpoint_surface.toml`.
///
/// The shared parser only reads `name` (for `validate_enum_ref` matching)
/// and `variants[].wire` (for default-value membership). The per-language
/// names (`rust_name`, `variant.rust`, `variant.python`) are
/// schema-validation-only here so `deny_unknown_fields` rejects typos in
/// the build-script TOML parse; the bin tree reparses
/// `endpoint_surface.toml` into its own `EnumProjection` for emission.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceEnum {
    pub(super) name: String,
    #[serde(rename = "rust_name")]
    _rust_name: String,
    pub(super) variants: Vec<SurfaceEnumVariant>,
}

/// A single enum variant across Rust, Python, TypeScript, and wire strings.
///
/// As with `SurfaceEnum`, the per-language identifier fields are
/// schema-validation-only on the build-script side. The bin tree's
/// `EnumProjection` reparses the TOML and consumes them.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceEnumVariant {
    pub(super) wire: String,
    #[serde(rename = "rust")]
    _rust: String,
    #[serde(rename = "python")]
    _python: String,
}

/// A cross-cutting request-level option (e.g. `timeout_ms`).
///
/// Schema-validation-only: fields exist to enforce the TOML shape via serde
/// (`deny_unknown_fields` + required keys) but are not read by the Rust
/// generator today. The docs-consistency drift check uses the raw TOML.
/// Leading underscores signal "shape-check only" to the compiler and suppress
/// `dead_code` without an `#[allow(dead_code)]` escape hatch.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceGlobalRequestOption {
    #[serde(rename = "name")]
    _name: String,
    #[serde(rename = "description")]
    _description: String,
    #[serde(rename = "type")]
    _ty: String,
}

/// Representative fixture values feeding the live-validator parameter-mode
/// matrix. Schema-validation-only on the build-script side — every field is
/// re-read from the same TOML by the bin tree's
/// `build_support_bin/endpoints/test_fixtures.rs` into the bin-owned
/// `TestFixtures`. The build script keeps this declaration solely to enforce
/// `deny_unknown_fields` against the `[test_fixtures]` block at TOML-load
/// time, catching typos as a build failure rather than a bin runtime error.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SurfaceTestFixtures {
    /// Anchor symbol per endpoint category. Feeds `Symbol`/`Symbols` fixtures.
    #[serde(rename = "category_symbol")]
    _category_symbol: HashMap<String, String>,
    /// Concrete (no-wildcard) fixture keyed on the wire `param_type`. Covers
    /// everything except `Symbol`/`Symbols`, which route through
    /// `category_symbol`.
    #[serde(rename = "concrete_by_type")]
    _concrete_by_type: HashMap<String, String>,
    /// Per-param-name overrides that beat `concrete_by_type` matching.
    #[serde(default, rename = "concrete_overrides")]
    _concrete_overrides: HashMap<String, String>,
    /// Per-mode param-name overrides for option ContractSpec variants.
    #[serde(default, rename = "mode_overrides")]
    _mode_overrides: HashMap<String, HashMap<String, String>>,
    /// Representative values for builder-bound optional params.
    #[serde(rename = "optional_defaults")]
    _optional_defaults: HashMap<String, String>,
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
    /// Upstream vendor docstring lifted from the ThetaData Python SDK
    /// (Apache-2.0). Single SSOT field feeding every target language's
    /// docstring rendering (Python / TypeScript / Rust / C++ /
    /// fluent builders / async variants). Templates may set a default
    /// (e.g. empty) which endpoints override.
    #[serde(default)]
    pub(super) vendor_docstring: Option<String>,
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
    /// Upstream vendor docstring lifted from the ThetaData Python SDK
    /// (Apache-2.0). Overrides any template-level `vendor_docstring`.
    /// Appended after the DX-native `description` when emitting per-
    /// language docs — single SSOT for sync, async, and builder paths.
    #[serde(default)]
    pub(super) vendor_docstring: Option<String>,
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
    #[serde(default, rename = "enum")]
    pub(super) enum_name: Option<String>,
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

/// A template after `extends` inheritance is flattened, before endpoints
/// merge their own overrides onto it.
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
    pub(super) vendor_docstring: Option<String>,
    pub(super) params: Vec<SurfaceParam>,
}

/// An endpoint after template and param-group expansion, with every field
/// resolved to a concrete value but before cross-validation against the wire
/// contract.
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
    /// Upstream vendor docstring (sourced from `vendor_docstring` in TOML).
    /// Empty when the endpoint has no vendor counterpart (e.g. streaming
    /// variants DX ships that the vendor SDK doesn't expose).
    pub(super) vendor_docstring: Option<String>,
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

/// A fully merged endpoint parameter consumed by the emitters.
#[derive(Debug, Clone)]
pub(super) struct GeneratedParam {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) param_type: String,
    pub(super) required: bool,
    pub(super) binding: String,
    /// In-house Rust client (`MarketDataClient`) arg-name override sourced
    /// from `endpoint_surface.toml`. Only the build-script render path
    /// honors this — the per-language SDK projection emitters drive
    /// their arg names from `sdk_method_arg_name` instead. The
    /// underscore prefix marks the field as build-side-only so the bin
    /// compile unit (which also sees this struct via `#[path]`) does
    /// not flag it as unread.
    pub(super) _arg_name: Option<String>,
    pub(super) default: Option<String>,
}

/// Endpoint-surface enum reachable from the build script's validation
/// pass.
///
/// Only the fields shared parser reads (`name` for validate_enum_ref
/// matching, `variants[].wire` for default-value membership) live here.
/// The bin tree reads `rust_name`, `variant.rust`, `variant.python`
/// directly off `SurfaceEnum` / `SurfaceEnumVariant` when emitting
/// enum projections.
#[derive(Debug, Clone)]
pub(super) struct GeneratedEnum {
    pub(super) name: String,
    pub(super) variants: Vec<GeneratedEnumVariant>,
}

/// A merged enum variant carrying the wire string the build script validates
/// default-value membership against.
#[derive(Debug, Clone)]
pub(super) struct GeneratedEnumVariant {
    pub(super) wire: String,
}

/// The merged endpoint model joining the TOML surface with the wire
/// contract. The single source of truth every renderer iterates over.
#[derive(Debug, Clone)]
pub(super) struct GeneratedEndpoint {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) category: String,
    pub(super) subcategory: String,
    /// REST path on the upstream service. Only the build-script
    /// registry emitter reads this — the bin's per-language SDK
    /// projection emitters drive their routing via the gRPC method
    /// name on `MarketDataClient`. Underscore-prefixed so the bin compile
    /// unit (which sees this struct via `#[path]`) does not flag it
    /// as unread.
    pub(super) _rest_path: String,
    pub(super) grpc_name: String,
    pub(super) request_type: String,
    pub(super) query_type: String,
    pub(super) fields: Vec<ProtoField>,
    pub(super) params: Vec<GeneratedParam>,
    pub(super) return_type: String,
    pub(super) kind: String,
    pub(super) list_column: Option<String>,
    /// Upstream vendor docstring. Feeds the per-language doc emitters
    /// so Python / TypeScript / Rust / C++ (and the sync / async /
    /// fluent-builder variants within each) all read from a single
    /// TOML field and can never drift.
    pub(super) vendor_docstring: Option<String>,
}

/// The full set of merged endpoints handed to the emitters.
#[derive(Debug, Clone)]
pub(super) struct ParsedEndpoints {
    pub(super) endpoints: Vec<GeneratedEndpoint>,
}

/// Output of `proto_parser::load_proto_endpoints` — the wire-truth set of
/// endpoints derived from `mdds.proto`. Joined with the TOML surface
/// inside `parser::load_endpoint_specs`.
#[derive(Debug, Clone)]
pub(super) struct WireEndpoints {
    pub(super) endpoints: Vec<GeneratedEndpoint>,
}
