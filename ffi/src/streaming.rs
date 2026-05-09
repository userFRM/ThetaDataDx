//! FPSS streaming and unified client surface.
//!
//! Contains the streaming-specific handles (`TdxUnified`, `TdxFpssHandle`),
//! the `#[repr(C)]` FPSS event types (generated — `include!`'d), the tagged
//! subscription / contract-map arrays, and every `tdx_unified_*` /
//! `tdx_fpss_*` `extern "C" fn`.
//!
//! # Callback C ABI
//!
//! Both [`TdxUnified`] and [`TdxFpssHandle`] expose a single callback-
//! registration entry point that wires user `extern "C"` functions
//! through the SSOT single-queue pipeline introduced in #513:
//!
//! - `tdx_*_set_callback` — the user callback runs on the LMAX
//!   Disruptor consumer thread, with each invocation wrapped in
//!   [`std::panic::catch_unwind`] so a C/C++ panic does not kill the
//!   consumer. The TLS reader publishes events via
//!   `Producer::try_publish`; on ring overflow the event is dropped and
//!   the drop count is exposed via `tdx_*_dropped_events`. The reader
//!   thread never blocks on user code.
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
/// with [`thetadatadx::ThetaDataDxClient::start_streaming`]. The bundle is
/// `Send + Sync + Copy` so the LMAX Disruptor consumer thread can call
/// into the user's `extern "C" fn` from a non-FFI thread, and so the
/// same bundle can be re-registered on `tdx_*_reconnect` without
/// re-invoking the user.
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

impl FfiCallback {
    fn invoke(&self, event: &thetadatadx::fpss::FpssEvent) {
        // Convert the Rust event to the FFI `#[repr(C)]` struct. The
        // returned `FfiBufferedEvent` owns the heap memory backing every
        // borrowed pointer in the event (`Contract.symbol`,
        // `LoginSuccess.permissions`, `ServerError.message`,
        // `Error.message`, `UnknownFrame.payload`, `Ping.payload`);
        // it is dropped at the end of this function,
        // after the user callback returns. The user MUST NOT retain the
        // `*const TdxFpssEvent` pointer past the callback boundary.
        let buffered = fpss_event_to_ffi(event);
        let event_ptr: *const TdxFpssEvent = std::ptr::from_ref(&buffered.event);
        (self.callback)(event_ptr, self.ctx);
    }
}

// ── Unified + FPSS handles ──

/// Opaque unified client handle — wraps both historical and streaming.
pub struct TdxUnified {
    pub(crate) inner: thetadatadx::ThetaDataDxClient,
    /// Callback registered via `tdx_unified_set_callback`. `None` until
    /// the first registration; persisted across `tdx_unified_reconnect`
    /// so the reconnect path can re-attach the same C user function
    /// without re-asking the caller for it.
    callback: Mutex<Option<FfiCallback>>,
}

/// FPSS handle lifecycle state — see [`TdxFpssHandle::state`].
///
/// The C ABI documents a strict three-state machine on every FPSS
/// handle. `tdx_fpss_set_callback` and `_reconnect` enforce the
/// transitions; `tdx_fpss_shutdown` is terminal (no further
/// registration / reconnect / shutdown calls succeed).
const FPSS_STATE_FRESH: u8 = 0;
const FPSS_STATE_ACTIVE: u8 = 1;
const FPSS_STATE_SHUTDOWN: u8 = 2;

/// Opaque FPSS streaming client handle.
///
/// `tdx_fpss_connect` allocates the handle and stores connection
/// parameters; the actual FPSS TLS connection is opened on the first
/// call to `tdx_fpss_set_callback`. This mirrors the unified handle's
/// lifecycle (`connect` then `set_callback`).
///
/// # Lifecycle state machine
///
/// `state` enforces the public C ABI contract:
///
/// - `FPSS_STATE_FRESH`  -> `FPSS_STATE_ACTIVE` on the first successful
///   `tdx_fpss_set_callback`. A second registration on an already-
///   `ACTIVE` handle returns -1 with "FPSS callback already installed
///   -- only one set_callback call is permitted per handle".
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
    /// User callback recorded at `tdx_fpss_set_callback` time. Stored
    /// on the handle so `tdx_fpss_reconnect` can re-register the same
    /// callback on the new FPSS connection without forcing the caller
    /// to re-supply it.
    callback: Mutex<Option<FfiCallback>>,
    /// Permanent lifecycle state — separate from `inner` so that a
    /// post-shutdown reconnect (which would re-populate `inner`) is
    /// rejected before any work happens. `Relaxed` ordering is
    /// sufficient because state transitions are coordinated by the
    /// inner `Mutex`es around the actual resources; `state` is purely
    /// observational from the perspective of the C ABI fast paths.
    state: AtomicU8,
    /// Quiescence flags for every superseded FPSS session that has
    /// not yet drained, captured during `tdx_fpss_reconnect` /
    /// `tdx_fpss_shutdown` before the previous client is dropped.
    /// `tdx_fpss_await_drain` waits for ALL flags to flip so callers
    /// can confirm every old user callback has stopped firing before
    /// freeing the previous `ctx`. Stacked reconnect/shutdown cycles
    /// layer multiple in-flight generations on top of each other; a
    /// single slot would silently drop earlier still-firing sessions
    /// when a later one retired (PR #514 HIGH-001).
    prev_drained: Mutex<Vec<Arc<std::sync::atomic::AtomicBool>>>,
}

/// Saved FPSS connection parameters for FFI-safe (re)connection.
struct FpssConnectParams {
    creds: thetadatadx::Credentials,
    hosts: Vec<(String, u16)>,
    ring_size: usize,
    flush_mode: thetadatadx::FpssFlushMode,
    reconnect_policy: thetadatadx::config::ReconnectPolicy,
    derive_ohlcvc: bool,
    connect_timeout_ms: u64,
    read_timeout_ms: u64,
    ping_interval_ms: u64,
}

// ═══════════════════════════════════════════════════════════════════════
//  #[repr(C)] FPSS streaming event types — zero-copy across FFI
//
//  All of the kind-enum / per-variant struct / ZERO_* const definitions
//  are generated from `crates/thetadatadx/fpss_event_schema.toml`. The
//  hand-written wrapper `FfiBufferedEvent` below owns the backing memory
//  for every borrowed pointer the generated `TdxFpssEvent` exposes
//  (the `Contract.symbol` C strings, the typed control variants'
//  `permissions` / `message` strings, and the `Ping` / `UnknownFrame`
//  byte payloads). Split into two include points so the
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
/// Each backing-storage `Option<...>` slot owns one variant's borrowed
/// pointer payload — `_contract_symbol` for any data event's
/// `Contract.symbol` (or `ContractAssigned.contract.symbol`),
/// `_login_permissions` for `LoginSuccess.permissions`,
/// `_control_message` for `ServerError.message` / `Error.message`
/// (mutually exclusive variants), and `_payload_bytes` for
/// `UnknownFrame.payload` / `Ping.payload`. The
/// borrowed `*const c_char` / `*const u8` pointers in the public
/// `TdxFpssEvent` reference INTO these slots; users MUST NOT retain
/// those pointers past the callback boundary.
#[repr(C)]
pub(crate) struct FfiBufferedEvent {
    pub(crate) event: TdxFpssEvent,
    /// Owns the `CString` backing every data event's `Contract.symbol`
    /// pointer and `ContractAssigned.contract.symbol`.
    _contract_symbol: Option<CString>,
    /// Owns the `CString` backing `LoginSuccess.permissions`.
    _login_permissions: Option<CString>,
    /// Owns the `CString` backing `ServerError.message` / `Error.message`.
    _control_message: Option<CString>,
    /// Owns the byte payload backing `UnknownFrame.payload` /
    /// `Ping.payload` / `RawData.payload`.
    _payload_bytes: Option<Vec<u8>>,
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

