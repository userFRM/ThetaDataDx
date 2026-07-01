//! FPSS streaming and unified client surface.
//!
//! Contains the streaming-specific handles (`ThetaDataDxClient`, `ThetaDataDxStreamHandle`),
//! the `#[repr(C)]` FPSS event types (generated — `include!`'d), the tagged
//! subscription / contract-map arrays, and every `thetadatadx_client_*` /
//! `thetadatadx_streaming_*` `extern "C" fn`.
//!
//! # Callback C ABI
//!
//! Both [`ThetaDataDxClient`] and [`ThetaDataDxStreamHandle`] expose a single callback-
//! registration entry point that wires user `extern "C"` functions
//! through a single-queue event pipeline:
//!
//! - `thetadatadx_*_set_callback` — the user callback runs on the event ring
//!   event-dispatch consumer thread, with each invocation wrapped in
//!   [`std::panic::catch_unwind`]. That wrapper contains a panic raised by
//!   our own Rust code on the dispatch path so it does not kill the
//!   consumer; it does not contain a foreign exception thrown out of the
//!   user callback. The user callback runs under the C ABI and must not let
//!   an exception or other unwind escape across that boundary (doing so is
//!   undefined behavior; see [`ThetaDataDxStreamCallback`]). The TLS reader
//!   publishes events via `Producer::try_publish`; on ring overflow the
//!   event is dropped and the drop count is exposed via
//!   `thetadatadx_*_dropped_events`. The reader thread never blocks on user
//!   code.
//!
//! The poll-based `thetadatadx_*_next_event` API and its supporting `mpsc`
//! pipeline have been removed; the C ABI is callback-only.

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicU8, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::{set_error, set_error_from};
use crate::types::{ThetaDataDxConfig, ThetaDataDxCredentials, ThetaDataDxHistoricalClient};
use thetadatadx::DispatcherSession as FfpssDispatcherSession;

/// Lock a `Mutex`, recovering the guard through poisoning rather than
/// propagating a panic across the C ABI. A poisoned lock here means some
/// other thread panicked while holding it; the FFI handles only own plain
/// data (callback slots, the dispatcher state machine, drain flags), so the
/// inner value stays usable and the consistent behaviour is to keep serving
/// the C caller. `into_inner` returns that still-valid guard.
trait LockRecover<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> LockRecover<T> for Mutex<T> {
    #[inline]
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

// ── Callback C ABI types ──

/// User callback signature: invoked once per FPSS event delivered to the
/// FFI layer. The `event` pointer is valid only for the duration of the
/// call; copy any fields the caller wants to outlive the callback.
///
/// `ctx` is the opaque user context pointer registered alongside the
/// callback (`thetadatadx_*_set_callback(handle, fn, ctx)`); it is passed back
/// unchanged on every invocation.
///
/// The callback runs under the C ABI and must not unwind across the
/// boundary. A C++ `throw` or a C `longjmp` that escapes the callback into
/// the calling Rust frame is undefined behavior. The dispatch path wraps
/// each invocation in [`std::panic::catch_unwind`], but that contains only a
/// Rust panic raised on our side of the boundary, not a foreign exception
/// out of the callback. Catch and handle every exception inside the callback
/// before returning. (The C++ wrapper's `set_callback` shim does this for
/// you: it is `noexcept` and swallows any exception its `std::function`
/// raises.)
pub type ThetaDataDxStreamCallback =
    extern "C" fn(event: *const ThetaDataDxStreamEvent, ctx: *mut c_void);

/// Bundle of `(callback, ctx)` stored inside a Rust closure registered
/// with [`thetadatadx::Client::start_streaming`]. The bundle is
/// `Send + Sync + Copy` so the LMAX event-dispatch consumer thread can call
/// into the user's `extern "C" fn` from a non-FFI thread, and so the
/// same bundle can be re-registered on `thetadatadx_*_reconnect` without
/// re-invoking the user.
#[derive(Clone, Copy)]
struct FfiCallback {
    callback: ThetaDataDxStreamCallback,
    ctx: *mut c_void,
}

// SAFETY: the contained `*mut c_void` is the user's opaque context —
// it is never dereferenced by Rust, only handed back to the user's
// `extern "C" fn` exactly as registered. Send-across-threads safety is
// the user's responsibility (documented on `thetadatadx_*_set_callback`).
unsafe impl Send for FfiCallback {}
// SAFETY: see the `Send` impl directly above — the pointer is opaque
// payload, never dereferenced, and shared-reference safety is the
// user's documented responsibility.
unsafe impl Sync for FfiCallback {}

impl FfiCallback {
    fn invoke(&self, event: &thetadatadx::fpss::StreamEvent) {
        // Convert the Rust event to the FFI `#[repr(C)]` struct. The
        // returned `FfiBufferedEvent` owns the heap memory backing every
        // borrowed pointer in the event (`Contract.symbol`,
        // `LoginSuccess.permissions`, `ServerError.message`,
        // `Error.message`, `UnknownFrame.payload`, `Ping.payload`);
        // it is dropped at the end of this function,
        // after the user callback returns. The user MUST NOT retain the
        // `*const ThetaDataDxStreamEvent` pointer past the callback boundary.
        let buffered = fpss_event_to_ffi(event);
        let event_ptr: *const ThetaDataDxStreamEvent = std::ptr::from_ref(&buffered.event);
        (self.callback)(event_ptr, self.ctx);
    }
}

// ── Unified + FPSS handles ──

/// Opaque unified client handle — wraps both historical and streaming.
pub struct ThetaDataDxClient {
    pub(crate) inner: thetadatadx::Client,
    /// Callback registered via `thetadatadx_client_set_callback`. `None` until
    /// the first registration; persisted across `thetadatadx_client_reconnect`
    /// so the reconnect path can re-attach the same C user function
    /// without re-asking the caller for it.
    callback: Mutex<Option<FfiCallback>>,
}

// `FfpssDispatcherSession` is imported above as a `use` alias of the
// canonical `thetadatadx::DispatcherSession` defined in
// `thetadatadx-rs/src/lifecycle.rs`. The three per-site enum
// definitions (client.rs / streaming.rs / fpss_client.rs) are consolidated
// there to eliminate drift risk.

/// FPSS handle lifecycle state — see [`ThetaDataDxStreamHandle::state`].
///
/// The C ABI documents a strict three-state machine on every FPSS
/// handle. `thetadatadx_streaming_set_callback` and `_reconnect` enforce the
/// transitions; `thetadatadx_streaming_shutdown` is terminal (no further
/// registration / reconnect / shutdown calls succeed).
const STREAM_STATE_FRESH: u8 = 0;
const STREAM_STATE_ACTIVE: u8 = 1;
const STREAM_STATE_SHUTDOWN: u8 = 2;

/// Opaque FPSS streaming client handle.
///
/// `thetadatadx_streaming_connect` allocates the handle and stores connection
/// parameters; the actual FPSS TLS connection is opened on the first
/// call to `thetadatadx_streaming_set_callback`. This mirrors the unified handle's
/// lifecycle (`connect` then `set_callback`).
///
/// # Lifecycle state machine
///
/// `state` enforces the public C ABI contract:
///
/// - `STREAM_STATE_FRESH`  -> `STREAM_STATE_ACTIVE` on the first successful
///   `thetadatadx_streaming_set_callback`. A second registration on an already-
///   `ACTIVE` handle returns -1 with "streaming callback already installed
///   -- only one set_callback call is permitted per handle".
/// - `STREAM_STATE_ACTIVE` -> `STREAM_STATE_SHUTDOWN` on
///   `thetadatadx_streaming_shutdown`. Shutdown is terminal: every subsequent
///   register / reconnect / shutdown call returns -1 with
///   "streaming handle has already been shut down -- this is terminal".
/// - `STREAM_STATE_FRESH` directly to `STREAM_STATE_SHUTDOWN` is allowed
///   (caller shut down a handle before installing a callback).
pub struct ThetaDataDxStreamHandle {
    inner: Arc<Mutex<Option<Arc<thetadatadx::fpss::StreamingClient>>>>,
    /// Saved connection parameters used at `set_callback` time and on
    /// every subsequent `thetadatadx_streaming_reconnect`.
    connect_params: StreamingConnectParams,
    /// User callback recorded at `thetadatadx_streaming_set_callback` time. Stored
    /// on the handle so `thetadatadx_streaming_reconnect` can re-register the same
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
    /// not yet drained, captured during `thetadatadx_streaming_reconnect` /
    /// `thetadatadx_streaming_shutdown` before the previous client is dropped.
    /// `thetadatadx_streaming_await_drain` waits for ALL flags to flip so callers
    /// can confirm every old user callback has stopped firing before
    /// freeing the previous `ctx`. Stacked reconnect/shutdown cycles
    /// layer multiple in-flight generations on top of each other; a
    /// single slot would silently drop earlier still-firing sessions
    /// when a later one retired.
    prev_drained: Mutex<Vec<Arc<std::sync::atomic::AtomicBool>>>,
    /// Dispatcher lifecycle — single mutex covering serialisation,
    /// the `JoinHandle`, and failure state.  Replaces the three-
    /// primitive cluster: `install_lock: Mutex<()>`,
    /// `dispatcher_handle: Mutex<Option<JoinHandle<()>>>`, and
    /// `dispatcher_failed: Arc<AtomicBool>`.  Every `set_callback` /
    /// `reconnect` / `shutdown` / `free` path acquires this one lock,
    /// transitions the variant, and releases.  Dispatcher panic state
    /// is derived from `JoinHandle::join()` returning `Err(_)`.
    ///
    /// Wrapped in `Arc` so the spawned dispatcher thread can hold an
    /// owning handle to just this slot (not the whole `*const` handle,
    /// whose lifetime it cannot express) and publish `Failed` from its own
    /// catch-arm the instant an outer panic kills the event loop — see
    /// [`publish_failed_if_current`].
    dispatcher: Arc<Mutex<FfpssDispatcherSession>>,
}

/// Saved FPSS connection parameters for FFI-safe (re)connection.
struct StreamingConnectParams {
    creds: thetadatadx::Credentials,
    /// Snapshot of `DirectConfig.streaming` at handle-construction time —
    /// hosts, ring size, timeouts, keepalive schedule, host-selection
    /// policy, flush mode.
    streaming: thetadatadx::config::StreamingConfig,
    /// Snapshot of `DirectConfig.reconnect` at handle-construction
    /// time — policy, per-class cadences, jitter, replay pacing.
    reconnect: thetadatadx::config::ReconnectConfig,
}

/// Thread every connection-side knob from a [`StreamingConnectParams`]
/// snapshot into an [`thetadatadx::fpss::StreamingClientBuilder`].
///
/// The single source of truth for the FFI's two build sites (initial
/// `set_callback` connect and `thetadatadx_streaming_reconnect`) so a future knob
/// cannot be wired into one and silently dropped from the other.
fn streaming_builder(
    params: &StreamingConnectParams,
) -> thetadatadx::fpss::StreamingClientBuilder<'_> {
    thetadatadx::fpss::StreamingClient::builder(&params.creds, params.streaming.hosts())
        .ring_size(params.streaming.ring_size)
        .flush_mode(params.streaming.flush_mode)
        .wait_strategy(params.streaming.wait_strategy)
        .wait_strategy_tuning(
            params.streaming.wait_spin_iters,
            params.streaming.wait_yield_iters,
            params.streaming.wait_park_us,
        )
        .consumer_cpu(params.streaming.consumer_cpu)
        .reconnect_policy(params.reconnect.policy.clone())
        .reconnect_wait_ms(params.reconnect.wait_ms)
        .reconnect_wait_max_ms(params.reconnect.wait_max_ms)
        .reconnect_wait_rate_limited_ms(params.reconnect.wait_rate_limited_ms)
        .reconnect_wait_server_restart_ms(params.reconnect.wait_server_restart_ms)
        .reconnect_jitter(params.reconnect.jitter)
        .reconnect_replay_burst_size(params.reconnect.replay_burst_size)
        .reconnect_replay_pace_ms(params.reconnect.replay_pace_ms)
        .connect_timeout_ms(params.streaming.connect_timeout_ms)
        .read_timeout_ms(params.streaming.timeout_ms)
        .ping_interval_ms(params.streaming.ping_interval_ms)
        .io_read_slice_ms(params.streaming.io_read_slice_ms)
        .keepalive_idle_secs(params.streaming.keepalive_idle_secs)
        .keepalive_interval_secs(params.streaming.keepalive_interval_secs)
        .keepalive_retries(params.streaming.keepalive_retries)
        .host_selection(params.streaming.host_selection)
        .host_shuffle_seed(params.streaming.host_shuffle_seed)
}

// ═══════════════════════════════════════════════════════════════════════
//  #[repr(C)] FPSS streaming event types — zero-copy across FFI
//
//  All of the kind-enum / per-variant struct / ZERO_* const definitions
//  are generated from `thetadatadx-rs/fpss_event_schema.toml`. The
//  hand-written wrapper `FfiBufferedEvent` below owns the backing memory
//  for every borrowed pointer the generated `ThetaDataDxStreamEvent` exposes
//  (the `Contract.symbol` C strings, the typed control variants'
//  `permissions` / `message` strings, and the `Ping` / `UnknownFrame`
//  byte payloads). Split into two include points so the
//  converter (which names `FfiBufferedEvent`) is compiled AFTER the
//  wrapper itself.
// ═══════════════════════════════════════════════════════════════════════

include!("fpss_event_structs.rs");

