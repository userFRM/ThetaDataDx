//! REST transport for the local ThetaTerminal HTTP API.
//!
//! Mirrors a strict subset of the MDDS gRPC endpoint surface against
//! the local Terminal's HTTP `/v3/...` paths. Wired in as the
//! alternative transport reached via [`crate::config::FallbackPolicy::RestAlways`]
//! when an operator wants every historical-quote call routed over a
//! locally-running Terminal — useful when network policy disallows
//! direct MDDS access or when the local Terminal exposes column
//! extensions the upstream gRPC service does not yet expose.
//!
//! # Surface
//!
//! The REST module exposes [`RestClient`] with one builder method per
//! supported endpoint. Builders mirror the gRPC builders' API
//! (`strike`, `right`, `interval`, etc.) and return the same tick
//! structs (`QuoteTick`, `TradeQuoteTick`, `GreeksAllTick`,
//! `GreeksFirstOrderTick`), so a call site can switch transports by
//! changing the receiver without touching the rest of the pipeline.
//!
//! # Scope
//!
//! The four historical-quote endpoints are wired in this revision:
//!
//! | Endpoint                                  | Tick type                |
//! |-------------------------------------------|--------------------------|
//! | `option_history_quote`                    | [`tdbe::types::tick::QuoteTick`] |
//! | `option_history_trade_quote`              | [`tdbe::types::tick::TradeQuoteTick`] |
//! | `option_history_greeks_implied_volatility`| [`tdbe::types::tick::IvTick`] |
//! | `option_history_greeks_first_order`       | [`tdbe::types::tick::GreeksFirstOrderTick`] |
//!
//! Additional endpoints can be added with the same shape; the existing
//! gRPC `parsed_endpoint!` macro is not reused here because the wire
//! payload is CSV, not protobuf `DataTable`. The CSV decoder is
//! deliberately small (~150 lines including lenient-column handling)
//! and lives at [`mod@csv`].
//!
//! # Authentication
//!
//! The local Terminal binds its session to whichever client started it
//! (the message `Invalid session ID. ... more than one terminal is
//! running` surfaces when a different client tries to talk to it).
//! `RestClient::new` does NOT authenticate against the Nexus API
//! itself; instead it expects the user's running Terminal to already
//! be authenticated, and forwards the SDK's own credentials only on
//! request paths the Terminal proxies through. The
//! [`FallbackPolicy`](crate::config::FallbackPolicy) wiring assumes the
//! caller has a locally-running, authenticated Terminal.

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
