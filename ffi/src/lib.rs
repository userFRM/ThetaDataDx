//! C FFI layer for `thetadatadx` — exposes the Rust SDK as `extern "C"` functions.
//!
//! This crate is compiled as both `cdylib` (shared library) and `staticlib` (archive).
//! It is consumed by the Go (CGo) and C++ SDKs.
//!
//! # Safety
//!
//! All `unsafe extern "C"` functions in this crate follow the same safety contract:
//!
//! - Pointer arguments must be either null (handled gracefully) or valid pointers
//!   obtained from a prior `tdx_*` call.
//! - `*const c_char` arguments must point to valid, NUL-terminated C strings.
//! - Returned `*mut` pointers are heap-allocated and must be freed with the
//!   corresponding `tdx_*_free` function.
//! - Functions are not thread-safe on the same handle; callers must synchronize.
//!
//! # Memory model
//!
//! - Opaque handles (`*mut TdxClient`, `*mut TdxCredentials`, etc.) are heap-allocated
//!   via `Box::into_raw` and freed via the corresponding `tdx_*_free` function.
//! - String results are returned as JSON (`*mut c_char`), freed with `tdx_string_free`.
//! - The caller MUST free every non-null pointer returned by this library.
//!
//! # Error handling
//!
//! Functions that can fail return a null pointer on error and set a thread-local
//! error string retrievable via `tdx_last_error`.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::OnceLock;

// ── Global tokio runtime (same pattern as the Python bindings) ──

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime for thetadatadx-ffi")
    })
}

// ── Thread-local error string ──

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
}

fn set_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

/// Retrieve the last error message (or null if no error).
///
/// The returned pointer is valid until the next FFI call on the same thread.
/// Do NOT free this pointer.
#[no_mangle]
pub extern "C" fn tdx_last_error() -> *const c_char {
    LAST_ERROR.with(|e| {
        let borrow = e.borrow();
        match borrow.as_ref() {
            Some(s) => s.as_ptr(),
            None => ptr::null(),
        }
    })
}

// ── Opaque handle types ──

/// Opaque credentials handle.
pub struct TdxCredentials {
    inner: thetadatadx::Credentials,
}

/// Opaque client handle.
pub struct TdxClient {
    inner: thetadatadx::DirectClient,
}

/// Opaque config handle.
pub struct TdxConfig {
    inner: thetadatadx::DirectConfig,
}

// ── Helper: C string to &str ──

unsafe fn cstr_to_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

/// Helper: serialize a serde_json::Value to a C string.
fn json_to_cstring(val: &serde_json::Value) -> *mut c_char {
    match CString::new(val.to_string()) {
        Ok(cs) => cs.into_raw(),
        Err(_) => {
            set_error("JSON serialization produced invalid C string");
            ptr::null_mut()
        }
    }
}

