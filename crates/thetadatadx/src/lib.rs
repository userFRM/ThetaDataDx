#![cfg_attr(docsrs, feature(doc_cfg))]
//! # thetadatadx
//!
//! Native Rust SDK for [ThetaData](https://thetadata.us) market data.
//! Historical data via ThetaData's MDDS service, real-time streaming via
//! ThetaData's FPSS service, and bulk flat-file pulls — all through a single
//! authenticated client, without a JVM, subprocess, or local proxy.
//!
//! Requires a valid ThetaData subscription.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
//! use thetadatadx::fpss::{FpssEvent, FpssData};
//! use thetadatadx::fpss::protocol::Contract;
//!
//! # async fn doc() -> Result<(), thetadatadx::Error> {
//! let creds = Credentials::from_file("creds.txt")?;
//! let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;
//!
//! // Historical — every historical endpoint available via Deref
//! let ticks = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
//!
//! // Real-time streaming
//! tdx.start_streaming(|event: &FpssEvent| {
//!     if let FpssEvent::Data(FpssData::Trade { contract, price, size, .. }) = event {
//!         println!("Trade: {} @ {price} x {size}", contract.symbol);
//!     }
//! })?;
//! tdx.subscribe(Contract::stock("AAPL").quote())?;
//! # Ok(()) }
//! ```
//!
//! For streaming-only workloads, build an [`fpss::FpssClient`] directly
//! and iterate events on the caller's thread:
//!
//! ```rust,no_run
//! use thetadatadx::fpss::{FpssClient, FpssEvent};
//! use thetadatadx::{Credentials, DirectConfig};
//! use thetadatadx::fpss::protocol::Contract;
//!
//! # fn doc() -> Result<(), thetadatadx::fpss::FpssError> {
//! let creds = Credentials::new("user@example.com", "pw");
//! let hosts = DirectConfig::production().fpss.hosts;
//!
//! let client = FpssClient::builder(&creds, &hosts)
//!     .ring_size(8192)
//!     .build()?;
//!
//! client.subscribe(Contract::stock("AAPL").quote())?;
//!
//! for event in &client {
//!     let _event: FpssEvent = event?;
//! }
//! # Ok(()) }
//! ```
//!
//! `client.next_event()` blocks until the next event or terminal
//! shutdown; `try_next_event` is the non-blocking variant;
//! `poll_batch(FnMut)` and `for_each(FnMut)` are the closure-driven
//! shapes.
//!
//! ## Data delivery
//!
//! Historical data arrives over ThetaData's MDDS service; real-time
//! ticks arrive over ThetaData's FPSS service. Both are decoded
//! inside the crate — consumers see typed tick rows on the historical side
//! and a typed [`fpss::FpssEvent`] stream on the streaming side.

// ─── Internal module tree ────────────────────────────────────────────────────

pub mod auth;
pub mod backoff;
pub(crate) mod client;
pub mod config;
pub mod error;
pub mod flatfiles;
pub mod fpss;
#[cfg(any(feature = "polars", feature = "arrow"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "polars", feature = "arrow"))))]
#[doc(hidden)]
pub mod frames;
pub(crate) mod lifecycle;

// The `grpc` module hosts the transport infrastructure (Channel, ChannelPool,
// Status, ServerStreaming). The user-facing path is
// `MddsClient::for_each_chunk(ServerStreaming<..>)`; the remainder is
// consumed by the SDK's own integration tests and benches.
//
// In shipped builds (default features) the module is `pub(crate)` so none
// of its types appear in the SemVer commitment or in rendered rustdoc.
// Errors flowing out of the transport layer are converted to the public
// [`crate::Error`] type at the crate boundary — consumers pattern-match on
// [`crate::Error`] only.
//
// The `__test-helpers` feature re-opens the module to integration tests and
// bench harnesses that need to drive the raw `Channel` / `ChannelPool`
// surface against synthetic frames. This feature is private and
// unsupported for downstream consumers.
#[cfg(not(feature = "__test-helpers"))]
pub(crate) mod grpc;
#[cfg(feature = "__test-helpers")]
#[doc(hidden)]
pub mod grpc;

pub(crate) mod observability;
pub mod util;

// `mdds/` holds the macros, registry, validate, wire_semantics, and the
// shared endpoint runtime (`endpoint_args`).
//
// In default-feature builds the module is `pub(crate)` — none of its types
// appear in the SemVer commitment or in rendered rustdoc. The `__internal`
// feature re-opens the module to workspace tools (`tools/cli`, `tools/server`,
// `tools/mcp`) and bindings (`ffi`, `sdks/python`, `sdks/typescript`) that
// need direct access to the registry, decode pipeline, and endpoint runtime.
#[cfg(not(feature = "__internal"))]
pub(crate) mod mdds;
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub mod mdds;

/// Shared endpoint runtime (`EndpointArgs`, `EndpointError`, `invoke_endpoint`).
/// Re-exported from [`mdds::endpoint_args`] so existing `thetadatadx::endpoint::*`
/// paths continue to resolve.
///
/// Only available when the `__internal` feature is enabled. NOT a stable public
/// surface — for workspace tools and bindings only.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use mdds::endpoint_args as endpoint;

