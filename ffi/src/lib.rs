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
//! - Returned typed arrays are heap-allocated and must be freed with the
//!   corresponding `tdx_*_free` function.
//! - Functions are not thread-safe on the same handle; callers must synchronize.
//!
//! # Memory model
//!
//! - Opaque handles (`*mut TdxClient`, `*mut TdxCredentials`, etc.) are heap-allocated
//!   via `Box::into_raw` and freed via the corresponding `tdx_*_free` function.
//! - Tick arrays are returned as `#[repr(C)]` structs with a `data` pointer and `len`.
//!   They MUST be freed with the corresponding `tdx_*_array_free` function.
//! - String arrays (`TdxStringArray`) must be freed with `tdx_string_array_free`.
//! - The caller MUST free every non-null pointer / non-empty array returned by this library.
//!
//! # Error handling
//!
//! Functions that can fail return an empty array (data=null, len=0) on error and set
//! a thread-local error string retrievable via `tdx_last_error`.

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
/// a timeout, returning a `*mut TdxFpssEvent` typed struct.
pub struct TdxFpssHandle {
    inner: Arc<Mutex<Option<thetadatadx::fpss::FpssClient>>>,
    rx: Arc<Mutex<std::sync::mpsc::Receiver<FfiBufferedEvent>>>,
}

// ═══════════════════════════════════════════════════════════════════════
//  #[repr(C)] FPSS streaming event types — zero-copy across FFI
// ═══════════════════════════════════════════════════════════════════════

/// FPSS event kind tag. Check this to determine which field of
/// `TdxFpssEvent` is valid.
#[repr(C)]
pub enum TdxFpssEventKind {
    Quote = 0,
    Trade = 1,
    OpenInterest = 2,
    Ohlcvc = 3,
    Control = 4,
    RawData = 5,
}

/// `#[repr(C)]` FPSS quote event.
///
/// `bid_f64` / `ask_f64` are pre-decoded from the raw integer + price_type.
#[repr(C)]
pub struct TdxFpssQuote {
    pub contract_id: i32,
    pub ms_of_day: i32,
    pub bid_size: i32,
    pub bid_exchange: i32,
    pub bid: i32,
    pub bid_f64: f64,
    pub bid_condition: i32,
    pub ask_size: i32,
    pub ask_exchange: i32,
    pub ask: i32,
    pub ask_f64: f64,
    pub ask_condition: i32,
    pub price_type: i32,
    pub date: i32,
    pub received_at_ns: u64,
}

/// `#[repr(C)]` FPSS trade event.
///
/// `price_f64` is pre-decoded from the raw integer + price_type.
#[repr(C)]
pub struct TdxFpssTrade {
    pub contract_id: i32,
    pub ms_of_day: i32,
    pub sequence: i32,
    pub ext_condition1: i32,
    pub ext_condition2: i32,
    pub ext_condition3: i32,
    pub ext_condition4: i32,
    pub condition: i32,
    pub size: i32,
    pub exchange: i32,
    pub price: i32,
    pub price_f64: f64,
    pub condition_flags: i32,
    pub price_flags: i32,
    pub volume_type: i32,
    pub records_back: i32,
    pub price_type: i32,
    pub date: i32,
    pub received_at_ns: u64,
}

/// `#[repr(C)]` FPSS open interest event.
#[repr(C)]
pub struct TdxFpssOpenInterest {
    pub contract_id: i32,
    pub ms_of_day: i32,
    pub open_interest: i32,
    pub date: i32,
    pub received_at_ns: u64,
}

/// `#[repr(C)]` FPSS OHLCVC event.
///
/// `volume` and `count` are `i64` to match the core crate and prevent
/// overflow on high-volume symbols.
///
/// `open_f64` / `high_f64` / `low_f64` / `close_f64` are pre-decoded
/// from the raw integers + price_type.
#[repr(C)]
pub struct TdxFpssOhlcvc {
    pub contract_id: i32,
    pub ms_of_day: i32,
    pub open: i32,
    pub open_f64: f64,
    pub high: i32,
    pub high_f64: f64,
    pub low: i32,
    pub low_f64: f64,
    pub close: i32,
    pub close_f64: f64,
    pub volume: i64,
    pub count: i64,
    pub price_type: i32,
    pub date: i32,
    pub received_at_ns: u64,
}

/// `#[repr(C)]` FPSS control event.
///
/// `kind` encodes the control sub-type:
///   0=login_success, 1=contract_assigned, 2=req_response,
///   3=market_open, 4=market_close, 5=server_error,
///   6=disconnected, 7=error
///
/// `id` carries the contract_id or req_id where applicable (0 otherwise).
/// `detail` is a NUL-terminated C string (may be null).
#[repr(C)]
pub struct TdxFpssControl {
    pub kind: i32,
    pub id: i32,
    pub detail: *const c_char,
}

/// `#[repr(C)]` FPSS raw/undecoded data event.
///
/// `code` is the wire message code. `payload` is a pointer to the raw
/// bytes and `payload_len` is the number of bytes.
#[repr(C)]
pub struct TdxFpssRawData {
    pub code: u8,
    pub payload: *const u8,
    pub payload_len: usize,
}

/// Tagged FPSS event for FFI. Check `kind` then read the corresponding
/// field. Only the field matching `kind` contains valid data — this is
/// a flat struct (not a C union) for simplicity and safety.
#[repr(C)]
pub struct TdxFpssEvent {
    pub kind: TdxFpssEventKind,
    pub quote: TdxFpssQuote,
    pub trade: TdxFpssTrade,
    pub open_interest: TdxFpssOpenInterest,
    pub ohlcvc: TdxFpssOhlcvc,
    pub control: TdxFpssControl,
    pub raw_data: TdxFpssRawData,
}

