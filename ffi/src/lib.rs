// Reason: FFI extern "C" functions use raw pointers, pattern matching, and C-style
// conventions that are fundamentally incompatible with many pedantic lints (let-else
// on nullable pointers, doc_markdown on C identifiers, ptr_cast_constness on FFI
// boundary types). Fixing these would make the FFI code less idiomatic for C interop.
#![allow(clippy::pedantic)]

//! C FFI layer for `thetadatadx` — exposes the Rust SDK as `extern "C"` functions.
//!
//! This crate is compiled as both `cdylib` (shared library) and `staticlib` (archive).
//! It is consumed by the Go (`CGo`) and C++ SDKs.
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
use std::sync::atomic::{AtomicU64, Ordering};
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
//
// Contract: the error slot is scoped to the OS thread that set it. Higher-
// level languages whose runtime can migrate a logical execution unit
// across OS threads (notably Go, where a goroutine can park on one thread
// and resume on another) MUST pin the execution unit for the duration of
// a clear/call/check sequence. The generated Go wrappers do this via
// `runtime.LockOSThread` + deferred unlock (see
// `crates/thetadatadx/build_support/endpoints/render/go.rs` —
// `render_go_endpoint_method`). C++ and Python never migrate threads
// implicitly, so no pinning is needed there.

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
}

fn set_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

/// Wrap an `extern "C"` fn body. Catches panics that would otherwise
/// abort the host process (C / Go / Python) and converts them into a
/// well-defined error return plus a thread-local `last_error` entry.
///
/// The wrapped block must return `T`. On panic, `default` is returned and
/// an error string describing the panic payload (if extractable) is set
/// via `set_error(...)`.
///
/// Rust 1.81+ aborts when a panic crosses an `extern "C"` boundary;
/// pre-1.81 the behavior is undefined. Both modes crash the host process,
/// which is unacceptable for a library binding (a typo in a macro arg or
/// an unexpected invariant violation inside `tokio::runtime::block_on`
/// would take down the user's entire program). Wrapping the body in
/// `catch_unwind` keeps the crash contained in the thread and surfaces
/// the reason through the normal FFI error channel.
macro_rules! ffi_boundary {
    ($default:expr, $body:block) => {{
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| $body)) {
            Ok(v) => v,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&'static str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("<non-string panic>");
                tracing::error!(
                    target: "thetadatadx::ffi::panic",
                    msg,
                    "FFI boundary caught panic",
                );
                set_error(&format!("panic at FFI boundary: {msg}"));
                $default
            }
        }
    }};
}

/// Retrieve the last error message (or null if no error).
///
/// The returned pointer is valid until the next FFI call on the same thread.
/// Do NOT free this pointer.
#[no_mangle]
pub extern "C" fn tdx_last_error() -> *const c_char {
    ffi_boundary!(ptr::null(), {
        LAST_ERROR.with(|e| {
            let borrow = e.borrow();
            match borrow.as_ref() {
                Some(s) => s.as_ptr(),
                None => ptr::null(),
            }
        })
    })
}

/// Clear the thread-local error string.
///
/// Wrappers in higher-level languages (Go, C++, Python) should call this
/// before issuing an FFI call so they can distinguish "the call set a new
/// error" from "the previous call left a stale error in the slot". Critical
/// for endpoints that return an empty value sentinel on both success
/// (no rows) and failure (e.g. timeout) — without clearing first, the
/// caller can't tell the two apart from the array alone.
#[no_mangle]
pub extern "C" fn tdx_clear_error() {
    ffi_boundary!((), {
        LAST_ERROR.with(|e| {
            *e.borrow_mut() = None;
        });
    })
}

// ── Opaque handle types ──

/// Opaque credentials handle.
pub struct TdxCredentials {
    inner: thetadatadx::Credentials,
}

/// Opaque client handle.
///
/// `repr(transparent)` guarantees `*const TdxClient` and `*const MddsClient`
/// have identical layout, allowing safe pointer casts in `tdx_unified_historical()`.
#[repr(transparent)]
pub struct TdxClient {
    inner: thetadatadx::mdds::MddsClient,
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
    /// Cumulative count of FPSS events dropped because the FFI receiver
    /// was gone (queue shut down or consumer stopped polling) before the
    /// callback could hand the event off. Exposed via
    /// `tdx_unified_dropped_events` so Go / C++ consumers have parity
    /// with the Python / TypeScript SDKs' `dropped_events()` getter.
    /// Lives on the handle (not the closure) so the count survives the
    /// single `start_streaming` call that this handle supports.
    dropped_events: Arc<AtomicU64>,
}

/// Opaque FPSS streaming client handle.
///
/// Uses the same pattern as the Python SDK: an internal mpsc channel buffering
/// events from the Disruptor callback, and `tdx_fpss_next_event` polls it with
/// a timeout, returning a `*mut TdxFpssEvent` typed struct.
pub struct TdxFpssHandle {
    inner: Arc<Mutex<Option<thetadatadx::fpss::FpssClient>>>,
    rx: Arc<Mutex<std::sync::mpsc::Receiver<FfiBufferedEvent>>>,
    /// Saved connection parameters for reconnection (Gap 3).
    connect_params: FpssConnectParams,
    /// Cumulative count of FPSS events dropped because the FFI receiver
    /// was gone (queue shut down or consumer stopped polling) before the
    /// callback could hand the event off. Exposed via
    /// `tdx_fpss_dropped_events` so Go / C++ consumers have parity with
    /// the Python / TypeScript SDKs' `dropped_events()` getter. The
    /// `Arc` is cloned into every callback closure (initial connect +
    /// each reconnect), so the counter survives reconnection.
    dropped_events: Arc<AtomicU64>,
}

/// Saved FPSS connection parameters for FFI-safe reconnection.
struct FpssConnectParams {
    creds: thetadatadx::Credentials,
    hosts: Vec<(String, u16)>,
    ring_size: usize,
    flush_mode: thetadatadx::FpssFlushMode,
    reconnect_policy: thetadatadx::config::ReconnectPolicy,
    derive_ohlcvc: bool,
}

include!("endpoint_request_options.rs");

// ═══════════════════════════════════════════════════════════════════════
//  #[repr(C)] FPSS streaming event types — zero-copy across FFI
//
//  All of the kind-enum / per-variant struct / ZERO_* const definitions
//  are generated from `crates/thetadatadx/fpss_event_schema.toml`. The
//  hand-written wrapper `FfiBufferedEvent` below owns the backing memory
//  for the generated `TdxFpssEvent`'s pointer fields (`control.detail`
//  and `raw_data.payload`). Split into two include points so the
//  converter (which names `FfiBufferedEvent`) is compiled AFTER the
//  wrapper itself.
// ═══════════════════════════════════════════════════════════════════════

include!("fpss_event_structs.rs");

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
    /// Owns the `CString` backing `event.control.detail`, if any.
    _detail_string: Option<CString>,
    /// Owns the raw payload bytes backing `event.raw_data.payload`, if any.
    _raw_payload: Option<Vec<u8>>,
}

