//! `ThetaData` binary-encoding layer — the data-format core of the SDK.
//!
//! Internal module. The crate root re-exports its public surface (tick
//! types, enums, [`Price`](types::price::Price), and the
//! conditions / exchange / sequences lookups) under stable
//! `thetadatadx::*` paths; consumers never name this module directly.
//!
//! Contains:
//! - **Tick types** -- [`EodTick`], [`TradeTick`], [`QuoteTick`], [`OhlcTick`], etc.
//! - **Price** -- fixed-point price encoding used by `ThetaData`
//! - **Enums** -- [`SecType`], [`StreamMsgType`](types::enums::StreamMsgType), etc.
//! - **FIT codec** -- 4-bit nibble encoding for FPSS tick compression
//! - **Flags** -- bit flags and condition codes for market data records
//!
//! Zero networking dependencies: this module is pure CPU-bound data math.

pub mod codec;
pub mod conditions;
pub mod exchange;
pub mod flags;
// Only the workspace tools reach the canonical-JSON helpers, through the
// `__internal` re-export.
#[cfg(feature = "__internal")]
pub mod json_canon;
pub mod right;
pub mod sequences;
pub mod time;
pub mod types;

// Short paths for the two enums internal callers name as
// `crate::tdbe::{CalendarStatus, Right}`. Everything else, including the
// crate root's public re-exports, uses the full `types::{enums, tick, price}`
// leaf paths.
pub use types::enums::{CalendarStatus, Right};
