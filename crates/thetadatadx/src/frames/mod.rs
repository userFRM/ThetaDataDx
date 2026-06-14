//! DataFrame extension traits — `.to_polars()` / `.to_arrow()` on
//! `&[Tick]`.
//!
//! Feature-gated: `polars`, `arrow`, or `frames` (both). Schemas are
//! generated from the same `tick_schema.toml` SSOT as the Python
//! slice_arrow emitter (`build_support/ticks/python_arrow.rs`), so
//! Python and Rust sides return columns in the same order.
//!
//! # Examples
//!
//! ```rust,no_run
//! # #[cfg(feature = "polars")]
//! # fn doc() {
//! use thetadatadx::frames::TicksPolarsExt;
//! use thetadatadx::EodTick;
//! let ticks: Vec<EodTick> = Vec::new();
//! let _df = ticks.as_slice().to_polars().expect("empty frame is valid");
//! # }
//! ```

/// Convert a slice of tick rows into a [`polars::prelude::DataFrame`].
///
/// Feature-gated on the `polars` Cargo feature. Implemented for every
/// tick type the crate exports (`thetadatadx::TradeTick`, `thetadatadx::QuoteTick`, ...); the per-type impls live in the
/// generator-emitted `frames/generated.rs`.
#[cfg(feature = "polars")]
#[cfg_attr(docsrs, doc(cfg(feature = "polars")))]
pub trait TicksPolarsExt {
    /// Materialise `self` as a polars `DataFrame`. Column order and
    /// names match the Arrow schema produced by [`TicksArrowExt::to_arrow`]
    /// and the Python `.to_polars()` terminal on the matching
    /// `<TickName>List` wrapper.
    ///
    /// # Errors
    ///
    /// Returns a [`polars::prelude::PolarsError`] when polars rejects the
    /// assembled columns (e.g. a length mismatch while building the frame).
    fn to_polars(&self) -> polars::prelude::PolarsResult<polars::prelude::DataFrame>;
}

/// Convert a slice of tick rows into an [`arrow_array::RecordBatch`].
///
/// Feature-gated on the `arrow` Cargo feature. Implemented for every
/// tick type the crate exports (`thetadatadx::TradeTick`, `thetadatadx::QuoteTick`, ...); the per-type impls live in the
/// generator-emitted `frames/generated.rs`.
#[cfg(feature = "arrow")]
#[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
pub trait TicksArrowExt {
    /// Materialise `self` as an Arrow `RecordBatch`. Column order,
    /// names, and dtypes match the schema emitted by the Python
    /// slice_arrow path in `sdks/python/src/tick_arrow.rs`.
    ///
    /// # Errors
    ///
    /// Returns an [`arrow_schema::ArrowError`] when the column arrays
    /// cannot be assembled into a `RecordBatch` against the schema.
    fn to_arrow(
        &self,
    ) -> ::core::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>;
}

// Per-tick-type impls. Both feature gates live inside the generated
// file — each `impl` block is `#[cfg(feature = "...")]` on its own, so
// enabling only one of `polars` / `arrow` compiles the matching impls
// without pulling the other dep.
include!("generated.rs");
