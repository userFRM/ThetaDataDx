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
//! use thetadatadx::streaming::{StreamEvent, StreamData};
//! use thetadatadx::streaming::Contract;
//!
//! # async fn doc() -> Result<(), thetadatadx::Error> {
//! let creds = Credentials::from_file("creds.txt")?;
//! let client = Client::connect(&creds, DirectConfig::production()).await?;
//!
//! // Market-data — every query endpoint on the `market-data` surface
//! let ticks = client.market_data().stock_history_eod("AAPL", "20240101", "20240301").await?;
//!
//! // Real-time streaming — on the `stream` surface
//! client.stream().start_streaming(|event: &StreamEvent| {
//!     if let StreamEvent::Data(StreamData::Trade { contract, price, size, .. }) = event {
//!         println!("Trade: {} @ {price} x {size}", contract.symbol);
//!     }
//! })?;
//! client.stream().subscribe(Contract::stock("AAPL").quote())?;
//!
//! // Bulk flat files — on the `flat_files` surface, decoded in memory
//! let rows = client.flat_files().option_trade_quote("20240115").await?;
//! # let _ = rows;
//! # Ok(()) }
//! ```
//!
//! For streaming-only workloads, build an [`streaming::StreamingClient`] directly
//! and iterate events on the caller's thread:
//!
//! ```rust,no_run
//! use thetadatadx::streaming::{StreamingClient, StreamEvent};
//! use thetadatadx::{Credentials, DirectConfig};
//! use thetadatadx::streaming::Contract;
//!
//! # fn doc() -> Result<(), thetadatadx::streaming::StreamError> {
//! let creds = Credentials::new("user@example.com", "pw");
//! let config = DirectConfig::production();
//! let hosts = config.streaming_hosts();
//!
//! let client = StreamingClient::builder(&creds, hosts)
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
//! For market-data-only workloads, build a [`market_data::MarketDataClient`]
//! directly and query endpoints on it:
//!
//! ```rust,no_run
//! use thetadatadx::market_data::MarketDataClient;
//! use thetadatadx::{Credentials, DirectConfig};
//!
//! # async fn doc() -> Result<(), thetadatadx::Error> {
//! let creds = Credentials::from_file("creds.txt")?;
//! let client = MarketDataClient::connect(&creds, DirectConfig::production()).await?;
//!
//! let eod = client.stock_history_eod("AAPL", "20240101", "20240301").await?;
//! println!("{} EOD ticks", eod.len());
//! # Ok(()) }
//! ```
//!
//! ## Data delivery
//!
//! Historical data arrives over ThetaData's MDDS service; real-time
//! ticks arrive over ThetaData's FPSS service. Both are decoded
//! inside the crate — consumers see typed tick rows on the market-data side
//! and a typed [`streaming::StreamEvent`] stream on the streaming side.

// `wire_semantics.rs` is `#[path]`-shared between this library and the
// `generate_sdk_surfaces` binary's code-generation tree. The binary sees
// the library as the external `thetadatadx` crate, so the shared file
// names the option-right parser through `thetadatadx::right` rather than
// `crate::`. This self-alias lets the same `thetadatadx::` path resolve
// inside the library build too.
extern crate self as thetadatadx;

// ─── Internal module tree ────────────────────────────────────────────────────

pub mod auth;
pub mod backoff;
pub(crate) mod client;
pub(crate) mod client_builder;
pub mod columns;
pub mod config;
pub mod error;
pub mod flatfiles;
// The streaming implementation lives here, but `thetadatadx::streaming` is the
// canonical public path for the streaming surface. `fpss` stays `pub` so the
// `streaming` re-exports resolve through it, and is `#[doc(hidden)]` so the
// vendor protocol name no longer fronts the rendered API. The set of items it
// re-exports tracks the public surface and is not itself a stability contract.
#[doc(hidden)]
pub mod fpss;
#[cfg(any(feature = "polars", feature = "arrow"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "polars", feature = "arrow"))))]
#[doc(hidden)]
pub mod frames;
pub(crate) mod lifecycle;

// The `grpc` module hosts the transport infrastructure (Channel, ChannelPool,
// Status, ServerStreaming). The user-facing path is
// `MarketDataClient::for_each_chunk(ServerStreaming<..>)`; the remainder is
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
// FIT codec, Black-Scholes Greeks, and the condition / exchange /
// sequence lookups. The crate root re-exports its public surface under
// stable `thetadatadx::*` paths (see the re-export blocks below); the
// `tdbe` name itself stays internal so the SDK ships as one crate with
// one published package. Never widen to `pub` — that would resurface
// the `tdbe` path consumers must not depend on.
//
// The layer carries the complete data-format API: some entry points
// (the per-Greek Black-Scholes primitives, the FIT codec, the
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
// feature re-opens the module to workspace tools (`tools/server`,
// `tools/mcp`) and bindings (`ffi`, `thetadatadx-py`, `thetadatadx-ts`) that
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
    // The gRPC client stubs are pre-generated and committed at
    // `proto/beta_endpoints.snapshot.rs`, so a normal build needs no
    // `protoc`. The snapshot is regenerated (and drift-checked against
    // `proto/mdds.proto`) only under the `grpc-codegen` feature; see
    // `build_support/grpc/` and `proto/MAINTENANCE.md`.
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/proto/beta_endpoints.snapshot.rs"
    ));
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
// ─── Curated public client surface ───────────────────────────────────────────

