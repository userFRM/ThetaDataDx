//! The tick table.
//!
//! Every tick type the endpoints return is written once, in `__ticks!`. The
//! engine expands it into the tick structs, their decoders and their Arrow
//! and Polars builders; the Python and TypeScript bindings into their row
//! classes and converters; the C ABI into its array types and entry points;
//! `generate_sdk_surfaces` into the C tick structs and declarations, the C++
//! layout asserts, flag accessors and Arrow IPC helpers, the Python type
//! stub's tick classes and the documentation.
//!
//! A consumer passes the name of one of its own macros and receives every
//! entry, in table order, after the tokens it passed, as `__endpoints!` does.
//! `__tick!` resolves one entry into the fixed shape every consumer reads.
//!
//! # Entries
//!
//! ```ignore
//! #[contract_id]
//! EodTick {
//!     doc: ["End-of-day tick" -- 20 fields. "Full EOD snapshot with OHLC + quote.", ".."],
//!     required: [created],
//!     columns: [
//!         created(created_ms_of_day): eod_num "Time the vendor generated the report, ..",
//!         #[nullable] bid_exchange: i32 "Exchange code of the closing NBBO bid. ..",
//!     ],
//!     timestamps: [created_timestamp_ms(date, created_ms_of_day)],
//!     flags: [],
//! }
//! ```
//!
//! - `#[contract_id]` adds the `expiration`, `strike` and `right` fields a
//!   wildcard option response fills. `#[owned]` marks a row that holds a
//!   `String`: it is not `Copy` and has no C layout.
//! - `doc` is the documentation, one string per paragraph. A first paragraph
//!   written `"Head" -- N fields. "Tail"` renders as is in Rust, where `N`
//!   counts the struct fields, and as `Head. Tail` in the other languages.
//! - `required` names the columns, by wire header, a response with rows must
//!   carry.
//!   `CalendarDay` and `OptionContract` are decoded by hand and list none.
//! - A column reads `wire(field): type "description"`; `(field)` names the
//!   struct field where it differs from the wire header. `#[nullable]` adds a
//!   `has_<field>` flag that tells a reported zero from an absent value.
//! - The column types are `i32`, `i64` and `f64`; `price`, a price cell read
//!   as `f64`; `String`; `right`, an option right held as a `char`;
//!   `calendar_status`, the vendor's day type; and the end-of-day report
//!   shapes `eod_num`, `eod_num64`, `eod_date` and `eod_price`, which also
//!   accept a price or timestamp cell.
//! - `timestamps` lists the epoch accessors, each joining a date field and a
//!   milliseconds-of-day field.
//! - `flags` lists the boolean accessors decoded from an integer column.

