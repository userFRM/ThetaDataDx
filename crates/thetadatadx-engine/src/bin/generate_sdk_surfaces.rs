// Reason: build_support_bin modules are string-heavy code generators; pedantic lints are noise here.
#![allow(clippy::pedantic)]

//! Regenerate checked-in SDK wrapper surfaces from `endpoint_surface.toml`.
//!
//! Normal crate builds only emit `OUT_DIR` artifacts. This helper keeps the
//! checked-in FFI/SDK projections explicit so CI can verify drift without
//! mutating files as a side effect of `cargo build`.

use std::path::PathBuf;

#[path = "../../build_support_bin/mod.rs"]
mod build_support_bin;

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate manifest should live under <repo>/crates/thetadatadx")
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
        build_support_bin::check_sdk_surface_generated_files(&root)?;
        build_support_bin::check_tick_sdk_generated_files(&root)?;
        build_support_bin::check_fpss_event_sdk_generated_files(&root)?;
    } else {
        build_support_bin::write_endpoint_sdk_generated_files(&root)?;
        build_support_bin::write_sdk_surface_generated_files(&root)?;
        build_support_bin::write_tick_sdk_generated_files(&root)?;
        build_support_bin::write_fpss_event_sdk_generated_files(&root)?;
    }
    Ok(())
}