/// Decode pipeline re-exported from `mdds::decode`.
///
/// `pub(crate)` in default builds — internal modules (`grpc/endpoints.rs`,
/// `mdds/endpoints.rs`, `error.rs`) reference it as `crate::decode`. The
/// `__internal` feature widens it to `pub` so workspace bindings can import
/// `thetadatadx::decode::*` directly.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use mdds::decode;
#[cfg(not(feature = "__internal"))]
pub(crate) use mdds::decode;

/// Generated protobuf types from `mdds.proto` (package `BetaEndpoints`).
///
/// Wire-internal: bindings and decode-fixture consumers reach the payload
/// shapes via [`crate::wire`], which surfaces only the types those callers
/// genuinely need.
#[allow(clippy::pedantic)]
pub(crate) mod proto {
    include!(concat!(env!("OUT_DIR"), "/beta_endpoints.rs"));
}

/// Wire-payload re-exports for offline-decode callers.
///
/// SDK bindings that recover endpoint outputs from recorded byte streams
/// need the protobuf payload types re-exported here. The generated `proto`
/// module that hosts them is otherwise wire-internal — this re-export is
/// the supported surface for that use case.
///
/// Only available when the `__internal` feature is enabled. NOT a stable public
/// surface — for workspace tools and bindings only.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub mod wire {
    pub use super::proto::{
        data_value, CompressionAlgo, CompressionDescription, DataTable, DataValue, DataValueList,
        Price, ResponseData, TimeZone, ZonedDateTime,
    };

    /// Request proto types re-exported behind the `__test-helpers` feature so
    /// integration tests can decode captured outbound wire bytes and assert
    /// field-level content. Symbol stays `pub(crate)` in shipped builds.
    #[cfg(feature = "__test-helpers")]
    #[doc(hidden)]
    pub mod test_requests {
        pub use crate::proto::{
            OptionHistoryGreeksFirstOrderRequest, OptionHistoryGreeksImpliedVolatilityRequest,
        };

        /// Request-side protos for `GetStockHistoryEod`, re-exported so
        /// the transport-comparison bench can issue the identical wire
        /// request through an external client stack and through the
        /// in-house transport.
        pub use crate::proto::{
            AuthToken, QueryInfo, StockHistoryEodRequest, StockHistoryEodRequestQuery,
        };
    }
}

// ─── Doc-hidden internals reachable by tools/bindings ────────────────────────
//
// All symbols below are gated on `__internal`. In default-feature builds the
// `mdds` module is `pub(crate)` so none of these paths are reachable from
// outside the crate. Enabling `__internal` re-opens the module and these
// re-exports so workspace tools and bindings can reference the registry,
// decode pipeline, and endpoint runtime without patching the module tree.

#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use lifecycle::DispatcherSession;
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use mdds::endpoint_args::{EndpointArgValue, EndpointArgs, EndpointError, EndpointOutput};
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use mdds::registry::{
    by_category, find, param_type_to_json_type, EndpointMeta, ParamMeta, ParamType, ReturnType,
    CATEGORIES, ENDPOINTS,
};
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use mdds::{MddsClient, SubscriptionTier};

// ─── Curated public client surface ───────────────────────────────────────────

pub use auth::Credentials;
pub use backoff::JitterMode;
pub use client::{ConnectionStatus, SubscriptionInfo, ThetaDataDxClient};
pub use config::{
    DirectConfig, FlatFilesConfig, FpssFlushMode, HostSelectionPolicy, ReconnectAttemptClass,
    ReconnectAttemptLimits, ReconnectPolicy, RetryPolicy, RuntimeConfig,
};
pub use error::{
    AuthErrorKind, ConfigErrorKind, DecodeErrorKind, DecompressErrorKind, Error, FpssErrorKind,
    GrpcStatusKind, TransportErrorKind,
};

// ─── Real-time streaming (FPSS) ──────────────────────────────────────────────
// The streaming surface lives in the [`fpss`] module: build a client with
// [`fpss::FpssClient::builder`], subscribe via [`fpss::protocol::Contract`],
// then drain with `next_event` / `poll_batch` / the `Iterator` impl.

/// Outcome of a single [`fpss::FpssClient::poll_batch`] call, re-exported at
/// the crate root for callers that drive the batch loop directly.
pub use fpss::PollOutcome;

// ─── Flat-file bulk pulls ─────────────────────────────────────────────────────

/// Bulk flat-file downloads from ThetaData's flat-file distribution.
///
/// Use [`flatfile_request`] to write directly to disk, or
/// [`flatfile_request_decoded`] to materialise rows in memory.
pub mod flatfiles_api {
    pub use crate::flatfiles::{
        default_output_filename as flatfile_default_filename, flatfile_request,
        flatfile_request_decoded, flatfile_request_raw, FlatFileFormat, FlatFileRow, FlatFileValue,
        FlatFilesUnavailableReason, ReqType as FlatFileReqType, SecType as FlatFileSecType,
    };
}
pub use flatfiles_api::*;

