//! Historical endpoints — list, snapshot, history, at-time, calendar, and
//! interest-rate wrappers exposed as `#[no_mangle] extern "C" fn`.
//!
//! The bulk of this file is macro-expanded from the four `ffi_typed_*` and
//! `ffi_list_*` wrapper macros. The generated `endpoint_request_options.rs`
//! and `endpoint_with_options.rs` are `include!`'d at the bottom so the
//! options-aware endpoint variants resolve the helpers that live in this
//! module's scope.

use std::os::raw::c_char;
use std::ptr;

use crate::error::{cstr_to_str, set_error};
use crate::runtime;
use crate::types::{
    insert_bool_arg, insert_float_arg, insert_int_arg, insert_optional_str_arg, parse_symbol_array,
    TdxCalendarDayArray, TdxClient, TdxEodTickArray, TdxGreeksTickArray, TdxInterestRateTickArray,
    TdxIvTickArray, TdxMarketValueTickArray, TdxOhlcTickArray, TdxOpenInterestTickArray,
    TdxOptionContractArray, TdxPriceTickArray, TdxQuoteTickArray, TdxStringArray,
    TdxTradeQuoteTickArray, TdxTradeTickArray,
};

// `endpoint_request_options.rs` declares `TdxEndpointRequestOptions` and the
// private helper `apply_endpoint_request_options`. Included here (not in
// `types.rs`) because the helper references `insert_*_arg` via this module's
// `use`s and is only consumed by the generated endpoint wrappers below.
include!("endpoint_request_options.rs");

// ═══════════════════════════════════════════════════════════════════════
//  FFI endpoint macros — typed array returns (no JSON serialization)
// ═══════════════════════════════════════════════════════════════════════

/// FFI wrapper for list endpoints that return `Vec<String>` (no extra params beyond client).
macro_rules! ffi_list_endpoint_no_params {
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(client: *const TdxClient) -> TdxStringArray {
            ffi_boundary!(TdxStringArray { data: ptr::null(), len: 0 }, {
                let empty = TdxStringArray { data: ptr::null(), len: 0 };
                if client.is_null() {
                    set_error("client handle is null");
                    return empty;
                }
                let client = unsafe { &*client };
                match runtime().block_on(async { client.inner.$method().await }) {
                    Ok(items) => match TdxStringArray::from_vec(items) {
                        Ok(arr) => arr,
                        Err(e) => {
                            set_error(&format!("interior NUL in server string: {e}"));
                            empty
                        }
                    },
                    Err(e) => {
                        set_error(&e.to_string());
                        empty
                    }
                }
            })
        }
    };
}

/// FFI wrapper for list endpoints that take C string params and return `Vec<String>`.
macro_rules! ffi_list_endpoint {
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident ( $($param:ident),+ )
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(
            client: *const TdxClient,
            $($param: *const c_char),+
        ) -> TdxStringArray {
            ffi_boundary!(TdxStringArray { data: ptr::null(), len: 0 }, {
                let empty = TdxStringArray { data: ptr::null(), len: 0 };
                if client.is_null() {
                    set_error("client handle is null");
                    return empty;
                }
                let client = unsafe { &*client };
                $(
                    let $param = match unsafe { cstr_to_str($param) } {
                        Ok(Some(s)) => s,
                        Ok(None) => {
                            set_error(concat!(stringify!($param), " is null"));
                            return empty;
                        }
                        Err(e) => {
                            set_error(&format!(
                                "{} is not valid UTF-8: {e}",
                                stringify!($param)
                            ));
                            return empty;
                        }
                    };
                )+
                match runtime().block_on(async { client.inner.$method($($param),+).await }) {
                    Ok(items) => match TdxStringArray::from_vec(items) {
                        Ok(arr) => arr,
                        Err(e) => {
                            set_error(&format!("interior NUL in server string: {e}"));
                            empty
                        }
                    },
                    Err(e) => {
                        set_error(&e.to_string());
                        empty
                    }
                }
            })
        }
    };
}

