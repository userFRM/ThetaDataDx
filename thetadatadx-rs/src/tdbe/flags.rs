//! Bit flags and condition codes for market data records.
//!
//! The vendor publishes a table of trade and quote condition codes; these
//! constants carry the ranges the SDK reads from it. The `condition_flags`,
//! `price_flags` and `volume_type` columns are carried on the tick exactly as
//! sent and are not interpreted here: the vendor has stated it does not define
//! them, and its documentation marks them reserved.

/// Trade condition codes (from `ext_condition1` through `condition` fields).
pub mod trade {
    /// Cancelled trade condition range (40..=44).
    pub const CANCELLED_RANGE: std::ops::RangeInclusive<i32> = 40..=44;
}
