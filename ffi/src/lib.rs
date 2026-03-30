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
use std::sync::{Arc, Mutex, OnceLock};

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
///
/// `repr(transparent)` guarantees `*const TdxClient` and `*const DirectClient`
/// have identical layout, allowing safe pointer casts in `tdx_unified_historical()`.
#[repr(transparent)]
pub struct TdxClient {
    inner: thetadatadx::direct::DirectClient,
}

/// Opaque config handle.
pub struct TdxConfig {
    inner: thetadatadx::DirectConfig,
}

/// Opaque unified client handle — wraps both historical and streaming.
pub struct TdxUnified {
    inner: thetadatadx::ThetaDataDx,
    /// Created lazily when `tdx_unified_start_streaming()` is called.
    rx: Mutex<Option<Arc<Mutex<std::sync::mpsc::Receiver<FfiBufferedEvent>>>>>,
}

/// Opaque FPSS streaming client handle.
///
/// Uses the same pattern as the Python SDK: an internal mpsc channel buffering
/// events from the Disruptor callback, and `tdx_fpss_next_event` polls it with
/// a timeout, returning JSON.
pub struct TdxFpssHandle {
    inner: Arc<Mutex<Option<thetadatadx::fpss::FpssClient>>>,
    rx: Arc<Mutex<std::sync::mpsc::Receiver<FfiBufferedEvent>>>,
}

/// Internal buffered event — carries decoded tick fields as JSON-ready data.
///
/// Tick data events carry all decoded fields directly. No raw payloads.
#[derive(Clone, Debug)]
struct FfiBufferedEvent {
    /// JSON object containing all event fields.
    json: serde_json::Value,
}

/// Convert raw integer price to f64 using ThetaData's price_type encoding.
fn ffi_price_to_f64(value: i32, price_type: i32) -> f64 {
    thetadatadx::types::price::Price::new(value, price_type).to_f64()
}

fn fpss_event_to_ffi(event: &thetadatadx::fpss::FpssEvent) -> FfiBufferedEvent {
    use thetadatadx::fpss::{FpssControl, FpssData, FpssEvent};
    let json = match event {
        FpssEvent::Data(FpssData::Quote {
            contract_id,
            ms_of_day,
            bid_size,
            bid_exchange,
            bid,
            bid_condition,
            ask_size,
            ask_exchange,
            ask,
            ask_condition,
            price_type,
            date,
        }) => serde_json::json!({
            "kind": "quote",
            "contract_id": contract_id,
            "ms_of_day": ms_of_day,
            "bid_size": bid_size,
            "bid_exchange": bid_exchange,
            "bid": ffi_price_to_f64(*bid, *price_type),
            "bid_condition": bid_condition,
            "ask_size": ask_size,
            "ask_exchange": ask_exchange,
            "ask": ffi_price_to_f64(*ask, *price_type),
            "ask_condition": ask_condition,
            "date": date,
        }),
        FpssEvent::Data(FpssData::Trade {
            contract_id,
            ms_of_day,
            sequence,
            condition,
            size,
            exchange,
            price,
            condition_flags,
            price_flags,
            volume_type,
            records_back,
            price_type,
            date,
            ..
        }) => serde_json::json!({
            "kind": "trade",
            "contract_id": contract_id,
            "ms_of_day": ms_of_day,
            "sequence": sequence,
            "condition": condition,
            "size": size,
            "exchange": exchange,
            "price": ffi_price_to_f64(*price, *price_type),
            "price_raw": price,
            "price_type": price_type,
            "condition_flags": condition_flags,
            "price_flags": price_flags,
            "volume_type": volume_type,
            "records_back": records_back,
            "date": date,
        }),
        FpssEvent::Data(FpssData::OpenInterest {
            contract_id,
            ms_of_day,
            open_interest,
            date,
        }) => serde_json::json!({
            "kind": "open_interest",
            "contract_id": contract_id,
            "ms_of_day": ms_of_day,
            "open_interest": open_interest,
            "date": date,
        }),
        FpssEvent::Data(FpssData::Ohlcvc {
            contract_id,
            ms_of_day,
            open,
            high,
            low,
            close,
            volume,
            count,
            price_type,
            date,
        }) => serde_json::json!({
            "kind": "ohlcvc",
            "contract_id": contract_id,
            "ms_of_day": ms_of_day,
            "open": ffi_price_to_f64(*open, *price_type),
            "high": ffi_price_to_f64(*high, *price_type),
            "low": ffi_price_to_f64(*low, *price_type),
            "close": ffi_price_to_f64(*close, *price_type),
            "volume": volume,
            "count": count,
            "date": date,
        }),
        FpssEvent::RawData { code, payload } => {
            use std::fmt::Write;
            let mut hex = String::with_capacity(payload.len() * 2);
            for byte in payload {
                let _ = write!(hex, "{byte:02x}");
            }
            serde_json::json!({
                "kind": "raw_data",
                "code": code,
                "payload_hex": hex,
            })
        }
        FpssEvent::Control(FpssControl::LoginSuccess { permissions }) => serde_json::json!({
            "kind": "login_success",
            "detail": permissions,
        }),
        FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) => serde_json::json!({
            "kind": "contract_assigned",
            "id": id,
            "detail": format!("{contract}"),
        }),
        FpssEvent::Control(FpssControl::ReqResponse { req_id, result }) => serde_json::json!({
            "kind": "req_response",
            "id": req_id,
            "detail": format!("{result:?}"),
        }),
        FpssEvent::Control(FpssControl::MarketOpen) => serde_json::json!({ "kind": "market_open" }),
        FpssEvent::Control(FpssControl::MarketClose) => {
            serde_json::json!({ "kind": "market_close" })
        }
        FpssEvent::Control(FpssControl::ServerError { message }) => serde_json::json!({
            "kind": "server_error",
            "detail": message,
        }),
        FpssEvent::Control(FpssControl::Disconnected { reason }) => serde_json::json!({
            "kind": "disconnected",
            "detail": format!("{reason:?}"),
        }),
        FpssEvent::Control(FpssControl::Error { message }) => serde_json::json!({
            "kind": "error",
            "detail": message,
        }),
        _ => serde_json::json!({ "kind": "unknown" }),
    };
    FfiBufferedEvent { json }
}

