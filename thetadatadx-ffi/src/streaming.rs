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
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::{set_error, set_error_from, set_error_with_code};
use crate::types::{ThetaDataDxConfig, ThetaDataDxCredentials, ThetaDataDxMarketDataClient};

/// Lock a `Mutex`, recovering the guard through poisoning rather than
/// propagating a panic across the C ABI. A poisoned lock here means some
/// other thread panicked while holding it; the FFI handles only own plain
/// data (callback slots, the dispatcher state machine, drain flags), so the
/// inner value stays usable and the consistent behaviour is to keep serving
/// the C caller. `into_inner` returns that still-valid guard.
pub(crate) trait LockRecover<T> {
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
pub(crate) struct FfiCallback {
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
        // `Error.message`, `UnknownFrame.payload`, `Ping.payload`, and
        // `cause_io_error_kind` on the three disconnect events);
        // it is dropped at the end of this function,
        // after the user callback returns. The user MUST NOT retain the
        // `*const ThetaDataDxStreamEvent` pointer past the callback boundary.
        let buffered = fpss_event_to_ffi(event);
        let event_ptr: *const ThetaDataDxStreamEvent = std::ptr::from_ref(&buffered.event);
        (self.callback)(event_ptr, self.ctx);
    }
}

// ── Unified + FPSS handles ──

/// Opaque unified client handle — wraps both market-data and streaming.
pub struct ThetaDataDxClient {
    pub(crate) inner: thetadatadx::Client,
    /// The client's streaming front-end, over the same core lifecycle as
    /// `inner.stream()`.
    pub(crate) stream: ThetaDataDxStreamHandle,
}

/// Opaque streaming handle: the streaming front-end of a client, without a
/// market-data connection.
///
/// `thetadatadx_streaming_connect` allocates it and opens nothing; the FPSS TLS
/// connection is opened by `thetadatadx_streaming_set_callback`. Every
/// `thetadatadx_streaming_*` call runs the same body as its
/// `thetadatadx_client_*` counterpart, over the core streaming lifecycle the
/// unified client uses, so the two handles share one lifecycle contract.
pub struct ThetaDataDxStreamHandle {
    service: Arc<thetadatadx::StreamService>,
    /// Callback registered via `*_set_callback`. `None` until the first
    /// registration; persisted across stop and reconnect so the reconnect
    /// path can re-attach the same C user function without re-asking the
    /// caller for it. Cleared when `thetadatadx_client_batches_open` starts a
    /// batch session in its place.
    pub(crate) callback: Mutex<Option<FfiCallback>>,
    /// The subscriptions a reconnect is replaying. A reconnect stops the
    /// session before the fallible restart, and the stop clears the session's
    /// own lists, so this is what a retry after a failed restart replays. An
    /// explicit stop or a new callback registration starts over and clears it.
    saved_subscriptions: Mutex<Option<SavedSubscriptions>>,
}

/// Per-contract and full-stream subscriptions captured for replay.
type SavedSubscriptions = (
    Vec<(
        thetadatadx::streaming::SubscriptionKind,
        thetadatadx::streaming::Contract,
    )>,
    Vec<(
        thetadatadx::streaming::SubscriptionKind,
        thetadatadx::SecType,
    )>,
);

