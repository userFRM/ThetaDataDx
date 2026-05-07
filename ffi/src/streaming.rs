//! FPSS streaming and unified client surface.
//!
//! Contains the streaming-specific handles (`TdxUnified`, `TdxFpssHandle`),
//! the `#[repr(C)]` FPSS event types (generated — `include!`'d), the tagged
//! subscription / contract-map arrays, and every `tdx_unified_*` /
//! `tdx_fpss_*` `extern "C" fn`.
//!
//! # Callback C ABI
//!
//! Both [`TdxUnified`] and [`TdxFpssHandle`] expose a pair of callback-
//! registration entry points that wire user `extern "C"` functions through
//! the SSOT [`thetadatadx::fpss::StreamingDispatcher`]:
//!
//! - `tdx_*_set_callback` — queued: the FPSS reader thread pushes onto a
//!   bounded `crossbeam_channel::bounded(8192)` queue inside the
//!   dispatcher; a dedicated drain thread invokes the user callback.
//!   The reader thread never blocks on user code, so a slow C/C++
//!   callback fills the queue and overflow events are silently dropped
//!   (with the drop count exposed via `tdx_*_dropped_events`).
//! - `tdx_*_set_inline_callback` — inline: the user callback fires
//!   directly on the FPSS reader thread. Microsecond-budget contract:
//!   any allocation, I/O, or lock acquisition will stall the reader and
//!   cause the vendor session to drop. Use only for trading loops with
//!   provably wait-free callbacks.
//!
//! The poll-based `tdx_*_next_event` API and its supporting `mpsc`
//! pipeline have been removed; the C ABI is callback-only.

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicU8, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use crate::error::set_error;
use crate::runtime;
use crate::types::{TdxClient, TdxConfig, TdxCredentials};

// ── Callback C ABI types ──

/// User callback signature: invoked once per FPSS event delivered to the
/// FFI layer. The `event` pointer is valid only for the duration of the
/// call; copy any fields the caller wants to outlive the callback.
///
/// `ctx` is the opaque user context pointer registered alongside the
/// callback (`tdx_*_set_callback(handle, fn, ctx)`); it is passed back
/// unchanged on every invocation.
pub type TdxFpssCallback = extern "C" fn(event: *const TdxFpssEvent, ctx: *mut c_void);

/// Bundle of `(callback, ctx)` stored inside a Rust closure registered
/// with [`thetadatadx::ThetaDataDx::start_streaming`]. The bundle is
/// `Send + Sync + Copy` so the dispatcher drain thread (or, for inline
/// mode, the FPSS reader thread) can call into the user's
/// `extern "C" fn` from a non-FFI thread, and so the same bundle can be
/// re-registered on `tdx_*_reconnect` without re-invoking the user.
#[derive(Clone, Copy)]
struct FfiCallback {
    callback: TdxFpssCallback,
    ctx: *mut c_void,
}

// SAFETY: `FfiCallback` is `Send + Sync` because the contained pointer
// is the user's opaque context — it is never dereferenced by Rust, only
// handed back to the user's `extern "C" fn` exactly as registered.
// Thread affinity of the context is the user's responsibility
// (documented on `tdx_*_set_callback`).
unsafe impl Send for FfiCallback {}
unsafe impl Sync for FfiCallback {}

/// Dispatch mode chosen at `tdx_*_set_callback` time. Stored on the
/// handle so `tdx_*_reconnect` can re-register the same callback on the
/// new stream without forcing the user to re-supply it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DispatchMode {
    Queued,
    Inline,
}

impl FfiCallback {
    fn invoke(&self, event: &thetadatadx::fpss::FpssEvent) {
        // Convert the Rust event to the FFI `#[repr(C)]` struct. The
        // returned `FfiBufferedEvent` owns the heap memory backing the
        // `control.detail` and `raw_data.payload` pointer fields; it is
        // dropped at the end of this function, after the user callback
        // returns. The user MUST NOT retain the `*const TdxFpssEvent`
        // pointer past the callback boundary.
        let buffered = fpss_event_to_ffi(event);
        let event_ptr: *const TdxFpssEvent = std::ptr::from_ref(&buffered.event);
        (self.callback)(event_ptr, self.ctx);
    }
}

// ── Unified + FPSS handles ──

/// Opaque unified client handle — wraps both historical and streaming.
pub struct TdxUnified {
    inner: thetadatadx::ThetaDataDx,
    /// Callback registered via `tdx_unified_set_callback` /
    /// `tdx_unified_set_inline_callback`. `None` until the first
    /// registration; persisted across `tdx_unified_reconnect` so the
    /// reconnect path can re-attach the same C user function without
    /// re-asking the caller for it.
    callback: Mutex<Option<(FfiCallback, DispatchMode)>>,
}

/// FPSS handle lifecycle state — see [`TdxFpssHandle::state`].
///
/// The C ABI documents a strict three-state machine on every FPSS
/// handle. `tdx_fpss_set_callback`, `_set_inline_callback`, and
/// `_reconnect` enforce the transitions; `tdx_fpss_shutdown` is
/// terminal (no further registration / reconnect / shutdown calls
/// succeed).
const FPSS_STATE_FRESH: u8 = 0;
const FPSS_STATE_ACTIVE: u8 = 1;
const FPSS_STATE_SHUTDOWN: u8 = 2;