// ─── Tick types ───────────────────────────────────────────────────────────────

pub use tdbe::types::tick::{
    CalendarDay, EodTick, GreeksAllTick, GreeksEodTick, GreeksFirstOrderTick,
    GreeksSecondOrderTick, GreeksThirdOrderTick, IndexPriceAtTimeTick, InterestRateTick, IvTick,
    MarketValueTick, OhlcTick, OpenInterestTick, OptionContract, PriceTick, QuoteTick,
    TradeGreeksAllTick, TradeGreeksFirstOrderTick, TradeGreeksImpliedVolatilityTick,
    TradeGreeksSecondOrderTick, TradeGreeksThirdOrderTick, TradeQuoteTick, TradeTick,
};

// ─── Enums and price wrapper ──────────────────────────────────────────────────

pub use tdbe::types::enums::{
    DataType, Interval, RateType, RemoveReason, RequestType, Right, SecType, StreamMsgType,
    StreamResponseType, Venue, Version,
};
pub use tdbe::types::price::Price;

// ─── Offline Black-Scholes (Greeks + implied volatility) ─────────────────────

/// Offline Black-Scholes Greeks and implied-volatility solver.
///
/// All calculations follow the standard Black-Scholes-Merton model.
/// Use [`all_greeks`] to compute the full Greek surface from a quoted option
/// price, or [`implied_volatility`] for the Newton-Raphson IV solve alone.
pub mod greeks {
    pub use tdbe::greeks::{all_greeks, implied_volatility, GreeksResult};
    pub use tdbe::right::{parse_right, parse_right_strict, ParsedRight};
}
pub use greeks::*;

// ─── Utility modules ─────────────────────────────────────────────────────────

/// Auxiliary lookup tables.
///
/// - [`utils::conditions`] — condition-code descriptions
/// - [`utils::exchange`] — exchange-code to name mapping
/// - [`utils::sequences`] — sequence-number utilities
pub mod utils {
    pub use tdbe::{conditions, exchange, sequences};
}

// ─── DataFrame extension traits (feature-gated) ──────────────────────────────

#[cfg(any(feature = "polars", feature = "arrow"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "polars", feature = "arrow"))))]
/// DataFrame conversion for tick slices.
///
/// Feature-gated on `polars` and/or `arrow`. Each tick type implements the
/// relevant trait so you can call `.to_polars()` or `.to_arrow()` on any
/// `&[TickType]`.
pub mod frames_api {
    #[cfg(feature = "arrow")]
    #[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
    pub use crate::frames::TicksArrowExt;

    #[cfg(feature = "polars")]
    #[cfg_attr(docsrs, doc(cfg(feature = "polars")))]
    pub use crate::frames::TicksPolarsExt;
}

// ─── Optional allocator ───────────────────────────────────────────────────────

#[cfg(feature = "mimalloc-allocator")]
#[cfg_attr(docsrs, doc(cfg(feature = "mimalloc-allocator")))]
/// Re-export of `MiMalloc` from the [mimalloc](https://crates.io/crates/mimalloc) crate for use as `#[global_allocator]`.
///
/// Library crates cannot set a global allocator — that must live in
/// the consuming binary. Enable the `mimalloc-allocator` feature and
/// attach the handle in your binary's `main.rs`:
///
/// ```rust,ignore
/// #[global_allocator]
/// static GLOBAL: thetadatadx::mimalloc::MiMalloc = thetadatadx::mimalloc::MiMalloc;
/// ```
pub mod mimalloc {
    pub use ::mimalloc::MiMalloc;
}

// ─── Prelude ──────────────────────────────────────────────────────────────────

/// Convenience re-exports for the contract-first streaming API.
///
/// ```rust,no_run
/// use thetadatadx::prelude::*;
/// # async fn doc() -> Result<(), thetadatadx::Error> {
/// let creds  = Credentials::from_file("creds.txt")?;
/// let client = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;
/// let stock  = Contract::stock("AAPL");
/// let option = Contract::option("SPX", OptionLeg { expiration: "20260620", strike: "5400", right: "C" })?;
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
        Contract, FullSubscriptionKind, OptionLeg, SecTypeExt, Subscription, SubscriptionKind,
    };
    pub use tdbe::types::enums::SecType;
}

/// Install the ring `CryptoProvider` as the process-wide rustls default.
///
/// `rustls`, `tokio-rustls`, `hyper-rustls`, and `rustls-platform-verifier`
/// are pinned workspace-wide to `default-features = false, features =
/// ["ring", ...]`, so ring is the sole `CryptoProvider` in the dep graph.
/// `rustls::crypto::CryptoProvider::install_default` still has to fire
/// before any TLS handshake; this helper is the binding-side hook the
/// language bindings call from their module-init paths. Idempotent —
/// second-and-later calls return `false` and leave the prior provider
/// intact. Returns `true` on the install pass that won the race.
#[doc(hidden)]
pub fn __internal_install_ring_crypto_provider() -> bool {
    rustls::crypto::ring::default_provider()
        .install_default()
        .is_ok()
}