        match runtime().block_on(thetadatadx::ThetaDataDxClient::connect(
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
/// `callback` is invoked from the LMAX Disruptor consumer thread for
/// every FPSS event the reader pulls off the wire. Each invocation is
/// wrapped in [`std::panic::catch_unwind`] so a C/C++ callback panic
/// does not kill the consumer. The TLS reader publishes events into a
/// pre-allocated ring via `Producer::try_publish`; on overflow the
/// event is dropped and the per-handle drop counter (queryable via
/// `tdx_unified_dropped_events`) ticks. The reader thread NEVER blocks
/// on `callback`.
///
/// # `ctx` lifetime + thread affinity
///
/// `ctx` is an opaque pointer passed back unchanged on every invocation.
/// It MUST remain valid until ONE of the following barriers completes:
///
/// - `tdx_unified_free` returns. `_free` calls `stop_streaming`
///   internally and polls the post-stop drain barrier with a 5-second
///   timeout, so on a non-overrun return the previous Disruptor
///   consumer has finished firing the callback. In the
///   timeout-overrun path (rare; emits a `tracing::error!`) the
///   consumer may still be firing, so under that diagnostic `ctx`
///   MUST remain valid past return.
/// - `tdx_unified_stop_streaming` / `tdx_unified_reconnect` returns
///   AND `tdx_unified_await_drain` has returned `1` for the prior
///   session.
/// - A successful replacement `tdx_unified_set_callback` has returned
///   AND `tdx_unified_await_drain` has returned `1` (the replacement
///   path races against the prior session's residual events the same
///   way stop / reconnect does).
///
/// Pass NULL if the callback does not need a context.
///
/// `ctx` is accessed from the Disruptor consumer thread (NOT the FPSS
/// TLS reader thread). The consumer invokes `callback(event, ctx)`
/// serially on a single thread, so the user does not need internal
/// locks for callback-private state. Freeing `ctx` early — including
/// the moment `tdx_unified_stop_streaming` / `_reconnect` returns
/// without an intervening `_await_drain`, or before `tdx_unified_free`
/// returns — is undefined behavior: the consumer continues firing
/// in-flight events until its exit path joins.
///
/// The `event` pointer handed to `callback` is valid only for the
/// duration of that invocation. Copy any fields the consumer wants to
/// outlive the callback before returning.
///
/// # Lifecycle contract (REPLACEMENT after stop)
///
/// After `tdx_unified_stop_streaming` the unified client accepts a
/// fresh `tdx_unified_set_callback`; the new `(callback, ctx)` REPLACES
/// the saved registration. This is intentionally different from the
/// FPSS-handle one-shot rule: the unified path is the high-level API,
/// where stop+restart is a normal user flow (`tdx_unified_reconnect`
/// is built on top of it).
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
                *guard = Some(cb);
                0
            }
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Polymorphic subscription request payload
// ═══════════════════════════════════════════════════════════════════════

/// Subscription request scope discriminator.
pub const TDX_SUB_SCOPE_CONTRACT: i32 = 0;
pub const TDX_SUB_SCOPE_FULL: i32 = 1;

/// Per-contract / full-stream tick kind discriminators. The set
/// reachable from each scope is constrained:
///
/// - `TDX_SUB_SCOPE_CONTRACT` accepts `QUOTE`, `TRADE`, `OPEN_INTEREST`.
/// - `TDX_SUB_SCOPE_FULL` accepts `TRADE`, `OPEN_INTEREST` (full-stream
///   quote is rejected — quotes are addressed per-contract only).
pub const TDX_SUB_KIND_QUOTE: i32 = 0;
pub const TDX_SUB_KIND_TRADE: i32 = 1;
pub const TDX_SUB_KIND_OPEN_INTEREST: i32 = 2;

/// Polymorphic subscribe / unsubscribe request payload.
///
/// Mirrors the Rust `Subscription` enum across the C ABI. One struct
/// shape carries every per-contract or full-stream variant.
///
/// - Per-contract stock: `scope = CONTRACT`, `symbol = "AAPL"`, all
///   option fields NULL.
/// - Per-contract option: `scope = CONTRACT`, `symbol = "SPY"`,
///   `expiration = "20260620"`, `strike = "550"`, `right = "C"`.
/// - Full-stream: `scope = FULL`, `sec_type = "OPTION"`, all
///   per-contract fields NULL.
#[repr(C)]
pub struct TdxSubscriptionRequest {
    /// `TDX_SUB_SCOPE_CONTRACT` or `TDX_SUB_SCOPE_FULL`.
    pub scope: i32,
    /// `TDX_SUB_KIND_QUOTE` / `_TRADE` / `_OPEN_INTEREST`.
    pub kind: i32,
    /// Stock or underlying symbol for per-contract subscriptions.
    /// NULL for full-stream.
    pub symbol: *const c_char,
    /// Option-only fields. NULL for non-option per-contract or for
    /// full-stream subscriptions.
    pub expiration: *const c_char,
    pub strike: *const c_char,
    pub right: *const c_char,
    /// `"STOCK"` / `"OPTION"` / `"INDEX"` for full-stream
    /// subscriptions. NULL for per-contract subscriptions.
    pub sec_type: *const c_char,
}

/// Decode a `TdxSubscriptionRequest` into a Rust [`Subscription`]. The
/// helper sets `tdx_last_error` on validation failure and returns
/// `None`. Used by both the unified and standalone-FPSS C ABI entry
/// points.
unsafe fn coerce_subscription(
    req: *const TdxSubscriptionRequest,
) -> Option<thetadatadx::fpss::protocol::Subscription> {
    use thetadatadx::fpss::protocol::{
        Contract, FullSubscriptionKind, Subscription, SubscriptionKind,
    };
    if req.is_null() {
        set_error("subscription request is null");
        return None;
    }
    let req = unsafe { &*req };
    let symbol_ptr = req.symbol;
    let expiration_ptr = req.expiration;
    let strike_ptr = req.strike;
    let right_ptr = req.right;
    let sec_type_ptr = req.sec_type;
    match req.scope {
        TDX_SUB_SCOPE_CONTRACT => {
            let kind = match req.kind {
                TDX_SUB_KIND_QUOTE => SubscriptionKind::Quote,
                TDX_SUB_KIND_TRADE => SubscriptionKind::Trade,
                TDX_SUB_KIND_OPEN_INTEREST => SubscriptionKind::OpenInterest,
                other => {
                    set_error(&format!("invalid kind {other}"));
                    return None;
                }
            };
            let symbol = require_cstr!(symbol_ptr, None);
            let contract =
                if expiration_ptr.is_null() && strike_ptr.is_null() && right_ptr.is_null() {
                    Contract::stock(symbol)
                } else {
                    let exp = require_cstr!(expiration_ptr, None);
                    let stk = require_cstr!(strike_ptr, None);
                    let rt = require_cstr!(right_ptr, None);
                    match Contract::option(symbol, exp, stk, rt) {
                        Ok(c) => c,
                        Err(e) => {
                            set_error(&e.to_string());
                            return None;
                        }
                    }
                };
            Some(Subscription::Contract { contract, kind })
        }
        TDX_SUB_SCOPE_FULL => {
            let kind = match req.kind {
                TDX_SUB_KIND_TRADE => FullSubscriptionKind::Trades,
                TDX_SUB_KIND_OPEN_INTEREST => FullSubscriptionKind::OpenInterest,
                TDX_SUB_KIND_QUOTE => {
                    set_error("full-stream Quote is not a valid subscription");
                    return None;
                }
                other => {
                    set_error(&format!("invalid kind {other}"));
                    return None;
                }
            };
            let sec_type_str = require_cstr!(sec_type_ptr, None);
            let sec_type = match sec_type_str.to_uppercase().as_str() {
                "STOCK" => tdbe::types::enums::SecType::Stock,
                "OPTION" => tdbe::types::enums::SecType::Option,
                "INDEX" => tdbe::types::enums::SecType::Index,
                other => {
                    set_error(&format!(
                        "invalid sec_type {other:?} (expected STOCK, OPTION, INDEX)"
                    ));
                    return None;
                }
            };
            Some(Subscription::Full { sec_type, kind })
        }
        other => {
            set_error(&format!("invalid scope {other}"));
            None
        }
    }
}

/// Polymorphic subscribe on the unified client. Mirrors the Rust
/// `client.subscribe(Subscription)` shape — one entry point handles
/// every per-contract and full-stream variant.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_subscribe(
    handle: *const TdxUnified,
    request: *const TdxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let sub = match unsafe { coerce_subscription(request) } {
            Some(s) => s,
            None => return -1,
        };
        let handle = unsafe { &*handle };
        match handle.inner.subscribe(sub) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Polymorphic unsubscribe on the unified client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_unsubscribe(
    handle: *const TdxUnified,
    request: *const TdxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        let sub = match unsafe { coerce_subscription(request) } {
            Some(s) => s,
            None => return -1,
        };
        let handle = unsafe { &*handle };
        match handle.inner.unsubscribe(sub) {
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
/// using the previously-registered callback, and re-subscribes
/// everything.
///
/// Requires that a callback has already been installed via
/// `tdx_unified_set_callback`. Returns -1 if no callback was registered
/// (the new ABI has no out-of-band buffer to fall back on).
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
///
/// # Event continuity
///
/// Events still pending in the old session's Disruptor ring continue
/// flowing through the previous registration's callback until the old
/// consumer thread joins; events buffered inside the old TLS read
/// path are lost. There is no gap-free delivery guarantee across
/// reconnections — callers that require gap-free streaming should
/// implement sequence-number-based gap detection and replay.
///
/// # Lifecycle restriction
///
/// MUST NOT be called from inside the user callback. The new stream
/// is opened immediately after the swap, but the old consumer keeps
/// firing the previous registration's callback until its exit path
/// joins. From the C ABI side that means freeing or replacing `ctx`
/// based on `tdx_unified_reconnect` returning is unsound — the old
/// callback is still in flight. Drive reconnect from a separate
/// thread and call `tdx_unified_await_drain` between stop and any
/// `ctx` replacement.
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
        // a prior `set_callback` — without one there is no destination
        // for the new stream's events.
        let cb = {
            let guard = handle
                .callback
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match *guard {
                Some(cb) => cb,
                None => {
                    set_error(
                        "no callback registered -- call tdx_unified_set_callback \
                         before tdx_unified_reconnect",
                    );
                    return -1;
                }
            }
        };

        // Initiate teardown of the FPSS reader and Disruptor consumer
        // for the current session. This swaps the streaming slot to
        // `Stopped` and signals the I/O thread; the consumer keeps
        // firing the old C callback until its exit path joins.
        handle.inner.stop_streaming();

        // Wait for the previous consumer thread to finish firing the
        // old callback BEFORE we open a fresh session bound to the
        // same C callback / `ctx`. The C ABI contract guarantees
        // single-threaded callback invocation; without this barrier
        // the old consumer can still be inside the user's `ctx` when
        // the new consumer starts firing on a different thread, and
        // the user's "no internal locks needed" assumption breaks. A
        // 5 s budget matches the FFI free-path drain budget; on
        // timeout we surface the error rather than racing.
        if !handle
            .inner
            .await_drain(std::time::Duration::from_millis(5_000))
        {
            set_error(
                "reconnect drain barrier timed out after 5s — previous \
                 callback is still in flight; refusing to bind the new \
                 session to the same ctx",
            );
            return -1;
        }

        let result = handle
            .inner
            .start_streaming(move |event: &thetadatadx::fpss::FpssEvent| {
                cb.invoke(event);
            });
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
            let result =
                match kind {
                    thetadatadx::fpss::protocol::SubscriptionKind::Quote => handle.inner.subscribe(
                        thetadatadx::fpss::protocol::Subscription::Contract {
                            contract: contract.clone(),
                            kind: thetadatadx::fpss::protocol::SubscriptionKind::Quote,
                        },
                    ),
                    thetadatadx::fpss::protocol::SubscriptionKind::Trade => handle.inner.subscribe(
                        thetadatadx::fpss::protocol::Subscription::Contract {
                            contract: contract.clone(),
                            kind: thetadatadx::fpss::protocol::SubscriptionKind::Trade,
                        },
                    ),
                    thetadatadx::fpss::protocol::SubscriptionKind::OpenInterest => handle
                        .inner
                        .subscribe(thetadatadx::fpss::protocol::Subscription::Contract {
                            contract: contract.clone(),
                            kind: thetadatadx::fpss::protocol::SubscriptionKind::OpenInterest,
                        }),
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
                    handle
                        .inner
                        .subscribe(thetadatadx::fpss::protocol::Subscription::Full {
                            sec_type: *sec_type,
                            kind: thetadatadx::fpss::protocol::FullSubscriptionKind::Trades,
                        })
                }
                thetadatadx::fpss::protocol::SubscriptionKind::OpenInterest => handle
                    .inner
                    .subscribe(thetadatadx::fpss::protocol::Subscription::Full {
                        sec_type: *sec_type,
                        kind: thetadatadx::fpss::protocol::FullSubscriptionKind::OpenInterest,
                    }),
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
/// `MddsClient`, and `ThetaDataDxClient` Derefs to `&MddsClient`.
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
/// Initiates teardown of the FPSS Disruptor consumer thread and the
/// underlying TLS reader, but returns immediately after the streaming
/// state cell is swapped to `Stopped`. The old consumer continues
/// firing the previously-registered C callback for any events still
/// in-flight in the ring buffer until its exit path joins. Use
/// `tdx_unified_await_drain` to confirm the consumer has finished
/// firing the callback before freeing `ctx` or replacing the
/// callback registration. The saved `(callback, ctx)` itself is
/// preserved so a subsequent `tdx_unified_reconnect` can re-attach it
/// without the caller re-supplying the function pointer.
///
/// # Lifecycle restriction
///
/// MUST NOT be called from inside the user callback. Doing so
/// returns control to the caller while the old callback is still
/// firing on the Disruptor consumer thread; freeing or replacing
/// `ctx` based on stop returning will trigger use-after-free in the
/// callback. Drive stop / reconnect from a separate thread instead,
/// then call `tdx_unified_await_drain` for the quiescence barrier.
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

/// Cumulative count of `Producer::try_publish` failures on this
/// unified handle since the current stream started: events the FPSS
/// TLS reader could not enqueue into the LMAX Disruptor ring because
/// the consumer had fallen behind and the ring was full.
///
/// All registrations share the Disruptor ring and can overflow under
/// sustained burst. Operators should poll on a periodic timer
/// (e.g. every second) and emit a `warn` log on any non-zero delta.
///
/// Returns 0 if the handle is null or no callback has been installed
/// yet.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_dropped_events(handle: *const TdxUnified) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        unsafe { (*handle).inner.dropped_event_count() }
    })
}

/// Wait for the previously-superseded streaming session to quiesce.
///
/// Returns `1` once the previous `tdx_unified_stop_streaming` /
/// `_reconnect` session's Disruptor consumer thread has finished
/// firing the registered callback. Returns `0` on timeout or when no
/// stream has been stopped on this handle.
///
/// # When to call
///
/// After `tdx_unified_stop_streaming` or `tdx_unified_reconnect`
/// returns, before freeing `ctx` or registering a fresh callback whose
/// captures must not alias the previous registration's still-running
/// invocations.
///
/// # Lifecycle restriction
///
/// MUST be called from a thread other than the FPSS Disruptor
/// consumer thread. Calling it from inside the user callback would
/// block the very thread the helper is waiting on and the call would
/// always return `0` after `timeout_ms` elapses.
///
/// `timeout_ms` is the maximum time to wait. A `5000` (5 s) value is
/// generous for typical drain times (single-digit milliseconds);
/// production callers should pick a value larger than the worst-case
/// in-flight callback latency they tolerate.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_await_drain(
    handle: *const TdxUnified,
    timeout_ms: u64,
) -> i32 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        let handle = unsafe { &*handle };
        let timeout = std::time::Duration::from_millis(timeout_ms);
        i32::from(handle.inner.await_drain(timeout))
    })
}