/// Drain budget of the reconnect barrier and of `*_free`.
const DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl ThetaDataDxStreamHandle {
    fn new(service: Arc<thetadatadx::StreamService>) -> Self {
        Self {
            service,
            callback: Mutex::new(None),
            saved_subscriptions: Mutex::new(None),
        }
    }

    fn stream(&self) -> thetadatadx::StreamSurface<'_> {
        self.service.stream()
    }

    fn set_callback(&self, callback: Option<ThetaDataDxStreamCallback>, ctx: *mut c_void) -> i32 {
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
        // so registration is serialised: two racing calls take this lock in
        // turn, the first installs and starts, the second observes the
        // now-`Live` slot and is rejected, so the started session's
        // `(callback, ctx)` can never diverge from the stored registration.
        // The dispatcher invokes the `cb` captured by copy below, never
        // reading this mutex, so holding it across `start_streaming` does not
        // deadlock the first delivered event.
        let mut guard = self.callback.lock_recover();
        // Registration is one-shot while a session is live: reject a second
        // call without disturbing the live session's stored `(callback, ctx)`.
        // Replacement is only permitted after a stop (or reconnect), where the
        // slot is no longer `Live`.
        if self.stream().is_streaming() {
            set_error("streaming already started");
            return -1;
        }
        // A new registration starts a fresh session, so a failed reconnect's
        // saved subscriptions no longer apply.
        *self.saved_subscriptions.lock_recover() = None;
        // Store BEFORE `start_streaming` so the engine observes a consistent
        // handle; roll back to `None` if start fails.
        *guard = Some(cb);
        match self
            .stream()
            .start_streaming(move |event: &thetadatadx::fpss::StreamEvent| {
                cb.invoke(event);
            }) {
            Ok(()) => 0,
            Err(e) => {
                *guard = None;
                set_error_from(&e);
                -1
            }
        }
    }

    /// Subscribe (`subscribe = true`) or unsubscribe one request.
    ///
    /// # Safety
    ///
    /// `request` is null or a `ThetaDataDxSubscriptionRequest` the caller pins
    /// for the call duration.
    unsafe fn subscription(
        &self,
        request: *const ThetaDataDxSubscriptionRequest,
        subscribe: bool,
    ) -> i32 {
        // SAFETY: forwarded from this function's contract; `coerce_subscription`
        // validates the discriminant + tagged-union fields, setting
        // `thetadatadx_last_error` on malformed payloads.
        let Some(sub) = (unsafe { coerce_subscription(request) }) else {
            return -1;
        };
        let stream = self.stream();
        let result = if subscribe {
            stream.subscribe(sub)
        } else {
            stream.unsubscribe(sub)
        };
        match result {
            Ok(()) => 0,
            Err(e) => {
                set_error_from(&e);
                -1
            }
        }
    }

    fn reconnect(&self, prefix: &str) -> i32 {
        let stream = self.stream();
        // Save active subscriptions. With no live session, a previous
        // reconnect stopped it and then failed to restart: replay what that
        // attempt saved. With neither, there is nothing to reconnect (a stop
        // drops the subscriptions), and falling back to an empty list would
        // open a session with nothing subscribed. Both snapshots fail only
        // when no session is live.
        let (saved_subs, saved_full_subs) = {
            let mut saved = self.saved_subscriptions.lock_recover();
            match (
                stream.active_subscriptions(),
                stream.active_full_subscriptions(),
            ) {
                (Ok(subs), Ok(full_subs)) => *saved = Some((subs, full_subs)),
                (Err(_), _) | (_, Err(_)) => {
                    if saved.is_none() {
                        set_error_with_code(
                            &format!(
                                "streaming not started -- call {prefix}_set_callback, then \
                                 subscribe, before {prefix}_reconnect"
                            ),
                            crate::error::THETADATADX_ERR_NETWORK,
                        );
                        return -1;
                    }
                }
            }
            saved.clone().unwrap_or_default()
        };

        // Reconnect requires a prior `set_callback`: without one there is no
        // destination for the new stream's events.
        let Some(cb) = *self.callback.lock_recover() else {
            set_error(&format!(
                "no callback registered -- call {prefix}_set_callback before {prefix}_reconnect"
            ));
            return -1;
        };

        // Initiate teardown of the FPSS reader and Disruptor consumer for the
        // current session. This swaps the streaming slot to `Stopped` and
        // signals the I/O thread; the consumer keeps firing the old C callback
        // until its exit path joins.
        stream.stop_streaming();

        // Wait for the previous consumer thread to finish firing the old
        // callback BEFORE opening a fresh session bound to the same C callback
        // / `ctx`. The C ABI guarantees single-threaded callback invocation;
        // without this barrier the old consumer can still be inside the user's
        // `ctx` when the new consumer starts firing on a different thread.
        if !stream.await_drain(DRAIN_TIMEOUT) {
            set_error(
                "reconnect drain barrier timed out after 5s — previous \
                 callback is still in flight; refusing to bind the new \
                 session to the same ctx",
            );
            return -1;
        }

        if let Err(e) = stream.start_streaming(move |event: &thetadatadx::fpss::StreamEvent| {
            cb.invoke(event);
        }) {
            set_error_from(&e);
            return -1;
        }

        // Re-subscribe through the core's paced replay engine, which spreads a
        // large saved set over wall-clock time instead of firing it at a
        // recovering upstream back-to-back. The session is live either way; a
        // subscription that failed to restore is reported as
        // `PartialReconnect` so the caller can retry it rather than wait on a
        // contract that never delivers.
        let restored = stream.restore_subscriptions(&saved_subs, &saved_full_subs);
        *self.saved_subscriptions.lock_recover() = None;
        if let Err(e) = restored {
            set_error_from(&e);
            return -1;
        }
        0
    }

    fn active_subscriptions(&self) -> *mut ThetaDataDxSubscriptionArray {
        match self.stream().active_subscriptions() {
            Ok(subs) => build_subscription_array(
                subs.iter()
                    .map(|(k, c)| (k.kind_str().to_string(), format!("{c}"))),
            ),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    }

    fn active_full_subscriptions(&self) -> *mut ThetaDataDxSubscriptionArray {
        match self.stream().active_full_subscriptions() {
            Ok(subs) => build_subscription_array(subs.iter().filter_map(|(k, st)| {
                k.full_kind_str()
                    .map(|kind| (kind.to_string(), format!("{st:?}")))
            })),
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
    }

    fn stop_streaming(&self) {
        *self.saved_subscriptions.lock_recover() = None;
        self.stream().stop_streaming();
    }

    /// # Safety
    ///
    /// `out_ms` is a non-null pointer the caller pins for the call duration.
    unsafe fn millis_since_last_event(&self, out_ms: *mut u64) -> i32 {
        let (ms, rc) = match self.stream().millis_since_last_event() {
            Some(ms) => (ms, 0),
            None => (0, 1),
        };
        // SAFETY: forwarded from this function's contract.
        unsafe { *out_ms = ms };
        rc
    }

    fn last_connected_addr(&self) -> *mut c_char {
        match self.stream().last_connected_addr() {
            Some(addr) => match CString::new(addr) {
                Ok(c) => c.into_raw(),
                Err(e) => {
                    set_error(&format!("connected address contains an interior NUL: {e}"));
                    ptr::null_mut()
                }
            },
            None => ptr::null_mut(),
        }
    }

    /// The `*_free` barrier: stop, then wait for the consumer to finish
    /// firing the callback so the caller can release `ctx` on return.
    fn stop_and_drain(&self, prefix: &str) {
        // `stop_streaming` is idempotent on an already-stopped slot and, when
        // the slot was `Live`, registers the retired session's drain flag. The
        // barrier polls every registered flag regardless of the current slot
        // state: an earlier explicit stop is the path most likely to leave a
        // callback still firing.
        self.stop_streaming();
        // `await_drain` also answers `true` when nothing was ever started; the
        // snapshot tells an honest timeout from a handle that never streamed,
        // so only the former is logged.
        let had_prior_session = self.stream().prev_drained_is_set();
        if had_prior_session && !self.stream().await_drain(DRAIN_TIMEOUT) {
            tracing::error!(
                target: "thetadatadx::ffi",
                timeout_ms = DRAIN_TIMEOUT.as_millis() as u64,
                "{prefix}_free: drain barrier exceeded timeout -- callback may still \
                 be firing on the consumer thread; user ctx must remain valid past return",
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  #[repr(C)] FPSS streaming event types — zero-copy across FFI
//
//  The kind tag, the per-event structs, the tagged `ThetaDataDxStreamEvent`
//  and the converter are expanded from the core event table in
//  `stream_events.rs`. `FfiBufferedEvent` below owns the backing memory for
//  every borrowed pointer the tagged event exposes (the `Contract.symbol`
//  C strings, the control events' strings, and the `Ping` / `UnknownFrame`
//  byte payloads).
// ═══════════════════════════════════════════════════════════════════════

/// Internal buffered event — owns heap data that backs the `ThetaDataDxStreamEvent`.
///
/// Constructed once per delivered FPSS event inside the user-callback
/// boundary (see `FfiCallback::invoke`); lives only for the duration of
/// the user's `extern "C" fn` call and is dropped immediately after.
///
/// Each backing-storage `Option<...>` slot owns one kind of borrowed
/// pointer payload: `_contract_symbol` for the `Contract.symbol` of any
/// event that carries a contract, `_string` for the single string a control
/// event carries (`LoginSuccess.permissions`, `ServerError.message`,
/// `ParseError.message`, and the `cause_io_error_kind` on `Disconnected` /
/// `Reconnecting` / `ReconnectsExhausted`), and `_payload_bytes` for
/// `UnknownFrame.payload` / `Ping.payload`. The borrowed `*const c_char` /
/// `*const u8` pointers in the public `ThetaDataDxStreamEvent` reference
/// INTO these slots; users MUST NOT retain those pointers past the callback
/// boundary.
#[repr(C)]
pub(crate) struct FfiBufferedEvent {
    pub(crate) event: ThetaDataDxStreamEvent,
    /// Owns the `CString` backing the event's `Contract.symbol` pointer.
    _contract_symbol: Option<CString>,
    /// Owns the `CString` backing the event's one string field.
    _string: Option<CString>,
    /// Owns the byte payload backing `UnknownFrame.payload` / `Ping.payload`.
    _payload_bytes: Option<Vec<u8>>,
}

include!("stream_events.rs");

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
#[unsafe(no_mangle)]
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
//  Unified client — market-data + streaming through one handle
// ═══════════════════════════════════════════════════════════════════════

/// Connect to `ThetaData` (market-data only — FPSS streaming is NOT started).
///
/// Authenticates once, opens gRPC channel. Call `thetadatadx_client_set_callback()`
/// later to start FPSS. Market-data endpoints are available immediately.
///
/// Returns null on connection/auth failure (check `thetadatadx_last_error()`).
#[unsafe(no_mangle)]
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
                stream: ThetaDataDxStreamHandle::new(Arc::clone(client.stream_service())),
                inner: client,
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
#[unsafe(no_mangle)]
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
/// the saved registration: stop+restart is a normal user flow
/// (`thetadatadx_client_reconnect` is built on top of it).
///
/// On the first call after `thetadatadx_client_connect` this is the initial
/// registration. Calling `thetadatadx_client_set_callback` while streaming is
/// already active returns -1 with `"streaming already started"`.
///
/// Returns 0 on success, -1 on error (check `thetadatadx_last_error()`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_set_callback(
    handle: *const ThetaDataDxClient,
    callback: Option<ThetaDataDxStreamCallback>,
    ctx: *mut c_void,
) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            set_error("unified handle is null");
            return -1;
        };
        h.set_callback(callback, ctx)
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
///   `expiration = "20261218"`, `strike = "550"`, `right = "C"`.
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
    /// `"STOCK"` / `"OPTION"` / `"INDEX"`. For a full-stream subscription it
    /// names the universe; for a per-contract underlier (no option legs) it
    /// selects `"STOCK"` (the default when NULL) vs `"INDEX"`. NULL / ignored
    /// for an option per-contract subscription.
    pub sec_type: *const c_char,
}

// Layout drift-guard: pin the LP64 `#[repr(C)]` size + alignment on the
// Rust side, matching `abi_struct_layout_asserts.hpp.inc`. `scope` (i32)
// @0, `kind` (i32) @4, then five pointer fields packed 8 bytes apart with no
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
                    set_error_with_code(
                        &format!("invalid kind {other}"),
                        crate::error::THETADATADX_ERR_INVALID_PARAMETER,
                    );
                    return None;
                }
            };
            let symbol = require_cstr!(symbol_ptr, None);
            let contract = if expiration_ptr.is_null()
                && strike_ptr.is_null()
                && right_ptr.is_null()
            {
                // No option legs: a stock or index underlier. `sec_type`
                // selects which; a null or empty sec_type is a stock, as the
                // header documents, so a zero-initialised request subscribes
                // to the stock.
                if sec_type_ptr.is_null() {
                    Contract::stock(symbol)
                } else {
                    let sec_type = require_cstr!(sec_type_ptr, None);
                    if sec_type.eq_ignore_ascii_case("index") {
                        Contract::index(symbol)
                    } else if sec_type.is_empty() || sec_type.eq_ignore_ascii_case("stock") {
                        Contract::stock(symbol)
                    } else {
                        set_error_with_code(
                            &format!(
                                "invalid sec_type '{sec_type}' for a contract subscription; expected STOCK or INDEX"
                            ),
                            crate::error::THETADATADX_ERR_INVALID_PARAMETER,
                        );
                        return None;
                    }
                }
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
                    set_error_with_code(
                        "full-stream Quote is not a valid subscription",
                        crate::error::THETADATADX_ERR_INVALID_PARAMETER,
                    );
                    return None;
                }
                THETADATADX_SUB_KIND_MARKET_VALUE => {
                    set_error_with_code(
                        "full-stream MarketValue is not a valid subscription",
                        crate::error::THETADATADX_ERR_INVALID_PARAMETER,
                    );
                    return None;
                }
                other => {
                    set_error_with_code(
                        &format!("invalid kind {other}"),
                        crate::error::THETADATADX_ERR_INVALID_PARAMETER,
                    );
                    return None;
                }
            };
            let sec_type_str = require_cstr!(sec_type_ptr, None);
            let sec_type = match sec_type_str.to_uppercase().as_str() {
                "STOCK" => thetadatadx::SecType::Stock,
                "OPTION" => thetadatadx::SecType::Option,
                "INDEX" => thetadatadx::SecType::Index,
                other => {
                    set_error_with_code(
                        &format!("invalid sec_type {other:?} (expected STOCK, OPTION, INDEX)"),
                        crate::error::THETADATADX_ERR_INVALID_PARAMETER,
                    );
                    return None;
                }
            };
            Some(Subscription::Full { sec_type, kind })
        }
        other => {
            set_error_with_code(
                &format!("invalid scope {other}"),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER,
            );
            None
        }
    }
}

