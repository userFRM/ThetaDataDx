//! # thetadatadx
//!
//! Native Rust SDK for ThetaData market data. Full terminal capability
//! — historical (MDDS gRPC), real-time streaming (FPSS TCP), and
//! flat-file bulk pulls — without the JVM, the subprocess, or the
//! local proxy. One async client object speaks every transport
//! directly.
//!
//! Requires a valid [ThetaData](https://thetadata.us) subscription.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
//! use thetadatadx::fpss::FpssEvent;
//! use thetadatadx::fpss::protocol::Contract;
//!
//! # async fn doc() -> Result<(), thetadatadx::Error> {
//! let creds = Credentials::from_file("creds.txt")?;
//! let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;
//!
//! // Historical
//! let ticks = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
//!
//! // Streaming — register a callback, then subscribe.
//! tdx.start_streaming(|event: &FpssEvent| {
//!     // ...
//! })?;
//! tdx.subscribe(Contract::stock("AAPL").quote())?;
//! # Ok(()) }
//! ```
//!
//! For streaming-only workloads (no MDDS / Nexus session), build an
//! [`fpss::FpssClient`] directly and iterate the ring on the caller's
//! own thread:
//!
//! ```rust,no_run
//! use thetadatadx::fpss::{FpssClient, FpssEvent};
//! use thetadatadx::auth::Credentials;
//! use thetadatadx::fpss::protocol::Contract;
//! # fn doc() -> Result<(), thetadatadx::fpss::FpssError> {
//! let creds = Credentials::new("user@example.com", "pw");
//! let hosts = thetadatadx::config::DirectConfig::production().fpss.hosts;
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
//! shutdown; `try_next_event` is the non-blocking cousin;
//! `poll_batch(FnMut)` and `for_each(FnMut)` are the closure-driven
//! shapes.

pub use thetadatadx_engine::*;