/// Free a unified client handle.
///
/// # Lifecycle contract
///
/// Returns only after the streaming consumer thread has finished firing
/// the registered callback. Internally calls `stop_streaming` and then
/// awaits the post-stop drain barrier (a 5-second internal timeout); on
/// timeout, emits a `tracing::error!` and proceeds with destruction. In
/// the timeout-overrun path the previous Disruptor consumer may still be
/// firing the user callback, so `ctx` MUST remain valid past return.
/// Under normal operation (callback returns within microseconds, ring
/// not deeply backlogged), drain completes in low single-digit
/// milliseconds and `ctx` is safe to free immediately on return.
///
/// Calling `tdx_unified_await_drain` from another thread before invoking
/// `tdx_unified_free` is no longer required for callback-context lifetime
/// safety — `_free` now serves as the public drain barrier as well.
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_free(handle: *mut TdxUnified) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }
        let handle = unsafe { Box::from_raw(handle) };

        // Raise the stop signal first. `stop_streaming` is idempotent
        // on an already-stopped slot and, when the slot was `Live`,
        // captures the drain flag of the superseded session into the
        // client's `prev_drained` slot — the flag we poll below.
        //
        // Importantly, if the caller already invoked
        // `tdx_unified_stop_streaming` before `_free`, `is_streaming()`
        // is already `false` here, but `prev_drained` was populated by
        // that earlier `stop_streaming` call. The barrier MUST poll
        // `prev_drained` regardless of the current slot state — the
        // earlier-stop path is the one most likely to hit a callback
        // still firing on the Disruptor consumer thread.
        handle.inner.stop_streaming();

        // Wait for the consumer thread to finish firing the registered
        // callback before we destroy the handle. This is the
        // institutional `free` contract: returning only after the
        // callback path is quiesced means user code can release `ctx`
        // immediately afterwards. Default 5 s timeout; overrun is
        // surfaced via `tracing::error!` so ops can spot a wedged
        // callback rather than pay an unbounded teardown cost.
        //
        // `await_drain` returns `false` in two distinct cases:
        //
        //   (a) timeout expired with the flag still false — the
        //       consumer is still firing past the budget, ops must
        //       see this surfaced so they can investigate.
        //   (b) no prior session was ever live — `prev_drained` is
        //       `None` (e.g. a unified handle that only ran historical
        //       endpoints; nothing to wait on). This is the normal
        //       free-without-streaming flow and must NOT be flagged.
        //
        // We disambiguate by snapshotting `prev_drained.is_some()`
        // BEFORE the wait: `true` means a session existed, so a
        // `false` return is an honest timeout worth logging.
        const FREE_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
        let had_prior_session = handle.inner.prev_drained_is_set();
        if had_prior_session && !handle.inner.await_drain(FREE_DRAIN_TIMEOUT) {
            tracing::error!(
                target: "thetadatadx::ffi",
                timeout_ms = FREE_DRAIN_TIMEOUT.as_millis() as u64,
                "tdx_unified_free: drain barrier exceeded timeout -- callback may still \
                 be firing on the consumer thread; user ctx must remain valid past return",
            );
        }

        // Now safe to destroy the handle.
        drop(handle);
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  FPSS — Real-time streaming client
// ═══════════════════════════════════════════════════════════════════════