/// Polymorphic subscribe on the unified client. Mirrors the Rust
/// `client.subscribe(Subscription)` shape — one entry point handles
/// every per-contract and full-stream variant.
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_subscribe(
    handle: *const ThetaDataDxClient,
    request: *const ThetaDataDxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            set_error("unified handle is null");
            return -1;
        };
        // SAFETY: forwarded from this function's contract.
        unsafe { h.subscription(request, true) }
    })
}

/// Polymorphic unsubscribe on the unified client.
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_unsubscribe(
    handle: *const ThetaDataDxClient,
    request: *const ThetaDataDxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            set_error("unified handle is null");
            return -1;
        };
        // SAFETY: forwarded from this function's contract.
        unsafe { h.subscription(request, false) }
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
/// Requires a live session started by `thetadatadx_client_set_callback`, or
/// a previous reconnect that failed to restart, whose subscriptions it
/// replays. Without either, including after `thetadatadx_client_stop_streaming`,
/// which drops the subscriptions, it returns -1 with `THETADATADX_ERR_NETWORK`.
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
/// If the session reconnected but some subscriptions failed to restore, the
/// stream stays live and the call returns -1 with `THETADATADX_ERR_STREAM`;
/// `thetadatadx_client_active_subscriptions` lists what was restored.
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_reconnect(handle: *const ThetaDataDxClient) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            set_error("unified handle is null");
            return -1;
        };
        h.reconnect("thetadatadx_client")
    })
}

/// Check if streaming is active on the unified client.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_is_streaming(handle: *const ThetaDataDxClient) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            return 0;
        };
        i32::from(h.stream().is_streaming())
    })
}

/// Check if the live streaming session is authenticated on the unified
/// client.
///
/// Distinct from `thetadatadx_client_is_streaming`: the session can be live
/// yet briefly unauthenticated mid-reconnect. Returns 1 when
/// authenticated, 0 otherwise (including a null handle, before streaming
/// starts, and after it stops).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_is_authenticated(
    handle: *const ThetaDataDxClient,
) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            return 0;
        };
        i32::from(h.stream().is_authenticated())
    })
}