/// Every tick type, in table order.
///
/// `__ticks! { consumer { ctx.. } }` expands to `consumer! { ctx.. <entry>
/// <entry> .. }`.
#[doc(hidden)]
#[macro_export]
macro_rules! __ticks {
    ($m:ident { $($ctx:tt)* }) => {
        $m! {
            $($ctx)*
            #[contract_id]
            TradeTick {
                doc: ["Trade tick" -- 18 fields. "Core unit of trade data."],
                required: [ms_of_day price],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    sequence: i32 "Exchange-assigned trade sequence number.",
                    #[nullable] ext_condition1: i32 "Additional trade condition code.",
                    #[nullable] ext_condition2: i32 "Additional trade condition code.",
                    #[nullable] ext_condition3: i32 "Additional trade condition code.",
                    #[nullable] ext_condition4: i32 "Additional trade condition code.",
                    #[nullable] condition: i32 "Trade condition code. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    size: i32 "Number of contracts or shares traded.",
                    #[nullable] exchange: i32 "Exchange code where the trade executed. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    price: price "Trade price.",
                    condition_flags: i32 "Trade condition flags bitmap.",
                    price_flags: i32 "Trade price flags bitmap.",
                    volume_type: i32 "Volume reporting mode, as sent. The vendor does not define this column.",
                    records_back: i32 "Offset of this record behind the most recent record.",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day)],
                flags: [is_cancelled: condition in 40..=44 "True when the trade carries a cancelled-trade condition (codes 40-44)."],
            }
            #[contract_id]
            QuoteTick {
                doc: [
                    "Quote tick" -- 13 fields. "NBBO quote data.",
                    "Some storage tiers return only `ms_of_day`, `bid_size`, `bid`, `ask_size`, `ask` and `date`; the exchange and condition fields are then absent.",
                ],
                required: [ms_of_day bid ask],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    bid_size: i32 "Last NBBO bid size.",
                    #[nullable] bid_exchange: i32 "Exchange code of the NBBO bid. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    bid: price "Last NBBO bid price.",
                    #[nullable] bid_condition: i32 "Quote condition code on the bid side. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    ask_size: i32 "Last NBBO ask size.",
                    #[nullable] ask_exchange: i32 "Exchange code of the NBBO ask. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    ask: price "Last NBBO ask price.",
                    #[nullable] ask_condition: i32 "Quote condition code on the ask side. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day)],
                flags: [],
            }
            #[contract_id]
            OhlcTick {
                doc: [
                    "OHLC tick" -- 12 fields. "Aggregated bar data including SIP-rule VWAP.",
                    "`stock_history_ohlc`, `option_history_ohlc` and `index_history_ohlc` carry every column. The snapshot variants (`*_snapshot_ohlc`) omit `vwap`, which then reads `0.0`, as `volume` and `count` read `0` on quote-only intraday bars.",
                ],
                required: [ms_of_day open],
                columns: [
                    ms_of_day: i32 "Opening time of the bar, milliseconds since midnight ET.",
                    open: price "Opening trade price.",
                    high: price "Highest traded price.",
                    low: price "Lowest traded price.",
                    close: price "Closing traded price.",
                    volume: i64 "Number of contracts or shares traded.",
                    count: i64 "Number of trades.",
                    vwap: price "Volume-weighted average price of the session.",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day)],
                flags: [],
            }
            #[contract_id]
            EodTick {
                doc: [
                    "End-of-day tick" -- 20 fields. "Full EOD snapshot with OHLC + quote.",
                    "The two time columns carry the vendor's v3 field semantics under their v3 names: `created` is when the EOD report was generated (~17:15 ET, NOT a trade time) and `last_trade` is the time of the day's final trade. On a day with no trades the report zero-fills the trade-derived columns: `open` / `high` / `low` / `close` are `0.0` and `last_trade_ms_of_day` is `0`; the quote-side columns still carry the closing NBBO snapshot.",
                ],
                required: [created],
                columns: [
                    created(created_ms_of_day): eod_num "EOD report creation time (NOT a trade time), milliseconds since midnight ET.",
                    last_trade(last_trade_ms_of_day): eod_num "Time of the day's last trade, milliseconds since midnight ET. 0 when no trades printed that day.",
                    open: eod_price "Opening trade price. 0.0 when no trades printed that day.",
                    high: eod_price "Highest traded price. 0.0 when no trades printed that day.",
                    low: eod_price "Lowest traded price. 0.0 when no trades printed that day.",
                    close: eod_price "Closing traded price. 0.0 when no trades printed that day.",
                    volume: eod_num64 "Number of contracts or shares traded.",
                    count: eod_num64 "Number of trades.",
                    bid_size: eod_num "Last NBBO bid size.",
                    #[nullable] bid_exchange: eod_num "Exchange code of the NBBO bid. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    bid: eod_price "Last NBBO bid price.",
                    #[nullable] bid_condition: eod_num "Quote condition code on the bid side. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    ask_size: eod_num "Last NBBO ask size.",
                    #[nullable] ask_exchange: eod_num "Exchange code of the NBBO ask. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    ask: eod_price "Last NBBO ask price.",
                    #[nullable] ask_condition: eod_num "Quote condition code on the ask side. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    date: eod_date "Trading date as a YYYYMMDD integer.",
                ],
                timestamps: [created_timestamp_ms(date, created_ms_of_day), last_trade_timestamp_ms(date, last_trade_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            OpenInterestTick {
                doc: ["Open interest tick" -- 6 fields.],
                required: [ms_of_day],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    open_interest: i32 "Total outstanding contracts.",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day)],
                flags: [],
            }
            #[contract_id]
            TradeQuoteTick {
                doc: ["Combined trade + quote tick" -- 27 fields.],
                required: [ms_of_day price],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    sequence: i32 "Exchange-assigned trade sequence number.",
                    #[nullable] ext_condition1: i32 "Additional trade condition code.",
                    #[nullable] ext_condition2: i32 "Additional trade condition code.",
                    #[nullable] ext_condition3: i32 "Additional trade condition code.",
                    #[nullable] ext_condition4: i32 "Additional trade condition code.",
                    #[nullable] condition: i32 "Trade condition code. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    size: i32 "Number of contracts or shares traded.",
                    #[nullable] exchange: i32 "Exchange code where the trade executed. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    price: price "Trade price.",
                    condition_flags: i32 "Trade condition flags bitmap.",
                    price_flags: i32 "Trade price flags bitmap.",
                    volume_type: i32 "Volume reporting mode, as sent. The vendor does not define this column.",
                    records_back: i32 "Offset of this record behind the most recent record.",
                    quote_ms_of_day: i32 "Timestamp of the paired quote, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    bid_size: i32 "Last NBBO bid size.",
                    #[nullable] bid_exchange: i32 "Exchange code of the NBBO bid. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    bid: price "NBBO bid at the time of the trade.",
                    #[nullable] bid_condition: i32 "Quote condition code on the bid side. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    ask_size: i32 "Last NBBO ask size.",
                    #[nullable] ask_exchange: i32 "Exchange code of the NBBO ask. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    ask: price "NBBO ask at the time of the trade.",
                    #[nullable] ask_condition: i32 "Quote condition code on the ask side. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), quote_timestamp_ms(date, quote_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            MarketValueTick {
                doc: ["Market value tick -- quoted bid/ask/price for a symbol."],
                required: [ms_of_day],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    market_bid: price "Last market bid.",
                    market_ask: price "Last market ask.",
                    market_price: price "Market value derived from the last NBBO.",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day)],
                flags: [],
            }
            #[contract_id]
            GreeksAllTick {
                doc: [
                    "Full union Greeks tick -- every Greek the v3 server publishes on the `option_*_greeks_all` endpoints.",
                    "The vendor's per-order endpoints (`option_*_greeks_first_order`, `_second_order`, `_third_order`) emit strict subsets of these columns and bind to `GreeksFirstOrderTick`, `GreeksSecondOrderTick` and `GreeksThirdOrderTick` respectively. `option_*_greeks_implied_volatility` binds to `IvTick`, and `option_*_greeks_eod` to `GreeksEodTick`.",
                ],
                required: [ms_of_day],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    bid: price "Last NBBO bid price.",
                    ask: price "Last NBBO ask price.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    delta: f64 "Sensitivity of the option value to a one-dollar move in the underlying.",
                    gamma: f64 "Rate of change of delta with respect to the underlying price.",
                    theta: f64 "Sensitivity of the option value to the passage of time.",
                    vega: f64 "Sensitivity of the option value to a change in implied volatility.",
                    rho: f64 "Sensitivity of the option value to the risk-free rate.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    vanna: f64 "Sensitivity of delta to a change in implied volatility.",
                    charm: f64 "Rate of change of delta over time.",
                    vomma: f64 "Sensitivity of vega to a change in implied volatility.",
                    veta: f64 "Rate of change of vega over time.",
                    speed: f64 "Rate of change of gamma with respect to the underlying price.",
                    zomma: f64 "Sensitivity of gamma to a change in implied volatility.",
                    color: f64 "Rate of change of gamma over time.",
                    ultima: f64 "Sensitivity of vomma to a change in implied volatility.",
                    d1: f64 "Black-Scholes d1 intermediate term.",
                    d2: f64 "Black-Scholes d2 intermediate term.",
                    dual_delta: f64 "Sensitivity of the option value to the strike price.",
                    dual_gamma: f64 "Second derivative of the option value with respect to the strike.",
                    epsilon: f64 "Sensitivity of the option value to the dividend yield.",
                    lambda: f64 "Percent change in the option value per percent change in the underlying.",
                    vera: f64 "Sensitivity of rho to a change in implied volatility.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            GreeksEodTick {
                doc: ["End-of-day union Greeks tick -- every Greek the v3 server publishes on `option_history_greeks_eod`, paired with the twelve EOD trade/quote context columns (`open`, `high`, `low`, `close`, `volume`, `count`, `bid_size`, `bid_exchange`, `bid_condition`, `ask_size`, `ask_exchange`, `ask_condition`) that identify the daily bar + closing NBBO snapshot the Greeks were calculated against."],
                required: [ms_of_day],
                columns: [
                    ms_of_day: i32 "EOD report creation time, milliseconds since midnight ET.",
                    open: price "Opening trade price.",
                    high: price "Highest traded price.",
                    low: price "Lowest traded price.",
                    close: price "Closing traded price.",
                    volume: i64 "Number of contracts or shares traded.",
                    count: i64 "Number of trades.",
                    bid_size: i32 "Last NBBO bid size.",
                    #[nullable] bid_exchange: i32 "Exchange code of the NBBO bid. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    bid: price "Last NBBO bid price.",
                    #[nullable] bid_condition: i32 "Quote condition code on the bid side. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    ask_size: i32 "Last NBBO ask size.",
                    #[nullable] ask_exchange: i32 "Exchange code of the NBBO ask. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    ask: price "Last NBBO ask price.",
                    #[nullable] ask_condition: i32 "Quote condition code on the ask side. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    delta: f64 "Sensitivity of the option value to a one-dollar move in the underlying.",
                    theta: f64 "Sensitivity of the option value to the passage of time.",
                    vega: f64 "Sensitivity of the option value to a change in implied volatility.",
                    rho: f64 "Sensitivity of the option value to the risk-free rate.",
                    epsilon: f64 "Sensitivity of the option value to the dividend yield.",
                    lambda: f64 "Percent change in the option value per percent change in the underlying.",
                    gamma: f64 "Rate of change of delta with respect to the underlying price.",
                    vanna: f64 "Sensitivity of delta to a change in implied volatility.",
                    charm: f64 "Rate of change of delta over time.",
                    vomma: f64 "Sensitivity of vega to a change in implied volatility.",
                    veta: f64 "Rate of change of vega over time.",
                    vera: f64 "Sensitivity of rho to a change in implied volatility.",
                    speed: f64 "Rate of change of gamma with respect to the underlying price.",
                    zomma: f64 "Sensitivity of gamma to a change in implied volatility.",
                    color: f64 "Rate of change of gamma over time.",
                    ultima: f64 "Sensitivity of vomma to a change in implied volatility.",
                    d1: f64 "Black-Scholes d1 intermediate term.",
                    d2: f64 "Black-Scholes d2 intermediate term.",
                    dual_delta: f64 "Sensitivity of the option value to the strike price.",
                    dual_gamma: f64 "Second derivative of the option value with respect to the strike.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            GreeksFirstOrderTick {
                doc: ["First-order Greeks tick -- the strict column subset emitted by the vendor's `option_*_greeks_first_order` endpoints (delta / theta / vega / rho / epsilon / lambda) plus the bid/ask quote pair, the IV pair, and the underlying snapshot used to derive each Greek."],
                required: [ms_of_day],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    bid: price "Last NBBO bid price.",
                    ask: price "Last NBBO ask price.",
                    delta: f64 "Sensitivity of the option value to a one-dollar move in the underlying.",
                    theta: f64 "Sensitivity of the option value to the passage of time.",
                    vega: f64 "Sensitivity of the option value to a change in implied volatility.",
                    rho: f64 "Sensitivity of the option value to the risk-free rate.",
                    epsilon: f64 "Sensitivity of the option value to the dividend yield.",
                    lambda: f64 "Percent change in the option value per percent change in the underlying.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            GreeksSecondOrderTick {
                doc: ["Second-order Greeks tick -- the strict column subset emitted by the vendor's `option_*_greeks_second_order` endpoints (gamma / vanna / charm / vomma / veta) plus the bid/ask quote pair, the IV pair, and the underlying snapshot."],
                required: [ms_of_day],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    bid: price "Last NBBO bid price.",
                    ask: price "Last NBBO ask price.",
                    gamma: f64 "Rate of change of delta with respect to the underlying price.",
                    vanna: f64 "Sensitivity of delta to a change in implied volatility.",
                    charm: f64 "Rate of change of delta over time.",
                    vomma: f64 "Sensitivity of vega to a change in implied volatility.",
                    veta: f64 "Rate of change of vega over time.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            GreeksThirdOrderTick {
                doc: ["Third-order Greeks tick -- the strict column subset emitted by the vendor's `option_*_greeks_third_order` endpoints (speed / zomma / color / ultima) plus the bid/ask quote pair, the IV pair, and the underlying snapshot. The vendor's third-order schema does not publish `vera`."],
                required: [ms_of_day],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    bid: price "Last NBBO bid price.",
                    ask: price "Last NBBO ask price.",
                    speed: f64 "Rate of change of gamma with respect to the underlying price.",
                    zomma: f64 "Sensitivity of gamma to a change in implied volatility.",
                    color: f64 "Rate of change of gamma over time.",
                    ultima: f64 "Sensitivity of vomma to a change in implied volatility.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            IvTick {
                doc: [
                    "Implied volatility tick" -- 15 fields.,
                    "The snapshot variant (`option_snapshot_greeks_implied_volatility`) carries only `ms_of_day`, `implied_volatility`, `iv_error` and `date`; the other fields then read zero.",
                ],
                required: [ms_of_day],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    bid: price "Last NBBO bid price.",
                    bid_implied_volatility: f64 "Implied volatility solved at the bid price. The vendor names this column `BID_IMPLIED_VOL`.",
                    midpoint: price "Midpoint of the bid and ask prices.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    ask: price "Last NBBO ask price.",
                    ask_implied_volatility: f64 "Implied volatility solved at the ask price. The vendor names this column `ASK_IMPLIED_VOL`.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            TradeGreeksAllTick {
                doc: ["Per-trade union Greeks tick -- every Greek the v3 server publishes on `option_history_trade_greeks_all`, paired with the trade-side execution columns (`sequence`, `ext_condition1..4`, `condition`, `size`, `exchange`, `price`) that identify which OPRA print each Greek was calculated against."],
                required: [ms_of_day price],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    sequence: i32 "Exchange-assigned trade sequence number.",
                    #[nullable] ext_condition1: i32 "Additional trade condition code.",
                    #[nullable] ext_condition2: i32 "Additional trade condition code.",
                    #[nullable] ext_condition3: i32 "Additional trade condition code.",
                    #[nullable] ext_condition4: i32 "Additional trade condition code.",
                    #[nullable] condition: i32 "Trade condition code. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    size: i32 "Number of contracts or shares traded.",
                    #[nullable] exchange: i32 "Exchange code where the trade executed. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    price: price "Trade price.",
                    delta: f64 "Sensitivity of the option value to a one-dollar move in the underlying.",
                    theta: f64 "Sensitivity of the option value to the passage of time.",
                    vega: f64 "Sensitivity of the option value to a change in implied volatility.",
                    rho: f64 "Sensitivity of the option value to the risk-free rate.",
                    epsilon: f64 "Sensitivity of the option value to the dividend yield.",
                    lambda: f64 "Percent change in the option value per percent change in the underlying.",
                    gamma: f64 "Rate of change of delta with respect to the underlying price.",
                    vanna: f64 "Sensitivity of delta to a change in implied volatility.",
                    charm: f64 "Rate of change of delta over time.",
                    vomma: f64 "Sensitivity of vega to a change in implied volatility.",
                    veta: f64 "Rate of change of vega over time.",
                    vera: f64 "Sensitivity of rho to a change in implied volatility.",
                    speed: f64 "Rate of change of gamma with respect to the underlying price.",
                    zomma: f64 "Sensitivity of gamma to a change in implied volatility.",
                    color: f64 "Rate of change of gamma over time.",
                    ultima: f64 "Sensitivity of vomma to a change in implied volatility.",
                    d1: f64 "Black-Scholes d1 intermediate term.",
                    d2: f64 "Black-Scholes d2 intermediate term.",
                    dual_delta: f64 "Sensitivity of the option value to the strike price.",
                    dual_gamma: f64 "Second derivative of the option value with respect to the strike.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            TradeGreeksFirstOrderTick {
                doc: ["Per-trade first-order Greeks tick (delta / theta / vega / rho / epsilon / lambda) paired with the trade-side execution columns identifying the OPRA print each Greek was calculated against."],
                required: [ms_of_day price],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    sequence: i32 "Exchange-assigned trade sequence number.",
                    #[nullable] ext_condition1: i32 "Additional trade condition code.",
                    #[nullable] ext_condition2: i32 "Additional trade condition code.",
                    #[nullable] ext_condition3: i32 "Additional trade condition code.",
                    #[nullable] ext_condition4: i32 "Additional trade condition code.",
                    #[nullable] condition: i32 "Trade condition code. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    size: i32 "Number of contracts or shares traded.",
                    #[nullable] exchange: i32 "Exchange code where the trade executed. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    price: price "Trade price.",
                    delta: f64 "Sensitivity of the option value to a one-dollar move in the underlying.",
                    theta: f64 "Sensitivity of the option value to the passage of time.",
                    vega: f64 "Sensitivity of the option value to a change in implied volatility.",
                    rho: f64 "Sensitivity of the option value to the risk-free rate.",
                    epsilon: f64 "Sensitivity of the option value to the dividend yield.",
                    lambda: f64 "Percent change in the option value per percent change in the underlying.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            TradeGreeksSecondOrderTick {
                doc: ["Per-trade second-order Greeks tick (gamma / vanna / charm / vomma / veta) paired with the trade-side execution columns identifying the OPRA print each Greek was calculated against."],
                required: [ms_of_day price],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    sequence: i32 "Exchange-assigned trade sequence number.",
                    #[nullable] ext_condition1: i32 "Additional trade condition code.",
                    #[nullable] ext_condition2: i32 "Additional trade condition code.",
                    #[nullable] ext_condition3: i32 "Additional trade condition code.",
                    #[nullable] ext_condition4: i32 "Additional trade condition code.",
                    #[nullable] condition: i32 "Trade condition code. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    size: i32 "Number of contracts or shares traded.",
                    #[nullable] exchange: i32 "Exchange code where the trade executed. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    price: price "Trade price.",
                    gamma: f64 "Rate of change of delta with respect to the underlying price.",
                    vanna: f64 "Sensitivity of delta to a change in implied volatility.",
                    charm: f64 "Rate of change of delta over time.",
                    vomma: f64 "Sensitivity of vega to a change in implied volatility.",
                    veta: f64 "Rate of change of vega over time.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            TradeGreeksThirdOrderTick {
                doc: ["Per-trade third-order Greeks tick (speed / zomma / color / ultima) paired with the trade-side execution columns identifying the OPRA print each Greek was calculated against. The vendor's third-order schema does not publish `vera`."],
                required: [ms_of_day price],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    sequence: i32 "Exchange-assigned trade sequence number.",
                    #[nullable] ext_condition1: i32 "Additional trade condition code.",
                    #[nullable] ext_condition2: i32 "Additional trade condition code.",
                    #[nullable] ext_condition3: i32 "Additional trade condition code.",
                    #[nullable] ext_condition4: i32 "Additional trade condition code.",
                    #[nullable] condition: i32 "Trade condition code. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    size: i32 "Number of contracts or shares traded.",
                    #[nullable] exchange: i32 "Exchange code where the trade executed. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    price: price "Trade price.",
                    speed: f64 "Rate of change of gamma with respect to the underlying price.",
                    zomma: f64 "Sensitivity of gamma to a change in implied volatility.",
                    color: f64 "Rate of change of gamma over time.",
                    ultima: f64 "Sensitivity of vomma to a change in implied volatility.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            #[contract_id]
            TradeGreeksImpliedVolatilityTick {
                doc: ["Per-trade implied-volatility tick (single `implied_volatility` + `iv_error` pair, NOT the bid/mid/ask IV triple of the interval-sampled `IvTick`) paired with the trade-side execution columns identifying the OPRA print the IV was calculated against."],
                required: [ms_of_day price],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    sequence: i32 "Exchange-assigned trade sequence number.",
                    #[nullable] ext_condition1: i32 "Additional trade condition code.",
                    #[nullable] ext_condition2: i32 "Additional trade condition code.",
                    #[nullable] ext_condition3: i32 "Additional trade condition code.",
                    #[nullable] ext_condition4: i32 "Additional trade condition code.",
                    #[nullable] condition: i32 "Trade condition code. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    size: i32 "Number of contracts or shares traded.",
                    #[nullable] exchange: i32 "Exchange code where the trade executed. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    price: price "Trade price.",
                    implied_volatility: f64 "Implied volatility solved from the option price. The vendor names this column `IMPLIED_VOL`.",
                    iv_error: f64 "Residual pricing error of the implied-volatility solve.",
                    underlying_ms_of_day: i32 "Timestamp of the underlying price, milliseconds since midnight ET. The vendor names this column `MS_OF_DAY2`.",
                    underlying_price: price "Underlying price used in the calculation (midpoint of the underlying).",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                    underlying_date: i32 "Trading date of the underlying price as a YYYYMMDD integer, decoded from the same wire column as `underlying_ms_of_day`. It differs from `date` when the underlying price and the option quote come from different sessions.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day), underlying_timestamp_ms(underlying_date, underlying_ms_of_day)],
                flags: [],
            }
            PriceTick {
                doc: ["Price tick" -- 3 fields. "Generic price data point."],
                required: [ms_of_day price],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    price: price "Index value.",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day)],
                flags: [],
            }
            IndexPriceAtTimeTick {
                doc: ["Index price-at-time tick -- the trade-shaped row the v3 server publishes on `index_at_time_price`."],
                required: [ms_of_day price],
                columns: [
                    ms_of_day: i32 "Milliseconds since midnight Eastern Time.",
                    sequence: i32 "Exchange-assigned trade sequence number.",
                    #[nullable] ext_condition1: i32 "Additional trade condition code.",
                    #[nullable] ext_condition2: i32 "Additional trade condition code.",
                    #[nullable] ext_condition3: i32 "Additional trade condition code.",
                    #[nullable] ext_condition4: i32 "Additional trade condition code.",
                    #[nullable] condition: i32 "Trade condition code. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    size: i32 "Number of contracts or shares traded.",
                    #[nullable] exchange: i32 "Exchange code where the trade executed. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    price: price "Index value at the requested time.",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                ],
                timestamps: [timestamp_ms(date, ms_of_day)],
                flags: [],
            }
            CalendarDay {
                doc: [
                    "Calendar day" -- 4 fields. "Market open/close schedule.",
                    "`status` carries the vendor's own day-type vocabulary (`open` / `early_close` / `full_close` / `weekend`); unknown wire text fails decode loudly rather than degrading to a sentinel. `date` is `0` on the single-row `calendar_on_date` / `calendar_open_today` responses, where the server omits the date column.",
                ],
                required: [],
                columns: [
                    date: i32 "Calendar date as a YYYYMMDD integer. 0 when the server omits the column (single-day endpoints).",
                    open_time: i32 "Market open time, milliseconds since midnight ET.",
                    close_time: i32 "Market close time, milliseconds since midnight ET.",
                    status: calendar_status "Vendor day classification: open, early_close, full_close, or weekend.",
                ],
                timestamps: [],
                flags: [],
            }
            InterestRateTick {
                doc: ["Interest rate tick" -- 2 fields. "End-of-day interest rate (percent)."],
                required: [created],
                columns: [
                    created(date): i32 "Report creation date as a YYYYMMDD integer.",
                    rate: f64 "Interest rate expressed as a percent (1.25 means 1.25%).",
                ],
                timestamps: [],
                flags: [],
            }
            #[owned]
            OptionContract {
                doc: ["Option contract" -- 4 fields. "Contract specification."],
                required: [],
                columns: [
                    root(symbol): String "Underlying symbol of the contract.",
                    expiration: i32 "Expiration date of the contract as a YYYYMMDD integer.",
                    strike: price "Strike price of the contract in dollars.",
                    right: right "Contract right: 'C' for a call, 'P' for a put.",
                ],
                timestamps: [],
                flags: [],
            }
            #[contract_id]
            IntervalQuoteTick {
                doc: [
                    "Interval quote tick" -- 14 fields. "The NBBO in effect at one interval.",
                    "Returned by the quote flat files (`*_flat_file_quote`), which carry one row per interval in which a new quote was reported. Each row holds two times: `interval_ms_of_day`, the interval the row samples, and `quote_ms_of_day`, when the quote in effect at that interval was reported.",
                ],
                required: [interval_ms_of_day bid ask],
                columns: [
                    interval_ms_of_day: i32 "Time of the interval the row samples, milliseconds since midnight ET.",
                    quote_ms_of_day: i32 "Time the quote in effect at the interval was reported, milliseconds since midnight ET.",
                    bid_size: i32 "NBBO bid size.",
                    #[nullable] bid_exchange: i32 "Exchange code of the NBBO bid. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    bid: price "NBBO bid price.",
                    #[nullable] bid_condition: i32 "Quote condition code on the bid side. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    ask_size: i32 "NBBO ask size.",
                    #[nullable] ask_exchange: i32 "Exchange code of the NBBO ask. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    ask: price "NBBO ask price.",
                    #[nullable] ask_condition: i32 "Quote condition code on the ask side. Zero is a code the vendor assigns a meaning, so read `columns()` to tell a reported zero from a response that carried no such column.",
                    date: i32 "Trading date as a YYYYMMDD integer.",
                ],
                timestamps: [interval_timestamp_ms(date, interval_ms_of_day), quote_timestamp_ms(date, quote_ms_of_day)],
                flags: [],
            }
        }
    };
}

