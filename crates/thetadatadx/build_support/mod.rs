//! Build-time generator orchestration for `thetadatadx`.
//!
//! The build pipeline has two distinct responsibilities:
//! - generate tick decoders from `tick_schema.toml`
//! - generate endpoint-facing surfaces from the explicit endpoint spec plus
//!   the upstream wire contract in `proto/mdds.proto`

mod endpoints;
mod fpss_events;
mod ticks;
mod upstream_openapi;
#[path = "../src/wire_semantics.rs"]
mod wire_semantics;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&["proto/mdds.proto"], &["proto"])?;

    endpoints::generate_all()?;
    ticks::generate()?;

    Ok(())
}