/// Internal buffered event — owns heap data that backs the `ThetaDataDxStreamEvent`.
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
/// `ThetaDataDxStreamEvent` reference INTO these slots; users MUST NOT retain
/// those pointers past the callback boundary.
#[repr(C)]
pub(crate) struct FfiBufferedEvent {
    pub(crate) event: ThetaDataDxStreamEvent,
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
pub struct ThetaDataDxSubscription {
    /// Subscription kind as a snake_case C string. Per-contract:
    /// `"quote"` / `"trade"` / `"open_interest"` / `"market_value"`.
    /// Full-stream: `"full_trades"` / `"full_open_interest"`. Matches the
    /// Python / TypeScript `Subscription.kind` labels.
    pub kind: *const c_char,
    /// Contract identifier as a C string (e.g. "SPY" or "SPY 20260417 550 C").
    pub contract: *const c_char,
}

// Layout drift-guard: pin the LP64 `#[repr(C)]` size + alignment on the
// Rust side, the same values `abi_struct_layout_asserts.hpp.inc` pins. A
// field-width or member-order change that shifts the layout fails the build
// here, before the C header and its C++ asserts can drift; the C++
// static_asserts alone cannot catch a Rust-side `#[repr(C)]` change.
const _: () = {
    assert!(core::mem::size_of::<ThetaDataDxSubscription>() == 16);
    assert!(core::mem::align_of::<ThetaDataDxSubscription>() == 8);
};

/// Array of active subscriptions returned by `thetadatadx_client_active_subscriptions`
/// and `thetadatadx_streaming_active_subscriptions`.
#[repr(C)]
pub struct ThetaDataDxSubscriptionArray {
    /// Pointer to the first element; null when empty.
    pub data: *const ThetaDataDxSubscription,
    /// Number of elements in the array.
    pub len: usize,
}

// Layout drift-guard: pin the LP64 `#[repr(C)]` size + alignment on the
// Rust side, matching `abi_struct_layout_asserts.hpp.inc`.
const _: () = {
    assert!(core::mem::size_of::<ThetaDataDxSubscriptionArray>() == 16);
    assert!(core::mem::align_of::<ThetaDataDxSubscriptionArray>() == 8);
};

/// Free both CString pointers on a `ThetaDataDxSubscription` if present.
/// Centralises the `// SAFETY: produced by CString::into_raw …`
/// annotation for every drop path that reclaims subscription
/// strings. The function takes a reference rather than ownership
/// because `ThetaDataDxSubscriptionArray::data` holds the values inside a
/// `Box<[ThetaDataDxSubscription]>` and the caller drops that box separately.
///
/// # Safety
///
/// `sub.kind` and `sub.contract` MUST each be either null or a
/// pointer produced by `CString::into_raw` on a matching path that
/// has not been freed yet. Concurrent free of the same pointer is
/// undefined behaviour.
unsafe fn drop_subscription_cstrings(sub: &ThetaDataDxSubscription) {
    if !sub.kind.is_null() {
        // SAFETY: per the function-level safety contract.
        drop(unsafe { CString::from_raw(sub.kind.cast_mut()) });
    }
    if !sub.contract.is_null() {
        // SAFETY: per the function-level safety contract.
        drop(unsafe { CString::from_raw(sub.contract.cast_mut()) });
    }
}

/// Build a `ThetaDataDxSubscriptionArray` from an iterator of `(kind_debug, contract_display)` pairs.
fn build_subscription_array<I>(iter: I) -> *mut ThetaDataDxSubscriptionArray
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
                // SAFETY: every `s.kind` / `s.contract` came from
                // `CString::into_raw` two iterations earlier on the
                // success path; nothing else can have freed them.
                unsafe { drop_subscription_cstrings(s) };
            }
            set_error("subscription kind contains null byte");
            return ptr::null_mut();
        };
        let contract_c = if let Ok(c) = CString::new(contract.as_str()) {
            c
        } else {
            drop(kind_c); // free the kind we just allocated
            for s in &subs {
                // SAFETY: see contract on `drop_subscription_cstrings`.
                unsafe { drop_subscription_cstrings(s) };
            }
            set_error("subscription contract contains null byte");
            return ptr::null_mut();
        };
        subs.push(ThetaDataDxSubscription {
            kind: kind_c.into_raw().cast_const(),
            contract: contract_c.into_raw().cast_const(),
        });
    }
    let len = subs.len();
    let data = if subs.is_empty() {
        ptr::null()
    } else {
        let boxed = subs.into_boxed_slice();
        Box::into_raw(boxed) as *const ThetaDataDxSubscription
    };
    Box::into_raw(Box::new(ThetaDataDxSubscriptionArray { data, len }))
}

/// Free a `ThetaDataDxSubscriptionArray` returned by `thetadatadx_client_active_subscriptions`
/// or `thetadatadx_streaming_active_subscriptions`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_subscription_array_free(
    arr: *mut ThetaDataDxSubscriptionArray,
) {
    ffi_boundary!((), {
        if arr.is_null() {
            return;
        }
        // SAFETY: the pointer was returned by Box::into_raw / thetadatadx_*_new and has not been freed; ownership returns to Rust.
        let arr = unsafe { Box::from_raw(arr) };
        if !arr.data.is_null() && arr.len > 0 {
            // SAFETY: data + len describe a contiguous slice the caller is required to keep valid for the call duration.
            let slice = unsafe { std::slice::from_raw_parts(arr.data.cast_mut(), arr.len) };
            for sub in slice {
                // SAFETY: every `sub` was produced by
                // `build_subscription_array`, which sources both
                // CString pointers from `CString::into_raw` and never
                // mutates them after. This is the matching free.
                unsafe { drop_subscription_cstrings(sub) };
            }
            // Reconstruct and drop the boxed slice.
            // SAFETY: `arr.data` was returned by `Box::into_raw` on a `Box<[ThetaDataDxSubscriptionRecord]>` of length `arr.len`; ownership returns to Rust for drop. Per-element CString and contract pointers were freed in the loop above.
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
/// Authenticates once, opens gRPC channel. Call `thetadatadx_client_set_callback()`
/// later to start FPSS. Historical endpoints are available immediately.
///
/// Returns null on connection/auth failure (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_connect(
    creds: *const ThetaDataDxCredentials,
    config: *const ThetaDataDxConfig,
) -> *mut ThetaDataDxClient {
    ffi_boundary!(ptr::null_mut(), {
        crate::ensure_crypto_provider();
        if creds.is_null() {
            set_error("credentials handle is null");
            return ptr::null_mut();
        }
        if config.is_null() {
            set_error("config handle is null");
            return ptr::null_mut();
        }
        // SAFETY: creds is a non-null pointer returned by thetadatadx_credentials_from_email / thetadatadx_credentials_from_file and not yet freed.
        let creds = unsafe { &*creds };
        // SAFETY: config is a non-null pointer returned by thetadatadx_direct_config_new and not yet freed.
        let config = unsafe { &*config };

        match crate::runtime_from_config(&config.inner.runtime).block_on(
            thetadatadx::Client::connect(&creds.inner, config.inner.clone()),
        ) {
            Ok(client) => Box::into_raw(Box::new(ThetaDataDxClient {
                inner: client,
                callback: Mutex::new(None),
            })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Connect a unified client, loading credentials from a file
/// (line 1 = email, line 2 = password) instead of a credentials handle.
///
/// One-call equivalent of `thetadatadx_credentials_from_file` followed by
/// `thetadatadx_client_connect`: the credentials are opened from `path`,
/// consumed for the connect, and freed internally. The returned handle
/// and its ownership / free convention are identical to
/// `thetadatadx_client_connect` (free with `thetadatadx_client_free`).
///
/// Returns null on argument validation or connection/auth failure
/// (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_connect_from_file(
    path: *const c_char,
    config: *const ThetaDataDxConfig,
) -> *mut ThetaDataDxClient {
    ffi_boundary!(ptr::null_mut(), {
        // SAFETY: `path` is a NUL-terminated C string valid for the call;
        // `thetadatadx_credentials_from_file` validates non-null + UTF-8 and sets
        // `thetadatadx_last_error()` on failure.
        let creds = unsafe { crate::auth::thetadatadx_credentials_from_file(path) };
        if creds.is_null() {
            return ptr::null_mut();
        }
        // SAFETY: `creds` was just allocated by `thetadatadx_credentials_from_file`
        // and is owned by this function; `thetadatadx_client_connect` borrows it
        // and we free it unconditionally below.
        let handle = unsafe { thetadatadx_client_connect(creds, config) };
        // SAFETY: `creds` is the non-null handle checked above;
        // `thetadatadx_client_connect` only borrowed it, so this scope still owns
        // it and frees it exactly once.
        unsafe { crate::auth::thetadatadx_credentials_free(creds) };
        handle
    })
}

/// Register a queued FPSS callback on the unified client and start streaming.
///
/// `callback` is invoked from the LMAX event-dispatch consumer thread for
/// every FPSS event the reader pulls off the wire. Each invocation is
/// wrapped in [`std::panic::catch_unwind`], which contains a panic raised by
/// our own Rust code on the dispatch path so it does not kill the consumer.
/// `callback` itself runs under the C ABI and must not unwind across the
/// boundary: an exception or `longjmp` escaping it is undefined behavior and
/// is not contained by that wrapper (see [`ThetaDataDxStreamCallback`]). The
/// TLS reader publishes events into a pre-allocated ring via
/// `Producer::try_publish`; on overflow the event is dropped and the
/// per-handle drop counter (queryable via `thetadatadx_client_dropped_events`)
/// ticks. The reader thread NEVER blocks on `callback`.
///
/// # `ctx` lifetime + thread affinity
///
/// `ctx` is an opaque pointer passed back unchanged on every invocation.
/// It MUST remain valid until ONE of the following barriers completes:
///
/// - `thetadatadx_client_free` returns. `_free` calls `stop_streaming`
///   internally and polls the post-stop drain barrier with a 5-second
///   timeout, so on a non-overrun return the previous Disruptor
///   consumer has finished firing the callback. In the
///   timeout-overrun path (rare; emits a `tracing::error!`) the
///   consumer may still be firing, so under that diagnostic `ctx`
///   MUST remain valid past return.
/// - `thetadatadx_client_stop_streaming` / `thetadatadx_client_reconnect` returns
///   AND `thetadatadx_client_await_drain` has returned `1` for the prior
///   session.
/// - A successful replacement `thetadatadx_client_set_callback` has returned
///   AND `thetadatadx_client_await_drain` has returned `1` (the replacement
///   path races against the prior session's residual events the same
///   way stop / reconnect does).
///
/// Pass NULL if the callback does not need a context.
///
/// `ctx` is accessed from the event-dispatch consumer thread (NOT the FPSS
/// TLS reader thread). The consumer invokes `callback(event, ctx)`
/// serially on a single thread, so the user does not need internal
/// locks for callback-private state. Freeing `ctx` early — including
/// the moment `thetadatadx_client_stop_streaming` / `_reconnect` returns
/// without an intervening `_await_drain`, or before `thetadatadx_client_free`
/// returns — is undefined behavior: the consumer continues firing
/// in-flight events until its exit path joins.
///
/// The `event` pointer handed to `callback` is valid only for the
/// duration of that invocation. Copy any fields the consumer wants to
/// outlive the callback before returning.
///
/// # Lifecycle contract (REPLACEMENT after stop)
///
/// After `thetadatadx_client_stop_streaming` the unified client accepts a
/// fresh `thetadatadx_client_set_callback`; the new `(callback, ctx)` REPLACES
/// the saved registration. This is intentionally different from the
/// FPSS-handle one-shot rule: the unified path is the high-level API,
/// where stop+restart is a normal user flow (`thetadatadx_client_reconnect`
/// is built on top of it).
///
/// On the first call after `thetadatadx_client_connect` this is the initial
/// registration. Calling `thetadatadx_client_set_callback` while streaming is
/// already active returns -1 with `"streaming already started"`.
///
/// Returns 0 on success, -1 on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_set_callback(
    handle: *const ThetaDataDxClient,
    callback: Option<ThetaDataDxStreamCallback>,
    ctx: *mut c_void,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        // A C caller can pass a null function pointer; modelling the
        // parameter as `Option` lets the null bit pattern be represented
        // and rejected here, instead of being stored and invoked on the
        // dispatcher thread where a call through address 0 would fault the
        // process beyond the reach of the unwind boundary.
        let Some(callback) = callback else {
            set_error("callback function pointer is null");
            return -1;
        };
        let cb = FfiCallback { callback, ctx };
        // Hold `callback` across the gate, the store, and `start_streaming`
        // so registration is serialised the same way the FPSS path holds
        // `dispatcher` across `reject_if_not_fresh` + `open_fpss`. Two racing
        // self-calls take this lock in turn: the first installs and starts,
        // the second observes the now-`Live` slot and is rejected, so the
        // started session's `(callback, ctx)` can never diverge from the
        // stored registration. The dispatcher invokes the `cb` captured by
        // copy below, never reading this mutex, so holding it across
        // `start_streaming` does not deadlock the first delivered event.
        let mut guard = handle.callback.lock_recover();
        // Registration is one-shot while a session is live: reject a second
        // call without disturbing the live session's stored `(callback, ctx)`.
        // Replacement is only permitted after `thetadatadx_client_stop_streaming`
        // (or `_reconnect`), where the slot is no longer `Live`. This makes the
        // documented `-1` + "streaming already started" contract real instead
        // of letting the prior registration be clobbered by an overwrite.
        if handle.inner.stream().is_streaming() {
            set_error("streaming already started");
            return -1;
        }
        // The slot is not `Live`, so this is either the first registration or a
        // replacement after stop. Store BEFORE `start_streaming` so the engine
        // observes a consistent handle; roll back to `None` if start fails.
        *guard = Some(cb);
        match handle.inner.stream().start_streaming(
            move |event: &thetadatadx::fpss::StreamEvent| {
                cb.invoke(event);
            },
        ) {
            Ok(()) => 0,
            Err(e) => {
                *guard = None;
                set_error_from(&e);
                -1
            }
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Polymorphic subscription request payload
// ═══════════════════════════════════════════════════════════════════════

/// Per-contract subscription scope: one named contract.
pub const THETADATADX_SUB_SCOPE_CONTRACT: i32 = 0;
/// Full-stream subscription scope: every contract of a security type.
pub const THETADATADX_SUB_SCOPE_FULL: i32 = 1;

// Per-contract / full-stream tick kind discriminators. The set
// reachable from each scope is constrained:
//
// - `THETADATADX_SUB_SCOPE_CONTRACT` accepts `QUOTE`, `TRADE`, `OPEN_INTEREST`,
//   `MARKET_VALUE`.
// - `THETADATADX_SUB_SCOPE_FULL` accepts `TRADE`, `OPEN_INTEREST` (full-stream
//   quote and market value are rejected — both are addressed
//   per-contract only).
/// Quote tick stream (per-contract scope only).
pub const THETADATADX_SUB_KIND_QUOTE: i32 = 0;
/// Trade tick stream.
pub const THETADATADX_SUB_KIND_TRADE: i32 = 1;
/// Open-interest stream.
pub const THETADATADX_SUB_KIND_OPEN_INTEREST: i32 = 2;
/// Market-value stream (per-contract scope only).
pub const THETADATADX_SUB_KIND_MARKET_VALUE: i32 = 3;

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
pub struct ThetaDataDxSubscriptionRequest {
    /// `THETADATADX_SUB_SCOPE_CONTRACT` or `THETADATADX_SUB_SCOPE_FULL`.
    pub scope: i32,
    /// `THETADATADX_SUB_KIND_QUOTE` / `_TRADE` / `_OPEN_INTEREST`.
    pub kind: i32,
    /// Stock or underlying symbol for per-contract subscriptions.
    /// NULL for full-stream.
    pub symbol: *const c_char,
    /// Option expiration as `YYYYMMDD`. NULL for non-option per-contract
    /// or for full-stream subscriptions.
    pub expiration: *const c_char,
    /// Option strike price. NULL for non-option per-contract or for
    /// full-stream subscriptions.
    pub strike: *const c_char,
    /// Option right (`"C"` / `"P"`). NULL for non-option per-contract or
    /// for full-stream subscriptions.
    pub right: *const c_char,
    /// `"STOCK"` / `"OPTION"` / `"INDEX"` for full-stream
    /// subscriptions. NULL for per-contract subscriptions.
    pub sec_type: *const c_char,
}

// Layout drift-guard: pin the LP64 `#[repr(C)]` size + alignment on the
// Rust side, matching `abi_struct_layout_asserts.hpp.inc`. `scope` (i32)
// @0, `kind` (i32) @4, then six pointer fields packed 8 bytes apart with no
// interior padding -> 48 bytes, align 8.
const _: () = {
    assert!(core::mem::size_of::<ThetaDataDxSubscriptionRequest>() == 48);
    assert!(core::mem::align_of::<ThetaDataDxSubscriptionRequest>() == 8);
};

/// Decode a `ThetaDataDxSubscriptionRequest` into a Rust [`Subscription`]. The
/// helper sets `thetadatadx_last_error` on validation failure and returns
/// `None`. Used by both the unified and standalone-FPSS C ABI entry
/// points.
unsafe fn coerce_subscription(
    req: *const ThetaDataDxSubscriptionRequest,
) -> Option<thetadatadx::fpss::protocol::Subscription> {
    use thetadatadx::fpss::protocol::{
        Contract, FullSubscriptionKind, OptionLeg, Subscription, SubscriptionKind,
    };
    if req.is_null() {
        set_error("subscription request is null");
        return None;
    }
    // SAFETY: req is a non-null pointer to a caller-owned FFI request struct kept alive for the call duration.
    let req = unsafe { &*req };
    let symbol_ptr = req.symbol;
    let expiration_ptr = req.expiration;
    let strike_ptr = req.strike;
    let right_ptr = req.right;
    let sec_type_ptr = req.sec_type;
    match req.scope {
        THETADATADX_SUB_SCOPE_CONTRACT => {
            let kind = match req.kind {
                THETADATADX_SUB_KIND_QUOTE => SubscriptionKind::Quote,
                THETADATADX_SUB_KIND_TRADE => SubscriptionKind::Trade,
                THETADATADX_SUB_KIND_OPEN_INTEREST => SubscriptionKind::OpenInterest,
                THETADATADX_SUB_KIND_MARKET_VALUE => SubscriptionKind::MarketValue,
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
                    match Contract::option(
                        symbol,
                        OptionLeg {
                            expiration: exp,
                            strike: stk,
                            right: rt,
                        },
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            set_error_from(&e);
                            return None;
                        }
                    }
                };
            Some(Subscription::Contract { contract, kind })
        }
        THETADATADX_SUB_SCOPE_FULL => {
            let kind = match req.kind {
                THETADATADX_SUB_KIND_TRADE => FullSubscriptionKind::Trades,
                THETADATADX_SUB_KIND_OPEN_INTEREST => FullSubscriptionKind::OpenInterest,
                THETADATADX_SUB_KIND_QUOTE => {
                    set_error("full-stream Quote is not a valid subscription");
                    return None;
                }
                THETADATADX_SUB_KIND_MARKET_VALUE => {
                    set_error("full-stream MarketValue is not a valid subscription");
                    return None;
                }
                other => {
                    set_error(&format!("invalid kind {other}"));
                    return None;
                }
            };
            let sec_type_str = require_cstr!(sec_type_ptr, None);
            let sec_type = match sec_type_str.to_uppercase().as_str() {
                "STOCK" => thetadatadx::SecType::Stock,
                "OPTION" => thetadatadx::SecType::Option,
                "INDEX" => thetadatadx::SecType::Index,
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
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_subscribe(
    handle: *const ThetaDataDxClient,
    request: *const ThetaDataDxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        // SAFETY: `request` is a non-null `*const ThetaDataDxSubscriptionRequest` the caller pins for the call duration; `coerce_subscription` validates its discriminant + tagged-union fields, setting `thetadatadx_last_error` on malformed payloads.
        let sub = match unsafe { coerce_subscription(request) } {
            Some(s) => s,
            None => return -1,
        };
        // SAFETY: `handle` is a non-null `*const ThetaDataDxClient` returned by `thetadatadx_client_*_new` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let handle = unsafe { &*handle };
        match handle.inner.stream().subscribe(sub) {
            Ok(()) => 0,
            Err(e) => {
                set_error_from(&e);
                -1
            }
        }
    })
}

/// Polymorphic unsubscribe on the unified client.
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_unsubscribe(
    handle: *const ThetaDataDxClient,
    request: *const ThetaDataDxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        // SAFETY: `request` is a non-null `*const ThetaDataDxSubscriptionRequest` the caller pins for the call duration; `coerce_subscription` validates its discriminant + tagged-union fields, setting `thetadatadx_last_error` on malformed payloads.
        let sub = match unsafe { coerce_subscription(request) } {
            Some(s) => s,
            None => return -1,
        };
        // SAFETY: `handle` is a non-null `*const ThetaDataDxClient` returned by `thetadatadx_client_*_new` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let handle = unsafe { &*handle };
        match handle.inner.stream().unsubscribe(sub) {
            Ok(()) => 0,
            Err(e) => {
                set_error_from(&e);
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
/// `thetadatadx_client_set_callback`. Returns -1 if no callback was registered
/// (the new ABI has no out-of-band buffer to fall back on).
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
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
/// based on `thetadatadx_client_reconnect` returning is unsound — the old
/// callback is still in flight. Drive reconnect from a separate
/// thread and call `thetadatadx_client_await_drain` between stop and any
/// `ctx` replacement.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_reconnect(handle: *const ThetaDataDxClient) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("unified handle is null");
            return -1;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };

        // Save active subscriptions. If streaming isn't running (or the
        // subscription locks are poisoned upstream) we must abort the
        // reconnect -- silently falling back to an empty list drops every
        // subscription on the floor.
        let saved_subs = match handle.inner.stream().active_subscriptions() {
            Ok(subs) => subs,
            Err(e) => {
                set_error_from(&e);
                return -1;
            }
        };
        let saved_full_subs = match handle.inner.stream().active_full_subscriptions() {
            Ok(subs) => subs,
            Err(e) => {
                set_error_from(&e);
                return -1;
            }
        };

        // Look up the previously-registered callback so we can re-attach
        // it on the new FPSS connection. `thetadatadx_client_reconnect` requires
        // a prior `set_callback` — without one there is no destination
        // for the new stream's events.
        let cb = {
            let guard = handle.callback.lock_recover();
            match *guard {
                Some(cb) => cb,
                None => {
                    set_error(
                        "no callback registered -- call thetadatadx_client_set_callback \
                         before thetadatadx_client_reconnect",
                    );
                    return -1;
                }
            }
        };

        // Initiate teardown of the FPSS reader and Disruptor consumer
        // for the current session. This swaps the streaming slot to
        // `Stopped` and signals the I/O thread; the consumer keeps
        // firing the old C callback until its exit path joins.
        handle.inner.stream().stop_streaming();

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
            .stream()
            .await_drain(std::time::Duration::from_millis(5_000))
        {
            set_error(
                "reconnect drain barrier timed out after 5s — previous \
                 callback is still in flight; refusing to bind the new \
                 session to the same ctx",
            );
            return -1;
        }

        let result =
            handle
                .inner
                .stream()
                .start_streaming(move |event: &thetadatadx::fpss::StreamEvent| {
                    cb.invoke(event);
                });
        if let Err(e) = result {
            set_error_from(&e);
            return -1;
        }

        // Re-subscribe all previous subscriptions through the core's
        // paced replay engine (best-effort; failures are non-fatal but
        // surfaced through tracing so ops can see silent
        // re-subscription failures across a reconnect boundary — a
        // dropped subscription here would otherwise manifest as "the
        // stream is up but no ticks for AAPL" with no log trail).
        // Pacing spreads a large saved set over wall-clock time
        // instead of firing it at a recovering upstream back-to-back.
        if let Err(e) = handle
            .inner
            .stream()
            .restore_subscriptions(&saved_subs, &saved_full_subs)
        {
            tracing::warn!(
                target: "thetadatadx::ffi::reconnect",
                error = %e,
                "subscription replay reported failures after reconnect"
            );
        }

        0
    })
}

/// Check if streaming is active on the unified client.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_is_streaming(handle: *const ThetaDataDxClient) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        i32::from(handle.inner.stream().is_streaming())
    })
}

/// Check if the live streaming session is authenticated on the unified
/// client.
///
/// Distinct from `thetadatadx_client_is_streaming`: the session can be live
/// yet briefly unauthenticated mid-reconnect. Returns 1 when
/// authenticated, 0 otherwise (including a null handle, before streaming
/// starts, and after it stops).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_is_authenticated(
    handle: *const ThetaDataDxClient,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        i32::from(handle.inner.stream().is_authenticated())
    })
}

/// Get active subscriptions as a typed array. Returns null on error.
///
/// Caller must free the result with `thetadatadx_subscription_array_free`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_active_subscriptions(
    handle: *const ThetaDataDxClient,
) -> *mut ThetaDataDxSubscriptionArray {
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null_mut();
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        match handle.inner.stream().active_subscriptions() {
            Ok(subs) => build_subscription_array(
                subs.iter()
                    .map(|(k, c)| (k.kind_str().to_string(), format!("{c}"))),
            ),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Get active full-stream subscriptions as a typed array. Returns
/// null on error.
///
/// Each entry's `contract` field carries the security-type discriminant
/// (`"Stock"` / `"Option"` / `"Index"`) the full-stream subscription is
/// bound to. The `kind` field is the snake_case full-stream kind label
/// (`"full_trades"` / `"full_open_interest"`), matching the Python /
/// TypeScript `Subscription.kind` accessors. Per-contract-only kinds
/// (`Quote` / `MarketValue`) have no full-stream form and are omitted.
///
/// Caller must free the result with `thetadatadx_subscription_array_free`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_active_full_subscriptions(
    handle: *const ThetaDataDxClient,
) -> *mut ThetaDataDxSubscriptionArray {
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null_mut();
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        match handle.inner.stream().active_full_subscriptions() {
            Ok(subs) => build_subscription_array(subs.iter().filter_map(|(k, st)| {
                k.full_kind_str()
                    .map(|kind| (kind.to_string(), format!("{st:?}")))
            })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    })
}

/// Borrow the historical client from a unified handle.
///
/// Returns a `*const ThetaDataDxHistoricalClient` that can be passed to all `thetadatadx_stock_*`,
/// `thetadatadx_option_*`, `thetadatadx_index_*`, `thetadatadx_calendar_*`, and `thetadatadx_interest_rate_*`
/// functions. This avoids a second `thetadatadx_historical_connect()` call and reuses
/// the same authenticated session.
///
/// The returned pointer is **NOT owned** -- do NOT call `thetadatadx_historical_free`
/// on it. It is valid as long as the `ThetaDataDxClient` handle is alive.
///
/// # Safety
///
/// This cast is sound because `ThetaDataDxHistoricalClient` is `#[repr(transparent)]` over
/// `HistoricalClient`, and `Client` Derefs to `&HistoricalClient`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_historical(
    handle: *const ThetaDataDxClient,
) -> *const ThetaDataDxHistoricalClient {
    ffi_boundary!(std::ptr::null(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null();
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        // ThetaDataDxHistoricalClient is #[repr(transparent)] over HistoricalClient, so this cast is safe.
        let mdds_ref: &thetadatadx::mdds::HistoricalClient = handle.inner.historical();
        std::ptr::from_ref::<thetadatadx::mdds::HistoricalClient>(mdds_ref)
            .cast::<ThetaDataDxHistoricalClient>()
    })
}

/// Stop streaming on the unified client. Historical remains available.
///
/// Initiates teardown of the FPSS event-dispatch consumer thread and the
/// underlying TLS reader, but returns immediately after the streaming
/// state cell is swapped to `Stopped`. The old consumer continues
/// firing the previously-registered C callback for any events still
/// in-flight in the ring buffer until its exit path joins. Use
/// `thetadatadx_client_await_drain` to confirm the consumer has finished
/// firing the callback before freeing `ctx` or replacing the
/// callback registration. The saved `(callback, ctx)` itself is
/// preserved so a subsequent `thetadatadx_client_reconnect` can re-attach it
/// without the caller re-supplying the function pointer.
///
/// # Lifecycle restriction
///
/// MUST NOT be called from inside the user callback. Doing so
/// returns control to the caller while the old callback is still
/// firing on the event-dispatch consumer thread; freeing or replacing
/// `ctx` based on stop returning will trigger use-after-free in the
/// callback. Drive stop / reconnect from a separate thread instead,
/// then call `thetadatadx_client_await_drain` for the quiescence barrier.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_stop_streaming(handle: *const ThetaDataDxClient) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        handle.inner.stream().stop_streaming();
    })
}

