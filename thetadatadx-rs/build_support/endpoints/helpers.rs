//! Cross-cutting helpers shared by every renderer.
//!
//! Shared between the build script (`build_support`) and the bin
//! (`build_support_bin`) trees. Holds the cross-renderer items both
//! contexts need: param classification (`is_*` filters), the in-house
//! Rust client's `direct_*` type table, docstring composition, and the
//! TOML-driven tick render map (`render_for`).
//!
//! Bin-only helpers (per-language SDK projection arg/literal renderers,
//! FFI option mappers, CLI command derivations, etc.) live in
//! `build_support_bin/endpoints/sdk_helpers.rs`.
//!
//! Anything that emits a multi-line chunk of target-language code belongs
//! in a `render/` tree, not here.
//!
//! ## Per-tick render map (TOML-driven)
//!
//! The 19 hand-coded match arms that previously lived here for each renderer
//! call (e.g. `direct_return_type`, `python_pyclass_list_class`) collapsed
//! into single-key lookups against `tick_schema.toml::[types.X.render]`.
//! Adding a tick type now means adding one TOML row -- no helper edits at
//! all. See [`render_for`].

use std::collections::HashMap;
use std::sync::OnceLock;

use heck::ToUpperCamelCase;

use super::model::{GeneratedEndpoint, GeneratedParam};

// ─────────────────────────── Render-map loader ─────────────────────────────

/// Shared per-tick decoder-name map keyed by wire-collection plural
/// (e.g. `"GreeksAllTicks"`). Only the `parser` name is shared between
/// the build script (which emits the `HistoricalClient` direct path) and the
/// bin (which emits the Python decode-bench dispatch arms). Per-language
/// type names (`ffi_*`, `cpp_*`, `python_*`, `ts_*`, the in-house Rust
/// `direct` name) live in tree-local render maps in
/// `build_support/endpoints/build_helpers.rs` and
/// `build_support_bin/endpoints/sdk_helpers.rs` respectively.
#[derive(Debug, Clone)]
pub(super) struct TickRender {
    pub(super) parser: String,
}

#[derive(serde::Deserialize)]
struct SchemaToml {
    types: HashMap<String, TickTypeToml>,
}

#[derive(serde::Deserialize)]
struct TickTypeToml {
    render: TickRenderToml,
}

#[derive(serde::Deserialize)]
struct TickRenderToml {
    collection: String,
    parser: String,
}

static RENDER_MAP: OnceLock<HashMap<String, TickRender>> = OnceLock::new();

fn load_render_map() -> HashMap<String, TickRender> {
    let schema_path = "tick_schema.toml";
    let raw = std::fs::read_to_string(schema_path)
        .unwrap_or_else(|e| panic!("failed to read {schema_path}: {e}"));
    let parsed: SchemaToml =
        toml::from_str(&raw).unwrap_or_else(|e| panic!("failed to parse {schema_path}: {e}"));
    let mut map = HashMap::new();
    let mut tick_to_collection: HashMap<String, String> = HashMap::new();
    for (tick_name, def) in parsed.types {
        let render = def.render;
        let collection = render.collection.clone();
        if let Some(prev) = tick_to_collection.insert(tick_name.clone(), collection.clone()) {
            panic!("tick type '{tick_name}' duplicates collection '{prev}' / '{collection}'");
        }
        let entry = TickRender {
            parser: render.parser,
        };
        if map.insert(collection.clone(), entry).is_some() {
            panic!("duplicate render collection '{collection}' in tick_schema.toml");
        }
    }
    map
}

/// Look up the per-tick decoder name for a wire-collection plural (e.g.
/// `"GreeksTicks"`). Panics with the available keys when the collection
/// is missing — a missing TOML row is a build-time bug.
pub(super) fn render_for(collection: &str) -> &'static TickRender {
    let map = RENDER_MAP.get_or_init(load_render_map);
    map.get(collection).unwrap_or_else(|| {
        let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
        keys.sort();
        panic!(
            "no render entry for collection '{collection}' in tick_schema.toml; available: {}",
            keys.join(", ")
        )
    })
}

// ───────────────────────── Param classification ────────────────────────────

/// True when the endpoint returns a flat list (`kind == "list"`).
pub(super) fn is_simple_list_endpoint(endpoint: &GeneratedEndpoint) -> bool {
    endpoint.kind == "list"
}

/// True when the endpoint is a real-time subscription (`kind == "stream"`).
pub(super) fn is_streaming_endpoint(endpoint: &GeneratedEndpoint) -> bool {
    endpoint.kind == "stream"
}

/// True for the snapshot / calendar endpoints that resolve to at most one
/// row per request.
///
/// Classification is structural, not a hand-curated allowlist:
///   * `subcategory = "snapshot"` — stock / option / index snapshot variants.
///   * `subcategory = "snapshot_greeks"` — `option_snapshot_greeks_*`.
///   * `category = "calendar"` — the calendar endpoints (both `calendar_status`
///     and `calendar_query` group under the `calendar` category).
///
/// These return a bounded handful of rows, so the chunk-callback streaming
/// shape does not apply: a snapshot has nothing to drain incrementally.
pub(super) fn is_snapshot_endpoint(endpoint: &GeneratedEndpoint) -> bool {
    if endpoint.kind != "parsed" {
        return false;
    }
    matches!(
        endpoint.subcategory.as_str(),
        "snapshot" | "snapshot_greeks"
    ) || endpoint.category == "calendar"
}