/// Opaque FPSS streaming client handle.
///
/// `tdx_fpss_connect` allocates the handle and stores connection
/// parameters; the actual FPSS TLS connection is opened on the first
/// call to `tdx_fpss_set_callback` or `tdx_fpss_set_inline_callback`.
/// This mirrors the unified handle's lifecycle (`connect` then
/// `set_callback`) and keeps a single connect-time decision point for
/// queued vs. inline dispatch.
///
/// # Lifecycle state machine
///
/// `state` enforces the public C ABI contract:
///
/// - `FPSS_STATE_FRESH`  -> `FPSS_STATE_ACTIVE` on the first successful
///   `tdx_fpss_set_callback` / `tdx_fpss_set_inline_callback`. A second
///   registration on an already-`ACTIVE` handle returns -1 with
///   "FPSS callback already installed -- only one set_callback call is
///   permitted per handle".
/// - `FPSS_STATE_ACTIVE` -> `FPSS_STATE_SHUTDOWN` on
///   `tdx_fpss_shutdown`. Shutdown is terminal: every subsequent
///   register / reconnect / shutdown call returns -1 with
///   "FPSS handle has already been shut down -- this is terminal".
/// - `FPSS_STATE_FRESH` directly to `FPSS_STATE_SHUTDOWN` is allowed
///   (caller shut down a handle before installing a callback).
pub struct TdxFpssHandle {
    inner: Arc<Mutex<Option<thetadatadx::fpss::FpssClient>>>,
    /// Saved connection parameters used at `set_callback` time and on
    /// every subsequent `tdx_fpss_reconnect`.
    connect_params: FpssConnectParams,
    /// Dispatcher attached when the user installs a queued callback.
    /// `None` for inline-mode handles or before any callback is set.
    dispatcher: Mutex<Option<thetadatadx::fpss::StreamingDispatcher>>,
    /// User callback recorded at `tdx_fpss_set_callback` /
    /// `tdx_fpss_set_inline_callback` time. Stored on the handle so
    /// `tdx_fpss_reconnect` can re-register the same callback on the
    /// new FPSS connection without forcing the caller to re-supply it.
    callback: Mutex<Option<(FfiCallback, DispatchMode)>>,
    /// Permanent lifecycle state — separate from `inner` so that a
    /// post-shutdown reconnect (which would re-populate `inner`) is
    /// rejected before any work happens. `Relaxed` ordering is
    /// sufficient because state transitions are coordinated by the
    /// inner `Mutex`es around the actual resources; `state` is purely
    /// observational from the perspective of the C ABI fast paths.
    state: AtomicU8,
}

/// Saved FPSS connection parameters for FFI-safe (re)connection.
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
/// Constructed once per delivered FPSS event inside the user-callback
/// boundary (see `FfiCallback::invoke`); lives only for the duration of
/// the user's `extern "C" fn` call and is dropped immediately after.
///
/// `_detail_string` and `_raw_payload` own the backing memory for
/// pointer fields inside `event.control.detail` and
/// `event.raw_data.payload` respectively. Users MUST NOT retain those
/// pointers past the callback boundary.
#[repr(C)]
pub(crate) struct FfiBufferedEvent {
    pub(crate) event: TdxFpssEvent,
    /// Owns the `CString` backing `event.control.detail`, if any.
    _detail_string: Option<CString>,
    /// Owns the raw payload bytes backing `event.raw_data.payload`, if any.
    _raw_payload: Option<Vec<u8>>,
}

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

// ═══════════════════════════════════════════════════════════════════════
//  Unified client — historical + streaming through one handle
// ═══════════════════════════════════════════════════════════════════════

/// Connect to `ThetaData` (historical only — FPSS streaming is NOT started).
///
/// Authenticates once, opens gRPC channel. Call `tdx_unified_set_callback()`
/// or `tdx_unified_set_inline_callback()`
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
                callback: Mutex::new(None),
            })),
            Err(e) => {
                set_error(&e.to_string());
                ptr::null_mut()
            }
        }
    })
}

