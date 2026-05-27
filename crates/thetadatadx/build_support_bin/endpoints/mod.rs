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
/// `"CalendarDays"`) that need a `<tick>_vec_to_pylist` fast-path
/// converter emitted into `sdks/python/src/_generated/tick_classes.rs`.
/// Consumed by the `ticks` generator to decide emission per tick type.
///
/// The set covers two callers today:
///
/// 1. Snapshot / calendar endpoints (`is_snapshot_endpoint`) — the
///    original consumer; uses the converter on the latency-sensitive
///    fast path that skips the `<TickName>List` wrapper allocation.
/// 2. Parsed-endpoint `.stream(handler)` terminals — the OOM-fix
///    consumer; uses the converter to build a per-chunk
///    `Py<PyList>` of typed tick instances handed to the user's
///    callback. Without this, every parsed endpoint with a wide
///    response (`option_history_quote`, `stock_history_trade`, ...)
///    would still buffer the full `Vec<T>` because the only converter
///    available is the `_to_pyclass_list` wrapper-returning variant.
///
/// Single SSOT: classification logic lives in
/// [`sdk_helpers::is_snapshot_endpoint`] for the snapshot half and is
/// driven entirely by TOML `category` / `subcategory` / `kind` fields.
/// The streaming half pulls every endpoint whose `kind == "parsed"`
/// (excluding the simple list endpoints that return `StringList` —
/// those have no per-tick decoder). Adding a parsed endpoint of a new
/// tick type to the TOML automatically opts its converter into
/// emission on the next generator run.
pub fn snapshot_return_types(
) -> Result<std::collections::HashSet<String>, Box<dyn std::error::Error>> {
    let endpoints = parser::load_endpoint_specs()?;
    Ok(endpoints
        .endpoints
        .iter()
        .filter(|e| {
            // Snapshot fast-path consumer + streaming `.stream()` consumer.
            // Parsed list endpoints that return `StringList` have no per-tick
            // decoder, so they're excluded; the streaming variant only
            // exists on endpoints whose `return_type` resolves to a
            // tick collection in `tick_schema.toml`.
            sdk_helpers::is_snapshot_endpoint(e)
                || (e.kind == "parsed" && e.return_type != "StringList")
        })
        .map(|e| e.return_type.clone())
        .collect())
}
