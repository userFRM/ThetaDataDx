#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
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
//! use thetadatadx::{Client, Credentials, DirectConfig};
//! use thetadatadx::fpss::{StreamEvent, StreamData};
//! use thetadatadx::fpss::protocol::Contract;
//!
//! # async fn doc() -> Result<(), thetadatadx::Error> {
//! let creds = Credentials::from_file("creds.txt")?;
//! let client = Client::connect(&creds, DirectConfig::production()).await?;
//!
//! // Historical — every query endpoint on the `historical` surface
//! let ticks = client.historical().stock_history_eod("AAPL", "20240101", "20240301").await?;
//!
//! // Real-time streaming — on the `stream` surface
//! client.stream().start_streaming(|event: &StreamEvent| {
//!     if let StreamEvent::Data(StreamData::Trade { contract, price, size, .. }) = event {
//!         println!("Trade: {} @ {price} x {size}", contract.symbol);
//!     }
//! })?;
//! client.stream().subscribe(Contract::stock("AAPL").quote())?;
//! # Ok(()) }
//! ```
//!
//! For streaming-only workloads, build an [`fpss::StreamingClient`] directly
//! and iterate events on the caller's thread:
//!
//! ```rust,no_run
//! use thetadatadx::fpss::{StreamingClient, StreamEvent};
//! use thetadatadx::{Credentials, DirectConfig};
//! use thetadatadx::fpss::protocol::Contract;
//!
//! # fn doc() -> Result<(), thetadatadx::fpss::FpssError> {
//! let creds = Credentials::new("user@example.com", "pw");
//! let hosts = DirectConfig::production().streaming.hosts;
//!
//! let client = StreamingClient::builder(&creds, &hosts)
//!     .ring_size(8192)
//!     .build()?;
//!
//! client.subscribe(Contract::stock("AAPL").quote())?;
//!
//! for event in &client {
//!     let _event: StreamEvent = event?;
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
//! and a typed [`fpss::StreamEvent`] stream on the streaming side.

// `wire_semantics.rs` is `#[path]`-shared between this library and the
// `generate_sdk_surfaces` binary's code-generation tree. The binary sees
// the library as the external `thetadatadx` crate, so the shared file
// names the offline right-parser through `thetadatadx::greeks` rather than
// `crate::`. This self-alias lets the same `thetadatadx::` path resolve
// inside the library build too.
extern crate self as thetadatadx;

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
// `HistoricalClient::for_each_chunk(ServerStreaming<..>)`; the remainder is
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

// The binary-encoding data layer — tick types, enums, `Price`, the
// FIT/FIE codecs, Black-Scholes Greeks, and the condition / exchange /
// sequence lookups. The crate root re-exports its public surface under
// stable `thetadatadx::*` paths (see the re-export blocks below); the
// `tdbe` name itself stays internal so the SDK ships as one crate with
// one published package. Never widen to `pub` — that would resurface
// the `tdbe` path consumers must not depend on.
//
// The layer carries the complete data-format API: some entry points
// (the per-Greek Black-Scholes primitives, the FIE encoder, the
// canonical-JSON helpers, a handful of enum/error constructors) have no
// caller inside a default-feature build — they are reached by the
// `__internal` re-exports below (workspace tools and bindings) and by the
// data-format benches. Enabling `__internal` makes those re-exports `pub`,
// so dead-code analysis still covers the whole layer there; the
// allow applies only to the narrower default build where the curated
// public surface does not name them.
#[cfg_attr(not(feature = "__internal"), allow(dead_code))]
pub(crate) mod tdbe;

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
pub use mdds::{HistoricalClient, SubscriptionTier};

// ─── Curated public client surface ───────────────────────────────────────────

pub use auth::Credentials;
pub use backoff::JitterMode;
pub use client::{Client, ConnectionStatus, StreamSurface, SubscriptionInfo};
pub use config::{
    DirectConfig, FlatFilesConfig, HostSelectionPolicy, ReconnectAttemptClass,
    ReconnectAttemptLimits, ReconnectPolicy, RetryPolicy, RuntimeConfig, StreamingFlushMode,
    StreamingWaitStrategy,
};
pub use error::{
    AuthErrorKind, ConfigErrorKind, DecodeErrorKind, DecompressErrorKind, Error, FpssErrorKind,
    GrpcStatusKind, TransportErrorKind,
};

// ─── Real-time streaming (FPSS) ──────────────────────────────────────────────
// The streaming surface lives in the [`fpss`] module: build a client with
// [`fpss::StreamingClient::builder`], subscribe via [`fpss::protocol::Contract`],
// then drain with `next_event` / `poll_batch` / the `Iterator` impl.

/// Outcome of a single [`fpss::StreamingClient::poll_batch`] call, re-exported at
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

pub use crate::tdbe::types::tick::{
    CalendarDay, EodTick, GreeksAllTick, GreeksEodTick, GreeksFirstOrderTick,
    GreeksSecondOrderTick, GreeksThirdOrderTick, IndexPriceAtTimeTick, InterestRateTick, IvTick,
    MarketValueTick, OhlcTick, OpenInterestTick, OptionContract, PriceTick, QuoteTick,
    TradeGreeksAllTick, TradeGreeksFirstOrderTick, TradeGreeksImpliedVolatilityTick,
    TradeGreeksSecondOrderTick, TradeGreeksThirdOrderTick, TradeQuoteTick, TradeTick,
};

// ─── Enums and price wrapper ──────────────────────────────────────────────────

