//! Endpoint surface generation for the `generate_sdk_surfaces` binary.
//!
//! This tree expands the endpoint table into plain records and renders the
//! per-language SDK projections, the live-validator matrix and the docs site
//! from them.
//!
//! Module layout:
//! * [`table`]: the endpoint table as records, and its proto cross-check.
//! * [`helpers`]: cross-renderer helpers.
//! * [`modes`]: live-validator parameter-mode matrix derivation.
//! * [`test_fixtures`]: the values the validators and examples send.
//! * [`sdk_helpers`]: render-side helpers.
//! * [`sdk_render`]: per-language emitters.
//! * [`docs_render`]: the docs site.

mod docs_render;
mod enum_projection;
mod helpers;
mod modes;
mod sdk_helpers;
mod sdk_render;
mod table;
mod test_fixtures;

use std::path::Path;

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
    Ok(table::endpoints()
        .iter()
        .filter(|e| sdk_helpers::snapshot_returns_plain_pylist(e))
        .map(|e| e.return_type.to_string())
        .collect())
}
