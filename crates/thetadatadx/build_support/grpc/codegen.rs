//! Shared gRPC codegen helpers: the prost-build configuration, the
//! crate's [`ServiceGenerator`] implementation, scratch-dir
//! management, and snapshot-path resolution.
//!
//! This module is consumed by two compile units:
//!
//! * `build.rs` (via `build_support/grpc/mod.rs`) — runs prost-build
//!   into `$OUT_DIR/beta_endpoints.rs` plus a scratch dir for the
//!   read-only drift check.
//! * The `refresh_grpc_snapshot` developer binary (via
//!   `build_support/grpc/regen.rs`) — runs prost-build into the
//!   scratch dir, then overwrites the committed snapshot.
//!
//! Everything shared by those two paths lives here so the build
//! script and the developer binary cannot drift apart.

use std::env;
use std::fs;
use std::path::PathBuf;

use prost_build::{Method, Service, ServiceGenerator};

/// Path to the committed codegen snapshot, relative to the crate
/// manifest dir.
pub const SNAPSHOT_PATH: &str = "proto/beta_endpoints.snapshot.rs";

/// Service generator that emits client stubs dispatching through
/// `crate::grpc::Channel`.
pub struct ChannelServiceGenerator;

impl ChannelServiceGenerator {
    /// Constructs a [`ChannelServiceGenerator`].
    pub fn new() -> Self {
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
        // `{:?}` defensively escapes the proto-derived path so a
        // hostile package / service / method name (containing `"` or
        // `\`) cannot produce uncompilable generated source. Real
        // proto identifiers follow [A-Za-z_][A-Za-z0-9_]* so this is a
        // belt-and-braces guard, not a hot path.
        use std::fmt::Write as _;
        write!(buf, ">(\n                {path:?},").unwrap();
        buf.push_str("\n                req,\n            )\n            .await\n");
        buf.push_str("    }\n\n");
    }
}

impl ServiceGenerator for ChannelServiceGenerator {
    fn generate(&mut self, service: Service, buf: &mut String) {
        // The proto exposes only server-streaming RPCs. Reject anything
        // else so a future proto change does not silently mis-frame.
        for method in &service.methods {
            assert!(
                !method.client_streaming,
                "client-streaming RPCs are not supported by `grpc::Channel`: {}.{}",
                service.proto_name, method.proto_name
            );
            assert!(
                method.server_streaming,
                "unary RPCs are not supported by `grpc::Channel`: {}.{}",
                service.proto_name, method.proto_name
            );
        }

        // Wrap the emitted functions in a module named after the
        // service (snake_case) so callers reach them via
        // `crate::proto::<service>::<method>(channel, req)`.
        let mod_name = to_snake_case(&service.proto_name);
        buf.push_str("/// gRPC client stubs for the `");
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
pub fn to_snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.char_indices() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

/// Run prost-build into a fresh scratch directory and return the
/// resulting codegen bytes.
pub fn regenerate_into_scratch() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let scratch = scratch_dir()?;
    let mut config = prost_build::Config::new();
    config.service_generator(Box::new(ChannelServiceGenerator::new()));
    config.out_dir(&scratch);
    config.compile_protos(&["proto/mdds.proto"], &["proto"])?;
    let fresh_path = scratch.join("beta_endpoints.rs");
    fs::read(&fresh_path)
        .map_err(|e| format!("failed to read regenerated codegen at {fresh_path:?}: {e}").into())
}

/// Path to the committed codegen snapshot, anchored at
/// `CARGO_MANIFEST_DIR`. Build scripts run with cwd = manifest dir
/// already, but resolving via the env var is robust against any
/// future change in cargo's behaviour.
pub fn snapshot_path() -> PathBuf {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default()).join(SNAPSHOT_PATH)
}

/// A scratch directory under `OUT_DIR` (build script) or
/// `<manifest_dir>/target/grpc-codegen-scratch` (binary path, when
/// `OUT_DIR` is absent). Distinct from any committed location so the
/// regenerated bytes never land in-tree by accident.
fn scratch_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = if let Some(out_dir) = env::var_os("OUT_DIR") {
        PathBuf::from(out_dir).join("grpc-codegen-check")
    } else {
        // Binary entry point: no `OUT_DIR`. Anchor under the crate's
        // `target/` so the scratch lives in the conventional throwaway
        // location, not in the source tree.
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default())
            .join("target")
            .join("grpc-codegen-scratch")
    };
    fs::create_dir_all(&dir)?;
    Ok(dir)
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