/// Register a queued FPSS callback on the unified client and start streaming.
///
/// `callback` is invoked from the dispatcher's drain thread for every
/// FPSS event delivered by the FPSS reader. The reader thread pushes
/// events onto a bounded `crossbeam_channel` queue (8192 slots) inside
/// the SSOT `StreamingDispatcher`; on overflow the event is dropped and
/// the per-handle drop counter (queryable via `tdx_unified_dropped_events`)
/// ticks. The reader thread NEVER blocks on `callback`.
///
/// # `ctx` lifetime + thread affinity
///
/// `ctx` is an opaque pointer passed back unchanged on every invocation.
/// It MUST remain valid from this call until either
/// (a) `tdx_unified_stop_streaming` returns / `tdx_unified_free` returns,
/// or (b) a successful subsequent call to `tdx_unified_set_callback` /
/// `tdx_unified_set_inline_callback` replaces it. Pass NULL if the
/// callback does not need a context.
///
/// `ctx` is accessed from the dispatcher drain thread (NOT the FPSS
/// reader thread). The dispatcher invokes `callback(event, ctx)`
/// serially on a single drain thread, so the user does not need
/// internal locks for callback-private state. Freeing `ctx` early is
/// undefined behavior.
///
/// The `event` pointer handed to `callback` is valid only for the
/// duration of that invocation. Copy any fields the consumer wants to
/// outlive the callback before returning.
///
/// # Lifecycle contract (REPLACEMENT after stop)
///
/// After `tdx_unified_stop_streaming` the unified client accepts a
/// fresh `tdx_unified_set_callback` / `_set_inline_callback`; the new
/// `(callback, ctx)` REPLACES the saved registration. This is
/// intentionally different from the FPSS-handle one-shot rule: the
/// unified path is the high-level API, where stop+restart is a normal
/// user flow (`tdx_unified_reconnect` is built on top of it).
///
/// On the first call after `tdx_unified_connect` this is the initial
/// registration. Calling `tdx_unified_set_callback` while streaming is
/// already active returns -1 with `"streaming already started"`.
///
/// Returns 0 on success, -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_set_callback(
    handle: *const TdxUnified,
    callback: TdxFpssCallback,
    ctx: *mut c_void,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let handle = unsafe { &*handle };
        let cb = FfiCallback { callback, ctx };
        match handle
            .inner
            .start_streaming(move |event: &thetadatadx::fpss::FpssEvent| {
                cb.invoke(event);
            }) {
            Ok(()) => {
                // Persist the callback for `tdx_unified_reconnect` to
                // re-register on the new FPSS connection without
                // re-asking the caller. Lock cannot be poisoned here
                // because no other thread holds it during connect.
                let mut guard = handle
                    .callback
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *guard = Some((cb, DispatchMode::Queued));
                0
            }
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Register an inline FPSS callback on the unified client and start streaming.
///
/// `callback` fires directly on the FPSS reader thread, bypassing the
/// dispatcher queue. Microsecond-budget contract: any allocation, I/O,
/// lock acquisition, or runtime/GC interaction in the callback will
/// stall the reader, fill the kernel TCP receive buffer, and cause the
/// vendor session to drop. Use only for trading loops with provably
/// wait-free callbacks.
///
/// For every other workload (Python/Node/Go bindings, file logging,
/// WebSocket fan-out), call `tdx_unified_set_callback` instead.
///
/// # `ctx` lifetime + thread affinity
///
/// `ctx` is an opaque pointer passed back unchanged on every invocation.
/// It MUST remain valid from this call until either
/// (a) `tdx_unified_stop_streaming` returns / `tdx_unified_free` returns,
/// or (b) a successful subsequent call to `tdx_unified_set_callback` /
/// `tdx_unified_set_inline_callback` replaces it.
///
/// `ctx` is accessed from the FPSS reader thread directly (NOT a
/// dispatcher drain thread). The reader invokes `callback(event, ctx)`
/// serially on a single thread, so the user does not need internal
/// locks for callback-private state. Freeing `ctx` early is undefined
/// behavior.
///
/// # Lifecycle contract (REPLACEMENT after stop)
///
/// Same replacement-after-stop semantics as
/// [`tdx_unified_set_callback`]: a fresh registration after
/// `tdx_unified_stop_streaming` REPLACES the saved `(callback, ctx)`.
///
/// Returns 0 on success, -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_set_inline_callback(
    handle: *const TdxUnified,
    callback: TdxFpssCallback,
    ctx: *mut c_void,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let handle = unsafe { &*handle };
        let cb = FfiCallback { callback, ctx };
        match handle
            .inner
            .start_streaming_inline(move |event: &thetadatadx::fpss::FpssEvent| {
                cb.invoke(event);
            }) {
            Ok(()) => {
                let mut guard = handle
                    .callback
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *guard = Some((cb, DispatchMode::Inline));
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
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
//  Unified — reconnect (Gap 3)
// ═══════════════════════════════════════════════════════════════════════

/// Reconnect the unified client's streaming connection.
///
/// Saves active subscriptions, stops the current streaming, starts a new one
/// using the previously-registered callback (queued or inline), and
/// re-subscribes everything.
///
/// Requires that a callback has already been installed via
/// `tdx_unified_set_callback` or `tdx_unified_set_inline_callback`.
/// Returns -1 if no callback was registered (the new ABI has no
/// out-of-band buffer to fall back on).
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
///
/// # Event continuity
///
/// Events still queued in the dispatcher when reconnect is invoked are
/// drained before the dispatcher shuts down; events buffered inside the
/// old TLS read path are lost. There is no gap-free delivery guarantee
/// across reconnections — callers that require gap-free streaming
/// should implement sequence-number-based gap detection and replay.
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

        // Look up the previously-registered callback so we can re-attach
        // it on the new FPSS connection. `tdx_unified_reconnect` requires
        // a prior `set_callback` / `set_inline_callback` — without one
        // there is no destination for the new stream's events.
        let (cb, mode) = {
            let guard = handle
                .callback
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match *guard {
                Some(pair) => pair,
                None => {
                    set_error(
                        "no callback registered -- call tdx_unified_set_callback or \
                         tdx_unified_set_inline_callback before tdx_unified_reconnect",
                    );
                    return -1;
                }
            }
        };

        // Stop the current streaming connection (drains the dispatcher,
        // tears down the FPSS reader thread). The next start_streaming
        // call below opens a fresh connection bound to the same
        // C callback.
        handle.inner.stop_streaming();

        let result =
            match mode {
                DispatchMode::Queued => {
                    handle
                        .inner
                        .start_streaming(move |event: &thetadatadx::fpss::FpssEvent| {
                            cb.invoke(event);
                        })
                }
                DispatchMode::Inline => handle.inner.start_streaming_inline(
                    move |event: &thetadatadx::fpss::FpssEvent| {
                        cb.invoke(event);
                    },
                ),
            };
        if let Err(e) = result {
            set_error(&e.to_string());
            return -1;
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
                    root = %contract.symbol,
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
///
/// Drains the dispatcher, joins the drain thread, and tears down the FPSS
/// reader. The previously-registered callback is preserved so a
/// subsequent `tdx_unified_reconnect` can re-attach it without the
/// caller re-supplying the function pointer.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_stop_streaming(handle: *const TdxUnified) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }
        let handle = unsafe { &*handle };
        handle.inner.stop_streaming();
    })
}

/// Cumulative count of FPSS events dropped by the dispatcher on this
/// unified handle because the bounded `(8192)` queue was full when the
/// FPSS reader thread tried to enqueue an event.
///
/// Returns 0 if the handle is null, no callback has been installed yet,
/// or the inline path was taken (no dispatcher queue exists in inline
/// mode — overflow cannot happen because there is no queue).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_dropped_events(handle: *const TdxUnified) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        unsafe { (*handle).inner.dropped_event_count() }
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

/// Allocate an FPSS handle and stash the connection parameters.
///
/// **Does NOT open the FPSS TLS connection** — connection is deferred
/// until the caller installs a callback via `tdx_fpss_set_callback` or
/// `tdx_fpss_set_inline_callback`. This is required because
/// `FpssClient::connect` registers its event handler at connect time;
/// deferring the connect until callback installation lets us avoid an
/// internal queue.
///
/// Returns null on argument validation failure (check `tdx_last_error()`).
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

        Box::into_raw(Box::new(TdxFpssHandle {
            inner: Arc::new(Mutex::new(None)),
            connect_params: FpssConnectParams {
                creds: creds.inner.clone(),
                hosts: config.inner.fpss.hosts.clone(),
                ring_size: config.inner.fpss.ring_size,
                flush_mode: config.inner.fpss.flush_mode,
                reconnect_policy: config.inner.reconnect.policy.clone(),
                derive_ohlcvc: config.inner.fpss.derive_ohlcvc,
            },
            dispatcher: Mutex::new(None),
            callback: Mutex::new(None),
            state: AtomicU8::new(FPSS_STATE_FRESH),
        }))
    })
}