/// Milliseconds since the most recent inbound streaming frame of any
/// kind (data tick, heartbeat, control) on this unified handle.
///
/// The operator-facing staleness clock: a healthy session stays in
/// the low hundreds of milliseconds (the upstream heartbeats even
/// when no market data flows), so a steadily growing value is the
/// earliest external signal of a dead or wedged connection.
///
/// Writes the value into `*out_ms`. Returns `0` on success, `1` when
/// streaming has not started or no frame has been received yet
/// (`*out_ms` is left `0`), `-1` on a null pointer.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_millis_since_last_event(
    handle: *const ThetaDataDxClient,
    out_ms: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() || out_ms.is_null() {
            set_error("handle or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        match handle.inner.stream().millis_since_last_event() {
            Some(ms) => {
                // SAFETY: out_ms checked non-null above; the FFI contract pins the storage for the call duration.
                unsafe {
                    *out_ms = ms;
                }
                0
            }
            None => {
                // SAFETY: out_ms checked non-null above; the FFI contract pins the storage for the call duration.
                unsafe {
                    *out_ms = 0;
                }
                1
            }
        }
    })
}

/// UNIX-nanosecond receive timestamp of the most recent inbound
/// streaming frame of any kind on this unified handle. Returns `0`
/// when the handle is null, streaming has not started, or no frame
/// has been received yet. Raw feed for
/// `thetadatadx_client_millis_since_last_event`, exposed for callers
/// correlating against their own pipeline timestamps.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_last_event_received_at_unix_nanos(
    handle: *const ThetaDataDxClient,
) -> i64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        unsafe { (*handle).inner.stream().last_event_received_at_unix_nanos() }
    })
}

/// Address (`host:port`) of the streaming server the current session
/// is connected to, following the session across auto-reconnects.
///
/// Returns a heap-owned C string the caller must release with
/// `thetadatadx_string_free`, or null when streaming has not started (or the
/// handle is null).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_last_connected_addr(
    handle: *const ThetaDataDxClient,
) -> *mut std::os::raw::c_char {
    ffi_boundary!(ptr::null_mut(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null_mut();
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        match handle.inner.stream().last_connected_addr() {
            Some(addr) => match std::ffi::CString::new(addr) {
                Ok(c) => c.into_raw(),
                Err(e) => {
                    set_error(&format!("connected address contains an interior NUL: {e}"));
                    ptr::null_mut()
                }
            },
            None => ptr::null_mut(),
        }
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
pub unsafe extern "C" fn thetadatadx_client_dropped_events(
    handle: *const ThetaDataDxClient,
) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        unsafe { (*handle).inner.stream().dropped_event_count() }
    })
}

/// Point-in-time count of streaming events published into the event
/// ring but not yet drained into the registered callback — the
/// in-flight depth between the I/O thread and the dispatcher.
///
/// The leading back-pressure signal: `thetadatadx_client_dropped_events`
/// only moves AFTER data has been lost, while a rising occupancy that
/// approaches `thetadatadx_client_ring_capacity` predicts those drops while
/// there is still time to react. Sampling never blocks the feed —
/// it is a pair of relaxed atomic loads on the calling thread; safe
/// to poll from any thread at any cadence.
///
/// Returns 0 if the handle is null or no callback has been installed
/// yet.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_ring_occupancy(
    handle: *const ThetaDataDxClient,
) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        unsafe { (*handle).inner.stream().ring_occupancy() as u64 }
    })
}

