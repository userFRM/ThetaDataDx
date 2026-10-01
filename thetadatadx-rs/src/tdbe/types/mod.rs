//! Wire data types for the FPSS data layer: tick structs, protocol enums, and
//! the fixed-point [`price::Price`].
//!
//! Splits into three leaves — [`enums`] (wire enum taxonomy), [`price`]
//! (variable-precision fixed-point price), and [`tick`] (per-tick
//! `#[repr(C)]` structs). Callers reach the types through the leaf paths.

pub mod enums;
pub mod price;
pub mod tick;

// Generator-emitted modules live in `generated/`. The submodule
// itself is empty (a doc hub) — the actual files are reached via
// `include!("generated/<name>.rs")` from the hand-written
// `enums.rs` / `tick.rs` siblings, so the feature gates and
// hand-written `impl` blocks keep their place above each include site.
mod generated;
