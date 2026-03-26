#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! # thetadatadx — No-JVM ThetaData Terminal
//!
//! Native Rust SDK that connects directly to ThetaData's upstream servers,
//! eliminating the Java terminal entirely. No JVM, no subprocess, no local proxy —
//! just your application speaking the same wire protocol the terminal uses.
//!
//! ## Architecture
//!
//! ThetaData exposes two upstream services:
//!
//! - **MDDS** (Market Data Distribution Server) — historical data via gRPC at `mdds-01.thetadata.us:443`
//! - **FPSS** (Feed Processing Streaming Server) — real-time streaming via custom TCP at `nj-a.thetadata.us:20000`
//!
//! This crate speaks both protocols natively, handling authentication, request building,
//! response decompression, and tick parsing entirely in Rust.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use thetadatadx::{DirectClient, Credentials, DirectConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), thetadatadx::Error> {
//!     let creds = Credentials::from_file("creds.txt")?;
//!     let client = DirectClient::connect(&creds, DirectConfig::production()).await?;
//!
//!     // Historical data (MDDS/gRPC)
//!     let ticks = client.stock_history_eod("AAPL", "20240101", "20240301").await?;
//!
//!     // Real-time streaming (FPSS/TCP)
//!     // let stream = client.subscribe_quotes("AAPL").await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Reverse-Engineering Notes
//!
//! This crate was built by decompiling ThetaData's Java terminal (v202603181, 58.5MB):
//!
//! - **Proto definitions**: Extracted via protobuf `FileDescriptor` reflection at runtime
//!   - `endpoints.proto` — shared types (ResponseData, DataTable, Price, etc.)
//!   - `v3_endpoints.proto` — v3 service (BetaThetaTerminal, 60 RPCs with QueryInfo wrapper)
//!
//! - **Auth flow**: POST to `https://nexus-api.thetadata.us/identity/terminal/auth_user`
//!   with header `TD-TERMINAL-KEY` and JSON `{email, password}` → `SessionInfoV3` with UUID
//!
//! - **MDDS**: Standard gRPC server-streaming over TLS. Session UUID embedded in
//!   `QueryInfo.auth_token` field of every request (in-band, not metadata).
//!
//! - **FPSS**: Custom TLS-over-TCP protocol. 1-byte length + 1-byte message code + payload.
//!   FIT nibble encoding (4-bit variable-length integers) with delta compression for ticks.
//!
//! To re-extract protos after a terminal update:
//! ```bash
//! # 1. Get latest version
//! curl -s https://nexus-api.thetadata.us/bootstrap/jars | jq '.[-1]'
//! # 2. Download
//! curl -L -o terminal.jar 'https://nexus-api.thetadata.us/bootstrap/jars/<version>'
//! # 3. Decompile
//! java -jar cfr.jar terminal.jar --outputdir decompiled/ --jarfilter "net.thetadata.*"
//! # 4. Extract proto via DumpV3Proto.java (see theta-terminal-re/)
//! ```

pub mod auth;
pub mod codec;
pub mod config;
pub mod decode;
pub mod direct;
pub mod error;
pub mod fpss;
pub mod greeks;
pub mod types;

/// Generated protobuf types from `endpoints.proto` (shared types).
///
/// Also re-exported as `endpoints` at crate root so that the v3 generated code
/// can resolve cross-proto references via `super::endpoints::*`.
pub mod proto {
    tonic::include_proto!("endpoints");
}

/// Alias required by prost codegen: `beta_endpoints.rs` references
/// `super::endpoints::AuthToken` (and other shared types), so the crate root
/// must expose an `endpoints` module that maps to the `endpoints` proto package.
pub use proto as endpoints;

/// Generated protobuf/gRPC types from `v3_endpoints.proto` (upstream MDDS service).
///
/// Contains `BetaThetaTerminalClient` gRPC stub and all v3 request/response types
/// (`QueryInfo`, `StockHistoryEodRequest`, etc.).
pub mod proto_v3 {
    tonic::include_proto!("beta_endpoints");
}

pub use auth::Credentials;
pub use config::DirectConfig;
pub use direct::DirectClient;
pub use error::Error;