/// Helper: serialize a `DataTable` as JSON `{ "headers": [...], "rows": [[...], ...] }`.
///
/// Each row is an array of values matching the header columns.
/// Values are either strings (text), numbers (int64), or price objects `{"value":N,"type":T}`.
fn data_table_to_cstring(table: &thetadatadx::proto::DataTable) -> *mut c_char {
    let headers: Vec<serde_json::Value> = table
        .headers
        .iter()
        .map(|h| serde_json::Value::String(h.clone()))
        .collect();

    let rows: Vec<serde_json::Value> = table
        .data_table
        .iter()
        .map(|row| {
            let vals: Vec<serde_json::Value> = row
                .values
                .iter()
                .map(|v| {
                    use thetadatadx::proto::data_value::DataType;
                    match &v.data_type {
                        Some(DataType::Text(s)) => serde_json::Value::String(s.clone()),
                        Some(DataType::Number(n)) => serde_json::json!(*n),
                        Some(DataType::Price(p)) => {
                            serde_json::json!({"value": p.value, "type": p.r#type})
                        }
                        Some(DataType::Timestamp(ts)) => serde_json::json!({
                            "epoch_ms": ts.epoch_ms,
                            "zone": ts.zone,
                        }),
                        None => serde_json::Value::Null,
                    }
                })
                .collect();
            serde_json::Value::Array(vals)
        })
        .collect();

    let json = serde_json::json!({
        "headers": headers,
        "rows": rows,
    });
    json_to_cstring(&json)
}

// ── Credentials ──

/// Create credentials from email and password strings.
///
/// Returns null on invalid input (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_credentials_new(
    email: *const c_char,
    password: *const c_char,
) -> *mut TdxCredentials {
    let email = match unsafe { cstr_to_str(email) } {
        Some(s) => s,
        None => {
            set_error("email is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };
    let password = match unsafe { cstr_to_str(password) } {
        Some(s) => s,
        None => {
            set_error("password is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };
    let creds = thetadatadx::Credentials::new(email, password);
    Box::into_raw(Box::new(TdxCredentials { inner: creds }))
}

/// Load credentials from a file (line 1 = email, line 2 = password).
///
/// Returns null on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_credentials_from_file(path: *const c_char) -> *mut TdxCredentials {
    let path = match unsafe { cstr_to_str(path) } {
        Some(s) => s,
        None => {
            set_error("path is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };
    match thetadatadx::Credentials::from_file(path) {
        Ok(creds) => Box::into_raw(Box::new(TdxCredentials { inner: creds })),
        Err(e) => {
            set_error(&e.to_string());
            ptr::null_mut()
        }
    }
}

/// Free a credentials handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_credentials_free(creds: *mut TdxCredentials) {
    if !creds.is_null() {
        drop(unsafe { Box::from_raw(creds) });
    }
}

// ── Config ──

/// Create a production config (ThetaData NJ datacenter).
#[no_mangle]
pub extern "C" fn tdx_config_production() -> *mut TdxConfig {
    Box::into_raw(Box::new(TdxConfig {
        inner: thetadatadx::DirectConfig::production(),
    }))
}

/// Create a dev config (shorter timeouts).
#[no_mangle]
pub extern "C" fn tdx_config_dev() -> *mut TdxConfig {
    Box::into_raw(Box::new(TdxConfig {
        inner: thetadatadx::DirectConfig::dev(),
    }))
}

/// Free a config handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_free(config: *mut TdxConfig) {
    if !config.is_null() {
        drop(unsafe { Box::from_raw(config) });
    }
}

// ── Client ──

/// Connect to ThetaData servers (authenticates via Nexus API).
///
/// Returns null on connection/auth failure (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_client_connect(
    creds: *const TdxCredentials,
    config: *const TdxConfig,
) -> *mut TdxClient {
    if creds.is_null() {
        set_error("credentials handle is null");
        return ptr::null_mut();
    }
    if config.is_null() {
        set_error("config handle is null");
        return ptr::null_mut();
    }
    let creds = unsafe { &*creds };
    let config = unsafe { &*config };
    match runtime().block_on(thetadatadx::DirectClient::connect(
        &creds.inner,
        config.inner.clone(),
    )) {
        Ok(client) => Box::into_raw(Box::new(TdxClient { inner: client })),
        Err(e) => {
            set_error(&e.to_string());
            ptr::null_mut()
        }
    }
}

/// Free a client handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_client_free(client: *mut TdxClient) {
    if !client.is_null() {
        drop(unsafe { Box::from_raw(client) });
    }
}

// ── String free ──

/// Free a string returned by any `tdx_*` function.
///
/// MUST be called for every non-null `*mut c_char` returned by this library.
#[no_mangle]
pub unsafe extern "C" fn tdx_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

// ── Tick serialization helpers ──

fn eod_tick_to_json(t: &thetadatadx::types::tick::EodTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "open": t.open_price().to_f64(),
        "high": t.high_price().to_f64(),
        "low": t.low_price().to_f64(),
        "close": t.close_price().to_f64(),
        "volume": t.volume,
        "count": t.count,
        "bid": t.bid_price().to_f64(),
        "ask": t.ask_price().to_f64(),
        "date": t.date,
    })
}

fn ohlc_tick_to_json(t: &thetadatadx::types::tick::OhlcTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "open": t.open_price().to_f64(),
        "high": t.high_price().to_f64(),
        "low": t.low_price().to_f64(),
        "close": t.close_price().to_f64(),
        "volume": t.volume,
        "count": t.count,
        "date": t.date,
    })
}

fn trade_tick_to_json(t: &thetadatadx::types::tick::TradeTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "sequence": t.sequence,
        "condition": t.condition,
        "size": t.size,
        "exchange": t.exchange,
        "price": t.get_price().to_f64(),
        "price_raw": t.price,
        "price_type": t.price_type,
        "condition_flags": t.condition_flags,
        "price_flags": t.price_flags,
        "volume_type": t.volume_type,
        "records_back": t.records_back,
        "date": t.date,
    })
}