// SAFETY: FfiBufferedEvent is sent across std::sync::mpsc channels.
// The owned data (_detail_string, _raw_payload) is heap-allocated and
// the pointers inside `event` point into that owned data. The event is
// only accessed from the receiving thread after the send completes.
unsafe impl Send for FfiBufferedEvent {}

include!("fpss_event_converter.rs");

// ── Helper: C string to &str ──

/// Decode a possibly-null C string pointer.
///
/// - `p.is_null()` → `Ok(None)` (caller chose not to pass this argument).
/// - Non-null with valid UTF-8 → `Ok(Some(&str))`.
/// - Non-null with invalid UTF-8 → `Err(Utf8Error)`.
///
/// Callers must distinguish these cases: a null pointer is usually a
/// legal "omit this optional arg" sentinel, while invalid UTF-8 is a bug
/// in the caller that should be surfaced through `tdx_last_error`.
unsafe fn cstr_to_str<'a>(p: *const c_char) -> Result<Option<&'a str>, std::str::Utf8Error> {
    if p.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(p) }.to_str().map(Some)
}

/// Extract a required C string arg. On failure, calls `set_error` with a
/// message that distinguishes null-pointer vs invalid-UTF-8 and returns
/// the given fallback value from the enclosing function.
macro_rules! require_cstr {
    ($p:ident, $fallback:expr) => {
        match unsafe { cstr_to_str($p) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error(concat!(stringify!($p), " is null"));
                return $fallback;
            }
            Err(e) => {
                set_error(&format!("{} is not valid UTF-8: {e}", stringify!($p)));
                return $fallback;
            }
        }
    };
}

fn insert_optional_str_arg(
    args: &mut thetadatadx::EndpointArgs,
    key: &str,
    raw: *const c_char,
) -> Result<(), String> {
    match unsafe { cstr_to_str(raw) } {
        Ok(None) => Ok(()),
        Ok(Some(value)) => {
            args.insert(
                key.to_string(),
                thetadatadx::EndpointArgValue::Str(value.to_string()),
            );
            Ok(())
        }
        Err(e) => Err(format!("{key} is not valid UTF-8: {e}")),
    }
}

fn insert_int_arg(args: &mut thetadatadx::EndpointArgs, key: &str, value: i32) {
    args.insert(
        key.to_string(),
        thetadatadx::EndpointArgValue::Int(i64::from(value)),
    );
}

fn insert_bool_arg(
    args: &mut thetadatadx::EndpointArgs,
    key: &str,
    value: i32,
) -> Result<(), String> {
    match value {
        0 => {
            args.insert(key.to_string(), thetadatadx::EndpointArgValue::Bool(false));
            Ok(())
        }
        1 => {
            args.insert(key.to_string(), thetadatadx::EndpointArgValue::Bool(true));
            Ok(())
        }
        other => Err(format!("{key} must be 0 (false) or 1 (true), got {other}")),
    }
}

fn insert_float_arg(args: &mut thetadatadx::EndpointArgs, key: &str, value: f64) {
    args.insert(key.to_string(), thetadatadx::EndpointArgValue::Float(value));
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
    ffi_boundary!(ptr::null_mut(), {
        let email = match unsafe { cstr_to_str(email) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("email is null");
                return ptr::null_mut();
            }
            Err(e) => {
                set_error(&format!("email is not valid UTF-8: {e}"));
                return ptr::null_mut();
            }
        };
        let password = match unsafe { cstr_to_str(password) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("password is null");
                return ptr::null_mut();
            }
            Err(e) => {
                set_error(&format!("password is not valid UTF-8: {e}"));
                return ptr::null_mut();
            }
        };
        let creds = thetadatadx::Credentials::new(email, password);
        Box::into_raw(Box::new(TdxCredentials { inner: creds }))
    })
}

/// Load credentials from a file (line 1 = email, line 2 = password).
///
/// Returns null on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_credentials_from_file(path: *const c_char) -> *mut TdxCredentials {
    ffi_boundary!(ptr::null_mut(), {
        let path = match unsafe { cstr_to_str(path) } {
            Ok(Some(s)) => s,
            Ok(None) => {
                set_error("path is null");
                return ptr::null_mut();
            }
            Err(e) => {
                set_error(&format!("path is not valid UTF-8: {e}"));
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
    })
}

/// Free a credentials handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_credentials_free(creds: *mut TdxCredentials) {
    ffi_boundary!((), {
        if !creds.is_null() {
            drop(unsafe { Box::from_raw(creds) });
        }
    })
}

// ── Config ──

/// Create a production config (`ThetaData` NJ datacenter).
#[no_mangle]
pub extern "C" fn tdx_config_production() -> *mut TdxConfig {
    ffi_boundary!(ptr::null_mut(), {
        Box::into_raw(Box::new(TdxConfig {
            inner: thetadatadx::DirectConfig::production(),
        }))
    })
}

/// Create a dev config (FPSS dev servers, port 20200, infinite replay).
#[no_mangle]
pub extern "C" fn tdx_config_dev() -> *mut TdxConfig {
    ffi_boundary!(ptr::null_mut(), {
        Box::into_raw(Box::new(TdxConfig {
            inner: thetadatadx::DirectConfig::dev(),
        }))
    })
}

/// Create a stage config (FPSS stage servers, port 20100, unstable).
#[no_mangle]
pub extern "C" fn tdx_config_stage() -> *mut TdxConfig {
    ffi_boundary!(ptr::null_mut(), {
        Box::into_raw(Box::new(TdxConfig {
            inner: thetadatadx::DirectConfig::stage(),
        }))
    })
}

/// Free a config handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_config_free(config: *mut TdxConfig) {
    ffi_boundary!((), {
        if !config.is_null() {
            drop(unsafe { Box::from_raw(config) });
        }
    })
}

/// Set FPSS flush mode on a config handle.
///
/// - `mode = 0`: Batched (default) -- flush only on PING every 100ms
/// - `mode = 1`: Immediate -- flush after every frame write (lowest latency)
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_flush_mode(config: *mut TdxConfig, mode: i32) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        let config = unsafe { &mut *config };
        config.inner.fpss_flush_mode = match mode {
            1 => thetadatadx::FpssFlushMode::Immediate,
            _ => thetadatadx::FpssFlushMode::Batched,
        };
    })
}

/// Set FPSS reconnect policy on a config handle.
///
/// - `policy = 0`: Auto (default) -- auto-reconnect matching Java terminal behavior
/// - `policy = 1`: Manual -- no auto-reconnect, user calls reconnect explicitly
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_reconnect_policy(config: *mut TdxConfig, policy: i32) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        let config = unsafe { &mut *config };
        config.inner.reconnect_policy = match policy {
            1 => thetadatadx::ReconnectPolicy::Manual,
            _ => thetadatadx::ReconnectPolicy::Auto,
        };
    })
}

