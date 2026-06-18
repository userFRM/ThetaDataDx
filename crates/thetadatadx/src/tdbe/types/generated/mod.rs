//! Generator-emitted modules.
//!
//! Holds the three `*_generated.rs` files the schema-driven code
//! generator produces, kept in a dedicated subdirectory so the on-disk
//! separation between human-authored and generated output is unambiguous.
//!
//! All three are `include!`-ed into a hand-written sibling so the
//! feature gates, hand-written `impl` blocks, and rustdoc above each
//! `include!` site keep their place. This `mod.rs` exists as the
//! re-export hub the file paths point to; nothing here is imported
//! directly from outside the `tdbe` data-format module.
//!
//! Files:
//!
//! - [`enums_endpoint`] — `Endpoint` / `EndpointMeta` enum bodies, included
//!   from `super::enums`.
//! - [`tick`] — `#[repr(C, align(N))]` tick struct definitions, included
//!   from `super::tick`.
//! - [`tick_layout_asserts`] — compile-time layout asserts pinned against
//!   the schema-derived figures the C FFI mirror and
//!   `tick_layout_asserts.hpp.inc` rely on, included from `super::tick`.

// The `*.rs` files are reached via `include!("generated/<name>.rs")`
// from the hand-written modules above, so no `mod` declarations are
// emitted here. This mod.rs is the re-export hub the file paths point to.