fn quote_tick_to_json(t: &thetadatadx::types::tick::QuoteTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "bid_size": t.bid_size,
        "bid_exchange": t.bid_exchange,
        "bid": t.bid_price().to_f64(),
        "bid_condition": t.bid_condition,
        "ask_size": t.ask_size,
        "ask_exchange": t.ask_exchange,
        "ask": t.ask_price().to_f64(),
        "ask_condition": t.ask_condition,
        "date": t.date,
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  FFI macros — eliminate boilerplate across all endpoint wrappers
// ═══════════════════════════════════════════════════════════════════════

/// FFI wrapper for list endpoints that return `Vec<String>` (no extra params beyond client).
macro_rules! ffi_list_endpoint_no_params {
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(client: *const TdxClient) -> *mut c_char {
            if client.is_null() {
                set_error("client handle is null");
                return ptr::null_mut();
            }
            let client = unsafe { &*client };
            match runtime().block_on(client.inner.$method()) {
                Ok(items) => {
                    let json = serde_json::Value::Array(
                        items.into_iter().map(serde_json::Value::String).collect(),
                    );
                    json_to_cstring(&json)
                }
                Err(e) => {
                    set_error(&e.to_string());
                    ptr::null_mut()
                }
            }
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
        ) -> *mut c_char {
            if client.is_null() {
                set_error("client handle is null");
                return ptr::null_mut();
            }
            let client = unsafe { &*client };
            $(
                let $param = match unsafe { cstr_to_str($param) } {
                    Some(s) => s,
                    None => {
                        set_error(concat!(stringify!($param), " is null or invalid UTF-8"));
                        return ptr::null_mut();
                    }
                };
            )+
            match runtime().block_on(client.inner.$method($($param),+)) {
                Ok(items) => {
                    let json = serde_json::Value::Array(
                        items.into_iter().map(serde_json::Value::String).collect(),
                    );
                    json_to_cstring(&json)
                }
                Err(e) => {
                    set_error(&e.to_string());
                    ptr::null_mut()
                }
            }
        }
    };
}

/// FFI wrapper for snapshot endpoints that take a JSON array of symbols and return tick arrays.
macro_rules! ffi_snapshot_endpoint {
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident, $tick_to_json:ident
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(
            client: *const TdxClient,
            symbols_json: *const c_char,
        ) -> *mut c_char {
            if client.is_null() {
                set_error("client handle is null");
                return ptr::null_mut();
            }
            let client = unsafe { &*client };
            let json_str = match unsafe { cstr_to_str(symbols_json) } {
                Some(s) => s,
                None => {
                    set_error("symbols_json is null or invalid UTF-8");
                    return ptr::null_mut();
                }
            };
            let symbols: Vec<String> = match serde_json::from_str(json_str) {
                Ok(s) => s,
                Err(e) => {
                    set_error(&format!("invalid symbols JSON: {}", e));
                    return ptr::null_mut();
                }
            };
            let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
            match runtime().block_on(client.inner.$method(&refs)) {
                Ok(ticks) => {
                    let json =
                        serde_json::Value::Array(ticks.iter().map($tick_to_json).collect());
                    json_to_cstring(&json)
                }
                Err(e) => {
                    set_error(&e.to_string());
                    ptr::null_mut()
                }
            }
        }
    };
}

/// FFI wrapper for raw snapshot endpoints (return DataTable as JSON).
macro_rules! ffi_snapshot_raw_endpoint {
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(
            client: *const TdxClient,
            symbols_json: *const c_char,
        ) -> *mut c_char {
            if client.is_null() {
                set_error("client handle is null");
                return ptr::null_mut();
            }
            let client = unsafe { &*client };
            let json_str = match unsafe { cstr_to_str(symbols_json) } {
                Some(s) => s,
                None => {
                    set_error("symbols_json is null or invalid UTF-8");
                    return ptr::null_mut();
                }
            };
            let symbols: Vec<String> = match serde_json::from_str(json_str) {
                Ok(s) => s,
                Err(e) => {
                    set_error(&format!("invalid symbols JSON: {}", e));
                    return ptr::null_mut();
                }
            };
            let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
            match runtime().block_on(client.inner.$method(&refs)) {
                Ok(table) => data_table_to_cstring(&table),
                Err(e) => {
                    set_error(&e.to_string());
                    ptr::null_mut()
                }
            }
        }
    };
}