/// Get active subscriptions as a typed array. Returns null on error.
///
/// Caller must free the result with `thetadatadx_subscription_array_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_active_subscriptions(
    handle: *const ThetaDataDxClient,
) -> *mut ThetaDataDxSubscriptionArray {
    ffi_boundary!(std::ptr::null_mut(), {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            set_error("unified handle is null");
            return ptr::null_mut();
        };
        h.active_subscriptions()
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_active_full_subscriptions(
    handle: *const ThetaDataDxClient,
) -> *mut ThetaDataDxSubscriptionArray {
    ffi_boundary!(std::ptr::null_mut(), {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            set_error("unified handle is null");
            return ptr::null_mut();
        };
        h.active_full_subscriptions()
    })
}

/// Borrow the market-data client from a unified handle.
///
/// Returns a `*const ThetaDataDxMarketDataClient` that can be passed to all `thetadatadx_stock_*`,
/// `thetadatadx_option_*`, `thetadatadx_index_*`, `thetadatadx_calendar_*`, and `thetadatadx_interest_rate_*`
/// functions. This avoids a second `thetadatadx_market_data_connect()` call and reuses
/// the same authenticated session.
///
/// The returned pointer is **NOT owned** -- do NOT call `thetadatadx_market_data_free`
/// on it. It is valid as long as the `ThetaDataDxClient` handle is alive.
///
/// # Safety
///
/// This cast is sound because `ThetaDataDxMarketDataClient` is `#[repr(transparent)]` over
/// `MarketDataClient`, and `Client` Derefs to `&MarketDataClient`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_market_data(
    handle: *const ThetaDataDxClient,
) -> *const ThetaDataDxMarketDataClient {
    ffi_boundary!(std::ptr::null(), {
        if handle.is_null() {
            set_error("unified handle is null");
            return ptr::null();
        }
        // SAFETY: handle is a non-null pointer returned by the matching thetadatadx_*_new and not yet passed to thetadatadx_*_free.
        let handle = unsafe { &*handle };
        // ThetaDataDxMarketDataClient is #[repr(transparent)] over MarketDataClient, so this cast is safe.
        let mdds_ref: &thetadatadx::mdds::MarketDataClient = handle.inner.market_data();
        std::ptr::from_ref::<thetadatadx::mdds::MarketDataClient>(mdds_ref)
            .cast::<ThetaDataDxMarketDataClient>()
    })
}

/// Stop streaming on the unified client. Market-data remains available.
///
/// Tears down the FPSS event-dispatch consumer thread and the underlying TLS
/// reader, and returns once the consumer has exited, except when called from
/// the consumer itself. Use `thetadatadx_client_await_drain` to confirm the
/// consumer has finished firing the callback before freeing `ctx` or
/// replacing the callback registration.
///
/// The stop drops the subscriptions, so `thetadatadx_client_reconnect` after
/// it returns -1 with `THETADATADX_ERR_NETWORK`. Resume with
/// `thetadatadx_client_set_callback` and subscribe again.
///
/// # Lifecycle restriction
///
/// MUST NOT be called from inside the user callback. Doing so
/// returns control to the caller while the old callback is still
/// firing on the event-dispatch consumer thread; freeing or replacing
/// `ctx` based on stop returning will trigger use-after-free in the
/// callback. Drive stop / reconnect from a separate thread instead,
/// then call `thetadatadx_client_await_drain` for the quiescence barrier.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_stop_streaming(handle: *const ThetaDataDxClient) {
    ffi_boundary!((), {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        if let Some(h) = unsafe { handle.as_ref() } {
            h.stream.stop_streaming();
        }
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_millis_since_last_event(
    handle: *const ThetaDataDxClient,
    out_ms: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() || out_ms.is_null() {
            set_error("handle or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: `handle` is a non-null live handle from `thetadatadx_client_connect`
        // and `out_ms` is non-null, pinned by the caller for the call duration.
        unsafe { (*handle).stream.millis_since_last_event(out_ms) }
    })
}

/// UNIX-nanosecond receive timestamp of the most recent inbound
/// streaming frame of any kind on this unified handle. Returns `0`
/// when the handle is null, streaming has not started, or no frame
/// has been received yet. Raw feed for
/// `thetadatadx_client_millis_since_last_event`, exposed for callers
/// correlating against their own pipeline timestamps.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_last_event_received_at_unix_nanos(
    handle: *const ThetaDataDxClient,
) -> i64 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            return 0;
        };
        h.stream().last_event_received_at_unix_nanos()
    })
}

/// Address (`host:port`) of the streaming server the current session
/// is connected to, following the session across auto-reconnects.
///
/// Returns a heap-owned C string the caller must release with
/// `thetadatadx_string_free`, or null when streaming has not started (or the
/// handle is null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_last_connected_addr(
    handle: *const ThetaDataDxClient,
) -> *mut std::os::raw::c_char {
    ffi_boundary!(ptr::null_mut(), {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            set_error("unified handle is null");
            return ptr::null_mut();
        };
        h.last_connected_addr()
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_dropped_events(
    handle: *const ThetaDataDxClient,
) -> u64 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            return 0;
        };
        h.stream().dropped_event_count()
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_ring_occupancy(
    handle: *const ThetaDataDxClient,
) -> u64 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            return 0;
        };
        h.stream().ring_occupancy() as u64
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_ring_capacity(handle: *const ThetaDataDxClient) -> u64 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            return 0;
        };
        h.stream().ring_capacity() as u64
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_panic_count(handle: *const ThetaDataDxClient) -> u64 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            return 0;
        };
        h.stream().panic_count()
    })
}