/// Set FPSS OHLCVC derivation on a config handle.
///
/// - `enabled = 1` (default): derive OHLCVC bars locally from trade events
/// - `enabled = 0`: only emit server-sent OHLCVC frames (lower overhead)
#[no_mangle]
pub unsafe extern "C" fn tdx_config_set_derive_ohlcvc(config: *mut TdxConfig, enabled: i32) {
    ffi_boundary!((), {
        if config.is_null() {
            return;
        }
        let config = unsafe { &mut *config };
        config.inner.derive_ohlcvc = enabled != 0;
    })
}

// ── Client ──

/// Connect to `ThetaData` servers (authenticates via Nexus API).
///
/// Returns null on connection/auth failure (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_client_connect(
    creds: *const TdxCredentials,
    config: *const TdxConfig,
) -> *mut TdxClient {
    ffi_boundary!(ptr::null_mut(), {
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
        match runtime().block_on(thetadatadx::mdds::MddsClient::connect(
            &creds.inner,
            config.inner.clone(),
        )) {
            Ok(client) => Box::into_raw(Box::new(TdxClient { inner: client })),
            Err(e) => {
                set_error(&e.to_string());
                ptr::null_mut()
            }
        }
    })
}

/// Free a client handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_client_free(client: *mut TdxClient) {
    ffi_boundary!((), {
        if !client.is_null() {
            drop(unsafe { Box::from_raw(client) });
        }
    })
}

// ── String free ──