/// FFI wrapper for parsed tick endpoints with C string params.
macro_rules! ffi_parsed_endpoint {
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident, $tick_to_json:ident ( $($param:ident),+ )
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(
            client: *const TdxClient,
            $($param: *const c_char),+
        ) -> *mut c_char {
            if client.is_null() {
                set_error("client handle is null");
                return ptr::null_mut();
            }
            let client = unsafe { &*client };
            $(
                let $param = match unsafe { cstr_to_str($param) } {
                    Some(s) => s,
                    None => {
                        set_error(concat!(stringify!($param), " is null or invalid UTF-8"));
                        return ptr::null_mut();
                    }
                };
            )+
            match runtime().block_on(client.inner.$method($($param),+)) {
                Ok(ticks) => {
                    let json =
                        serde_json::Value::Array(ticks.iter().map($tick_to_json).collect());
                    json_to_cstring(&json)
                }
                Err(e) => {
                    set_error(&e.to_string());
                    ptr::null_mut()
                }
            }
        }
    };
}

/// FFI wrapper for raw DataTable endpoints with C string params.
macro_rules! ffi_raw_endpoint {
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident ( $($param:ident),+ )
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(
            client: *const TdxClient,
            $($param: *const c_char),+
        ) -> *mut c_char {
            if client.is_null() {
                set_error("client handle is null");
                return ptr::null_mut();
            }
            let client = unsafe { &*client };
            $(
                let $param = match unsafe { cstr_to_str($param) } {
                    Some(s) => s,
                    None => {
                        set_error(concat!(stringify!($param), " is null or invalid UTF-8"));
                        return ptr::null_mut();
                    }
                };
            )+
            match runtime().block_on(client.inner.$method($($param),+)) {
                Ok(table) => data_table_to_cstring(&table),
                Err(e) => {
                    set_error(&e.to_string());
                    ptr::null_mut()
                }
            }
        }
    };
}

