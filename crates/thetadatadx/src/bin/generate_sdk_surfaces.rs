// Reason: build_support modules are string-heavy code generators; pedantic lints are noise here.
#![allow(clippy::pedantic)]

//! Regenerate checked-in SDK wrapper surfaces from `endpoint_surface.toml`.
//!
//! Normal crate builds only emit `OUT_DIR` artifacts. This helper keeps the
//! checked-in FFI/SDK projections explicit so CI can verify drift without
//! mutating files as a side effect of `cargo build`.

use std::path::PathBuf;

// Reason: modules shared with build.rs via #[path]; many helpers are only called from build.rs.
#[allow(dead_code)]
#[path = "../../build_support/endpoints/mod.rs"]
mod endpoints;
#[allow(dead_code)]
#[path = "../../build_support/sdk_surface.rs"]
mod sdk_surface;
#[allow(dead_code)]
#[path = "../../build_support/ticks.rs"]
mod ticks;
#[allow(dead_code)]
#[path = "../../build_support/upstream_openapi.rs"]
mod upstream_openapi;

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

    if check_only {
        endpoints::check_sdk_generated_files(&repo_root())?;
        sdk_surface::check_sdk_generated_files(&repo_root())?;
        ticks::check_sdk_generated_files(&repo_root())?;
    } else {
        endpoints::write_sdk_generated_files(&repo_root())?;
        sdk_surface::write_sdk_generated_files(&repo_root())?;
        ticks::write_sdk_generated_files(&repo_root())?;
    }
    Ok(())
}
