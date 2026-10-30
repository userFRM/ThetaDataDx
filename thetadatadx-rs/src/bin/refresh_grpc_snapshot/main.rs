//! Regenerate the committed gRPC codegen snapshot.
//!
//! Run when `proto/mdds.proto` (or the service generator in
//! `codegen.rs`) changes intentionally:
//!
//! ```sh
//! cargo run -p thetadatadx-rs \
//!     --bin refresh_grpc_snapshot \
//!     --features grpc-codegen
//! ```
//!
//! Commit the resulting `proto/beta_endpoints.snapshot.rs` diff
//! alongside the proto change. CI runs this binary and fails when it
//! changes the committed snapshot.

mod codegen;

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // prost-build resolves `proto/mdds.proto` against the current directory.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    std::env::set_current_dir(&manifest_dir)?;

    let fresh_bytes = codegen::regenerate_into_scratch()?;
    let written = codegen::snapshot_path();
    write_snapshot(&written, &fresh_bytes)?;
    println!(
        "refreshed gRPC codegen snapshot at {} ({} bytes). \
        Review and commit the diff alongside any proto change.",
        written.display(),
        fresh_bytes.len()
    );
    Ok(())
}

/// Persist the snapshot atomically: write to a sibling temp file,
/// then rename onto the final path. Avoids leaving a half-written
/// snapshot if the regen is interrupted mid-write.
fn write_snapshot(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("snapshot path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension("rs.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}
