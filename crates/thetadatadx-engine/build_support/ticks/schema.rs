//! TOML-backed schema types for `tick_schema.toml` reachable from `build.rs`.
//!
//! Only the fields the wire decoder (`parser`) consumes live here. The
//! bin tree (`build_support_bin/ticks/`) deserializes the same TOML into
//! a richer `Schema` with the per-language render block + doc / copy /
//! align metadata it needs.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Schema {
    pub(crate) types: HashMap<String, TickTypeDef>,
}

/// Parsed from `tick_schema.toml` — only the fields the build-script's
/// wire-decoder emitter (`parser`) reads.
#[derive(Debug, Deserialize)]
pub(crate) struct TickTypeDef {
    pub(crate) parser: String,
    #[serde(default)]
    pub(crate) required: Vec<String>,
    #[serde(default)]
    pub(crate) eod_style: bool,
    #[serde(default)]
    pub(crate) contract_id: bool,
    pub(crate) columns: Vec<ColumnDef>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ColumnDef {
    pub(crate) name: String,
    pub(crate) field: String,
    pub(crate) r#type: String,
}

pub(super) fn load_schema() -> Result<Schema, Box<dyn std::error::Error>> {
    let schema_path = "tick_schema.toml";
    let schema_str = std::fs::read_to_string(schema_path)?;
    let schema: Schema = toml::from_str(&schema_str)?;
    Ok(schema)
}
