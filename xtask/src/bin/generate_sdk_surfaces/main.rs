// Reason: these modules are string-heavy code generators; pedantic lints are noise here.
#![allow(clippy::pedantic)]
// The endpoint table expands through `__resolve!`, one recursion per parameter key.
#![recursion_limit = "256"]

//! Regenerate the checked-in SDK wrapper surfaces and the docs site from the
//! surface tables.
//!
//! Normal crate builds generate nothing. This helper keeps the
//! checked-in FFI/SDK projections and docs pages explicit so CI can verify
//! drift without mutating files as a side effect of `cargo build`. `--check`
//! verifies the checked-in tree without writing.

mod endpoints;
mod fpss_events;
mod ticks;
mod upstream_openapi;
#[path = "../../../../thetadatadx-rs/src/mdds/wire_semantics.rs"]
mod wire_semantics;

use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let check_only = std::env::args().skip(1).any(|arg| arg == "--check");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the xtask manifest lives under <repo>/xtask");
    // The generators read `proto/mdds.proto` and the vendor snapshot relative
    // to the core crate.
    std::env::set_current_dir(root.join("thetadatadx-rs"))?;

    if check_only {
        endpoints::check_sdk_generated_files(root)?;
        ticks::check_sdk_generated_files(root)?;
        fpss_events::check_sdk_generated_files(root)?;
        endpoints::check_docs_site_files(root)?;
    } else {
        endpoints::write_sdk_generated_files(root)?;
        ticks::write_sdk_generated_files(root)?;
        fpss_events::write_sdk_generated_files(root)?;
        endpoints::write_docs_site_files(root)?;
    }
    Ok(())
}