/// Allocate an FPSS handle and stash the connection parameters.
///
/// **Does NOT open the FPSS TLS connection** — connection is deferred
/// until the caller installs a callback via `tdx_fpss_set_callback`.
/// This is required because `FpssClient::connect` registers its event
/// handler at connect time; deferring the connect until callback
/// installation lets us avoid an internal queue.
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
                connect_timeout_ms: config.inner.fpss.connect_timeout_ms,
                read_timeout_ms: config.inner.fpss.timeout_ms,
                ping_interval_ms: config.inner.fpss.ping_interval_ms,
            },
            callback: Mutex::new(None),
            state: AtomicU8::new(FPSS_STATE_FRESH),
            prev_drained: Mutex::new(Vec::new()),
        }))
    })
}

/// Reject the call if the handle is already past its first
/// registration (`Active`) or has been shut down (`Shutdown`).
///
/// Returns `true` if the caller should proceed (handle is `Fresh`);
/// `false` after setting `tdx_last_error()` to a contract-specific
/// message. Used by `tdx_fpss_set_callback` to enforce one-shot
/// registration and the terminal-shutdown rule.
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
/// Internal helper used by `tdx_fpss_set_callback`. The caller supplies
/// a Rust closure that consumes `FpssEvent` references; this is the
/// closure registered with `FpssClient::connect` and lives for the
/// lifetime of the connection. Returns -1 on connect failure (error
/// already set), 0 on success.
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
            connect_timeout_ms: params.connect_timeout_ms,
            read_timeout_ms: params.read_timeout_ms,
            ping_interval_ms: params.ping_interval_ms,
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
/// `callback` is invoked from the LMAX Disruptor consumer thread for
/// every FPSS event the reader pulls off the wire, with each
/// invocation wrapped in [`std::panic::catch_unwind`] so a C/C++ panic
/// does not kill the consumer. The TLS reader publishes events via
/// `Producer::try_publish`; on ring overflow events are dropped and
/// counted (queryable via `tdx_fpss_dropped_events`). The reader
/// thread NEVER blocks on `callback`.
///
/// `ctx` is an opaque pointer passed back unchanged on every invocation.
/// It MUST remain valid until ONE of the following barriers completes:
///
/// - `tdx_fpss_free` returns (the simple path; `_free` performs the
///   shutdown if the handle is still live and internally polls the
///   drain barrier with a 5-second timeout, so on a non-overrun return
///   the consumer thread has finished firing the callback);
/// - `tdx_fpss_shutdown` (or `tdx_fpss_reconnect`) returns AND
///   `tdx_fpss_await_drain` has confirmed `1`. Stop / reconnect return
///   asynchronously; events still in the ring continue flowing through
///   the old callback until the consumer exits.
///
/// In the `_free` timeout-overrun path (rare; emits a
/// `tracing::error!`) the consumer may still be firing the callback,
/// so under that diagnostic `ctx` MUST remain valid past return; the
/// caller is expected to investigate the wedged callback rather than
/// race destruction. The Disruptor consumer thread accesses `ctx` on
/// every event and on every `tdx_fpss_reconnect`.
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
        // Wire the user callback directly into `FpssClient::connect`;
        // the core SDK's Disruptor consumer invokes it under
        // `catch_unwind` and counts ring-buffer overflow on
        // `dropped_count`. No second queue, no extra drain thread.
        let rc = open_fpss(handle, move |event: &thetadatadx::fpss::FpssEvent| {
            cb.invoke(event);
        });
        if rc != 0 {
            // Connect failed; state stays Fresh so the caller can
            // retry once the underlying problem is fixed.
            return rc;
        }
        let mut cb_guard = handle
            .callback
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *cb_guard = Some(cb);
        // Transition to Active only after every fallible operation has
        // succeeded -- a failed connect leaves the handle Fresh so the
        // caller can retry.
        handle
            .state
            .store(FPSS_STATE_ACTIVE, AtomicOrdering::Relaxed);
        0
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
            set_error("FPSS client not started -- call tdx_fpss_set_callback first, or has been shut down");
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
//  Standalone FPSS — polymorphic subscribe / unsubscribe
// ═══════════════════════════════════════════════════════════════════════

