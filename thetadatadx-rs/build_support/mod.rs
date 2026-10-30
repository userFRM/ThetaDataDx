//! Build-time generator orchestration for `thetadatadx`.
//!
//! The build pipeline has these responsibilities:
//! - generate tick decoders from `tick_schema.toml`.
//!
//! The endpoint surface is not generated here: the market-data methods,
//! builders, registry and dispatch expand from the endpoint table in
//! `src/surface/endpoints.rs`.
//!
//! The gRPC client stubs are NOT compiled here on a normal build: they
//! are pre-generated and committed at `proto/beta_endpoints.snapshot.rs`
//! (included directly by `src/lib.rs`), so a default build needs no
//! `protoc`. Regenerating the snapshot and drift-checking it against
//! `proto/mdds.proto` (`grpc::run` / `grpc::check`, which drive
//! `prost-build` and shell out to `protoc`) is gated behind the
//! `grpc-codegen` feature.

#[cfg(feature = "grpc-codegen")]
mod grpc;
mod ticks;

/// Runs the build-time generation pipeline: the tick decoders. Under the
/// `grpc-codegen` feature it also regenerates the committed gRPC snapshot and
/// drift-checks it (needs `protoc`); a default build skips that and uses the
/// committed snapshot.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // gRPC stubs ship pre-generated at `proto/beta_endpoints.snapshot.rs`.
    // Only regenerate + drift-check them under `grpc-codegen`, so a normal
    // build invokes no `protoc`. The check is read-only; refresh the
    // snapshot with the `refresh_grpc_snapshot` binary (see `grpc/mod.rs`).
    #[cfg(feature = "grpc-codegen")]
    {
        grpc::run()?;
        grpc::check()?;
    }
    // `src/lib.rs` includes the committed snapshot, so recompile the crate
    // when it — or the proto it is generated from — changes. Emitted on
    // every build, including the default one that skips the block above.
    println!("cargo:rerun-if-changed=proto/beta_endpoints.snapshot.rs");
    println!("cargo:rerun-if-changed=proto/mdds.proto");
    ticks::generate()?;
    Ok(())
}