pub use auth::Credentials;
pub use backoff::JitterMode;
pub use client::{Client, ConnectionStatus, FlatFiles, StreamSurface, SubscriptionInfo};
pub use client_builder::ClientBuilder;
pub use config::{
    BulkFetchPolicy, DirectConfig, FlatFilesConfig, MarketDataEnvironment, ReconnectAttemptClass,
    ReconnectAttemptLimits, ReconnectPolicy, RetryPolicy, RuntimeConfig, StreamingEnvironment,
    WaitMode,
};
pub use error::{
    AuthErrorKind, ConfigErrorKind, DecodeErrorKind, DecompressErrorKind, Error, GrpcStatusKind,
    StreamErrorKind, TransportErrorKind,
};

// ─── Real-time streaming ─────────────────────────────────────────────────────
// The canonical streaming surface lives in the [`streaming`] module: build a
// client with [`streaming::StreamingClient::builder`], subscribe via
// [`streaming::Contract`], then drain with `next_event` / `poll_batch` / the
// `Iterator` impl.

/// Outcome of a single [`streaming::StreamingClient::poll_batch`] call, re-exported
/// at the crate root for callers that drive the batch loop directly.
pub use fpss::PollOutcome;

/// Real-time streaming consumer surface.
///
/// This is the canonical module for the streaming client, its events, and the
/// subscription-building types. Build a client with
/// [`StreamingClient::builder`](crate::streaming::StreamingClient::builder),
/// subscribe via [`Contract`](crate::streaming::Contract), then drain events
/// with `next_event` / `poll_batch` / `for_each` or the `Iterator` impl on
/// `&StreamingClient`.
pub mod streaming {
    pub use crate::fpss::protocol::{
        Contract, FullSubscriptionKind, OptionLeg, SecTypeExt, Subscription, SubscriptionKind,
    };
    pub use crate::fpss::{
        PollOutcome, StreamControl, StreamData, StreamError, StreamEvent, StreamingClient,
        StreamingClientBuilder,
    };

    /// Pull-based columnar delivery — read the live stream as Apache Arrow
    /// `RecordBatch` values instead of per-event callbacks.
    ///
    /// [`RecordBatchStream`] is a sibling to the per-event callback
    /// registered through `client.stream().start_streaming(..)`: the same
    /// subscriptions feed it, but market-data events arrive in columnar
    /// batches under a fixed schema rather than one event at a time. Open it
    /// with `client.stream().batches()`, tune `batch_size` / `linger` /
    /// `backpressure` on the returned [`BatchReaderBuilder`], then pull
    /// batches with the [`futures_core::Stream`] impl or `next_blocking`.
    /// See [`crate::fpss::batch_schema::stream_batch_schema`] for the layout.
    #[cfg(feature = "arrow")]
    #[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
    pub use crate::fpss::batch_reader::{
        Backpressure, BatchReaderBuilder, RecordBatchStream, DEFAULT_BATCH_SIZE, DEFAULT_LINGER,
        DEFAULT_QUEUE_DEPTH,
    };

    /// Estimated Arrow IPC stream size, in bytes, for a single batch of
    /// `num_rows` rows under the fixed streaming schema. A buffer-sizing hint
    /// for the binding batch encoders so they seed their output `Vec` from the
    /// USED size (keyed on `num_rows`) rather than the builder's preallocated
    /// column capacity. Hidden: an internal hint shared with the bindings, not
    /// part of the supported surface.
    #[cfg(feature = "arrow")]
    #[doc(hidden)]
    pub use crate::fpss::batch_schema::estimated_ipc_len;

    /// The fixed streaming-batch Arrow schema. Hidden: shared with the binding
    /// layers (and their tests) so they describe a streaming batch without
    /// reconstructing the column list.
    #[cfg(feature = "arrow")]
    #[doc(hidden)]
    pub use crate::fpss::batch_schema::stream_batch_schema;
}

// ─── Market-data queries ──────────────────────────────────────────────────────
// The canonical market-data surface lives in the [`market_data`] module: build a
// standalone [`market_data::MarketDataClient`], or reach the same query surface
// through [`Client::market_data`] on the unified client.

/// Standalone market-data query client.
///
/// `MarketDataClient` and its [`SubscriptionTier`] are also re-exported at the
/// crate root so both `thetadatadx::MarketDataClient` and
/// `thetadatadx::market_data::MarketDataClient` resolve.
pub use mdds::{MarketDataClient, SubscriptionTier};

/// Bulk-fetch shard planning (manual mode): describe a history query as a
/// [`ShardQuery`], obtain the balanced [`ShardPlan`] via
/// [`MarketDataClient::bulk_fetch_plan`], and run the [`ShardBand`]
/// sub-requests under your own concurrency. The automatic path
/// ([`BulkFetchPolicy::Auto`]) uses exactly these plans.
pub use mdds::shard::{ShardBand, ShardPlan, ShardQuery};