/// Polymorphic subscribe on the standalone FPSS client. Mirrors the
/// Rust `FpssClient::subscribe(Subscription)` shape.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_subscribe(
    handle: *const TdxFpssHandle,
    request: *const TdxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let sub = match unsafe { coerce_subscription(request) } {
            Some(s) => s,
            None => return -1,
        };
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error(
                "FPSS client not started -- call tdx_fpss_set_callback first, or has been shut down",
            );
            return -1;
        };
        match client.subscribe(sub) {
            Ok(()) => 0,
            Err(e) => {
                set_error(&e.to_string());
                -1
            }
        }
    })
}

/// Polymorphic unsubscribe on the standalone FPSS client.
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_unsubscribe(
    handle: *const TdxFpssHandle,
    request: *const TdxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("FPSS handle is null");
            return -1;
        }
        let sub = match unsafe { coerce_subscription(request) } {
            Some(s) => s,
            None => return -1,
        };
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error(
                "FPSS client not started -- call tdx_fpss_set_callback first, or has been shut down",
            );
            return -1;
        };
        match client.unsubscribe(sub) {
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
/// the C callback registered via the most recent `tdx_fpss_set_callback`.
/// Returns -1 if no callback was ever installed or if the handle has
/// been shut down (shutdown is terminal — see [`tdx_fpss_shutdown`]).
///
/// Returns 0 on success, or -1 on error (check `tdx_last_error()`).
///
/// # Event continuity
///
/// Events still pending in the old session's Disruptor ring continue
/// flowing through the previous registration's callback until the old
/// consumer thread joins; events buffered inside the old TLS read
/// path are lost. There is no gap-free delivery guarantee across
/// reconnections — callers that require gap-free streaming should
/// implement sequence-number-based gap detection and replay.
///
/// # Lifecycle restriction
///
/// MUST NOT be called from inside the user callback. The new
/// connection is opened immediately after the old client is dropped,
/// but the old consumer thread keeps firing the previous callback
/// until its exit path joins. Pair with `tdx_fpss_await_drain` from
/// a separate thread when the application needs to free `ctx` or
/// otherwise rely on the old callback having stopped firing.
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
        let cb = {
            let guard = handle
                .callback
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match *guard {
                Some(cb) => cb,
                None => {
                    set_error(
                        "no callback registered -- call tdx_fpss_set_callback \
                         before tdx_fpss_reconnect",
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

        // 2. Shut down the old client. With the SSOT pipeline there is
        // no separate dispatcher to tear down — the Disruptor consumer
        // joins inside `FpssClient::Drop` when the last `Arc` goes
        // away. Capture the drain flag BEFORE dropping `old` so a
        // subsequent `tdx_fpss_await_drain` poll observes the previous
        // session's quiescence even though `Drop` runs asynchronously
        // when invoked from the consumer thread.
        let prev_drain_flag = {
            let mut guard = handle
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(old) = guard.take() {
                let flag = old.drained_flag();
                handle
                    .prev_drained
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(flag.clone());
                old.shutdown();
                Some(flag)
            } else {
                None
            }
        };

        // 2b. Block until the previous consumer thread has finished
        // firing the old C callback BEFORE opening a fresh session
        // bound to the same `cb` / `ctx`. The C ABI contract
        // guarantees single-threaded callback invocation; without
        // this barrier the old consumer can still be inside the
        // user's `ctx` when the new consumer starts firing on a
        // different thread, and the user's "no internal locks
        // needed" assumption breaks. A 5 s budget matches the FFI
        // free-path drain budget; on timeout we surface the error
        // rather than racing.
        if let Some(flag) = prev_drain_flag {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(5_000);
            while !flag.load(std::sync::atomic::Ordering::Acquire) {
                if std::time::Instant::now() >= deadline {
                    set_error(
                        "reconnect drain barrier timed out after 5s — \
                         previous callback is still in flight; refusing \
                         to bind the new session to the same ctx",
                    );
                    return -1;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        // 3. Build the new event handler bound to the same C callback.
        // After #513 there is no dispatcher hop: the Disruptor consumer
        // invokes `cb.invoke(event)` under `catch_unwind`.
        let new_client = thetadatadx::fpss::FpssClient::connect(
            thetadatadx::fpss::FpssConnectArgs {
                creds: &params.creds,
                hosts: &params.hosts,
                ring_size: params.ring_size,
                flush_mode: params.flush_mode,
                policy: params.reconnect_policy.clone(),
                derive_ohlcvc: params.derive_ohlcvc,
                connect_timeout_ms: params.connect_timeout_ms,
                read_timeout_ms: params.read_timeout_ms,
                ping_interval_ms: params.ping_interval_ms,
            },
            move |event: &thetadatadx::fpss::FpssEvent| {
                cb.invoke(event);
            },
        );

        let new_client = match new_client {
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
            let result =
                new_client.subscribe(thetadatadx::fpss::protocol::Subscription::Contract {
                    contract: contract.clone(),
                    kind: *kind,
                });
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
            let full_kind = match kind {
                thetadatadx::fpss::protocol::SubscriptionKind::Trade => {
                    thetadatadx::fpss::protocol::FullSubscriptionKind::Trades
                }
                thetadatadx::fpss::protocol::SubscriptionKind::OpenInterest => {
                    thetadatadx::fpss::protocol::FullSubscriptionKind::OpenInterest
                }
                thetadatadx::fpss::protocol::SubscriptionKind::Quote => continue,
            };
            let result = new_client.subscribe(thetadatadx::fpss::protocol::Subscription::Full {
                sec_type: *sec_type,
                kind: full_kind,
            });
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

        // 5. Store the new client.
        {
            let mut guard = handle
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = Some(new_client);
        }

        0
    })
}

/// Cumulative count of FPSS events the TLS reader could not publish
/// into the Disruptor ring because the consumer fell behind and the
/// ring was full (`Producer::try_publish` returned `RingBufferFull`).
///
/// Returns 0 if the handle is null or no callback has been installed
/// yet.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_dropped_events(handle: *const TdxFpssHandle) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        let handle = unsafe { &*handle };
        let guard = handle
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_ref()
            .map_or(0, thetadatadx::fpss::FpssClient::dropped_count)
    })
}

/// Shut down the FPSS client, stopping all background threads.
///
/// # Lifecycle contract (terminal)
///
/// Shutdown is terminal: every subsequent `tdx_fpss_set_callback` /
/// `_reconnect` / `_shutdown` call on this handle returns -1 with the
/// error message
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
        // Drop the FPSS reader; the Disruptor consumer drains the ring
        // and joins inside `FpssClient::Drop` when the last `Arc` is
        // dropped. There is no separate dispatcher to tear down.
        // Capture the drain flag BEFORE dropping the client so
        // `tdx_fpss_await_drain` can confirm the user callback has
        // stopped firing — `Drop` is asynchronous when shutdown is
        // invoked from the consumer thread.
        {
            let mut guard = handle
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(client) = guard.take() {
                handle
                    .prev_drained
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(client.drained_flag());
                client.shutdown();
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

/// Wait for every superseded FPSS session to quiesce.
///
/// Returns `1` once **all** prior `tdx_fpss_reconnect` /
/// `tdx_fpss_shutdown` sessions' Disruptor consumers have finished
/// firing the registered callback. Returns `0` on timeout or when no
/// session has been superseded on this handle.
///
/// Stacked reconnect/shutdown cycles layer multiple in-flight
/// generations on top of each other; this barrier waits for ALL of
/// them, not just the most-recent. Drained flags are GC'd from the
/// internal Vec on each poll.
///
/// # When to call
///
/// After `tdx_fpss_reconnect` or `tdx_fpss_shutdown` returns, before
/// freeing `ctx` or otherwise relying on the old callback having
/// stopped firing.
///
/// # Lifecycle restriction
///
/// MUST be called from a thread other than the FPSS Disruptor
/// consumer thread. Calling it from inside the user callback would
/// block the helper the consumer is waiting on and always time out.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_await_drain(
    handle: *const TdxFpssHandle,
    timeout_ms: u64,
) -> i32 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        let handle = unsafe { &*handle };
        // Snapshot the pending generations once and walk them on each
        // poll. New stops landing during the wait join the next call's
        // working set — `await_drain` semantics are "wait for what was
        // outstanding when I started", which mirrors the in-process
        // `ThetaDataDxClient::await_drain` contract.
        let initial = {
            let guard = handle
                .prev_drained
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if guard.is_empty() {
                return 0;
            }
            guard.clone()
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            // All flags drained?
            if initial
                .iter()
                .all(|f| f.load(std::sync::atomic::Ordering::Acquire))
            {
                // Lazy GC of the shared Vec so a long-lived handle that
                // cycles through many sessions does not accumulate
                // entries. Take the lock briefly; this is a clean-up
                // path, never on the hot tick path.
                let mut guard = handle
                    .prev_drained
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.retain(|f| !f.load(std::sync::atomic::Ordering::Acquire));
                return 1;
            }
            if std::time::Instant::now() >= deadline {
                return 0;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    })
}

/// Free a FPSS handle.
///
/// # Lifecycle contract
///
/// `tdx_fpss_free` accepts the handle in either state:
///
/// - **Already shut down**: the prior `tdx_fpss_shutdown` (or
///   `tdx_fpss_reconnect`) populated the drain flag; `_free` polls that
///   flag with a 5-second internal timeout so it returns only after the
///   superseded session's Disruptor consumer has finished firing the
///   registered callback.
/// - **Not yet shut down**: `_free` performs the equivalent of
///   `tdx_fpss_shutdown` first (drops the FPSS client, captures the
///   drain flag, marks the state terminal) and then polls the same
///   barrier.
///
/// On drain-flag timeout, `_free` emits a `tracing::error!` and proceeds
/// with destruction. In the timeout-overrun path the previous Disruptor
/// consumer may still be firing the user callback, so `ctx` MUST remain
/// valid past return. Under normal operation (callback returns within
/// microseconds, ring not deeply backlogged), drain completes in low
/// single-digit milliseconds and `ctx` is safe to free immediately on
/// return.
///
/// Calling `tdx_fpss_await_drain` from another thread before invoking
/// `tdx_fpss_free` is no longer required for callback-context lifetime
/// safety — `_free` now serves as the public drain barrier as well.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_free(handle: *mut TdxFpssHandle) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }

        // Shut down first if the handle is still live, mirroring
        // `tdx_fpss_shutdown` so callers who skip the explicit shutdown
        // call still get a quiesced consumer thread by the time `_free`
        // returns. Detect "already shut down" via the lifecycle state
        // so we never attempt a double shutdown.
        {
            let h = unsafe { &*handle };
            if h.state.load(AtomicOrdering::Relaxed) != FPSS_STATE_SHUTDOWN {
                let mut guard = h
                    .inner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(client) = guard.take() {
                    h.prev_drained
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(client.drained_flag());
                    client.shutdown();
                }
                h.state.store(FPSS_STATE_SHUTDOWN, AtomicOrdering::Relaxed);
            }

            // Wait for every superseded session's consumer thread to
            // finish firing the registered callback. The Vec is empty
            // when no callback was ever installed (FRESH -> SHUTDOWN
            // direct transition), so suppress the timeout log on that
            // path.
            const FREE_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
            let pending: Vec<Arc<std::sync::atomic::AtomicBool>> = {
                let guard = h
                    .prev_drained
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.clone()
            };
            if !pending.is_empty() {
                let deadline = std::time::Instant::now() + FREE_DRAIN_TIMEOUT;
                let drained = loop {
                    if pending
                        .iter()
                        .all(|f| f.load(std::sync::atomic::Ordering::Acquire))
                    {
                        break true;
                    }
                    if std::time::Instant::now() >= deadline {
                        break false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                };
                if !drained {
                    tracing::error!(
                        target: "thetadatadx::ffi",
                        timeout_ms = FREE_DRAIN_TIMEOUT.as_millis() as u64,
                        pending_generations = pending.len(),
                        "tdx_fpss_free: drain barrier exceeded timeout -- callback may still \
                         be firing on the consumer thread; user ctx must remain valid past return",
                    );
                }
            }
        }

        // Now safe to destroy the handle.
        drop(unsafe { Box::from_raw(handle) });
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Pull-iter delivery — C ABI
// ═══════════════════════════════════════════════════════════════════════

/// Opaque pull-iter handle returned by
/// [`tdx_unified_start_streaming_iter`].
///
/// Drains the per-client bounded queue populated by the FPSS Disruptor
/// consumer thread. Mutually exclusive with `tdx_unified_set_callback`
/// on the same `TdxUnified`; switch by calling `tdx_unified_stop_streaming`
/// first. The handle owns its own `FfiBufferedEvent` slot (re-used
/// across `_next` calls) so the borrowed-pointer lifetime contract on
/// the returned `TdxFpssEvent` matches the push-callback path.
pub struct TdxFpssEventIterator {
    inner: thetadatadx::EventIterator,
    /// Re-usable backing-buffer slot. Each successful `_next` rebuilds
    /// the slot from the freshly popped event so the borrowed
    /// `*const c_char` / `*const u8` pointers in the public
    /// `TdxFpssEvent` reference into THIS slot's owned heap memory.
    /// Lifetime contract: the borrowed pointers are valid until the
    /// next `_next` / `_free` call on the same iterator handle.
    last_buffered: Option<FfiBufferedEvent>,
}

/// Start FPSS streaming on the unified client in pull-iter mode.
///
/// Returns an opaque `TdxFpssEventIterator*` on success. Free with
/// `tdx_fpss_event_iter_free` once the caller is done iterating. Pull
/// events with `tdx_fpss_event_iter_next` (blocking with timeout) or
/// `tdx_fpss_event_iter_close` (explicit termination).
///
/// Mutually exclusive with `tdx_unified_set_callback`. Calling either
/// while streaming is already running returns NULL with
/// `tdx_last_error()` set to `"streaming already started"`.
///
/// Returns NULL on connection / auth / state failure (check
/// `tdx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn tdx_unified_start_streaming_iter(
    handle: *const TdxUnified,
) -> *mut TdxFpssEventIterator {
    ffi_boundary!(ptr::null_mut(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null_mut();
        }
        let handle = unsafe { &*handle };
        match handle.inner.start_streaming_iter() {
            Ok(iterator) => Box::into_raw(Box::new(TdxFpssEventIterator {
                inner: iterator,
                last_buffered: None,
            })),
            Err(e) => {
                set_error(&e.to_string());
                ptr::null_mut()
            }
        }
    })
}

/// Pop the next FPSS event from the pull-iter queue.
///
/// `out_event` MUST point to a writable `TdxFpssEvent` slot. On a
/// successful pop the slot is overwritten with the freshly converted
/// event and `0` is returned. On timeout (no event arrived within
/// `timeout_ms`), `1` is returned and `*out_event` is left untouched.
/// On terminal end-of-stream (the streaming session has shut down and
/// the queue is drained), `-1` is returned.
///
/// Pass `timeout_ms = 0` for non-blocking polling. Pass a large value
/// (e.g. `5000`) for blocking-with-deadline drain. There is no
/// "infinite" wait — long-running consumers should loop on a short
/// timeout so signal handlers can break the loop.
///
/// # Lifetime of the returned event
///
/// The borrowed `*const c_char` / `*const u8` pointers inside
/// `*out_event` (`Contract.symbol`, `LoginSuccess.permissions`, the
/// payload byte slices, etc.) reference heap memory owned by the
/// iterator handle's internal buffer. Those pointers are valid
/// until the next `tdx_fpss_event_iter_next` call on the same
/// iterator OR until `tdx_fpss_event_iter_free` is called. Copy any
/// fields the consumer wants to outlive the next call before
/// re-entering the iterator.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_event_iter_next(
    it: *mut TdxFpssEventIterator,
    out_event: *mut TdxFpssEvent,
    timeout_ms: i32,
) -> i32 {
    ffi_boundary!(-1, {
        if it.is_null() {
            set_error("event iterator handle is null");
            return -1;
        }
        if out_event.is_null() {
            set_error("out_event is null");
            return -1;
        }
        let it = unsafe { &mut *it };
        // Three-state outcome — both branches drive off the typed
        // `NextEvent` enum so `Timeout` and `Closed` cannot collapse
        // back into ambiguous `None`. The non-blocking branch
        // (`timeout_ms <= 0`) calls `try_next`, which since 9.1.0
        // returns the same trichotomy as `next_timeout`: an empty
        // queue on a live upstream is `Timeout` (rc `1`, soft re-poll
        // signal); an empty queue on a shut-down session is `Closed`
        // (rc `-1`, terminal end-of-stream). Earlier the
        // non-blocking path mapped every `None` to `Timeout` and a C
        // client polling after `stop_streaming()` saw rc `1` forever.
        let outcome: ::thetadatadx::NextEvent = if timeout_ms <= 0 {
            it.inner.try_next()
        } else {
            let timeout_ms_u64 = u64::try_from(timeout_ms).unwrap_or(0);
            it.inner
                .next_timeout(std::time::Duration::from_millis(timeout_ms_u64))
        };
        match outcome {
            ::thetadatadx::NextEvent::Ready(event) => {
                let buffered = fpss_event_to_ffi(&event);
                // Write the public `TdxFpssEvent` view into the caller's
                // out parameter BEFORE storing the backing buffer on the
                // iterator. Pointer fields inside `buffered.event`
                // reference into `buffered`'s owned `Option<CString>` /
                // `Option<Vec<u8>>` slots — moving `buffered` into
                // `it.last_buffered` keeps those slots alive at a stable
                // address, but their addresses MUST be stabilised first
                // by the move below. `TdxFpssEvent` is `#[repr(C)]` and
                // contains only POD + raw pointers, so the bytewise
                // copy here is sound.
                it.last_buffered = Some(buffered);
                let stored = it.last_buffered.as_ref().expect("just stored");
                unsafe {
                    std::ptr::copy_nonoverlapping(std::ptr::from_ref(&stored.event), out_event, 1);
                }
                0
            }
            ::thetadatadx::NextEvent::Timeout => 1,
            ::thetadatadx::NextEvent::Closed => -1,
        }
    })
}

/// Mark the pull-iter iterator as closed. Subsequent `_next` calls
/// return `-1` (terminal) once the queue is drained, without shutting
/// down the underlying streaming session. Idempotent.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_event_iter_close(it: *mut TdxFpssEventIterator) {
    ffi_boundary!((), {
        if it.is_null() {
            return;
        }
        let it = unsafe { &*it };
        it.inner.close();
    })
}

