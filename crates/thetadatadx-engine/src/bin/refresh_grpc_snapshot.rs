//! Regenerate the committed gRPC codegen snapshot.
//!
//! Run when `proto/mdds.proto` (or the in-house service generator
//! logic under `build_support/grpc/`) changes intentionally:
//!
//! ```sh
//! cargo run -p thetadatadx \
//!     --bin refresh_grpc_snapshot \
//!     --features grpc-codegen
//! ```
//!
//! Commit the resulting `proto/beta_endpoints.snapshot.rs` diff
//! alongside the proto change. The build script's `grpc::check()` is
//! read-only and refuses to write the source tree itself — that
//! responsibility lives here.

// Pulled in via `#[path]` so the bin and the build script compile
// the exact same generator. Only `codegen.rs` + `regen.rs` are
// referenced — never the `mod.rs` drift-check entry point — so the
// bin's compile unit has no dead code under `warnings = deny`.
#[path = "../../build_support/grpc/codegen.rs"]
mod codegen;
#[path = "../../build_support/grpc/regen.rs"]
mod regen;

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The build script runs with cwd = manifest dir; mirror that here
    // so the relative `proto/mdds.proto` lookup resolves identically.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::env::set_current_dir(&manifest_dir)?;

    let written = regen::refresh_snapshot()?;
    let bytes = std::fs::metadata(&written).map(|m| m.len()).unwrap_or(0);
    println!(
        "refreshed gRPC codegen snapshot at {} ({bytes} bytes). \
        Review and commit the diff alongside any proto change.",
        written.display()
    );
    Ok(())
}
