//! The SDK surface, written once as `macro_rules!` tables, and the coded
//! fields read from them and from the core types.

mod codes;
mod config;
mod endpoints;
mod enums;
mod events;
mod ticks;

pub use codes::{CodedField, coded_fields};
