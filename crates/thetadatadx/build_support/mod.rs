//! Build-time generator orchestration for `thetadatadx`.
//!
//! The build pipeline has two distinct responsibilities:
//! - generate tick decoders from `tick_schema.toml`
//! - generate endpoint-facing surfaces from the explicit endpoint spec plus
//!   the upstream wire contract in `proto/external.proto`

mod endpoints;
mod ticks;
mod upstream_openapi;
// Runtime wire-canonicalization rules, reused at build time via `#[path]`.
// Everything in this file is consumed at runtime; build-support extensions
// live in the sibling `wire_semantics` module below.
#[path = "../src/wire_semantics.rs"]
mod wire_semantics_runtime;
// Build-time-only extensions (mode-collapsing sentinel + canonicalizer).
mod wire_semantics;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&["proto/external.proto"], &["proto"])?;

    endpoints::generate_all()?;
    ticks::generate()?;

    Ok(())
}
