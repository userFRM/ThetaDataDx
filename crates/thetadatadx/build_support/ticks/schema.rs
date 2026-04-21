//! TOML-backed schema types for `tick_schema.toml`.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Schema {
    pub(crate) types: HashMap<String, TickTypeDef>,
}

/// Parsed from `tick_schema.toml`. `doc`, `copy`, and `align` exist in the
/// TOML for documentation / FFI layout hints but are not used by the parser
/// generator — tick structs live in `tdbe::types::tick` (hand-written).
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
