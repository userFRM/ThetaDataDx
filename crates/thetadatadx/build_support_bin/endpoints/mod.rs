//! Endpoint surface generation for the `generate_sdk_surfaces` binary.
//!
//! This tree pulls the shared parser / model / proto plumbing from
//! `build_support/endpoints/` via `#[path]` and adds the bin-only render
//! pipeline (per-language SDK projections + the live-validator matrix
//! emitter) that the build script never compiles.
//!
//! Module layout:
//! * [`model`] — shared TOML data types (`#[path]` from `build_support`).
//! * [`parser`] — shared TOML loader (`#[path]` from `build_support`).
//! * [`proto_parser`] — shared proto parser (`#[path]` from `build_support`).
//! * [`helpers`] — shared cross-renderer helpers (`#[path]` from `build_support`).
//! * [`modes`] — bin-only live-validator parameter-mode matrix derivation.
//! * [`fixture_validation`] — bin-only `[test_fixtures]` cross-check.
//! * [`sdk_helpers`] — bin-only render-side helpers.
//! * [`sdk_render`] — bin-only per-language emitters.

#[path = "../../build_support/endpoints/helpers.rs"]
pub(super) mod helpers;
#[path = "../../build_support/endpoints/model.rs"]
pub(super) mod model;
#[path = "../../build_support/endpoints/parser.rs"]
pub(super) mod parser;
#[path = "../../build_support/endpoints/proto_parser.rs"]
pub(super) mod proto_parser;

mod enum_projection;
mod fixture_validation;
mod modes;
mod sdk_helpers;
mod sdk_render;
mod test_fixtures;

use std::path::Path;

pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    sdk_render::write_sdk_generated_files(repo_root)
}

pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    sdk_render::check_sdk_generated_files(repo_root)
}

/// Compute the set of tick return-type names (e.g. `"OhlcTicks"`,
/// `"CalendarDays"`) that are reached by any snapshot-kind endpoint in
/// `endpoint_surface.toml`. Consumed by the `ticks` generator to decide
/// which tick types get a `<tick>_vec_to_pylist` fast-path converter
/// emitted into `sdks/python/src/_generated/tick_classes.rs` — emitting for every
/// tick type would leave dead-code fns (each `_vec_to_pylist` is only
/// called from snapshot-endpoint pymethods, so a tick type with no
/// snapshot endpoint has no caller).
///
/// Single SSOT: classification logic lives in [`sdk_helpers::is_snapshot_endpoint`]
/// and is driven entirely by TOML `category` / `subcategory` / `kind`
/// fields — no hand-curated allowlist, so adding a snapshot endpoint of
/// a new tick type automatically opts its converter into emission.
pub fn snapshot_return_types(
) -> Result<std::collections::HashSet<String>, Box<dyn std::error::Error>> {
    let endpoints = parser::load_endpoint_specs()?;
    Ok(endpoints
        .endpoints
        .iter()
        .filter(|e| sdk_helpers::is_snapshot_endpoint(e))
        .map(|e| e.return_type.clone())
        .collect())
}
