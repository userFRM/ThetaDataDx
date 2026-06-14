//! `ThetaData` binary-encoding layer — the data-format core of the SDK.
//!
//! Internal module. The crate root re-exports its public surface (tick
//! types, enums, [`Price`](types::price::Price), Greeks, and the
//! conditions / exchange / sequences lookups) under stable
//! `thetadatadx::*` paths; consumers never name this module directly.
//!
//! Contains:
//! - **Tick types** -- [`EodTick`], [`TradeTick`], [`QuoteTick`], [`OhlcTick`], etc.
//! - **Price** -- fixed-point price encoding used by `ThetaData`
//! - **Enums** -- [`SecType`], [`DataType`], [`StreamMsgType`](types::enums::StreamMsgType), etc.
//! - **FIT/FIE codecs** -- 4-bit nibble encoding for FPSS tick compression
//! - **Greeks** -- Black-Scholes option pricing, Greek surface, and IV solver
//! - **Error** -- encoding-layer error types
//! - **Flags** -- bit flags and condition codes for market data records
//!
//! Zero networking dependencies: this module is pure CPU-bound data math.

pub mod codec;
pub mod conditions;
pub mod error;
pub mod exchange;
pub mod flags;
pub mod greeks;
pub mod json_canon;
pub mod latency;
pub mod right;
pub mod sequences;
pub mod time;
pub mod types;

// Module-root facade. The data-format layer keeps a complete, flat
// re-export surface so internal callers reach `crate::tdbe::Price`,
// `crate::tdbe::Error`, `crate::tdbe::CalendarStatus`, and the tick types
// without threading the full submodule path, and so the crate root can
// re-export from one coherent place. The crate's curated public surface
// (see `lib.rs`) reaches several of these through the longer submodule
// path, so `unused_imports` is allowed on the facade rather than trimming
// it to whichever items today's callers happen to reach the short way.
#[allow(unused_imports)]
pub use error::Error;
#[allow(unused_imports)]
pub use types::enums::{
    CalendarStatus, DataType, Interval, RateType, RequestType, Right, SecType, Venue, Version,
};
#[allow(unused_imports)]
pub use types::price::Price;
#[allow(unused_imports)]
pub use types::tick::*;
