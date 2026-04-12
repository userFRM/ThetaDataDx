//! Build-time generator orchestration for `thetadatadx`.
//!
//! The build pipeline has two distinct responsibilities:
//! - generate tick decoders from `tick_schema.toml`
//! - generate endpoint-facing surfaces from the explicit endpoint spec plus
//!   the upstream wire contract in `proto/external.proto`

mod endpoints;
mod ticks;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&["proto/external.proto"], &["proto"])?;

    endpoints::generate_all()?;
    ticks::generate()?;

    // Write checked-in SDK surface files (Go, C++, Python, FFI) so that a
    // plain `cargo build` keeps them in sync with endpoint_surface.toml.
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate manifest should live under <repo>/crates/thetadatadx")
        .to_path_buf();
    endpoints::write_sdk_generated_files(&repo_root)?;

    Ok(())
}