/// Free a string returned by any `tdx_*` function.
///
/// MUST be called for every non-null `*mut c_char` returned by this library.
#[no_mangle]
pub unsafe extern "C" fn tdx_string_free(s: *mut c_char) {
    ffi_boundary!((), {
        if !s.is_null() {
            drop(unsafe { CString::from_raw(s) });
        }
    })
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
            /// Infallible for tick types (no `CString` allocation). Returns
            /// `Result` to match the signature of fallible sibling arrays
            /// (`TdxStringArray`, `TdxOptionContractArray`) so the shared
            /// FFI endpoint macros stay generic over `$array_type`.
            pub(crate) fn from_vec(v: Vec<$tick>) -> Result<Self, std::ffi::NulError> {
                let len = v.len();
                if len == 0 {
                    return Ok(Self {
                        data: ptr::null(),
                        len: 0,
                    });
                }
                let boxed = v.into_boxed_slice();
                let data = Box::into_raw(boxed) as *const $tick;
                Ok(Self { data, len })
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
tick_array_type!(TdxTradeQuoteTickArray, tdbe::TradeQuoteTick);

/// Generate a `#[no_mangle] extern "C"` free function for a tick array type.
macro_rules! tick_array_free {
    ($fn_name:ident, $array_type:ident) => {
        /// Free a tick array returned by an FFI endpoint.
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(arr: $array_type) {
            ffi_boundary!((), {
                unsafe { arr.free() };
            })
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
    pub strike: f64,
    pub right: i32,
}

/// Array of FFI-safe option contracts.
#[repr(C)]
pub struct TdxOptionContractArray {
    pub data: *const TdxOptionContract,
    pub len: usize,
}

impl TdxOptionContractArray {
    fn from_vec(contracts: Vec<tdbe::OptionContract>) -> Result<Self, std::ffi::NulError> {
        let len = contracts.len();
        if len == 0 {
            return Ok(Self {
                data: ptr::null(),
                len: 0,
            });
        }
        let ffi_contracts = contracts
            .into_iter()
            .map(|c| {
                Ok(TdxOptionContract {
                    root: CString::new(c.root)?.into_raw().cast_const(),
                    expiration: c.expiration,
                    strike: c.strike,
                    right: c.right,
                })
            })
            .collect::<Result<Vec<_>, std::ffi::NulError>>()?;
        let boxed = ffi_contracts.into_boxed_slice();
        let data = Box::into_raw(boxed) as *const TdxOptionContract;
        Ok(Self { data, len })
    }
}

/// Free an option contract array, including all heap-allocated root strings.
#[no_mangle]
pub unsafe extern "C" fn tdx_option_contract_array_free(arr: TdxOptionContractArray) {
    ffi_boundary!((), {
        if !arr.data.is_null() && arr.len > 0 {
            // First free each root C string
            let slice = unsafe { std::slice::from_raw_parts(arr.data, arr.len) };
            for contract in slice {
                if !contract.root.is_null() {
                    drop(unsafe { CString::from_raw(contract.root.cast_mut()) });
                }
            }
            // Then free the array itself
            let _ = unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    arr.data.cast_mut(),
                    arr.len,
                ))
            };
        }
    })
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
    fn from_vec(strings: Vec<String>) -> Result<Self, std::ffi::NulError> {
        let len = strings.len();
        if len == 0 {
            return Ok(Self {
                data: ptr::null(),
                len: 0,
            });
        }
        let cstrings = strings
            .into_iter()
            .map(|s| CString::new(s).map(|c| c.into_raw().cast_const()))
            .collect::<Result<Vec<*const c_char>, std::ffi::NulError>>()?;
        let boxed = cstrings.into_boxed_slice();
        let data = Box::into_raw(boxed) as *const *const c_char;
        Ok(Self { data, len })
    }
}

/// Free a string array, including all individual C strings.
#[no_mangle]
pub unsafe extern "C" fn tdx_string_array_free(arr: TdxStringArray) {
    ffi_boundary!((), {
        if !arr.data.is_null() && arr.len > 0 {
            let slice = unsafe { std::slice::from_raw_parts(arr.data, arr.len) };
            for &s in slice {
                if !s.is_null() {
                    drop(unsafe { CString::from_raw(s.cast_mut()) });
                }
            }
            let _ = unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    arr.data.cast_mut(),
                    arr.len,
                ))
            };
        }
    })
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
            ffi_boundary!(TdxStringArray { data: ptr::null(), len: 0 }, {
                let empty = TdxStringArray { data: ptr::null(), len: 0 };
                ffi_boundary!(TdxStringArray { data: ptr::null(), len: 0 }, {
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
            Ok(Some(s)) => out.push(s.to_owned()),
            Ok(None) => {
                set_error(&format!("symbols[{i}] is null"));
                return None;
            }
            Err(e) => {
                set_error(&format!("symbols[{i}] is not valid UTF-8: {e}"));
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
/// `right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively (see
/// the `tdbe::right::parse_right` canonical parser). Returns a heap-allocated
/// `TdxGreeksResult`, or null on error (invalid UTF-8 / unrecognised right /
/// resolves to `both`). Caller must free the result with
/// `tdx_greeks_result_free`.
///
/// # Safety
///
/// `right` must be a valid NUL-terminated C string pointer (or null, which
/// returns null with an error set).
#[no_mangle]
pub unsafe extern "C" fn tdx_all_greeks(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    right: *const c_char,
) -> *mut TdxGreeksResult {
    ffi_boundary!(std::ptr::null_mut(), {
        let right_str = require_cstr!(right, std::ptr::null_mut());
        if let Err(e) = thetadatadx::parse_right_strict(right_str) {
            set_error(&e.to_string());
            return std::ptr::null_mut();
        }
        let g =
            tdbe::greeks::all_greeks(spot, strike, rate, div_yield, tte, option_price, right_str);
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
    })
}

/// Free a `TdxGreeksResult` returned by `tdx_all_greeks`.
#[no_mangle]
pub unsafe extern "C" fn tdx_greeks_result_free(ptr: *mut TdxGreeksResult) {
    ffi_boundary!((), {
        if !ptr.is_null() {
            drop(unsafe { Box::from_raw(ptr) });
        }
    })
}

/// Compute implied volatility via bisection.
///
/// `right` accepts `"C"`/`"P"` or `"call"`/`"put"` case-insensitively (see
/// the `tdbe::right::parse_right` canonical parser). Returns IV in `*out_iv`
/// and error in `*out_error`. Returns 0 on success, -1 on failure (null
/// pointers / invalid UTF-8 / unrecognised right / resolves to `both`).
///
/// # Safety
///
/// `right` must be a valid NUL-terminated C string pointer. `out_iv` and
/// `out_error` must be valid, writable `double` pointers.
#[no_mangle]
pub unsafe extern "C" fn tdx_implied_volatility(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    right: *const c_char,
    out_iv: *mut f64,
    out_error: *mut f64,
) -> i32 {
    ffi_boundary!(-1, {
        if out_iv.is_null() || out_error.is_null() {
            set_error("output pointers must not be null");
            return -1;
        }
        let right_str = require_cstr!(right, -1);
        if let Err(e) = thetadatadx::parse_right_strict(right_str) {
            set_error(&e.to_string());
            return -1;
        }
        let (iv, err) = tdbe::greeks::implied_volatility(
            spot,
            strike,
            rate,
            div_yield,
            tte,
            option_price,
            right_str,
        );
        unsafe {
            *out_iv = iv;
            *out_error = err;
        }
        0
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Subscription types — used by both unified and FPSS active_subscriptions
// ═══════════════════════════════════════════════════════════════════════

/// A single active subscription entry.
#[repr(C)]
pub struct TdxSubscription {
    /// Subscription kind as a C string (e.g. "Quote", "Trade", "`OpenInterest`").
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
        let kind_c = if let Ok(c) = CString::new(kind.as_str()) {
            c
        } else {
            // Free already-allocated CStrings before returning null
            for s in &subs {
                let s: &TdxSubscription = s;
                if !s.kind.is_null() {
                    drop(unsafe { CString::from_raw(s.kind.cast_mut()) });
                }
                if !s.contract.is_null() {
                    drop(unsafe { CString::from_raw(s.contract.cast_mut()) });
                }
            }
            set_error("subscription kind contains null byte");
            return ptr::null_mut();
        };
        let contract_c = if let Ok(c) = CString::new(contract.as_str()) {
            c
        } else {
            drop(kind_c); // free the kind we just allocated
            for s in &subs {
                let s: &TdxSubscription = s;
                if !s.kind.is_null() {
                    drop(unsafe { CString::from_raw(s.kind.cast_mut()) });
                }
                if !s.contract.is_null() {
                    drop(unsafe { CString::from_raw(s.contract.cast_mut()) });
                }
            }
            set_error("subscription contract contains null byte");
            return ptr::null_mut();
        };
        subs.push(TdxSubscription {
            kind: kind_c.into_raw().cast_const(),
            contract: contract_c.into_raw().cast_const(),
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
    ffi_boundary!((), {
        if arr.is_null() {
            return;
        }
        let arr = unsafe { Box::from_raw(arr) };
        if !arr.data.is_null() && arr.len > 0 {
            let slice = unsafe { std::slice::from_raw_parts(arr.data.cast_mut(), arr.len) };
            for sub in slice {
                if !sub.kind.is_null() {
                    drop(unsafe { CString::from_raw(sub.kind.cast_mut()) });
                }
                if !sub.contract.is_null() {
                    drop(unsafe { CString::from_raw(sub.contract.cast_mut()) });
                }
            }
            // Reconstruct and drop the boxed slice
            drop(unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    arr.data.cast_mut(),
                    arr.len,
                ))
            });
        }
    })
}

/// A single FPSS contract-map entry.
#[repr(C)]
pub struct TdxContractMapEntry {
    /// Server-assigned contract ID.
    pub id: i32,
    /// Display-formatted contract string (e.g. "SPY 20260417 550 C").
    pub contract: *const c_char,
}

/// Array of contract-map entries returned by `tdx_unified_contract_map`
/// and `tdx_fpss_contract_map`.
#[repr(C)]
pub struct TdxContractMapArray {
    pub data: *const TdxContractMapEntry,
    pub len: usize,
}

/// Build a `TdxContractMapArray` from an iterator of `(id, contract_display)` pairs.
fn build_contract_map_array<I>(iter: I) -> *mut TdxContractMapArray
where
    I: Iterator<Item = (i32, String)>,
{
    let items: Vec<(i32, String)> = iter.collect();
    let mut entries = Vec::with_capacity(items.len());
    for (id, contract) in &items {
        let contract_c = if let Ok(c) = CString::new(contract.as_str()) {
            c
        } else {
            for entry in &entries {
                let entry: &TdxContractMapEntry = entry;
                if !entry.contract.is_null() {
                    drop(unsafe { CString::from_raw(entry.contract.cast_mut()) });
                }
            }
            set_error("contract map entry contains null byte");
            return ptr::null_mut();
        };
        entries.push(TdxContractMapEntry {
            id: *id,
            contract: contract_c.into_raw().cast_const(),
        });
    }
    let len = entries.len();
    let data = if entries.is_empty() {
        ptr::null()
    } else {
        let boxed = entries.into_boxed_slice();
        Box::into_raw(boxed) as *const TdxContractMapEntry
    };
    Box::into_raw(Box::new(TdxContractMapArray { data, len }))
}

/// Free a `TdxContractMapArray` returned by `tdx_unified_contract_map`
/// or `tdx_fpss_contract_map`.
#[no_mangle]
pub unsafe extern "C" fn tdx_contract_map_array_free(arr: *mut TdxContractMapArray) {
    ffi_boundary!((), {
        if arr.is_null() {
            return;
        }
        let arr = unsafe { Box::from_raw(arr) };
        if !arr.data.is_null() && arr.len > 0 {
            let slice = unsafe { std::slice::from_raw_parts(arr.data.cast_mut(), arr.len) };
            for entry in slice {
                if !entry.contract.is_null() {
                    drop(unsafe { CString::from_raw(entry.contract.cast_mut()) });
                }
            }
            drop(unsafe {
                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    arr.data.cast_mut(),
                    arr.len,
                ))
            });
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Unified client — historical + streaming through one handle
// ═══════════════════════════════════════════════════════════════════════

/// Connect to `ThetaData` (historical only — FPSS streaming is NOT started).
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
    ffi_boundary!(ptr::null_mut(), {
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
                dropped_events: Arc::new(AtomicU64::new(0)),
            })),
            Err(e) => {
                set_error(&e.to_string());
                ptr::null_mut()
            }
        }
    })
}

/// Start FPSS streaming on the unified client.
///
/// Creates an internal mpsc channel and registers a callback handler.
/// Events are buffered — poll with `tdx_unified_next_event()`.
///
/// Returns 0 on success, -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_start_streaming(handle: *const TdxUnified) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let handle = unsafe { &*handle };

        let (tx, rx) = std::sync::mpsc::channel::<FfiBufferedEvent>();
        // Clone the handle-level counter so the send-failure path in the
        // callback increments the same `AtomicU64` the
        // `tdx_unified_dropped_events` getter reads. Parity with the Python
        // / TS SDKs: silent drops need to be countable from C / Go / C++.
        let dropped_events = Arc::clone(&handle.dropped_events);

        match handle
            .inner
            .start_streaming(move |event: &thetadatadx::fpss::FpssEvent| {
                let buffered = fpss_event_to_ffi(event);
                if tx.send(buffered).is_err() {
                    let count = dropped_events.fetch_add(1, Ordering::Relaxed) + 1;
                    tracing::debug!(
                        target: "thetadatadx::ffi::streaming",
                        dropped_total = count,
                        "fpss event dropped -- receiver dead",
                    );
                }
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
    })
}

/// Subscribe to quote data for a stock symbol via the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_quotes(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match handle.inner.subscribe_quotes(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to trade data for a stock symbol via the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_trades(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match handle.inner.subscribe_trades(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from quote data for a stock symbol via the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_quotes(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match handle.inner.unsubscribe_quotes(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from trade data for a stock symbol via the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_trades(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match handle.inner.unsubscribe_trades(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to open interest data for a stock symbol on the unified client.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_open_interest(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match handle.inner.subscribe_open_interest(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to all trades for a security type on the unified client.
/// `sec_type`: "STOCK", "OPTION", or "INDEX".
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_full_trades(
    handle: *const TdxUnified,
    sec_type: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let sec_type_str = require_cstr!(sec_type, -1);
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
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to all open interest for a security type on the unified client.
/// `sec_type`: "STOCK", "OPTION", or "INDEX".
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_full_open_interest(
    handle: *const TdxUnified,
    sec_type: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let sec_type_str = require_cstr!(sec_type, -1);
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
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from all trades for a security type on the unified client.
/// `sec_type`: "STOCK", "OPTION", or "INDEX".
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_full_trades(
    handle: *const TdxUnified,
    sec_type: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let sec_type_str = require_cstr!(sec_type, -1);
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
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from all open interest for a security type on the unified client.
/// `sec_type`: "STOCK", "OPTION", or "INDEX".
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_full_open_interest(
    handle: *const TdxUnified,
    sec_type: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let sec_type_str = require_cstr!(sec_type, -1);
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
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from open interest data on the unified client.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_open_interest(
    handle: *const TdxUnified,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match handle.inner.unsubscribe_open_interest(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Unified — Option-level subscribe/unsubscribe (Gap 1)
// ═══════════════════════════════════════════════════════════════════════

/// Subscribe to quote data for an option contract via the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_option_quotes(
    handle: *const TdxUnified,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match handle.inner.subscribe_quotes(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to trade data for an option contract via the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_option_trades(
    handle: *const TdxUnified,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match handle.inner.subscribe_trades(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to open interest data for an option contract via the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe_option_open_interest(
    handle: *const TdxUnified,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match handle.inner.subscribe_open_interest(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from quote data for an option contract via the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_option_quotes(
    handle: *const TdxUnified,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match handle.inner.unsubscribe_quotes(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from trade data for an option contract via the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_option_trades(
    handle: *const TdxUnified,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match handle.inner.unsubscribe_trades(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from open interest data for an option contract via the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe_option_open_interest(
    handle: *const TdxUnified,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match handle.inner.unsubscribe_open_interest(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Unified — contract_map (Gap 2) and reconnect (Gap 3)
// ═══════════════════════════════════════════════════════════════════════

/// Get the full contract map from the unified client.
///
/// Returns a heap-allocated `TdxContractMapArray` (null on error).
/// Caller must free the result with `tdx_contract_map_array_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_contract_map(
    handle: *const TdxUnified,
) -> *mut TdxContractMapArray {
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null_mut();
        }
        let handle = unsafe { &*handle };
        match handle.inner.contract_map() {
            Ok(map) => build_contract_map_array(
                map.into_iter()
                    .map(|(id, contract)| (id, format!("{contract}"))),
            ),
            Err(e) => {
                set_error(&e.to_string());
                ptr::null_mut()
            }
        }
    })
}

/// Reconnect the unified client's streaming connection.
///
/// Saves active subscriptions, stops the current streaming, starts a new one
/// using the same credentials, and re-subscribes everything. The callback-based
/// version (`reconnect_streaming(handler)`) stays Rust/Python-only.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
///
/// # Event continuity
///
/// Events buffered in the old streaming channel are dropped during reconnect.
/// There is no gap-free delivery guarantee across reconnections. Callers that
/// require gap-free streaming should implement their own sequence-number-based
/// gap detection and replay logic.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_reconnect(handle: *const TdxUnified) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let handle = unsafe { &*handle };

        // Save active subscriptions. If streaming isn't running (or the
        // subscription locks are poisoned upstream) we must abort the
        // reconnect -- silently falling back to an empty list drops every
        // subscription on the floor.
        let saved_subs = match handle.inner.active_subscriptions() {
            Ok(subs) => subs,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        let saved_full_subs = match handle.inner.active_full_subscriptions() {
            Ok(subs) => subs,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };

        // Stop streaming
        handle.inner.stop_streaming();

        // Clear the old rx
        if let Ok(mut guard) = handle.rx.lock() {
            *guard = None;
        }

        // Start a new streaming connection with an internal buffered channel
        let (tx, rx) = std::sync::mpsc::channel::<FfiBufferedEvent>();
        // Reuse the handle-level counter so `tdx_unified_dropped_events`
        // reflects drops across both the initial `start_streaming` and
        // every subsequent reconnect. Closure-local counters would reset
        // here and make the getter useless for long-lived clients.
        let dropped_events = Arc::clone(&handle.dropped_events);

        if let Err(e) = handle
            .inner
            .start_streaming(move |event: &thetadatadx::fpss::FpssEvent| {
                let buffered = fpss_event_to_ffi(event);
                if tx.send(buffered).is_err() {
                    let count = dropped_events.fetch_add(1, Ordering::Relaxed) + 1;
                    tracing::debug!(
                        target: "thetadatadx::ffi::streaming",
                        dropped_total = count,
                        "fpss event dropped -- receiver dead (post-reconnect)",
                    );
                }
            })
        {
            set_error(&e.to_string());
            return -1;
        }

        // Store the new rx
        if let Ok(mut guard) = handle.rx.lock() {
            *guard = Some(Arc::new(Mutex::new(rx)));
        }

        // Re-subscribe all previous subscriptions (best-effort; failures are non-fatal,
        // but MUST be surfaced through tracing so ops can see silent re-subscription
        // failures across a reconnect boundary — a dropped subscription here would
        // otherwise manifest as "the stream is up but no ticks for AAPL" with no log
        // trail to diagnose it).
        for (kind, contract) in &saved_subs {
            let result = match kind {
                thetadatadx::fpss::protocol::SubscriptionKind::Quote => {
                    handle.inner.subscribe_quotes(contract)
                }
                thetadatadx::fpss::protocol::SubscriptionKind::Trade => {
                    handle.inner.subscribe_trades(contract)
                }
                thetadatadx::fpss::protocol::SubscriptionKind::OpenInterest => {
                    handle.inner.subscribe_open_interest(contract)
                }
            };
            if let Err(e) = result {
                tracing::warn!(
                    target: "thetadatadx::ffi::reconnect",
                    error = %e,
                    kind = ?kind,
                    root = %contract.root,
                    "resubscribe failed after reconnect"
                );
            }
        }

        for (kind, sec_type) in &saved_full_subs {
            let result = match kind {
                thetadatadx::fpss::protocol::SubscriptionKind::Trade => {
                    handle.inner.subscribe_full_trades(*sec_type)
                }
                thetadatadx::fpss::protocol::SubscriptionKind::OpenInterest => {
                    handle.inner.subscribe_full_open_interest(*sec_type)
                }
                thetadatadx::fpss::protocol::SubscriptionKind::Quote => continue,
            };
            if let Err(e) = result {
                tracing::warn!(
                    target: "thetadatadx::ffi::reconnect",
                    error = %e,
                    kind = ?kind,
                    sec_type = ?sec_type,
                    "full-stream resubscribe failed after reconnect"
                );
            }
        }

        0
    })
}

/// Check if streaming is active on the unified client.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_is_streaming(handle: *const TdxUnified) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            return 0;
        }
        let handle = unsafe { &*handle };
        i32::from(handle.inner.is_streaming())
    })
}

/// Look up a contract by ID. Returns a Display-formatted C string or null.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_contract_lookup(
    handle: *const TdxUnified,
    id: i32,
) -> *mut c_char {
    ffi_boundary!(std::ptr::null_mut(), {
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
            Ok(None) => {
                // Clear last error so callers can distinguish "not found" (empty error)
                // from a real error (non-empty error) when they receive NULL.
                set_error("");
                ptr::null_mut()
            }
            Err(e) => {
                set_error(&e.to_string());
                ptr::null_mut()
            }
        }
    })
}

/// Get active subscriptions as a typed array. Returns null on error.
///
/// Caller must free the result with `tdx_subscription_array_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_active_subscriptions(
    handle: *const TdxUnified,
) -> *mut TdxSubscriptionArray {
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null_mut();
        }
        let handle = unsafe { &*handle };
        match handle.inner.active_subscriptions() {
            Ok(subs) => build_subscription_array(
                subs.iter().map(|(k, c)| (format!("{k:?}"), format!("{c}"))),
            ),
            Err(e) => {
                set_error(&e.to_string());
                ptr::null_mut()
            }
        }
    })
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
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null_mut();
        }
        let handle = unsafe { &*handle };
        let rx_guard = handle
            .rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let rx_arc = if let Some(arc) = rx_guard.as_ref() {
            Arc::clone(arc)
        } else {
            set_error("streaming not started -- call tdx_unified_start_streaming() first");
            return ptr::null_mut();
        };
        drop(rx_guard);
        let rx = rx_arc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let timeout = std::time::Duration::from_millis(timeout_ms);
        match rx.recv_timeout(timeout) {
            Ok(buffered) => {
                // Box the entire FfiBufferedEvent so _detail_string/_raw_payload
                // stay alive. Cast to *mut TdxFpssEvent (first field) for FFI.
                let ptr = Box::into_raw(Box::new(buffered));
                ptr.cast::<TdxFpssEvent>()
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ptr::null_mut(),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => ptr::null_mut(),
        }
    })
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
/// `MddsClient`, and `ThetaDataDx` Derefs to `&MddsClient`.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_historical(handle: *const TdxUnified) -> *const TdxClient {
    ffi_boundary!(std::ptr::null(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null();
        }
        let handle = unsafe { &*handle };
        // TdxClient is #[repr(transparent)] over MddsClient, so this cast is safe.
        let direct_ref: &thetadatadx::mdds::MddsClient = &handle.inner;
        std::ptr::from_ref::<thetadatadx::mdds::MddsClient>(direct_ref).cast::<TdxClient>()
    })
}

/// Stop streaming on the unified client. Historical remains available.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_stop_streaming(handle: *const TdxUnified) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }
        let handle = unsafe { &*handle };
        handle.inner.stop_streaming();
        // Clear the rx so next_event knows streaming is stopped.
        if let Ok(mut guard) = handle.rx.lock() {
            *guard = None;
        }
    })
}

