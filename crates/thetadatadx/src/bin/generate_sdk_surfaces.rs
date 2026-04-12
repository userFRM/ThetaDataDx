#![allow(clippy::pedantic)]

//! Regenerate checked-in SDK wrapper surfaces from `endpoint_surface.toml`.
//!
//! Normal crate builds only emit `OUT_DIR` artifacts. This helper keeps the
//! checked-in FFI/SDK projections explicit so CI can verify drift without
//! mutating files as a side effect of `cargo build`.

use std::path::PathBuf;

#[allow(dead_code)]
#[path = "../../build_support/endpoints.rs"]
mod endpoints;

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
    } else {
        endpoints::write_sdk_generated_files(&repo_root())?;
    }
    Ok(())
}