/// FFI wrapper for raw DataTable endpoints with no params.
macro_rules! ffi_raw_endpoint_no_params {
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident
    ) => {
        $(#[$meta])*
        #[no_mangle]
        pub unsafe extern "C" fn $ffi_name(client: *const TdxClient) -> *mut c_char {
            if client.is_null() {
                set_error("client handle is null");
                return ptr::null_mut();
            }
            let client = unsafe { &*client };
            match runtime().block_on(client.inner.$method()) {
                Ok(table) => data_table_to_cstring(&table),
                Err(e) => {
                    set_error(&e.to_string());
                    ptr::null_mut()
                }
            }
        }
    };
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — List endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

// 1. stock_list_symbols
ffi_list_endpoint_no_params! {
    /// List all available stock symbols. Returns JSON array of strings.
    tdx_stock_list_symbols => stock_list_symbols
}

// 2. stock_list_dates
ffi_list_endpoint! {
    /// List available dates for a stock by request type. Returns JSON array of date strings.
    tdx_stock_list_dates => stock_list_dates(request_type, symbol)
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — Snapshot endpoints (4)
// ═══════════════════════════════════════════════════════════════════════

// 3. stock_snapshot_ohlc
ffi_snapshot_endpoint! {
    /// Get latest OHLC snapshot. symbols_json is JSON array. Returns JSON array of OHLC ticks.
    tdx_stock_snapshot_ohlc => stock_snapshot_ohlc, ohlc_tick_to_json
}

// 4. stock_snapshot_trade
ffi_snapshot_endpoint! {
    /// Get latest trade snapshot. symbols_json is JSON array. Returns JSON array of trade ticks.
    tdx_stock_snapshot_trade => stock_snapshot_trade, trade_tick_to_json
}

// 5. stock_snapshot_quote
ffi_snapshot_endpoint! {
    /// Get latest NBBO quote snapshot. symbols_json is JSON array. Returns JSON array of quote ticks.
    tdx_stock_snapshot_quote => stock_snapshot_quote, quote_tick_to_json
}

// 6. stock_snapshot_market_value
ffi_snapshot_raw_endpoint! {
    /// Get latest market value snapshot. symbols_json is JSON array. Returns JSON DataTable.
    tdx_stock_snapshot_market_value => stock_snapshot_market_value
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — History endpoints (5 + bonus)
// ═══════════════════════════════════════════════════════════════════════

// 7. stock_history_eod
ffi_parsed_endpoint! {
    /// Fetch stock end-of-day history. Returns JSON array of EOD ticks.
    tdx_stock_history_eod => stock_history_eod, eod_tick_to_json(symbol, start_date, end_date)
}

// 8. stock_history_ohlc
ffi_parsed_endpoint! {
    /// Fetch stock intraday OHLC bars. Returns JSON array of OHLC ticks.
    tdx_stock_history_ohlc => stock_history_ohlc, ohlc_tick_to_json(symbol, date, interval)
}

// 8b. stock_history_ohlc_range
ffi_parsed_endpoint! {
    /// Fetch stock intraday OHLC bars across a date range. Returns JSON array.
    tdx_stock_history_ohlc_range => stock_history_ohlc_range, ohlc_tick_to_json(symbol, start_date, end_date, interval)
}

// 9. stock_history_trade
ffi_parsed_endpoint! {
    /// Fetch all trades on a date. Returns JSON array of trade ticks.
    tdx_stock_history_trade => stock_history_trade, trade_tick_to_json(symbol, date)
}

// 10. stock_history_quote
ffi_parsed_endpoint! {
    /// Fetch NBBO quotes. Returns JSON array of quote ticks.
    tdx_stock_history_quote => stock_history_quote, quote_tick_to_json(symbol, date, interval)
}

// 11. stock_history_trade_quote
ffi_raw_endpoint! {
    /// Fetch combined trade + quote ticks. Returns JSON DataTable.
    tdx_stock_history_trade_quote => stock_history_trade_quote(symbol, date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — At-Time endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

// 12. stock_at_time_trade
ffi_parsed_endpoint! {
    /// Fetch the trade at a specific time of day across a date range.
    tdx_stock_at_time_trade => stock_at_time_trade, trade_tick_to_json(symbol, start_date, end_date, time_of_day)
}

// 13. stock_at_time_quote
ffi_parsed_endpoint! {
    /// Fetch the quote at a specific time of day across a date range.
    tdx_stock_at_time_quote => stock_at_time_quote, quote_tick_to_json(symbol, start_date, end_date, time_of_day)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — List endpoints (5)
// ═══════════════════════════════════════════════════════════════════════

// 14. option_list_symbols
ffi_list_endpoint_no_params! {
    /// List all option underlyings. Returns JSON array of strings.
    tdx_option_list_symbols => option_list_symbols
}

// 15. option_list_dates
ffi_list_endpoint! {
    /// List available dates for an option contract. Returns JSON array of date strings.
    tdx_option_list_dates => option_list_dates(request_type, symbol, expiration, strike, right)
}

// 16. option_list_expirations
ffi_list_endpoint! {
    /// List expiration dates. Returns JSON array of date strings.
    tdx_option_list_expirations => option_list_expirations(symbol)
}

// 17. option_list_strikes
ffi_list_endpoint! {
    /// List strike prices. Returns JSON array of strings.
    tdx_option_list_strikes => option_list_strikes(symbol, expiration)
}

// 18. option_list_contracts
ffi_raw_endpoint! {
    /// List all option contracts for a symbol on a date. Returns JSON DataTable.
    tdx_option_list_contracts => option_list_contracts(request_type, symbol, date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — Snapshot endpoints (10)
// ═══════════════════════════════════════════════════════════════════════

// 19. option_snapshot_ohlc
ffi_parsed_endpoint! {
    /// Get latest OHLC snapshot for options. Returns JSON array.
    tdx_option_snapshot_ohlc => option_snapshot_ohlc, ohlc_tick_to_json(symbol, expiration, strike, right)
}

// 20. option_snapshot_trade
ffi_parsed_endpoint! {
    /// Get latest trade snapshot for options. Returns JSON array.
    tdx_option_snapshot_trade => option_snapshot_trade, trade_tick_to_json(symbol, expiration, strike, right)
}

// 21. option_snapshot_quote
ffi_parsed_endpoint! {
    /// Get latest NBBO quote snapshot for options. Returns JSON array.
    tdx_option_snapshot_quote => option_snapshot_quote, quote_tick_to_json(symbol, expiration, strike, right)
}

// 22. option_snapshot_open_interest
ffi_raw_endpoint! {
    /// Get latest open interest snapshot for options. Returns JSON DataTable.
    tdx_option_snapshot_open_interest => option_snapshot_open_interest(symbol, expiration, strike, right)
}

// 23. option_snapshot_market_value
ffi_raw_endpoint! {
    /// Get latest market value snapshot for options. Returns JSON DataTable.
    tdx_option_snapshot_market_value => option_snapshot_market_value(symbol, expiration, strike, right)
}

// 24. option_snapshot_greeks_implied_volatility
ffi_raw_endpoint! {
    /// Get IV snapshot for options. Returns JSON DataTable.
    tdx_option_snapshot_greeks_implied_volatility => option_snapshot_greeks_implied_volatility(symbol, expiration, strike, right)
}

// 25. option_snapshot_greeks_all
ffi_raw_endpoint! {
    /// Get all Greeks snapshot for options. Returns JSON DataTable.
    tdx_option_snapshot_greeks_all => option_snapshot_greeks_all(symbol, expiration, strike, right)
}

// 26. option_snapshot_greeks_first_order
ffi_raw_endpoint! {
    /// Get first-order Greeks snapshot. Returns JSON DataTable.
    tdx_option_snapshot_greeks_first_order => option_snapshot_greeks_first_order(symbol, expiration, strike, right)
}

// 27. option_snapshot_greeks_second_order
ffi_raw_endpoint! {
    /// Get second-order Greeks snapshot. Returns JSON DataTable.
    tdx_option_snapshot_greeks_second_order => option_snapshot_greeks_second_order(symbol, expiration, strike, right)
}

// 28. option_snapshot_greeks_third_order
ffi_raw_endpoint! {
    /// Get third-order Greeks snapshot. Returns JSON DataTable.
    tdx_option_snapshot_greeks_third_order => option_snapshot_greeks_third_order(symbol, expiration, strike, right)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — History endpoints (6)
// ═══════════════════════════════════════════════════════════════════════

// 29. option_history_eod
ffi_parsed_endpoint! {
    /// Fetch EOD option data for a contract over a date range. Returns JSON array of EOD ticks.
    tdx_option_history_eod => option_history_eod, eod_tick_to_json(symbol, expiration, strike, right, start_date, end_date)
}

// 30. option_history_ohlc
ffi_parsed_endpoint! {
    /// Fetch intraday OHLC bars for an option contract. Returns JSON array.
    tdx_option_history_ohlc => option_history_ohlc, ohlc_tick_to_json(symbol, expiration, strike, right, date, interval)
}

// 31. option_history_trade
ffi_parsed_endpoint! {
    /// Fetch all trades for an option contract on a date. Returns JSON array.
    tdx_option_history_trade => option_history_trade, trade_tick_to_json(symbol, expiration, strike, right, date)
}

// 32. option_history_quote
ffi_parsed_endpoint! {
    /// Fetch NBBO quotes for an option contract on a date. Returns JSON array.
    tdx_option_history_quote => option_history_quote, quote_tick_to_json(symbol, expiration, strike, right, date, interval)
}

// 33. option_history_trade_quote
ffi_raw_endpoint! {
    /// Fetch combined trade + quote ticks for an option contract. Returns JSON DataTable.
    tdx_option_history_trade_quote => option_history_trade_quote(symbol, expiration, strike, right, date)
}

// 34. option_history_open_interest
ffi_raw_endpoint! {
    /// Fetch open interest history for an option contract. Returns JSON DataTable.
    tdx_option_history_open_interest => option_history_open_interest(symbol, expiration, strike, right, date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — History Greeks endpoints (11)
// ═══════════════════════════════════════════════════════════════════════

// 35. option_history_greeks_eod
ffi_raw_endpoint! {
    /// Fetch EOD Greeks history. Returns JSON DataTable.
    tdx_option_history_greeks_eod => option_history_greeks_eod(symbol, expiration, strike, right, start_date, end_date)
}

// 36. option_history_greeks_all
ffi_raw_endpoint! {
    /// Fetch all Greeks history (intraday). Returns JSON DataTable.
    tdx_option_history_greeks_all => option_history_greeks_all(symbol, expiration, strike, right, date, interval)
}

// 37. option_history_trade_greeks_all
ffi_raw_endpoint! {
    /// Fetch all Greeks on each trade. Returns JSON DataTable.
    tdx_option_history_trade_greeks_all => option_history_trade_greeks_all(symbol, expiration, strike, right, date)
}

// 38. option_history_greeks_first_order
ffi_raw_endpoint! {
    /// Fetch first-order Greeks history. Returns JSON DataTable.
    tdx_option_history_greeks_first_order => option_history_greeks_first_order(symbol, expiration, strike, right, date, interval)
}

// 39. option_history_trade_greeks_first_order
ffi_raw_endpoint! {
    /// Fetch first-order Greeks on each trade. Returns JSON DataTable.
    tdx_option_history_trade_greeks_first_order => option_history_trade_greeks_first_order(symbol, expiration, strike, right, date)
}

// 40. option_history_greeks_second_order
ffi_raw_endpoint! {
    /// Fetch second-order Greeks history. Returns JSON DataTable.
    tdx_option_history_greeks_second_order => option_history_greeks_second_order(symbol, expiration, strike, right, date, interval)
}

// 41. option_history_trade_greeks_second_order
ffi_raw_endpoint! {
    /// Fetch second-order Greeks on each trade. Returns JSON DataTable.
    tdx_option_history_trade_greeks_second_order => option_history_trade_greeks_second_order(symbol, expiration, strike, right, date)
}

// 42. option_history_greeks_third_order
ffi_raw_endpoint! {
    /// Fetch third-order Greeks history. Returns JSON DataTable.
    tdx_option_history_greeks_third_order => option_history_greeks_third_order(symbol, expiration, strike, right, date, interval)
}

// 43. option_history_trade_greeks_third_order
ffi_raw_endpoint! {
    /// Fetch third-order Greeks on each trade. Returns JSON DataTable.
    tdx_option_history_trade_greeks_third_order => option_history_trade_greeks_third_order(symbol, expiration, strike, right, date)
}

// 44. option_history_greeks_implied_volatility
ffi_raw_endpoint! {
    /// Fetch IV history (intraday). Returns JSON DataTable.
    tdx_option_history_greeks_implied_volatility => option_history_greeks_implied_volatility(symbol, expiration, strike, right, date, interval)
}

// 45. option_history_trade_greeks_implied_volatility
ffi_raw_endpoint! {
    /// Fetch IV on each trade. Returns JSON DataTable.
    tdx_option_history_trade_greeks_implied_volatility => option_history_trade_greeks_implied_volatility(symbol, expiration, strike, right, date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — At-Time endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

// 46. option_at_time_trade
ffi_parsed_endpoint! {
    /// Fetch the trade at a specific time for an option contract. Returns JSON array.
    tdx_option_at_time_trade => option_at_time_trade, trade_tick_to_json(symbol, expiration, strike, right, start_date, end_date, time_of_day)
}

// 47. option_at_time_quote
ffi_parsed_endpoint! {
    /// Fetch the quote at a specific time for an option contract. Returns JSON array.
    tdx_option_at_time_quote => option_at_time_quote, quote_tick_to_json(symbol, expiration, strike, right, start_date, end_date, time_of_day)
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — List endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

// 48. index_list_symbols
ffi_list_endpoint_no_params! {
    /// List all index symbols. Returns JSON array of strings.
    tdx_index_list_symbols => index_list_symbols
}

// 49. index_list_dates
ffi_list_endpoint! {
    /// List available dates for an index. Returns JSON array of date strings.
    tdx_index_list_dates => index_list_dates(symbol)
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — Snapshot endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

// 50. index_snapshot_ohlc
ffi_snapshot_endpoint! {
    /// Get latest OHLC snapshot for indices. Returns JSON array.
    tdx_index_snapshot_ohlc => index_snapshot_ohlc, ohlc_tick_to_json
}

// 51. index_snapshot_price
ffi_snapshot_raw_endpoint! {
    /// Get latest price snapshot for indices. Returns JSON DataTable.
    tdx_index_snapshot_price => index_snapshot_price
}

// 52. index_snapshot_market_value
ffi_snapshot_raw_endpoint! {
    /// Get latest market value snapshot for indices. Returns JSON DataTable.
    tdx_index_snapshot_market_value => index_snapshot_market_value
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — History endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

// 53. index_history_eod
ffi_parsed_endpoint! {
    /// Fetch EOD index data for a date range. Returns JSON array of EOD ticks.
    tdx_index_history_eod => index_history_eod, eod_tick_to_json(symbol, start_date, end_date)
}

// 54. index_history_ohlc
ffi_parsed_endpoint! {
    /// Fetch intraday OHLC bars for an index. Returns JSON array.
    tdx_index_history_ohlc => index_history_ohlc, ohlc_tick_to_json(symbol, start_date, end_date, interval)
}

// 55. index_history_price
ffi_raw_endpoint! {
    /// Fetch intraday price history for an index. Returns JSON DataTable.
    tdx_index_history_price => index_history_price(symbol, date, interval)
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — At-Time endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

// 56. index_at_time_price
ffi_raw_endpoint! {
    /// Fetch index price at a specific time across a date range. Returns JSON DataTable.
    tdx_index_at_time_price => index_at_time_price(symbol, start_date, end_date, time_of_day)
}

// ═══════════════════════════════════════════════════════════════════════
//  Calendar endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

// 57. calendar_open_today
ffi_raw_endpoint_no_params! {
    /// Check whether the market is open today. Returns JSON DataTable.
    tdx_calendar_open_today => calendar_open_today
}

// 58. calendar_on_date
ffi_raw_endpoint! {
    /// Get calendar information for a specific date. Returns JSON DataTable.
    tdx_calendar_on_date => calendar_on_date(date)
}

// 59. calendar_year
ffi_raw_endpoint! {
    /// Get calendar information for an entire year. Returns JSON DataTable.
    tdx_calendar_year => calendar_year(year)
}

// ═══════════════════════════════════════════════════════════════════════
//  Interest Rate endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

// 60. interest_rate_history_eod
ffi_raw_endpoint! {
    /// Fetch EOD interest rate history. Returns JSON DataTable.
    tdx_interest_rate_history_eod => interest_rate_history_eod(symbol, start_date, end_date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Greeks (standalone, not client methods)
// ═══════════════════════════════════════════════════════════════════════

/// Compute all 22 Black-Scholes Greeks + IV.
///
/// Returns a JSON object with all greek values.
/// Caller must free the result with `tdx_string_free`.
#[no_mangle]
pub extern "C" fn tdx_all_greeks(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    is_call: i32,
) -> *mut c_char {
    let g = thetadatadx::greeks::all_greeks(
        spot,
        strike,
        rate,
        div_yield,
        tte,
        option_price,
        is_call != 0,
    );
    let json = serde_json::json!({
        "value": g.value,
        "delta": g.delta,
        "gamma": g.gamma,
        "theta": g.theta,
        "vega": g.vega,
        "rho": g.rho,
        "iv": g.iv,
        "iv_error": g.iv_error,
        "vanna": g.vanna,
        "charm": g.charm,
        "vomma": g.vomma,
        "veta": g.veta,
        "speed": g.speed,
        "zomma": g.zomma,
        "color": g.color,
        "ultima": g.ultima,
        "d1": g.d1,
        "d2": g.d2,
        "dual_delta": g.dual_delta,
        "dual_gamma": g.dual_gamma,
        "epsilon": g.epsilon,
        "lambda": g.lambda,
    });
    json_to_cstring(&json)
}

/// Compute implied volatility via bisection.
///
/// Returns IV in `*out_iv` and error in `*out_error`.
/// Returns 0 on success, -1 on failure.
#[no_mangle]
pub unsafe extern "C" fn tdx_implied_volatility(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    is_call: i32,
    out_iv: *mut f64,
    out_error: *mut f64,
) -> i32 {
    if out_iv.is_null() || out_error.is_null() {
        set_error("output pointers must not be null");
        return -1;
    }
    let (iv, err) = thetadatadx::greeks::implied_volatility(
        spot,
        strike,
        rate,
        div_yield,
        tte,
        option_price,
        is_call != 0,
    );
    unsafe {
        *out_iv = iv;
        *out_error = err;
    }
    0
}