/// Configured capacity of the streaming event ring in slots (the
/// `streaming_ring_size` setting, a power of two), the fixed denominator
/// for `thetadatadx_client_ring_occupancy`. When the occupancy sample
/// approaches this value the ring is saturating and further events
/// will be dropped (counted by `thetadatadx_client_dropped_events`).
///
/// Returns 0 if the handle is null or no callback has been installed
/// yet.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_ring_capacity(handle: *const ThetaDataDxClient) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        unsafe { (*handle).inner.stream().ring_capacity() as u64 }
    })
}

/// Cumulative count of user-callback panics caught by the per-invocation
/// `catch_unwind` boundary on this unified handle since the current
/// stream started.
///
/// Each caught panic is also surfaced via `tracing::error!` with target
/// `thetadatadx::fpss::poller`. A panic in the callback is caught,
/// recorded here, and does not stop event delivery — the next event
/// continues normally. Safe to call from any thread without blocking.
///
/// Returns 0 if the handle is null or no callback has been installed yet.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_panic_count(handle: *const ThetaDataDxClient) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        unsafe { (*handle).inner.stream().panic_count() }
    })
}

/// Wait for the previously-superseded streaming session to quiesce.
///
/// Returns `1` once the previous `thetadatadx_client_stop_streaming` /
/// `_reconnect` session's event-dispatch consumer thread has finished
/// firing the registered callback. Returns `0` on timeout or when no
/// stream has been stopped on this handle.
///
/// # When to call
///
/// After `thetadatadx_client_stop_streaming` or `thetadatadx_client_reconnect`
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
pub unsafe extern "C" fn thetadatadx_client_await_drain(
    handle: *const ThetaDataDxClient,
    timeout_ms: u64,
) -> i32 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        let timeout = std::time::Duration::from_millis(timeout_ms);
        i32::from(handle.inner.stream().await_drain(timeout))
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
/// Calling `thetadatadx_client_await_drain` from another thread before invoking
/// `thetadatadx_client_free` is no longer required for callback-context lifetime
/// safety — `_free` now serves as the public drain barrier as well.
///
/// # Lifecycle restriction
///
/// Do NOT call `thetadatadx_client_free` from inside the user callback. The
/// callback runs on the dispatcher thread; `_free` waits for that
/// thread to exit before destroying the handle. Issuing `_free` from
/// inside the callback means the dispatcher cannot exit while
/// `_free` is waiting on it. The 5-second drain budget elapses,
/// `_free` logs the overrun and proceeds to destruction; control
/// then returns into the user callback which is now operating
/// against freed memory. Drive `_free` from a separate thread.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_free(handle: *mut ThetaDataDxClient) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }
        // SAFETY: the pointer was returned by Box::into_raw / thetadatadx_*_new and has not been freed; ownership returns to Rust.
        let handle = unsafe { Box::from_raw(handle) };

        // Raise the stop signal first. `stop_streaming` is idempotent
        // on an already-stopped slot and, when the slot was `Live`,
        // captures the drain flag of the superseded session into the
        // client's `prev_drained` slot — the flag we poll below.
        //
        // Importantly, if the caller already invoked
        // `thetadatadx_client_stop_streaming` before `_free`, `is_streaming()`
        // is already `false` here, but `prev_drained` was populated by
        // that earlier `stop_streaming` call. The barrier MUST poll
        // `prev_drained` regardless of the current slot state — the
        // earlier-stop path is the one most likely to hit a callback
        // still firing on the event-dispatch consumer thread.
        handle.inner.stream().stop_streaming();

        // Wait for the consumer thread to finish firing the registered
        // callback before we destroy the handle. This is the strict
        // `free` contract: returning only after the
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
        let had_prior_session = handle.inner.stream().prev_drained_is_set();
        if had_prior_session && !handle.inner.stream().await_drain(FREE_DRAIN_TIMEOUT) {
            tracing::error!(
                target: "thetadatadx::ffi",
                timeout_ms = FREE_DRAIN_TIMEOUT.as_millis() as u64,
                "thetadatadx_client_free: drain barrier exceeded timeout -- callback may still \
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

/// Allocate a streaming handle and stash the connection parameters.
///
/// **Does NOT open the FPSS TLS connection** — connection is deferred
/// until the caller installs a callback via `thetadatadx_streaming_set_callback`.
/// This is required because `StreamingClient::connect` registers its event
/// handler at connect time; deferring the connect until callback
/// installation lets us avoid an internal queue.
///
/// Returns null on argument validation failure (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_connect(
    creds: *const ThetaDataDxCredentials,
    config: *const ThetaDataDxConfig,
) -> *mut ThetaDataDxStreamHandle {
    ffi_boundary!(std::ptr::null_mut(), {
        crate::ensure_crypto_provider();
        if creds.is_null() {
            set_error("credentials handle is null");
            return ptr::null_mut();
        }
        if config.is_null() {
            set_error("config handle is null");
            return ptr::null_mut();
        }
        // SAFETY: creds is a non-null pointer returned by thetadatadx_credentials_from_email / thetadatadx_credentials_from_file and not yet freed.
        let creds = unsafe { &*creds };
        // SAFETY: config is a non-null pointer returned by thetadatadx_direct_config_new and not yet freed.
        let config = unsafe { &*config };

        // Seed the process-global async runtime from this client's config so
        // `worker_threads` is honored when a standalone FPSS client is the
        // first client created in the process; the worker pool is built once.
        crate::runtime_from_config(&config.inner.runtime);

        Box::into_raw(Box::new(ThetaDataDxStreamHandle {
            inner: Arc::new(Mutex::new(None)),
            connect_params: StreamingConnectParams {
                creds: creds.inner.clone(),
                streaming: config.inner.streaming.clone(),
                reconnect: config.inner.reconnect.clone(),
            },
            callback: Mutex::new(None),
            state: AtomicU8::new(STREAM_STATE_FRESH),
            prev_drained: Mutex::new(Vec::new()),
            dispatcher: Arc::new(Mutex::new(FfpssDispatcherSession::Idle)),
        }))
    })
}

/// Allocate a streaming handle, loading credentials from a file
/// (line 1 = email, line 2 = password) instead of a credentials handle.
///
/// One-call equivalent of `thetadatadx_credentials_from_file` followed by
/// `thetadatadx_streaming_connect`: the credentials are opened from `path`, consumed
/// for the connect, and freed internally. As with `thetadatadx_streaming_connect`
/// this does NOT open the FPSS TLS connection — connection is deferred
/// until `thetadatadx_streaming_set_callback`. The returned handle and its ownership
/// / free convention are identical to `thetadatadx_streaming_connect` (free with
/// `thetadatadx_streaming_free`).
///
/// Returns null on argument validation failure (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_connect_from_file(
    path: *const c_char,
    config: *const ThetaDataDxConfig,
) -> *mut ThetaDataDxStreamHandle {
    ffi_boundary!(std::ptr::null_mut(), {
        // SAFETY: `path` is a NUL-terminated C string valid for the call;
        // `thetadatadx_credentials_from_file` validates non-null + UTF-8 and sets
        // `thetadatadx_last_error()` on failure.
        let creds = unsafe { crate::auth::thetadatadx_credentials_from_file(path) };
        if creds.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: `creds` was just allocated by `thetadatadx_credentials_from_file`
        // and is owned by this function; `thetadatadx_streaming_connect` clones what it
        // needs and we free it unconditionally below.
        let handle = unsafe { thetadatadx_streaming_connect(creds, config) };
        // SAFETY: `creds` is the non-null handle checked above;
        // `thetadatadx_streaming_connect` cloned what it needed, so this scope still owns
        // it and frees it exactly once.
        unsafe { crate::auth::thetadatadx_credentials_free(creds) };
        handle
    })
}

/// Reject the call if the handle is already past its first
/// registration (`Active`) or has been shut down (`Shutdown`).
///
/// Returns `true` if the caller should proceed (handle is `Fresh`);
/// `false` after setting `thetadatadx_last_error()` to a contract-specific
/// message. Used by `thetadatadx_streaming_set_callback` to enforce one-shot
/// registration and the terminal-shutdown rule.
fn reject_if_not_fresh(handle: &ThetaDataDxStreamHandle) -> bool {
    match handle.state.load(AtomicOrdering::Relaxed) {
        STREAM_STATE_FRESH => true,
        STREAM_STATE_ACTIVE => {
            set_error(
                "streaming callback already installed -- only one set_callback call is permitted per handle",
            );
            false
        }
        STREAM_STATE_SHUTDOWN => {
            set_error("streaming handle has already been shut down -- this is terminal");
            false
        }
        _ => {
            // Unreachable -- state is only ever set to one of the three
            // constants above. Treat as terminal to fail closed.
            set_error("streaming handle in unknown lifecycle state -- refusing operation");
            false
        }
    }
}

/// Reject the call if the handle has been shut down. Used by
/// `thetadatadx_streaming_reconnect` and `thetadatadx_streaming_shutdown` (the latter to make
/// double-shutdown a clean error rather than silently no-op).
fn reject_if_shutdown(handle: &ThetaDataDxStreamHandle) -> bool {
    if handle.state.load(AtomicOrdering::Relaxed) == STREAM_STATE_SHUTDOWN {
        set_error("streaming handle has already been shut down -- this is terminal");
        false
    } else {
        true
    }
}

/// Open the FPSS connection if not already open.
///
/// Internal helper used by `thetadatadx_streaming_set_callback`. The caller supplies
/// a Rust closure that consumes `StreamEvent` references; this is the
/// closure registered with `StreamingClient::connect` and lives for the
/// lifetime of the connection. Returns -1 on connect failure (error
/// already set), 0 on success.
///
/// Lifecycle enforcement (one-shot registration, terminal shutdown)
/// happens upstream in [`reject_if_not_fresh`]; this helper only
/// touches the inner `StreamingClient` slot and spawns the dispatcher.
///
/// Returns the spawned `JoinHandle` on success; callers store it in
/// `handle.dispatcher`.  On failure the error is already set via
/// [`set_error`] / [`set_error_from`] and the caller returns `-1`.
fn open_fpss<F>(
    handle: &ThetaDataDxStreamHandle,
    callback: Option<FfiCallback>,
    mut on_event: F,
) -> Result<std::thread::JoinHandle<()>, ()>
where
    F: FnMut(&thetadatadx::fpss::StreamEvent) + Send + 'static,
{
    let mut guard = handle.inner.lock_recover();
    if guard.is_some() {
        // Belt-and-suspenders: reject_if_not_fresh should already have
        // caught this at the C ABI entry point. Keep the check so a
        // future caller that bypasses the state gate cannot end up
        // double-connecting silently.
        set_error(
            "streaming callback already installed -- only one set_callback call is permitted per handle",
        );
        return Err(());
    }
    let build_result = streaming_builder(&handle.connect_params).build();
    match build_result {
        Ok(client) => {
            let client_arc = std::sync::Arc::new(client);

            // Publish every state slot BEFORE spawning the dispatcher so
            // a callback that fires on the first delivered event sees a
            // fully initialised handle (`inner`, stored callback, and
            // lifecycle state all consistent). Re-entrant teardown calls
            // serialise on `handle.inner.lock()`, which we hold here, so
            // they observe the new state only after this function has
            // returned and the dispatcher is running.
            *guard = Some(std::sync::Arc::clone(&client_arc));
            if let Some(cb) = callback {
                let mut cb_guard = handle.callback.lock_recover();
                *cb_guard = Some(cb);
            }
            handle
                .state
                .store(STREAM_STATE_ACTIVE, AtomicOrdering::Relaxed);

            let dispatcher_client = std::sync::Arc::clone(&client_arc);
            let dispatcher_slot = std::sync::Arc::clone(&handle.dispatcher);
            let spawn_result = std::thread::Builder::new()
                .name("thetadatadx-ffi-fpss-dispatcher".into())
                .spawn(move || {
                    // `StreamingClient::for_each` drives `poll_batch`, which wraps
                    // each callback invocation in its own `catch_unwind`.  A
                    // panic in the handler is caught, recorded via
                    // `panic_count()`, and does not stop event delivery for
                    // subsequent events.  The outer `catch_unwind` below
                    // guards only the event-iteration machinery itself.
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        dispatcher_client.for_each(|event| on_event(event));
                    }));
                    if let Err(payload) = outcome {
                        let reason = downcast_ffi_panic_payload(payload);
                        tracing::error!(
                            target: "thetadatadx::ffi",
                            reason = %reason,
                            "thetadatadx-ffi-fpss-dispatcher panicked in event iteration machinery; handle transitioning to failed state",
                        );
                        // Publish `Failed` from this thread before it exits so
                        // health checks reflect the dead loop immediately, not
                        // only once teardown joins.
                        publish_failed_if_current(
                            &dispatcher_slot,
                            std::thread::current().id(),
                            reason,
                        );
                    }
                });
            match spawn_result {
                Ok(h) => Ok(h),
                Err(e) => {
                    // Roll the publishes back so a failed spawn does not
                    // leave the handle wedged with a `Some(client)` slot
                    // and an ACTIVE state but no dispatcher behind them.
                    let taken = guard.take();
                    handle
                        .state
                        .store(STREAM_STATE_FRESH, AtomicOrdering::Relaxed);
                    if let Some(client) = taken {
                        client.shutdown();
                        drop(client);
                    }
                    *handle.callback.lock_recover() = None;
                    set_error(&format!("failed to spawn streaming dispatcher thread: {e}"));
                    Err(())
                }
            }
        }
        Err(e) => {
            set_error_from(&thetadatadx::error::Error::from(e));
            Err(())
        }
    }
}