/// Cumulative count of FPSS events dropped on this unified handle because
/// the FFI receiver was gone before the callback could deliver the event.
///
/// Survives `tdx_unified_reconnect()` (the counter `Arc` is cloned into
/// every callback). Returns 0 if the handle is null.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_dropped_events(handle: *const TdxUnified) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        unsafe { (*handle).dropped_events.load(Ordering::Relaxed) }
    })
}

/// Free a unified client handle.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_free(handle: *mut TdxUnified) {
    ffi_boundary!((), {
        if !handle.is_null() {
            let handle = unsafe { Box::from_raw(handle) };
            handle.inner.stop_streaming();
            drop(handle);
        }
    })
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
    ffi_boundary!(std::ptr::null_mut(), {
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
        // Pre-allocate the counter Arc so the callback closure (registered
        // with the client before the handle exists) and the returned handle
        // point at the same `AtomicU64`. `tdx_fpss_reconnect` will also
        // clone this Arc into the new callback, preserving the count across
        // reconnects.
        let dropped_events = Arc::new(AtomicU64::new(0));
        let dropped_events_cb = Arc::clone(&dropped_events);

        let client = match thetadatadx::fpss::FpssClient::connect(
            &creds.inner,
            &config.inner.fpss_hosts,
            config.inner.fpss_ring_size,
            config.inner.fpss_flush_mode,
            config.inner.reconnect_policy.clone(),
            config.inner.derive_ohlcvc,
            move |event: &thetadatadx::fpss::FpssEvent| {
                let buffered = fpss_event_to_ffi(event);
                if tx.send(buffered).is_err() {
                    let count = dropped_events_cb.fetch_add(1, Ordering::Relaxed) + 1;
                    tracing::debug!(
                        target: "thetadatadx::ffi::streaming",
                        dropped_total = count,
                        "fpss event dropped -- receiver dead",
                    );
                }
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
            connect_params: FpssConnectParams {
                creds: creds.inner.clone(),
                hosts: config.inner.fpss_hosts.clone(),
                ring_size: config.inner.fpss_ring_size,
                flush_mode: config.inner.fpss_flush_mode,
                reconnect_policy: config.inner.reconnect_policy.clone(),
                derive_ohlcvc: config.inner.derive_ohlcvc,
            },
            dropped_events,
        }))
    })
}

