//! In-house gRPC client (Phase 1 — foundation).
//!
//! Tonic-free gRPC stack built directly on the [`h2`] crate, gated behind
//! the `inhouse-grpc` Cargo feature. Phase 1 lands the framing codec, the
//! trailers-parsed [`Status`], the [`Channel`] / [`ServerStreaming`]
//! transport, plus a single proof-of-life endpoint. Tonic remains the
//! production code path until later phases finish migration.
//!
//! The full wire-shape rustdoc and the multi-phase plan land in the docs
//! commit that closes this phase.

pub mod codec;
pub mod status;

pub use codec::{Codec, CodecError};
pub use status::{Status, StatusParseError};