/// Register a queued FPSS callback and open the FPSS connection.
///
/// `callback` is invoked from the LMAX event-dispatch consumer thread for
/// every FPSS event the reader pulls off the wire, with each invocation
/// wrapped in [`std::panic::catch_unwind`]. That wrapper contains a panic
/// raised by our own Rust code on the dispatch path so it does not kill the
/// consumer; `callback` itself runs under the C ABI and must not unwind
/// across the boundary, because an exception or `longjmp` escaping it is
/// undefined behavior and is not contained by the wrapper (see
/// [`ThetaDataDxStreamCallback`]). The TLS reader publishes events via
/// `Producer::try_publish`; on ring overflow events are dropped and
/// counted (queryable via `thetadatadx_streaming_dropped_events`). The reader
/// thread NEVER blocks on `callback`.
///
/// `ctx` is an opaque pointer passed back unchanged on every invocation.
/// It MUST remain valid until ONE of the following barriers completes:
///
/// - `thetadatadx_streaming_free` returns (the simple path; `_free` performs the
///   shutdown if the handle is still live and internally polls the
///   drain barrier with a 5-second timeout, so on a non-overrun return
///   the consumer thread has finished firing the callback);
/// - `thetadatadx_streaming_shutdown` (or `thetadatadx_streaming_reconnect`) returns AND
///   `thetadatadx_streaming_await_drain` has confirmed `1`. Stop / reconnect return
///   asynchronously; events still in the ring continue flowing through
///   the old callback until the consumer exits.
///
/// In the `_free` timeout-overrun path (rare; emits a
/// `tracing::error!`) the consumer may still be firing the callback,
/// so under that diagnostic `ctx` MUST remain valid past return; the
/// caller is expected to investigate the wedged callback rather than
/// race destruction. The event-dispatch consumer thread accesses `ctx` on
/// every event and on every `thetadatadx_streaming_reconnect`.
///
/// # Lifecycle contract (FPSS one-shot rule)
///
/// May only be called ONCE per handle, and ONLY before
/// `thetadatadx_streaming_shutdown`. Subsequent calls — including any call after
/// shutdown — return -1 with an error message:
///
/// - second register on an already-active handle:
///   `"streaming callback already installed -- only one set_callback call is permitted per handle"`
/// - register after shutdown:
///   `"streaming handle has already been shut down -- this is terminal"`
///
/// This is intentionally stricter than the unified C ABI's
/// `thetadatadx_client_set_callback`, which supports stop-then-re-register as
/// a normal user flow. The FPSS handle is the low-level surface; the
/// unified handle is the high-level surface. See
/// [`thetadatadx_client_set_callback`] for the replacement contract.
///
/// Returns 0 on success, -1 on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_set_callback(
    handle: *const ThetaDataDxStreamHandle,
    callback: Option<ThetaDataDxStreamCallback>,
    ctx: *mut c_void,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("streaming handle is null");
            return -1;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        // A C caller can pass a null function pointer; modelling the
        // parameter as `Option` lets the null bit pattern be represented
        // and rejected before the dispatcher thread would invoke it.
        let Some(callback) = callback else {
            set_error("callback function pointer is null");
            return -1;
        };
        // Serialise concurrent installs: `dispatcher` mutex prevents two
        // racing callers from each publishing a client into `handle.inner`
        // and orphaning one another's dispatcher.
        let mut dispatcher_guard = handle.dispatcher.lock_recover();
        if !reject_if_not_fresh(handle) {
            return -1;
        }
        let cb = FfiCallback { callback, ctx };
        let dispatch_cb = cb;
        // `open_fpss` publishes the client, the stored callback handle,
        // and the lifecycle state atomically under the inner mutex
        // BEFORE the dispatcher thread is spawned, so a callback that
        // fires on the first delivered event observes a fully
        // initialised handle.
        match open_fpss(
            handle,
            Some(cb),
            move |event: &thetadatadx::fpss::StreamEvent| {
                dispatch_cb.invoke(event);
            },
        ) {
            Ok(h) => {
                // The callback dispatcher parks only on the event ring, which
                // the client shutdown signals on teardown, so no wake hook.
                *dispatcher_guard = FfpssDispatcherSession::Running {
                    handle: h,
                    on_teardown: None,
                    // The C ABI runs its own teardown and never reads this flag.
                    registers_drain_flag: true,
                };
                0
            }
            Err(()) => -1,
        }
    })
}

/// Check if the standalone FPSS streaming connection is currently open.
///
/// Distinct from `thetadatadx_streaming_is_authenticated`: the connection
/// can be open yet briefly unauthenticated mid-reconnect. A panicked
/// dispatcher folds back to `0` so the failed state is uniformly visible
/// across status readers. Returns 1 when streaming, 0 otherwise
/// (including a null handle and after shutdown).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_is_streaming(
    handle: *const ThetaDataDxStreamHandle,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        // Lock order is uniformly `dispatcher` -> `inner` across every
        // lifecycle mutator (set_callback, reconnect, shutdown, free), so a
        // status reader must never hold `inner` while taking `dispatcher`.
        // Snapshot the one bit we need from `inner`, drop that guard, then
        // take `dispatcher` -- the two locks are never held at once here.
        let has_session = {
            let inner_guard = handle.inner.lock_recover();
            inner_guard.as_ref().is_some()
        };
        if !has_session {
            return 0;
        }
        let session = handle.dispatcher.lock_recover();
        if let FfpssDispatcherSession::Failed { reason } = &*session {
            tracing::debug!(
                target: "thetadatadx::ffi",
                reason = %reason,
                "thetadatadx_streaming_is_streaming: dispatcher failed",
            );
            return 0;
        }
        1
    })
}

/// Check if the FPSS client is currently authenticated.
///
/// Returns 1 if authenticated, 0 if not (or if handle is null).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_is_authenticated(
    handle: *const ThetaDataDxStreamHandle,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        // Lock order is uniformly `dispatcher` -> `inner` across every
        // lifecycle mutator, so snapshot the authentication bit from `inner`,
        // drop that guard, THEN take `dispatcher`. Holding both at once in
        // the opposite order is the lock-order inversion that can deadlock
        // against a concurrent mutator.
        let authenticated = {
            let inner_guard = handle.inner.lock_recover();
            match inner_guard.as_ref() {
                Some(c) => c.is_authenticated(),
                None => return 0,
            }
        };
        // A panicked dispatcher folds back to `!authenticated` so status
        // readers see a visible failed state instead of "authenticated with
        // no callbacks".
        let session = handle.dispatcher.lock_recover();
        let dispatcher_failed = if let FfpssDispatcherSession::Failed { reason } = &*session {
            tracing::debug!(
                target: "thetadatadx::ffi",
                reason = %reason,
                "thetadatadx_streaming_is_authenticated: dispatcher failed",
            );
            true
        } else {
            false
        };
        i32::from(authenticated && !dispatcher_failed)
    })
}

/// Get a snapshot of currently active subscriptions.
///
/// Returns a heap-allocated `ThetaDataDxSubscriptionArray` (null on error).
/// Caller must free the result with `thetadatadx_subscription_array_free`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_active_subscriptions(
    handle: *const ThetaDataDxStreamHandle,
) -> *mut ThetaDataDxSubscriptionArray {
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("streaming handle is null");
            return ptr::null_mut();
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        let guard = handle.inner.lock_recover();
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("streaming client not started -- call thetadatadx_streaming_set_callback first, or has been shut down");
            return ptr::null_mut();
        };
        let subs = client.active_subscriptions();
        build_subscription_array(
            subs.into_iter()
                .map(|(kind, contract)| (kind.kind_str().to_string(), format!("{contract}"))),
        )
    })
}

/// Get a snapshot of currently active full-stream subscriptions.
///
/// Each entry's `contract` field carries the security-type discriminant
/// (`"Stock"` / `"Option"` / `"Index"`) the full-stream subscription is
/// bound to. The `kind` field is the snake_case full-stream kind label
/// (`"full_trades"` / `"full_open_interest"`), matching the unified
/// `thetadatadx_client_active_full_subscriptions` projection. Per-contract-only
/// kinds (`Quote` / `MarketValue`) have no full-stream form and are
/// omitted.
///
/// Returns a heap-allocated `ThetaDataDxSubscriptionArray` (null on error).
/// Caller must free the result with `thetadatadx_subscription_array_free`.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_active_full_subscriptions(
    handle: *const ThetaDataDxStreamHandle,
) -> *mut ThetaDataDxSubscriptionArray {
    ffi_boundary!(std::ptr::null_mut(), {
        if handle.is_null() {
            set_error("streaming handle is null");
            return ptr::null_mut();
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        let guard = handle.inner.lock_recover();
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error("streaming client not started -- call thetadatadx_streaming_set_callback first, or has been shut down");
            return ptr::null_mut();
        };
        let subs = client.active_full_subscriptions();
        build_subscription_array(subs.into_iter().filter_map(|(kind, sec_type)| {
            kind.full_kind_str()
                .map(|full| (full.to_string(), format!("{sec_type:?}")))
        }))
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  Standalone FPSS — polymorphic subscribe / unsubscribe
// ═══════════════════════════════════════════════════════════════════════

/// Polymorphic subscribe on the standalone FPSS client. Mirrors the
/// Rust `StreamingClient::subscribe(Subscription)` shape.
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_subscribe(
    handle: *const ThetaDataDxStreamHandle,
    request: *const ThetaDataDxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("streaming handle is null");
            return -1;
        }
        // SAFETY: `request` is a non-null `*const ThetaDataDxSubscriptionRequest` the caller pins for the call duration; `coerce_subscription` validates its discriminant + tagged-union fields, setting `thetadatadx_last_error` on malformed payloads.
        let sub = match unsafe { coerce_subscription(request) } {
            Some(s) => s,
            None => return -1,
        };
        // SAFETY: `handle` is a non-null `*const ThetaDataDxStreamHandle` returned by `thetadatadx_streaming_new` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let handle = unsafe { &*handle };
        let guard = handle.inner.lock_recover();
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error(
                "streaming client not started -- call thetadatadx_streaming_set_callback first, or has been shut down",
            );
            return -1;
        };
        match client.subscribe(sub) {
            Ok(()) => 0,
            Err(e) => {
                set_error_from(&e);
                -1
            }
        }
    })
}

/// Polymorphic unsubscribe on the standalone FPSS client.
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_unsubscribe(
    handle: *const ThetaDataDxStreamHandle,
    request: *const ThetaDataDxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("streaming handle is null");
            return -1;
        }
        // SAFETY: `request` is a non-null `*const ThetaDataDxSubscriptionRequest` the caller pins for the call duration; `coerce_subscription` validates its discriminant + tagged-union fields, setting `thetadatadx_last_error` on malformed payloads.
        let sub = match unsafe { coerce_subscription(request) } {
            Some(s) => s,
            None => return -1,
        };
        // SAFETY: `handle` is a non-null `*const ThetaDataDxStreamHandle` returned by `thetadatadx_streaming_new` and not yet freed; `&*` produces a shared reference valid for the call duration.
        let handle = unsafe { &*handle };
        let guard = handle.inner.lock_recover();
        let client = if let Some(c) = guard.as_ref() {
            c
        } else {
            set_error(
                "streaming client not started -- call thetadatadx_streaming_set_callback first, or has been shut down",
            );
            return -1;
        };
        match client.unsubscribe(sub) {
            Ok(()) => 0,
            Err(e) => {
                set_error_from(&e);
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
/// Reuses the credentials/config saved at `thetadatadx_streaming_connect` time and
/// the C callback registered via the most recent `thetadatadx_streaming_set_callback`.
/// Returns -1 if no callback was ever installed or if the handle has
/// been shut down (shutdown is terminal — see [`thetadatadx_streaming_shutdown`]).
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
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
/// until its exit path joins. Pair with `thetadatadx_streaming_await_drain` from
/// a separate thread when the application needs to free `ctx` or
/// otherwise rely on the old callback having stopped firing.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_reconnect(
    handle: *const ThetaDataDxStreamHandle,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() {
            set_error("streaming handle is null");
            return -1;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        // Serialise concurrent reconnects: `dispatcher` mutex prevents two
        // callers from each building a replacement client and racing on the
        // inner-slot publish.
        let mut dispatcher_guard = handle.dispatcher.lock_recover();
        if !reject_if_shutdown(handle) {
            return -1;
        }
        let params = &handle.connect_params;

        // Look up the previously-registered C callback. Reconnect cannot
        // make forward progress without one — `StreamingClient::connect`
        // requires an event handler at construction time.
        let cb = {
            let guard = handle.callback.lock_recover();
            match *guard {
                Some(cb) => cb,
                None => {
                    set_error(
                        "no callback registered -- call thetadatadx_streaming_set_callback \
                         before thetadatadx_streaming_reconnect",
                    );
                    return -1;
                }
            }
        };

        // 1. Save active subscriptions from the current client (if any).
        let (saved_subs, saved_full_subs) = {
            let guard = handle.inner.lock_recover();
            match guard.as_ref() {
                Some(c) => (c.active_subscriptions(), c.active_full_subscriptions()),
                None => (Vec::new(), Vec::new()),
            }
        };

        // 2. Shut down the old client. With the SSOT pipeline there is
        // no separate dispatcher to tear down — the Disruptor consumer
        // joins inside `StreamingClient::Drop` when the last `Arc` goes
        // away. Capture the drain flag BEFORE dropping `old` so a
        // subsequent `thetadatadx_streaming_await_drain` poll observes the previous
        // session's quiescence even though `Drop` runs asynchronously
        // when invoked from the consumer thread.
        // Take the previous `Arc<StreamingClient>` OUT of the inner lock so
        // a callback re-entering any `thetadatadx_streaming_*` API that needs
        // `handle.inner.lock()` never sees the lock held while the old
        // session tears down.
        let taken_old = handle.inner.lock_recover().take();
        // Extract the old dispatcher session and RELEASE the dispatcher lock
        // before the join: the old dispatcher keeps draining ring-buffered
        // events through the user callback until it observes the shutdown, and
        // such a callback may call a `thetadatadx_streaming_*` status reader that
        // takes this same lock. Joining under the lock would deadlock. The lock
        // is re-acquired below (step 3) to publish the replacement session; a
        // shutdown racing in the lock-free window is caught by the
        // `reject_if_shutdown` re-check there.
        let old_session = extract_dispatcher_session(&mut dispatcher_guard);
        drop(dispatcher_guard);
        let prev_drain_flag = if let Some(old) = taken_old {
            let flag = old.drained_flag();
            handle.prev_drained.lock_recover().push(flag.clone());
            old.shutdown();
            drop(old);
            // Join the OLD dispatcher (lock-free) BEFORE spawning the
            // replacement so the new dispatcher does not race the old one over
            // the same C callback context.
            join_extracted_session(handle, old_session);
            Some(flag)
        } else {
            // No old client, but a stale Running session could still exist
            // (e.g. a prior client already taken); join it lock-free too.
            join_extracted_session(handle, old_session);
            None
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
            if !await_flags(
                std::slice::from_ref(&flag),
                std::time::Duration::from_millis(5_000),
            ) {
                set_error(
                    "reconnect drain barrier timed out after 5s — \
                     previous callback is still in flight; refusing \
                     to bind the new session to the same ctx",
                );
                return -1;
            }
        }

        // 3. Build the new client + spawn a fresh dispatcher thread
        // bound to the same C callback.
        let connect_result = streaming_builder(params).build();

        let new_client = match connect_result {
            Ok(c) => Arc::new(c),
            Err(e) => {
                set_error_from(&thetadatadx::error::Error::from(e));
                return -1;
            }
        };

        // Re-acquire the dispatcher lock for the publish-and-install, now that
        // the lock-free old-session join is done. A `thetadatadx_streaming_shutdown`
        // / `_free` could have run in the lock-free window and flipped the
        // handle terminal; re-check and bail (shutting the freshly built client)
        // rather than resurrecting a shut-down handle with a new dispatcher.
        let mut dispatcher_guard = handle.dispatcher.lock_recover();
        if handle.state.load(AtomicOrdering::Relaxed) == STREAM_STATE_SHUTDOWN {
            drop(dispatcher_guard);
            new_client.shutdown();
            drop(new_client);
            set_error(
                "handle was shut down concurrently with reconnect -- \
                 the replacement session was discarded",
            );
            return -1;
        }
        // Hold `handle.inner` for the entire publish-and-spawn so a
        // racing `thetadatadx_streaming_subscribe` / `_unsubscribe` /
        // `_active_subscriptions` (the lock-free control surface that
        // only takes `inner.lock`) either serialises in front of the
        // publish (sees `None`) or behind both publish and spawn
        // (sees a fully wired session). `thetadatadx_streaming_shutdown` / `_free`
        // / `_set_callback` are serialised against this install by
        // `handle.dispatcher` (re-acquired just above and held through the
        // `*dispatcher_guard = Running` write below). The spawned dispatcher
        // iterates the FPSS client poller via its own internal mutex
        // and never touches `handle.inner`, so the held guard does
        // NOT deadlock the dispatcher.
        let spawn_result = {
            let mut guard = handle.inner.lock_recover();
            *guard = Some(std::sync::Arc::clone(&new_client));

            let dispatcher_client = std::sync::Arc::clone(&new_client);
            let dispatcher_slot = std::sync::Arc::clone(&handle.dispatcher);
            let spawn_result = std::thread::Builder::new()
                .name("thetadatadx-ffi-fpss-dispatcher".into())
                .spawn(move || {
                    // `StreamingClient::for_each` drives `poll_batch`, which wraps
                    // each callback invocation in its own `catch_unwind`.  A
                    // panic in the handler is caught, recorded via
                    // `panic_count()`, and does not stop event delivery for
                    // subsequent events.  The outer `catch_unwind` below
                    // guards only the event-iteration machinery itself.
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        dispatcher_client.for_each(|event| cb.invoke(event));
                    }));
                    if let Err(payload) = outcome {
                        let reason = downcast_ffi_panic_payload(payload);
                        tracing::error!(
                            target: "thetadatadx::ffi",
                            reason = %reason,
                            "thetadatadx-ffi-fpss-dispatcher panicked in event iteration machinery across reconnect; handle transitioning to failed state",
                        );
                        // Publish `Failed` from this thread before it exits so
                        // health checks reflect the dead loop immediately, not
                        // only once teardown joins.
                        publish_failed_if_current(
                            &dispatcher_slot,
                            std::thread::current().id(),
                            reason,
                        );
                    }
                });
            match spawn_result {
                Ok(h) => Ok(h),
                Err(e) => {
                    // Roll publication back inside the same locked
                    // section so no concurrent `thetadatadx_streaming_*` call ever
                    // observes the transient `Some(client)` state.
                    let taken = guard.take();
                    if let Some(client) = taken {
                        client.shutdown();
                        drop(client);
                    }
                    Err(e)
                }
            }
        };
        match spawn_result {
            Ok(h) => {
                // The callback dispatcher parks only on the event ring, which
                // the client shutdown signals on teardown, so no wake hook.
                *dispatcher_guard = FfpssDispatcherSession::Running {
                    handle: h,
                    on_teardown: None,
                    // The C ABI runs its own teardown and never reads this flag.
                    registers_drain_flag: true,
                };
            }
            Err(e) => {
                set_error(&format!("failed to spawn streaming dispatcher thread: {e}"));
                return -1;
            }
        }

        // 4. Re-subscribe all previous subscriptions through the core's
        // paced replay engine (best-effort; failures are non-fatal but
        // surfaced through tracing so ops can see silent
        // re-subscription failures across a reconnect boundary). The
        // engine paces submissions in bursts so a large saved set is
        // not fired at a recovering upstream back-to-back; per-item
        // diagnostics are emitted by the engine itself.
        if let Err(e) = new_client.restore_subscriptions(&saved_subs, &saved_full_subs) {
            tracing::warn!(
                target: "thetadatadx::ffi::reconnect",
                error = %e,
                "subscription replay reported failures after reconnect"
            );
        }

        // The new client was already published into `handle.inner`
        // before the dispatcher started; nothing left to commit.
        drop(new_client);

        0
    })
}

