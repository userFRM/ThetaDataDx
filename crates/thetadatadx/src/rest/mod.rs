//! REST transport for the local ThetaTerminal HTTP API (issue #571).
//!
//! Mirrors a strict subset of the gRPC endpoint surface against the local
//! Terminal's HTTP `/v3/...` paths. The motivating use-case is the
//! issue #571 h2-cascade bug: the upstream Terminal's Java `QuoteTick`
//! / `TradeQuoteTick` constructors length-check incoming row arrays
//! against a fixed 11 / 25 element shape, and throw
//! `IllegalArgumentException` when the storage tier surfaces a
//! pre-extension 6-field NBBO row from 2022-era options data. The
//! exception bubbles through the gRPC handler and terminates the
//! HTTP/2 stream with no error frame -- the SDK observes
//! [`crate::error::TransportErrorKind::ConnectionClosed`] mid-response
//! with no recovery path. The patched Terminal at
//! `theta-terminal-re/patches/QuoteTick.java` upcasts 6-field rows to
//! the 11-field shape by zero-filling the absent exchange / condition
//! columns; the REST API path serves the same upcast rows directly,
//! one request per HTTP transaction, so a per-row exception does not
//! cascade across multiple results (HTTP/1.1 per-request isolation
//! versus h2 stream multiplexing).
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
//! Only the four quote-bearing endpoints from issue #571's failure
//! matrix are wired in this revision:
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
//! deliberately small (~150 lines including legacy-column handling)
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
