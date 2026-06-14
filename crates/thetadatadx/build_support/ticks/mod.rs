//! Code-generate tick decoders from `tick_schema.toml`.
//!
//! Build-script compile unit: only the wire decoder (`parser`) and its
//! schema loader (`schema`) live here. Checked-in SDK projections
//! (Python pyclass + Arrow, TypeScript napi, C++ layout asserts, tdbe
//! repr structs, CLI raw headers) live in `build_support_bin/ticks/`
//! and never enter this compile unit.

pub(super) mod parser;
pub(super) mod schema;

/// Generates the tick decoders from `tick_schema.toml`.
pub fn generate() -> Result<(), Box<dyn std::error::Error>> {
    parser::generate()
}
