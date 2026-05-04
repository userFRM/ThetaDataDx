//! FPSS streaming and unified client surface.
//!
//! Contains the streaming-specific handles (`TdxUnified`, `TdxFpssHandle`),
//! the `#[repr(C)]` FPSS event types (generated — `include!`'d), the tagged
//! subscription / contract-map arrays, and every `tdx_unified_*` /
//! `tdx_fpss_*` `extern "C" fn`. Split verbatim from `lib.rs`; the exported
//! C ABI is unchanged.

use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::set_error;
use crate::runtime;
use crate::types::{TdxClient, TdxConfig, TdxCredentials};

// ── Unified + FPSS handles ──

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
pub(crate) struct FfiBufferedEvent {
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
                    symbol = %contract.symbol,
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
        let mdds_ref: &thetadatadx::mdds::MddsClient = &handle.inner;
        std::ptr::from_ref::<thetadatadx::mdds::MddsClient>(mdds_ref).cast::<TdxClient>()
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
                    symbol = %contract.symbol,
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

/// Free a FPSS handle. Must be called after `tdx_fpss_free()`.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_free(handle: *mut TdxFpssHandle) {
    ffi_boundary!((), {
        if !handle.is_null() {
            drop(unsafe { Box::from_raw(handle) });
        }
    })
}