/// Serialize a buffered event to a JSON C string.
fn buffered_event_to_cstring(event: &FfiBufferedEvent) -> *mut c_char {
    json_to_cstring(&event.json)
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
#[allow(dead_code)]
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
                        Some(DataType::NullValue(_)) | None => serde_json::Value::Null,
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
    match runtime().block_on(thetadatadx::direct::DirectClient::connect(
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

fn trade_quote_tick_to_json(t: &thetadatadx::types::tick::TradeQuoteTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "sequence": t.sequence,
        "condition": t.condition,
        "size": t.size,
        "exchange": t.exchange,
        "price": t.trade_price().to_f64(),
        "price_raw": t.price,
        "price_type": t.price_type,
        "condition_flags": t.condition_flags,
        "price_flags": t.price_flags,
        "volume_type": t.volume_type,
        "records_back": t.records_back,
        "quote_ms_of_day": t.quote_ms_of_day,
        "bid_size": t.bid_size,
        "bid_exchange": t.bid_exchange,
        "bid": t.bid_price().to_f64(),
        "bid_condition": t.bid_condition,
        "ask_size": t.ask_size,
        "ask_exchange": t.ask_exchange,
        "ask": t.ask_price().to_f64(),
        "ask_condition": t.ask_condition,
        "quote_price_type": t.quote_price_type,
        "date": t.date,
    })
}

fn open_interest_tick_to_json(t: &thetadatadx::types::tick::OpenInterestTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "open_interest": t.open_interest,
        "date": t.date,
    })
}

fn market_value_tick_to_json(t: &thetadatadx::types::tick::MarketValueTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "market_cap": t.market_cap,
        "shares_outstanding": t.shares_outstanding,
        "enterprise_value": t.enterprise_value,
        "book_value": t.book_value,
        "free_float": t.free_float,
        "date": t.date,
    })
}

fn greeks_tick_to_json(t: &thetadatadx::types::tick::GreeksTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "implied_volatility": t.implied_volatility,
        "delta": t.delta,
        "gamma": t.gamma,
        "theta": t.theta,
        "vega": t.vega,
        "rho": t.rho,
        "iv_error": t.iv_error,
        "vanna": t.vanna,
        "charm": t.charm,
        "vomma": t.vomma,
        "veta": t.veta,
        "speed": t.speed,
        "zomma": t.zomma,
        "color": t.color,
        "ultima": t.ultima,
        "d1": t.d1,
        "d2": t.d2,
        "dual_delta": t.dual_delta,
        "dual_gamma": t.dual_gamma,
        "epsilon": t.epsilon,
        "lambda": t.lambda,
        "vera": t.vera,
        "date": t.date,
    })
}

