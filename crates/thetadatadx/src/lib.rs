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
//! The recommended entry point is [`ThetaDataDxClient`], which authenticates once and
//! provides both historical and streaming through a single object:
//!
//! ```rust,no_run
//! use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
//! use thetadatadx::fpss::{FpssData, FpssEvent};
//! use thetadatadx::fpss::protocol::Contract;
//!
//! # async fn doc() -> Result<(), thetadatadx::Error> {
//! let creds = Credentials::from_file("creds.txt")?;
//! // Or inline: let creds = Credentials::new("user@example.com", "your-password");
//!
//! // Connect -- authenticates once, historical ready immediately
//! let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;
//!
//! // Historical (MDDS gRPC) -- every generated method via Deref
//! let ticks = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
//!
//! // Streaming (FPSS TCP) -- connects lazily on first call
//! tdx.start_streaming(|event: &FpssEvent| {
//!     if let FpssEvent::Data(FpssData::Trade { contract, price, size, .. }) = event {
//!         println!("Trade: {} @ {price} x {size}", contract.symbol);
//!     }
//! })?;
//!
//! tdx.subscribe(Contract::stock("AAPL").quote())?;
//!
//! // ... when done:
//! tdx.stop_streaming();
//! # Ok(()) }
//! ```
//!
//! For historical-only usage, just skip `start_streaming()` -- every historical
//! methods are available directly on `ThetaDataDxClient` via `Deref<Target = MddsClient>`:
//!
//! ```rust,no_run
//! use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
//!
//! # async fn doc() -> Result<(), thetadatadx::Error> {
//! let creds = Credentials::from_file("creds.txt")?;
//! // Or inline: let creds = Credentials::new("user@example.com", "your-password");
//! let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;
//! let ticks = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
//! # Ok(()) }
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
pub mod grpc;
pub mod observability;
pub mod rest;
pub mod util;

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
    include!(concat!(env!("OUT_DIR"), "/beta_endpoints.rs"));
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
pub use client::{ConnectionStatus, SubscriptionInfo, ThetaDataDxClient};
pub use fpss::protocol::{
    Contract, ContractParseError, FullSubscriptionKind, SecTypeExt, Subscription, SubscriptionKind,
};
pub use fpss::{EventIterator, NextEvent};

/// Convenience prelude for the fluent contract-first API.
///
/// ```rust,no_run
/// use thetadatadx::prelude::*;
/// # async fn doc() -> Result<(), thetadatadx::Error> {
/// let creds  = Credentials::from_file("creds.txt")?;
/// let client = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;
/// let stock  = Contract::stock("AAPL");
/// let option = Contract::option("SPY", "20260620", "550", "C")?;
/// client.subscribe(stock.quote())?;
/// client.subscribe(option.trade())?;
/// client.subscribe(SecType::Option.full_trades())?;
/// # Ok(()) }
/// ```
pub mod prelude {
    pub use crate::auth::Credentials;
    pub use crate::client::{ConnectionStatus, ThetaDataDxClient};
    pub use crate::config::DirectConfig;
    pub use crate::error::Error;
    pub use crate::fpss::protocol::{
        Contract, FullSubscriptionKind, SecTypeExt, Subscription, SubscriptionKind,
    };
    pub use tdbe::types::enums::SecType;
}
pub use config::{
    DirectConfig, FallbackPolicy, FlatFilesConfig, FpssFlushMode, ReconnectAttemptClass,
    ReconnectAttemptLimits, ReconnectPolicy, DEFAULT_REST_BASE_URL,
};
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
// `ThetaDataDxClient` / `MddsClient` surface should not need a second crate in
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

/// Optional [`mimalloc`](https://crates.io/crates/mimalloc) re-export
/// for consumers that prefer mimalloc over the system allocator.
///
/// Library crates cannot install a `#[global_allocator]` — that lives
/// in the binary. The `mimalloc-allocator` feature pulls the crate
/// into the dependency graph and re-exports the allocator handle here
/// so the consuming binary can attach it with one line:
///
/// ```rust,ignore
/// // `ignore` here because `#[global_allocator]` may only appear in
/// // the consuming binary's compile unit, not in a library doc-test.
/// // In your binary's `main.rs` (NOT in a library):
/// #[global_allocator]
/// static GLOBAL: thetadatadx::mimalloc::MiMalloc = thetadatadx::mimalloc::MiMalloc;
/// ```
///
/// And in the binary's `Cargo.toml`:
///
/// ```toml
/// [dependencies]
/// thetadatadx = { version = "10", features = ["mimalloc-allocator"] }
/// ```
///
/// Mimalloc trades a small fixed overhead per process (~64 KB of
/// shared bookkeeping) for materially fewer page faults and lower
/// fragmentation on the allocation-heavy gRPC decode path. The
/// per-call savings scale with response size; tabular MDDS responses
/// past 1 KB consistently show shorter p99 tails on workloads that
/// fan calls out across many threads.
///
/// See `docs-site/docs/configuration.md` (Performance tuning) for the
/// full integration walk-through.
#[cfg(feature = "mimalloc-allocator")]
#[cfg_attr(docsrs, doc(cfg(feature = "mimalloc-allocator")))]
pub mod mimalloc {
    pub use ::mimalloc::MiMalloc;
}
