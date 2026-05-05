//! TOML-backed schema types for `tick_schema.toml`.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Schema {
    pub(crate) types: HashMap<String, TickTypeDef>,
}

/// Parsed from `tick_schema.toml`. `doc`, `copy`, and `align` drive the
/// generated tick struct layout (see `crates/tdbe/build_support`); `render`
/// drives the per-language binding name lookups consumed by every renderer
/// in `build_support/endpoints/render` and `build_support/ticks`.
#[derive(Debug, Deserialize)]
pub(crate) struct TickTypeDef {
    pub(crate) doc: String,
    pub(crate) copy: bool,
    #[serde(default)]
    pub(crate) align: Option<u32>,
    pub(crate) parser: String,
    #[serde(default)]
    pub(crate) required: Vec<String>,
    #[serde(default)]
    pub(crate) eod_style: bool,
    #[serde(default)]
    pub(crate) contract_id: bool,
    pub(crate) columns: Vec<ColumnDef>,
    /// Per-language binding name map. Populated for every tick type so the
    /// 19 helper match arms collapse into single-key lookups (see
    /// `build_support/endpoints/helpers.rs::render_for`).
    pub(crate) render: TickRenderDef,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ColumnDef {
    pub(crate) name: String,
    pub(crate) field: String,
    pub(crate) r#type: String,
}

/// Every per-language binding name a renderer needs for one tick type.
/// Single TOML row replaces the parallel match arms previously hand-coded
/// across `helpers.rs` and `ticks/mod.rs::pyclass_name`.
#[derive(Debug, Deserialize, Clone)]
pub(crate) struct TickRenderDef {
    /// Wire-collection plural keying every renderer call (e.g. `"GreeksTicks"`).
    /// Matches the `returns` value declared on each endpoint in
    /// `endpoint_surface.toml`. Build-time validator rejects duplicates and
    /// strays.
    pub(crate) collection: String,
    pub(crate) direct: String,
    pub(crate) parser: String,
    pub(crate) go_struct: String,
    pub(crate) go_converter: String,
    pub(crate) ffi_array: String,
    pub(crate) ffi_array_empty: String,
    pub(crate) ffi_output_variant: String,
    pub(crate) ffi_from_vec_array: String,
    pub(crate) ffi_header_return: String,
    pub(crate) ffi_free_fn: String,
    pub(crate) cpp_value: String,
    pub(crate) python_converter: String,
    pub(crate) python_columnar: String,
    pub(crate) python_pyclass_list: String,
    pub(crate) python_vec_to_pylist: String,
    pub(crate) python_slice_arrow: String,
    pub(crate) ts_class: String,
    pub(crate) ts_class_vec: String,
    pub(crate) pyclass: String,
}

pub(super) fn load_schema() -> Result<Schema, Box<dyn std::error::Error>> {
    let schema_path = "tick_schema.toml";
    let schema_str = std::fs::read_to_string(schema_path)?;
    let schema: Schema = toml::from_str(&schema_str)?;
    Ok(schema)
}