/// Internal buffered event — owns heap data that backs the `TdxFpssEvent`.
///
/// `#[repr(C)]` guarantees `event` is at offset 0 so that casting
/// `*mut FfiBufferedEvent` to `*mut TdxFpssEvent` is sound. The field
/// is read through that pointer cast (not via `.event`), which the
/// compiler cannot see — hence `pub(crate)`.
///
/// `_detail_string` and `_raw_payload` own the backing memory for
/// pointer fields inside `event.control.detail` and
/// `event.raw_data.payload` respectively.
#[repr(C)]
struct FfiBufferedEvent {
    pub(crate) event: TdxFpssEvent,
    /// Owns the CString backing `event.control.detail`, if any.
    _detail_string: Option<CString>,
    /// Owns the raw payload bytes backing `event.raw_data.payload`, if any.
    _raw_payload: Option<Vec<u8>>,
}

// SAFETY: FfiBufferedEvent is sent across std::sync::mpsc channels.
// The owned data (_detail_string, _raw_payload) is heap-allocated and
// the pointers inside `event` point into that owned data. The event is
// only accessed from the receiving thread after the send completes.
unsafe impl Send for FfiBufferedEvent {}

/// Zero-initialized defaults for inactive union-style fields.
const ZERO_QUOTE: TdxFpssQuote = TdxFpssQuote {
    contract_id: 0,
    ms_of_day: 0,
    bid_size: 0,
    bid_exchange: 0,
    bid: 0,
    bid_f64: 0.0,
    bid_condition: 0,
    ask_size: 0,
    ask_exchange: 0,
    ask: 0,
    ask_f64: 0.0,
    ask_condition: 0,
    price_type: 0,
    date: 0,
    received_at_ns: 0,
};
const ZERO_TRADE: TdxFpssTrade = TdxFpssTrade {
    contract_id: 0,
    ms_of_day: 0,
    sequence: 0,
    ext_condition1: 0,
    ext_condition2: 0,
    ext_condition3: 0,
    ext_condition4: 0,
    condition: 0,
    size: 0,
    exchange: 0,
    price: 0,
    price_f64: 0.0,
    condition_flags: 0,
    price_flags: 0,
    volume_type: 0,
    records_back: 0,
    price_type: 0,
    date: 0,
    received_at_ns: 0,
};
const ZERO_OI: TdxFpssOpenInterest = TdxFpssOpenInterest {
    contract_id: 0,
    ms_of_day: 0,
    open_interest: 0,
    date: 0,
    received_at_ns: 0,
};
const ZERO_OHLCVC: TdxFpssOhlcvc = TdxFpssOhlcvc {
    contract_id: 0,
    ms_of_day: 0,
    open: 0,
    open_f64: 0.0,
    high: 0,
    high_f64: 0.0,
    low: 0,
    low_f64: 0.0,
    close: 0,
    close_f64: 0.0,
    volume: 0,
    count: 0,
    price_type: 0,
    date: 0,
    received_at_ns: 0,
};
const ZERO_CONTROL: TdxFpssControl = TdxFpssControl {
    kind: 0,
    id: 0,
    detail: ptr::null(),
};
const ZERO_RAW: TdxFpssRawData = TdxFpssRawData {
    code: 0,
    payload: ptr::null(),
    payload_len: 0,
};

