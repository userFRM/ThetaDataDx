//! Build-time generator orchestration for `thetadatadx`.
//!
//! The build pipeline has these responsibilities:
//! - generate the trade/quote condition tables from `data/*.toml`
//!   into the internal `tdbe` module;
//! - generate endpoint-facing surfaces from the explicit endpoint
//!   spec plus the upstream wire contract in `proto/mdds.proto`;
//! - generate tick decoders from `tick_schema.toml`.
//!
//! The gRPC client stubs are NOT compiled here on a normal build: they
//! are pre-generated and committed at `proto/beta_endpoints.snapshot.rs`
//! (included directly by `src/lib.rs`), so a default build needs no
//! `protoc`. Regenerating the snapshot and drift-checking it against
//! `proto/mdds.proto` (`grpc::run` / `grpc::check`, which drive
//! `prost-build` and shell out to `protoc`) is gated behind the
//! `grpc-codegen` feature.

mod conditions;
mod endpoints;
#[cfg(feature = "grpc-codegen")]
mod grpc;
mod ticks;

/// Runs the build-time generation pipeline: condition tables, endpoint
/// surfaces, and tick decoders. Under the `grpc-codegen` feature it also
/// regenerates the committed gRPC snapshot and drift-checks it (needs
/// `protoc`); a default build skips that and uses the committed snapshot.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    conditions::generate()?;
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
    endpoints::generate_all()?;
    ticks::generate()?;
    Ok(())
}
