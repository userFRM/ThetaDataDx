//! Snapshot refresh path for the `refresh_grpc_snapshot` developer
//! binary.
//!
//! Kept separate from `grpc/mod.rs` so the build script (which only
//! includes `mod.rs` via `build_support/mod.rs`) cannot accidentally
//! pull in the write path. The build script side stays strictly
//! read-only; any source-tree mutation lives behind
//! `cargo run -p thetadatadx-rs --bin refresh_grpc_snapshot
//! --features grpc-codegen`.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::codegen::{regenerate_into_scratch, snapshot_path};

/// Regenerate the gRPC codegen output and overwrite the committed
/// snapshot at `proto/beta_endpoints.snapshot.rs`.
///
/// # Errors
///
/// Returns an error if prost-build fails to compile the proto, the
/// scratch read fails, or the snapshot write fails.
pub fn refresh_snapshot() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let fresh_bytes = regenerate_into_scratch()?;
    let snapshot_path = snapshot_path();
    write_snapshot(&snapshot_path, &fresh_bytes)?;
    Ok(snapshot_path)
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
