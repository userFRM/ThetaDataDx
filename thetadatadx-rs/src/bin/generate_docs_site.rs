// Reason: build_support_bin modules are string-heavy code generators; pedantic lints are noise here.
#![allow(clippy::pedantic)]

//! Regenerate the checked-in docs-site reference tree from the endpoint,
//! tick, and streaming-event registries.
//!
//! Emits one reference page per registry endpoint (language tab carousel,
//! parameters, response schema, capture-backed sample output), the
//! streaming reference pages, the subscriptions matrix, the sidebar
//! manifests consumed by `docs-site/docs/.vitepress/config.ts`, and the
//! site's `llms.txt` index. `--check` verifies the checked-in tree matches
//! the current registries without writing.

use std::path::PathBuf;

// This binary reaches only the docs-site pair of the shared generator
// tree; the SDK-surface emitters and their re-exports in the same tree
// stay unreferenced here (they belong to `generate_sdk_surfaces`), so
// the dead-code lints are scoped off for this compile unit.
#[allow(dead_code, unused_imports)]
#[path = "../../build_support_bin/mod.rs"]
mod build_support_bin;

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .expect("crate manifest should live under <repo>/thetadatadx-rs")
        .to_path_buf()
}

fn package_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let check_only = std::env::args().skip(1).any(|arg| arg == "--check");
    std::env::set_current_dir(package_root())?;

    let root = repo_root();
    if check_only {
        build_support_bin::check_docs_site_files(&root)?;
        println!("docs site: generated pages match the registries");
    } else {
        build_support_bin::write_docs_site_files(&root)?;
        println!("docs site: generated pages written");
    }
    Ok(())
}