/// Reject the call if the handle is already past its first
/// registration (`Active`) or has been shut down (`Shutdown`).
///
/// Returns `true` if the caller should proceed (handle is `Fresh`);
/// `false` after setting `tdx_last_error()` to a contract-specific
/// message. Used by `tdx_fpss_set_callback` /
/// `tdx_fpss_set_inline_callback` to enforce one-shot registration
/// and the terminal-shutdown rule.
fn reject_if_not_fresh(handle: &TdxFpssHandle) -> bool {
    match handle.state.load(AtomicOrdering::Relaxed) {
        FPSS_STATE_FRESH => true,
        FPSS_STATE_ACTIVE => {
            set_error(
                "FPSS callback already installed -- only one set_callback call is permitted per handle",
            );
            false
        }
        FPSS_STATE_SHUTDOWN => {
            set_error("FPSS handle has already been shut down -- this is terminal");
            false
        }
        _ => {
            // Unreachable -- state is only ever set to one of the three
            // constants above. Treat as terminal to fail closed.
            set_error("FPSS handle in unknown lifecycle state -- refusing operation");
            false
        }
    }
}

/// Reject the call if the handle has been shut down. Used by
/// `tdx_fpss_reconnect` and `tdx_fpss_shutdown` (the latter to make
/// double-shutdown a clean error rather than silently no-op).
fn reject_if_shutdown(handle: &TdxFpssHandle) -> bool {
    if handle.state.load(AtomicOrdering::Relaxed) == FPSS_STATE_SHUTDOWN {
        set_error("FPSS handle has already been shut down -- this is terminal");
        false
    } else {
        true
    }
}

/// Open the FPSS connection if not already open.
///
/// Internal helper shared by `tdx_fpss_set_callback` and
/// `tdx_fpss_set_inline_callback`. The caller supplies a Rust closure
/// that consumes `FpssEvent` references; this is the closure registered
/// with `FpssClient::connect` and lives for the lifetime of the
/// connection. Returns -1 on connect failure (error already set), 0 on
/// success.
///
/// Lifecycle enforcement (one-shot registration, terminal shutdown)
/// happens upstream in [`reject_if_not_fresh`]; this helper only
/// touches the inner `FpssClient` slot.
fn open_fpss<F>(handle: &TdxFpssHandle, on_event: F) -> i32
where
    F: FnMut(&thetadatadx::fpss::FpssEvent) + Send + 'static,
{
    let mut guard = handle
        .inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.is_some() {
        // Belt-and-suspenders: reject_if_not_fresh should already have
        // caught this at the C ABI entry point. Keep the check so a
        // future caller that bypasses the state gate cannot end up
        // double-connecting silently.
        set_error(
            "FPSS callback already installed -- only one set_callback call is permitted per handle",
        );
        return -1;
    }
    let params = &handle.connect_params;
    match thetadatadx::fpss::FpssClient::connect(
        thetadatadx::fpss::FpssConnectArgs {
            creds: &params.creds,
            hosts: &params.hosts,
            ring_size: params.ring_size,
            flush_mode: params.flush_mode,
            policy: params.reconnect_policy.clone(),
            derive_ohlcvc: params.derive_ohlcvc,
        },
        on_event,
    ) {
        Ok(client) => {
            *guard = Some(client);
            0
        }
        Err(e) => {
            set_error(&e.to_string());
            -1
        }
    }
}