/// Wait for the previously-superseded streaming session to quiesce.
///
/// Returns `1` once the previous `thetadatadx_client_stop_streaming` /
/// `_reconnect` session's event-dispatch consumer thread has finished
/// firing the registered callback, and also when no stream has ever been
/// started or stopped on this handle (an idle handle has nothing to
/// drain, so it is already quiesced). Returns `0` on timeout or a null handle.
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_await_drain(
    handle: *const ThetaDataDxClient,
    timeout_ms: u64,
) -> i32 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_client_connect`
        // not yet passed to `thetadatadx_client_free`.
        let Some(h) = (unsafe { handle.as_ref() }).map(|h| &h.stream) else {
            return 0;
        };
        i32::from(
            h.stream()
                .await_drain(std::time::Duration::from_millis(timeout_ms)),
        )
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_client_free(handle: *mut ThetaDataDxClient) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }
        // SAFETY: the pointer was returned by Box::into_raw / thetadatadx_*_new and has not been freed; ownership returns to Rust.
        let handle = unsafe { Box::from_raw(handle) };
        handle.stream.stop_and_drain("thetadatadx_client");
        drop(handle);
    })
}

// ═══════════════════════════════════════════════════════════════════════
//  FPSS — Real-time streaming client
//
//  Every entry point below runs the same body as its `thetadatadx_client_*`
//  counterpart, so the contracts documented there apply here unchanged.
// ═══════════════════════════════════════════════════════════════════════

/// Allocate a streaming handle and stash the connection parameters.
///
/// **Does NOT open the FPSS TLS connection** and issues no authentication
/// request: the connection is opened by `thetadatadx_streaming_set_callback`,
/// through the streaming service's own credentials handshake. The config is
/// validated here, as `thetadatadx_client_connect` validates it, so an
/// out-of-range value is refused naming its field.
///
/// Returns null on argument validation failure (check `thetadatadx_last_error()`).
#[unsafe(no_mangle)]
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
        match thetadatadx::StreamService::new(&creds.inner, config.inner.clone()) {
            Ok(service) => {
                // Seed the process-global async runtime from this client's
                // config so `worker_threads` is honored when a standalone
                // streaming client is the first client created in the process.
                crate::runtime_from_config(&service.config().runtime);
                Box::into_raw(Box::new(ThetaDataDxStreamHandle::new(Arc::new(service))))
            }
            Err(e) => {
                set_error_from(&e);
                ptr::null_mut()
            }
        }
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
#[unsafe(no_mangle)]
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

/// Register a queued FPSS callback and open the FPSS connection.
///
/// Same contract as `thetadatadx_client_set_callback`, including the `ctx`
/// lifetime and thread-affinity rules: `ctx` must stay valid until
/// `thetadatadx_streaming_free` returns, or until a stop / reconnect has
/// returned and `thetadatadx_streaming_await_drain` has returned `1`.
///
/// Calling it while streaming is active returns -1 with
/// `"streaming already started"`. After `thetadatadx_streaming_shutdown` the
/// handle accepts a fresh registration, which REPLACES the saved one and
/// starts a session with nothing subscribed.
///
/// Returns 0 on success, -1 on error (check `thetadatadx_last_error()`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_set_callback(
    handle: *const ThetaDataDxStreamHandle,
    callback: Option<ThetaDataDxStreamCallback>,
    ctx: *mut c_void,
) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`
        // not yet passed to `thetadatadx_streaming_free`.
        let Some(h) = (unsafe { handle.as_ref() }) else {
            set_error("streaming handle is null");
            return -1;
        };
        h.set_callback(callback, ctx)
    })
}

/// Check if the streaming connection is currently open.
///
/// Distinct from `thetadatadx_streaming_is_authenticated`: the connection
/// can be open yet briefly unauthenticated mid-reconnect. A panicked
/// dispatcher folds back to `0` so the failed state is uniformly visible
/// across status readers. Returns 1 when streaming, 0 otherwise
/// (including a null handle and after shutdown).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_is_streaming(
    handle: *const ThetaDataDxStreamHandle,
) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        unsafe { handle.as_ref() }.map_or(0, |h| i32::from(h.stream().is_streaming()))
    })
}

/// Check if the live streaming session is authenticated.
///
/// Returns 1 if authenticated, 0 if not (including a null handle, before
/// streaming starts, after it stops, and after a dispatcher panic).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_is_authenticated(
    handle: *const ThetaDataDxStreamHandle,
) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        unsafe { handle.as_ref() }.map_or(0, |h| i32::from(h.stream().is_authenticated()))
    })
}

/// Get a snapshot of currently active subscriptions.
///
/// Returns a heap-allocated `ThetaDataDxSubscriptionArray`, or null with
/// `thetadatadx_last_error()` set when the handle is null or streaming is not
/// active. Caller must free the result with `thetadatadx_subscription_array_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_active_subscriptions(
    handle: *const ThetaDataDxStreamHandle,
) -> *mut ThetaDataDxSubscriptionArray {
    ffi_boundary!(std::ptr::null_mut(), {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        let Some(h) = (unsafe { handle.as_ref() }) else {
            set_error("streaming handle is null");
            return ptr::null_mut();
        };
        h.active_subscriptions()
    })
}

/// Get a snapshot of currently active full-stream subscriptions.
///
/// Same projection as `thetadatadx_client_active_full_subscriptions`: each
/// entry's `contract` field carries the security type and its `kind` field
/// the full-stream kind label. Returns null with `thetadatadx_last_error()`
/// set when the handle is null or streaming is not active. Caller must free
/// the result with `thetadatadx_subscription_array_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_active_full_subscriptions(
    handle: *const ThetaDataDxStreamHandle,
) -> *mut ThetaDataDxSubscriptionArray {
    ffi_boundary!(std::ptr::null_mut(), {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        let Some(h) = (unsafe { handle.as_ref() }) else {
            set_error("streaming handle is null");
            return ptr::null_mut();
        };
        h.active_full_subscriptions()
    })
}

