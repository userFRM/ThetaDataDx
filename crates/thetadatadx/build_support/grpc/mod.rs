//! Build-time gRPC client codegen.
//!
//! Replaces `tonic-prost-build`. The build step reads
//! `proto/mdds.proto`, invokes `prost-build` for the message types,
//! and installs a custom [`ServiceGenerator`] that emits one async
//! function per RPC method.
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
//! `check()` regenerates the codegen output into a side-buffer and
//! compares it byte-for-byte against the committed snapshot at
//! `proto/beta_endpoints.snapshot.rs`. The snapshot is git-tracked
//! so any drift between `mdds.proto` (or the codegen logic) and the
//! committed stub surface fails the build on both local pre-push
//! and CI — same posture as gofmt diff or rustfmt --check in CI
//! pipelines.
//!
//! When the snapshot needs to be regenerated (intentional proto
//! changes), set `THETADATADX_GRPC_REGEN=1` before `cargo build`.
//! The build script then writes the freshly-generated output back
//! over the snapshot file in-tree. Commit the resulting snapshot
//! diff alongside the proto change.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use prost_build::{Method, Service, ServiceGenerator};

/// Path to the committed codegen snapshot, relative to the crate
/// manifest dir. The build script reads / writes this file directly
/// — it is the source of truth for drift detection.
const SNAPSHOT_PATH: &str = "proto/beta_endpoints.snapshot.rs";

/// Environment variable that, when set to `1`, instructs `check()`
/// to overwrite the committed snapshot rather than fail on drift.
/// Local workflow: edit `mdds.proto`, run `THETADATADX_GRPC_REGEN=1
/// cargo build`, commit `proto/beta_endpoints.snapshot.rs` alongside
/// the proto change.
const REGEN_ENV: &str = "THETADATADX_GRPC_REGEN";

/// Run the gRPC codegen step under `cargo build`.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = prost_build::Config::new();
    config.service_generator(Box::new(InhouseServiceGenerator::new()));
    config.compile_protos(&["proto/mdds.proto"], &["proto"])?;
    // Rebuild when the committed snapshot changes so `check()` picks
    // up edits to it (e.g. an intentional regen the developer just
    // committed) without a `cargo clean`.
    println!("cargo:rerun-if-changed={SNAPSHOT_PATH}");
    // Honour the regen env var as a build input so cargo invalidates
    // the build cache when it flips on / off.
    println!("cargo:rerun-if-env-changed={REGEN_ENV}");
    Ok(())
}

/// Service generator that emits in-house gRPC client stubs.
struct InhouseServiceGenerator;

impl InhouseServiceGenerator {
    fn new() -> Self {
        Self
    }

    /// Emit one server-streaming function for `method` into `buf`.
    fn emit_method(service: &Service, method: &Method, buf: &mut String) {
        // Method path per the gRPC HTTP/2 spec:
        //   /<package>.<service>/<MethodCamelCase>
        let path = format!(
            "/{}.{}/{}",
            service.package, service.proto_name, method.proto_name
        );

        // Preserve doc comments from the proto, line-by-line.
        for line in &method.comments.leading {
            buf.push_str("    /// ");
            buf.push_str(line.trim_end());
            buf.push('\n');
        }
        buf.push_str("    ///\n    /// gRPC method: `");
        buf.push_str(&path);
        buf.push_str("`.\n    ///\n    /// # Errors\n    ///\n");
        buf.push_str(
            "    /// Returns a [`crate::grpc::ChannelError`] when the RPC fails to open\n",
        );
        buf.push_str("    /// or the server's response head is malformed.\n");

        buf.push_str("    pub async fn ");
        buf.push_str(&method.name);
        buf.push_str("(\n");
        buf.push_str("        channel: &crate::grpc::Channel,\n");
        buf.push_str("        req: super::");
        buf.push_str(&method.input_type);
        buf.push_str(",\n");
        buf.push_str("    ) -> Result<crate::grpc::ServerStreaming<super::");
        buf.push_str(&method.output_type);
        buf.push_str(">, crate::grpc::ChannelError> {\n");
        buf.push_str("        channel\n");
        buf.push_str("            .server_streaming::<super::");
        buf.push_str(&method.input_type);
        buf.push_str(", super::");
        buf.push_str(&method.output_type);
        buf.push_str(">(\n                \"");
        buf.push_str(&path);
        buf.push_str("\",\n                req,\n            )\n            .await\n");
        buf.push_str("    }\n\n");
    }
}

