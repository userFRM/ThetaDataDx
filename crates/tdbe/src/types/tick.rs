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
impl_contract_id!(GreeksAllTick);
impl_contract_id!(GreeksFirstOrderTick);
impl_contract_id!(GreeksSecondOrderTick);
impl_contract_id!(GreeksThirdOrderTick);
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
    fn greeks_all_tick_layout() {
        // Full-union Greeks. Pinned size/align match the schema-derived
        // figure in `sdks/cpp/include/tick_layout_asserts.hpp.inc` and the
        // C-mirror tail-padding hand-written in `sdks/cpp/include/thetadx.h`
        // and `sdks/go/tick_ffi_mirrors.go`.
        assert_eq!(size_of::<GreeksAllTick>(), 256);
        assert_eq!(align_of::<GreeksAllTick>(), 64);
    }

    #[test]
    fn greeks_first_order_tick_layout() {
        assert_eq!(size_of::<GreeksFirstOrderTick>(), 128);
        assert_eq!(align_of::<GreeksFirstOrderTick>(), 64);
    }

    #[test]
    fn greeks_second_order_tick_layout() {
        assert_eq!(size_of::<GreeksSecondOrderTick>(), 128);
        assert_eq!(align_of::<GreeksSecondOrderTick>(), 64);
    }

    #[test]
    fn greeks_third_order_tick_layout() {
        assert_eq!(size_of::<GreeksThirdOrderTick>(), 128);
        assert_eq!(align_of::<GreeksThirdOrderTick>(), 64);
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

    // Per-field offset asserts. Field-offset drift sneaks past
    // `size_of` / `align_of` checks alone -- e.g. swapping the order of
    // two same-size fields keeps total size constant but moves every
    // offset. The asserts below pin every observable Rust-side field
    // offset that the C / Go FFI mirrors index into via `offsetof()`.

    #[test]
    fn quote_tick_field_offsets() {
        // QuoteTick is the canonical drift trap: `midpoint` lives AFTER
        // the contract_id triple (`expiration`, `strike`, `right`) so the
        // generator MUST emit them in that order. Reordering shifts
        // offsets the C header (`sdks/cpp/include/thetadx.h:191-211`) and
        // Go FFI mirror rely on.
        use std::mem::offset_of;
        assert_eq!(offset_of!(QuoteTick, ms_of_day), 0);
        assert_eq!(offset_of!(QuoteTick, bid_size), 4);
        assert_eq!(offset_of!(QuoteTick, bid_exchange), 8);
        assert_eq!(offset_of!(QuoteTick, bid), 16);
        assert_eq!(offset_of!(QuoteTick, bid_condition), 24);
        assert_eq!(offset_of!(QuoteTick, ask_size), 28);
        assert_eq!(offset_of!(QuoteTick, ask_exchange), 32);
        assert_eq!(offset_of!(QuoteTick, ask), 40);
        assert_eq!(offset_of!(QuoteTick, ask_condition), 48);
        assert_eq!(offset_of!(QuoteTick, date), 52);
        assert_eq!(offset_of!(QuoteTick, expiration), 56);
        assert_eq!(offset_of!(QuoteTick, strike), 64);
        assert_eq!(offset_of!(QuoteTick, right), 72);
        // `midpoint` MUST come last, after the contract_id triple.
        assert_eq!(offset_of!(QuoteTick, midpoint), 80);
    }

    #[test]
    fn greeks_all_tick_field_offsets() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(GreeksAllTick, ms_of_day), 0);
        // First f64 lands at the next 8-byte boundary (4 bytes pad).
        assert_eq!(offset_of!(GreeksAllTick, bid), 8);
        assert_eq!(offset_of!(GreeksAllTick, ask), 16);
        assert_eq!(offset_of!(GreeksAllTick, implied_volatility), 24);
        assert_eq!(offset_of!(GreeksAllTick, delta), 32);
        // Underlying snapshot pair lands after every Greek (vera).
        assert_eq!(offset_of!(GreeksAllTick, underlying_ms_of_day), 200);
        assert_eq!(offset_of!(GreeksAllTick, underlying_price), 208);
        assert_eq!(offset_of!(GreeksAllTick, date), 216);
        // contract_id triple comes last (no `midpoint` on Greeks ticks).
        assert_eq!(offset_of!(GreeksAllTick, expiration), 220);
        assert_eq!(offset_of!(GreeksAllTick, strike), 224);
        assert_eq!(offset_of!(GreeksAllTick, right), 232);
    }

    #[test]
    fn greeks_first_order_tick_field_offsets() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(GreeksFirstOrderTick, ms_of_day), 0);
        assert_eq!(offset_of!(GreeksFirstOrderTick, bid), 8);
        assert_eq!(offset_of!(GreeksFirstOrderTick, ask), 16);
        assert_eq!(offset_of!(GreeksFirstOrderTick, delta), 24);
        assert_eq!(offset_of!(GreeksFirstOrderTick, theta), 32);
        assert_eq!(offset_of!(GreeksFirstOrderTick, vega), 40);
        assert_eq!(offset_of!(GreeksFirstOrderTick, rho), 48);
        assert_eq!(offset_of!(GreeksFirstOrderTick, epsilon), 56);
        assert_eq!(offset_of!(GreeksFirstOrderTick, lambda), 64);
        assert_eq!(offset_of!(GreeksFirstOrderTick, implied_volatility), 72);
        assert_eq!(offset_of!(GreeksFirstOrderTick, iv_error), 80);
        assert_eq!(offset_of!(GreeksFirstOrderTick, underlying_ms_of_day), 88);
        assert_eq!(offset_of!(GreeksFirstOrderTick, underlying_price), 96);
        assert_eq!(offset_of!(GreeksFirstOrderTick, date), 104);
        assert_eq!(offset_of!(GreeksFirstOrderTick, expiration), 108);
        assert_eq!(offset_of!(GreeksFirstOrderTick, strike), 112);
        assert_eq!(offset_of!(GreeksFirstOrderTick, right), 120);
    }

    #[test]
    fn greeks_second_order_tick_field_offsets() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(GreeksSecondOrderTick, ms_of_day), 0);
        assert_eq!(offset_of!(GreeksSecondOrderTick, bid), 8);
        assert_eq!(offset_of!(GreeksSecondOrderTick, ask), 16);
        assert_eq!(offset_of!(GreeksSecondOrderTick, gamma), 24);
        assert_eq!(offset_of!(GreeksSecondOrderTick, vanna), 32);
        assert_eq!(offset_of!(GreeksSecondOrderTick, charm), 40);
        assert_eq!(offset_of!(GreeksSecondOrderTick, vomma), 48);
        assert_eq!(offset_of!(GreeksSecondOrderTick, veta), 56);
        assert_eq!(offset_of!(GreeksSecondOrderTick, implied_volatility), 64);
        assert_eq!(offset_of!(GreeksSecondOrderTick, iv_error), 72);
        assert_eq!(offset_of!(GreeksSecondOrderTick, underlying_ms_of_day), 80);
        assert_eq!(offset_of!(GreeksSecondOrderTick, underlying_price), 88);
        assert_eq!(offset_of!(GreeksSecondOrderTick, date), 96);
        assert_eq!(offset_of!(GreeksSecondOrderTick, expiration), 100);
        assert_eq!(offset_of!(GreeksSecondOrderTick, strike), 104);
        assert_eq!(offset_of!(GreeksSecondOrderTick, right), 112);
    }

    #[test]
    fn greeks_third_order_tick_field_offsets() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(GreeksThirdOrderTick, ms_of_day), 0);
        assert_eq!(offset_of!(GreeksThirdOrderTick, bid), 8);
        assert_eq!(offset_of!(GreeksThirdOrderTick, ask), 16);
        assert_eq!(offset_of!(GreeksThirdOrderTick, speed), 24);
        assert_eq!(offset_of!(GreeksThirdOrderTick, zomma), 32);
        assert_eq!(offset_of!(GreeksThirdOrderTick, color), 40);
        assert_eq!(offset_of!(GreeksThirdOrderTick, ultima), 48);
        assert_eq!(offset_of!(GreeksThirdOrderTick, implied_volatility), 56);
        assert_eq!(offset_of!(GreeksThirdOrderTick, iv_error), 64);
        assert_eq!(offset_of!(GreeksThirdOrderTick, underlying_ms_of_day), 72);
        assert_eq!(offset_of!(GreeksThirdOrderTick, underlying_price), 80);
        assert_eq!(offset_of!(GreeksThirdOrderTick, date), 88);
        assert_eq!(offset_of!(GreeksThirdOrderTick, expiration), 92);
        assert_eq!(offset_of!(GreeksThirdOrderTick, strike), 96);
        assert_eq!(offset_of!(GreeksThirdOrderTick, right), 104);
    }

    #[test]
    fn trade_tick_field_offsets() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(TradeTick, ms_of_day), 0);
        assert_eq!(offset_of!(TradeTick, exchange), 32);
        assert_eq!(offset_of!(TradeTick, price), 40);
        assert_eq!(offset_of!(TradeTick, condition_flags), 48);
        assert_eq!(offset_of!(TradeTick, date), 64);
        assert_eq!(offset_of!(TradeTick, expiration), 68);
        assert_eq!(offset_of!(TradeTick, strike), 72);
        assert_eq!(offset_of!(TradeTick, right), 80);
    }
}