/// Register a queued FPSS callback and open the FPSS connection.
///
/// `callback` is invoked from a dedicated drain thread (the SSOT
/// [`StreamingDispatcher`](thetadatadx::fpss::StreamingDispatcher) drain
/// thread) for every FPSS event the reader pulls off the wire. The
/// reader thread pushes events onto a bounded `crossbeam_channel` queue
/// (8192 slots); on overflow events are dropped silently and counted
/// (queryable via `tdx_fpss_dropped_events`). The reader thread NEVER
/// blocks on `callback`.
///
/// `ctx` is an opaque pointer passed back unchanged on every invocation.
/// It MUST remain valid from this call until either
/// `tdx_fpss_shutdown` returns or `tdx_fpss_free` returns; the dispatcher
/// drain thread accesses it on every event and on every
/// `tdx_fpss_reconnect`. Freeing `ctx` before shutdown is undefined
/// behavior.
///
/// # Lifecycle contract (FPSS one-shot rule)
///
/// May only be called ONCE per handle, and ONLY before
/// `tdx_fpss_shutdown`. Subsequent calls — including any call after
/// shutdown — return -1 with an error message:
///
/// - second register on an already-active handle:
///   `"FPSS callback already installed -- only one set_callback call is permitted per handle"`
/// - register after shutdown:
///   `"FPSS handle has already been shut down -- this is terminal"`
///
/// This is intentionally stricter than the unified C ABI's
/// `tdx_unified_set_callback`, which supports stop-then-re-register as
/// a normal user flow. The FPSS handle is the low-level surface; the
/// unified handle is the high-level surface. See
/// [`tdx_unified_set_callback`] for the replacement contract.
///
/// Returns 0 on success, -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_set_callback(
    handle: *const TdxFpssHandle,
    callback: TdxFpssCallback,
    ctx: *mut c_void,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let handle = unsafe { &*handle };
        if !reject_if_not_fresh(handle) {
            return -1;
        }
        let cb = FfiCallback { callback, ctx };
        // Spawn a `StreamingDispatcher` (queued mode) and register a
        // producer-side closure with `FpssClient::connect`. The
        // dispatcher drain thread owns the user callback; the FPSS
        // reader never blocks on user code.
        let dispatcher = thetadatadx::fpss::StreamingDispatcher::spawn(Box::new(
            move |event: &thetadatadx::fpss::FpssEvent| {
                cb.invoke(event);
            },
        ));
        let producer = dispatcher.producer();
        let rc = open_fpss(handle, move |event: &thetadatadx::fpss::FpssEvent| {
            producer.send(event.clone());
        });
        if rc != 0 {
            // Connect failed -- shut down the dispatcher we just spawned
            // so the drain thread doesn't outlive the failure path. State
            // stays Fresh so the caller can retry once the underlying
            // problem is fixed.
            dispatcher.shutdown();
            return rc;
        }
        let mut dispatch_guard = handle
            .dispatcher
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *dispatch_guard = Some(dispatcher);
        let mut cb_guard = handle
            .callback
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *cb_guard = Some((cb, DispatchMode::Queued));
        // Transition to Active only after every fallible operation has
        // succeeded -- a failed connect leaves the handle Fresh so the
        // caller can retry.
        handle
            .state
            .store(FPSS_STATE_ACTIVE, AtomicOrdering::Relaxed);
        0
    })
}