fn iv_tick_to_json(t: &thetadatadx::types::tick::IvTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "implied_volatility": t.implied_volatility,
        "iv_error": t.iv_error,
        "date": t.date,
    })
}

fn price_tick_to_json(t: &thetadatadx::types::tick::PriceTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "price": t.get_price().to_f64(),
        "price_raw": t.price,
        "price_type": t.price_type,
        "date": t.date,
    })
}

fn calendar_day_to_json(t: &thetadatadx::types::tick::CalendarDay) -> serde_json::Value {
    serde_json::json!({
        "date": t.date,
        "is_open": t.is_open,
        "open_time": t.open_time,
        "close_time": t.close_time,
        "status": t.status,
    })
}

fn interest_rate_tick_to_json(t: &thetadatadx::types::tick::InterestRateTick) -> serde_json::Value {
    serde_json::json!({
        "ms_of_day": t.ms_of_day,
        "rate": t.rate,
        "date": t.date,
    })
}

fn option_contract_to_json(t: &thetadatadx::types::tick::OptionContract) -> serde_json::Value {
    serde_json::json!({
        "root": t.root,
        "expiration": t.expiration,
        "strike": t.strike,
        "right": t.right,
        "strike_price_type": t.strike_price_type,
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

/// FFI wrapper for parsed endpoints with no params.
macro_rules! ffi_parsed_endpoint_no_params {
    (
        $(#[$meta:meta])*
        $ffi_name:ident => $method:ident, $tick_to_json:ident
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
ffi_snapshot_endpoint! {
    /// Get latest market value snapshot. symbols_json is JSON array. Returns JSON array.
    tdx_stock_snapshot_market_value => stock_snapshot_market_value, market_value_tick_to_json
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
ffi_parsed_endpoint! {
    /// Fetch combined trade + quote ticks. Returns JSON array.
    tdx_stock_history_trade_quote => stock_history_trade_quote, trade_quote_tick_to_json(symbol, date)
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
ffi_parsed_endpoint! {
    /// List all option contracts for a symbol on a date. Returns JSON array.
    tdx_option_list_contracts => option_list_contracts, option_contract_to_json(request_type, symbol, date)
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
ffi_parsed_endpoint! {
    /// Get latest open interest snapshot for options. Returns JSON array.
    tdx_option_snapshot_open_interest => option_snapshot_open_interest, open_interest_tick_to_json(symbol, expiration, strike, right)
}

// 23. option_snapshot_market_value
ffi_parsed_endpoint! {
    /// Get latest market value snapshot for options. Returns JSON array.
    tdx_option_snapshot_market_value => option_snapshot_market_value, market_value_tick_to_json(symbol, expiration, strike, right)
}

// 24. option_snapshot_greeks_implied_volatility
ffi_parsed_endpoint! {
    /// Get IV snapshot for options. Returns JSON array.
    tdx_option_snapshot_greeks_implied_volatility => option_snapshot_greeks_implied_volatility, iv_tick_to_json(symbol, expiration, strike, right)
}

// 25. option_snapshot_greeks_all
ffi_parsed_endpoint! {
    /// Get all Greeks snapshot for options. Returns JSON array.
    tdx_option_snapshot_greeks_all => option_snapshot_greeks_all, greeks_tick_to_json(symbol, expiration, strike, right)
}

// 26. option_snapshot_greeks_first_order
ffi_parsed_endpoint! {
    /// Get first-order Greeks snapshot. Returns JSON array.
    tdx_option_snapshot_greeks_first_order => option_snapshot_greeks_first_order, greeks_tick_to_json(symbol, expiration, strike, right)
}

// 27. option_snapshot_greeks_second_order
ffi_parsed_endpoint! {
    /// Get second-order Greeks snapshot. Returns JSON array.
    tdx_option_snapshot_greeks_second_order => option_snapshot_greeks_second_order, greeks_tick_to_json(symbol, expiration, strike, right)
}

// 28. option_snapshot_greeks_third_order
ffi_parsed_endpoint! {
    /// Get third-order Greeks snapshot. Returns JSON array.
    tdx_option_snapshot_greeks_third_order => option_snapshot_greeks_third_order, greeks_tick_to_json(symbol, expiration, strike, right)
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
ffi_parsed_endpoint! {
    /// Fetch combined trade + quote ticks for an option contract. Returns JSON array.
    tdx_option_history_trade_quote => option_history_trade_quote, trade_quote_tick_to_json(symbol, expiration, strike, right, date)
}

// 34. option_history_open_interest
ffi_parsed_endpoint! {
    /// Fetch open interest history for an option contract. Returns JSON array.
    tdx_option_history_open_interest => option_history_open_interest, open_interest_tick_to_json(symbol, expiration, strike, right, date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — History Greeks endpoints (11)
// ═══════════════════════════════════════════════════════════════════════

// 35. option_history_greeks_eod
ffi_parsed_endpoint! {
    /// Fetch EOD Greeks history. Returns JSON array.
    tdx_option_history_greeks_eod => option_history_greeks_eod, greeks_tick_to_json(symbol, expiration, strike, right, start_date, end_date)
}

// 36. option_history_greeks_all
ffi_parsed_endpoint! {
    /// Fetch all Greeks history (intraday). Returns JSON array.
    tdx_option_history_greeks_all => option_history_greeks_all, greeks_tick_to_json(symbol, expiration, strike, right, date, interval)
}

// 37. option_history_trade_greeks_all
ffi_parsed_endpoint! {
    /// Fetch all Greeks on each trade. Returns JSON array.
    tdx_option_history_trade_greeks_all => option_history_trade_greeks_all, greeks_tick_to_json(symbol, expiration, strike, right, date)
}

// 38. option_history_greeks_first_order
ffi_parsed_endpoint! {
    /// Fetch first-order Greeks history. Returns JSON array.
    tdx_option_history_greeks_first_order => option_history_greeks_first_order, greeks_tick_to_json(symbol, expiration, strike, right, date, interval)
}

// 39. option_history_trade_greeks_first_order
ffi_parsed_endpoint! {
    /// Fetch first-order Greeks on each trade. Returns JSON array.
    tdx_option_history_trade_greeks_first_order => option_history_trade_greeks_first_order, greeks_tick_to_json(symbol, expiration, strike, right, date)
}

// 40. option_history_greeks_second_order
ffi_parsed_endpoint! {
    /// Fetch second-order Greeks history. Returns JSON array.
    tdx_option_history_greeks_second_order => option_history_greeks_second_order, greeks_tick_to_json(symbol, expiration, strike, right, date, interval)
}

// 41. option_history_trade_greeks_second_order
ffi_parsed_endpoint! {
    /// Fetch second-order Greeks on each trade. Returns JSON array.
    tdx_option_history_trade_greeks_second_order => option_history_trade_greeks_second_order, greeks_tick_to_json(symbol, expiration, strike, right, date)
}

// 42. option_history_greeks_third_order
ffi_parsed_endpoint! {
    /// Fetch third-order Greeks history. Returns JSON array.
    tdx_option_history_greeks_third_order => option_history_greeks_third_order, greeks_tick_to_json(symbol, expiration, strike, right, date, interval)
}

// 43. option_history_trade_greeks_third_order
ffi_parsed_endpoint! {
    /// Fetch third-order Greeks on each trade. Returns JSON array.
    tdx_option_history_trade_greeks_third_order => option_history_trade_greeks_third_order, greeks_tick_to_json(symbol, expiration, strike, right, date)
}

// 44. option_history_greeks_implied_volatility
ffi_parsed_endpoint! {
    /// Fetch IV history (intraday). Returns JSON array.
    tdx_option_history_greeks_implied_volatility => option_history_greeks_implied_volatility, iv_tick_to_json(symbol, expiration, strike, right, date, interval)
}

// 45. option_history_trade_greeks_implied_volatility
ffi_parsed_endpoint! {
    /// Fetch IV on each trade. Returns JSON array.
    tdx_option_history_trade_greeks_implied_volatility => option_history_trade_greeks_implied_volatility, iv_tick_to_json(symbol, expiration, strike, right, date)
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
ffi_snapshot_endpoint! {
    /// Get latest price snapshot for indices. Returns JSON array.
    tdx_index_snapshot_price => index_snapshot_price, price_tick_to_json
}

// 52. index_snapshot_market_value
ffi_snapshot_endpoint! {
    /// Get latest market value snapshot for indices. Returns JSON array.
    tdx_index_snapshot_market_value => index_snapshot_market_value, market_value_tick_to_json
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
ffi_parsed_endpoint! {
    /// Fetch intraday price history for an index. Returns JSON array.
    tdx_index_history_price => index_history_price, price_tick_to_json(symbol, date, interval)
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — At-Time endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

// 56. index_at_time_price
ffi_parsed_endpoint! {
    /// Fetch index price at a specific time across a date range. Returns JSON array.
    tdx_index_at_time_price => index_at_time_price, price_tick_to_json(symbol, start_date, end_date, time_of_day)
}

// ═══════════════════════════════════════════════════════════════════════
//  Calendar endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

// 57. calendar_open_today
ffi_parsed_endpoint_no_params! {
    /// Check whether the market is open today. Returns JSON array.
    tdx_calendar_open_today => calendar_open_today, calendar_day_to_json
}

// 58. calendar_on_date
ffi_parsed_endpoint! {
    /// Get calendar information for a specific date. Returns JSON array.
    tdx_calendar_on_date => calendar_on_date, calendar_day_to_json(date)
}

// 59. calendar_year
ffi_parsed_endpoint! {
    /// Get calendar information for an entire year. Returns JSON array.
    tdx_calendar_year => calendar_year, calendar_day_to_json(year)
}

// ═══════════════════════════════════════════════════════════════════════
//  Interest Rate endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

// 60. interest_rate_history_eod
ffi_parsed_endpoint! {
    /// Fetch EOD interest rate history. Returns JSON array.
    tdx_interest_rate_history_eod => interest_rate_history_eod, interest_rate_tick_to_json(symbol, start_date, end_date)
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

// ═══════════════════════════════════════════════════════════════════════
//  Unified client — historical + streaming through one handle
// ═══════════════════════════════════════════════════════════════════════

/// Connect to ThetaData (historical only — FPSS streaming is NOT started).
///
/// Authenticates once, opens gRPC channel. Call `tdx_unified_start_streaming()`
/// later to start FPSS. Historical endpoints are available immediately.
///
/// Returns null on connection/auth failure (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_connect(
    creds: *const TdxCredentials,
    config: *const TdxConfig,
) -> *mut TdxUnified {
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

    match runtime().block_on(thetadatadx::ThetaDataDx::connect(
        &creds.inner,
        config.inner.clone(),
    )) {
        Ok(tdx) => Box::into_raw(Box::new(TdxUnified {
            inner: tdx,
            rx: Mutex::new(None),
        })),
        Err(e) => {
            set_error(&e.to_string());
            ptr::null_mut()
        }
    }
}

/// Start FPSS streaming on the unified client.
///
/// Creates an internal mpsc channel and registers a callback handler.
/// Events are buffered — poll with `tdx_unified_next_event()`.
///
/// Returns 0 on success, -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_start_streaming(handle: *const TdxUnified) -> i32 {
    if handle.is_null() {
        set_error("unified handle is null");
        return -1;
    }
    let handle = unsafe { &*handle };

    let (tx, rx) = std::sync::mpsc::channel::<FfiBufferedEvent>();

    match handle
        .inner
        .start_streaming(move |event: &thetadatadx::fpss::FpssEvent| {
            let buffered = fpss_event_to_ffi(event);
            let _ = tx.send(buffered);
        }) {
        Ok(()) => {
            if let Ok(mut guard) = handle.rx.lock() {
                *guard = Some(Arc::new(Mutex::new(rx)));
            }
            0
        }
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Start FPSS streaming with OHLCVC derivation disabled.
///
/// Returns 0 on success, -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_start_streaming_no_ohlcvc(handle: *const TdxUnified) -> i32 {
    if handle.is_null() {
        set_error("unified handle is null");
        return -1;
    }
    let handle = unsafe { &*handle };

    let (tx, rx) = std::sync::mpsc::channel::<FfiBufferedEvent>();

    match handle
        .inner
        .start_streaming_no_ohlcvc(move |event: &thetadatadx::fpss::FpssEvent| {
            let buffered = fpss_event_to_ffi(event);
            let _ = tx.send(buffered);
        }) {
        Ok(()) => {
            if let Ok(mut guard) = handle.rx.lock() {
                *guard = Some(Arc::new(Mutex::new(rx)));
            }
            0
        }
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Subscribe to quote data for a stock symbol via the unified client.
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_quotes(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("unified handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null or invalid UTF-8");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match handle.inner.subscribe_quotes(&contract) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Subscribe to trade data for a stock symbol via the unified client.
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_trades(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("unified handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null or invalid UTF-8");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match handle.inner.subscribe_trades(&contract) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Unsubscribe from quote data for a stock symbol via the unified client.
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_quotes(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("unified handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null or invalid UTF-8");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match handle.inner.unsubscribe_quotes(&contract) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Unsubscribe from trade data for a stock symbol via the unified client.
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_trades(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("unified handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null or invalid UTF-8");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match handle.inner.unsubscribe_trades(&contract) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Subscribe to open interest data for a stock symbol on the unified client.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_open_interest(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("unified handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match handle.inner.subscribe_open_interest(&contract) {
        Ok(id) => id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Subscribe to all trades for a security type on the unified client.
/// sec_type: "STOCK", "OPTION", or "INDEX".
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_full_trades(
    handle: *const TdxUnified,
    sec_type: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("unified handle is null");
        return -1;
    }
    let sec_type_str = match unsafe { cstr_to_str(sec_type) } {
        Some(s) => s,
        None => {
            set_error("sec_type is null");
            return -1;
        }
    };
    let st = match sec_type_str.to_uppercase().as_str() {
        "STOCK" => thetadatadx::types::enums::SecType::Stock,
        "OPTION" => thetadatadx::types::enums::SecType::Option,
        "INDEX" => thetadatadx::types::enums::SecType::Index,
        _ => {
            set_error("invalid sec_type: expected STOCK, OPTION, or INDEX");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    match handle.inner.subscribe_full_trades(st) {
        Ok(id) => id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Unsubscribe from open interest data on the unified client.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_open_interest(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("unified handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match handle.inner.unsubscribe_open_interest(&contract) {
        Ok(id) => id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Check if streaming is active on the unified client.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_is_streaming(handle: *const TdxUnified) -> i32 {
    if handle.is_null() {
        return 0;
    }
    let handle = unsafe { &*handle };
    if handle.inner.is_streaming() {
        1
    } else {
        0
    }
}

/// Look up a contract by ID. Returns JSON string or null.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_contract_lookup(
    handle: *const TdxUnified,
    id: i32,
) -> *mut c_char {
    if handle.is_null() {
        set_error("unified handle is null");
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    match handle.inner.contract_lookup(id) {
        Ok(Some(c)) => match CString::new(format!("{c}")) {
            Ok(s) => s.into_raw(),
            Err(_) => ptr::null_mut(),
        },
        Ok(None) => ptr::null_mut(),
        Err(e) => {
            set_error(&e.to_string());
            ptr::null_mut()
        }
    }
}

/// Get active subscriptions as JSON array. Returns null on error.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_active_subscriptions(
    handle: *const TdxUnified,
) -> *mut c_char {
    if handle.is_null() {
        set_error("unified handle is null");
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    match handle.inner.active_subscriptions() {
        Ok(subs) => {
            let json = serde_json::Value::Array(
                subs.iter()
                    .map(|(k, c)| {
                        serde_json::json!({
                            "kind": format!("{k:?}"),
                            "contract": format!("{c}"),
                        })
                    })
                    .collect(),
            );
            json_to_cstring(&json)
        }
        Err(e) => {
            set_error(&e.to_string());
            ptr::null_mut()
        }
    }
}

/// Poll for the next streaming event from the unified client.
///
/// Blocks for up to `timeout_ms` milliseconds. Returns a JSON string.
/// Returns null if no event arrived within the timeout (NOT an error),
/// or if streaming has not been started yet (check `tdx_last_error()`).
/// Caller must free the returned string with `tdx_string_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_next_event(
    handle: *const TdxUnified,
    timeout_ms: u64,
) -> *mut c_char {
    if handle.is_null() {
        set_error("unified handle is null");
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let rx_guard = handle.rx.lock().unwrap_or_else(|e| e.into_inner());
    let rx_arc = match rx_guard.as_ref() {
        Some(arc) => Arc::clone(arc),
        None => {
            set_error("streaming not started -- call tdx_unified_start_streaming() first");
            return ptr::null_mut();
        }
    };
    drop(rx_guard);
    let rx = rx_arc.lock().unwrap_or_else(|e| e.into_inner());
    let timeout = std::time::Duration::from_millis(timeout_ms);
    match rx.recv_timeout(timeout) {
        Ok(event) => buffered_event_to_cstring(&event),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ptr::null_mut(),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => ptr::null_mut(),
    }
}

/// Borrow the historical client from a unified handle.
///
/// Returns a `*const TdxClient` that can be passed to all `tdx_stock_*`,
/// `tdx_option_*`, `tdx_index_*`, `tdx_calendar_*`, and `tdx_interest_rate_*`
/// functions. This avoids a second `tdx_client_connect()` call and reuses the
/// same authenticated session.
///
/// The returned pointer is **NOT owned** -- do NOT call `tdx_client_free` on it.
/// It is valid as long as the `TdxUnified` handle is alive.
///
/// # Safety
///
/// This cast is sound because `TdxClient` is `#[repr(transparent)]` over
/// `DirectClient`, and `ThetaDataDx` Derefs to `&DirectClient`.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_historical(handle: *const TdxUnified) -> *const TdxClient {
    if handle.is_null() {
        set_error("unified handle is null");
        return ptr::null();
    }
    let handle = unsafe { &*handle };
    // TdxClient is #[repr(transparent)] over DirectClient, so this cast is safe.
    let direct_ref: &thetadatadx::direct::DirectClient = &handle.inner;
    direct_ref as *const thetadatadx::direct::DirectClient as *const TdxClient
}

/// Stop streaming on the unified client. Historical remains available.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_stop_streaming(handle: *const TdxUnified) {
    if handle.is_null() {
        return;
    }
    let handle = unsafe { &*handle };
    handle.inner.stop_streaming();
    // Clear the rx so next_event knows streaming is stopped.
    if let Ok(mut guard) = handle.rx.lock() {
        *guard = None;
    }
}

/// Free a unified client handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_free(handle: *mut TdxUnified) {
    if !handle.is_null() {
        let handle = unsafe { Box::from_raw(handle) };
        handle.inner.stop_streaming();
        drop(handle);
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  FPSS — Real-time streaming client
// ═══════════════════════════════════════════════════════════════════════

/// Connect to FPSS streaming servers.
///
/// Events are collected in an internal queue. Call `tdx_fpss_next_event()` to poll.
///
/// Returns null on connection failure (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_connect(
    creds: *const TdxCredentials,
    config: *const TdxConfig,
) -> *mut TdxFpssHandle {
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

    let (tx, rx) = std::sync::mpsc::channel::<FfiBufferedEvent>();

    let client = match thetadatadx::fpss::FpssClient::connect(
        &creds.inner,
        config.inner.fpss_ring_size,
        move |event: &thetadatadx::fpss::FpssEvent| {
            let buffered = fpss_event_to_ffi(event);
            let _ = tx.send(buffered);
        },
    ) {
        Ok(c) => c,
        Err(e) => {
            set_error(&e.to_string());
            return ptr::null_mut();
        }
    };

    Box::into_raw(Box::new(TdxFpssHandle {
        inner: Arc::new(Mutex::new(Some(client))),
        rx: Arc::new(Mutex::new(rx)),
    }))
}

/// Subscribe to quote data for a stock symbol.
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_quotes(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null or invalid UTF-8");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    let client = match guard.as_ref() {
        Some(c) => c,
        None => {
            set_error("FPSS client is shut down");
            return -1;
        }
    };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match client.subscribe_quotes(&contract) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Subscribe to trade data for a stock symbol.
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_trades(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null or invalid UTF-8");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    let client = match guard.as_ref() {
        Some(c) => c,
        None => {
            set_error("FPSS client is shut down");
            return -1;
        }
    };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match client.subscribe_trades(&contract) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Unsubscribe from quote data for a stock symbol.
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_quotes(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null or invalid UTF-8");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    let client = match guard.as_ref() {
        Some(c) => c,
        None => {
            set_error("FPSS client is shut down");
            return -1;
        }
    };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match client.unsubscribe_quotes(&contract) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Unsubscribe from trade data for a stock symbol.
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_trades(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null or invalid UTF-8");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    let client = match guard.as_ref() {
        Some(c) => c,
        None => {
            set_error("FPSS client is shut down");
            return -1;
        }
    };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match client.unsubscribe_trades(&contract) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Subscribe to open interest data for a stock symbol.
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_open_interest(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null or invalid UTF-8");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    let client = match guard.as_ref() {
        Some(c) => c,
        None => {
            set_error("FPSS client is shut down");
            return -1;
        }
    };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match client.subscribe_open_interest(&contract) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Unsubscribe from open interest data for a stock symbol.
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_open_interest(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return -1;
    }
    let symbol = match unsafe { cstr_to_str(symbol) } {
        Some(s) => s,
        None => {
            set_error("symbol is null or invalid UTF-8");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    let client = match guard.as_ref() {
        Some(c) => c,
        None => {
            set_error("FPSS client is shut down");
            return -1;
        }
    };
    let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
    match client.unsubscribe_open_interest(&contract) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Subscribe to all trades for a security type (full trade stream).
///
/// `sec_type` must be one of: "STOCK", "OPTION", "INDEX".
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_full_trades(
    handle: *const TdxFpssHandle,
    sec_type: *const c_char,
) -> i32 {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return -1;
    }
    let sec_type_str = match unsafe { cstr_to_str(sec_type) } {
        Some(s) => s,
        None => {
            set_error("sec_type is null or invalid UTF-8");
            return -1;
        }
    };
    let st = match sec_type_str.to_uppercase().as_str() {
        "STOCK" => thetadatadx::types::enums::SecType::Stock,
        "OPTION" => thetadatadx::types::enums::SecType::Option,
        "INDEX" => thetadatadx::types::enums::SecType::Index,
        other => {
            set_error(&format!(
                "unknown sec_type: {other:?} (expected STOCK, OPTION, or INDEX)"
            ));
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    let guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    let client = match guard.as_ref() {
        Some(c) => c,
        None => {
            set_error("FPSS client is shut down");
            return -1;
        }
    };
    match client.subscribe_full_trades(st) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Check if the FPSS client is currently authenticated.
///
/// Returns 1 if authenticated, 0 if not (or if handle is null).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_is_authenticated(handle: *const TdxFpssHandle) -> i32 {
    if handle.is_null() {
        return 0;
    }
    let handle = unsafe { &*handle };
    let guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(c) => {
            if c.is_authenticated() {
                1
            } else {
                0
            }
        }
        None => 0,
    }
}

/// Look up a single contract by its server-assigned ID.
///
/// Returns a JSON string representation of the contract, or NULL if not found.
/// Caller must free the returned string with `tdx_string_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_contract_lookup(
    handle: *const TdxFpssHandle,
    id: i32,
) -> *mut c_char {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    let client = match guard.as_ref() {
        Some(c) => c,
        None => {
            set_error("FPSS client is shut down");
            return ptr::null_mut();
        }
    };
    match client.contract_lookup(id) {
        Some(contract) => {
            let s = format!("{contract}");
            match CString::new(s) {
                Ok(cs) => cs.into_raw(),
                Err(_) => ptr::null_mut(),
            }
        }
        None => ptr::null_mut(),
    }
}

/// Get a snapshot of currently active subscriptions.
///
/// Returns a JSON array of objects with "kind" and "contract" keys.
/// Caller must free the returned string with `tdx_string_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_active_subscriptions(
    handle: *const TdxFpssHandle,
) -> *mut c_char {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    let client = match guard.as_ref() {
        Some(c) => c,
        None => {
            set_error("FPSS client is shut down");
            return ptr::null_mut();
        }
    };
    let subs = client.active_subscriptions();
    let json_array: Vec<serde_json::Value> = subs
        .into_iter()
        .map(|(kind, contract)| {
            serde_json::json!({
                "kind": format!("{kind:?}"),
                "contract": format!("{contract}"),
            })
        })
        .collect();
    let json_str = serde_json::to_string(&json_array).unwrap_or_else(|_| "[]".to_string());
    match CString::new(json_str) {
        Ok(cs) => cs.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Poll for the next FPSS event.
///
/// Blocks for up to `timeout_ms` milliseconds. Returns a JSON string with keys
/// `kind`, `payload_hex` (hex-encoded binary), `detail`, and `id`.
///
/// Returns null if no event arrived within the timeout (this is NOT an error).
/// Caller must free the returned string with `tdx_string_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_next_event(
    handle: *const TdxFpssHandle,
    timeout_ms: u64,
) -> *mut c_char {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let rx = handle.rx.lock().unwrap_or_else(|e| e.into_inner());
    let timeout = std::time::Duration::from_millis(timeout_ms);
    match rx.recv_timeout(timeout) {
        Ok(event) => buffered_event_to_cstring(&event),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ptr::null_mut(),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => ptr::null_mut(),
    }
}

/// Shut down the FPSS client, stopping all background threads.
///
/// The handle remains valid for `tdx_fpss_free()` but all subsequent operations
/// will return errors.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_shutdown(handle: *const TdxFpssHandle) {
    if handle.is_null() {
        return;
    }
    let handle = unsafe { &*handle };
    let mut guard = handle.inner.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(client) = guard.take() {
        client.shutdown();
    }
}

/// Free a FPSS handle. Must be called after `tdx_fpss_shutdown()`.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_free(handle: *mut TdxFpssHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}
