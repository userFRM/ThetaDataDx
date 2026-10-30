// Reason: build_support_bin modules are string-heavy code generators; pedantic lints are noise here.
#![allow(clippy::pedantic)]
// The endpoint table expands through `__resolve!`, one recursion per parameter key.
#![recursion_limit = "256"]

//! Regenerate the checked-in SDK wrapper surfaces and the docs site from the
//! endpoint table and the schema files.
//!
//! Normal crate builds generate nothing. This helper keeps the
//! checked-in FFI/SDK projections and docs pages explicit so CI can verify
//! drift without mutating files as a side effect of `cargo build`. `--check`
//! verifies the checked-in tree without writing.

use std::path::PathBuf;

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
        build_support_bin::check_endpoint_sdk_generated_files(&root)?;
        build_support_bin::check_tick_sdk_generated_files(&root)?;
        build_support_bin::check_fpss_event_sdk_generated_files(&root)?;
        build_support_bin::check_docs_site_files(&root)?;
    } else {
        build_support_bin::write_endpoint_sdk_generated_files(&root)?;
        build_support_bin::write_tick_sdk_generated_files(&root)?;
        build_support_bin::write_fpss_event_sdk_generated_files(&root)?;
        build_support_bin::write_docs_site_files(&root)?;
    }
    Ok(())
}
