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
//!   (because of the `String` `symbol` field) so the macro doesn't apply.
//!
//! The structs themselves are generated at build-time from
//! `crates/thetadatadx/tick_schema.toml` by
//! `cargo run -p thetadatadx --bin generate_sdk_surfaces`.

include!("generated/tick.rs");

// ─────────────────────────────────────────────────────────────────────────────
//  Contract identification helpers
// ─────────────────────────────────────────────────────────────────────────────

macro_rules! impl_contract_id {
    ($ty:ident) => {
        impl $ty {
            /// `true` when `right` is `'C'` (call).
            #[inline]
            pub fn is_call(&self) -> bool {
                self.right == 'C'
            }
            /// `true` when `right` is `'P'` (put).
            #[inline]
            pub fn is_put(&self) -> bool {
                self.right == 'P'
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
impl_contract_id!(GreeksEodTick);
impl_contract_id!(GreeksFirstOrderTick);
impl_contract_id!(GreeksSecondOrderTick);
impl_contract_id!(GreeksThirdOrderTick);
impl_contract_id!(IvTick);
impl_contract_id!(TradeGreeksAllTick);
impl_contract_id!(TradeGreeksFirstOrderTick);
impl_contract_id!(TradeGreeksSecondOrderTick);
impl_contract_id!(TradeGreeksThirdOrderTick);
impl_contract_id!(TradeGreeksImpliedVolatilityTick);

// ─────────────────────────────────────────────────────────────────────────────
//  Hand-written impl blocks
// ─────────────────────────────────────────────────────────────────────────────

use crate::tdbe::flags;

impl TradeTick {
    /// `true` when the trade condition falls in the cancellation range.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        flags::trade::CANCELLED_RANGE.contains(&self.condition)
    }

    /// `true` when the condition flags carry the "do not update last" bit.
    #[must_use]
    pub fn trade_condition_no_last(&self) -> bool {
        self.condition_flags & flags::condition_flags::NO_LAST == flags::condition_flags::NO_LAST
    }

    /// `true` when the price flags carry the "sets last" bit.
    #[must_use]
    pub fn price_condition_set_last(&self) -> bool {
        self.price_flags & flags::price_flags::SET_LAST == flags::price_flags::SET_LAST
    }

    /// `true` when `volume_type` marks this trade as incremental volume.
    #[must_use]
    pub fn is_incremental_volume(&self) -> bool {
        self.volume_type == flags::volume::INCREMENTAL
    }

    /// `true` when `ms_of_day` falls within regular trading hours
    /// (9:30 AM - 4:00 PM ET).
    #[must_use]
    pub fn regular_trading_hours(&self) -> bool {
        (flags::trade::RTH_START_MS..=flags::trade::RTH_END_MS).contains(&self.ms_of_day)
    }

    /// `true` when the extended condition marks this trade as seller-initiated.
    #[must_use]
    pub fn is_seller(&self) -> bool {
        self.ext_condition1 == flags::trade::SELLER_CONDITION
    }
}

impl OptionContract {
    /// `true` when `right` is `'C'` (call).
    #[inline]
    pub fn is_call(&self) -> bool {
        self.right == 'C'
    }
    /// `true` when `right` is `'P'` (put).
    #[inline]
    pub fn is_put(&self) -> bool {
        self.right == 'P'
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Layout asserts -- pin the generated struct sizes/alignments AND every
//  field offset to the schema-derived figures every C FFI mirror and
//  `tick_layout_asserts.hpp.inc` rely on. The whole module is generator-
//  emitted from `tick_schema.toml` so adding a tick type picks up coverage
//  automatically. A schema edit that drifts any layout is caught here on
//  `cargo test -p tdbe` before it lands on the FFI side.
// ─────────────────────────────────────────────────────────────────────────────

include!("generated/tick_layout_asserts.rs");