/// Phase 1 of teardown: move the dispatcher session OUT of the lock so the
/// join can run with no lock held. The caller holds the `dispatcher` guard
/// only for this `mem::replace`; it must then DROP the guard and hand the
/// returned session to [`join_extracted_session`].
///
/// Splitting extract from join is what keeps teardown deadlock-free: while the
/// client is shutting down it keeps draining already-ring-buffered events
/// through the user callback until it observes the shutdown, and such a
/// callback may call `thetadatadx_streaming_is_streaming` /
/// `_is_authenticated`, which take the `dispatcher` lock. Joining the
/// dispatcher thread while still holding that lock would block the re-entrant
/// status read, the dispatcher would never reach its exit, and the join would
/// hang. With the lock released before the join, those calls proceed and the
/// dispatcher reaches its shutdown exit.
fn extract_dispatcher_session(session: &mut FfpssDispatcherSession) -> FfpssDispatcherSession {
    std::mem::replace(session, FfpssDispatcherSession::Idle)
}

/// Spin-poll a set of quiescence flags until every one reads `true` or the
/// `timeout` elapses, sleeping 1 ms between polls. Returns `true` when all
/// flags drained, `false` on timeout. `checked_add` overflow on an extreme
/// `timeout` is treated as "effectively never" — the wait proceeds without
/// a deadline rather than panicking. Callers own any pre-snapshot, logging,
/// or post-drain cleanup around this barrier.
fn await_flags(flags: &[Arc<std::sync::atomic::AtomicBool>], timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now().checked_add(timeout);
    loop {
        if flags
            .iter()
            .all(|f| f.load(std::sync::atomic::Ordering::Acquire))
        {
            return true;
        }
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Phase 2 of teardown: join the extracted dispatcher thread with NO lock
/// held. `handle` is taken only to RE-ACQUIRE the `dispatcher` lock on a panic
/// join, to publish `Failed`. Defers to detach via the `prev_drained` chain
/// when called from inside the dispatcher itself (the consumer-thread
/// self-join hazard).
fn join_extracted_session(handle: &ThetaDataDxStreamHandle, session: FfpssDispatcherSession) {
    let FfpssDispatcherSession::Running {
        handle: thread_handle,
        on_teardown,
        ..
    } = session
    else {
        return;
    };
    // Wake a dispatcher parked on a teardown-specific primitive before joining.
    // The per-event callback dispatcher parks only on the event ring, which the
    // caller's `client.shutdown()` already signalled, so it installs no hook;
    // the call is a no-op there but keeps every teardown route converging on
    // the same wakeup contract the core client uses.
    if let Some(wake) = on_teardown {
        wake();
    }
    // Self-join hazard: a callback (or a dispatcher-thread drop) may reach this
    // path on the dispatcher thread itself. Detach in that case; the
    // `prev_drained` chain still provides quiescence visibility.
    if thread_handle.thread().id() == std::thread::current().id() {
        return;
    }
    if let Err(payload) = thread_handle.join() {
        let reason = downcast_ffi_panic_payload(payload);
        tracing::error!(
            target: "thetadatadx::ffi",
            reason = %reason,
            "thetadatadx-ffi-fpss-dispatcher panicked; handle marked as failed",
        );
        // Publish `Failed` by re-acquiring the lock, which is safe now that the
        // join has completed. Record it ONLY if the slot is still `Idle`: the
        // lock was released across the join, so a concurrent
        // `thetadatadx_streaming_set_callback` / `_reconnect` may have installed
        // a fresh `Running` session in that window. The panic belongs to the
        // now-superseded OLD session, so overwriting unconditionally would
        // clobber the new session's `JoinHandle` (orphaning its thread) and
        // falsely report a healthy live session as failed.
        let mut guard = handle.dispatcher.lock_recover();
        if matches!(*guard, FfpssDispatcherSession::Idle) {
            *guard = FfpssDispatcherSession::Failed { reason };
        }
    }
}

/// Tear down a live FPSS session: take the `StreamingClient` out of
/// `handle.inner` (a different lock than `dispatcher`), record its drain
/// flag in `prev_drained`, signal shutdown, drop it, then join the extracted
/// dispatcher thread lock-free. Ordering the take + shutdown ahead of the
/// join keeps a dispatcher re-entering `handle.inner` or `dispatcher` via the
/// user callback from observing either lock held, so the join cannot
/// deadlock. The caller must already hold no relevant lock and have flipped
/// the handle terminal under the `dispatcher` lock before extracting
/// `session`.
fn retire_session(handle: &ThetaDataDxStreamHandle, session: FfpssDispatcherSession) {
    // Bind the take to a `let` so the `inner` guard drops at the end of THIS
    // statement, before `shutdown()`/`drop()` — keeping a dispatcher that
    // re-enters `handle.inner` via the user callback from observing the lock
    // held. Holding it across the if-let block (scrutinee form) would extend
    // the guard over the teardown and break the lock-free re-entry invariant.
    let taken = handle.inner.lock_recover().take();
    if let Some(client) = taken {
        handle
            .prev_drained
            .lock_recover()
            .push(client.drained_flag());
        client.shutdown();
        drop(client);
    }
    join_extracted_session(handle, session);
}

/// Downcast a thread-panic payload to a human-readable string.
fn downcast_ffi_panic_payload(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_owned();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "dispatcher panicked with non-string payload".to_owned()
}

/// Publish `Failed` from the dispatcher thread's OWN catch-arm after an outer
/// panic in the event-iteration machinery, so `thetadatadx_streaming_is_streaming`
/// / `_is_authenticated` report the dead loop immediately rather than only
/// after teardown joins the corpse.
///
/// `dispatcher_thread_id` is the id of the thread the session's `JoinHandle`
/// names; the caller passes its own [`std::thread::current`] id. The store
/// happens ONLY when the slot still holds the matching `Running` session: the
/// lock is dropped between spawning the dispatcher and a later
/// `set_callback` / `reconnect`, so a fresh session (different thread id) or a
/// teardown-extracted `Idle` may already occupy the slot. Overwriting either
/// would clobber a live session's `JoinHandle` or resurrect a torn-down one.
///
/// Orthogonal to teardown: this is a mutate-UNDER-lock-then-RELEASE with no
/// join and no drain wait held across the guard, exactly like the publish in
/// [`join_extracted_session`]. The lock order is unchanged (this takes only
/// `dispatcher`, never `inner`). When this wins the race against a concurrent
/// teardown, the teardown's `extract` then yields the `Failed` variant — whose
/// `let-else` skips the join — so the already-finished panicked thread is
/// detached rather than reaped; the `_free` drain barrier still waits on the
/// client's own drained flag, so quiescence is unaffected.
fn publish_failed_if_current(
    dispatcher: &Mutex<FfpssDispatcherSession>,
    dispatcher_thread_id: std::thread::ThreadId,
    reason: String,
) {
    let mut guard = dispatcher.lock_recover();
    if let FfpssDispatcherSession::Running { handle, .. } = &*guard {
        if handle.thread().id() == dispatcher_thread_id {
            *guard = FfpssDispatcherSession::Failed { reason };
        }
    }
}

/// Milliseconds since the most recent inbound streaming frame of any
/// kind on this FPSS handle. Same contract as
/// `thetadatadx_client_millis_since_last_event`: returns `0` on success with
/// the value in `*out_ms`, `1` when no session is live or no frame
/// has been received yet, `-1` on a null pointer.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_millis_since_last_event(
    handle: *const ThetaDataDxStreamHandle,
    out_ms: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() || out_ms.is_null() {
            set_error("handle or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        let value = {
            let guard = handle.inner.lock_recover();
            guard.as_ref().and_then(|c| c.millis_since_last_event())
        };
        match value {
            Some(ms) => {
                // SAFETY: out_ms checked non-null above; the FFI contract pins the storage for the call duration.
                unsafe {
                    *out_ms = ms;
                }
                0
            }
            None => {
                // SAFETY: out_ms checked non-null above; the FFI contract pins the storage for the call duration.
                unsafe {
                    *out_ms = 0;
                }
                1
            }
        }
    })
}

/// UNIX-nanosecond receive timestamp of the most recent inbound
/// streaming frame of any kind on this FPSS handle. Returns `0` when
/// the handle is null, no session is live, or no frame has been
/// received yet.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_last_event_received_at_unix_nanos(
    handle: *const ThetaDataDxStreamHandle,
) -> i64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        let guard = handle.inner.lock_recover();
        guard
            .as_ref()
            .map_or(0, |c| c.last_event_received_at_unix_nanos())
    })
}

/// Address (`host:port`) of the streaming server the current FPSS
/// session is connected to, following the session across
/// auto-reconnects. Returns a heap-owned C string the caller must
/// release with `thetadatadx_string_free`, or null when no session is live.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_last_connected_addr(
    handle: *const ThetaDataDxStreamHandle,
) -> *mut std::os::raw::c_char {
    ffi_boundary!(ptr::null_mut(), {
        if handle.is_null() {
            set_error("streaming handle is null");
            return ptr::null_mut();
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        let addr = {
            let guard = handle.inner.lock_recover();
            guard.as_ref().map(|c| c.last_connected_addr())
        };
        match addr {
            Some(addr) => match std::ffi::CString::new(addr) {
                Ok(c) => c.into_raw(),
                Err(e) => {
                    set_error(&format!("connected address contains an interior NUL: {e}"));
                    ptr::null_mut()
                }
            },
            None => ptr::null_mut(),
        }
    })
}

/// Cumulative count of FPSS events the TLS reader could not publish
/// into the Disruptor ring because the consumer fell behind and the
/// ring was full (`Producer::try_publish` returned `RingBufferFull`).
///
/// Returns 0 if the handle is null or no callback has been installed
/// yet.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_dropped_events(
    handle: *const ThetaDataDxStreamHandle,
) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        let guard = handle.inner.lock_recover();
        guard.as_ref().map_or(0, |c| c.dropped_count())
    })
}

/// Point-in-time count of FPSS events published into the event ring
/// but not yet drained into the registered callback — the in-flight
/// depth between the I/O thread and the dispatcher.
///
/// The leading back-pressure signal: `thetadatadx_streaming_dropped_events` only
/// moves AFTER data has been lost, while a rising occupancy that
/// approaches `thetadatadx_streaming_ring_capacity` predicts those drops while
/// there is still time to react. Sampling never blocks the feed; safe
/// to poll from any thread at any cadence.
///
/// Returns 0 if the handle is null or has been shut down.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_ring_occupancy(
    handle: *const ThetaDataDxStreamHandle,
) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        let guard = handle.inner.lock_recover();
        guard.as_ref().map_or(0, |c| c.ring_occupancy() as u64)
    })
}

/// Configured capacity of the FPSS event ring in slots (the
/// `streaming_ring_size` setting, a power of two), the fixed denominator
/// for `thetadatadx_streaming_ring_occupancy`. When the occupancy sample
/// approaches this value the ring is saturating and further events
/// will be dropped (counted by `thetadatadx_streaming_dropped_events`).
///
/// Returns 0 if the handle is null or has been shut down.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_ring_capacity(
    handle: *const ThetaDataDxStreamHandle,
) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        let guard = handle.inner.lock_recover();
        guard.as_ref().map_or(0, |c| c.ring_capacity() as u64)
    })
}

/// Cumulative count of user-callback panics caught by the per-invocation
/// `catch_unwind` boundary on this FPSS handle since the current stream
/// started.
///
/// Each caught panic is also surfaced via `tracing::error!` with target
/// `thetadatadx::fpss::poller`. A panic in the callback is caught,
/// recorded here, and does not stop event delivery — the next event
/// continues normally. Safe to call from any thread without blocking.
///
/// Returns 0 if the handle is null or no callback has been installed yet.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_panic_count(
    handle: *const ThetaDataDxStreamHandle,
) -> u64 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        let guard = handle.inner.lock_recover();
        guard.as_ref().map_or(0, |c| c.panic_count())
    })
}

