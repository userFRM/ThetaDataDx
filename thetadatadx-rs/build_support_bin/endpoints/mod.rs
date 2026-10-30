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