fn fpss_event_to_ffi(event: &thetadatadx::fpss::FpssEvent) -> FfiBufferedEvent {
    use thetadatadx::fpss::{FpssControl, FpssData, FpssEvent};

    fn price_f64(value: i32, price_type: i32) -> f64 {
        tdbe::types::price::Price::new(value, price_type).to_f64()
    }

    match event {
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
            received_at_ns,
            ..
        }) => FfiBufferedEvent {
            event: TdxFpssEvent {
                kind: TdxFpssEventKind::Quote,
                quote: TdxFpssQuote {
                    contract_id: *contract_id,
                    ms_of_day: *ms_of_day,
                    bid_size: *bid_size,
                    bid_exchange: *bid_exchange,
                    bid: *bid,
                    bid_f64: price_f64(*bid, *price_type),
                    bid_condition: *bid_condition,
                    ask_size: *ask_size,
                    ask_exchange: *ask_exchange,
                    ask: *ask,
                    ask_f64: price_f64(*ask, *price_type),
                    ask_condition: *ask_condition,
                    price_type: *price_type,
                    date: *date,
                    received_at_ns: *received_at_ns,
                },
                trade: ZERO_TRADE,
                open_interest: ZERO_OI,
                ohlcvc: ZERO_OHLCVC,
                control: ZERO_CONTROL,
                raw_data: ZERO_RAW,
            },
            _detail_string: None,
            _raw_payload: None,
        },

        FpssEvent::Data(FpssData::Trade {
            contract_id,
            ms_of_day,
            sequence,
            ext_condition1,
            ext_condition2,
            ext_condition3,
            ext_condition4,
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
            received_at_ns,
            ..
        }) => FfiBufferedEvent {
            event: TdxFpssEvent {
                kind: TdxFpssEventKind::Trade,
                trade: TdxFpssTrade {
                    contract_id: *contract_id,
                    ms_of_day: *ms_of_day,
                    sequence: *sequence,
                    ext_condition1: *ext_condition1,
                    ext_condition2: *ext_condition2,
                    ext_condition3: *ext_condition3,
                    ext_condition4: *ext_condition4,
                    condition: *condition,
                    size: *size,
                    exchange: *exchange,
                    price: *price,
                    price_f64: price_f64(*price, *price_type),
                    condition_flags: *condition_flags,
                    price_flags: *price_flags,
                    volume_type: *volume_type,
                    records_back: *records_back,
                    price_type: *price_type,
                    date: *date,
                    received_at_ns: *received_at_ns,
                },
                quote: ZERO_QUOTE,
                open_interest: ZERO_OI,
                ohlcvc: ZERO_OHLCVC,
                control: ZERO_CONTROL,
                raw_data: ZERO_RAW,
            },
            _detail_string: None,
            _raw_payload: None,
        },

        FpssEvent::Data(FpssData::OpenInterest {
            contract_id,
            ms_of_day,
            open_interest,
            date,
            received_at_ns,
        }) => FfiBufferedEvent {
            event: TdxFpssEvent {
                kind: TdxFpssEventKind::OpenInterest,
                open_interest: TdxFpssOpenInterest {
                    contract_id: *contract_id,
                    ms_of_day: *ms_of_day,
                    open_interest: *open_interest,
                    date: *date,
                    received_at_ns: *received_at_ns,
                },
                quote: ZERO_QUOTE,
                trade: ZERO_TRADE,
                ohlcvc: ZERO_OHLCVC,
                control: ZERO_CONTROL,
                raw_data: ZERO_RAW,
            },
            _detail_string: None,
            _raw_payload: None,
        },

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
            received_at_ns,
            ..
        }) => FfiBufferedEvent {
            event: TdxFpssEvent {
                kind: TdxFpssEventKind::Ohlcvc,
                ohlcvc: TdxFpssOhlcvc {
                    contract_id: *contract_id,
                    ms_of_day: *ms_of_day,
                    open: *open,
                    open_f64: price_f64(*open, *price_type),
                    high: *high,
                    high_f64: price_f64(*high, *price_type),
                    low: *low,
                    low_f64: price_f64(*low, *price_type),
                    close: *close,
                    close_f64: price_f64(*close, *price_type),
                    volume: *volume,
                    count: *count,
                    price_type: *price_type,
                    date: *date,
                    received_at_ns: *received_at_ns,
                },
                quote: ZERO_QUOTE,
                trade: ZERO_TRADE,
                open_interest: ZERO_OI,
                control: ZERO_CONTROL,
                raw_data: ZERO_RAW,
            },
            _detail_string: None,
            _raw_payload: None,
        },

        FpssEvent::RawData { code, payload } => {
            let owned = payload.clone();
            let ptr = owned.as_ptr();
            let len = owned.len();
            FfiBufferedEvent {
                event: TdxFpssEvent {
                    kind: TdxFpssEventKind::RawData,
                    raw_data: TdxFpssRawData {
                        code: *code,
                        payload: ptr,
                        payload_len: len,
                    },
                    quote: ZERO_QUOTE,
                    trade: ZERO_TRADE,
                    open_interest: ZERO_OI,
                    ohlcvc: ZERO_OHLCVC,
                    control: ZERO_CONTROL,
                },
                _detail_string: None,
                _raw_payload: Some(owned),
            }
        }

        FpssEvent::Control(ctrl) => {
            let (kind, id, detail_str) = match ctrl {
                FpssControl::LoginSuccess { permissions } => (0, 0, Some(permissions.clone())),
                FpssControl::ContractAssigned { id, contract } => {
                    (1, *id, Some(format!("{contract}")))
                }
                FpssControl::ReqResponse { req_id, result } => {
                    (2, *req_id, Some(format!("{result:?}")))
                }
                FpssControl::MarketOpen => (3, 0, None),
                FpssControl::MarketClose => (4, 0, None),
                FpssControl::ServerError { message } => (5, 0, Some(message.clone())),
                FpssControl::Disconnected { reason } => (6, 0, Some(format!("{reason:?}"))),
                FpssControl::Reconnecting {
                    reason,
                    attempt,
                    delay_ms,
                } => (
                    8,
                    *attempt as i32,
                    Some(format!("{reason:?} delay={delay_ms}ms")),
                ),
                FpssControl::Reconnected => (9, 0, None),
                FpssControl::Error { message } => (10, 0, Some(message.clone())),
                _ => (99, 0, None), // unknown control
            };
            let cstring = detail_str.and_then(|s| CString::new(s).ok());
            let detail_ptr = cstring.as_ref().map_or(ptr::null(), |cs| cs.as_ptr());
            FfiBufferedEvent {
                event: TdxFpssEvent {
                    kind: TdxFpssEventKind::Control,
                    control: TdxFpssControl {
                        kind,
                        id,
                        detail: detail_ptr,
                    },
                    quote: ZERO_QUOTE,
                    trade: ZERO_TRADE,
                    open_interest: ZERO_OI,
                    ohlcvc: ZERO_OHLCVC,
                    raw_data: ZERO_RAW,
                },
                _detail_string: cstring,
                _raw_payload: None,
            }
        }

        _ => {
            // Empty / unknown — surface as a control event with kind=8 (unknown)
            FfiBufferedEvent {
                event: TdxFpssEvent {
                    kind: TdxFpssEventKind::Control,
                    control: TdxFpssControl {
                        kind: 8,
                        id: 0,
                        detail: ptr::null(),
                    },
                    quote: ZERO_QUOTE,
                    trade: ZERO_TRADE,
                    open_interest: ZERO_OI,
                    ohlcvc: ZERO_OHLCVC,
                    raw_data: ZERO_RAW,
                },
                _detail_string: None,
                _raw_payload: None,
            }
        }
    }
}