/// Shut down the FPSS client, stopping all background threads.
///
/// # Lifecycle contract (terminal)
///
/// Shutdown is terminal: every subsequent `thetadatadx_streaming_set_callback` /
/// `_reconnect` / `_shutdown` call on this handle returns -1 with the
/// error message
/// `"streaming handle has already been shut down -- this is terminal"`. The
/// handle remains valid for `thetadatadx_streaming_free()` only.
///
/// Idempotency: calling shutdown twice on the same handle is rejected
/// rather than silently no-op'd, so a misuse caller cannot accidentally
/// observe "success" after the resource is gone.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_shutdown(handle: *const ThetaDataDxStreamHandle) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        // Serialise with `thetadatadx_streaming_set_callback` / `thetadatadx_streaming_reconnect`
        // so an in-flight install cannot publish a fresh client AFTER
        // we have flipped the state to `SHUTDOWN`. Without this lock
        // the terminal-shutdown contract is violated: a concurrent
        // reconnect could resurrect the handle with a new dispatcher
        // and keep firing the C callback on a handle that
        // `thetadatadx_last_error()` already reports as shut down.
        let mut dispatcher_guard = handle.dispatcher.lock_recover();
        if !reject_if_shutdown(handle) {
            // Double-shutdown -- error already set, nothing to drop.
            return;
        }
        // Flip to terminal SHUTDOWN while still holding the dispatcher lock, and
        // extract the session, BEFORE releasing the lock for the join. A racing
        // `thetadatadx_streaming_set_callback` / `_reconnect` serialises on this
        // same lock: it can only proceed after we release, by which point the
        // state is SHUTDOWN and the session is extracted, so its
        // `reject_if_shutdown` bails and it cannot resurrect the handle with a
        // fresh dispatcher. (The join itself must run lock-free, below, so a
        // callback re-entering a status read does not deadlock the join.)
        handle
            .state
            .store(STREAM_STATE_SHUTDOWN, AtomicOrdering::Relaxed);
        let session = extract_dispatcher_session(&mut dispatcher_guard);
        drop(dispatcher_guard);
        // Take the client out of `handle.inner`, push its drain flag, signal
        // shutdown, drop, then join lock-free — see `retire_session`. The join
        // runs AFTER the producer-drop signal has propagated through the ring
        // shutdown to the iterator, so the `for ... in &client` loop returns
        // `Ok(None)` and the thread exits cleanly while any re-entrant status
        // read proceeds lock-free.
        retire_session(handle, session);
    })
}

/// Wait for every superseded FPSS session to quiesce.
///
/// Returns `1` once **all** prior `thetadatadx_streaming_reconnect` /
/// `thetadatadx_streaming_shutdown` sessions' Disruptor consumers have finished
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
/// After `thetadatadx_streaming_reconnect` or `thetadatadx_streaming_shutdown` returns, before
/// freeing `ctx` or otherwise relying on the old callback having
/// stopped firing.
///
/// # Lifecycle restriction
///
/// MUST be called from a thread other than the FPSS Disruptor
/// consumer thread. Calling it from inside the user callback would
/// block the helper the consumer is waiting on and always time out.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_await_drain(
    handle: *const ThetaDataDxStreamHandle,
    timeout_ms: u64,
) -> i32 {
    ffi_boundary!(0, {
        if handle.is_null() {
            return 0;
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        // Snapshot the pending generations once and walk them on each
        // poll. New stops landing during the wait join the next call's
        // working set — `await_drain` semantics are "wait for what was
        // outstanding when I started", which mirrors the in-process
        // `Client::await_drain` contract.
        let initial = {
            let guard = handle.prev_drained.lock_recover();
            if guard.is_empty() {
                return 0;
            }
            guard.clone()
        };
        if !await_flags(&initial, std::time::Duration::from_millis(timeout_ms)) {
            return 0;
        }
        // Lazy GC of the shared Vec so a long-lived handle that cycles through
        // many sessions does not accumulate entries. Take the lock briefly;
        // this is a clean-up path, never on the hot tick path.
        let mut guard = handle.prev_drained.lock_recover();
        guard.retain(|f| !f.load(std::sync::atomic::Ordering::Acquire));
        1
    })
}

/// Free a FPSS handle.
///
/// # Lifecycle contract
///
/// `thetadatadx_streaming_free` accepts the handle in either state:
///
/// - **Already shut down**: the prior `thetadatadx_streaming_shutdown` (or
///   `thetadatadx_streaming_reconnect`) populated the drain flag; `_free` polls that
///   flag with a 5-second internal timeout so it returns only after the
///   superseded session's Disruptor consumer has finished firing the
///   registered callback.
/// - **Not yet shut down**: `_free` performs the equivalent of
///   `thetadatadx_streaming_shutdown` first (drops the FPSS client, captures the
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
/// Calling `thetadatadx_streaming_await_drain` from another thread before invoking
/// `thetadatadx_streaming_free` is no longer required for callback-context lifetime
/// safety — `_free` now serves as the public drain barrier as well.
///
/// # Lifecycle restriction
///
/// Do NOT call `thetadatadx_streaming_free` from inside the user callback. The
/// callback runs on the dispatcher thread; `_free` first acquires
/// `handle.dispatcher` and then waits for the dispatcher's drain flag.
/// Issuing `_free` from inside the callback means the dispatcher is
/// still inside user code while `_free` waits for it to exit. The
/// 5-second drain budget elapses, `_free` logs the overrun and
/// proceeds to `Box::from_raw(handle)`; control then returns into
/// the user callback which is now operating against freed memory.
/// Drive `_free` from a separate thread; if the callback wants to
/// signal teardown, post to an external channel and have the
/// non-callback thread invoke `_free` (or `_shutdown` followed by
/// `_await_drain` then `_free`).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_streaming_free(handle: *mut ThetaDataDxStreamHandle) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }

        // Shut down first if the handle is still live, mirroring
        // `thetadatadx_streaming_shutdown` so callers who skip the explicit shutdown
        // call still get a quiesced consumer thread by the time `_free`
        // returns. Detect "already shut down" via the lifecycle state
        // so we never attempt a double shutdown.
        {
            // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
            let h = unsafe { &*handle };
            // Acquire `dispatcher` so an in-flight
            // `thetadatadx_streaming_set_callback` / `thetadatadx_streaming_reconnect` cannot be
            // mid-publish when we destroy the handle. `STREAM_STATE_SHUTDOWN`
            // is flipped under this lock (below) before the lock is released
            // for the lock-free join, so a concurrent install that later
            // acquires the lock observes SHUTDOWN and bails out before touching
            // freed memory. The drain wait further down runs lock-free.
            let mut disp_guard = h.dispatcher.lock_recover();
            if h.state.load(AtomicOrdering::Relaxed) != STREAM_STATE_SHUTDOWN {
                // Flip terminal + extract the session under the lock, then
                // RELEASE the lock before the join: a dispatcher draining
                // ring-buffered events through the user callback may re-enter a
                // `thetadatadx_streaming_*` status reader that takes this same lock,
                // so joining under it would deadlock. A concurrent install that
                // acquires the lock after we release observes SHUTDOWN and bails.
                h.state
                    .store(STREAM_STATE_SHUTDOWN, AtomicOrdering::Relaxed);
                let session = extract_dispatcher_session(&mut disp_guard);
                drop(disp_guard);
                retire_session(h, session);
            } else {
                // Already terminal: nothing to retire, but the drain wait
                // below must run lock-free on this path too, so release the
                // dispatcher lock here rather than holding it to scope end.
                drop(disp_guard);
            }

            // Wait for every superseded session's consumer thread to
            // finish firing the registered callback. The Vec is empty
            // when no callback was ever installed (FRESH -> SHUTDOWN
            // direct transition), so suppress the timeout log on that
            // path.
            const FREE_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
            let pending: Vec<Arc<std::sync::atomic::AtomicBool>> = {
                let guard = h.prev_drained.lock_recover();
                guard.clone()
            };
            if !pending.is_empty() {
                let drained = await_flags(&pending, FREE_DRAIN_TIMEOUT);
                if !drained {
                    tracing::error!(
                        target: "thetadatadx::ffi",
                        timeout_ms = FREE_DRAIN_TIMEOUT.as_millis() as u64,
                        pending_generations = pending.len(),
                        "thetadatadx_streaming_free: drain barrier exceeded timeout -- callback may still \
                         be firing on the consumer thread; user ctx must remain valid past return",
                    );
                }
            }
        }

        // Now safe to destroy the handle.
        // SAFETY: the pointer was returned by Box::into_raw / thetadatadx_*_new and has not been freed; ownership returns to Rust.
        drop(unsafe { Box::from_raw(handle) });
    })
}