/// FFI wrapper for snapshot endpoints that take a C string array of symbols and return typed tick arrays.
macro_rules! ffi_typed_snapshot_endpoint {
    // Variant with opts (appends)
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident, $array_type:ident,
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(
            client: *const TdxClient,
            symbols: *const *const c_char,
            symbols_len: usize,
        ) -> $array_type {
            ffi_boundary!($array_type { data: ptr::null(), len: 0 }, {
                let empty = $array_type { data: ptr::null(), len: 0 };
                if client.is_null() {
                    set_error("client handle is null");
                    return empty;
                }
                let client = unsafe { &*client };
                let syms = match unsafe { parse_symbol_array(symbols, symbols_len) } {
                    Some(s) => s,
                    None => return empty,
                };
                let refs: Vec<&str> = syms.iter().map(|s| s.as_str()).collect();
                match runtime().block_on(async { client.inner.$method(&refs).await }) {
                    Ok(ticks) => match $array_type::from_vec(ticks) {
                        Ok(arr) => arr,
                        Err(e) => {
                            set_error(&format!("interior NUL in server string: {e}"));
                            empty
                        }
                    },
                    Err(e) => {
                        set_error(&e.to_string());
                        empty
                    }
                }
            })
        }
    };
    // Original variant (no opts)
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident, $array_type:ident
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(
            client: *const TdxClient,
            symbols: *const *const c_char,
            symbols_len: usize,
        ) -> $array_type {
            ffi_boundary!($array_type { data: ptr::null(), len: 0 }, {
                let empty = $array_type { data: ptr::null(), len: 0 };
                if client.is_null() {
                    set_error("client handle is null");
                    return empty;
                }
                let client = unsafe { &*client };
                let syms = match unsafe { parse_symbol_array(symbols, symbols_len) } {
                    Some(s) => s,
                    None => return empty,
                };
                let refs: Vec<&str> = syms.iter().map(|s| s.as_str()).collect();
                match runtime().block_on(async { client.inner.$method(&refs).await }) {
                    Ok(ticks) => match $array_type::from_vec(ticks) {
                        Ok(arr) => arr,
                        Err(e) => {
                            set_error(&format!("interior NUL in server string: {e}"));
                            empty
                        }
                    },
                    Err(e) => {
                        set_error(&e.to_string());
                        empty
                    }
                }
            })
        }
    };
}

/// FFI wrapper for typed tick endpoints with C string params.
macro_rules! ffi_typed_endpoint {
    // Variant with params only
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident, $array_type:ident ( $($param:ident),+ )
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(
            client: *const TdxClient,
            $($param: *const c_char),+
        ) -> $array_type {
            ffi_boundary!($array_type { data: ptr::null(), len: 0 }, {
                let empty = $array_type { data: ptr::null(), len: 0 };
                if client.is_null() {
                    set_error("client handle is null");
                    return empty;
                }
                let client = unsafe { &*client };
                $(
                    let $param = match unsafe { cstr_to_str($param) } {
                        Ok(Some(s)) => s,
                        Ok(None) => {
                            set_error(concat!(stringify!($param), " is null"));
                            return empty;
                        }
                        Err(e) => {
                            set_error(&format!(
                                "{} is not valid UTF-8: {e}",
                                stringify!($param)
                            ));
                            return empty;
                        }
                    };
                )+
                match runtime().block_on(async { client.inner.$method($($param),+).await }) {
                    Ok(ticks) => match $array_type::from_vec(ticks) {
                        Ok(arr) => arr,
                        Err(e) => {
                            set_error(&format!("interior NUL in server string: {e}"));
                            empty
                        }
                    },
                    Err(e) => {
                        set_error(&e.to_string());
                        empty
                    }
                }
            })
        }
    };
}

/// FFI wrapper for typed endpoints with no params.
macro_rules! ffi_typed_endpoint_no_params {
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident, $array_type:ident
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(client: *const TdxClient) -> $array_type {
            ffi_boundary!($array_type { data: ptr::null(), len: 0 }, {
                let empty = $array_type { data: ptr::null(), len: 0 };
                if client.is_null() {
                    set_error("client handle is null");
                    return empty;
                }
                let client = unsafe { &*client };
                match runtime().block_on(async { client.inner.$method().await }) {
                    Ok(ticks) => match $array_type::from_vec(ticks) {
                        Ok(arr) => arr,
                        Err(e) => {
                            set_error(&format!("interior NUL in server string: {e}"));
                            empty
                        }
                    },
                    Err(e) => {
                        set_error(&e.to_string());
                        empty
                    }
                }
            })
        }
    };
}

include!("endpoint_with_options.rs");

// ═══════════════════════════════════════════════════════════════════════
//  Stock — List endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

// 1. stock_list_symbols
ffi_list_endpoint_no_params! {
    /// List all available stock symbols. Returns TdxStringArray.
    tdx_stock_list_symbols => stock_list_symbols
}

