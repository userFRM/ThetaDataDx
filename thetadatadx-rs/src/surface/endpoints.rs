//! The market-data endpoint tables.
//!
//! Every endpoint is written once, in `__endpoints!`. The engine expands the
//! tables into the `MarketDataClient` methods and builders, the endpoint
//! registry, the dispatch behind it and the shard fan-out allowlist; the
//! Python and TypeScript bindings expand them into their endpoint methods and
//! builders, and the C ABI into its entry points and `__request_options!` into
//! its options struct. `generate_sdk_surfaces` expands them too, to render the
//! Python type stub, the C and C++ headers, the live validators and the
//! documentation.
//!
//! A consumer passes the name of one of its own macros and receives every
//! entry, in table order, after the tokens it passed:
//!
//! ```ignore
//! thetadatadx::__endpoints! { my_consumer { leading tokens passed through } }
//! // expands to
//! my_consumer! { leading tokens passed through  <entry> <entry> ... }
//! ```
//!
//! # Entries
//!
//! ```ignore
//! #[bands]
//! chunked stock_history_ohlc {
//!     group: stock / history,
//!     rest: "/v3/stock/history/ohlc",
//!     returns: OhlcTick,
//!     params: [stock_history_intraday_interval],
//!     doc: ["Fetch intraday OHLC bars for a stock on a single date."],
//! }
//! ```
//!
//! - The shape is `list` (one string column: `returns: StringList("date")`),
//!   `buffered` (ticks in one response) or `chunked` (ticks that can also be
//!   drained chunk by chunk). The raw server streams in
//!   `__stream_endpoints!` are `stream` entries that name their RPC with
//!   `#[wire(rpc)]`.
//! - The name is the method and registry name. The gRPC stub, request and
//!   query types derive from it (`get_<name>`, `<Name>Request`,
//!   `<Name>RequestQuery`).
//! - `group` is the registry category and subcategory, `rest` the REST path.
//! - `params` lists vocabulary keys, or an inline definition in braces for a
//!   parameter only one endpoint has. Groups expand in place, and the result
//!   is the argument order on every surface.
//! - `doc` is the documentation, one string per line, kept verbatim. The
//!   first line is the registry description.
//! - `#[bands]` marks an endpoint whose large pulls may be split into
//!   concurrent bands.
//!
//! The registry lists the endpoints in table order.
//!
//! # Parameters
//!
//! `__param!` defines each key once, so a description is written once
//! however many endpoints share the parameter. A definition reads
//! `name(arg) req|opt Type [default] "description"`: `name` is the wire and
//! registry name, `(arg)` the argument name where it differs, `req` a
//! required argument and `opt` a builder setter. The type selects the
//! validation and the wire conversion; `Time` is a time of day, which the
//! registry reports as a string. A default is documented and sent when the
//! caller sets nothing. `__resolve!` expands the keys and hands the consumer
//! one normalized record per parameter.

/// Resolves an endpoint's parameter keys, then calls back.
///
/// `__resolve! { [keys..] => consumer { ctx.. } }` expands to
///
/// ```ignore
/// consumer! { ctx.. params {
///     all [records..] req [records..] opt [records..]
///     defaults [{ name value quote }..] dates [arg..]
/// } }
/// ```
///
/// Each record is `{ name arg req|opt Type [default?] "description" }`:
/// `name` is the wire and registry name, `arg` the binding argument name.
/// `all` keeps declaration order, `req` and `opt` split it by binding,
/// `defaults` lists the documented defaults (`quote` is the quote the
/// documentation renders the value in), and `dates` the required date
/// arguments the builder validates before sending.
///
/// A key the vocabulary does not define fails the build with an error that
/// names it, and names the endpoint too when the keys are followed by
/// `for <endpoint>`.
#[doc(hidden)]
#[macro_export]
macro_rules! __resolve {
    ([$($key:tt)*] $(for $name:ident)? => $m:ident { $($ctx:tt)* }) => {
        $crate::__resolve! { @next [$($key)*] {} {} {} {} {} $m { $($ctx)* } [$($name)?] }
    };
    (@next [] {$($all:tt)*} {$($req:tt)*} {$($opt:tt)*} {$($def:tt)*} {$($dates:tt)*} $m:ident { $($ctx:tt)* } $name:tt) => {
        $m! { $($ctx)* params {
            all [$($all)*] req [$($req)*] opt [$($opt)*] defaults [$($def)*] dates [$($dates)*]
        } }
    };
    // A definition written inline in the entry, for a parameter only it has.
    (@next [{ $($def:tt)* } $($rest:tt)*] $($s:tt)*) => {
        $crate::__resolve! { @def { $($def)* } [$($rest)*] $($s)* }
    };
    (@next [$key:ident $($rest:tt)*] $($s:tt)*) => {
        $crate::__param! { $key [$($rest)*] $($s)* }
    };
    // A group or template: its keys resolve in place, in order.
    (@keys [$($key:tt)*] [$($rest:tt)*] $($s:tt)*) => {
        $crate::__resolve! { @next [$($key)* $($rest)*] $($s)* }
    };
    (@def { $p:ident($a:ident) $($d:tt)* } $($s:tt)*) => {
        $crate::__resolve! { @push { $p $a $($d)* } $($s)* }
    };
    (@def { $p:ident $($d:tt)* } $($s:tt)*) => {
        $crate::__resolve! { @push { $p $p $($d)* } $($s)* }
    };
    (@push { $p:ident $a:ident req Date $($d:tt)* } $k:tt {$($all:tt)*} {$($req:tt)*} $opt:tt $def:tt {$($dates:tt)*} $($s:tt)*) => {
        $crate::__resolve! { @next $k {$($all)* {$p $a req Date $($d)*}} {$($req)* {$p $a req Date $($d)*}} $opt $def {$($dates)* $a} $($s)* }
    };
    (@push { $p:ident $a:ident req $($d:tt)* } $k:tt {$($all:tt)*} {$($req:tt)*} $($s:tt)*) => {
        $crate::__resolve! { @next $k {$($all)* {$p $a req $($d)*}} {$($req)* {$p $a req $($d)*}} $($s)* }
    };
    (@push { $p:ident $a:ident opt $t:ident [] $doc:tt } $k:tt {$($all:tt)*} $req:tt {$($opt:tt)*} $($s:tt)*) => {
        $crate::__resolve! { @next $k {$($all)* {$p $a opt $t [] $doc}} $req {$($opt)* {$p $a opt $t [] $doc}} $($s)* }
    };
    // Booleans and numbers are documented bare, every other default quoted.
    (@push { $p:ident $a:ident opt Bool $($d:tt)* } $($s:tt)*) => { $crate::__resolve! { @default "" { $p $a opt Bool $($d)* } $($s)* } };
    (@push { $p:ident $a:ident opt Int $($d:tt)* } $($s:tt)*) => { $crate::__resolve! { @default "" { $p $a opt Int $($d)* } $($s)* } };
    (@push { $p:ident $a:ident opt Float $($d:tt)* } $($s:tt)*) => { $crate::__resolve! { @default "" { $p $a opt Float $($d)* } $($s)* } };
    (@push { $p:ident $a:ident opt $($d:tt)* } $($s:tt)*) => { $crate::__resolve! { @default "\"" { $p $a opt $($d)* } $($s)* } };
    (@default $q:tt { $p:ident $a:ident opt $t:ident [$v:tt] $doc:tt } $k:tt {$($all:tt)*} $req:tt {$($opt:tt)*} {$($def:tt)*} $($s:tt)*) => {
        $crate::__resolve! { @next $k {$($all)* {$p $a opt $t [$v] $doc}} $req {$($opt)* {$p $a opt $t [$v] $doc}} {$($def)* {$p $v $q}} $($s)* }
    };
}

