//! Endpoint surface generation and validation.
//!
//! This module treats `endpoint_surface.toml` as the checked-in source of truth
//! for the normalized SDK surface, while still validating each declared
//! endpoint against the upstream gRPC wire contract in `proto/external.proto`.
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