/// Free a pull-iter iterator handle returned by
/// [`tdx_unified_start_streaming_iter`]. Releases the iterator's
/// internal buffer and any borrowed-pointer slot references. Does NOT
/// stop the underlying streaming session — call
/// `tdx_unified_stop_streaming` first if you need a full shutdown.
#[no_mangle]
pub unsafe extern "C" fn tdx_fpss_event_iter_free(it: *mut TdxFpssEventIterator) {
    ffi_boundary!((), {
        if it.is_null() {
            return;
        }
        drop(unsafe { Box::from_raw(it) });
    })
}

#[cfg(test)]
mod tests {
    //! Unit tests for the C ABI callback wiring.
    //!
    //! These tests exercise the `FfiCallback` shim and the
    //! Disruptor-consumer integration without opening a real FPSS
    //! TLS connection. The contract a downstream C/C++ consumer relies
    //! on: events handed to `FfiCallback::invoke` arrive at the user
    //! `extern "C" fn` with the registered `ctx` and a valid
    //! `*const TdxFpssEvent`. The Disruptor consumer runs the callback
    //! on its own thread (not the producer thread).

    use super::*;
    use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::thread;

    /// Mutable test context observed from the C-shaped callback. Holds
    /// the captured contract id, thread id observed inside the user
    /// callback (so tests can prove the consumer thread invoked the
    /// callback), and a hit counter.
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
        // caller's thread id and verify the consumer-thread routing.
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