/// Register an inline FPSS callback and open the FPSS connection.
///
/// `callback` fires directly on the FPSS reader thread, bypassing the
/// dispatcher queue. Microsecond-budget contract: any allocation, I/O,
/// lock acquisition, or runtime/GC interaction will stall the reader,
/// fill the kernel TCP receive buffer, and cause the vendor session to
/// drop. Use only for trading loops with provably wait-free callbacks.
///
/// `ctx` is an opaque pointer passed back unchanged on every invocation.
/// It MUST remain valid from this call until either
/// `tdx_fpss_shutdown` returns or `tdx_fpss_free` returns; the FPSS
/// reader thread accesses it on every event and on every
/// `tdx_fpss_reconnect`. Freeing `ctx` before shutdown is undefined
/// behavior. Inline mode invokes `callback` serially on the FPSS
/// reader thread (no dispatcher queue, no extra thread); the user is
/// responsible for any cross-thread synchronization on `ctx` outside
/// the callback.
///
/// # Lifecycle contract (FPSS one-shot rule)
///
/// Same one-shot / terminal-shutdown rules as
/// [`tdx_fpss_set_callback`]. Subsequent register calls — including
/// any call after shutdown — return -1.
///
/// Returns 0 on success, -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_set_inline_callback(
    handle: *const TdxFpssHandle,
    callback: TdxFpssCallback,
    ctx: *mut c_void,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let handle = unsafe { &*handle };
        if !reject_if_not_fresh(handle) {
            return -1;
        }
        let cb = FfiCallback { callback, ctx };
        let rc = open_fpss(handle, move |event: &thetadatadx::fpss::FpssEvent| {
            cb.invoke(event);
        });
        if rc == 0 {
            let mut cb_guard = handle
                .callback
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *cb_guard = Some((cb, DispatchMode::Inline));
            handle
                .state
                .store(FPSS_STATE_ACTIVE, AtomicOrdering::Relaxed);
        }
        rc
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
            return ptr::null_mut();
        };
        let subs = client.active_subscriptions();
        build_subscription_array(
            subs.into_iter()
                .map(|(kind, contract)| (format!("{kind:?}"), format!("{contract}"))),
        )
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback or tdx_fpss_set_inline_callback first, or has been shut down");
            return -1;
        };
        let contract = match thetadatadx::fpss::protocol::Contract::option(sym, (exp, stk, rt)) {
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
//  FPSS — reconnect (Gap 3)
// ═══════════════════════════════════════════════════════════════════════

/// Reconnect the FPSS streaming client, re-subscribing all previous subscriptions.
///
/// Reuses the credentials/config saved at `tdx_fpss_connect` time and
/// the C callback registered via the most recent
/// `tdx_fpss_set_callback` / `tdx_fpss_set_inline_callback`. Returns
/// -1 if no callback was ever installed or if the handle has been
/// shut down (shutdown is terminal — see [`tdx_fpss_shutdown`]).
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
///
/// # Event continuity
///
/// Events still queued in the dispatcher when reconnect is invoked are
/// drained before the dispatcher shuts down; events buffered inside the
/// old TLS read path are lost. There is no gap-free delivery guarantee
/// across reconnections — callers that require gap-free streaming
/// should implement sequence-number-based gap detection and replay.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_reconnect(handle: *const TdxFpssHandle) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let handle = unsafe { &*handle };
        if !reject_if_shutdown(handle) {
            return -1;
        }
        let params = &handle.connect_params;

        // Look up the previously-registered C callback. Reconnect cannot
        // make forward progress without one — `FpssClient::connect`
        // requires an event handler at construction time.
        let (cb, mode) = {
            let guard = handle
                .callback
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match *guard {
                Some(pair) => pair,
                None => {
                    set_error(
                        "no callback registered -- call tdx_fpss_set_callback or \
                         tdx_fpss_set_inline_callback before tdx_fpss_reconnect",
                    );
                    return -1;
                }
            }
        };

        // 1. Save active subscriptions from the current client (if any).
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

        // 2. Shut down the old client and the old dispatcher (if any).
        {
            let mut guard = handle
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(old) = guard.take() {
                old.shutdown();
            }
        }
        {
            let mut dispatch_guard = handle
                .dispatcher
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(old) = dispatch_guard.take() {
                old.shutdown();
            }
        }

        // 3. Build the new event handler bound to the same C callback,
        // routing through a fresh dispatcher (queued mode) or directly
        // (inline mode).
        let new_dispatcher = match mode {
            DispatchMode::Queued => Some(thetadatadx::fpss::StreamingDispatcher::spawn(Box::new(
                move |event: &thetadatadx::fpss::FpssEvent| {
                    cb.invoke(event);
                },
            ))),
            DispatchMode::Inline => None,
        };

        let new_client = match mode {
            DispatchMode::Queued => {
                // SAFETY: in queued mode we just spawned `new_dispatcher` above,
                // so unwrap is sound.
                let dispatcher = new_dispatcher
                    .as_ref()
                    .expect("dispatcher present for queued mode");
                let producer = dispatcher.producer();
                thetadatadx::fpss::FpssClient::connect(
                    thetadatadx::fpss::FpssConnectArgs {
                        creds: &params.creds,
                        hosts: &params.hosts,
                        ring_size: params.ring_size,
                        flush_mode: params.flush_mode,
                        policy: params.reconnect_policy.clone(),
                        derive_ohlcvc: params.derive_ohlcvc,
                    },
                    move |event: &thetadatadx::fpss::FpssEvent| {
                        producer.send(event.clone());
                    },
                )
            }
            DispatchMode::Inline => thetadatadx::fpss::FpssClient::connect(
                thetadatadx::fpss::FpssConnectArgs {
                    creds: &params.creds,
                    hosts: &params.hosts,
                    ring_size: params.ring_size,
                    flush_mode: params.flush_mode,
                    policy: params.reconnect_policy.clone(),
                    derive_ohlcvc: params.derive_ohlcvc,
                },
                move |event: &thetadatadx::fpss::FpssEvent| {
                    cb.invoke(event);
                },
            ),
        };

        let new_client = match new_client {
            Ok(c) => c,
            Err(e) => {
                // Tear down the dispatcher we just spawned so its drain
                // thread does not outlive the failed reconnect.
                if let Some(d) = new_dispatcher {
                    d.shutdown();
                }
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
                    root = %contract.symbol,
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

        // 5. Store the new client + dispatcher.
        {
            let mut guard = handle
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = Some(new_client);
        }
        if let Some(d) = new_dispatcher {
            let mut dispatch_guard = handle
                .dispatcher
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *dispatch_guard = Some(d);
        }

        0
    })
}

/// Cumulative count of FPSS events dropped by the dispatcher on this
/// handle because the bounded `(8192)` queue was full when the FPSS
/// reader thread tried to enqueue an event.
///
/// Returns 0 if the handle is null, no callback has been installed yet,
/// or the inline path was taken (no dispatcher queue exists in inline
/// mode — overflow cannot happen because there is no queue).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_dropped_events(handle: *const TdxFpssHandle) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        let handle = unsafe { &*handle };
        let guard = handle
            .dispatcher
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_ref()
            .map_or(0, thetadatadx::fpss::StreamingDispatcher::dropped_count)
    })
}

/// Shut down the FPSS client, stopping all background threads.
///
/// # Lifecycle contract (terminal)
///
/// Shutdown is terminal: every subsequent `tdx_fpss_set_callback` /
/// `_set_inline_callback` / `_reconnect` / `_shutdown` call on this
/// handle returns -1 with the error message
/// `"FPSS handle has already been shut down -- this is terminal"`. The
/// handle remains valid for `tdx_fpss_free()` only.
///
/// Idempotency: calling shutdown twice on the same handle is rejected
/// rather than silently no-op'd, so a misuse caller cannot accidentally
/// observe "success" after the resource is gone.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_shutdown(handle: *const TdxFpssHandle) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }
        let handle = unsafe { &*handle };
        if !reject_if_shutdown(handle) {
            // Double-shutdown -- error already set, nothing to drop.
            return;
        }
        // Drop the FPSS reader first so no more events can land in the
        // dispatcher queue, then drain + join the dispatcher drain
        // thread before returning.
        {
            let mut guard = handle
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(client) = guard.take() {
                client.shutdown();
            }
        }
        {
            let mut dispatch_guard = handle
                .dispatcher
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(d) = dispatch_guard.take() {
                d.shutdown();
            }
        }
        // Mark terminal AFTER teardown so any racing register/reconnect
        // attempt that observes Shutdown is guaranteed to see a fully
        // torn-down handle.
        handle
            .state
            .store(FPSS_STATE_SHUTDOWN, AtomicOrdering::Relaxed);
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

