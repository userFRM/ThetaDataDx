//! DataFrame ergonomics for Rust consumers — `.to_polars()` /
//! `.to_arrow()` extension traits on slices of tick rows.
//!
//! Python users have chainable `.to_polars()` / `.to_arrow()` /
//! `.to_pandas()` terminals on every `<TickName>List` returned by the
//! historical endpoints. Rust users return `Vec<Tick>` — ergonomic for
//! iteration, awkward for DataFrame workflows. This module closes the
//! gap behind opt-in Cargo features:
//!
//! * `polars` — enable [`TicksPolarsExt::to_polars`].
//! * `arrow` — enable [`TicksArrowExt::to_arrow`].
//! * `frames` — convenience alias for `polars,arrow`.
//!
//! Neither `polars` nor `arrow` is pulled into the default dependency
//! graph. Opt in from Cargo.toml:
//!
//! ```toml
//! [dependencies]
//! thetadatadx = { version = "8", features = ["polars"] }
//! ```
//!
//! Then chain off the decoder-owned `Vec<Tick>`:
//!
//! ```rust,ignore
//! use thetadatadx::frames::TicksPolarsExt;
//!
//! let ticks: Vec<tdbe::types::tick::EodTick> = tdx
//!     .stock_history_eod("AAPL", "20240101", "20240301")
//!     .await?;
//! let df = ticks.as_slice().to_polars()?;
//! ```
//!
//! # SSOT
//!
//! The column-shape decisions (column names, Arrow data types,
//! `OptionContract.right` projection, the contract-id tail
//! `expiration` / `strike` / `right`, the `QuoteTick.midpoint` virtual
//! column) are identical to the Python slice_arrow emitter in
//! `build_support/ticks/python_arrow.rs`. Both generators read from the
//! same `tick_schema.toml` and emit matching schemas, so
//! `tdx.stock_history_eod(...).to_polars()` on the Python side and
//! `ticks.as_slice().to_polars()?` on the Rust side produce the same
//! DataFrame columns in the same order.

/// Convert a slice of tick rows into a [`polars::prelude::DataFrame`].
///
/// Feature-gated on the `polars` Cargo feature. Implemented for every
/// tick type in [`tdbe::types::tick`]; the per-type impls live in the
/// generator-emitted `frames_generated.rs`.
#[cfg(feature = "polars")]
#[cfg_attr(docsrs, doc(cfg(feature = "polars")))]
pub trait TicksPolarsExt {
    /// Materialise `self` as a polars `DataFrame`. Column order and
    /// names match the Arrow schema produced by [`TicksArrowExt::to_arrow`]
    /// and the Python `.to_polars()` terminal on the matching
    /// `<TickName>List` wrapper.
    fn to_polars(&self) -> polars::prelude::PolarsResult<polars::prelude::DataFrame>;
}

/// Convert a slice of tick rows into an [`arrow_array::RecordBatch`].
///
/// Feature-gated on the `arrow` Cargo feature. Implemented for every
/// tick type in [`tdbe::types::tick`]; the per-type impls live in the
/// generator-emitted `frames_generated.rs`.
#[cfg(feature = "arrow")]
#[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
pub trait TicksArrowExt {
    /// Materialise `self` as an Arrow `RecordBatch`. Column order,
    /// names, and dtypes match the schema emitted by the Python
    /// slice_arrow path in `sdks/python/src/tick_arrow.rs`.
    fn to_arrow(
        &self,
    ) -> ::core::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>;
}

// Per-tick-type impls. Both feature gates live inside the generated
// file — each `impl` block is `#[cfg(feature = "...")]` on its own, so
// enabling only one of `polars` / `arrow` compiles the matching impls
// without pulling the other dep.
include!("frames_generated.rs");