pub use crate::tdbe::types::enums::{
    DataType, Interval, RateType, RemoveReason, RequestType, Right, SecType, StreamMsgType,
    StreamResponseType, Venue, Version,
};
// `Price` plus the `PriceError` its fallible constructor
// (`Price::with_value_and_type`) returns and the `MAX_PRICE_TYPE` bound
// that constructor validates against — the public fixed-point price
// surface.
pub use crate::tdbe::types::price::{Price, PriceError, MAX_PRICE_TYPE};

// ─── Offline Black-Scholes (Greeks + implied volatility) ─────────────────────

/// Offline Black-Scholes Greeks and implied-volatility solver.
///
/// All calculations follow the standard Black-Scholes-Merton model.
/// Use [`all_greeks`] to compute the full Greek surface from a quoted option
/// price, or [`implied_volatility`] for the Newton-Raphson IV solve alone.
pub mod greeks {
    /// Error returned by the offline analytics surface ([`all_greeks`],
    /// [`implied_volatility`], [`parse_right`], [`parse_right_strict`]) for
    /// an unrecognised `right` or an out-of-domain input. Distinct from the
    /// networking [`crate::Error`]; it converts into it via `?`.
    pub use crate::tdbe::error::Error;
    pub use crate::tdbe::greeks::{all_greeks, implied_volatility, GreeksResult};
    pub use crate::tdbe::right::{parse_right, parse_right_strict, ParsedRight};
}
// Crate-root re-export of the offline-analytics surface. `greeks::Error`
// is deliberately NOT glob-promoted here — the crate root already binds
// the networking [`Error`], and the analytics error stays addressable as
// `greeks::Error`.
pub use greeks::{
    all_greeks, implied_volatility, parse_right, parse_right_strict, GreeksResult, ParsedRight,
};

// ─── Utility modules ─────────────────────────────────────────────────────────

/// Auxiliary lookup tables.
///
/// - [`utils::conditions`] — condition-code descriptions
/// - [`utils::exchange`] — exchange-code to name mapping
/// - [`utils::sequences`] — sequence-number utilities
pub mod utils {
    pub use crate::tdbe::{conditions, exchange, sequences};
}

// ─── Doc-hidden data-layer internals reachable by tools/bindings/benches ──────
//
// The shipped public surface above is curated. Workspace tools
// (`tools/cli`, `tools/server`, `tools/mcp`), bindings (`ffi`,
// `sdks/python`, `sdks/typescript`), and the bench harnesses reach a few
// more data-layer items: the DST-aware epoch math, the canonical JSON
// finite-or-null sanitiser, the FIT/FIE codecs, and the calendar-status
// enum the generated tick constructors validate against. These are gated
// on `__internal` so they stay out of the SemVer commitment and rendered
// rustdoc; external crates MUST NOT enable that feature. The `tdbe` name
// never appears in the path — these resolve as `thetadatadx::time`,
// `thetadatadx::json_canon`, `thetadatadx::codec`, and
// `thetadatadx::CalendarStatus`.

/// DST-aware epoch / civil-date math (`date_ms_to_epoch_ms`,
/// `is_valid_yyyymmdd`, `timestamp_to_date`, ...).
///
/// Only available when the `__internal` feature is enabled. NOT a stable
/// public surface — for workspace tools and bindings only.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use crate::tdbe::time;

/// Canonical JSON helpers (`finite_or_null`, `canonicalize`,
/// `canonicalize_and_serialize`) for the CLI / server / MCP renderers.
///
/// Only available when the `__internal` feature is enabled. NOT a stable
/// public surface — for workspace tools and bindings only.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use crate::tdbe::json_canon;

/// FIT/FIE 4-bit nibble codecs for FPSS tick compression.
///
/// Only available when the `__internal` feature is enabled. NOT a stable
/// public surface — for workspace tools and bindings only.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use crate::tdbe::codec;

/// Full Black-Scholes primitive surface (`value`, `delta`, `gamma`, the
/// higher-order Greeks, and the IV solver). The curated [`greeks`] module
/// re-exports only the three stable entry points; this doc-hidden alias
/// gives the offline-pricing bench the per-Greek functions it measures.
///
/// Only available when the `__internal` feature is enabled. NOT a stable
/// public surface — for workspace tools and bindings only.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use crate::tdbe::greeks as black_scholes;

/// Calendar-day market status enum (`Open`, `EarlyClose`, `FullClose`,
/// `Weekend`). The generated tick constructors validate the wire string
/// against it; the FFI calendar-status helper resolves codes through it.
///
/// Only available when the `__internal` feature is enabled. NOT a stable
/// public surface — for workspace tools and bindings only.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use crate::tdbe::types::enums::CalendarStatus;

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
/// let client = Client::connect(&creds, DirectConfig::production()).await?;
/// let stock  = Contract::stock("AAPL");
/// let option = Contract::option("SPX", OptionLeg { expiration: "20260620", strike: "5400", right: "C" })?;
/// client.stream().subscribe(stock.quote())?;
/// client.stream().subscribe(option.trade())?;
/// client.stream().subscribe(SecType::Option.full_trades())?;
/// # Ok(()) }
/// ```
pub mod prelude {
    pub use crate::auth::Credentials;
    pub use crate::client::{Client, ConnectionStatus};
    pub use crate::config::DirectConfig;
    pub use crate::error::Error;
    pub use crate::fpss::protocol::{
        Contract, FullSubscriptionKind, OptionLeg, SecTypeExt, Subscription, SubscriptionKind,
    };
    pub use crate::tdbe::types::enums::SecType;
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