/// Subscribe to quote data for a stock symbol.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_quotes(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match client.subscribe_quotes(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to trade data for a stock symbol.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_trades(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match client.subscribe_trades(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from quote data for a stock symbol.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_quotes(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match client.unsubscribe_quotes(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from trade data for a stock symbol.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_trades(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match client.unsubscribe_trades(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to open interest data for a stock symbol.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_open_interest(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match client.subscribe_open_interest(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from open interest data for a stock symbol.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_open_interest(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let symbol = require_cstr!(symbol, -1);
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = thetadatadx::fpss::protocol::Contract::stock(symbol);
        match client.unsubscribe_open_interest(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to all trades for a security type (full trade stream).
///
/// `sec_type` must be one of: "STOCK", "OPTION", "INDEX".
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_full_trades(
    handle: *const TdxFpssHandle,
    sec_type: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let sec_type_str = require_cstr!(sec_type, -1);
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
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        match client.subscribe_full_trades(st) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to all open interest for a security type (full OI stream).
///
/// `sec_type` must be one of: "STOCK", "OPTION", "INDEX".
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_full_open_interest(
    handle: *const TdxFpssHandle,
    sec_type: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let sec_type_str = require_cstr!(sec_type, -1);
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
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        match client.subscribe_full_open_interest(st) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from all trades for a security type (full trade stream).
///
/// `sec_type` must be one of: "STOCK", "OPTION", "INDEX".
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_full_trades(
    handle: *const TdxFpssHandle,
    sec_type: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let sec_type_str = require_cstr!(sec_type, -1);
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
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        match client.unsubscribe_full_trades(st) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from all open interest for a security type (full OI stream).
///
/// `sec_type` must be one of: "STOCK", "OPTION", "INDEX".
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_full_open_interest(
    handle: *const TdxFpssHandle,
    sec_type: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let sec_type_str = require_cstr!(sec_type, -1);
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
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        match client.unsubscribe_full_open_interest(st) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Check if the FPSS client is currently authenticated.
///
/// Returns 1 if authenticated, 0 if not (or if handle is null).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_is_authenticated(handle: *const TdxFpssHandle) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            return 0;
        }
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match guard.as_ref() {
            Some(c) => i32::from(c.is_authenticated()),
            None => 0,
        }
    })
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
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return ptr::null_mut();
        }
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return ptr::null_mut();
        };
        match client.contract_lookup(id) {
            Some(contract) => {
                let s = format!("{contract}");
                match CString::new(s) {
                    Ok(cs) => cs.into_raw(),
                    Err(_) => ptr::null_mut(),
                }
            }
            None => {
                // Clear last error so callers can distinguish "not found" (empty error)
                // from a real error (non-empty error) when they receive NULL.
                set_error("");
                ptr::null_mut()
            }
        }
    })
}

