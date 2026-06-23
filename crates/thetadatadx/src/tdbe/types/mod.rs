//! Wire data types for the streaming data layer: tick structs, protocol enums, and
//! the fixed-point [`price::Price`].
//!
//! Splits into three leaves — [`enums`] (wire enum taxonomy), [`price`]
//! (variable-precision fixed-point price), and [`tick`] (per-tick
//! `#[repr(C)]` structs) — and re-exports each as a flat facade so callers
//! reach the leaf types directly off `types`.

pub mod enums;
pub mod price;
pub mod tick;

// Generator-emitted modules live in `generated/`. The submodule
// itself is empty (a doc hub) — the actual files are reached via
// `include!("generated/<name>.rs")` from the hand-written
// `enums.rs` / `tick.rs` siblings, so the feature gates and
// hand-written `impl` blocks keep their place above each include site.
mod generated;

// Flat facade for the `types` submodule. Callers and the crate root
// reach the leaf modules (`types::tick`, `types::enums`, `types::price`)
// directly, so `unused_imports` is allowed on the convenience surface.
//
// The fixed-point price encoding (`price::Price` and friends) is wire-internal
// and stays off this facade; the decode layer reaches `types::price` directly.
#[allow(unused_imports)]
pub use enums::*;
#[allow(unused_imports)]
pub use tick::*;
