pub mod enums;
pub mod price;
pub mod tick;

// Generator-emitted modules live in `generated/`. The submodule
// itself is empty (a doc hub) — the actual files are reached via
// `include!("generated/<name>.rs")` from the hand-written
// `enums.rs` / `tick.rs` siblings, so the feature gates and
// hand-written `impl` blocks keep their place above each include site.
mod generated;

pub use enums::*;
pub use price::Price;
pub use tick::*;