#[cfg(test)]
mod panic_isolation_tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use thetadatadx::fpss::{HarnessPublishMode, StreamingClient};

    fn wait_for_drain(drained: &std::sync::atomic::AtomicBool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !drained.load(Ordering::Acquire) {
            if std::time::Instant::now() > deadline {
                panic!("StreamingClient did not drain within 5 seconds");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn wait_for_deliveries(delivered: &AtomicU64, expected: u64) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while delivered.load(Ordering::Relaxed) < expected {
            if std::time::Instant::now() > deadline {
                panic!(
                    "consumer did not deliver {expected} events within 5 s; \
                     got {}",
                    delivered.load(Ordering::Relaxed)
                );
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The FFI dispatch path uses `StreamingClient::for_each` (via `poll_batch`)
    /// as its event vehicle.  `poll_batch` wraps every callback invocation
    /// in `catch_unwind` and increments `StreamingClient::panic_count()` on
    /// each caught panic.  This test exercises the panic counter directly
    /// on the same `StreamingClient` type the FFI layer wraps, confirming the
    /// shared counter is incremented and event delivery continues.
    ///
    /// Contract: `client.panic_count() == 1` AND `delivered == N_EVENTS - 1`.
    #[test]
    fn fpss_client_panic_count_incremented_by_catch_unwind() {
        const N_EVENTS: usize = 10;

        let delivered = Arc::new(AtomicU64::new(0));
        let delivered_c = Arc::clone(&delivered);
        let mut call_index: u64 = 0;

        let client = StreamingClient::for_self_join_test(
            N_EVENTS,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            move |_event| {
                let idx = call_index;
                call_index += 1;
                if idx == 0 {
                    panic!("intentional test panic on event 0");
                }
                delivered_c.fetch_add(1, Ordering::Relaxed);
            },
        );

        // Wait until the consumer has processed all N_EVENTS - 1 deliveries.
        // Event 0 panicked, so the delivery counter saturates at N_EVENTS - 1.
        // At that point the consumer has processed every event and
        // `panic_count()` is stable on the still-live client.
        wait_for_deliveries(&delivered, (N_EVENTS - 1) as u64);

        // Read `panic_count()` on the live client before triggering Drop.
        let observed_panics = client.panic_count();
        let delivered_count = delivered.load(Ordering::Relaxed);

        let drained = client.drained_flag();
        client.shutdown();
        drop(client);
        wait_for_drain(&drained);

        assert_eq!(
            observed_panics, 1,
            "StreamingClient::panic_count() must equal 1 after one caught panic; \
             got {observed_panics}"
        );
        assert_eq!(
            delivered_count,
            (N_EVENTS - 1) as u64,
            "event delivery must continue after the caught panic; \
             got {delivered_count}"
        );
    }
}

#[cfg(test)]
mod null_callback_guard_tests {
    use std::ffi::c_void;

    use super::{ThetaDataDxStreamCallback, ThetaDataDxStreamEvent};

    extern "C" fn noop(_event: *const ThetaDataDxStreamEvent, _ctx: *mut c_void) {}

    #[test]
    fn null_callback_is_the_none_niche_the_guard_rejects() {
        // A C caller passing a null function pointer arrives as the `None`
        // niche of `Option<ThetaDataDxStreamCallback>`; both set_callback
        // entries reject that before constructing an `FfiCallback`. A real
        // pointer is `Some` and proceeds. This pins the representation the
        // guards depend on so the parameter type cannot silently revert to
        // the non-nullable `extern "C" fn`.
        let null_cb: Option<ThetaDataDxStreamCallback> = None;
        assert!(null_cb.is_none());
        let real_cb: Option<ThetaDataDxStreamCallback> = Some(noop);
        assert!(real_cb.is_some());
    }

    /// Read the thread-local last-error slot set by the C ABI entry points
    /// as an owned `String`, so an assertion does not hold a borrow across
    /// the next FFI call. Returns `None` when the slot is empty.
    fn last_error() -> Option<String> {
        let ptr = crate::error::thetadatadx_last_error();
        if ptr.is_null() {
            return None;
        }
        // SAFETY: `thetadatadx_last_error` returns a pointer into the
        // thread-local `CString` valid until the next FFI call on this
        // thread; we copy it before any further call.
        Some(
            unsafe { std::ffi::CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    #[test]
    fn unified_set_callback_rejects_null_handle() {
        crate::error::thetadatadx_clear_error();
        // SAFETY: a null handle is exactly the input the entry point must
        // reject before any dereference.
        let rc = unsafe {
            super::thetadatadx_client_set_callback(
                std::ptr::null(),
                Some(noop),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, -1, "null handle must return -1");
        assert_eq!(last_error().as_deref(), Some("unified handle is null"));
    }

    #[test]
    fn unified_set_callback_rejects_null_callback_before_install() {
        crate::error::thetadatadx_clear_error();
        // A non-null but bogus handle pointer is never dereferenced because
        // the null-callback niche is rejected first, mirroring the FPSS
        // entry point's ordering. Use a dangling-but-non-null address so the
        // null-handle branch is skipped and the null-callback branch is the
        // one under test.
        let bogus = std::ptr::NonNull::<super::ThetaDataDxClient>::dangling()
            .as_ptr()
            .cast_const();
        // SAFETY: the entry point rejects the null callback before touching
        // `handle`, so `bogus` is never dereferenced.
        let rc =
            unsafe { super::thetadatadx_client_set_callback(bogus, None, std::ptr::null_mut()) };
        assert_eq!(rc, -1, "null callback must return -1");
        assert_eq!(
            last_error().as_deref(),
            Some("callback function pointer is null"),
        );
    }

    #[test]
    fn unified_already_streaming_contract_string_is_stable() {
        // Pin the exact wording the live-handle gate emits. A second
        // `thetadatadx_client_set_callback` while the slot is `Live` returns
        // -1 with this message (documented in the function's "REPLACEMENT
        // after stop" contract and in the C header); the unified path mirrors
        // the FPSS one-shot rule for the active window while still permitting
        // replacement after stop. The string is asserted here so the
        // documented C ABI contract cannot drift from the implementation
        // without this test failing.
        crate::error::thetadatadx_clear_error();
        crate::error::set_error("streaming already started");
        assert_eq!(last_error().as_deref(), Some("streaming already started"));
    }
}

#[cfg(test)]
mod discriminant_conversion_tests {
    use thetadatadx::fpss::{StreamControl, StreamEvent};
    use thetadatadx::{RemoveReason, StreamResponseType};

    use super::{fpss_event_to_ffi, ThetaDataDxStreamEventKind};

    // The wire `reason` / `result` fields are `i32` on the C ABI, while the
    // backing core enums are `#[repr(i16)]` (`RemoveReason`) and
    // `#[repr(u8)]` (`StreamResponseType`). Both reprs are strictly narrower
    // than `i32`, so the discriminant widens losslessly and the conversion
    // is total: every variant maps to its own discriminant with no panic and
    // no silent wrap. The converter expresses this with the infallible
    // `i32::from(.. as repr)` rather than a truncating `as i32` cast, so the
    // moment a future repr no longer fits `i32` the conversion stops
    // compiling instead of wrapping a defined-but-wrong value onto the wire.

    #[test]
    fn disconnected_reason_discriminant_survives_unchanged() {
        // A representative in-range reason and the negative sentinel variant
        // both round-trip to their exact discriminant value.
        for reason in [RemoveReason::TooManyRequests, RemoveReason::Unspecified] {
            let event = StreamEvent::Control(StreamControl::Disconnected { reason });
            let buffered = fpss_event_to_ffi(&event);
            assert!(matches!(
                buffered.event.kind,
                ThetaDataDxStreamEventKind::Disconnected
            ));
            assert_eq!(
                buffered.event.disconnected.reason,
                i32::from(reason as i16),
                "in-range disconnect reason must convert to its discriminant unchanged"
            );
        }
    }

    #[test]
    fn req_response_result_discriminant_survives_unchanged() {
        let event = StreamEvent::Control(StreamControl::ReqResponse {
            req_id: 7,
            result: StreamResponseType::InvalidPerms,
        });
        let buffered = fpss_event_to_ffi(&event);
        assert!(matches!(
            buffered.event.kind,
            ThetaDataDxStreamEventKind::ReqResponse
        ));
        assert_eq!(buffered.event.req_response.req_id, 7);
        assert_eq!(
            buffered.event.req_response.result,
            i32::from(StreamResponseType::InvalidPerms as u8),
            "in-range subscription result must convert to its discriminant unchanged"
        );
    }

    #[test]
    fn every_remove_reason_discriminant_converts_totally() {
        // Exhaustive over every declared reason: the conversion never panics
        // and never wraps — each fits `i32` and round-trips exactly. This
        // pins totality so the converter stays panic-free for any reason the
        // server can emit.
        let reasons = [
            RemoveReason::Unspecified,
            RemoveReason::InvalidCredentials,
            RemoveReason::InvalidLoginValues,
            RemoveReason::InvalidLoginSize,
            RemoveReason::GeneralValidationError,
            RemoveReason::TimedOut,
            RemoveReason::ClientForcedDisconnect,
            RemoveReason::AccountAlreadyConnected,
            RemoveReason::SessionTokenExpired,
            RemoveReason::InvalidSessionToken,
            RemoveReason::FreeAccount,
            RemoveReason::TooManyRequests,
            RemoveReason::NoStartDate,
            RemoveReason::LoginTimedOut,
            RemoveReason::ServerRestarting,
            RemoveReason::SessionTokenNotFound,
            RemoveReason::ServerUserDoesNotExist,
            RemoveReason::InvalidCredentialsNullUser,
        ];
        for reason in reasons {
            let event = StreamEvent::Control(StreamControl::Disconnected { reason });
            let buffered = fpss_event_to_ffi(&event);
            assert_eq!(buffered.event.disconnected.reason, i32::from(reason as i16));
        }
    }
}

#[cfg(test)]
mod teardown_deadlock_tests {
    //! Deterministic watchdog for the standalone-handle teardown deadlock.
    //!
    //! The bug: `thetadatadx_streaming_shutdown` / `_free` / `_reconnect` joined
    //! the dispatcher thread WHILE holding the `dispatcher` lock. A user
    //! callback draining ring-buffered events during shutdown can re-enter a
    //! status reader (`thetadatadx_streaming_is_streaming` / `_is_authenticated`)
    //! that takes the same lock, so the join blocks forever on a thread that is
    //! itself blocked on the held lock.
    //!
    //! The fix splits teardown into `extract_dispatcher_session` (under the
    //! lock) + `join_extracted_session` (lock-free). This test reproduces the
    //! exact re-entrancy: a stand-in dispatcher thread blocks on
    //! `handle.dispatcher.lock()` (the callback's status read) and only then
    //! exits. Run the teardown sequence the production paths use; it must
    //! complete within a watchdog budget. Against the OLD join-under-lock code
    //! the stand-in could never acquire the lock, the join would hang, and the
    //! watchdog would fire.

    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::time::Duration;

    use super::{
        extract_dispatcher_session, join_extracted_session, FfiCallback, FfpssDispatcherSession,
        LockRecover, StreamingConnectParams, ThetaDataDxStreamCallback, ThetaDataDxStreamEvent,
        ThetaDataDxStreamHandle, STREAM_STATE_ACTIVE,
    };
    use std::ffi::c_void;

    extern "C" fn noop(_event: *const ThetaDataDxStreamEvent, _ctx: *mut c_void) {}

    /// Build a minimal handle whose `dispatcher` slot holds the given session.
    /// The connection params / callback are never exercised by the teardown
    /// helpers; only `handle.dispatcher` matters here.
    fn handle_with(session: FfpssDispatcherSession) -> ThetaDataDxStreamHandle {
        ThetaDataDxStreamHandle {
            inner: Arc::new(Mutex::new(None)),
            connect_params: StreamingConnectParams {
                creds: thetadatadx::Credentials::api_key("test"),
                streaming: thetadatadx::config::StreamingConfig::production_defaults(),
                reconnect: thetadatadx::config::ReconnectConfig::production_defaults(),
            },
            callback: Mutex::new(Some(FfiCallback {
                callback: noop as ThetaDataDxStreamCallback,
                ctx: std::ptr::null_mut(),
            })),
            state: AtomicU8::new(STREAM_STATE_ACTIVE),
            prev_drained: Mutex::new(Vec::new()),
            dispatcher: Arc::new(Mutex::new(session)),
        }
    }

    #[test]
    fn teardown_does_not_deadlock_when_callback_re_enters_the_dispatcher_lock() {
        let handle = Arc::new(handle_with(FfpssDispatcherSession::Idle));

        // `released` flips true only AFTER the teardown worker drops the
        // dispatcher guard. The stand-in (the re-entrant callback) records
        // whether it acquired the lock BEFORE that release; a correct lock-free
        // join lets it acquire only after the release.
        let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let acquired_before_release = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // The worker holds the guard, then opens this gate so the stand-in
        // attempts the lock while the worker still holds it — the deadlock
        // condition, made deterministic rather than timing-dependent.
        let attempt_gate = Arc::new(Barrier::new(2));

        let dispatcher_handle = {
            let handle = Arc::clone(&handle);
            let released = Arc::clone(&released);
            let acquired_before_release = Arc::clone(&acquired_before_release);
            let attempt_gate = Arc::clone(&attempt_gate);
            std::thread::Builder::new()
                .name("test-reentrant-dispatcher".into())
                .spawn(move || {
                    // Wait until the worker holds the guard and signals us.
                    attempt_gate.wait();
                    // Re-enter the dispatcher lock, as a status reader called
                    // from the user callback would. Blocks until the worker
                    // releases the guard.
                    let guard = handle.dispatcher.lock_recover();
                    // With the fix the worker drops the guard before joining, so
                    // we get in only after `released` is set. With the old
                    // join-under-lock code the worker never releases before the
                    // join, so this acquisition (and thus the thread, and thus
                    // the join) blocks forever — the watchdog fires.
                    if !released.load(Ordering::Acquire) {
                        acquired_before_release.store(true, Ordering::Release);
                    }
                    let _ = matches!(*guard, FfpssDispatcherSession::Idle);
                    drop(guard);
                })
                .expect("spawn stand-in dispatcher")
        };

        // Install the running session carrying the stand-in thread's handle.
        {
            let mut guard = handle.dispatcher.lock_recover();
            *guard = FfpssDispatcherSession::Running {
                handle: dispatcher_handle,
                on_teardown: None,
                registers_drain_flag: true,
            };
        }

        // Run the production teardown discipline on a worker, with a watchdog.
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let teardown = {
            let handle = Arc::clone(&handle);
            let done = Arc::clone(&done);
            let released = Arc::clone(&released);
            let attempt_gate = Arc::clone(&attempt_gate);
            std::thread::spawn(move || {
                // Phase 1: acquire the guard and extract the session.
                let mut guard = handle.dispatcher.lock_recover();
                let session = extract_dispatcher_session(&mut guard);
                // While STILL holding the guard, release the stand-in to attempt
                // the lock. It now blocks on a lock we hold — the precise
                // deadlock condition.
                attempt_gate.wait();
                std::thread::sleep(Duration::from_millis(50));
                // Phase 2: drop the guard, THEN join lock-free. The OLD code
                // joined here while still holding `guard`.
                released.store(true, Ordering::Release);
                drop(guard);
                join_extracted_session(&handle, session);
                done.store(true, Ordering::Release);
            })
        };

        // Watchdog: teardown must finish well within this budget. A hang means
        // the deadlock is back.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "teardown deadlocked: the dispatcher join did not complete with a \
                 callback blocked on the dispatcher lock (join-under-lock regression)",
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        teardown.join().expect("teardown thread");
        // The re-entrant reader got in only AFTER the guard was released —
        // proving the join did not hold the lock.
        assert!(
            !acquired_before_release.load(Ordering::Acquire),
            "the re-entrant status read acquired the dispatcher lock before teardown \
             released it — the join was not lock-free",
        );
    }
}

#[cfg(test)]
mod health_on_outer_panic_tests {
    //! An OUTER dispatcher panic (the event-iteration machinery, not a user
    //! callback) must flip `thetadatadx_streaming_is_streaming` /
    //! `_is_authenticated` to `0` IMMEDIATELY — from the dispatcher thread's
    //! own catch-arm — rather than staying healthy until teardown joins the
    //! dead thread. This pins [`super::publish_failed_if_current`], the catch-arm
    //! publish both `set_callback` and `reconnect` spawns route through.

    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::{Arc, Mutex};

    use super::{
        publish_failed_if_current, FfiCallback, FfpssDispatcherSession, LockRecover,
        StreamingConnectParams, ThetaDataDxStreamCallback, ThetaDataDxStreamEvent,
        ThetaDataDxStreamHandle, STREAM_STATE_ACTIVE,
    };
    use std::ffi::c_void;
    use thetadatadx::fpss::{HarnessPublishMode, StreamingClient};

    extern "C" fn noop(_event: *const ThetaDataDxStreamEvent, _ctx: *mut c_void) {}

    /// Build a handle whose `inner` holds a live harness `StreamingClient`
    /// (`for_self_join_test` flips its `authenticated` flag `true`), so absent
    /// a `Failed` dispatcher both status readers report healthy. The returned
    /// `(handle, drained)` lets the caller shut the harness client down so its
    /// consumer thread cannot outlive the test.
    fn handle_with_live_client(
        session: FfpssDispatcherSession,
    ) -> (ThetaDataDxStreamHandle, Arc<AtomicBool>) {
        // One idle harness event through a noop handler; the consumer parks on
        // the ring afterwards and exits on the shutdown the test signals.
        let client = StreamingClient::for_self_join_test(
            1,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            |_event| {},
        );
        let drained = client.drained_flag();
        let handle = ThetaDataDxStreamHandle {
            inner: Arc::new(Mutex::new(Some(client))),
            connect_params: StreamingConnectParams {
                creds: thetadatadx::Credentials::api_key("test"),
                streaming: thetadatadx::config::StreamingConfig::production_defaults(),
                reconnect: thetadatadx::config::ReconnectConfig::production_defaults(),
            },
            callback: Mutex::new(Some(FfiCallback {
                callback: noop as ThetaDataDxStreamCallback,
                ctx: std::ptr::null_mut(),
            })),
            state: AtomicU8::new(STREAM_STATE_ACTIVE),
            prev_drained: Mutex::new(Vec::new()),
            dispatcher: Arc::new(Mutex::new(session)),
        };
        (handle, drained)
    }

    #[test]
    fn outer_panic_flips_health_checks_to_failed_immediately() {
        // A stand-in for the dispatcher thread whose `JoinHandle` the
        // `Running` session carries. It parks until released, so the handle
        // stays joinable while the test drives the catch-arm publish — the
        // production catch-arm runs ON this thread, so the test passes this
        // thread's id to `publish_failed_if_current`.
        let release = Arc::new(AtomicBool::new(false));
        let dispatcher_handle = {
            let release = Arc::clone(&release);
            std::thread::Builder::new()
                .name("test-parked-dispatcher".into())
                .spawn(move || {
                    while !release.load(Ordering::Acquire) {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                })
                .expect("spawn parked dispatcher")
        };
        let dispatcher_thread_id = dispatcher_handle.thread().id();

        let (handle, drained) = handle_with_live_client(FfpssDispatcherSession::Running {
            handle: dispatcher_handle,
            on_teardown: None,
            registers_drain_flag: true,
        });

        // The two C-ABI status readers, each wrapped once.
        // SAFETY: `&handle` points at a live `ThetaDataDxStreamHandle` pinned on
        // this stack for the whole test, never freed, so the is_streaming entry
        // point's non-null / not-yet-freed precondition holds on every call.
        let is_streaming = || unsafe { super::thetadatadx_streaming_is_streaming(&handle) };
        // SAFETY: same stack-pinned, never-freed `&handle` as the reader above;
        // the is_authenticated entry point shares the identical precondition.
        let is_authenticated = || unsafe { super::thetadatadx_streaming_is_authenticated(&handle) };

        // Healthy before the panic: live authenticated client, no `Failed`.
        assert_eq!(
            is_streaming(),
            1,
            "a live session with a Running dispatcher must report streaming",
        );
        assert_eq!(
            is_authenticated(),
            1,
            "a live authenticated session must report authenticated before any panic",
        );

        // A non-matching thread id must NOT publish (models a fresh session
        // installed by a concurrent reconnect in the lock-release window).
        publish_failed_if_current(
            &handle.dispatcher,
            std::thread::current().id(),
            "wrong-thread panic must not clobber".to_owned(),
        );
        assert_eq!(
            is_streaming(),
            1,
            "publish_failed_if_current must not overwrite a session owned by a different thread",
        );

        // The dispatcher thread's own catch-arm publishes `Failed`.
        publish_failed_if_current(
            &handle.dispatcher,
            dispatcher_thread_id,
            "intentional outer-machinery panic".to_owned(),
        );

        // Both status readers now report the dead loop, with no teardown join.
        assert_eq!(
            is_streaming(),
            0,
            "is_streaming must return 0 immediately after an outer dispatcher panic",
        );
        assert_eq!(
            is_authenticated(),
            0,
            "is_authenticated must return 0 immediately after an outer dispatcher panic",
        );

        // Release the parked stand-in (it self-terminates on the flag; the
        // publish already detached its `JoinHandle` into the `Failed` variant),
        // then shut the harness client down and wait for its consumer thread to
        // drain so nothing outlives the test.
        release.store(true, Ordering::Release);
        if let Some(client) = handle.inner.lock_recover().take() {
            client.shutdown();
            drop(client);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !drained.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "harness client did not drain within 5 s",
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
