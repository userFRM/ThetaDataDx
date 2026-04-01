//! Bit flags and condition codes for market data records.
//!
//! ThetaData encodes trade conditions and price flags as integer bit fields.
//! This module provides named constants and helper functions for decoding them.

/// Trade condition codes (from `ext_condition1` through `condition` fields).
pub mod trade {
    /// Cancelled trade condition range (40..=44).
    pub const CANCELLED_RANGE: std::ops::RangeInclusive<i32> = 40..=44;

    /// Regular trading hours: 9:30 AM - 4:00 PM ET.
    pub const RTH_START_MS: i32 = 34_200_000;
    pub const RTH_END_MS: i32 = 57_600_000;

    /// Seller-initiated trade (ext_condition1 == 12).
    pub const SELLER_CONDITION: i32 = 12;
}

/// Condition flags (bit fields in `condition_flags`).
pub mod condition_flags {
    /// Bit 0: trade condition "no last" -- this trade should not update the last price.
    pub const NO_LAST: i32 = 1;
}

/// Price flags (bit fields in `price_flags`).
pub mod price_flags {
    /// Bit 0: price condition "set last" -- this trade should set the last price.
    pub const SET_LAST: i32 = 1;
}

/// Volume type discriminants.
pub mod volume {
    /// Incremental volume (each trade adds to daily total).
    pub const INCREMENTAL: i32 = 0;
}
