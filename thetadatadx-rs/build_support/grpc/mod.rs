//! Build-time gRPC client codegen entry points.
//!
//! Replaces `tonic-prost-build`. The build step reads
//! `proto/mdds.proto`, invokes `prost-build` for the message types,
//! and installs a custom service generator (see [`codegen`]) that
//! emits one async function per RPC method.
//!
//! Each emitted function has shape:
//!
//! ```ignore
//! pub async fn <method_snake_case>(
//!     channel: &crate::grpc::Channel,
//!     req: <RequestType>,
//! ) -> Result<crate::grpc::ServerStreaming<<ResponseType>>, crate::grpc::ChannelError>
//! ```
//!
//! The stubs land in `$OUT_DIR/beta_endpoints.rs` alongside the prost
//! message types — they are wrapped in a `pub mod
//! beta_theta_terminal {}` block so callers reach them via
//! `crate::proto::beta_theta_terminal::<method>(channel, req)`.
//!
//! ## Drift detection
//!
//! [`check`] regenerates the codegen output into a side-buffer and
//! compares it byte-for-byte against the committed snapshot at
//! `proto/beta_endpoints.snapshot.rs`. The snapshot is git-tracked
//! so any drift between `mdds.proto` (or the codegen logic) and the
//! committed stub surface fails the build on both local pre-push
//! and CI — same posture as gofmt diff or rustfmt --check in CI
//! pipelines.
//!
//! [`check`] is read-only: it never writes to the source tree. When
//! the snapshot needs to be regenerated (intentional proto changes),
//! run the dedicated binary:
//!
//! ```sh
//! cargo run -p thetadatadx-rs \
//!     --bin refresh_grpc_snapshot \
//!     --features grpc-codegen
//! ```
//!
//! Then commit the resulting `proto/beta_endpoints.snapshot.rs` diff
//! alongside the proto change.

use std::fs;

mod codegen;

/// Run the gRPC codegen step under `cargo build`.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = prost_build::Config::new();
    config.service_generator(Box::new(codegen::ChannelServiceGenerator::new()));
    config.compile_protos(&["proto/mdds.proto"], &["proto"])?;
    // Rebuild when the committed snapshot changes so `check()` picks
    // up edits to it (e.g. an intentional regen the developer just
    // committed) without a `cargo clean`.
    println!("cargo:rerun-if-changed={}", codegen::SNAPSHOT_PATH);
    Ok(())
}

/// Regenerate the codegen output and confirm it matches the committed
/// snapshot at [`SNAPSHOT_PATH`].
///
/// The snapshot is git-tracked so this check fires whenever
/// `mdds.proto` (or the codegen logic) drifts from the version on
/// disk — both locally during `cargo build` and on every CI run.
///
/// This function is read-only: it never writes the source tree. When
/// the snapshot legitimately needs to change (proto edits), run the
/// `refresh_grpc_snapshot` binary; see the module docs.
///
/// # Errors
///
/// Returns an error when the regenerated codegen output differs
/// from the committed snapshot, when the snapshot does not exist,
/// or when prost-build fails to compile the proto.
pub fn check() -> Result<(), Box<dyn std::error::Error>> {
    let fresh_bytes = codegen::regenerate_into_scratch()?;
    let snapshot_path = codegen::snapshot_path();

    if !snapshot_path.exists() {
        return Err(format!(
            "codegen snapshot missing at {}; \
            regenerate with `cargo run -p thetadatadx-rs --bin refresh_grpc_snapshot --features grpc-codegen` \
            and commit the resulting snapshot",
            snapshot_path.display()
        )
        .into());
    }

    let committed_bytes = fs::read(&snapshot_path).map_err(|e| {
        format!(
            "failed to read committed snapshot at {}: {e}",
            snapshot_path.display()
        )
    })?;

    // Normalize line endings on both sides before comparing. The
    // committed snapshot is canonicalized to LF in `.gitattributes`,
    // but a misconfigured checkout (e.g. `core.autocrlf=true` on
    // Windows without the attribute applied) could still surface
    // CRLF on disk. Stripping `\r` on both sides keeps the drift
    // check resilient to checkout-side translation.
    let committed_lf = strip_cr(&committed_bytes);
    let fresh_lf = strip_cr(&fresh_bytes);
    if committed_lf != fresh_lf {
        let diff_summary = describe_drift(&committed_lf, &fresh_lf);
        return Err(format!(
            "codegen drift detected between proto/mdds.proto and {snapshot}.\n\
            {diff_summary}\n\
            Refresh the snapshot with `cargo run -p thetadatadx-rs --bin refresh_grpc_snapshot --features grpc-codegen` and commit the resulting diff.",
            snapshot = snapshot_path.display(),
        )
        .into());
    }

    Ok(())
}

/// Strip CR bytes so the drift check compares logical content
/// regardless of checkout-side line-ending translation.
fn strip_cr(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().filter(|b| *b != b'\r').collect()
}

/// Produce a short, human-actionable diagnostic when the snapshot
/// drifts. Heuristics on byte counts and the first differing line
/// keep the build log readable without dragging in a full diff
/// engine — the user reruns the refresh binary to see the rewritten
/// snapshot.
fn describe_drift(committed: &[u8], fresh: &[u8]) -> String {
    let first_diff = committed
        .iter()
        .zip(fresh.iter())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| committed.len().min(fresh.len()));
    let line = byte_offset_to_line(committed, first_diff);
    format!(
        "  committed snapshot: {committed_len} bytes\n  regenerated output: {fresh_len} bytes\n  first byte differs at offset {first_diff} (line ~{line}).",
        committed_len = committed.len(),
        fresh_len = fresh.len(),
    )
}

/// 1-indexed line number for a byte offset into the file.
fn byte_offset_to_line(bytes: &[u8], offset: usize) -> usize {
    bytes[..offset.min(bytes.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}
