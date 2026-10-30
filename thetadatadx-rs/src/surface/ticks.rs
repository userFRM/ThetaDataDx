//! The tick table.
//!
//! `__ticks!` lists every tick type the endpoints return, in
//! `tick_schema.toml` order. It names the types only: their columns, layouts
//! and accessors are still generated from `tick_schema.toml`.

/// Every tick type, in schema order.
///
/// `__ticks! { consumer { ctx.. } }` expands to
/// `consumer! { ctx.. TradeTick QuoteTick .. }`.
#[doc(hidden)]
#[macro_export]
macro_rules! __ticks {
    ($m:ident { $($ctx:tt)* }) => {
        $m! {
            $($ctx)*
            TradeTick
            QuoteTick
            OhlcTick
            EodTick
            OpenInterestTick
            TradeQuoteTick
            MarketValueTick
            GreeksAllTick
            GreeksEodTick
            GreeksFirstOrderTick
            GreeksSecondOrderTick
            GreeksThirdOrderTick
            IvTick
            TradeGreeksAllTick
            TradeGreeksFirstOrderTick
            TradeGreeksSecondOrderTick
            TradeGreeksThirdOrderTick
            TradeGreeksImpliedVolatilityTick
            PriceTick
            IndexPriceAtTimeTick
            CalendarDay
            InterestRateTick
            OptionContract
            IntervalQuoteTick
        }
    };
}