/// Resolves one `__ticks!` entry, then calls back.
///
/// `__tick! { [entry] => consumer { ctx.. } }` expands to
///
/// ```ignore
/// consumer! { ctx.. Name kind contract [..] doc [..] summary [..] more [..] required [..]
///     columns [{ field wire type [has_field?] "description" }..]
///     timestamps [{ accessor date time }..] flags [{ name field lo hi "doc" }..] }
/// ```
///
/// - `kind` is `pod` for a `Copy` row and `owned` for an `#[owned]` one.
/// - `contract` holds `expiration strike right` on a `#[contract_id]` row.
/// - `doc` is the first paragraph as `[head [count?] [tail?]]`, `summary` the
///   same paragraph without the field count, as the pieces to join, and `more`
///   the other paragraphs.
/// - A nullable column carries the name of its presence flag.
#[doc(hidden)]
#[macro_export]
macro_rules! __tick {
    ([#[contract_id] $name:ident $body:tt] => $($cb:tt)*) => {
        $crate::__tick! { @body [$name pod contract [expiration strike right]] $body $($cb)* }
    };
    ([#[owned] $name:ident $body:tt] => $($cb:tt)*) => {
        $crate::__tick! { @body [$name owned contract []] $body $($cb)* }
    };
    ([$name:ident $body:tt] => $($cb:tt)*) => {
        $crate::__tick! { @body [$name pod contract []] $body $($cb)* }
    };
    (@body $head:tt {
        doc: [$($doc:tt)*],
        required: [$($req:ident)*],
        columns: [$($cols:tt)*],
        timestamps: [$($acc:ident($date:ident, $ms:ident)),* $(,)?],
        flags: [$($flag:ident: $field:ident in $lo:tt..=$hi:tt $fdoc:tt),* $(,)?] $(,)?
    } $m:ident { $($ctx:tt)* }) => {
        $crate::__tick! { @doc [$($doc)*] [
            $m [$($ctx)*] $head [required [$($req)*]]
            [timestamps [$({ $acc $date $ms })*] flags [$({ $flag $field $lo $hi $fdoc })*]]
        ] [$($cols)*] }
    };
    (@doc [$head:tt -- $n:tt fields. $(, $more:tt)* $(,)?] $st:tt $cols:tt) => {
        $crate::__tick! { @cols $st [
            doc [$head [$n] []] summary [$head "."] more [$($more)*]
        ] [] $cols }
    };
    (@doc [$head:tt -- $n:tt fields. $tail:tt $(, $more:tt)* $(,)?] $st:tt $cols:tt) => {
        $crate::__tick! { @cols $st [
            doc [$head [$n] [$tail]] summary [$head ". " $tail] more [$($more)*]
        ] [] $cols }
    };
    (@doc [$first:tt $(, $more:tt)* $(,)?] $st:tt $cols:tt) => {
        $crate::__tick! { @cols $st [doc [$first [] []] summary [$first] more [$($more)*]] [] $cols }
    };
    (@cols $st:tt $doc:tt [$($done:tt)*] [#[nullable] $wire:ident($field:ident): $ty:ident $d:tt $(, $($rest:tt)*)?]) => {
        $crate::__tick! { @cols $st $doc [$($done)* { $field $wire $ty [[<has_ $field>]] $d }] [$($($rest)*)?] }
    };
    (@cols $st:tt $doc:tt [$($done:tt)*] [#[nullable] $wire:ident: $ty:ident $d:tt $(, $($rest:tt)*)?]) => {
        $crate::__tick! { @cols $st $doc [$($done)* { $wire $wire $ty [[<has_ $wire>]] $d }] [$($($rest)*)?] }
    };
    (@cols $st:tt $doc:tt [$($done:tt)*] [$wire:ident($field:ident): $ty:ident $d:tt $(, $($rest:tt)*)?]) => {
        $crate::__tick! { @cols $st $doc [$($done)* { $field $wire $ty [] $d }] [$($($rest)*)?] }
    };
    (@cols $st:tt $doc:tt [$($done:tt)*] [$wire:ident: $ty:ident $d:tt $(, $($rest:tt)*)?]) => {
        $crate::__tick! { @cols $st $doc [$($done)* { $wire $wire $ty [] $d }] [$($($rest)*)?] }
    };
    (@cols [$m:ident [$($ctx:tt)*] [$($head:tt)*] [$($req:tt)*] [$($tail:tt)*]] [$($doc:tt)*] [$($done:tt)*] []) => {
        $crate::__paste! { $m! { $($ctx)* $($head)* $($doc)* $($req)* columns [$($done)*] $($tail)* } }
    };
}

/// The decoder of a tick type: `decode::parse_<type>s`, or the hand-written
/// decoder of a type the table does not decode.
#[doc(hidden)]
#[macro_export]
macro_rules! __parse_fn {
    (CalendarDay) => { $crate::decode::parse_calendar_days_v3 };
    (OptionContract) => { $crate::decode::parse_option_contracts_v3 };
    ($item:ident) => { $crate::__paste!($crate::decode::[<parse_ $item:snake s>]) };
}
