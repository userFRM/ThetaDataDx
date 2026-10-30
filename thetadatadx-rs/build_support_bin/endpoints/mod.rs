//! Endpoint surface generation for the `generate_sdk_surfaces` binary.
//!
//! This tree loads `endpoint_surface.toml`, joins it to the wire contract in
//! `proto/mdds.proto`, and renders the per-language SDK projections and the
//! live-validator matrix from the result.
//!
//! Module layout:
//! * [`model`]: TOML data types and the joined endpoint model.
//! * [`parser`]: TOML loader and its validation against the wire contract.
//! * [`proto_parser`]: proto parser.
//! * [`helpers`]: cross-renderer helpers.
//! * [`modes`]: live-validator parameter-mode matrix derivation.
//! * [`fixture_validation`]: `[test_fixtures]` cross-check.
//! * [`sdk_helpers`]: render-side helpers.
//! * [`sdk_render`]: per-language emitters.

pub(super) mod helpers;
pub(super) mod model;
pub(super) mod parser;
pub(super) mod proto_parser;

// Dead-code allowance mirrors the bin-side mod declarations: when
// `generate_sdk_surfaces` is compiled with the `__internal` feature in
// the same invocation as `generate_docs_site`, this module is present
// in a compile unit that never calls it.
#[cfg(feature = "__internal")]
#[allow(dead_code)]
mod docs_render;
mod enum_projection;
mod fixture_validation;
mod modes;
mod sdk_helpers;
mod sdk_render;
mod test_fixtures;

use std::path::Path;

#[cfg(feature = "__internal")]
pub use docs_render::{check_docs_site_files, write_docs_site_files};

/// Renders every per-language SDK surface file and writes it to disk
/// under `repo_root`.
pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    sdk_render::write_sdk_generated_files(repo_root)
}

/// Renders every per-language SDK surface file and verifies the on-disk
/// copy matches, returning an error on the first missing or stale file.
pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    sdk_render::check_sdk_generated_files(repo_root)
}

/// Compute the set of tick return-type names (e.g. `"OhlcTicks"`,
/// `"CalendarDays"`) that need a `<tick>_vec_to_pylist` fast-path
/// converter emitted into `thetadatadx-py/src/_generated/tick_classes.rs`.
/// Consumed by the `ticks` generator to decide emission per tick type.
///
/// The only caller is a snapshot / calendar endpoint that returns a plain
/// list ([`sdk_helpers::snapshot_returns_plain_pylist`]): the converter
/// skips the `<TickName>List` wrapper on that latency-sensitive path. Every
/// other endpoint, the `.stream(handler)` chunks included, returns the
/// wrapper, which carries the column presence and so each row's root. Keying
/// emission on the same predicate the endpoint emitter uses keeps a tick type
/// no plain-list snapshot returns from landing as a dead-code function.
pub fn snapshot_return_types()
-> Result<std::collections::HashSet<String>, Box<dyn std::error::Error>> {
    let endpoints = parser::load_endpoint_specs()?;
    Ok(endpoints
        .endpoints
        .iter()
        .filter(|e| sdk_helpers::snapshot_returns_plain_pylist(e))
        .map(|e| e.return_type.clone())
        .collect())
}
