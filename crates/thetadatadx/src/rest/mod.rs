//! REST transport for the local ThetaTerminal HTTP API.
//!
//! [`RestClient`] mirrors the gRPC builder shape on top of the
//! Terminal's `/v3/...` CSV endpoints, wired in as the
//! [`crate::config::FallbackPolicy::RestAlways`] transport. Auth
//! delegates to the running Terminal; this module does not call the
//! Nexus API directly.

pub mod client;
pub mod csv;
pub mod error;

// Generated builder structs + their `RestClient` constructor methods.
// Emitted by `build_support_bin/endpoints/sdk_render/rest_builder.rs`
// from `endpoint_surface.toml`; see issue #580. The module is wired in
// here (not at file scope from `client.rs`) so the `impl RestClient`
// block inside it sits in the same crate-public namespace as the
// hand-written `impl RestClient` in `client.rs`.
mod _generated {
    pub mod rest_endpoints;
}

pub use _generated::rest_endpoints::{
    OptionHistoryGreeksFirstOrderRestBuilder, OptionHistoryGreeksIvRestBuilder,
    OptionHistoryQuoteRestBuilder, OptionHistoryTradeQuoteRestBuilder,
};
pub use client::RestClient;
pub use error::RestError;

#[cfg(test)]
mod tests;
