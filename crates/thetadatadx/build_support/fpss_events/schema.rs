//! TOML-backed schema types for `fpss_event_schema.toml`.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Schema {
    #[serde(default)]
    pub(crate) version: u32,
    pub(crate) events: HashMap<String, EventDef>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EventDef {
    /// "data" (typed market-data tick), "simple" (diagnostic control /
    /// fallback), or "raw_data" (unrecognized wire frame). Drives how the
    /// generator places the variant in the `BufferedEvent` enum, maps it
    /// to the `FpssEvent` source variant, and whether its typed struct
    /// carries a `kind` discriminator getter.
    #[serde(default = "default_kind")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) doc: String,
    pub(crate) columns: Vec<ColumnDef>,
}

fn default_kind() -> String {
    "data".to_string()
}

#[derive(Debug, Deserialize)]
pub(crate) struct ColumnDef {
    pub(crate) name: String,
    pub(crate) r#type: String,
}

pub(super) fn load_schema() -> Result<Schema, Box<dyn std::error::Error>> {
    let schema_str = std::fs::read_to_string("fpss_event_schema.toml")?;
    let schema: Schema = toml::from_str(&schema_str)?;
    Ok(schema)
}

/// Stable sorted list of every event name — every emitter consumes this.
pub(super) fn sorted_event_names(schema: &Schema) -> Vec<&str> {
    let mut names = schema.events.keys().map(String::as_str).collect::<Vec<_>>();
    names.sort();
    names
}

/// Names of `[events.*]` entries whose `kind = "data"` — the
/// market-data tick variants. The TypeScript emitter uses this to
/// skip the per-variant `#[napi(object)]` struct emission for the
/// Simple / RawData variants, which have their own dedicated
/// `FpssSimplePayload` / `FpssRawDataPayload` payloads on the
/// `FpssEvent` wrapper.
pub(super) fn sorted_data_event_names(schema: &Schema) -> Vec<&str> {
    let mut names: Vec<&str> = schema
        .events
        .iter()
        .filter(|(_, def)| def.kind == "data")
        .map(|(n, _)| n.as_str())
        .collect();
    names.sort();
    names
}

/// Iterate schema variants in a stable order, yielding only `kind = "data"`
/// entries. The Rust-FFI, C-header, and Go emitters all share this ordering
/// so the tagged-struct `TdxFpssEvent` / `FpssEvent` layouts line up across
/// languages without manual coordination.
pub(super) fn sorted_data_events(schema: &Schema) -> Vec<(&str, &EventDef)> {
    let mut out: Vec<(&str, &EventDef)> = schema
        .events
        .iter()
        .filter(|(_, def)| def.kind == "data")
        .map(|(n, d)| (n.as_str(), d))
        .collect();
    out.sort_by_key(|(n, _)| *n);
    out
}
