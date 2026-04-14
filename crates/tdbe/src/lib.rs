//! # tdbe -- `ThetaData` Binary Encoding
//!
//! Pure data-format crate for `ThetaData` market data. Zero networking dependencies.
//!
//! Contains:
//! - **Tick types** -- [`EodTick`], [`TradeTick`], [`QuoteTick`], [`OhlcTick`], etc.
//! - **Price** -- fixed-point price encoding used by `ThetaData`
//! - **Enums** -- [`SecType`], [`DataType`], [`StreamMsgType`](types::enums::StreamMsgType), etc.
//! - **FIT/FIE codecs** -- 4-bit nibble encoding for FPSS tick compression
//! - **Greeks** -- Black-Scholes option pricing (22 Greeks + IV solver)
//! - **Error** -- encoding-layer error types
//! - **Flags** -- bit flags and condition codes for market data records
//!
//! For network access, use the `thetadatadx` crate which depends on `tdbe`.

pub mod codec;
pub mod conditions;
pub mod error;
pub mod errors;
pub mod exchange;
pub mod flags;
pub mod greeks;
pub mod latency;
pub mod right;
pub mod sequences;
pub mod types;

// Convenience re-exports at crate root
pub use error::Error;
pub use types::enums::{DataType, SecType};
pub use types::price::Price;
pub use types::tick::*;