#[cfg(test)]
mod tests {
    //! Unit tests for the C ABI callback wiring.
    //!
    //! These tests exercise the `FfiCallback` shim and the
    //! `StreamingDispatcher` integration without opening a real FPSS
    //! TLS connection. They cover the two contracts a downstream C/C++
    //! consumer relies on:
    //!
    //! 1. **`set_callback` semantics**: events handed to `FfiCallback::invoke`
    //!    arrive at the user `extern "C" fn` with the registered `ctx` and a
    //!    valid `*const TdxFpssEvent`. A queued `StreamingDispatcher` plus
    //!    `FfiCallback` should run the callback on the dispatcher's drain
    //!    thread (not the producer thread).
    //! 2. **`set_inline_callback` semantics**: the same `FfiCallback`
    //!    fires synchronously on the calling thread when invoked
    //!    directly, with no intermediate queue.

    use super::*;
    use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// Mutable test context observed from the C-shaped callback. Holds
    /// the captured contract id, thread id observed inside the user
    /// callback (so tests can prove the dispatcher drain thread was
    /// used), and a hit counter.
    struct TestCtx {
        hits: AtomicU64,
        last_kind: AtomicI32,
        callback_thread: AtomicU64,
    }

    extern "C" fn capture_callback(event: *const TdxFpssEvent, ctx: *mut std::os::raw::c_void) {
        // SAFETY: the FFI layer guarantees `event` is non-null for the
        // duration of the call and `ctx` is the pointer registered
        // alongside `capture_callback`.
        assert!(!event.is_null(), "FFI handed null event pointer");
        let ctx = unsafe { &*(ctx.cast::<TestCtx>()) };
        ctx.hits.fetch_add(1, Ordering::Relaxed);
        // Read the kind discriminant via a pointer cast to i32. The
        // `TdxFpssEventKind` enum is `#[repr(C)]` with explicit small
        // integer variants, so the first 4 bytes of `*event` are the
        // tag value. Reading by reference would `move` the non-Copy
        // enum, which `&self` access on a `*const` borrow forbids.
        let kind = unsafe { *event.cast::<i32>() };
        ctx.last_kind.store(kind, Ordering::Relaxed);
        // Record the OS thread id so the test can compare against the
        // caller's thread id and verify queued vs. inline routing.
        let tid = thread_id_u64();
        ctx.callback_thread.store(tid, Ordering::Relaxed);
    }

    fn thread_id_u64() -> u64 {
        // `ThreadId::as_u64` is unstable, so format the Debug form
        // (e.g. "ThreadId(7)") and parse the integer back. Fine for
        // test-only thread-affinity assertions.
        let id = thread::current().id();
        let s = format!("{id:?}");
        // Strip "ThreadId(" / ")" and parse the inner integer.
        let inner = s
            .trim_start_matches("ThreadId(")
            .trim_end_matches(')')
            .trim();
        inner.parse::<u64>().unwrap_or(0)
    }

    fn synthetic_quote_event() -> thetadatadx::fpss::FpssEvent {
        thetadatadx::fpss::FpssEvent::Data(thetadatadx::fpss::FpssData::Quote {
            contract_id: 42,
            contract: Arc::new(thetadatadx::fpss::protocol::Contract::stock("AAPL")),
            ms_of_day: 0,
            bid: 0.0,
            bid_size: 0,
            bid_exchange: 0,
            bid_condition: 0,
            ask: 0.0,
            ask_size: 0,
            ask_exchange: 0,
            ask_condition: 0,
            date: 20260505,
            received_at_ns: 0,
        })
    }

    /// Inline mode: invoking the FfiCallback runs the user fn on the
    /// caller's thread synchronously, with the registered ctx.
    #[test]
    fn ffi_callback_inline_invokes_user_fn_on_caller_thread() {
        let ctx_box = Box::new(TestCtx {
            hits: AtomicU64::new(0),
            last_kind: AtomicI32::new(-1),
            callback_thread: AtomicU64::new(0),
        });
        let ctx_ptr: *mut std::os::raw::c_void = Box::into_raw(ctx_box).cast();
        let cb = FfiCallback {
            callback: capture_callback,
            ctx: ctx_ptr,
        };

        let event = synthetic_quote_event();
        let caller_tid = thread_id_u64();
        cb.invoke(&event);

        // SAFETY: we own `ctx_ptr` and have not freed it yet.
        let ctx_back = unsafe { Box::from_raw(ctx_ptr.cast::<TestCtx>()) };
        assert_eq!(
            ctx_back.hits.load(Ordering::Relaxed),
            1,
            "callback fired once"
        );
        assert_eq!(
            ctx_back.last_kind.load(Ordering::Relaxed),
            TdxFpssEventKind::Quote as i32,
            "callback observed Quote event kind",
        );
        assert_eq!(
            ctx_back.callback_thread.load(Ordering::Relaxed),
            caller_tid,
            "inline callback ran on the caller thread",
        );
    }