/// The shared parameter vocabulary: one definition or group per key.
///
/// `@def { name req|opt Type [default] "description" }` defines a parameter;
/// `name(arg)` gives it a binding argument name that differs from the wire
/// name. `@keys [..]` is a group or template of other keys.
#[doc(hidden)]
#[macro_export]
macro_rules! __param {
    (request_type $($s:tt)*) => { $crate::__resolve! { @def {
        request_type req RequestType []
        "Request type. Accepted values: `trade`, `quote`, `eod`, `ohlc`."
    } $($s)* } };
    (stock_symbol $($s:tt)*) => { $crate::__resolve! { @def {
        symbol req Symbol []
        "Ticker symbol (e.g. AAPL)"
    } $($s)* } };
    (multi_symbol $($s:tt)*) => { $crate::__resolve! { @def {
        symbol(symbols) req Symbols []
        "Comma-separated ticker symbols (e.g. AAPL,MSFT)"
    } $($s)* } };
    (contract_symbol_filter $($s:tt)*) => { $crate::__resolve! { @def {
        symbol opt Symbol []
        "Ticker symbol to filter by (e.g. AAPL), or a comma-separated list (AAPL,SPY) for the contracts of each. Omit to list every contract for the date."
    } $($s)* } };
    (expiration_param $($s:tt)*) => { $crate::__resolve! { @def {
        expiration req Expiration []
        "Expiration date YYYYMMDD"
    } $($s)* } };
    (strike_param $($s:tt)*) => { $crate::__resolve! { @def {
        strike opt Strike ["*"]
        "Strike price in dollars as a string (e.g. 500 or 17.5). Use `*` for wildcard selection."
    } $($s)* } };
    (right_param $($s:tt)*) => { $crate::__resolve! { @def {
        right opt Right ["both"]
        "Option side. Use `both` or `*` (alias) for calls and puts. Accepted values: `call`, `put`, `both`, `*`."
    } $($s)* } };
    (contract_identity $($s:tt)*) => { $crate::__resolve! { @keys [stock_symbol expiration_param strike_param right_param] $($s)* } };
    (required_date $($s:tt)*) => { $crate::__resolve! { @def {
        date req Date []
        "Date YYYYMMDD"
    } $($s)* } };
    (optional_date $($s:tt)*) => { $crate::__resolve! { @def {
        date opt Date []
        "Single date YYYYMMDD. Supply this for a single-day pull, or supply `start_date`/`end_date` for a range. When present, `date` takes precedence over the range."
    } $($s)* } };
    (interval $($s:tt)*) => { $crate::__resolve! { @def {
        interval opt Interval ["1s"]
        "Interval preset. Defaults to `1s` when omitted, matching the upstream ThetaData Python library. Accepted values: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`."
    } $($s)* } };
    (start_time_filter $($s:tt)*) => { $crate::__resolve! { @def {
        start_time opt Time ["09:30:00"]
        "Start time filter"
    } $($s)* } };
    (end_time_filter $($s:tt)*) => { $crate::__resolve! { @def {
        end_time opt Time ["16:00:00"]
        "End time filter"
    } $($s)* } };
    (history_time_filters $($s:tt)*) => { $crate::__resolve! { @keys [start_time_filter end_time_filter] $($s)* } };
    (stock_venue_filter $($s:tt)*) => { $crate::__resolve! { @def {
        venue opt Venue ["nqb"]
        "Venue/exchange filter. Accepted values: `nqb`, `utp_cta`."
    } $($s)* } };
    (min_time_filter $($s:tt)*) => { $crate::__resolve! { @def {
        min_time opt Time []
        "Minimum time filter"
    } $($s)* } };
    (start_date_required $($s:tt)*) => { $crate::__resolve! { @def {
        start_date req Date []
        "Start date YYYYMMDD"
    } $($s)* } };
    (end_date_required $($s:tt)*) => { $crate::__resolve! { @def {
        end_date req Date []
        "End date YYYYMMDD"
    } $($s)* } };
    (date_range_required $($s:tt)*) => { $crate::__resolve! { @keys [start_date_required end_date_required] $($s)* } };
    (start_date_optional $($s:tt)*) => { $crate::__resolve! { @def {
        start_date opt Date []
        "Start date YYYYMMDD"
    } $($s)* } };
    (end_date_optional $($s:tt)*) => { $crate::__resolve! { @def {
        end_date opt Date []
        "End date YYYYMMDD"
    } $($s)* } };
    (date_range_optional $($s:tt)*) => { $crate::__resolve! { @keys [start_date_optional end_date_optional] $($s)* } };
    (time_of_day $($s:tt)*) => { $crate::__resolve! { @def {
        time_of_day req Time []
        "ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM ET; legacy 34200000 is also accepted)"
    } $($s)* } };
    (exclusive_flag $($s:tt)*) => { $crate::__resolve! { @def {
        exclusive opt Bool [true]
        "When true, quotes whose timestamp equals the trade timestamp are excluded; only quotes strictly before the trade are paired. Defaults to true, matching the terminal, which injects exclusive=true when the value is omitted."
    } $($s)* } };
    (use_time_based_trade_quote_flag $($s:tt)*) => { $crate::__resolve! { @def {
        use_time_based_trade_quote opt Bool [true]
        "Affects requests for current-day data only. When true, each trade is matched with a quote by millisecond timestamp. Defaults to true, matching the terminal, which sends true when the value is omitted."
    } $($s)* } };
    (max_dte_filter $($s:tt)*) => { $crate::__resolve! { @def {
        max_dte opt Int []
        "Maximum days to expiration"
    } $($s)* } };
    (strike_range_filter $($s:tt)*) => { $crate::__resolve! { @def {
        strike_range opt Int []
        "Strike range filter"
    } $($s)* } };
    (annual_dividend_input $($s:tt)*) => { $crate::__resolve! { @def {
        annual_dividend opt Float []
        "Annualized continuous dividend yield used in the Greeks calculation, as a decimal (e.g. 0.0042 for a 0.42% yield). Must be non-negative; to pass a dollar amount per share, use `annual_dividend_amount`."
    } $($s)* } };
    (annual_dividend_amount_input $($s:tt)*) => { $crate::__resolve! { @def {
        annual_dividend_amount opt Float []
        "Annual dividend per share, in dollars, used in the Greeks calculation (e.g. 0.96 for $0.96 per year). Must be non-negative, and the server refuses it beside a non-zero `annual_dividend`."
    } $($s)* } };
    (rate_type_input $($s:tt)*) => { $crate::__resolve! { @def {
        rate_type opt RateType ["sofr"]
        "Risk-free-rate source used in the Greeks calculation. Accepted values: `sofr`, `treasury_m1`, `treasury_m3`, `treasury_m6`, `treasury_y1`, `treasury_y2`, `treasury_y3`, `treasury_y5`, `treasury_y7`, `treasury_y10`, `treasury_y20`, `treasury_y30`."
    } $($s)* } };
    (rate_value_input $($s:tt)*) => { $crate::__resolve! { @def {
        rate_value opt Float []
        "Interest rate as a percent (4.36 means 4.36%, matching the InterestRateTick.rate convention) used in the Greeks calculation. Applied when rate_type selects a manual rate."
    } $($s)* } };
    (option_greeks_rate_inputs $($s:tt)*) => { $crate::__resolve! { @keys [annual_dividend_input annual_dividend_amount_input rate_type_input rate_value_input] $($s)* } };
    (stock_price_input $($s:tt)*) => { $crate::__resolve! { @def {
        stock_price opt Float []
        "Underlying price in dollars used in the Greeks calculation, overriding the observed underlying when set."
    } $($s)* } };
    (version_input $($s:tt)*) => { $crate::__resolve! { @def {
        version opt Version ["latest"]
        "Greeks model version. Accepted values: `latest`, `1`."
    } $($s)* } };
    (option_snapshot_greeks_inputs $($s:tt)*) => { $crate::__resolve! { @keys [option_greeks_rate_inputs stock_price_input version_input] $($s)* } };
    (option_history_greeks_inputs $($s:tt)*) => { $crate::__resolve! { @keys [option_greeks_rate_inputs version_input] $($s)* } };
    (use_market_value_flag $($s:tt)*) => { $crate::__resolve! { @def {
        use_market_value opt Bool [false]
        "When true, calculate Greeks against the option market value (mid-price) instead of the NBBO bid/ask pair."
    } $($s)* } };
    (underlyer_use_nbbo_flag $($s:tt)*) => { $crate::__resolve! { @def {
        underlyer_use_nbbo opt Bool [false]
        "When true, use the NBBO-derived underlyer price as the Greeks input instead of the last trade."
    } $($s)* } };
    (option_use_nbbo_flag $($s:tt)*) => { $crate::__resolve! { @def {
        option_use_nbbo opt Bool [true]
        "Selects the option price used in the Greeks calculation: true uses the midpoint of the closing NBBO, false uses the last trade price. Defaults to true, matching the terminal, which sends true when the value is omitted."
    } $($s)* } };
    (binomial_steps_input $($s:tt)*) => { $crate::__resolve! { @def {
        binomial_steps opt Int [101]
        "Number of steps in the binomial tree used for the Greeks calculation. Accurate at higher values and cheaper at lower ones; the server's cost grows with the square of the value. An even value is rounded up to the next odd one, and an `expiration=*` request is capped at 101 steps."
    } $($s)* } };
    (perf_boost_intraday_flag $($s:tt)*) => { $crate::__resolve! { @def {
        perf_boost_intraday opt Bool [false]
        "When true, the underlying price for a current-day query is looked up with 1-second precision instead of millisecond precision, which answers faster. Defaults to false, matching the terminal, which sends false when the value is omitted."
    } $($s)* } };
    (current_day_only_flag $($s:tt)*) => { $crate::__resolve! { @def {
        current_day_only opt Bool [false]
        "When true, only snapshot data from the current trading day (New York date) is returned. Applied before `min_time`. Defaults to false, matching the terminal, which sends false when the value is omitted."
    } $($s)* } };
    (year_param $($s:tt)*) => { $crate::__resolve! { @def {
        year req Year []
        "4-digit year (e.g. 2024)"
    } $($s)* } };
    (stock_snapshot $($s:tt)*) => { $crate::__resolve! { @keys [multi_symbol stock_venue_filter min_time_filter current_day_only_flag] $($s)* } };
    (stock_history_intraday_interval $($s:tt)*) => { $crate::__resolve! { @keys [stock_symbol optional_date interval history_time_filters stock_venue_filter date_range_optional] $($s)* } };
    (stock_at_time $($s:tt)*) => { $crate::__resolve! { @keys [stock_symbol date_range_required time_of_day stock_venue_filter] $($s)* } };
    (stock_flat_file $($s:tt)*) => { $crate::__resolve! { @keys [required_date] $($s)* } };
    (option_snapshot_standard $($s:tt)*) => { $crate::__resolve! { @keys [contract_identity max_dte_filter strike_range_filter min_time_filter current_day_only_flag] $($s)* } };
    (option_snapshot_greeks $($s:tt)*) => { $crate::__resolve! { @keys [contract_identity option_snapshot_greeks_inputs max_dte_filter strike_range_filter min_time_filter use_market_value_flag current_day_only_flag] $($s)* } };
    (option_snapshot_binomial_greeks $($s:tt)*) => { $crate::__resolve! { @keys [contract_identity option_greeks_rate_inputs stock_price_input binomial_steps_input version_input max_dte_filter strike_range_filter min_time_filter use_market_value_flag current_day_only_flag] $($s)* } };
    (option_history_greeks $($s:tt)*) => { $crate::__resolve! { @keys [contract_identity optional_date interval history_time_filters option_history_greeks_inputs strike_range_filter date_range_optional] $($s)* } };
    (option_history_trade_greeks $($s:tt)*) => { $crate::__resolve! { @keys [contract_identity optional_date history_time_filters option_history_greeks_inputs max_dte_filter strike_range_filter date_range_optional perf_boost_intraday_flag] $($s)* } };
    (option_history_binomial_greeks $($s:tt)*) => { $crate::__resolve! { @keys [contract_identity optional_date interval history_time_filters option_greeks_rate_inputs binomial_steps_input version_input strike_range_filter date_range_optional] $($s)* } };
    (option_history_binomial_trade_greeks $($s:tt)*) => { $crate::__resolve! { @keys [contract_identity optional_date history_time_filters option_greeks_rate_inputs binomial_steps_input version_input max_dte_filter strike_range_filter date_range_optional perf_boost_intraday_flag] $($s)* } };
    (option_at_time $($s:tt)*) => { $crate::__resolve! { @keys [contract_identity date_range_required time_of_day max_dte_filter strike_range_filter] $($s)* } };
    (option_flat_file $($s:tt)*) => { $crate::__resolve! { @keys [required_date] $($s)* } };
    (index_snapshot $($s:tt)*) => { $crate::__resolve! { @keys [multi_symbol min_time_filter current_day_only_flag] $($s)* } };
    ($key:tt $keys:tt $all:tt $req:tt $opt:tt $def:tt $dates:tt $m:ident $ctx:tt [$($name:ident)?]) => {
        compile_error! { concat!("unknown endpoint parameter key `", stringify!($key), "`" $(, " in `", stringify!($name), "`")?) }
    };
}

