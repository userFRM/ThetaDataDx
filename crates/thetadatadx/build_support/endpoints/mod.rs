//! Endpoint surface generation and validation.
//!
//! This module treats `endpoint_surface.toml` as the checked-in source of truth
//! for the normalized SDK surface, while still validating each declared
//! endpoint against the upstream gRPC wire contract in `proto/mdds.proto`.
//! The resulting joined model drives generated registry metadata, the shared
//! endpoint runtime, and all `MddsClient` methods (list, parsed, and streaming).
//!
//! Note: runtime parameter validation (date format, symbol format, interval,
//! right, year) lives in `crate::validate`. The validators here operate at
//! *build time* on the TOML surface spec and proto schema — a fundamentally
//! different domain — so they are intentionally separate.
//!
//! Module layout:
//! * [`model`] — plain data types shared across parse and emit.
//! * [`parser`] — TOML + proto parsing, template/param-group resolution,
//!   cross-validation, and the `ParsedEndpoints` intermediate form.
//! * [`helpers`] — pure mapping and naming utilities used by every renderer.
//! * [`modes`] — live-validator parameter-mode matrix derivation.
//! * [`render`] — one emitter per target (Rust OUT_DIR, per-language SDKs,
//!   per-language validators).

// Reason: shared between build.rs and generate_sdk_surfaces binary via #[path]; not all
// functions are used from both entry points.
#![allow(dead_code, unused_imports)]

mod helpers;
mod model;
mod modes;
mod parser;
mod proto_parser;
mod render;

pub use render::{check_sdk_generated_files, generate_all, write_sdk_generated_files};

/// Compute the set of tick return-type names (e.g. `"OhlcTicks"`,
/// `"CalendarDays"`) that are reached by any snapshot-kind endpoint in
/// `endpoint_surface.toml`. Consumed by the `ticks` generator to decide
/// which tick types get a `<tick>_vec_to_pylist` fast-path converter
/// emitted into `sdks/python/src/tick_classes.rs` — emitting for every
/// tick type would leave dead-code fns (each `_vec_to_pylist` is only
/// called from snapshot-endpoint pymethods, so a tick type with no
/// snapshot endpoint has no caller).
///
/// Single SSOT: classification logic lives in [`helpers::is_snapshot_endpoint`]
/// and is driven entirely by TOML `category` / `subcategory` / `kind`
/// fields — no hand-curated allowlist, so adding a snapshot endpoint of
/// a new tick type automatically opts its converter into emission.
pub fn snapshot_return_types(
) -> Result<std::collections::HashSet<String>, Box<dyn std::error::Error>> {
    let endpoints = parser::load_endpoint_specs()?;
    Ok(endpoints
        .endpoints
        .iter()
        .filter(|e| helpers::is_snapshot_endpoint(e))
        .map(|e| e.return_type.clone())
        .collect())
}