/// Get a snapshot of currently active subscriptions.
///
/// Returns a heap-allocated `TdxSubscriptionArray` (null on error).
/// Caller must free the result with `tdx_subscription_array_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_active_subscriptions(
    handle: *const TdxFpssHandle,
) -> *mut TdxSubscriptionArray {
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return ptr::null_mut();
        }
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return ptr::null_mut();
        };
        let subs = client.active_subscriptions();
        build_subscription_array(
            subs.into_iter()
                .map(|(kind, contract)| (format!("{kind:?}"), format!("{contract}"))),
        )
    })
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
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return ptr::null_mut();
        }
        let handle = unsafe { &*handle };
        let rx = handle
            .rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let timeout = std::time::Duration::from_millis(timeout_ms);
        match rx.recv_timeout(timeout) {
            Ok(buffered) => {
                // Box the entire FfiBufferedEvent so _detail_string/_raw_payload
                // stay alive. Cast to *mut TdxFpssEvent (first field) for FFI.
                let ptr = Box::into_raw(Box::new(buffered));
                ptr.cast::<TdxFpssEvent>()
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ptr::null_mut(),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => ptr::null_mut(),
        }
    })
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
    ffi_boundary!((), {
        if !event.is_null() {
            // The pointer was created by boxing a FfiBufferedEvent and casting
            // to *mut TdxFpssEvent (which is the first field). Cast back to
            // free the entire FfiBufferedEvent including owned _detail_string
            // and _raw_payload.
            drop(unsafe { Box::from_raw(event.cast::<FfiBufferedEvent>()) });
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  FPSS — Option-level subscribe/unsubscribe (Gap 1)
// ═══════════════════════════════════════════════════════════════════════

/// Helper: parse the four option-contract strings from C FFI pointers.
///
/// Returns `(symbol, expiration, strike, right)` as `&str` slices,
/// or sets the FFI error and returns `None`.
unsafe fn parse_option_args<'a>(
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> Option<(&'a str, &'a str, &'a str, &'a str)> {
    let sym = require_cstr!(symbol, None);
    let exp = require_cstr!(expiration, None);
    let stk = require_cstr!(strike, None);
    let rt = require_cstr!(right, None);
    Some((sym, exp, stk, rt))
}

/// Subscribe to quote data for an option contract.
///
/// `expiration`: YYYYMMDD, `strike`: e.g. "500" or "17.5", `right`: "C" or "P".
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_option_quotes(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match client.subscribe_quotes(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to trade data for an option contract.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_option_trades(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match client.subscribe_trades(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Subscribe to open interest data for an option contract.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe_option_open_interest(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match client.subscribe_open_interest(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from quote data for an option contract.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_option_quotes(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match client.unsubscribe_quotes(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from trade data for an option contract.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_option_trades(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match client.unsubscribe_trades(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Unsubscribe from open interest data for an option contract.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe_option_open_interest(
    handle: *const TdxFpssHandle,
    symbol: *const c_char,
    expiration: *const c_char,
    strike: *const c_char,
    right: *const c_char,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let (sym, exp, stk, rt) =
            if let Some(args) = unsafe { parse_option_args(symbol, expiration, strike, right) } {
                args
            } else {
                return -1;
            };
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, exp, stk, rt) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };
        match client.unsubscribe_open_interest(&contract) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  FPSS — contract_map (Gap 2) and reconnect (Gap 3)
// ═══════════════════════════════════════════════════════════════════════

/// Get the full contract map.
///
/// Returns a heap-allocated `TdxContractMapArray` (null on error).
/// Caller must free the result with `tdx_contract_map_array_free`.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_contract_map(
    handle: *const TdxFpssHandle,
) -> *mut TdxContractMapArray {
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return ptr::null_mut();
        }
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("FPSS client is shut down");
            return ptr::null_mut();
        };
        build_contract_map_array(
            client
                .contract_map()
                .into_iter()
                .map(|(id, contract)| (id, format!("{contract}"))),
        )
    })
}

/// Reconnect the FPSS streaming client, re-subscribing all previous subscriptions.
///
/// This is the FFI-safe version of reconnect: it reuses the same credentials
/// and config from the initial connect. The callback-based version stays Rust-only.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
///
/// # Event continuity
///
/// Events buffered in the old streaming channel are dropped during reconnect.
/// There is no gap-free delivery guarantee across reconnections. Callers that
/// require gap-free streaming should implement their own sequence-number-based
/// gap detection and replay logic.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_reconnect(handle: *const TdxFpssHandle) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let handle = unsafe { &*handle };
        let params = &handle.connect_params;

        // 1. Save active subscriptions from the current client
        let (saved_subs, saved_full_subs) = {
            let guard = handle
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match guard.as_ref() {
                Some(c) => (c.active_subscriptions(), c.active_full_subscriptions()),
                None => (Vec::new(), Vec::new()),
            }
        };

        // 2. Shut down the old client
        {
            let mut guard = handle
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(old) = guard.take() {
                old.shutdown();
            }
        }

        // 3. Create a new mpsc channel and connect
        let (tx, rx) = std::sync::mpsc::channel::<FfiBufferedEvent>();
        // Clone the handle-level counter so the drop count survives
        // reconnect (same reasoning as Python / TS SDKs; see the struct-
        // level field doc for the full rationale).
        let dropped_events = Arc::clone(&handle.dropped_events);

        let new_client = match thetadatadx::fpss::FpssClient::connect(
            &params.creds,
            &params.hosts,
            params.ring_size,
            params.flush_mode,
            params.reconnect_policy.clone(),
            params.derive_ohlcvc,
            move |event: &thetadatadx::fpss::FpssEvent| {
                let buffered = fpss_event_to_ffi(event);
                if tx.send(buffered).is_err() {
                    let count = dropped_events.fetch_add(1, Ordering::Relaxed) + 1;
                    tracing::debug!(
                        target: "thetadatadx::ffi::streaming",
                        dropped_total = count,
                        "fpss event dropped -- receiver dead (post-reconnect)",
                    );
                }
            },
        ) {
            Ok(c) => c,
            Err(e) => {
                set_error(&e.to_string());
                return -1;
            }
        };

        // 4. Re-subscribe all previous subscriptions (best-effort; failures are non-fatal,
        // but MUST be surfaced through tracing so ops can see silent re-subscription
        // failures across a reconnect boundary — mirrors the same diagnostic the
        // unified reconnect path above emits).
        for (kind, contract) in &saved_subs {
            let result = match kind {
                thetadatadx::fpss::protocol::SubscriptionKind::Quote => {
                    new_client.subscribe_quotes(contract)
                }
                thetadatadx::fpss::protocol::SubscriptionKind::Trade => {
                    new_client.subscribe_trades(contract)
                }
                thetadatadx::fpss::protocol::SubscriptionKind::OpenInterest => {
                    new_client.subscribe_open_interest(contract)
                }
            };
            if let Err(e) = result {
                tracing::warn!(
                    target: "thetadatadx::ffi::reconnect",
                    error = %e,
                    kind = ?kind,
                    root = %contract.root,
                    "resubscribe failed after reconnect"
                );
            }
        }

        for (kind, sec_type) in &saved_full_subs {
            let result = match kind {
                thetadatadx::fpss::protocol::SubscriptionKind::Trade => {
                    new_client.subscribe_full_trades(*sec_type)
                }
                thetadatadx::fpss::protocol::SubscriptionKind::OpenInterest => {
                    new_client.subscribe_full_open_interest(*sec_type)
                }
                thetadatadx::fpss::protocol::SubscriptionKind::Quote => continue,
            };
            if let Err(e) = result {
                tracing::warn!(
                    target: "thetadatadx::ffi::reconnect",
                    error = %e,
                    kind = ?kind,
                    sec_type = ?sec_type,
                    "full-stream resubscribe failed after reconnect"
                );
            }
        }

        // 5. Store the new client and rx
        {
            let mut guard = handle
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = Some(new_client);
        }
        {
            let mut rx_guard = handle
                .rx
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // The old rx is dropped here; any buffered events from the old connection
            // are lost, which is expected behavior for reconnection.
            *rx_guard = rx;
        }

        0
    })
}

/// Cumulative count of FPSS events dropped on this handle because the FFI
/// receiver was gone (channel disconnected) before the callback could deliver
/// the event.
///
/// The counter lives on the handle, so the value survives
/// `tdx_fpss_reconnect()` — callers get a true lifetime total for drops
/// across the initial connection and every subsequent reconnect.
///
/// Returns 0 if the handle is null (defensive; matches the Python
/// `tdx.dropped_events()` contract of "always callable, never panic").
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_dropped_events(handle: *const TdxFpssHandle) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        unsafe { (*handle).dropped_events.load(Ordering::Relaxed) }
    })
}

/// Shut down the FPSS client, stopping all background threads.
///
/// The handle remains valid for `tdx_fpss_free()` but all subsequent operations
/// will return errors.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_shutdown(handle: *const TdxFpssHandle) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }
        let handle = unsafe { &*handle };
        let mut guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(client) = guard.take() {
            client.shutdown();
        }
    })
}