/// True when an endpoint gets a server-stream terminal (`request.stream`
/// surfaced per binding).
///
/// SSOT for the predicate the managed bindings (Python / TypeScript) share:
/// the bounded snapshot / calendar endpoints are excluded (nothing to drain),
/// the `stream`-kind FPSS-shaped endpoints are excluded (they are real-time
/// subscriptions, not a single large result), and the flat `StringList` list
/// endpoints are excluded (they return a flat `Vec<String>`, not a typed row
/// collection, so the row-chunk callback shape does not fit). Everything
/// else — the multi-day / full-universe history pulls — streams.
pub(super) fn endpoint_streams(endpoint: &GeneratedEndpoint) -> bool {
    !is_snapshot_endpoint(endpoint)
        && !is_streaming_endpoint(endpoint)
        && endpoint.return_type != "StringList"
}

/// True when an endpoint gets a server-stream terminal on the C ABI and the
/// C++ wrapper.
///
/// Narrows [`endpoint_streams`] by the constraint the zero-copy chunk
/// callback imposes: the per-chunk pointer is the SAME flat `#[repr(C)]` tick
/// the buffered tick array exposes, so the C side reinterprets it with no
/// re-marshaling. `OptionContracts` is the lone streaming return type whose
/// core row carries an owned `String` (the contract symbol) — its FFI
/// representation (`ThetaDataDxOptionContract`, a `*const c_char` symbol) is a
/// distinct layout the buffered path marshals row-by-row. That per-row
/// `CString` allocation is exactly the heap traffic the streaming path exists
/// to avoid, and it has no stable lifetime inside a borrowed-slice callback,
/// so the contract-discovery endpoint keeps its buffered C-ABI form only. The
/// managed bindings (Python / TypeScript) marshal per row in GC'd memory and
/// stream it regardless.
pub(super) fn endpoint_streams_repr_c_ticks(endpoint: &GeneratedEndpoint) -> bool {
    endpoint_streams(endpoint) && endpoint.return_type != "OptionContracts"
}

/// True when the param is passed as a method-call argument
/// (`binding == "method"`) rather than a builder setter.
pub(super) fn is_method_call_param(param: &GeneratedParam) -> bool {
    param.binding == "method"
}

// ───────────────────────── Docstring composition ───────────────────────────
//
// SSOT: `endpoint.description` is the short DX-native sentence that already
// drives every sync method's `///` line today. `endpoint.vendor_docstring`
// is the upstream vendor's richer prose (feed-source notes, subscription
// tier behavior, parameter defaults). We emit `description` first — the
// typed-return description stays on top for grep-ability — then a blank
// line and the vendor block. Both sync and async methods, and the fluent
// builder's `arrow()` / `list()` / `polars()` / `pandas()` terminals,
// pull from the same composed string so no variant can drift.

/// Compose the full doc body for an endpoint: native description first,
/// vendor block (if any) appended with a blank separator line.
pub(super) fn compose_endpoint_doc(endpoint: &GeneratedEndpoint) -> String {
    let mut body = match endpoint.vendor_docstring.as_deref() {
        Some(vendor) if !vendor.is_empty() => {
            format!("{}\n\n{vendor}", endpoint.description)
        }
        _ => endpoint.description.clone(),
    };
    let defaults_block = render_param_defaults_block(endpoint);
    if !defaults_block.is_empty() {
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.push('\n');
        body.push_str(&defaults_block);
    }
    body
}

/// Render the "Defaults (upstream)" block surfacing every param whose
/// `default` is set in `endpoint_surface.toml`. Single SSOT origin so
/// `help()` (Python), JSDoc hover (TypeScript), and `cargo doc` (Rust)
/// all agree. String defaults render with quotes; numeric / bool
/// defaults render bare. Empty output when no param has a default.
fn render_param_defaults_block(endpoint: &GeneratedEndpoint) -> String {
    use std::fmt::Write as _;
    let mut rows: Vec<(String, String)> = Vec::new();
    for param in &endpoint.params {
        let Some(default) = param.default.as_deref() else {
            continue;
        };
        let value = match param.param_type.as_str() {
            "Bool" | "Int" | "Float" => default.to_string(),
            _ => format!("\"{default}\""),
        };
        rows.push((param.name.clone(), value));
    }
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from("Defaults (upstream):\n");
    for (name, value) in rows {
        writeln!(out, "- `{name}`: `{value}`").unwrap();
    }
    out
}

// ───────────────────────── Casing ────────────────────────────────────────────

/// Converts a `snake_case` identifier to `PascalCase`.
pub(super) fn to_pascal_case(value: &str) -> String {
    value.to_upper_camel_case()
}

// ───────────────────────── Direct (Rust) client tick map lookup ───────────

/// Map a wire-collection plural to the generated `parse_*_ticks` function
/// name. Shared because the bin's Python wrapper emitter also needs the
/// fully-qualified parser path to forward decoded chunks.
pub(super) fn direct_parser_name(return_type: &str) -> String {
    format!("decode::{}", render_for(return_type).parser)
}

// ───────────────────────── Per-language type tables ─────────────────────────

// ───────────────────────── Builder / FFI option tables ─────────────────────

// ───────────────────────── SDK method arg declarations ─────────────────────

// ───────────────────────── Validator arg literals ──────────────────────────

// ───────────────────────── CLI / validator scaffolding ─────────────────────