// 2. stock_list_dates
ffi_list_endpoint! {
    /// List available dates for a stock by request type. Returns TdxStringArray.
    tdx_stock_list_dates => stock_list_dates(request_type, symbol)
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — Snapshot endpoints (4)
// ═══════════════════════════════════════════════════════════════════════

// 3. stock_snapshot_ohlc
ffi_typed_snapshot_endpoint! {
    /// Get latest OHLC snapshot. Returns TdxOhlcTickArray.
    tdx_stock_snapshot_ohlc => stock_snapshot_ohlc, TdxOhlcTickArray
}

// 4. stock_snapshot_trade
ffi_typed_snapshot_endpoint! {
    /// Get latest trade snapshot. Returns TdxTradeTickArray.
    tdx_stock_snapshot_trade => stock_snapshot_trade, TdxTradeTickArray
}

// 5. stock_snapshot_quote
ffi_typed_snapshot_endpoint! {
    /// Get latest NBBO quote snapshot. Returns TdxQuoteTickArray.
    tdx_stock_snapshot_quote => stock_snapshot_quote, TdxQuoteTickArray
}

// 6. stock_snapshot_market_value
ffi_typed_snapshot_endpoint! {
    /// Get latest market value snapshot. Returns TdxMarketValueTickArray.
    tdx_stock_snapshot_market_value => stock_snapshot_market_value, TdxMarketValueTickArray
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — History endpoints (5 + bonus)
// ═══════════════════════════════════════════════════════════════════════

// 7. stock_history_eod
ffi_typed_endpoint! {
    /// Fetch stock end-of-day history. Returns TdxEodTickArray.
    tdx_stock_history_eod => stock_history_eod, TdxEodTickArray(symbol, start_date, end_date)
}

// 8. stock_history_ohlc
ffi_typed_endpoint! {
    /// Fetch stock intraday OHLC bars. Returns TdxOhlcTickArray.
    tdx_stock_history_ohlc => stock_history_ohlc, TdxOhlcTickArray(symbol, date, interval)
}

// 8b. stock_history_ohlc_range
ffi_typed_endpoint! {
    /// Fetch stock intraday OHLC bars across a date range. Returns TdxOhlcTickArray.
    tdx_stock_history_ohlc_range => stock_history_ohlc_range, TdxOhlcTickArray(symbol, start_date, end_date, interval)
}

// 9. stock_history_trade
ffi_typed_endpoint! {
    /// Fetch all trades on a date. Returns TdxTradeTickArray.
    tdx_stock_history_trade => stock_history_trade, TdxTradeTickArray(symbol, date)
}

// 10. stock_history_quote
ffi_typed_endpoint! {
    /// Fetch NBBO quotes. Returns TdxQuoteTickArray.
    tdx_stock_history_quote => stock_history_quote, TdxQuoteTickArray(symbol, date, interval)
}

// 11. stock_history_trade_quote
ffi_typed_endpoint! {
    /// Fetch combined trade + quote ticks. Returns TdxTradeQuoteTickArray.
    tdx_stock_history_trade_quote => stock_history_trade_quote, TdxTradeQuoteTickArray(symbol, date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — At-Time endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

// 12. stock_at_time_trade
ffi_typed_endpoint! {
    /// Fetch the trade at a specific time of day across a date range.
    tdx_stock_at_time_trade => stock_at_time_trade, TdxTradeTickArray(symbol, start_date, end_date, time_of_day)
}

// 13. stock_at_time_quote
ffi_typed_endpoint! {
    /// Fetch the quote at a specific time of day across a date range.
    tdx_stock_at_time_quote => stock_at_time_quote, TdxQuoteTickArray(symbol, start_date, end_date, time_of_day)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — List endpoints (5)
// ═══════════════════════════════════════════════════════════════════════

// 14. option_list_symbols
ffi_list_endpoint_no_params! {
    /// List all option underlyings. Returns TdxStringArray.
    tdx_option_list_symbols => option_list_symbols
}

// 15. option_list_dates
ffi_list_endpoint! {
    /// List available dates for an option contract. Returns TdxStringArray.
    tdx_option_list_dates => option_list_dates(request_type, symbol, expiration, strike, right)
}

// 16. option_list_expirations
ffi_list_endpoint! {
    /// List expiration dates. Returns TdxStringArray.
    tdx_option_list_expirations => option_list_expirations(symbol)
}

// 17. option_list_strikes
ffi_list_endpoint! {
    /// List strike prices. Returns TdxStringArray.
    tdx_option_list_strikes => option_list_strikes(symbol, expiration)
}

// 18. option_list_contracts
ffi_typed_endpoint! {
    /// List all option contracts for a symbol on a date. Returns TdxOptionContractArray.
    tdx_option_list_contracts => option_list_contracts, TdxOptionContractArray(request_type, symbol, date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — Snapshot endpoints (10)
// ═══════════════════════════════════════════════════════════════════════

// 19. option_snapshot_ohlc
ffi_typed_endpoint! {
    /// Get latest OHLC snapshot for options. Returns TdxOhlcTickArray.
    tdx_option_snapshot_ohlc => option_snapshot_ohlc, TdxOhlcTickArray(symbol, expiration, strike, right)
}

// 20. option_snapshot_trade
ffi_typed_endpoint! {
    /// Get latest trade snapshot for options. Returns TdxTradeTickArray.
    tdx_option_snapshot_trade => option_snapshot_trade, TdxTradeTickArray(symbol, expiration, strike, right)
}

// 21. option_snapshot_quote
ffi_typed_endpoint! {
    /// Get latest NBBO quote snapshot for options. Returns TdxQuoteTickArray.
    tdx_option_snapshot_quote => option_snapshot_quote, TdxQuoteTickArray(symbol, expiration, strike, right)
}

// 22. option_snapshot_open_interest
ffi_typed_endpoint! {
    /// Get latest open interest snapshot for options. Returns TdxOpenInterestTickArray.
    tdx_option_snapshot_open_interest => option_snapshot_open_interest, TdxOpenInterestTickArray(symbol, expiration, strike, right)
}

// 23. option_snapshot_market_value
ffi_typed_endpoint! {
    /// Get latest market value snapshot for options. Returns TdxMarketValueTickArray.
    tdx_option_snapshot_market_value => option_snapshot_market_value, TdxMarketValueTickArray(symbol, expiration, strike, right)
}

// 24. option_snapshot_greeks_implied_volatility
ffi_typed_endpoint! {
    /// Get IV snapshot for options. Returns TdxIvTickArray.
    tdx_option_snapshot_greeks_implied_volatility => option_snapshot_greeks_implied_volatility, TdxIvTickArray(symbol, expiration, strike, right)
}

// 25. option_snapshot_greeks_all
ffi_typed_endpoint! {
    /// Get all Greeks snapshot for options. Returns TdxGreeksTickArray.
    tdx_option_snapshot_greeks_all => option_snapshot_greeks_all, TdxGreeksTickArray(symbol, expiration, strike, right)
}

// 26. option_snapshot_greeks_first_order
ffi_typed_endpoint! {
    /// Get first-order Greeks snapshot. Returns TdxGreeksTickArray.
    tdx_option_snapshot_greeks_first_order => option_snapshot_greeks_first_order, TdxGreeksTickArray(symbol, expiration, strike, right)
}

// 27. option_snapshot_greeks_second_order
ffi_typed_endpoint! {
    /// Get second-order Greeks snapshot. Returns TdxGreeksTickArray.
    tdx_option_snapshot_greeks_second_order => option_snapshot_greeks_second_order, TdxGreeksTickArray(symbol, expiration, strike, right)
}

// 28. option_snapshot_greeks_third_order
ffi_typed_endpoint! {
    /// Get third-order Greeks snapshot. Returns TdxGreeksTickArray.
    tdx_option_snapshot_greeks_third_order => option_snapshot_greeks_third_order, TdxGreeksTickArray(symbol, expiration, strike, right)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — History endpoints (6)
// ═══════════════════════════════════════════════════════════════════════

// 29. option_history_eod
ffi_typed_endpoint! {
    /// Fetch EOD option data for a contract over a date range. Returns TdxEodTickArray.
    tdx_option_history_eod => option_history_eod, TdxEodTickArray(symbol, expiration, strike, right, start_date, end_date)
}

// 30. option_history_ohlc
ffi_typed_endpoint! {
    /// Fetch intraday OHLC bars for an option contract. Returns TdxOhlcTickArray.
    tdx_option_history_ohlc => option_history_ohlc, TdxOhlcTickArray(symbol, expiration, strike, right, date, interval)
}

// 31. option_history_trade
ffi_typed_endpoint! {
    /// Fetch all trades for an option contract on a date. Returns TdxTradeTickArray.
    tdx_option_history_trade => option_history_trade, TdxTradeTickArray(symbol, expiration, strike, right, date)
}

// 32. option_history_quote
ffi_typed_endpoint! {
    /// Fetch NBBO quotes for an option contract on a date. Returns TdxQuoteTickArray.
    tdx_option_history_quote => option_history_quote, TdxQuoteTickArray(symbol, expiration, strike, right, date, interval)
}

// 33. option_history_trade_quote
ffi_typed_endpoint! {
    /// Fetch combined trade + quote ticks for an option contract. Returns TdxTradeQuoteTickArray.
    tdx_option_history_trade_quote => option_history_trade_quote, TdxTradeQuoteTickArray(symbol, expiration, strike, right, date)
}

// 34. option_history_open_interest
ffi_typed_endpoint! {
    /// Fetch open interest history for an option contract. Returns TdxOpenInterestTickArray.
    tdx_option_history_open_interest => option_history_open_interest, TdxOpenInterestTickArray(symbol, expiration, strike, right, date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — History Greeks endpoints (11)
// ═══════════════════════════════════════════════════════════════════════

// 35. option_history_greeks_eod
ffi_typed_endpoint! {
    /// Fetch EOD Greeks history. Returns TdxGreeksTickArray.
    tdx_option_history_greeks_eod => option_history_greeks_eod, TdxGreeksTickArray(symbol, expiration, strike, right, start_date, end_date)
}

// 36. option_history_greeks_all
ffi_typed_endpoint! {
    /// Fetch all Greeks history (intraday). Returns TdxGreeksTickArray.
    tdx_option_history_greeks_all => option_history_greeks_all, TdxGreeksTickArray(symbol, expiration, strike, right, date, interval)
}

// 37. option_history_trade_greeks_all
ffi_typed_endpoint! {
    /// Fetch all Greeks on each trade. Returns TdxGreeksTickArray.
    tdx_option_history_trade_greeks_all => option_history_trade_greeks_all, TdxGreeksTickArray(symbol, expiration, strike, right, date)
}

// 38. option_history_greeks_first_order
ffi_typed_endpoint! {
    /// Fetch first-order Greeks history. Returns TdxGreeksTickArray.
    tdx_option_history_greeks_first_order => option_history_greeks_first_order, TdxGreeksTickArray(symbol, expiration, strike, right, date, interval)
}

// 39. option_history_trade_greeks_first_order
ffi_typed_endpoint! {
    /// Fetch first-order Greeks on each trade. Returns TdxGreeksTickArray.
    tdx_option_history_trade_greeks_first_order => option_history_trade_greeks_first_order, TdxGreeksTickArray(symbol, expiration, strike, right, date)
}

// 40. option_history_greeks_second_order
ffi_typed_endpoint! {
    /// Fetch second-order Greeks history. Returns TdxGreeksTickArray.
    tdx_option_history_greeks_second_order => option_history_greeks_second_order, TdxGreeksTickArray(symbol, expiration, strike, right, date, interval)
}

// 41. option_history_trade_greeks_second_order
ffi_typed_endpoint! {
    /// Fetch second-order Greeks on each trade. Returns TdxGreeksTickArray.
    tdx_option_history_trade_greeks_second_order => option_history_trade_greeks_second_order, TdxGreeksTickArray(symbol, expiration, strike, right, date)
}

// 42. option_history_greeks_third_order
ffi_typed_endpoint! {
    /// Fetch third-order Greeks history. Returns TdxGreeksTickArray.
    tdx_option_history_greeks_third_order => option_history_greeks_third_order, TdxGreeksTickArray(symbol, expiration, strike, right, date, interval)
}

// 43. option_history_trade_greeks_third_order
ffi_typed_endpoint! {
    /// Fetch third-order Greeks on each trade. Returns TdxGreeksTickArray.
    tdx_option_history_trade_greeks_third_order => option_history_trade_greeks_third_order, TdxGreeksTickArray(symbol, expiration, strike, right, date)
}

// 44. option_history_greeks_implied_volatility
ffi_typed_endpoint! {
    /// Fetch IV history (intraday). Returns TdxIvTickArray.
    tdx_option_history_greeks_implied_volatility => option_history_greeks_implied_volatility, TdxIvTickArray(symbol, expiration, strike, right, date, interval)
}

// 45. option_history_trade_greeks_implied_volatility
ffi_typed_endpoint! {
    /// Fetch IV on each trade. Returns TdxIvTickArray.
    tdx_option_history_trade_greeks_implied_volatility => option_history_trade_greeks_implied_volatility, TdxIvTickArray(symbol, expiration, strike, right, date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — At-Time endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

// 46. option_at_time_trade
ffi_typed_endpoint! {
    /// Fetch the trade at a specific time for an option contract. Returns TdxTradeTickArray.
    tdx_option_at_time_trade => option_at_time_trade, TdxTradeTickArray(symbol, expiration, strike, right, start_date, end_date, time_of_day)
}

// 47. option_at_time_quote
ffi_typed_endpoint! {
    /// Fetch the quote at a specific time for an option contract. Returns TdxQuoteTickArray.
    tdx_option_at_time_quote => option_at_time_quote, TdxQuoteTickArray(symbol, expiration, strike, right, start_date, end_date, time_of_day)
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — List endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

// 48. index_list_symbols
ffi_list_endpoint_no_params! {
    /// List all index symbols. Returns TdxStringArray.
    tdx_index_list_symbols => index_list_symbols
}

// 49. index_list_dates
ffi_list_endpoint! {
    /// List available dates for an index. Returns TdxStringArray.
    tdx_index_list_dates => index_list_dates(symbol)
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — Snapshot endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

// 50. index_snapshot_ohlc
ffi_typed_snapshot_endpoint! {
    /// Get latest OHLC snapshot for indices. Returns TdxOhlcTickArray.
    tdx_index_snapshot_ohlc => index_snapshot_ohlc, TdxOhlcTickArray
}

// 51. index_snapshot_price
ffi_typed_snapshot_endpoint! {
    /// Get latest price snapshot for indices. Returns TdxPriceTickArray.
    tdx_index_snapshot_price => index_snapshot_price, TdxPriceTickArray
}

// 52. index_snapshot_market_value
ffi_typed_snapshot_endpoint! {
    /// Get latest market value snapshot for indices. Returns TdxMarketValueTickArray.
    tdx_index_snapshot_market_value => index_snapshot_market_value, TdxMarketValueTickArray
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — History endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

// 53. index_history_eod
ffi_typed_endpoint! {
    /// Fetch EOD index data for a date range. Returns TdxEodTickArray.
    tdx_index_history_eod => index_history_eod, TdxEodTickArray(symbol, start_date, end_date)
}

// 54. index_history_ohlc
ffi_typed_endpoint! {
    /// Fetch intraday OHLC bars for an index. Returns TdxOhlcTickArray.
    tdx_index_history_ohlc => index_history_ohlc, TdxOhlcTickArray(symbol, start_date, end_date, interval)
}

// 55. index_history_price
ffi_typed_endpoint! {
    /// Fetch intraday price history for an index. Returns TdxPriceTickArray.
    tdx_index_history_price => index_history_price, TdxPriceTickArray(symbol, date, interval)
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — At-Time endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

// 56. index_at_time_price
ffi_typed_endpoint! {
    /// Fetch index price at a specific time across a date range. Returns TdxPriceTickArray.
    tdx_index_at_time_price => index_at_time_price, TdxPriceTickArray(symbol, start_date, end_date, time_of_day)
}

// ═══════════════════════════════════════════════════════════════════════
//  Calendar endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

// 57. calendar_open_today
ffi_typed_endpoint_no_params! {
    /// Check whether the market is open today. Returns TdxCalendarDayArray.
    tdx_calendar_open_today => calendar_open_today, TdxCalendarDayArray
}

// 58. calendar_on_date
ffi_typed_endpoint! {
    /// Get calendar information for a specific date. Returns TdxCalendarDayArray.
    tdx_calendar_on_date => calendar_on_date, TdxCalendarDayArray(date)
}

// 59. calendar_year
ffi_typed_endpoint! {
    /// Get calendar information for an entire year. Returns TdxCalendarDayArray.
    tdx_calendar_year => calendar_year, TdxCalendarDayArray(year)
}

// ═══════════════════════════════════════════════════════════════════════
//  Interest Rate endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

// 60. interest_rate_history_eod
ffi_typed_endpoint! {
    /// Fetch EOD interest rate history. Returns TdxInterestRateTickArray.
    tdx_interest_rate_history_eod => interest_rate_history_eod, TdxInterestRateTickArray(symbol, start_date, end_date)
}
