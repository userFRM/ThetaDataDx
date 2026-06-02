//! Cross-cutting utilities shared by multiple subsystems.
//!
//! Modules here have no domain coupling — they belong to none of
//! `fpss`, `grpc`, `mdds`, or `flatfiles` in particular and are
//! reused across them.

pub mod ring;