// ── Helper: C string to &str ──

unsafe fn cstr_to_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
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

/// Create a dev config (FPSS dev servers, port 20200, infinite replay).
#[no_mangle]
pub extern "C" fn tdx_config_dev() -> *mut TdxConfig {
    Box::into_raw(Box::new(TdxConfig {
        inner: thetadatadx::DirectConfig::dev(),
    }))
}

/// Create a stage config (FPSS stage servers, port 20100, unstable).
#[no_mangle]
pub extern "C" fn tdx_config_stage() -> *mut TdxConfig {
    Box::into_raw(Box::new(TdxConfig {
        inner: thetadatadx::DirectConfig::stage(),
    }))
}

/// Free a config handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_free(config: *mut TdxConfig) {
    if !config.is_null() {
        drop(unsafe { Box::from_raw(config) });
    }
}

/// Set FPSS flush mode on a config handle.
///
/// - `mode = 0`: Batched (default) -- flush only on PING every 100ms
/// - `mode = 1`: Immediate -- flush after every frame write (lowest latency)
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_flush_mode(config: *mut TdxConfig, mode: i32) {
    if config.is_null() {
        return;
    }
    let config = unsafe { &mut *config };
    config.inner.fpss_flush_mode = match mode {
        1 => thetadatadx::FpssFlushMode::Immediate,
        _ => thetadatadx::FpssFlushMode::Batched,
    };
}

/// Set FPSS reconnect policy on a config handle.
///
/// - `policy = 0`: Auto (default) -- auto-reconnect matching Java terminal behavior
/// - `policy = 1`: Manual -- no auto-reconnect, user calls reconnect explicitly
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_policy(config: *mut TdxConfig, policy: i32) {
    if config.is_null() {
        return;
    }
    let config = unsafe { &mut *config };
    config.inner.reconnect_policy = match policy {
        1 => thetadatadx::ReconnectPolicy::Manual,
        _ => thetadatadx::ReconnectPolicy::Auto,
    };
}

/// Set FPSS OHLCVC derivation on a config handle.
///
/// - `enabled = 1` (default): derive OHLCVC bars locally from trade events
/// - `enabled = 0`: only emit server-sent OHLCVC frames (lower overhead)
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_derive_ohlcvc(config: *mut TdxConfig, enabled: i32) {
    if config.is_null() {
        return;
    }
    let config = unsafe { &mut *config };
    config.inner.derive_ohlcvc = enabled != 0;
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

// ═══════════════════════════════════════════════════════════════════════
//  #[repr(C)] typed array types — zero-copy tick buffers for FFI
// ═══════════════════════════════════════════════════════════════════════

/// Generate a `#[repr(C)]` array wrapper for a tick type, plus a free function.
///
/// Each generated type has:
/// - `data`: pointer to the first element (null if empty)
/// - `len`: number of elements
/// - `from_vec()`: consumes a `Vec<T>` and returns the array
/// - `free()`: deallocates the backing memory
macro_rules! tick_array_type {
    ($name:ident, $tick:ty) => {
        /// Heap-allocated array of ticks returned from FFI.
        /// Caller MUST free with the corresponding `tdx_*_array_free` function.
        #[repr(C)]
        pub struct $name {
            pub data: *const $tick,
            pub len: usize,
        }

        impl $name {
            #[allow(dead_code)]
            fn from_vec(v: Vec<$tick>) -> Self {
                let len = v.len();
                if len == 0 {
                    return Self {
                        data: ptr::null(),
                        len: 0,
                    };
                }
                let boxed = v.into_boxed_slice();
                let data = Box::into_raw(boxed) as *const $tick;
                Self { data, len }
            }

            unsafe fn free(self) {
                if !self.data.is_null() && self.len > 0 {
                    let _ = unsafe {
                        Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                            self.data as *mut $tick,
                            self.len,
                        ))
                    };
                }
            }
        }
    };
}

tick_array_type!(TdxEodTickArray, tdbe::EodTick);
tick_array_type!(TdxOhlcTickArray, tdbe::OhlcTick);
tick_array_type!(TdxTradeTickArray, tdbe::TradeTick);
tick_array_type!(TdxQuoteTickArray, tdbe::QuoteTick);
tick_array_type!(TdxGreeksTickArray, tdbe::GreeksTick);
tick_array_type!(TdxIvTickArray, tdbe::IvTick);
tick_array_type!(TdxPriceTickArray, tdbe::PriceTick);
tick_array_type!(TdxOpenInterestTickArray, tdbe::OpenInterestTick);
tick_array_type!(TdxMarketValueTickArray, tdbe::MarketValueTick);
tick_array_type!(TdxCalendarDayArray, tdbe::CalendarDay);
tick_array_type!(TdxInterestRateTickArray, tdbe::InterestRateTick);
tick_array_type!(TdxSnapshotTradeTickArray, tdbe::SnapshotTradeTick);
tick_array_type!(TdxTradeQuoteTickArray, tdbe::TradeQuoteTick);

/// Generate a `#[no_mangle] extern "C"` free function for a tick array type.
macro_rules! tick_array_free {
    ($fn_name:ident, $array_type:ident) => {
        /// Free a tick array returned by an FFI endpoint.
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(arr: $array_type) {
            unsafe { arr.free() };
        }
    };
}

