//! Endpoint surface generation reachable from `build.rs`.
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
//! Module layout (build-script compile unit):
//! * [`model`] — plain data types shared across parse and emit.
//! * [`parser`] — TOML + proto parsing, template/param-group resolution,
//!   and the `ParsedEndpoints` intermediate form.
//! * [`helpers`] — pure mapping and naming utilities used by the
//!   build-script emitter (`render::build_out`).
//! * [`render`] — `OUT_DIR` artifact emitters (registry / runtime / MDDS).
//!
//! The checked-in SDK projections (Python / TypeScript / C++ / FFI /
//! validators) and the live-validator parameter-mode matrix live in
//! `build_support_bin/endpoints/`. They share the core data types above
//! via `#[path]` but never enter this compile unit.

mod build_helpers;
pub(super) mod helpers;
pub(super) mod model;
pub(super) mod parser;
pub(super) mod proto_parser;
mod render;

pub use render::generate_all;
