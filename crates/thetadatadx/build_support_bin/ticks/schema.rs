//! TOML-backed schema types for `tick_schema.toml` reachable from the
//! `generate_sdk_surfaces` binary.
//!
//! Adds the per-language render block + doc / copy / align metadata that
//! the SDK projection emitters need on top of the build-script's
//! decoder-focused view. Both compile units deserialize the same TOML —
//! extra fields here, missing fields in the build-script schema.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Schema {
    pub(crate) types: HashMap<String, TickTypeDef>,
}

/// Parsed from `tick_schema.toml`. `doc`, `copy`, and `align` drive the
/// generated tick struct layout; `render` drives the per-language binding
/// name lookups consumed by every renderer in
/// `build_support_bin/endpoints/sdk_render/` and `build_support_bin/ticks`.
#[derive(Debug, Deserialize)]
pub(crate) struct TickTypeDef {
    pub(crate) doc: String,
    pub(crate) copy: bool,
    #[serde(default)]
    pub(crate) align: Option<u32>,
    /// Wire decoder function name. Build-script-only — the bin emitters
    /// never see decoded ticks, only their schema. Underscore-prefixed
    /// so the lint allows the field on the bin compile unit.
    #[serde(rename = "parser")]
    _parser: String,
    /// Wire decoder required-fields list. Build-script-only on the same
    /// rationale as `_parser`.
    #[serde(default, rename = "required")]
    _required: Vec<String>,
    /// Wire decoder EOD-row layout flag. Build-script-only on the same
    /// rationale as `_parser`.
    #[serde(default, rename = "eod_style")]
    _eod_style: bool,
    #[serde(default)]
    pub(crate) contract_id: bool,
    pub(crate) columns: Vec<ColumnDef>,
    /// Per-language binding name map. Populated for every tick type so
    /// the SDK projection emitters reach for one TOML row per tick type.
    pub(crate) render: TickRenderDef,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ColumnDef {
    /// Wire / decode-layer column spelling. Every public surface emits
    /// `field`; only the docs generator still prints `name` (in its
    /// missing-doc diagnostics), so the field is dead code in the
    /// `generate_sdk_surfaces` compile unit — same gate as `doc`.
    #[cfg_attr(not(feature = "__internal"), allow(dead_code))]
    pub(crate) name: String,
    pub(crate) field: String,
    pub(crate) r#type: String,
    /// One-sentence field description rendered on the docs-site
    /// response-schema tables (`generate_docs_site`). Optional at the
    /// serde layer so the build-script view of the same TOML stays
    /// untouched; the docs generator fails loudly on a missing doc.
    /// Only the docs generator reads it, so the field is dead code in
    /// the `generate_sdk_surfaces` compile unit (no `__internal`).
    #[serde(default)]
    #[cfg_attr(not(feature = "__internal"), allow(dead_code))]
    pub(crate) doc: Option<String>,
}

/// Every per-language binding name a renderer needs for one tick type.
/// Single TOML row replaces the parallel match arms previously hand-coded
/// across `helpers.rs` and `ticks/mod.rs::pyclass_name`.
///
/// Underscore-prefixed fields are schema-validation-only at this seam:
/// the per-language binding name they hold is read elsewhere (the
/// endpoint emitters' `sdk_helpers` reparses the same TOML for FFI / C++
/// / Python emit paths), but the per-tick emitters in this tree only
/// reach for `collection`, the pyclass / Vec / Arrow projections, and
/// `ts_class_vec`. Keeping the full shape here lets `deny_unknown_fields`
/// (added at this tree's load seam) reject typos at TOML-load time.
#[derive(Debug, Deserialize, Clone)]
pub(crate) struct TickRenderDef {
    /// Wire-collection plural keying every renderer call (e.g. `"GreeksTicks"`).
    /// Matches the `returns` value declared on each endpoint in
    /// `endpoint_surface.toml`. Build-time validator rejects duplicates and
    /// strays.
    pub(crate) collection: String,
    #[serde(rename = "direct")]
    _direct: String,
    #[serde(rename = "parser")]
    _parser: String,
    #[serde(rename = "ffi_array")]
    _ffi_array: String,
    #[serde(rename = "ffi_output_variant")]
    _ffi_output_variant: String,
    #[serde(rename = "ffi_from_vec_array")]
    _ffi_from_vec_array: String,
    #[serde(rename = "ffi_header_return")]
    _ffi_header_return: String,
    #[serde(rename = "ffi_free_fn")]
    _ffi_free_fn: String,
    #[serde(rename = "cpp_value")]
    _cpp_value: String,
    #[serde(rename = "python_converter")]
    _python_converter: String,
    #[serde(rename = "python_columnar")]
    _python_columnar: String,
    pub(crate) python_pyclass_list: String,
    pub(crate) python_vec_to_pylist: String,
    pub(crate) python_slice_arrow: String,
    #[serde(rename = "ts_class")]
    _ts_class: String,
    pub(crate) ts_class_vec: String,
    pub(crate) pyclass: String,
}

pub(crate) fn load_schema() -> Result<Schema, Box<dyn std::error::Error>> {
    let schema_path = "tick_schema.toml";
    let schema_str = std::fs::read_to_string(schema_path)?;
    let schema: Schema = toml::from_str(&schema_str)?;
    Ok(schema)
}

/// Borrow the render block of a schema type by name. Panics with the
/// available keys when the type is missing -- a missing tick type is a
/// build-time bug. Used by every ticks/* emitter that previously kept a
/// hand-coded match arm per tick type for FFI / Python / TS binding
/// names.
pub(crate) fn render_for_type<'a>(schema: &'a Schema, type_name: &str) -> &'a TickRenderDef {
    schema
        .types
        .get(type_name)
        .map(|d| &d.render)
        .unwrap_or_else(|| {
            let mut keys: Vec<&str> = schema.types.keys().map(String::as_str).collect();
            keys.sort();
            panic!(
                "no render block for tick type '{type_name}' in tick_schema.toml; available: {}",
                keys.join(", ")
            )
        })
}