tick_array_free!(tdx_eod_tick_array_free, TdxEodTickArray);
tick_array_free!(tdx_ohlc_tick_array_free, TdxOhlcTickArray);
tick_array_free!(tdx_trade_tick_array_free, TdxTradeTickArray);
tick_array_free!(tdx_quote_tick_array_free, TdxQuoteTickArray);
tick_array_free!(tdx_greeks_tick_array_free, TdxGreeksTickArray);
tick_array_free!(tdx_iv_tick_array_free, TdxIvTickArray);
tick_array_free!(tdx_price_tick_array_free, TdxPriceTickArray);
tick_array_free!(tdx_open_interest_tick_array_free, TdxOpenInterestTickArray);
tick_array_free!(tdx_market_value_tick_array_free, TdxMarketValueTickArray);
tick_array_free!(tdx_calendar_day_array_free, TdxCalendarDayArray);
tick_array_free!(tdx_interest_rate_tick_array_free, TdxInterestRateTickArray);
tick_array_free!(
    tdx_snapshot_trade_tick_array_free,
    TdxSnapshotTradeTickArray
);
tick_array_free!(tdx_trade_quote_tick_array_free, TdxTradeQuoteTickArray);

// ═══════════════════════════════════════════════════════════════════════
//  OptionContract FFI type (String field requires special handling)
// ═══════════════════════════════════════════════════════════════════════

/// FFI-safe option contract descriptor.
///
/// The `root` field is a heap-allocated C string. Freed when the array is freed.
#[repr(C)]
pub struct TdxOptionContract {
    /// Heap-allocated NUL-terminated C string. Freed with the array.
    pub root: *const c_char,
    pub expiration: i32,
    pub strike: i32,
    pub right: i32,
    pub strike_price_type: i32,
}

/// Array of FFI-safe option contracts.
#[repr(C)]
pub struct TdxOptionContractArray {
    pub data: *const TdxOptionContract,
    pub len: usize,
}

impl TdxOptionContractArray {
    fn from_vec(contracts: Vec<tdbe::OptionContract>) -> Self {
        let len = contracts.len();
        if len == 0 {
            return Self {
                data: ptr::null(),
                len: 0,
            };
        }
        let ffi_contracts: Vec<TdxOptionContract> = contracts
            .into_iter()
            .map(|c| {
                let root = CString::new(c.root).unwrap_or_default();
                TdxOptionContract {
                    root: root.into_raw() as *const c_char,
                    expiration: c.expiration,
                    strike: c.strike,
                    right: c.right,
                    strike_price_type: c.strike_price_type,
                }
            })
            .collect();
        let boxed = ffi_contracts.into_boxed_slice();
        let data = Box::into_raw(boxed) as *const TdxOptionContract;
        Self { data, len }
    }
}

/// Free an option contract array, including all heap-allocated root strings.
#[no_mangle]
pub unsafe extern "C" fn tdx_option_contract_array_free(arr: TdxOptionContractArray) {
    if !arr.data.is_null() && arr.len > 0 {
        // First free each root C string
        let slice = unsafe { std::slice::from_raw_parts(arr.data, arr.len) };
        for contract in slice {
            if !contract.root.is_null() {
                drop(unsafe { CString::from_raw(contract.root as *mut c_char) });
            }
        }
        // Then free the array itself
        let _ = unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                arr.data as *mut TdxOptionContract,
                arr.len,
            ))
        };
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  TdxStringArray — for list endpoints returning Vec<String>
// ═══════════════════════════════════════════════════════════════════════

/// Array of heap-allocated C strings.
#[repr(C)]
pub struct TdxStringArray {
    /// Array of pointers to NUL-terminated C strings.
    pub data: *const *const c_char,
    pub len: usize,
}

impl TdxStringArray {
    fn from_vec(strings: Vec<String>) -> Self {
        let len = strings.len();
        if len == 0 {
            return Self {
                data: ptr::null(),
                len: 0,
            };
        }
        let cstrings: Vec<*const c_char> = strings
            .into_iter()
            .map(|s| CString::new(s).unwrap_or_default().into_raw() as *const c_char)
            .collect();
        let boxed = cstrings.into_boxed_slice();
        let data = Box::into_raw(boxed) as *const *const c_char;
        Self { data, len }
    }
}

/// Free a string array, including all individual C strings.
#[no_mangle]
pub unsafe extern "C" fn tdx_string_array_free(arr: TdxStringArray) {
    if !arr.data.is_null() && arr.len > 0 {
        let slice = unsafe { std::slice::from_raw_parts(arr.data, arr.len) };
        for &s in slice {
            if !s.is_null() {
                drop(unsafe { CString::from_raw(s as *mut c_char) });
            }
        }
        let _ = unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                arr.data as *mut *const c_char,
                arr.len,
            ))
        };
    }
}

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
            let empty = TdxStringArray { data: ptr::null(), len: 0 };
            if client.is_null() {
                set_error("client handle is null");
                return empty;
            }
            let client = unsafe { &*client };
            match runtime().block_on(async { client.inner.$method().await }) {
                Ok(items) => TdxStringArray::from_vec(items),
                Err(e) => {
                    set_error(&e.to_string());
                    empty
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
        ) -> TdxStringArray {
            let empty = TdxStringArray { data: ptr::null(), len: 0 };
            if client.is_null() {
                set_error("client handle is null");
                return empty;
            }
            let client = unsafe { &*client };
            $(
                let $param = match unsafe { cstr_to_str($param) } {
                    Some(s) => s,
                    None => {
                        set_error(concat!(stringify!($param), " is null or invalid UTF-8"));
                        return empty;
                    }
                };
            )+
            match runtime().block_on(async { client.inner.$method($($param),+).await }) {
                Ok(items) => TdxStringArray::from_vec(items),
                Err(e) => {
                    set_error(&e.to_string());
                    empty
                }
            }
        }
    };
}

