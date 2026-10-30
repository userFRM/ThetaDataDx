//! Generator-emitted modules.
//!
//! Holds the file the code generator produces, kept in a dedicated
//! subdirectory so the on-disk separation between human-authored and
//! generated output is unambiguous.
//!
//! It is `include!`-ed into a hand-written sibling so the
//! feature gates, hand-written `impl` blocks, and rustdoc above the
//! `include!` site keep their place. This `mod.rs` exists as the
//! re-export hub the file paths point to; nothing here is imported
//! directly from outside the `tdbe` data-format module.
//!
//! Files:
//!
//! - [`enums_endpoint`] — `Endpoint` / `EndpointMeta` enum bodies, included
//!   from `super::enums`.

// The file is reached via `include!("generated/<name>.rs")`
// from the hand-written module above, so no `mod` declarations are
// emitted here. This mod.rs is the re-export hub the file paths point to.