    /// Queued mode (dispatcher path): the FfiCallback wired through a
    /// StreamingDispatcher producer fires on the dispatcher's drain
    /// thread, not the producer (caller) thread.
    #[test]
    fn ffi_callback_queued_runs_on_dispatcher_thread() {
        let ctx_box = Box::new(TestCtx {
            hits: AtomicU64::new(0),
            last_kind: AtomicI32::new(-1),
            callback_thread: AtomicU64::new(0),
        });
        let ctx_ptr: *mut std::os::raw::c_void = Box::into_raw(ctx_box).cast();
        let cb = FfiCallback {
            callback: capture_callback,
            ctx: ctx_ptr,
        };

        // Replicate the wiring that `tdx_fpss_set_callback` /
        // `tdx_unified_set_callback` install: the dispatcher's drain
        // thread owns the user callback, the producer side is what the
        // FPSS reader (here, the test thread) calls.
        let dispatcher = thetadatadx::fpss::StreamingDispatcher::spawn(Box::new(
            move |event: &thetadatadx::fpss::FpssEvent| {
                cb.invoke(event);
            },
        ));
        let producer_tid = thread_id_u64();
        {
            // Hold the producer in a tight scope so it is dropped before
            // `dispatcher.shutdown()`. `shutdown` only drops the
            // dispatcher's internal sender, but the cloned producer side
            // would keep the channel alive and the drain-thread join
            // would deadlock waiting for `Receiver::iter` to terminate.
            let producer = dispatcher.producer();
            let event = synthetic_quote_event();
            producer.send(event);
        }

        // Shutdown drains every queued event before joining the drain
        // thread, so by the time `shutdown` returns the callback has
        // fired exactly once.
        dispatcher.shutdown();

        // SAFETY: ctx_ptr still valid until we re-Box below.
        let ctx_back = unsafe { Box::from_raw(ctx_ptr.cast::<TestCtx>()) };
        assert_eq!(
            ctx_back.hits.load(Ordering::Relaxed),
            1,
            "callback fired once via dispatcher drain thread",
        );
        let observed_tid = ctx_back.callback_thread.load(Ordering::Relaxed);
        assert_ne!(
            observed_tid, producer_tid,
            "dispatcher path ran callback on a different thread than the producer (queued semantics)",
        );
    }

    /// Smoke test for `FfiCallback: Send + Sync`. Without these
    /// auto-trait impls, `start_streaming` would refuse to accept a
    /// closure capturing the bundle.
    #[test]
    fn ffi_callback_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FfiCallback>();
    }

    /// `tdx_fpss_dropped_events` returns 0 when no callback is
    /// installed (no dispatcher exists yet).
    #[test]
    fn fpss_dropped_events_zero_before_callback() {
        // Build a minimal handle without going through the
        // `tdx_fpss_connect` boundary (avoids needing valid creds).
        let handle = TdxFpssHandle {
            inner: Arc::new(Mutex::new(None)),
            connect_params: FpssConnectParams {
                creds: thetadatadx::Credentials::new("user", "password"),
                hosts: vec![("localhost".to_owned(), 25503)],
                ring_size: 4096,
                flush_mode: thetadatadx::FpssFlushMode::default(),
                reconnect_policy: thetadatadx::config::ReconnectPolicy::default(),
                derive_ohlcvc: false,
            },
            dispatcher: Mutex::new(None),
            callback: Mutex::new(None),
            state: AtomicU8::new(FPSS_STATE_FRESH),
        };
        let raw = Box::into_raw(Box::new(handle));
        let count = unsafe { tdx_fpss_dropped_events(raw) };
        assert_eq!(count, 0, "no dispatcher means dropped count is 0");
        // SAFETY: we just allocated this handle.
        unsafe { drop(Box::from_raw(raw)) };
    }

    /// HIGH 2 follow-up: the FPSS handle state gate rejects
    /// post-shutdown register / reconnect / shutdown calls without
    /// touching live resources. We exercise the gate directly on a
    /// minimal handle (no live FPSS connect) so the test does not need
    /// network credentials.
    #[test]
    fn fpss_state_gate_rejects_after_shutdown() {
        let handle = TdxFpssHandle {
            inner: Arc::new(Mutex::new(None)),
            connect_params: FpssConnectParams {
                creds: thetadatadx::Credentials::new("user", "password"),
                hosts: vec![("localhost".to_owned(), 25503)],
                ring_size: 4096,
                flush_mode: thetadatadx::FpssFlushMode::default(),
                reconnect_policy: thetadatadx::config::ReconnectPolicy::default(),
                derive_ohlcvc: false,
            },
            dispatcher: Mutex::new(None),
            callback: Mutex::new(None),
            state: AtomicU8::new(FPSS_STATE_SHUTDOWN),
        };
        assert!(
            !reject_if_not_fresh(&handle),
            "register on Shutdown handle must be rejected",
        );
        assert!(
            !reject_if_shutdown(&handle),
            "reconnect / shutdown on Shutdown handle must be rejected",
        );

        // And the Active state rejects fresh-only operations but
        // allows reconnect / shutdown.
        handle
            .state
            .store(FPSS_STATE_ACTIVE, AtomicOrdering::Relaxed);
        assert!(
            !reject_if_not_fresh(&handle),
            "second register on Active handle must be rejected",
        );
        assert!(
            reject_if_shutdown(&handle),
            "reconnect / shutdown on Active handle must be allowed",
        );

        // Fresh allows everything.
        handle
            .state
            .store(FPSS_STATE_FRESH, AtomicOrdering::Relaxed);
        assert!(reject_if_not_fresh(&handle));
        assert!(reject_if_shutdown(&handle));
    }

    /// `tdx_unified_dropped_events` returns 0 on a null handle.
    #[test]
    fn unified_dropped_events_handles_null() {
        let count = unsafe { tdx_unified_dropped_events(std::ptr::null()) };
        assert_eq!(count, 0);

        // And exercising the queued dispatcher: spawn one with a no-op
        // callback, send an event, ensure dropped_count stays 0 (queue
        // has plenty of headroom for one event), then shutdown.
        let dispatcher = thetadatadx::fpss::StreamingDispatcher::spawn(Box::new(|_event| {}));
        dispatcher.producer().send(synthetic_quote_event());
        // Allow the drain thread a moment to consume.
        thread::sleep(Duration::from_millis(20));
        assert_eq!(dispatcher.dropped_count(), 0);
        dispatcher.shutdown();
    }
}