/// Parse a C array of C string pointers into `Vec<String>`.
///
/// When `symbols` is null and `symbols_len` is 0 (Go empty-slice convention),
/// returns `Some(vec![])`. Returns `None` and sets the thread-local error if
/// the pointer is null with a non-zero length, or any element is null / invalid
/// UTF-8.
unsafe fn parse_symbol_array(
    symbols: *const *const c_char,
    symbols_len: usize,
) -> Option<Vec<String>> {
    if symbols.is_null() {
        if symbols_len == 0 {
            // Go sends (nil, 0) for empty slices — that's valid.
            return Some(vec![]);
        }
        set_error("symbols array pointer is null");
        return None;
    }
    let ptrs = unsafe { std::slice::from_raw_parts(symbols, symbols_len) };
    let mut out = Vec::with_capacity(symbols_len);
    for (i, &p) in ptrs.iter().enumerate() {
        match unsafe { cstr_to_str(p) } {
            Some(s) => out.push(s.to_owned()),
            None => {
                set_error(&format!("symbols[{}] is null or invalid UTF-8", i));
                return None;
            }
        }
    }
    Some(out)
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
                Ok(ticks) => $array_type::from_vec(ticks),
                Err(e) => {
                    set_error(&e.to_string());
                    empty
                }
            }
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
                Ok(ticks) => $array_type::from_vec(ticks),
                Err(e) => {
                    set_error(&e.to_string());
                    empty
                }
            }
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
            let empty = $array_type { data: ptr::null(), len: 0 };
            if client.is_null() {
                set_error("client handle is null");
                return empty;
            }
            let client = unsafe { &*client };
            $(
                let $param = match unsafe { cstr_to_str($param) } {
                    Some(s) => s,
                    None => {
                        set_error(concat!(stringify!($param), " is null or invalid UTF-8"));
                        return empty;
                    }
                };
            )+
            match runtime().block_on(async { client.inner.$method($($param),+).await }) {
                Ok(ticks) => $array_type::from_vec(ticks),
                Err(e) => {
                    set_error(&e.to_string());
                    empty
                }
            }
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
            let empty = $array_type { data: ptr::null(), len: 0 };
            if client.is_null() {
                set_error("client handle is null");
                return empty;
            }
            let client = unsafe { &*client };
            match runtime().block_on(async { client.inner.$method().await }) {
                Ok(ticks) => $array_type::from_vec(ticks),
                Err(e) => {
                    set_error(&e.to_string());
                    empty
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

// ═══════════════════════════════════════════════════════════════════════
//  Greeks (standalone, not client methods)
// ═══════════════════════════════════════════════════════════════════════

/// All 22 Black-Scholes Greeks + IV as a typed C struct.
#[repr(C)]
pub struct TdxGreeksResult {
    pub value: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
    pub rho: f64,
    pub epsilon: f64,
    pub lambda: f64,
    pub vanna: f64,
    pub charm: f64,
    pub vomma: f64,
    pub veta: f64,
    pub speed: f64,
    pub zomma: f64,
    pub color: f64,
    pub ultima: f64,
    pub iv: f64,
    pub iv_error: f64,
    pub d1: f64,
    pub d2: f64,
    pub dual_delta: f64,
    pub dual_gamma: f64,
}

/// Compute all 22 Black-Scholes Greeks + IV.
///
/// Returns a heap-allocated `TdxGreeksResult` (null on error).
/// Caller must free the result with `tdx_greeks_result_free`.
#[no_mangle]
pub extern "C" fn tdx_all_greeks(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    is_call: i32,
) -> *mut TdxGreeksResult {
    let g = tdbe::greeks::all_greeks(
        spot,
        strike,
        rate,
        div_yield,
        tte,
        option_price,
        is_call != 0,
    );
    let result = TdxGreeksResult {
        value: g.value,
        delta: g.delta,
        gamma: g.gamma,
        theta: g.theta,
        vega: g.vega,
        rho: g.rho,
        epsilon: g.epsilon,
        lambda: g.lambda,
        vanna: g.vanna,
        charm: g.charm,
        vomma: g.vomma,
        veta: g.veta,
        speed: g.speed,
        zomma: g.zomma,
        color: g.color,
        ultima: g.ultima,
        iv: g.iv,
        iv_error: g.iv_error,
        d1: g.d1,
        d2: g.d2,
        dual_delta: g.dual_delta,
        dual_gamma: g.dual_gamma,
    };
    Box::into_raw(Box::new(result))
}

/// Free a `TdxGreeksResult` returned by `tdx_all_greeks`.
#[no_mangle]
pub unsafe extern "C" fn tdx_greeks_result_free(ptr: *mut TdxGreeksResult) {
    if !ptr.is_null() {
        drop(unsafe { Box::from_raw(ptr) });
    }
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
    let (iv, err) = tdbe::greeks::implied_volatility(
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
//  Subscription types — used by both unified and FPSS active_subscriptions
// ═══════════════════════════════════════════════════════════════════════

/// A single active subscription entry.
#[repr(C)]
pub struct TdxSubscription {
    /// Subscription kind as a C string (e.g. "Quote", "Trade", "OpenInterest").
    pub kind: *const c_char,
    /// Contract identifier as a C string (e.g. "SPY" or "SPY 20260417 550 C").
    pub contract: *const c_char,
}

/// Array of active subscriptions returned by `tdx_unified_active_subscriptions`
/// and `tdx_fpss_active_subscriptions`.
#[repr(C)]
pub struct TdxSubscriptionArray {
    pub data: *const TdxSubscription,
    pub len: usize,
}

/// Build a `TdxSubscriptionArray` from an iterator of `(kind_debug, contract_display)` pairs.
fn build_subscription_array<I>(iter: I) -> *mut TdxSubscriptionArray
where
    I: Iterator<Item = (String, String)>,
{
    let pairs: Vec<(String, String)> = iter.collect();
    let mut subs = Vec::with_capacity(pairs.len());
    for (kind, contract) in &pairs {
        let kind_c = match CString::new(kind.as_str()) {
            Ok(c) => c,
            Err(_) => {
                // Free already-allocated CStrings before returning null
                for s in &subs {
                    let s: &TdxSubscription = s;
                    if !s.kind.is_null() {
                        drop(unsafe { CString::from_raw(s.kind as *mut c_char) });
                    }
                    if !s.contract.is_null() {
                        drop(unsafe { CString::from_raw(s.contract as *mut c_char) });
                    }
                }
                set_error("subscription kind contains null byte");
                return ptr::null_mut();
            }
        };
        let contract_c = match CString::new(contract.as_str()) {
            Ok(c) => c,
            Err(_) => {
                drop(kind_c); // free the kind we just allocated
                for s in &subs {
                    let s: &TdxSubscription = s;
                    if !s.kind.is_null() {
                        drop(unsafe { CString::from_raw(s.kind as *mut c_char) });
                    }
                    if !s.contract.is_null() {
                        drop(unsafe { CString::from_raw(s.contract as *mut c_char) });
                    }
                }
                set_error("subscription contract contains null byte");
                return ptr::null_mut();
            }
        };
        subs.push(TdxSubscription {
            kind: kind_c.into_raw() as *const c_char,
            contract: contract_c.into_raw() as *const c_char,
        });
    }
    let len = subs.len();
    let data = if subs.is_empty() {
        ptr::null()
    } else {
        let boxed = subs.into_boxed_slice();
        Box::into_raw(boxed) as *const TdxSubscription
    };
    Box::into_raw(Box::new(TdxSubscriptionArray { data, len }))
}

/// Free a `TdxSubscriptionArray` returned by `tdx_unified_active_subscriptions`
/// or `tdx_fpss_active_subscriptions`.
#[no_mangle]
pub unsafe extern "C" fn tdx_subscription_array_free(arr: *mut TdxSubscriptionArray) {
    if arr.is_null() {
        return;
    }
    let arr = unsafe { Box::from_raw(arr) };
    if !arr.data.is_null() && arr.len > 0 {
        let slice =
            unsafe { std::slice::from_raw_parts(arr.data as *mut TdxSubscription, arr.len) };
        for sub in slice {
            if !sub.kind.is_null() {
                drop(unsafe { CString::from_raw(sub.kind as *mut c_char) });
            }
            if !sub.contract.is_null() {
                drop(unsafe { CString::from_raw(sub.contract as *mut c_char) });
            }
        }
        // Reconstruct and drop the boxed slice
        drop(unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                arr.data as *mut TdxSubscription,
                arr.len,
            ))
        });
    }
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
        "STOCK" => tdbe::types::enums::SecType::Stock,
        "OPTION" => tdbe::types::enums::SecType::Option,
        "INDEX" => tdbe::types::enums::SecType::Index,
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

/// Subscribe to all open interest for a security type on the unified client.
/// sec_type: "STOCK", "OPTION", or "INDEX".
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_full_open_interest(
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
        "STOCK" => tdbe::types::enums::SecType::Stock,
        "OPTION" => tdbe::types::enums::SecType::Option,
        "INDEX" => tdbe::types::enums::SecType::Index,
        _ => {
            set_error("invalid sec_type: expected STOCK, OPTION, or INDEX");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    match handle.inner.subscribe_full_open_interest(st) {
        Ok(id) => id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Unsubscribe from all trades for a security type on the unified client.
/// sec_type: "STOCK", "OPTION", or "INDEX".
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_full_trades(
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
        "STOCK" => tdbe::types::enums::SecType::Stock,
        "OPTION" => tdbe::types::enums::SecType::Option,
        "INDEX" => tdbe::types::enums::SecType::Index,
        _ => {
            set_error("invalid sec_type: expected STOCK, OPTION, or INDEX");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    match handle.inner.unsubscribe_full_trades(st) {
        Ok(id) => id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Unsubscribe from all open interest for a security type on the unified client.
/// sec_type: "STOCK", "OPTION", or "INDEX".
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_full_open_interest(
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
        "STOCK" => tdbe::types::enums::SecType::Stock,
        "OPTION" => tdbe::types::enums::SecType::Option,
        "INDEX" => tdbe::types::enums::SecType::Index,
        _ => {
            set_error("invalid sec_type: expected STOCK, OPTION, or INDEX");
            return -1;
        }
    };
    let handle = unsafe { &*handle };
    match handle.inner.unsubscribe_full_open_interest(st) {
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

/// Look up a contract by ID. Returns a Display-formatted C string or null.
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

/// Get active subscriptions as a typed array. Returns null on error.
///
/// Caller must free the result with `tdx_subscription_array_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_active_subscriptions(
    handle: *const TdxUnified,
) -> *mut TdxSubscriptionArray {
    if handle.is_null() {
        set_error("unified handle is null");
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    match handle.inner.active_subscriptions() {
        Ok(subs) => {
            build_subscription_array(subs.iter().map(|(k, c)| (format!("{k:?}"), format!("{c}"))))
        }
        Err(e) => {
            set_error(&e.to_string());
            ptr::null_mut()
        }
    }
}

/// Poll for the next streaming event from the unified client.
///
/// Blocks for up to `timeout_ms` milliseconds. Returns a heap-allocated
/// `TdxFpssEvent` that MUST be freed with `tdx_fpss_event_free`.
/// Returns null on timeout (not an error), or if streaming has not been
/// started yet (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_next_event(
    handle: *const TdxUnified,
    timeout_ms: u64,
) -> *mut TdxFpssEvent {
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
        Ok(buffered) => {
            // Box the entire FfiBufferedEvent so _detail_string/_raw_payload
            // stay alive. Cast to *mut TdxFpssEvent (first field) for FFI.
            let ptr = Box::into_raw(Box::new(buffered));
            ptr as *mut TdxFpssEvent
        }
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
        &config.inner.fpss_hosts,
        config.inner.fpss_ring_size,
        config.inner.fpss_flush_mode,
        config.inner.reconnect_policy.clone(),
        config.inner.derive_ohlcvc,
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
        "STOCK" => tdbe::types::enums::SecType::Stock,
        "OPTION" => tdbe::types::enums::SecType::Option,
        "INDEX" => tdbe::types::enums::SecType::Index,
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

/// Subscribe to all open interest for a security type (full OI stream).
///
/// `sec_type` must be one of: "STOCK", "OPTION", "INDEX".
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_full_open_interest(
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
        "STOCK" => tdbe::types::enums::SecType::Stock,
        "OPTION" => tdbe::types::enums::SecType::Option,
        "INDEX" => tdbe::types::enums::SecType::Index,
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
    match client.subscribe_full_open_interest(st) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Unsubscribe from all trades for a security type (full trade stream).
///
/// `sec_type` must be one of: "STOCK", "OPTION", "INDEX".
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_full_trades(
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
        "STOCK" => tdbe::types::enums::SecType::Stock,
        "OPTION" => tdbe::types::enums::SecType::Option,
        "INDEX" => tdbe::types::enums::SecType::Index,
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
    match client.unsubscribe_full_trades(st) {
        Ok(req_id) => req_id,
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Unsubscribe from all open interest for a security type (full OI stream).
///
/// `sec_type` must be one of: "STOCK", "OPTION", "INDEX".
///
/// Returns the request ID on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_full_open_interest(
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
        "STOCK" => tdbe::types::enums::SecType::Stock,
        "OPTION" => tdbe::types::enums::SecType::Option,
        "INDEX" => tdbe::types::enums::SecType::Index,
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
    match client.unsubscribe_full_open_interest(st) {
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
/// Returns a Display-formatted C string representation of the contract, or NULL if not found.
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
/// Returns a heap-allocated `TdxSubscriptionArray` (null on error).
/// Caller must free the result with `tdx_subscription_array_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_active_subscriptions(
    handle: *const TdxFpssHandle,
) -> *mut TdxSubscriptionArray {
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
    build_subscription_array(
        subs.into_iter()
            .map(|(kind, contract)| (format!("{kind:?}"), format!("{contract}"))),
    )
}

/// Poll for the next FPSS event as a typed `#[repr(C)]` struct.
///
/// Blocks for up to `timeout_ms` milliseconds. Returns a heap-allocated
/// `TdxFpssEvent` that MUST be freed with `tdx_fpss_event_free`.
///
/// Returns null if no event arrived within the timeout (this is NOT an error).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_next_event(
    handle: *const TdxFpssHandle,
    timeout_ms: u64,
) -> *mut TdxFpssEvent {
    if handle.is_null() {
        set_error("FPSS handle is null");
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let rx = handle.rx.lock().unwrap_or_else(|e| e.into_inner());
    let timeout = std::time::Duration::from_millis(timeout_ms);
    match rx.recv_timeout(timeout) {
        Ok(buffered) => {
            // Box the entire FfiBufferedEvent so _detail_string/_raw_payload
            // stay alive. Cast to *mut TdxFpssEvent (first field) for FFI.
            let ptr = Box::into_raw(Box::new(buffered));
            ptr as *mut TdxFpssEvent
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ptr::null_mut(),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => ptr::null_mut(),
    }
}

/// Free a `TdxFpssEvent` returned by `tdx_fpss_next_event` or
/// `tdx_unified_next_event`.
///
/// Note: the `control.detail` pointer inside the event is NOT separately
/// heap-allocated — it was owned by the internal buffered event and is
/// invalidated when the event struct is freed. Do NOT call
/// `tdx_string_free` on `control.detail`.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_event_free(event: *mut TdxFpssEvent) {
    if !event.is_null() {
        // The pointer was created by boxing a FfiBufferedEvent and casting
        // to *mut TdxFpssEvent (which is the first field). Cast back to
        // free the entire FfiBufferedEvent including owned _detail_string
        // and _raw_payload.
        drop(unsafe { Box::from_raw(event as *mut FfiBufferedEvent) });
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
