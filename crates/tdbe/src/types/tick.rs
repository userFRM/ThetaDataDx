//! Per-tick `#[repr(C, align(N))]` struct definitions plus the items the
//! schema cannot (yet) express:
//!
//! * `impl_contract_id!` macro applications -- the `is_call` / `is_put` /
//!   `has_contract_id` helpers shared by every tick type that injects a
//!   `(expiration, strike, right)` triple from `contract_id = true`.
//! * `impl TradeTick` flag helpers (`is_cancelled`, `regular_trading_hours`,
//!   ...). These read `flags::*` constants and don't fit the schema's
//!   field-only model.
//! * `impl OptionContract` for `is_call` / `is_put` -- a non-`Copy` struct
//!   so the macro doesn't apply.
//!
//! The structs themselves are generated at build-time from
//! `crates/thetadatadx/tick_schema.toml` by
//! `cargo run -p thetadatadx --bin generate_sdk_surfaces`.

include!("tick_generated.rs");

// ─────────────────────────────────────────────────────────────────────────────
//  Contract identification helpers
// ─────────────────────────────────────────────────────────────────────────────

macro_rules! impl_contract_id {
    ($ty:ident) => {
        impl $ty {
            /// `true` when `right` == 'C' (ASCII 67).
            #[inline]
            pub fn is_call(&self) -> bool {
                self.right == 67
            }
            /// `true` when `right` == 'P' (ASCII 80).
            #[inline]
            pub fn is_put(&self) -> bool {
                self.right == 80
            }
            /// `true` when the server populated contract identification fields.
            #[inline]
            pub fn has_contract_id(&self) -> bool {
                self.expiration != 0
            }
        }
    };
}

impl_contract_id!(TradeTick);
impl_contract_id!(QuoteTick);
impl_contract_id!(OhlcTick);
impl_contract_id!(EodTick);
impl_contract_id!(OpenInterestTick);
impl_contract_id!(TradeQuoteTick);
impl_contract_id!(MarketValueTick);
impl_contract_id!(GreeksTick);
impl_contract_id!(IvTick);

// ─────────────────────────────────────────────────────────────────────────────
//  Hand-written impl blocks
// ─────────────────────────────────────────────────────────────────────────────

use crate::flags;

impl TradeTick {
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        flags::trade::CANCELLED_RANGE.contains(&self.condition)
    }

    #[must_use]
    pub fn trade_condition_no_last(&self) -> bool {
        self.condition_flags & flags::condition_flags::NO_LAST == flags::condition_flags::NO_LAST
    }

    #[must_use]
    pub fn price_condition_set_last(&self) -> bool {
        self.price_flags & flags::price_flags::SET_LAST == flags::price_flags::SET_LAST
    }

    #[must_use]
    pub fn is_incremental_volume(&self) -> bool {
        self.volume_type == flags::volume::INCREMENTAL
    }

    /// Regular trading hours: 9:30 AM - 4:00 PM ET.
    #[must_use]
    pub fn regular_trading_hours(&self) -> bool {
        (flags::trade::RTH_START_MS..=flags::trade::RTH_END_MS).contains(&self.ms_of_day)
    }

    #[must_use]
    pub fn is_seller(&self) -> bool {
        self.ext_condition1 == flags::trade::SELLER_CONDITION
    }
}

impl OptionContract {
    /// `true` when `right` == 'C' (ASCII 67).
    #[inline]
    pub fn is_call(&self) -> bool {
        self.right == 67
    }
    /// `true` when `right` == 'P' (ASCII 80).
    #[inline]
    pub fn is_put(&self) -> bool {
        self.right == 80
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Layout asserts -- pin the generated struct sizes/alignments to the values
//  the C / Go FFI mirrors and `tick_layout_asserts.hpp.inc` rely on. A schema
//  edit that drifts a layout is caught here on `cargo test --workspace -p tdbe`
//  before it lands on the FFI side.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod layout_asserts {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn calendar_day_layout() {
        assert_eq!(size_of::<CalendarDay>(), 64);
        assert_eq!(align_of::<CalendarDay>(), 64);
    }

    #[test]
    fn eod_tick_layout() {
        assert_eq!(size_of::<EodTick>(), 128);
        assert_eq!(align_of::<EodTick>(), 64);
    }

    #[test]
    fn greeks_tick_layout() {
        assert_eq!(size_of::<GreeksTick>(), 256);
        assert_eq!(align_of::<GreeksTick>(), 64);
    }

    #[test]
    fn interest_rate_tick_layout() {
        assert_eq!(size_of::<InterestRateTick>(), 64);
        assert_eq!(align_of::<InterestRateTick>(), 64);
    }

    #[test]
    fn iv_tick_layout() {
        assert_eq!(size_of::<IvTick>(), 64);
        assert_eq!(align_of::<IvTick>(), 64);
    }

    #[test]
    fn market_value_tick_layout() {
        assert_eq!(size_of::<MarketValueTick>(), 64);
        assert_eq!(align_of::<MarketValueTick>(), 64);
    }

    #[test]
    fn ohlc_tick_layout() {
        assert_eq!(size_of::<OhlcTick>(), 128);
        assert_eq!(align_of::<OhlcTick>(), 64);
    }

    #[test]
    fn open_interest_tick_layout() {
        assert_eq!(size_of::<OpenInterestTick>(), 64);
        assert_eq!(align_of::<OpenInterestTick>(), 64);
    }

    #[test]
    fn price_tick_layout() {
        assert_eq!(size_of::<PriceTick>(), 64);
        assert_eq!(align_of::<PriceTick>(), 64);
    }

    #[test]
    fn quote_tick_layout() {
        assert_eq!(size_of::<QuoteTick>(), 128);
        assert_eq!(align_of::<QuoteTick>(), 64);
    }

    #[test]
    fn trade_quote_tick_layout() {
        assert_eq!(size_of::<TradeQuoteTick>(), 192);
        assert_eq!(align_of::<TradeQuoteTick>(), 64);
    }

    #[test]
    fn trade_tick_layout() {
        assert_eq!(size_of::<TradeTick>(), 128);
        assert_eq!(align_of::<TradeTick>(), 64);
    }
}