/// The optional parameters of the C ABI options struct, in field order.
///
/// Append only: a field's position is ABI. `__request_options! { consumer {
/// ctx.. } }` resolves the keys and calls back like `__resolve!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __request_options {
    ($m:ident { $($ctx:tt)* }) => {
        $crate::__resolve! { [
            stock_venue_filter min_time_filter current_day_only_flag optional_date interval
            start_time_filter end_time_filter start_date_optional end_date_optional exclusive_flag
            use_time_based_trade_quote_flag strike_param right_param contract_symbol_filter
            max_dte_filter strike_range_filter annual_dividend_input annual_dividend_amount_input
            rate_type_input rate_value_input stock_price_input version_input use_market_value_flag
            binomial_steps_input underlyer_use_nbbo_flag option_use_nbbo_flag perf_boost_intraday_flag
        ] => $m { $($ctx)* } }
    };
}

/// Every market-data endpoint, in registry order.
#[doc(hidden)]
#[macro_export]
macro_rules! __endpoints {
    ($m:ident { $($ctx:tt)* }) => {
        $m! {
            $($ctx)*
            list stock_list_symbols {
                group: stock / list,
                rest: "/v3/stock/list/symbols",
                returns: StringList("symbol"),
                params: [],
                doc: [
                    "List all available stock ticker symbols.",
                    "",
                    "A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for stocks. This endpoint is updated overnight.",
                ],
            }
            list stock_list_dates {
                group: stock / list,
                rest: "/v3/stock/list/dates",
                returns: StringList("date"),
                params: [request_type stock_symbol],
                doc: [
                    "List available dates for a stock by request type (EOD, TRADE, QUOTE, etc.).",
                    "",
                    "Lists all dates of data that are available for a stock with a given request type and symbol. This endpoint is updated overnight.",
                ],
            }
            buffered stock_snapshot_ohlc {
                group: stock / snapshot,
                rest: "/v3/stock/snapshot/ohlc",
                returns: OhlcTick,
                params: [stock_snapshot],
                doc: [
                    "Get the latest OHLC snapshot for one or more stocks.",
                    "",
                    "Provides a real-time Open, High, Low, Close for the current day.",
                    "* Returns a real-time session OHLC from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.",
                    "* Returns a 15-minute delayed session OHLC from the UTP & CTA feeds if the account has the stocks value subscription.",
                    "* Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.",
                ],
            }
            buffered stock_snapshot_trade {
                group: stock / snapshot,
                rest: "/v3/stock/snapshot/trade",
                returns: TradeTick,
                params: [stock_snapshot],
                doc: [
                    "Get the latest trade snapshot for one or more stocks.",
                    "",
                    "Returns a real-time last trade from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.",
                    "",
                    "- Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.",
                ],
            }
            buffered stock_snapshot_quote {
                group: stock / snapshot,
                rest: "/v3/stock/snapshot/quote",
                returns: QuoteTick,
                params: [stock_snapshot],
                doc: [
                    "Get the latest NBBO quote snapshot for one or more stocks.",
                    "",
                    "* Returns a real-time last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.",
                    "* Returns a 15-minute delayed NBBO quote from the UTP & CTA feeds account has the stocks value subscription subscription.",
                    "- Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.",
                ],
            }
            buffered stock_snapshot_market_value {
                group: stock / snapshot,
                rest: "/v3/stock/snapshot/market_value",
                returns: MarketValueTick,
                params: [stock_snapshot],
                doc: [
                    "Get the latest market value snapshot for one or more stocks.",
                    "",
                    "* Returns a real-time market value derived from the last BBO quote from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.",
                    "* Returns a 15-minute delayed market value derived from an NBBO quote from the UTP & CTA feeds if the account has the stocks value subscription subscription.",
                    "- Theta Data resets its snapshot cache at midnight ET every day. This endpoint may not work on a weekend where there were no eligible messages sent over exchange feeds. We recommend using historic requests during the weekend.",
                ],
            }
            chunked stock_history_eod {
                group: stock / history,
                rest: "/v3/stock/history/eod",
                returns: EodTick,
                params: [stock_symbol date_range_required],
                doc: [
                    "Fetch end-of-day stock data for a date range. Returns OHLCV + bid/ask per trading day.",
                    "",
                    "Since the equity SIPs only generate a partial EOD report, Theta Data generates a national EOD report at 17:15 ET each day. ``created`` represents the datetime the report was generated and ``last_trade`` represents the datetime of the last trade. The quote in the response represents the last NBBO reported by CTA or UTP at the time of report generation. You can read more about EOD & OHLC data here. Theta Data plans to avail SIP EOD reports in the near future.",
                ],
            }
            #[bands]
            chunked stock_history_ohlc {
                group: stock / history,
                rest: "/v3/stock/history/ohlc",
                returns: OhlcTick,
                params: [stock_history_intraday_interval],
                doc: [
                    "Fetch intraday OHLC bars for a stock on a single date.",
                    "",
                    "- Aggregated OHLC bars that use SIP rules for each bar. Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar time`` <= ``trade time`` < ``bar timestamp + ivl``, where ivl is the specified interval size in milliseconds. ",
                    "- Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked stock_history_trade {
                group: stock / history,
                rest: "/v3/stock/history/trade",
                returns: TradeTick,
                params: [stock_symbol optional_date history_time_filters stock_venue_filter date_range_optional],
                doc: [
                    "Fetch all trades for a stock on a given date.",
                    "",
                    "Returns every trade reported by UTP & CTA. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked stock_history_quote {
                group: stock / history,
                rest: "/v3/stock/history/quote",
                returns: QuoteTick,
                params: [stock_history_intraday_interval],
                doc: [
                    "Fetch NBBO quotes for a stock on a given date at a given interval.",
                    "",
                    "- Returns every NBBO quote reported by UTP and CTA. ",
                    "- If the ``interval`` parameter is specified, the quote for each interval represents the last quote prior to the interval's timestamp. ",
                    "- Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked stock_history_trade_quote {
                group: stock / history,
                rest: "/v3/stock/history/trade_quote",
                returns: TradeQuoteTick,
                params: [stock_symbol optional_date history_time_filters exclusive_flag stock_venue_filter date_range_optional use_time_based_trade_quote_flag],
                doc: [
                    "Fetch combined trade + quote ticks for a stock on a given date.",
                    "",
                    "Returns every trade reported by UTP & CTA paired with the last BBO quote reported by UTP or CTA at the time of trade. A quote is matched with a trade if its timestamp ``<=`` the trade timestamp. If you prefer to match quotes with timestamps that are ``<`` the trade timestamp, specify the ``exclusive`` parameter to ``true``. Set the ``venue`` parameter to ``nqb`` to access current-day real-time historic data from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked stock_at_time_trade {
                group: stock / at_time,
                rest: "/v3/stock/at_time/trade",
                returns: TradeTick,
                params: [stock_at_time],
                doc: [
                    "Fetch the trade at a specific time of day across a date range.",
                    "",
                    "#### Real-time request:",
                    "- Returns a real-time session from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.",
                    "- Returns a 15-minute delayed session from the UTP & CTA feeds account has the stocks value subscription subscription.",
                    "",
                    "#### Historical request:",
                    "Returns the last trade reported by UTP & CTA feeds at a specified millisecond of the day.",
                    "Trade condition mappings can be found here.",
                ],
            }
            #[bands]
            chunked stock_at_time_quote {
                group: stock / at_time,
                rest: "/v3/stock/at_time/quote",
                returns: QuoteTick,
                params: [stock_at_time],
                doc: [
                    "Fetch the quote at a specific time of day across a date range.",
                    "",
                    "#### Real-time request:",
                    "  - Subscription tier standard or higher will default to NQB.",
                    "  - Real-time last BBO quote at-time_of_day-time from the Nasdaq Basic feed if the account has a stocks standard or pro subscription.",
                    "  - 15-minute delayed NBBO quote at-time_of_day-time from the UTP & CTA feeds account has the stocks value subscription subscription.",
                    "",
                    "#### Historical request:",
                    "  Returns the last NBBO quote reported by UTP & CTA feeds at a specified millisecond of the day.",
                ],
            }
            chunked stock_flat_file_eod {
                group: stock / flat_file,
                rest: "/v3/stock/flat_file/eod",
                returns: EodTick,
                params: [stock_flat_file],
                doc: [
                    "Fetch the end-of-day report of every stock for one date. The vendor keeps the most recent 7 calendar days.",
                ],
            }
            chunked stock_flat_file_trade_quote {
                group: stock / flat_file,
                rest: "/v3/stock/flat_file/trade_quote",
                returns: TradeQuoteTick,
                params: [stock_flat_file],
                doc: [
                    "Fetch every stock trade of one date, each paired with the NBBO quote at the time of the trade. The vendor keeps the most recent 7 calendar days.",
                ],
            }
            chunked stock_flat_file_quote {
                group: stock / flat_file,
                rest: "/v3/stock/flat_file/quote",
                returns: IntervalQuoteTick,
                params: [stock_flat_file],
                doc: [
                    "Fetch every stock's 1-minute NBBO quote intervals for one date: one row per minute in which a new quote was reported. The vendor keeps the most recent 7 calendar days.",
                ],
            }
            list option_list_symbols {
                group: option / list,
                rest: "/v3/option/list/symbols",
                returns: StringList("symbol"),
                params: [],
                doc: [
                    "List all available option underlying symbols.",
                    "",
                    "A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded symbols for options. This endpoint is updated overnight.",
                ],
            }
            list option_list_dates {
                group: option / list,
                rest: "/v3/option/list/dates",
                returns: StringList("date"),
                params: [request_type contract_identity],
                doc: [
                    "List available dates for an option contract by request type.",
                    "",
                    "Lists all dates of data that are available for an option with a given symbol, request type, and expiration.",
                    "This endpoint is updated overnight.",
                ],
            }
            list option_list_expirations {
                group: option / list,
                rest: "/v3/option/list/expirations",
                returns: StringList("expiration"),
                params: [stock_symbol],
                doc: [
                    "List available expiration dates for an option underlying.",
                    "",
                    "Lists all dates of expirations that are available for an option with a given symbol.",
                    "This endpoint is updated overnight.",
                ],
            }
            list option_list_strikes {
                group: option / list,
                rest: "/v3/option/list/strikes",
                returns: StringList("strike"),
                params: [stock_symbol expiration_param],
                doc: [
                    "List available strike prices for an option at a given expiration.",
                    "",
                    "Lists all strikes that are available for an option with a given symbol and expiration date.",
                    "This endpoint is updated overnight.",
                ],
            }
            chunked option_list_contracts {
                group: option / list,
                rest: "/v3/option/list/contracts",
                returns: OptionContract,
                params: [request_type contract_symbol_filter required_date max_dte_filter],
                doc: [
                    "List all option contracts traded or quoted on a given date, optionally filtered to a symbol.",
                    "",
                    "Lists all contracts that were traded or quoted on a particular date.",
                    "",
                    "If the ``symbol`` parameter is specified, the returned contracts will be filtered to match the symbol.",
                    "When ``symbol`` is omitted the full universe of contracts for that date is returned.",
                    "This endpoint is updated real-time.",
                ],
            }
            buffered option_snapshot_ohlc {
                group: option / snapshot,
                rest: "/v3/option/snapshot/ohlc",
                returns: OhlcTick,
                params: [option_snapshot_standard],
                doc: [
                    "Get the latest OHLC snapshot for an option contract.",
                    "",
                    "- Retrieve a real-time last ohlc of an option contract for the trading day.",
                ],
            }
            buffered option_snapshot_trade {
                group: option / snapshot,
                rest: "/v3/option/snapshot/trade",
                returns: TradeTick,
                params: [contract_identity strike_range_filter min_time_filter current_day_only_flag],
                doc: [
                    "Get the latest trade snapshot for an option contract.",
                    "",
                    "- Retrieve the real-time last trade of an option contract.",
                    "- This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            buffered option_snapshot_quote {
                group: option / snapshot,
                rest: "/v3/option/snapshot/quote",
                returns: QuoteTick,
                params: [option_snapshot_standard],
                doc: [
                    "Get the latest NBBO quote snapshot for an option contract.",
                    "",
                    "- Retrieve a real-time last NBBO quote of an option contract.",
                    "- This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            buffered option_snapshot_open_interest {
                group: option / snapshot,
                rest: "/v3/option/snapshot/open_interest",
                returns: OpenInterestTick,
                params: [option_snapshot_standard],
                doc: [
                    "Get the latest open interest snapshot for an option contract.",
                    "",
                    "- Retrieve the last open interest message of an option contract.",
                    "- Open interest is reported around 06:30 ET every morning by OPRA and reflects the open interest at the end of the previous trading day.",
                    "- This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            buffered option_snapshot_market_value {
                group: option / snapshot,
                rest: "/v3/option/snapshot/market_value",
                returns: MarketValueTick,
                params: [option_snapshot_standard],
                doc: [
                    "Get the latest market value snapshot for an option contract.",
                    "",
                    "* Returns a real-time market value derived from the last NBBO quote of an option contract.",
                ],
            }
            buffered option_snapshot_greeks_implied_volatility {
                group: option / snapshot_greeks,
                rest: "/v3/option/snapshot/greeks/implied_volatility",
                returns: IvTick,
                params: [option_snapshot_greeks],
                doc: [
                    "Get implied volatility snapshot for an option contract (from ThetaData server).",
                    "",
                    "Returns implied volatilies calculated using the national best bid, mid, and ask price",
                    "of the option respectively. The underlying price represents whatever the last underlying price was at the",
                    "``underlying_timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                ],
            }
            buffered option_snapshot_greeks_all {
                group: option / snapshot_greeks,
                rest: "/v3/option/snapshot/greeks/all",
                returns: GreeksAllTick,
                params: [option_snapshot_greeks],
                doc: [
                    "Get all Greeks snapshot for an option contract (from ThetaData server).",
                    "",
                    "- Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.",
                    "> This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            buffered option_snapshot_binomial_greeks_all {
                group: option / snapshot_greeks,
                rest: "/v3/option/snapshot/binomial_greeks/all",
                returns: GreeksAllTick,
                params: [option_snapshot_binomial_greeks],
                doc: [
                    "Get all binomial Greeks snapshot for an option contract (from ThetaData server).",
                    "",
                    "- Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "> This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            buffered option_snapshot_greeks_first_order {
                group: option / snapshot_greeks,
                rest: "/v3/option/snapshot/greeks/first_order",
                returns: GreeksFirstOrderTick,
                params: [option_snapshot_greeks],
                doc: [
                    "Get first-order Greeks snapshot (delta, theta, rho) for an option contract.",
                    "",
                    "- Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.",
                    "> This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            buffered option_snapshot_binomial_greeks_first_order {
                group: option / snapshot_greeks,
                rest: "/v3/option/snapshot/binomial_greeks/first_order",
                returns: GreeksFirstOrderTick,
                params: [option_snapshot_binomial_greeks],
                doc: [
                    "Get first-order binomial Greeks snapshot (delta, theta, rho) for an option contract.",
                    "",
                    "- Retrieve a real-time last greeks calculation for all option contracts that lie on a provided expiration.",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "> This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            buffered option_snapshot_greeks_second_order {
                group: option / snapshot_greeks,
                rest: "/v3/option/snapshot/greeks/second_order",
                returns: GreeksSecondOrderTick,
                params: [option_snapshot_greeks],
                doc: [
                    "Get second-order Greeks snapshot (gamma, vanna, charm) for an option contract.",
                    "",
                    "- Retrieve a real-time last second order greeks calculation for all option contracts that lie on a provided expiration.",
                    "> This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            buffered option_snapshot_binomial_greeks_second_order {
                group: option / snapshot_greeks,
                rest: "/v3/option/snapshot/binomial_greeks/second_order",
                returns: GreeksSecondOrderTick,
                params: [option_snapshot_binomial_greeks],
                doc: [
                    "Get second-order binomial Greeks snapshot (gamma, vanna, charm) for an option contract.",
                    "",
                    "- Retrieve a real-time last second order binomial greeks calculation for all option contracts that lie on a provided expiration.",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "> This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            buffered option_snapshot_greeks_third_order {
                group: option / snapshot_greeks,
                rest: "/v3/option/snapshot/greeks/third_order",
                returns: GreeksThirdOrderTick,
                params: [option_snapshot_greeks],
                doc: [
                    "Get third-order Greeks snapshot (speed, color, ultima) for an option contract.",
                    "",
                    "- Retrieve a real-time last third order greeks calculation for all option contracts that lie on a provided expiration.",
                    "> This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            buffered option_snapshot_binomial_greeks_third_order {
                group: option / snapshot_greeks,
                rest: "/v3/option/snapshot/binomial_greeks/third_order",
                returns: GreeksThirdOrderTick,
                params: [option_snapshot_binomial_greeks],
                doc: [
                    "Get third-order binomial Greeks snapshot (speed, color, ultima) for an option contract.",
                    "",
                    "- Retrieve a real-time last third order binomial greeks calculation for all option contracts that lie on a provided expiration.",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "> This endpoint will return no data if the market was closed for the day. Theta Data resets the snapshot cache at midnight ET every night.",
                ],
            }
            chunked option_history_eod {
                group: option / history,
                rest: "/v3/option/history/eod",
                returns: EodTick,
                params: [contract_identity date_range_required max_dte_filter strike_range_filter],
                doc: [
                    "Fetch end-of-day option data for a contract over a date range.",
                    "",
                    "- Since OPRA does not provide a national EOD report for options, Theta Data generates a national EOD report at 17:15 ET each day.",
                    "- ``created`` represents the datetime the report was generated and ``last_trade`` represents the datetime of the last trade. ",
                    "- The quote in the response represents the last NBBO reported by OPRA at the time of report generation. ",
                    "- You can read more about EOD & OHLC data here.",
                ],
            }
            #[bands]
            chunked option_history_ohlc {
                group: option / history,
                rest: "/v3/option/history/ohlc",
                returns: OhlcTick,
                params: [contract_identity optional_date interval history_time_filters strike_range_filter date_range_optional],
                doc: [
                    "Fetch intraday OHLC bars for an option contract.",
                    "",
                    "- Aggregated OHLC bars that use SIP rules for each bar. ",
                    "- Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar timestamp`` <= ``trade time`` < ``bar timestamp + interval``.",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked option_history_trade {
                group: option / history,
                rest: "/v3/option/history/trade",
                returns: TradeTick,
                params: [contract_identity optional_date history_time_filters max_dte_filter strike_range_filter date_range_optional],
                doc: [
                    "Fetch all trades for an option contract on a given date.",
                    "",
                    "- Returns every trade reported by OPRA. ",
                    "- Trade condition mappings can be found here.",
                    "- Extended trade conditions are not reported by OPRA for options, so they can be ignored.",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_history_quote {
                group: option / history,
                rest: "/v3/option/history/quote",
                returns: QuoteTick,
                params: [contract_identity optional_date interval history_time_filters max_dte_filter strike_range_filter date_range_optional],
                doc: [
                    "Fetch NBBO quotes for an option contract on a given date.",
                    "",
                    "- Returns every NBBO quote reported by OPRA. ",
                    "- If the ``interval`` parameter is specified, the quote for each interval represents the last quote at the interval's timestamp.",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_history_trade_quote {
                group: option / history,
                rest: "/v3/option/history/trade_quote",
                returns: TradeQuoteTick,
                params: [contract_identity optional_date history_time_filters exclusive_flag max_dte_filter strike_range_filter date_range_optional use_time_based_trade_quote_flag],
                doc: [
                    "Fetch combined trade + quote ticks for an option contract.",
                    "",
                    "- Returns every trade reported by OPRA paired with the last NBBO quote reported by OPRA at the time of trade.",
                    "- A quote is matched with a trade if its timestamp ``<=`` the trade timestamp. ",
                    "- To match trades with quotes timestamps that are ``<`` the trade timestamp, specify the ``exclusive``parameter to ``true``. After thorough testing, we have determined that using ``exclusive=true`` might yield better results for various applications.",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            chunked option_history_open_interest {
                group: option / history,
                rest: "/v3/option/history/open_interest",
                returns: OpenInterestTick,
                params: [contract_identity optional_date max_dte_filter strike_range_filter date_range_optional],
                doc: [
                    "Fetch open interest history for an option contract.",
                    "",
                    "- Open Interest is normally reported once per day by OPRA at approximately 06:30 ET.",
                    "- A new open interest message might not be sent by OPRA if there is no open interest for the option contract.",
                    "- The reported open interest represents the open interest at the end of the previous trading day.",
                ],
            }
            chunked option_history_greeks_eod {
                group: option / history_greeks,
                rest: "/v3/option/history/greeks/eod",
                returns: GreeksEodTick,
                params: [contract_identity date_range_required option_history_greeks_inputs underlyer_use_nbbo_flag max_dte_filter strike_range_filter option_use_nbbo_flag],
                doc: [
                    "Fetch end-of-day Greeks history for an option contract.",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration.",
                    "- Uses Theta Data's EOD reports that get generated at 17:15 ET each day. The closing option price and closing underlying price are used for the greeks calculation.",
                    "- **Any ``expiration=*`` request must be made day by day.**",
                ],
            }
            chunked option_history_binomial_greeks_eod {
                group: option / history_greeks,
                rest: "/v3/option/history/binomial_greeks/eod",
                returns: GreeksEodTick,
                params: [contract_identity date_range_required option_greeks_rate_inputs binomial_steps_input version_input underlyer_use_nbbo_flag max_dte_filter strike_range_filter option_use_nbbo_flag],
                doc: [
                    "Fetch end-of-day binomial Greeks history for an option contract.",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration.",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "- Uses Theta Data's EOD reports that get generated at 17:15 ET each day. The closing option price and closing underlying price are used for the greeks calculation.",
                    "- **Any ``expiration=*`` request must be made day by day.**",
                ],
            }
            #[bands]
            chunked option_history_greeks_all {
                group: option / history_greeks,
                rest: "/v3/option/history/greeks/all",
                returns: GreeksAllTick,
                params: [option_history_greeks],
                doc: [
                    "Fetch all Greeks history for an option contract (intraday, sampled by interval).",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration. ",
                    "- Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. ",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked option_history_binomial_greeks_all {
                group: option / history_greeks,
                rest: "/v3/option/history/binomial_greeks/all",
                returns: GreeksAllTick,
                params: [option_history_binomial_greeks],
                doc: [
                    "Fetch all binomial Greeks history for an option contract (intraday, sampled by interval).",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration. ",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "- Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. ",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked option_history_trade_greeks_all {
                group: option / history_trade_greeks,
                rest: "/v3/option/history/trade_greeks/all",
                returns: TradeGreeksAllTick,
                params: [option_history_trade_greeks],
                doc: [
                    "Fetch all Greeks on each trade for an option contract.",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration. ",
                    "- Calculates greeks for every trade reported by OPRA.",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_history_binomial_trade_greeks_all {
                group: option / history_trade_greeks,
                rest: "/v3/option/history/binomial_trade_greeks/all",
                returns: TradeGreeksAllTick,
                params: [option_history_binomial_trade_greeks],
                doc: [
                    "Fetch all binomial Greeks on each trade for an option contract.",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration.",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "- Calculates greeks for every trade reported by OPRA.",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_history_greeks_first_order {
                group: option / history_greeks,
                rest: "/v3/option/history/greeks/first_order",
                returns: GreeksFirstOrderTick,
                params: [option_history_greeks],
                doc: [
                    "Fetch first-order Greeks history (intraday, sampled by interval).",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration. ",
                    "- Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. ",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked option_history_binomial_greeks_first_order {
                group: option / history_greeks,
                rest: "/v3/option/history/binomial_greeks/first_order",
                returns: GreeksFirstOrderTick,
                params: [option_history_binomial_greeks],
                doc: [
                    "Fetch first-order binomial Greeks history (intraday, sampled by interval).",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration. ",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "- Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. ",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked option_history_trade_greeks_first_order {
                group: option / history_trade_greeks,
                rest: "/v3/option/history/trade_greeks/first_order",
                returns: TradeGreeksFirstOrderTick,
                params: [option_history_trade_greeks],
                doc: [
                    "Fetch first-order Greeks on each trade for an option contract.",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration.",
                    "- Calculates greeks for every trade reported by OPRA.",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_history_binomial_trade_greeks_first_order {
                group: option / history_trade_greeks,
                rest: "/v3/option/history/binomial_trade_greeks/first_order",
                returns: TradeGreeksFirstOrderTick,
                params: [option_history_binomial_trade_greeks],
                doc: [
                    "Fetch first-order binomial Greeks on each trade for an option contract.",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration.",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "- Calculates greeks for every trade reported by OPRA.",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_history_greeks_second_order {
                group: option / history_greeks,
                rest: "/v3/option/history/greeks/second_order",
                returns: GreeksSecondOrderTick,
                params: [option_history_greeks],
                doc: [
                    "Fetch second-order Greeks history (intraday, sampled by interval).",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration. ",
                    "- Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. ",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked option_history_binomial_greeks_second_order {
                group: option / history_greeks,
                rest: "/v3/option/history/binomial_greeks/second_order",
                returns: GreeksSecondOrderTick,
                params: [option_history_binomial_greeks],
                doc: [
                    "Fetch second-order binomial Greeks history (intraday, sampled by interval).",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration. ",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "- Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. ",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked option_history_trade_greeks_second_order {
                group: option / history_trade_greeks,
                rest: "/v3/option/history/trade_greeks/second_order",
                returns: TradeGreeksSecondOrderTick,
                params: [option_history_trade_greeks],
                doc: [
                    "Fetch second-order Greeks on each trade for an option contract.",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration.",
                    "- Calculates greeks for every trade reported by OPRA.",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_history_binomial_trade_greeks_second_order {
                group: option / history_trade_greeks,
                rest: "/v3/option/history/binomial_trade_greeks/second_order",
                returns: TradeGreeksSecondOrderTick,
                params: [option_history_binomial_trade_greeks],
                doc: [
                    "Fetch second-order binomial Greeks on each trade for an option contract.",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration.",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "- Calculates greeks for every trade reported by OPRA.",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_history_greeks_third_order {
                group: option / history_greeks,
                rest: "/v3/option/history/greeks/third_order",
                returns: GreeksThirdOrderTick,
                params: [option_history_greeks],
                doc: [
                    "Fetch third-order Greeks history (intraday, sampled by interval).",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration. ",
                    "- Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. ",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked option_history_binomial_greeks_third_order {
                group: option / history_greeks,
                rest: "/v3/option/history/binomial_greeks/third_order",
                returns: GreeksThirdOrderTick,
                params: [option_history_binomial_greeks],
                doc: [
                    "Fetch third-order binomial Greeks history (intraday, sampled by interval).",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration. ",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "- Calculated using the option and underlying midpoint price. If an interval size is specified (*highly recommended*), the option quote used in the calculation follows the same rules as the quote endpoint. ",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked option_history_trade_greeks_third_order {
                group: option / history_trade_greeks,
                rest: "/v3/option/history/trade_greeks/third_order",
                returns: TradeGreeksThirdOrderTick,
                params: [option_history_trade_greeks],
                doc: [
                    "Fetch third-order Greeks on each trade for an option contract.",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration.",
                    "- Calculates greeks for every trade reported by OPRA.",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_history_binomial_trade_greeks_third_order {
                group: option / history_trade_greeks,
                rest: "/v3/option/history/binomial_trade_greeks/third_order",
                returns: TradeGreeksThirdOrderTick,
                params: [option_history_binomial_trade_greeks],
                doc: [
                    "Fetch third-order binomial Greeks on each trade for an option contract.",
                    "",
                    "- Returns the data for all contracts that share the same provided symbol and expiration.",
                    "- Greeks are calculated with a Leisen-Reimer binomial tree that allows early exercise at every node, so the values are American-style.",
                    "- Calculates greeks for every trade reported by OPRA.",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_history_greeks_implied_volatility {
                group: option / history_greeks,
                rest: "/v3/option/history/greeks/implied_volatility",
                returns: IvTick,
                params: [option_history_greeks],
                doc: [
                    "Fetch implied volatility history (intraday, sampled by interval).",
                    "",
                    "- Returns implied volatilies calculated using the national best bid, mid, and ask price of the option respectively. ",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked option_history_trade_greeks_implied_volatility {
                group: option / history_trade_greeks,
                rest: "/v3/option/history/trade_greeks/implied_volatility",
                returns: TradeGreeksImpliedVolatilityTick,
                params: [option_history_trade_greeks],
                doc: [
                    "Fetch implied volatility on each trade for an option contract.",
                    "",
                    "- Returns implied volatilies calculated using the trade reported by OPRA. ",
                    "- The underlying price represents whatever the last underlying price was at the ``timestamp`` field. You can read more about how Theta Data calculates greeks [here](/articles/option-greeks).",
                    "- Multi-day requests are limited to 1 month of data, and must specify an expiration.",
                ],
            }
            #[bands]
            chunked option_at_time_trade {
                group: option / at_time,
                rest: "/v3/option/at_time/trade",
                returns: TradeTick,
                params: [option_at_time],
                doc: [
                    "Fetch the trade at a specific time of day across a date range for an option.",
                    "",
                    "- Returns the last trade reported by OPRA at a specified millisecond of the day.",
                    "- Trade condition mappings can be found here.",
                    "- Extended trade conditions are not reported by OPRA for options, so they can be ignored.",
                    "- The ``time_of_day``parameter represents the 00:00:00.000 ET that the trade should be provided for.",
                ],
            }
            #[bands]
            chunked option_at_time_quote {
                group: option / at_time,
                rest: "/v3/option/at_time/quote",
                returns: QuoteTick,
                params: [option_at_time],
                doc: [
                    "Fetch the quote at a specific time of day across a date range for an option.",
                    "",
                    "- Returns the last NBBO quote reported by OPRA at a specified millisecond of the day.",
                    "- The ``time_of_day``parameter represents the 00:00:00.000 ET that the quote should be provided for.",
                ],
            }
            chunked option_flat_file_eod {
                group: option / flat_file,
                rest: "/v3/option/flat_file/eod",
                returns: EodTick,
                params: [option_flat_file],
                doc: [
                    "Fetch the end-of-day report of every option contract for one date. The vendor keeps the most recent 7 calendar days.",
                ],
            }
            chunked option_flat_file_trade_quote {
                group: option / flat_file,
                rest: "/v3/option/flat_file/trade_quote",
                returns: TradeQuoteTick,
                params: [option_flat_file],
                doc: [
                    "Fetch every option trade of one date, each paired with the NBBO quote at the time of the trade. The vendor keeps the most recent 7 calendar days.",
                ],
            }
            chunked option_flat_file_open_interest {
                group: option / flat_file,
                rest: "/v3/option/flat_file/open_interest",
                returns: OpenInterestTick,
                params: [option_flat_file],
                doc: [
                    "Fetch the open interest of every option contract for one date. The vendor keeps the most recent 7 calendar days.",
                ],
            }
            chunked option_flat_file_quote {
                group: option / flat_file,
                rest: "/v3/option/flat_file/quote",
                returns: IntervalQuoteTick,
                params: [option_flat_file],
                doc: [
                    "Fetch every option contract's 1-minute NBBO quote intervals for one date: one row per minute in which a new quote was reported. The vendor keeps the most recent 7 calendar days.",
                ],
            }
            list index_list_symbols {
                group: index / list,
                rest: "/v3/index/list/symbols",
                returns: StringList("symbol"),
                params: [],
                doc: [
                    "List all available index symbols.",
                    "",
                    "A symbol can be defined as a unique identifier for a stock / underlying asset. Common terms also include: root, ticker, and underlying. This endpoint returns all traded index symbols. This endpoint is updated overnight.",
                ],
            }
            list index_list_dates {
                group: index / list,
                rest: "/v3/index/list/dates",
                returns: StringList("date"),
                params: [stock_symbol],
                doc: [
                    "List available dates for an index symbol.",
                    "",
                    "Lists all dates of data that are available for a index with a given request type and symbol. This endpoint is updated overnight.",
                ],
            }
            buffered index_snapshot_ohlc {
                group: index / snapshot,
                rest: "/v3/index/snapshot/ohlc",
                returns: OhlcTick,
                params: [index_snapshot],
                doc: [
                    "Get the latest OHLC snapshot for one or more indices.",
                    "",
                    "- Retrieves the real-time current day OHLC.",
                    "- Exchanges typically generate a price report every second for popular indices like SPX.",
                ],
            }
            buffered index_snapshot_price {
                group: index / snapshot,
                rest: "/v3/index/snapshot/price",
                returns: PriceTick,
                params: [index_snapshot],
                doc: [
                    "Get the latest price snapshot for one or more indices.",
                    "",
                    "- Retrieves a real-time last index price.",
                    "- Exchanges typically generate a price report every second for popular indices like SPX.",
                ],
            }
            buffered index_snapshot_market_value {
                group: index / snapshot,
                rest: "/v3/index/snapshot/market_value",
                returns: MarketValueTick,
                params: [index_snapshot],
                doc: [
                    "Get the latest market value snapshot for one or more indices.",
                    "",
                    "- Retrieves a real-time last index market value.",
                    "- Exchanges typically generate a price report every second for popular indices like SPX.",
                ],
            }
            chunked index_history_eod {
                group: index / history,
                rest: "/v3/index/history/eod",
                returns: EodTick,
                params: [stock_symbol date_range_required],
                doc: [
                    "Fetch end-of-day index data for a date range.",
                    "",
                    "- Since the indices feeds do not provide a national EOD report, Theta Data generates a national EOD report at 17:15 each day.",
                ],
            }
            #[bands]
            chunked index_history_ohlc {
                group: index / history,
                rest: "/v3/index/history/ohlc",
                returns: OhlcTick,
                params: [stock_symbol date_range_required interval history_time_filters],
                doc: [
                    "Fetch intraday OHLC bars for an index.",
                    "",
                    "- Aggregated OHLC bars that use SIP rules for each bar.",
                    "- Time timestamp of the bar represents the opening time of the bar. For a trade to be part of the bar:  ``bar timestamp`` <= ``trade time`` < ``bar timestamp + interval``.",
                    "- Exchanges typically generate a price report every second for popular indices like SPX.",
                ],
            }
            #[bands]
            chunked index_history_price {
                group: index / history,
                rest: "/v3/index/history/price",
                returns: PriceTick,
                params: [stock_symbol optional_date interval history_time_filters date_range_optional],
                doc: [
                    "Fetch intraday price history for an index.",
                    "",
                    "- Retrieves historical indices price reports. Exchanges typically generate a price report every second for popular indices like SPX.",
                    "- When the ``interval`` parameter is specified, the returned data represents the price at the exact time of each timestamp. If the timestamp in the response is 10:30:00, the price field represents the price at that exact time of the day.",
                    "- A price update from the exchange is omitted if the price remained the same from the previous update.",
                    "- Multi-day requests are limited to 1 month of data.",
                ],
            }
            #[bands]
            chunked index_at_time_price {
                group: index / at_time,
                rest: "/v3/index/at_time/price",
                returns: IndexPriceAtTimeTick,
                params: [stock_symbol date_range_required time_of_day],
                doc: [
                    "Fetch the index price at a specific time of day across a date range.",
                    "",
                    "- Retrieves historical indices price reports. Exchanges typically generate a price report every second for popular indices like SPX.",
                    "- The ``time_of_day`` parameter represents the 00:00:00.000 ET that the price should be provided for.",
                ],
            }
            chunked index_flat_file_eod {
                group: index / flat_file,
                rest: "/v3/index/flat_file/eod",
                returns: EodTick,
                params: [required_date],
                doc: [
                    "Fetch the end-of-day report of every index for one date. The vendor keeps the most recent 7 calendar days.",
                ],
            }
            buffered calendar_open_today {
                group: calendar / status,
                rest: "/v3/calendar/open_today",
                returns: CalendarDay,
                params: [],
                doc: [
                    "Check whether the market is open today.",
                    "",
                    "- Retrieves current day equity market schedule",
                    "- *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.",
                    "- **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.",
                ],
            }
            buffered calendar_on_date {
                group: calendar / query,
                rest: "/v3/calendar/on_date",
                returns: CalendarDay,
                params: [required_date],
                doc: [
                    "Get calendar information for a specific date.",
                    "",
                    "- Retrieves equity market schedule for a given date",
                    "- Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year",
                    "- *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.",
                    "- **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.",
                ],
            }
            buffered calendar_year {
                group: calendar / query,
                rest: "/v3/calendar/year",
                returns: CalendarDay,
                params: [year_param],
                doc: [
                    "Get equity market holidays and early-close days for a year (vendor `year_holidays` endpoint: only non-standard days, not every trading day).",
                    "",
                    "- Retrieves equity market holidays for a given year",
                    "- Note: Holiday data is available 01/01/2012 through the end of the calendar year that immediately follows the current year",
                    "- *On days when the market closes early at 1:00 PM ET; eligible options will trade until 1:15 PM.",
                    "- **Some NYSE exchanges will continue late trading until 5:00 PM ET on early close days.",
                ],
            }
            chunked interest_rate_history_eod {
                group: rate / history,
                rest: "/v3/rate/history/eod",
                returns: InterestRateTick,
                params: [stock_symbol date_range_required],
                doc: [
                    "Fetch end-of-day interest rate history.",
                    "",
                    "- Returns the interest rate reported. Depending on the rate, reports can occur in the morning or the afternoon.",
                    "- Valid `symbol` values per upstream `RateType` enum:",
                    "  `SOFR`, `TREASURY_M1`, `TREASURY_M3`, `TREASURY_M6`,",
                    "  `TREASURY_Y1`, `TREASURY_Y2`, `TREASURY_Y3`, `TREASURY_Y5`,",
                    "  `TREASURY_Y7`, `TREASURY_Y10`, `TREASURY_Y20`, `TREASURY_Y30`.",
                ],
            }
        }
    };
}

/// The raw server streams: Rust builders that drain one RPC chunk by chunk.
/// They are not in the registry, and dispatch collects them.
#[doc(hidden)]
#[macro_export]
macro_rules! __stream_endpoints {
    ($m:ident { $($ctx:tt)* }) => {
        $m! {
            $($ctx)*
            #[wire(stock_history_trade)]
            stream stock_history_trade_stream {
                group: stock / history,
                rest: "/v3/stock/history/trade",
                returns: TradeTick,
                params: [stock_symbol optional_date history_time_filters stock_venue_filter date_range_optional],
                doc: [
                    "Stream all trades for a stock on a given date, chunk-by-chunk.",
                ],
            }
            #[wire(stock_history_quote)]
            stream stock_history_quote_stream {
                group: stock / history,
                rest: "/v3/stock/history/quote",
                returns: QuoteTick,
                params: [stock_symbol optional_date interval history_time_filters stock_venue_filter date_range_optional],
                doc: [
                    "Stream NBBO quotes for a stock on a given date, chunk-by-chunk.",
                ],
            }
            #[wire(option_history_trade)]
            stream option_history_trade_stream {
                group: option / history,
                rest: "/v3/option/history/trade",
                returns: TradeTick,
                params: [contract_identity optional_date history_time_filters max_dte_filter strike_range_filter date_range_optional],
                doc: [
                    "Stream all trades for an option contract, chunk-by-chunk.",
                ],
            }
            #[wire(option_history_quote)]
            stream option_history_quote_stream {
                group: option / history,
                rest: "/v3/option/history/quote",
                returns: QuoteTick,
                params: [contract_identity optional_date interval history_time_filters max_dte_filter strike_range_filter date_range_optional],
                doc: [
                    "Stream NBBO quotes for an option contract, chunk-by-chunk.",
                ],
            }
        }
    };
}
