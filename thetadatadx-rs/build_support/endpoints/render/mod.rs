//! Per-language emitters reachable from `build.rs`.
//!
//! The two submodules here own the `OUT_DIR` artifacts consumed at
//! `include!()` time by the main crate (registry, dispatch runtime, and
//! the three per-kind `MarketDataClient` extension impls). Checked-in SDK
//! projections (Python / TypeScript / C++ / FFI / validators) live in
//! `build_support_bin/endpoints/sdk_render/` and never enter the build
//! script's compile unit.

mod build_out;
mod mdds;

pub use build_out::generate_all;