/// Date-range split math for history requests beyond the server's 365-day
/// cap: [`split_date_range`] divides an inclusive `(start, end)` span into
/// contiguous server-accepted chunks. Also exposed to Python as
/// `thetadatadx.split_date_range`.
pub use mdds::shard::{split_date_range, ChunkError};

/// Market-data query consumer surface.
///
/// `thetadatadx::market_data` is the canonical path for the standalone
/// market-data query client, the counterpart to [`streaming`].
/// Build a [`MarketDataClient`] directly, or reach the same query surface
/// through [`Client::market_data`](crate::Client::market_data) on the unified
/// client.
pub mod market_data {
    pub use crate::mdds::{MarketDataClient, SubscriptionTier};
}

// ─── Flat-file bulk pulls ─────────────────────────────────────────────────────

/// Bulk flat-file downloads from ThetaData's flat-file distribution.
///
/// For the accessor shape that matches the Python, TypeScript, and C++
/// bindings, use [`Client::flat_files`] to reach the same surface through
/// a [`FlatFiles`] view (`client.flat_files().option_trade_quote(date)`).
/// The free functions below are the lower-level API: use
/// [`flatfile_request`] to write directly to disk, or
/// [`flatfile_request_decoded`] to materialise rows in memory.
pub mod flatfiles_api {
    pub use crate::client::FlatFiles;
    pub use crate::flatfiles::{
        flatfile_request, flatfile_request_decoded, flatfile_request_raw, FlatFileFormat,
        FlatFileRow, FlatFileValue, FlatFilesUnavailableReason, ReqType as FlatFileReqType,
        SecType as FlatFileSecType,
    };
}
pub use flatfiles_api::*;

// ─── Tick types ───────────────────────────────────────────────────────────────

/// Per-response wire column set carried alongside decoded rows so the
/// DataFrame builders project to the terminal's exact columns, the buffered
/// [`Ticks`] return that carries it, and the trait a tick type implements to
/// compute the set from a wire header list.
pub use crate::columns::{ColumnPresence, Ticks, WireColumns};

pub use crate::tdbe::types::tick::{
    CalendarDay, EodTick, GreeksAllTick, GreeksEodTick, GreeksFirstOrderTick,
    GreeksSecondOrderTick, GreeksThirdOrderTick, IndexPriceAtTimeTick, InterestRateTick, IvTick,
    MarketValueTick, OhlcTick, OpenInterestTick, OptionContract, PriceTick, QuoteTick,
    TradeGreeksAllTick, TradeGreeksFirstOrderTick, TradeGreeksImpliedVolatilityTick,
    TradeGreeksSecondOrderTick, TradeGreeksThirdOrderTick, TradeQuoteTick, TradeTick,
};

// ─── Enums ────────────────────────────────────────────────────────────────────

pub use crate::tdbe::types::enums::{
    Interval, RateType, RemoveReason, RequestType, Right, SecType, StreamMsgType,
    StreamResponseType, Venue, Version,
};
/// Variable-precision fixed-point price encoding (`value` / `price_type`
/// mantissa-and-exponent pair) and its supporting types: the validated
/// `PriceType` exponent, the `PriceError` its fallible constructor returns,
/// and the `MAX_PRICE_TYPE` bound that constructor validates against.
///
/// This is a wire-encoding detail: a client receives decoded prices
/// (`f64` dollars on the tick rows) and never sees, sets, or reasons about
/// the raw `(value, price_type)` pair. The encoding therefore stays off the
/// public API. Only available when the `__internal` feature is enabled, for
/// workspace tools, bindings, and the data-format benches. NOT a stable
/// public surface — external crates MUST NOT enable that feature.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use crate::tdbe::types::price::{Price, PriceError, PriceType, MAX_PRICE_TYPE};

// ─── Option-right parsing ────────────────────────────────────────────────────

/// Canonical parser for the option `right` parameter.
///
/// Accepts `call`/`put`/`both`/`C`/`P`/`*` (case-insensitive) at every
/// user-facing input boundary. Use [`parse_right`] where the wildcard is
/// meaningful and [`parse_right_strict`] where a single side is required.
pub mod right {
    pub use crate::tdbe::right::{parse_right, parse_right_strict, ParsedRight, RightError};
}
// Crate-root re-export of the option-right parser.
pub use right::{parse_right, parse_right_strict, ParsedRight, RightError};

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
// (`tools/server`, `tools/mcp`), bindings (`ffi`,
// `thetadatadx-py`, `thetadatadx-ts`), and the bench harnesses reach a few
// more data-layer items: the DST-aware epoch math, the canonical JSON
// finite-or-null sanitiser, the FIT codec, and the calendar-status
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

/// FIT 4-bit nibble codec for FPSS tick compression.
///
/// Only available when the `__internal` feature is enabled. NOT a stable
/// public surface — for workspace tools and bindings only.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub use crate::tdbe::codec;

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
    pub use crate::streaming::{
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