    /// Direct invocation: calling `FfiCallback::invoke` runs the user
    /// fn synchronously on the caller's thread with the registered ctx.
    #[test]
    fn ffi_callback_direct_invoke_runs_user_fn_on_caller_thread() {
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
            "direct callback invocation ran on the caller thread",
        );
    }

    /// Queued mode (Disruptor consumer path): the FfiCallback wired
    /// through a Disruptor consumer fires on the consumer thread, not
    /// the producer (caller) thread.
    #[test]
    fn ffi_callback_queued_runs_on_consumer_thread() {
        use disruptor::{build_single_producer, BusySpin, Producer, Sequence};
        use std::panic::{catch_unwind, AssertUnwindSafe};

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
        // `tdx_unified_set_callback` install through `start_streaming`:
        // the Disruptor consumer thread owns the user callback wrapped
        // in `catch_unwind`; the producer side is what the FPSS reader
        // (here, the test thread) calls via `try_publish`.
        #[derive(Default)]
        struct Slot {
            event: Option<thetadatadx::fpss::FpssEvent>,
        }
        // SAFETY: matches the live `RingEvent`'s `unsafe impl Sync`.
        unsafe impl Sync for Slot {}

        let mut producer = build_single_producer(64, || Slot { event: None }, BusySpin)
            .handle_events_with(move |slot: &Slot, _seq: Sequence, _eob: bool| {
                if let Some(ref evt) = slot.event {
                    let _ = catch_unwind(AssertUnwindSafe(|| cb.invoke(evt)));
                }
            })
            .build();

        let producer_tid = thread_id_u64();
        producer
            .try_publish(|slot| {
                slot.event = Some(synthetic_quote_event());
            })
            .expect("ring buffer has room for a single event");

        // Drop the producer to drain + join the consumer thread. By
        // the time `drop` returns the callback has fired exactly once.
        drop(producer);

        // SAFETY: ctx_ptr still valid until we re-Box below.
        let ctx_back = unsafe { Box::from_raw(ctx_ptr.cast::<TestCtx>()) };
        assert_eq!(
            ctx_back.hits.load(Ordering::Relaxed),
            1,
            "callback fired once via Disruptor consumer thread",
        );
        let observed_tid = ctx_back.callback_thread.load(Ordering::Relaxed);
        assert_ne!(
            observed_tid, producer_tid,
            "consumer path ran callback on a different thread than the producer (queued semantics)",
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
    /// installed (no inner client exists yet).
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
                connect_timeout_ms: 2_000,
                read_timeout_ms: 10_000,
                ping_interval_ms: 100,
            },
            callback: Mutex::new(None),
            state: AtomicU8::new(FPSS_STATE_FRESH),
            prev_drained: Mutex::new(Vec::new()),
        };
        let raw = Box::into_raw(Box::new(handle));
        let count = unsafe { tdx_fpss_dropped_events(raw) };
        assert_eq!(count, 0, "no inner client means dropped count is 0");
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
                connect_timeout_ms: 2_000,
                read_timeout_ms: 10_000,
                ping_interval_ms: 100,
            },
            callback: Mutex::new(None),
            state: AtomicU8::new(FPSS_STATE_SHUTDOWN),
            prev_drained: Mutex::new(Vec::new()),
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
    }

    /// `tdx_fpss_free` MUST wait on the saved `prev_drained` flag
    /// before destroying the handle. We exercise the barrier by
    /// constructing an FPSS handle whose lifecycle state is already
    /// past first registration (so the free path skips the
    /// `inner.take()` shutdown step) but whose `prev_drained` is
    /// populated with a flag we control on a helper thread.
    ///
    /// The test installs a flag that flips to `true` only after a
    /// short sleep, calls `tdx_fpss_free` on a watchdogged thread, and
    /// asserts:
    ///
    ///   1. `_free` did not return until the flag flipped
    ///      (the wall-clock elapsed at least `FLAG_DELAY`),
    ///   2. `_free` returned within the watchdog budget
    ///      (the barrier is bounded by its 5 s internal timeout),
    ///   3. the helper thread observed `_free` returning AFTER
    ///      it set the flag, not before.
    ///
    /// This is the load-bearing assertion for the round-4 fix: the
    /// barrier polls `prev_drained` regardless of whether the caller
    /// invoked `tdx_fpss_shutdown` first (the bug was the unified path
    /// gated on `is_streaming()` and skipped the wait when shutdown
    /// had already flipped that bit to `false`).
    #[test]
    fn ffi_fpss_free_blocks_on_prev_drained_flag() {
        use std::sync::atomic::AtomicBool;
        use std::time::{Duration, Instant};

        const FLAG_DELAY: Duration = Duration::from_millis(50);
        const WATCHDOG: Duration = Duration::from_secs(2);

        let drain_flag = Arc::new(AtomicBool::new(false));

        // Build the handle in `Shutdown` state so `tdx_fpss_free` skips
        // the inner-take/shutdown path and goes straight to the
        // drain-flag poll. `prev_drained` is the load-bearing field.
        let handle = TdxFpssHandle {
            inner: Arc::new(Mutex::new(None)),
            connect_params: FpssConnectParams {
                creds: thetadatadx::Credentials::new("user", "password"),
                hosts: vec![("localhost".to_owned(), 25503)],
                ring_size: 4096,
                flush_mode: thetadatadx::FpssFlushMode::default(),
                reconnect_policy: thetadatadx::config::ReconnectPolicy::default(),
                derive_ohlcvc: false,
                connect_timeout_ms: 2_000,
                read_timeout_ms: 10_000,
                ping_interval_ms: 100,
            },
            callback: Mutex::new(None),
            state: AtomicU8::new(FPSS_STATE_SHUTDOWN),
            prev_drained: Mutex::new(vec![Arc::clone(&drain_flag)]),
        };
        let raw = Box::into_raw(Box::new(handle));

        // Helper thread flips the flag after a known delay so the
        // barrier inside `_free` actually has to poll. If the barrier
        // is missing, `_free` returns instantly and the wall-clock
        // elapsed below is close to zero.
        let flag_for_helper = Arc::clone(&drain_flag);
        let helper = thread::Builder::new()
            .name("flip-drain-flag".to_owned())
            .spawn(move || {
                thread::sleep(FLAG_DELAY);
                flag_for_helper.store(true, Ordering::Release);
            })
            .expect("spawn helper");

        let started = Instant::now();
        unsafe { tdx_fpss_free(raw) };
        let elapsed = started.elapsed();

        helper.join().expect("helper thread completed");

        assert!(
            elapsed >= FLAG_DELAY / 2,
            "tdx_fpss_free returned in {elapsed:?} -- below the {FLAG_DELAY:?} \
             helper-flip delay; the drain-flag poll was skipped",
        );
        assert!(
            elapsed < WATCHDOG,
            "tdx_fpss_free took {elapsed:?} -- exceeded the {WATCHDOG:?} watchdog; \
             the barrier should have observed the helper's flag flip and returned",
        );
        assert!(
            drain_flag.load(Ordering::Acquire),
            "drain flag must be set by the time `_free` returns; otherwise the \
             post-return ctx-lifetime contract is violated",
        );
    }

    /// Round-4 critical 1 regression: `tdx_unified_free` must wait on
    /// the saved drain flag even after the caller has already invoked
    /// `tdx_unified_stop_streaming`.
    ///
    /// PR #514 HIGH-001: the slot is now a `Vec<Arc<AtomicBool>>` so
    /// stacked stop/start/stop cycles cannot lose an earlier still-
    /// firing generation when a later one retires. This test pins the
    /// `prev_drained_is_set` predicate semantics on the Vec storage
    /// that backs the FFI free path.
    #[test]
    fn unified_prev_drained_is_set_persists_through_stop_then_free() {
        use std::sync::atomic::AtomicBool;

        let slot: Mutex<Vec<Arc<AtomicBool>>> = Mutex::new(Vec::new());
        assert!(
            slot.lock().unwrap().is_empty(),
            "fresh slot is empty -- no streaming session ever existed"
        );

        // First `stop_streaming` pushes a drain flag.
        let flag_a = Arc::new(AtomicBool::new(false));
        slot.lock().unwrap().push(Arc::clone(&flag_a));
        assert_eq!(
            slot.lock().unwrap().len(),
            1,
            "after first stop, one retired generation is pending",
        );

        // A second stacked stop pushes ANOTHER flag — the prior one
        // is NOT overwritten (the bug PR #514 closed).
        let flag_b = Arc::new(AtomicBool::new(false));
        slot.lock().unwrap().push(Arc::clone(&flag_b));
        assert_eq!(
            slot.lock().unwrap().len(),
            2,
            "stacked stop must NOT overwrite the earlier generation's flag",
        );

        // After the later generation drains, the earlier one is
        // STILL pending. The await-drain predicate (lazy GC + emptiness
        // check) reflects this faithfully.
        flag_b.store(true, Ordering::Release);
        let mut g = slot.lock().unwrap();
        g.retain(|f| !f.load(Ordering::Acquire));
        assert_eq!(
            g.len(),
            1,
            "later generation drained; earlier still pending — `_free` \
             MUST wait on it before destroying the ctx",
        );
        drop(g);

        flag_a.store(true, Ordering::Release);
        let mut g = slot.lock().unwrap();
        g.retain(|f| !f.load(Ordering::Acquire));
        assert!(
            g.is_empty(),
            "all retired generations drained — `_free` may proceed",
        );
    }
}