/// Free a FPSS handle. Must be called after `tdx_fpss_shutdown()`.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_free(handle: *mut TdxFpssHandle) {
    ffi_boundary!((), {
        if !handle.is_null() {
            drop(unsafe { Box::from_raw(handle) });
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Test-only panic entry points (feature `testing-panic-boundary`)
//
//  These exist purely so the integration test at
//  `ffi/tests/panic_boundary.rs` can prove that panics inside an
//  `extern "C"` body:
//    1. do NOT abort the process (the test binary would crash),
//    2. return the declared default (`-1` here, matching the
//       existing `i32` status-code convention),
//    3. make the panic payload retrievable via `tdx_last_error()`.
//
//  The symbols are only compiled in when the feature is enabled, so
//  the shared library shipped to Go / C++ / Python consumers never
//  carries a "panic-on-demand" entry point.
// ═══════════════════════════════════════════════════════════════════════

/// Deliberately panic with a `&'static str` payload. Returns -1 via the
/// boundary's default handler. The panic message becomes part of the
/// `tdx_last_error()` string so the caller can verify the downcast path
/// that handles `&'static str` payloads works end to end.
#[cfg(feature = "testing-panic-boundary")]
#[no_mangle]
pub extern "C" fn tdx_test_panic_str() -> i32 {
    ffi_boundary!(-1, {
        panic!("intentional test panic via &'static str");
    })
}

/// Deliberately panic with a heap-allocated `String` payload. Returns -1
/// via the boundary's default handler. Separate from the `&'static str`
/// variant so the test suite can exercise both `downcast_ref::<&'static
/// str>` and `downcast_ref::<String>` branches of the macro.
#[cfg(feature = "testing-panic-boundary")]
#[no_mangle]
pub extern "C" fn tdx_test_panic_string() -> i32 {
    ffi_boundary!(-1, {
        panic!("{}", String::from("intentional test panic via String"));
    })
}
