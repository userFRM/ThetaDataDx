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
//! `check()` reruns the codegen and confirms the output is byte-
//! identical to the file already on disk; it runs unconditionally
//! from `build_support::run` so drift between `mdds.proto` and the
//! committed stubs surfaces as a build failure on both local
//! pre-push and CI.

use std::env;
use std::fs;
use std::path::PathBuf;

use prost_build::{Method, Service, ServiceGenerator};

/// Run the gRPC codegen step under `cargo build`.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = prost_build::Config::new();
    config.service_generator(Box::new(InhouseServiceGenerator::new()));
    config.compile_protos(&["proto/mdds.proto"], &["proto"])?;
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

/// Re-run the codegen and confirm the output matches the file already
/// on disk. Used by CI to detect drift between `mdds.proto` (or the
/// codegen logic) and the previously generated stubs.
///
/// # Errors
///
/// Returns an error when the regenerated output differs from the
/// previous run, or when the file cannot be read.
pub fn check() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = env::var("OUT_DIR")?;
    let target = PathBuf::from(&out_dir).join("beta_endpoints.rs");

    if !target.exists() {
        return Err(
            format!("expected codegen output at {target:?} — run `cargo build` first").into(),
        );
    }

    // Snapshot the existing output before re-running prost-build.
    let before = fs::read(&target)?;

    let mut config = prost_build::Config::new();
    config.service_generator(Box::new(InhouseServiceGenerator::new()));
    config.out_dir(&out_dir);
    config.compile_protos(&["proto/mdds.proto"], &["proto"])?;

    let after = fs::read(&target)?;
    if before != after {
        return Err(format!(
            "codegen drift detected at {target:?}; regenerate by running `cargo build`"
        )
        .into());
    }
    Ok(())
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
