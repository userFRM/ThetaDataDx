#![cfg_attr(docsrs, feature(doc_cfg))]

//! # thetadatadx — No-JVM `ThetaData` Terminal
//!
//! Native Rust SDK that connects directly to `ThetaData`'s upstream servers,
//! eliminating the Java terminal entirely. No JVM, no subprocess, no local proxy —
//! just your application speaking the same wire protocol the terminal uses.
//!
//! ## Data types live in `tdbe`
//!
//! Tick types (`TradeTick`, `EodTick`, ...), `Price`, enums (`SecType`, `DataType`),
//! the FIT/FIE codecs, and the Greeks calculator have been extracted into the
//! [`tdbe`](https://crates.io/crates/tdbe) crate. This crate re-exports what it
//! needs, but if you only want types and offline Greeks, depend on `tdbe` directly.
//!
//! ## Architecture
//!
//! `ThetaData` exposes two upstream services:
//!
//! - **MDDS** (Market Data Distribution Server) — historical data via gRPC at `mdds-01.thetadata.us:443`
//! - **FPSS** (Feed Processing Streaming Server) — real-time streaming via custom TCP at `nj-a.thetadata.us:20000`
//!
//! This crate speaks both protocols natively, handling authentication, request building,
//! response decompression, and tick parsing entirely in Rust.
//!
//! ## Quick Start
//!
//! The recommended entry point is [`ThetaDataDx`], which authenticates once and
//! provides both historical and streaming through a single object:
//!
//! ```rust,ignore
//! use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
//! use thetadatadx::fpss::{FpssData, FpssControl, FpssEvent};
//! use thetadatadx::fpss::protocol::Contract;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), thetadatadx::Error> {
//!     let creds = Credentials::from_file("creds.txt")?;
//!     // Or inline: let creds = Credentials::new("user@example.com", "your-password");
//!
//!     // Connect -- authenticates once, historical ready immediately
//!     let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
//!
//!     // Historical (MDDS gRPC) -- every generated method via Deref
//!     let ticks = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
//!
//!     // Streaming (FPSS TCP) -- connects lazily on first call
//!     tdx.start_streaming(|event: &FpssEvent| {
//!         match event {
//!             FpssEvent::Data(FpssData::Trade { contract_id, price, size, .. }) => {
//!                 println!("Trade: {contract_id} @ {price} x {size}");
//!             }
//!             _ => {}
//!         }
//!     })?;
//!
//!     tdx.subscribe_quotes(&Contract::stock("AAPL"))?;
//!
//!     // ... when done:
//!     tdx.stop_streaming();
//!     Ok(())
//! }
//! ```
//!
//! For historical-only usage, just skip `start_streaming()` -- every historical
//! methods are available directly on `ThetaDataDx` via `Deref<Target = MddsClient>`:
//!
//! ```rust,ignore
//! use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
//!
//! let creds = Credentials::from_file("creds.txt")?;
//! // Or inline: let creds = Credentials::new("user@example.com", "your-password");
//! let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
//! let ticks = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
//! ```
//!
//! ## Wire protocol
//!
//! - **Proto definitions**: `crates/thetadatadx/proto/mdds.proto` — single
//!   `BetaEndpoints` package, 60 RPCs, `BetaThetaTerminal` service.
//!
//! - **Auth flow**: POST to `https://nexus-api.thetadata.us/identity/terminal/auth_user`
//!   with header `TD-TERMINAL-KEY` and JSON `{email, password}` → `SessionInfoV3` with UUID.
//!
//! - **MDDS**: Standard gRPC server-streaming over TLS. Session UUID embedded in
//!   `QueryInfo.auth_token` field of every request (in-band, not metadata).
//!
//! - **FPSS**: Custom TLS-over-TCP protocol. 1-byte length + 1-byte message code + payload.
//!   FIT nibble encoding (4-bit variable-length integers) with delta compression for ticks.
//!
//! See [`proto/MAINTENANCE.md`](../../crates/thetadatadx/proto/MAINTENANCE.md) for how to
//! update the proto file and regenerate stubs when ThetaData ships a new version.

pub mod auth;
pub(crate) mod client;
pub mod config;
pub mod error;
pub mod flatfiles;
pub mod fpss;
#[cfg(any(feature = "polars", feature = "arrow"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "polars", feature = "arrow"))))]
pub mod frames;
pub mod observability;

// Wave 3 layout: macros, registry, validate, wire_semantics, and the
// shared endpoint runtime (`endpoint_args`) all live under `mdds/`.
// The macro_rules in `mdds/macros.rs` are made textually visible to
// the sibling `mdds/endpoints` module via `#[macro_use]` on the
// `macros` declaration inside `mdds/mod.rs`.
pub mod mdds;

