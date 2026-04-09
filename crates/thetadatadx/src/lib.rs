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
//!     // Historical (MDDS gRPC) -- all 61 methods via Deref
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
//! For historical-only usage, just skip `start_streaming()` -- all 61 historical
//! methods are available directly on `ThetaDataDx` via `Deref<Target = DirectClient>`:
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
//! - **Proto definitions**: `crates/thetadatadx/proto/external.proto` — single
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
pub mod config;
pub mod decode;
pub mod direct;
pub mod endpoint;
pub mod error;
pub mod fpss;
pub mod registry;
pub mod unified;

/// Generated protobuf types from `external.proto`.
///
/// Contains all wire types in a single package `BetaEndpoints`:
/// - Shared types: `AuthToken`, `ContractSpec`, `Price`, `DataValue`, `DataValueList`,
///   `DataTable`, `ResponseData`, `CompressionAlgo`, `CompressionDescription`,
///   `TimeZone`, `ZonedDateTime`, `QueryInfo`
/// - 60 request/response types (`StockHistoryEodRequest`, `OptionSnapshotQuoteRequest`, ...)
/// - gRPC client stub (`beta_theta_terminal_client::BetaThetaTerminalClient`)
// Generated code -- not under our control.
#[allow(clippy::pedantic)]
pub mod proto {
    tonic::include_proto!("beta_endpoints");
}

pub use auth::Credentials;
pub use config::{DirectConfig, FpssFlushMode, ReconnectPolicy};
pub use endpoint::{EndpointArgValue, EndpointArgs, EndpointError, EndpointOutput};
pub use error::{AuthErrorKind, Error, FpssErrorKind};
pub use registry::{EndpointMeta, ParamMeta, ParamType, ReturnType, ENDPOINTS};
pub use unified::{ConnectionStatus, SubscriptionInfo, ThetaDataDx};