/// Polymorphic subscribe on the standalone streaming client.
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`),
/// including when streaming is not active.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_subscribe(
    handle: *const ThetaDataDxStreamHandle,
    request: *const ThetaDataDxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        let Some(h) = (unsafe { handle.as_ref() }) else {
            set_error("streaming handle is null");
            return -1;
        };
        // SAFETY: `request` is null or pinned by the caller for the call duration.
        unsafe { h.subscription(request, true) }
    })
}

/// Polymorphic unsubscribe on the standalone streaming client.
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_unsubscribe(
    handle: *const ThetaDataDxStreamHandle,
    request: *const ThetaDataDxSubscriptionRequest,
) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        let Some(h) = (unsafe { handle.as_ref() }) else {
            set_error("streaming handle is null");
            return -1;
        };
        // SAFETY: `request` is null or pinned by the caller for the call duration.
        unsafe { h.subscription(request, false) }
    })
}

/// Reconnect the streaming client, re-subscribing all previous subscriptions.
///
/// Same contract as `thetadatadx_client_reconnect`: requires a live session
/// started by `thetadatadx_streaming_set_callback`, waits up to 5 s for the
/// previous session's callback to finish before binding the new session to
/// the same `ctx`, and replays the saved subscriptions. A retry after a failed
/// reconnect replays the subscriptions the failed attempt saved. After
/// `thetadatadx_streaming_shutdown`, which drops the subscriptions, it returns
/// -1 with `THETADATADX_ERR_NETWORK`: resume with
/// `thetadatadx_streaming_set_callback` and subscribe again.
///
/// Returns 0 on success, or -1 on error (check `thetadatadx_last_error()`).
/// If the session reconnected but some subscriptions failed to restore, the
/// stream stays live and the call returns -1 with `THETADATADX_ERR_STREAM`;
/// `thetadatadx_streaming_active_subscriptions` lists what was restored.
///
/// # Lifecycle restriction
///
/// MUST NOT be called from inside the user callback. Drive reconnect from a
/// separate thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_reconnect(
    handle: *const ThetaDataDxStreamHandle,
) -> i32 {
    ffi_boundary!(-1, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        let Some(h) = (unsafe { handle.as_ref() }) else {
            set_error("streaming handle is null");
            return -1;
        };
        h.reconnect("thetadatadx_streaming")
    })
}

/// Milliseconds since the most recent inbound streaming frame of any
/// kind on this handle. Same contract as
/// `thetadatadx_client_millis_since_last_event`: returns `0` on success with
/// the value in `*out_ms`, `1` when no session is live or no frame
/// has been received yet, `-1` on a null pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_millis_since_last_event(
    handle: *const ThetaDataDxStreamHandle,
    out_ms: *mut u64,
) -> i32 {
    ffi_boundary!(-1, {
        if handle.is_null() || out_ms.is_null() {
            set_error("handle or out-parameter pointer is null");
            return -1;
        }
        // SAFETY: `handle` is a non-null live handle from `thetadatadx_streaming_connect`
        // and `out_ms` is non-null, pinned by the caller for the call duration.
        unsafe { (*handle).millis_since_last_event(out_ms) }
    })
}

/// UNIX-nanosecond receive timestamp of the most recent inbound
/// streaming frame of any kind on this handle. Returns `0` when
/// the handle is null, no session is live, or no frame has been
/// received yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_last_event_received_at_unix_nanos(
    handle: *const ThetaDataDxStreamHandle,
) -> i64 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        unsafe { handle.as_ref() }.map_or(0, |h| h.stream().last_event_received_at_unix_nanos())
    })
}

/// Address (`host:port`) of the streaming server the current
/// session is connected to, following the session across
/// auto-reconnects. Returns a heap-owned C string the caller must
/// release with `thetadatadx_string_free`, or null when no session is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_last_connected_addr(
    handle: *const ThetaDataDxStreamHandle,
) -> *mut c_char {
    ffi_boundary!(ptr::null_mut(), {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        let Some(h) = (unsafe { handle.as_ref() }) else {
            set_error("streaming handle is null");
            return ptr::null_mut();
        };
        h.last_connected_addr()
    })
}

/// Cumulative count of streaming events the TLS reader could not publish
/// into the Disruptor ring because the consumer fell behind and the
/// ring was full, counted across every session this handle has run: a
/// reconnect or a shutdown retires the session, and the drops it recorded
/// stay in the total.
///
/// Returns 0 if the handle is null or no session has ever started.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_dropped_events(
    handle: *const ThetaDataDxStreamHandle,
) -> u64 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        unsafe { handle.as_ref() }.map_or(0, |h| h.stream().dropped_event_count())
    })
}

/// Point-in-time count of streaming events published into the event ring
/// but not yet drained into the registered callback — the in-flight
/// depth between the I/O thread and the dispatcher.
///
/// The leading back-pressure signal: `thetadatadx_streaming_dropped_events` only
/// moves AFTER data has been lost, while a rising occupancy that
/// approaches `thetadatadx_streaming_ring_capacity` predicts those drops while
/// there is still time to react. Sampling never blocks the feed; safe
/// to poll from any thread at any cadence.
///
/// Returns 0 if the handle is null or no session is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_ring_occupancy(
    handle: *const ThetaDataDxStreamHandle,
) -> u64 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        unsafe { handle.as_ref() }.map_or(0, |h| h.stream().ring_occupancy() as u64)
    })
}

/// Configured capacity of the streaming event ring in slots (the
/// `streaming_ring_size` setting, a power of two), the fixed denominator
/// for `thetadatadx_streaming_ring_occupancy`. When the occupancy sample
/// approaches this value the ring is saturating and further events
/// will be dropped (counted by `thetadatadx_streaming_dropped_events`).
///
/// Returns 0 if the handle is null or no session is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_ring_capacity(
    handle: *const ThetaDataDxStreamHandle,
) -> u64 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        unsafe { handle.as_ref() }.map_or(0, |h| h.stream().ring_capacity() as u64)
    })
}

/// Cumulative count of user-callback panics caught by the per-invocation
/// `catch_unwind` boundary on this handle, counted across every
/// session it has run: a reconnect or a shutdown retires the session, and
/// the panics it caught stay in the total.
///
/// Each caught panic is also surfaced via `tracing::error!` with target
/// `thetadatadx::fpss::poller`. A panic in the callback is caught,
/// recorded here, and does not stop event delivery — the next event
/// continues normally. Safe to call from any thread without blocking.
///
/// Returns 0 if the handle is null or no session has ever started.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_panic_count(
    handle: *const ThetaDataDxStreamHandle,
) -> u64 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        unsafe { handle.as_ref() }.map_or(0, |h| h.stream().panic_count())
    })
}

/// Stop streaming, stopping all background threads.
///
/// Same contract as `thetadatadx_client_stop_streaming`: idempotent, and
/// returns once the consumer has exited, except when called from the consumer
/// itself (pair with `thetadatadx_streaming_await_drain` before freeing
/// `ctx`). The shutdown drops the subscriptions, so
/// `thetadatadx_streaming_reconnect` after it returns -1 with
/// `THETADATADX_ERR_NETWORK`; resume with `thetadatadx_streaming_set_callback`
/// and subscribe again.
///
/// # Lifecycle restriction
///
/// MUST NOT be called from inside the user callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_shutdown(handle: *const ThetaDataDxStreamHandle) {
    ffi_boundary!((), {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        if let Some(h) = unsafe { handle.as_ref() } {
            h.stop_streaming();
        }
    })
}

/// Wait for every superseded streaming session to quiesce.
///
/// Same contract as `thetadatadx_client_await_drain`: returns `1` once every
/// prior reconnect / shutdown session's consumer has finished firing the
/// registered callback, and also when no session was ever stopped on this
/// handle (nothing to drain). Returns `0` only on timeout or a null handle.
///
/// # Lifecycle restriction
///
/// MUST be called from a thread other than the consumer thread. Calling it
/// from inside the user callback blocks the very thread it waits on and
/// always times out.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_await_drain(
    handle: *const ThetaDataDxStreamHandle,
    timeout_ms: u64,
) -> i32 {
    ffi_boundary!(0, {
        // SAFETY: `handle` is null or a live handle from `thetadatadx_streaming_connect`.
        unsafe { handle.as_ref() }.map_or(0, |h| {
            i32::from(
                h.stream()
                    .await_drain(std::time::Duration::from_millis(timeout_ms)),
            )
        })
    })
}

/// Free a streaming handle.
///
/// Same contract as `thetadatadx_client_free`: stops streaming if it is
/// live, then waits up to 5 s for the consumer thread to finish firing the
/// registered callback, so `ctx` is safe to free on return. On a drain
/// timeout it emits a `tracing::error!` and proceeds, and `ctx` MUST then
/// remain valid past return.
///
/// # Lifecycle restriction
///
/// Do NOT call `thetadatadx_streaming_free` from inside the user callback:
/// the drain waits for the very thread the call runs on, the budget
/// elapses, and control returns into a callback whose handle is freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thetadatadx_streaming_free(handle: *mut ThetaDataDxStreamHandle) {
    ffi_boundary!((), {
        if handle.is_null() {
            return;
        }
        // SAFETY: the pointer was returned by Box::into_raw in thetadatadx_streaming_connect and has not been freed; ownership returns to Rust.
        let handle = unsafe { Box::from_raw(handle) };
        handle.stop_and_drain("thetadatadx_streaming");
        drop(handle);
    })
}

#[cfg(test)]
mod null_callback_guard_tests {
    use std::ffi::c_void;

    use super::ThetaDataDxStreamEvent;

    extern "C" fn noop(_event: *const ThetaDataDxStreamEvent, _ctx: *mut c_void) {}

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
    fn coerce_subscription_honours_index_sec_type() {
        // A per-contract underlier with sec_type "INDEX" must subscribe as an
        // index, not silently as a stock (regression: VIX -> wrong instrument).
        let symbol = std::ffi::CString::new("VIX").unwrap();
        let sec_type = std::ffi::CString::new("INDEX").unwrap();
        let req = super::ThetaDataDxSubscriptionRequest {
            scope: super::THETADATADX_SUB_SCOPE_CONTRACT,
            kind: super::THETADATADX_SUB_KIND_QUOTE,
            symbol: symbol.as_ptr(),
            expiration: std::ptr::null(),
            strike: std::ptr::null(),
            right: std::ptr::null(),
            sec_type: sec_type.as_ptr(),
        };
        // SAFETY: every pointer is a live CString (or null) held for the call.
        let sub = unsafe { super::coerce_subscription(&req) }.expect("valid index contract");
        match sub {
            thetadatadx::fpss::protocol::Subscription::Contract { contract, .. } => {
                assert_eq!(contract.sec_type, thetadatadx::SecType::Index);
            }
            _ => panic!("expected a per-contract subscription"),
        }
    }

    #[test]
    fn coerce_subscription_null_sec_type_defaults_to_stock() {
        // A null sec_type, as a zero-initialised request carries, resolves to
        // a stock subscription, as the header documents.
        let symbol = std::ffi::CString::new("AAPL").unwrap();
        let req = super::ThetaDataDxSubscriptionRequest {
            scope: super::THETADATADX_SUB_SCOPE_CONTRACT,
            kind: super::THETADATADX_SUB_KIND_QUOTE,
            symbol: symbol.as_ptr(),
            expiration: std::ptr::null(),
            strike: std::ptr::null(),
            right: std::ptr::null(),
            sec_type: std::ptr::null(),
        };
        // SAFETY: `symbol` is a live CString held for the call; other ptrs null.
        let sub = unsafe { super::coerce_subscription(&req) }.expect("valid stock contract");
        match sub {
            thetadatadx::fpss::protocol::Subscription::Contract { contract, .. } => {
                assert_eq!(contract.sec_type, thetadatadx::SecType::Stock);
            }
            _ => panic!("expected a per-contract subscription"),
        }
    }

    #[test]
    fn coerce_subscription_rejects_unknown_sec_type_as_invalid_parameter() {
        // An unrecognized sec_type is a caller error, not a silent stock:
        // coercion fails and sets the typed INVALID_PARAMETER code (not the
        // untyped OTHER), so the C++ wrapper throws InvalidParameterError.
        let symbol = std::ffi::CString::new("AAPL").unwrap();
        let sec_type = std::ffi::CString::new("FUTURE").unwrap();
        let req = super::ThetaDataDxSubscriptionRequest {
            scope: super::THETADATADX_SUB_SCOPE_CONTRACT,
            kind: super::THETADATADX_SUB_KIND_QUOTE,
            symbol: symbol.as_ptr(),
            expiration: std::ptr::null(),
            strike: std::ptr::null(),
            right: std::ptr::null(),
            sec_type: sec_type.as_ptr(),
        };
        // SAFETY: `symbol` and `sec_type` are live CStrings held for the call.
        let sub = unsafe { super::coerce_subscription(&req) };
        assert!(sub.is_none(), "an unknown sec_type must be rejected");
        assert_eq!(
            crate::error::thetadatadx_last_error_code(),
            crate::error::THETADATADX_ERR_INVALID_PARAMETER,
        );
    }
}

#[cfg(test)]
mod discriminant_conversion_tests {
    use std::ffi::CStr;
    use std::io::ErrorKind;
    use std::time::Duration;

    use thetadatadx::fpss::{DisconnectCause, StreamControl, StreamEvent};
    use thetadatadx::{RemoveReason, StreamResponseType};

    use super::{ThetaDataDxStreamEventKind, fpss_event_to_ffi};

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
            let event = StreamEvent::Control(StreamControl::Disconnected {
                reason,
                cause: DisconnectCause::ServerSent,
            });
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

    /// The cause reaches the C ABI as its code plus the two details a C
    /// caller cannot derive from the code alone: the name of the I/O
    /// error kind behind a read or write failure, and the read deadline
    /// that expired. Both are flattened beside the code because the C
    /// struct carries scalars, so each has to be empty or zero on the
    /// causes that do not carry it rather than hold a stale value from
    /// the variant the schema shares the field with.
    #[test]
    fn disconnect_cause_and_its_details_reach_the_c_struct() {
        let cases = [
            (DisconnectCause::ServerSent, "", 0u64),
            (DisconnectCause::ClosedByServer, "", 0),
            (
                DisconnectCause::ReadFailed(ErrorKind::ConnectionReset),
                "ConnectionReset",
                0,
            ),
            (
                DisconnectCause::WriteFailed(ErrorKind::BrokenPipe),
                "BrokenPipe",
                0,
            ),
            (
                DisconnectCause::ReadTimeout(Duration::from_millis(45_000)),
                "",
                45_000,
            ),
        ];
        for (cause, kind_name, timeout_ms) in cases {
            // The reason is incidental here: this test is about the three
            // cause fields reaching the C struct. Which reason each cause
            // pairs with is pinned in the core crate, where the pairing
            // lives.
            let event = StreamEvent::Control(StreamControl::Disconnected {
                reason: RemoveReason::Unspecified,
                cause,
            });
            let buffered = fpss_event_to_ffi(&event);
            let disconnected = &buffered.event.disconnected;
            assert_eq!(disconnected.cause, cause.code(), "{cause:?} code");
            assert!(
                !disconnected.cause_io_error_kind.is_null(),
                "{cause:?}: the kind name is always a readable C string, empty when absent"
            );
            // SAFETY: the pointer is non-null (asserted above) and points
            // into the `CString` `buffered` owns, which outlives this read.
            let kind = unsafe { CStr::from_ptr(disconnected.cause_io_error_kind) };
            assert_eq!(
                kind.to_str().expect("the kind name is ASCII"),
                kind_name,
                "{cause:?} I/O error kind name"
            );
            assert_eq!(
                disconnected.cause_timeout_ms, timeout_ms,
                "{cause:?} read timeout"
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
            let event = StreamEvent::Control(StreamControl::Disconnected {
                reason,
                cause: DisconnectCause::ServerSent,
            });
            let buffered = fpss_event_to_ffi(&event);
            assert_eq!(buffered.event.disconnected.reason, i32::from(reason as i16));
        }
    }
}

#[cfg(test)]
mod standalone_connect_tests {
    /// The standalone handle validates its configuration when it is built, as
    /// the unified client's connect does, so an out-of-range reconnect budget
    /// is refused naming its field instead of reaching the reconnect driver.
    #[test]
    fn standalone_connect_refuses_an_out_of_range_config() {
        let cfg = crate::auth::thetadatadx_config_production();
        // SAFETY: `cfg` was just returned by thetadatadx_config_production, the
        // two C-string literals live for the whole call, and every handle this
        // test creates is freed exactly once at its end.
        unsafe {
            crate::auth::thetadatadx_config_set_reconnect_max_attempts(cfg, 0);
            let creds = crate::auth::thetadatadx_credentials_from_email(
                c"user@example.com".as_ptr(),
                c"pw".as_ptr(),
            );
            assert!(!creds.is_null());
            let handle = super::thetadatadx_streaming_connect(creds, cfg);
            assert!(handle.is_null(), "an out-of-range budget must be refused");
            assert_eq!(
                crate::error::thetadatadx_last_error_code(),
                crate::error::THETADATADX_ERR_INVALID_PARAMETER
            );
            let message = std::ffi::CStr::from_ptr(crate::error::thetadatadx_last_error())
                .to_string_lossy()
                .into_owned();
            assert!(message.contains("reconnect.max_attempts"), "{message}");
            crate::auth::thetadatadx_credentials_free(creds);
            crate::auth::thetadatadx_config_free(cfg);
        }
    }

    /// A reconnect stops the previous session before it dials. When the dial
    /// fails, the subscriptions that session held must survive for the retry
    /// to replay, or the retry succeeds with nothing subscribed.
    #[test]
    fn failed_reconnect_keeps_the_subscriptions_for_the_retry() {
        use super::{
            FfiCallback, LockRecover, ThetaDataDxStreamEvent, ThetaDataDxStreamHandle,
            thetadatadx_streaming_reconnect,
        };
        use std::os::raw::c_void;
        use thetadatadx::streaming::{Contract, SubscriptionKind};

        extern "C" fn noop(_event: *const ThetaDataDxStreamEvent, _ctx: *mut c_void) {}

        // A loopback port nothing listens on, so the dial is refused at once.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .expect("bind an ephemeral loopback port")
            .port();
        let mut config = thetadatadx::config::DirectConfig::production();
        config.set_streaming_hosts(vec![("127.0.0.1".to_string(), port)]);
        let service =
            thetadatadx::StreamService::new(&thetadatadx::Credentials::api_key("test"), config)
                .expect("a valid config");
        let handle = ThetaDataDxStreamHandle::new(std::sync::Arc::new(service));
        *handle.callback.lock_recover() = Some(FfiCallback {
            callback: noop,
            ctx: std::ptr::null_mut(),
        });
        let aapl = (SubscriptionKind::Quote, Contract::stock("AAPL"));
        *handle.saved_subscriptions.lock_recover() = Some((vec![aapl.clone()], Vec::new()));

        for attempt in 1..=2 {
            // SAFETY: `&handle` is a live, stack-pinned, never-freed handle
            // for the whole test.
            let rc = unsafe { thetadatadx_streaming_reconnect(&handle) };
            assert_eq!(rc, -1, "attempt {attempt}: the dial must fail");
            assert_eq!(
                handle
                    .saved_subscriptions
                    .lock_recover()
                    .as_ref()
                    .map(|saved| saved.0.clone()),
                Some(vec![aapl.clone()]),
                "attempt {attempt}: the saved subscription was dropped",
            );
        }
    }

    /// A shutdown drops the subscriptions, so a reconnect after it has nothing
    /// to replay: it must refuse with `THETADATADX_ERR_NETWORK`, naming the
    /// call that resumes, rather than dial a session with nothing subscribed.
    #[test]
    fn reconnect_after_shutdown_refuses_and_names_set_callback() {
        use super::{
            FfiCallback, LockRecover, ThetaDataDxStreamEvent, ThetaDataDxStreamHandle,
            thetadatadx_streaming_reconnect, thetadatadx_streaming_shutdown,
        };
        use std::os::raw::c_void;
        use thetadatadx::streaming::{Contract, SubscriptionKind};

        extern "C" fn noop(_event: *const ThetaDataDxStreamEvent, _ctx: *mut c_void) {}

        // A loopback port nothing listens on, so a dial would fail with a
        // connect error rather than the refusal under test.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .expect("bind an ephemeral loopback port")
            .port();
        let mut config = thetadatadx::config::DirectConfig::production();
        config.set_streaming_hosts(vec![("127.0.0.1".to_string(), port)]);
        let service =
            thetadatadx::StreamService::new(&thetadatadx::Credentials::api_key("test"), config)
                .expect("a valid config");
        let handle = ThetaDataDxStreamHandle::new(std::sync::Arc::new(service));
        *handle.callback.lock_recover() = Some(FfiCallback {
            callback: noop,
            ctx: std::ptr::null_mut(),
        });
        *handle.saved_subscriptions.lock_recover() = Some((
            vec![(SubscriptionKind::Quote, Contract::stock("AAPL"))],
            Vec::new(),
        ));

        // SAFETY: `&handle` is a live, stack-pinned, never-freed handle for the
        // whole test; `thetadatadx_last_error` returns a pointer valid until the
        // next error on this thread, read before any further call.
        unsafe {
            thetadatadx_streaming_shutdown(&handle);
            assert_eq!(thetadatadx_streaming_reconnect(&handle), -1);
            assert_eq!(
                crate::error::thetadatadx_last_error_code(),
                crate::error::THETADATADX_ERR_NETWORK
            );
            let message = std::ffi::CStr::from_ptr(crate::error::thetadatadx_last_error())
                .to_string_lossy()
                .into_owned();
            assert!(
                message.contains("thetadatadx_streaming_set_callback"),
                "{message}"
            );
        }
    }
}