/// Shared endpoint runtime (`EndpointArgs`, `EndpointError`,
/// `invoke_endpoint`). Re-exported from [`mdds::endpoint_args`] so
/// existing `thetadatadx::endpoint::*` paths continue to resolve.
pub use mdds::endpoint_args as endpoint;

// `decode` is re-exported from `mdds::decode` to preserve the public surface
// (`thetadatadx::decode::*`). Wave 2 split the original decode.rs god-file
// into `mdds/decode/{error, headers, transport, extract, cell, v3}`; the
// re-export keeps existing consumer paths unchanged.
pub use mdds::decode;

/// Generated protobuf types from `mdds.proto` (package `BetaEndpoints`).
///
/// Wire-internal: bindings and decode-fixture consumers reach the
/// gRPC payload shapes via [`crate::wire`], which surfaces only the
/// types those callers genuinely need. Inside the crate the full
/// generated module is reachable via `crate::proto`.
#[allow(clippy::pedantic)]
pub(crate) mod proto {
    tonic::include_proto!("beta_endpoints");
}

/// gRPC wire-payload re-exports for offline-decode callers.
///
/// The MDDS gRPC server emits `ResponseData` frames; each frame's body
/// is a zstd-compressed `DataTable` of `DataValueList` rows. SDK
/// bindings that recover endpoint outputs from recorded byte streams
/// (the parity-bench harness in particular) need these three types
/// plus the `data_value` oneof. The generated `proto` module that
/// hosts them is otherwise wire-internal — this re-export is the
/// supported surface for that one use case.
pub mod wire {
    pub use super::proto::{
        data_value, CompressionAlgo, CompressionDescription, DataTable, DataValue, DataValueList,
        Price, ResponseData,
    };
}

pub use auth::Credentials;
pub use client::{ConnectionStatus, SubscriptionInfo, ThetaDataDx};
pub use config::{DirectConfig, FpssFlushMode, ReconnectPolicy};
pub use error::{AuthErrorKind, Error, FpssErrorKind};
pub use flatfiles::{
    default_output_filename as flatfile_default_filename, flatfile_request,
    flatfile_request_decoded, flatfile_request_raw, FlatFileFormat, FlatFileRow, FlatFileValue,
    FlatFilesUnavailableReason, ReqType as FlatFileReqType, SecType as FlatFileSecType,
};
pub use mdds::endpoint_args::{EndpointArgValue, EndpointArgs, EndpointError, EndpointOutput};
pub use mdds::registry::{
    by_category, find, param_type_to_json_type, EndpointMeta, ParamMeta, ParamType, ReturnType,
    CATEGORIES, ENDPOINTS,
};
pub use mdds::{MddsClient, SubscriptionTier};
pub use tdbe::right::{parse_right, parse_right_strict, ParsedRight};

// Offline Black-Scholes utilities re-exported from `tdbe`. Prefer these at
// the `thetadatadx` top level so SDK users do not need a separate `tdbe`
// dependency for the common "compute Greeks from a quoted option price"
// path. See [`tdbe::greeks`] for the full surface (per-Greek helpers,
// `GreeksResult` struct, etc.).
pub use tdbe::greeks::{all_greeks, implied_volatility, GreeksResult};

// Re-export every tick / row type returned by the SDK's network methods.
// These all live in `tdbe::types::tick`, but consumers of the high-level
// `ThetaDataDx` / `MddsClient` surface should not need a second crate in
// their `Cargo.toml` just to name a return type. `tdbe` remains a
// standalone crate for offline use cases (Greeks math, format primitives,
// no network); customers consuming the SDK get every type they need
// at the `thetadatadx` crate root, fronting the same structs.
//
// Adding a new tick type? Mirror the addition here so `thetadatadx`
// consumers stay on a single dep. The companion items above
// (`ParsedRight`, `GreeksResult`, …) follow the same policy.
pub use tdbe::types::tick::{
    CalendarDay, EodTick, GreeksAllTick, GreeksFirstOrderTick, GreeksSecondOrderTick,
    GreeksThirdOrderTick, InterestRateTick, IvTick, MarketValueTick, OhlcTick, OpenInterestTick,
    OptionContract, PriceTick, QuoteTick, TradeQuoteTick, TradeTick,
};

// Enums + the `Price` wrapper appear on SDK method signatures and inside
// every tick struct, so consumers naming method parameters or unpacking
// tick fields need them in scope. Re-exported here for the same single-
// dep reason as the tick types above. `tdbe::Error` is intentionally
// NOT re-exported to avoid colliding with [`crate::Error`]; the SDK's
// own `Error` transparently wraps codec failures from `tdbe`.
pub use tdbe::types::enums::{
    DataType, Interval, RateType, RemoveReason, RequestType, Right, SecType, StreamMsgType,
    StreamResponseType, Venue, Version,
};
pub use tdbe::types::price::Price;

pub mod utils {
    pub use tdbe::{conditions, exchange, sequences};
}