impl ServiceGenerator for InhouseServiceGenerator {
    fn generate(&mut self, service: Service, buf: &mut String) {
        // The proto exposes only server-streaming RPCs. Reject anything
        // else so a future proto change does not silently mis-frame.
        for method in &service.methods {
            assert!(
                !method.client_streaming,
                "client-streaming RPCs are not supported by the in-house transport: {}.{}",
                service.proto_name, method.proto_name
            );
            assert!(
                method.server_streaming,
                "unary RPCs are not supported by the in-house transport: {}.{}",
                service.proto_name, method.proto_name
            );
        }

        // Wrap the emitted functions in a module named after the
        // service (snake_case) so callers reach them via
        // `crate::proto::<service>::<method>(channel, req)`.
        let mod_name = to_snake_case(&service.proto_name);
        buf.push_str("/// In-house gRPC client stubs for the `");
        buf.push_str(&service.proto_name);
        buf.push_str("` service.\n");
        buf.push_str("///\n/// Generated from `proto/mdds.proto` by\n");
        buf.push_str("/// `crates/thetadatadx/build_support/grpc/`.\n");
        buf.push_str("pub mod ");
        buf.push_str(&mod_name);
        buf.push_str(" {\n");
        for method in &service.methods {
            Self::emit_method(&service, method, buf);
        }
        buf.push_str("}\n");
    }
}

/// Convert `BetaThetaTerminal` to `beta_theta_terminal`. Matches the
/// convention `prost-build` uses elsewhere; kept local so the codegen
/// does not depend on prost-build's private helpers.
fn to_snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.char_indices() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

/// Regenerate the codegen output and confirm it matches the committed
/// snapshot at [`SNAPSHOT_PATH`].
///
/// The snapshot is git-tracked so this check fires whenever
/// `mdds.proto` (or the codegen logic) drifts from the version on
/// disk — both locally during `cargo build` and on every CI run.
///
/// When the [`REGEN_ENV`] environment variable is set to `1`, the
/// freshly-generated bytes are written back over the snapshot
/// instead of triggering a drift failure. Use this for intentional
/// proto changes, then commit the snapshot delta alongside the
/// proto change.
///
/// # Errors
///
/// Returns an error when the regenerated codegen output differs
/// from the committed snapshot, when the snapshot does not exist
/// (and regen mode is not enabled), or when prost-build fails to
/// compile the proto.
pub fn check() -> Result<(), Box<dyn std::error::Error>> {
    // Generate into a dedicated temporary directory so we never
    // accidentally compare the OUT_DIR copy against itself (the bug
    // the previous shape had — both sides of the diff came from the
    // same file).
    let scratch = scratch_dir()?;
    let mut config = prost_build::Config::new();
    config.service_generator(Box::new(InhouseServiceGenerator::new()));
    config.out_dir(&scratch);
    config.compile_protos(&["proto/mdds.proto"], &["proto"])?;
    let fresh_path = scratch.join("beta_endpoints.rs");
    let fresh_bytes = fs::read(&fresh_path)
        .map_err(|e| format!("failed to read regenerated codegen at {fresh_path:?}: {e}"))?;

    let snapshot_path = snapshot_path();
    let regen = env::var(REGEN_ENV).is_ok_and(|v| v == "1");

    if regen {
        // Snapshot refresh mode: write the freshly-generated bytes
        // back over the committed snapshot. The user commits the
        // resulting diff alongside their proto change.
        write_snapshot(&snapshot_path, &fresh_bytes)?;
        println!(
            "cargo:warning=Regenerated codegen snapshot at {} ({} bytes). Commit alongside the proto change.",
            snapshot_path.display(),
            fresh_bytes.len()
        );
        return Ok(());
    }

    if !snapshot_path.exists() {
        return Err(format!(
            "codegen snapshot missing at {}; run `{REGEN_ENV}=1 cargo build` once and commit the snapshot",
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
            Refresh the snapshot with `{REGEN_ENV}=1 cargo build` and commit the resulting diff.",
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

/// Path to the committed codegen snapshot, anchored at
/// `CARGO_MANIFEST_DIR`. Build scripts run with cwd = manifest dir
/// already, but resolving via the env var is robust against any
/// future change in cargo's behaviour.
fn snapshot_path() -> PathBuf {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default()).join(SNAPSHOT_PATH)
}

/// A scratch directory under `OUT_DIR` for the drift-detection
/// regeneration. Distinct from the main codegen output dir so the
/// two copies stay separable and the diff is meaningful.
fn scratch_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let out_dir = env::var_os("OUT_DIR").ok_or("OUT_DIR not set; not running under cargo")?;
    let dir = PathBuf::from(out_dir).join("grpc-codegen-check");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Persist the snapshot atomically: write to a sibling temp file,
/// then rename onto the final path. Avoids leaving a half-written
/// snapshot if the build is interrupted mid-write.
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

/// Produce a short, human-actionable diagnostic when the snapshot
/// drifts. Heuristics on byte counts and the first differing line
/// keep the build log readable without dragging in a full diff
/// engine — the user reruns `cargo build` with the regen env var
/// to see the rewritten snapshot.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_basic() {
        assert_eq!(to_snake_case("BetaThetaTerminal"), "beta_theta_terminal");
        assert_eq!(to_snake_case("HTTPServer"), "h_t_t_p_server");
        assert_eq!(to_snake_case("simple"), "simple");
        assert_eq!(to_snake_case(""), "");
    }
}
