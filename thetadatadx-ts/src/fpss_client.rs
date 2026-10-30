//! Standalone TypeScript (napi-rs) `StreamingClient` — streaming only.
//!
//! Opens ONLY the streaming TLS transport, no market-data channel, no Nexus HTTP
//! authentication, no market-data / Treasury / Calendar surface. Mirrors
//! the Python `StreamingClient` (`thetadatadx-py/src/fpss_client.rs`), the C++
//! `thetadatadx::StreamingClient` (`thetadatadx-cpp/include/thetadatadx.hpp`), and the standalone
//! C ABI entry points (`thetadatadx_streaming_*` in `thetadatadx-ffi/src/streaming.rs`), letting a
//! Node.js caller run a streaming-only session alongside an externally
//! managed market-data process without the bundled
//! [`crate::Client`] preempting the parallel market-data work at the
//! Nexus session layer.
//!
//! The client is a [`crate::StreamView`] over a core
//! [`thetadatadx::StreamService`] it owns alone, so every method runs the body
//! `client.stream` runs, over the same core lifecycle. This module also holds
//! the `ThreadsafeFunction` delivery helpers that view uses.
//!
//! # Nexus session behaviour
//!
//! This client does NOT issue a Nexus authentication. The streaming service speaks its own
//! protocol-level `CREDENTIALS` handshake on the TLS connection itself; no
//! separate Nexus session is acquired. Run the bundled
//! [`crate::Client`] when you need the market-data surface and Nexus
//! session machinery side by side.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use thetadatadx::auth;
use thetadatadx::fpss;

use crate::fluent::Subscription;
use crate::{
    Config, Credentials, StreamEvent, StreamView, TsfnCallback, config_or_production, to_napi_err,
};

/// Wake first, mark second: `aborted.write()` parks behind the read guard a blocking `call` holds.
pub(crate) fn abort_waking_before_marking(
    already_marked: bool,
    wake: impl FnOnce(),
    mark: impl FnOnce(),
) {
    if !already_marked {
        wake();
    }
    mark();
}

/// Build the dispatcher teardown wake hook for a `ThreadsafeFunction`-backed
/// per-event callback path.
///
/// # The deadlock this resolves
///
/// The per-event dispatcher hands each event to the napi `ThreadsafeFunction`
/// with [`ThreadsafeFunctionCallMode::Blocking`] and a bounded call queue
/// ([`crate::STREAMING_CALLBACK_QUEUE_DEPTH`]). When the queue fills, the
/// dispatcher thread blocks INSIDE `call` waiting for the Node main thread to
/// drain it. A synchronous teardown (`stopStreaming`, or a drop that runs on the
/// JS thread) can run on that same Node main thread and join the dispatcher; the
/// main thread is therefore parked in the join and can never drain the queue, so
/// the blocked `call`
/// never returns, the dispatcher never reaches its shutdown exit, and the join
/// hangs forever. Dropping every `Arc<TsfnCallback>` would normally release the
/// function, but it cannot here: the blocked consumer is itself holding a clone,
/// so the strong count never reaches zero while it is stuck. The function must
/// be aborted explicitly.
///
/// # Mechanism
///
/// The returned hook clones the function's shared
/// [`ThreadsafeFunctionHandle`](napi::threadsafe_function::ThreadsafeFunctionHandle)
/// (the `handle` field is an `Arc` shared by every clone of the function,
/// including the one the blocked consumer holds) and performs the same abort
/// the framework's deprecated [`ThreadsafeFunction::abort`] performs, but in a
/// lock order that can wake a blocked caller: it releases the underlying napi
/// threadsafe function with
/// [`ThreadsafeFunctionReleaseMode::abort`](napi::sys::ThreadsafeFunctionReleaseMode).
/// N-API then makes a blocked `call(.., Blocking)` return
/// [`napi::Status::Closing`], dropping napi-rs's `aborted` read guard. Only
/// after that wake is sent does the hook take the write lock and mark the
/// shared `aborted` flag, so future calls reject immediately.
///
/// The replicated-abort form is used in preference to calling the deprecated
/// `abort()` because `abort()` consumes `self` by value, which is impossible
/// from a shared `Arc<TsfnCallback>` (the blocked consumer co-owns it). A
/// per-hook once flag makes a repeated wake attempt a harmless no-op even if
/// the hook is ever wrapped behind a replayable teardown adapter.
///
/// # When it runs
///
/// Teardown installs this, through [`abort_hook_expect_closing`], as the
/// session's `on_teardown`, and the core's teardown fires it only as a
/// FALLBACK, after the dispatcher fails to exit on its own within the grace
/// window. The abort is permanent (a later `call` returns
/// `Closing` forever) and `reconnect` re-uses the same function, so firing it on
/// every stop would break a reconnected session. The grace and the thread check
/// in [`abort_hook_expect_closing`] together fire it only when it is the sole
/// way to break a real deadlock.
pub(crate) fn abort_hook(callback: &Arc<TsfnCallback>) -> Box<dyn FnOnce() + Send> {
    // Clone the SHARED handle (an `Arc<ThreadsafeFunctionHandle>`). Every clone
    // of the threadsafe function — including the one the blocked consumer holds
    // — points at this same handle, so aborting through it aborts the call the
    // consumer is parked in.
    let handle = Arc::clone(&callback.handle);
    let fired = AtomicBool::new(false);
    Box::new(move || {
        if fired
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let already_aborted = handle.with_read_aborted(|aborted| aborted);
        abort_waking_before_marking(
            already_aborted,
            || {
                let raw = handle.get_raw();
                if !raw.is_null() {
                    // SAFETY: `raw` is the live `napi_threadsafe_function` pointer
                    // owned by `handle`; it is non-null here. Release happens
                    // BEFORE taking `aborted.write()`: napi-rs holds
                    // `aborted.read()` across `call(.., Blocking)`, so taking the
                    // writer first deadlocks behind the caller this release is
                    // meant to wake. N-API allows abort release while a call is
                    // blocked; that call returns `Closing` and drops the read guard.
                    let status = unsafe {
                        napi::sys::napi_release_threadsafe_function(
                            raw,
                            napi::sys::ThreadsafeFunctionReleaseMode::abort,
                        )
                    };
                    debug_assert_eq!(
                        status,
                        napi::sys::Status::napi_ok,
                        "napi_release_threadsafe_function(abort) failed",
                    );
                }
            },
            || {
                handle.with_write_aborted(|mut aborted| {
                    *aborted = true;
                });
            },
        );
    })
}

/// Wrap [`abort_hook`] so it aborts only when the teardown joins on
/// `js_thread`, the Node main thread that drains `callback`'s queue, and marks
/// the `Closing` statuses that abort produces as expected.
///
/// Only a join on that thread can keep the queue from draining. A join on any
/// other thread, such as the blocking worker `reconnect()` tears the old
/// session down on, leaves the main thread delivering the backlog, so the
/// dispatcher drains the ring and exits however long that takes. Aborting
/// there would discard every event still queued and leave `reconnect()` with a
/// closed callback it cannot re-register.
pub(crate) fn abort_hook_expect_closing(
    callback: &Arc<TsfnCallback>,
    closing_expected: Arc<AtomicBool>,
    js_thread: std::thread::ThreadId,
) -> Box<dyn FnOnce() + Send> {
    let hook = abort_hook(callback);
    Box::new(move || {
        if std::thread::current().id() != js_thread {
            return;
        }
        closing_expected.store(true, Ordering::Release);
        hook();
    })
}

pub(crate) fn ensure_callback_open(callback: &TsfnCallback) -> napi::Result<()> {
    if callback.handle.with_read_aborted(|aborted| aborted) {
        Err(aborted_callback_error())
    } else {
        Ok(())
    }
}

pub(crate) fn aborted_callback_error() -> napi::Error {
    napi::Error::from_reason(
        "[StreamError] streaming callback is closed; call startStreaming(callback) with a fresh function",
    )
}

pub(crate) fn note_callback_delivery_status(
    status: napi::Status,
    closing_expected: &AtomicBool,
    delivery_failed: &AtomicBool,
) {
    if status == napi::Status::Closing && !closing_expected.load(Ordering::Acquire) {
        delivery_failed.store(true, Ordering::Release);
    }
}

pub(crate) fn callback_delivery_outcome(
    outcome: fpss::PollOutcome,
    delivery_failed: &AtomicBool,
) -> fpss::PollOutcome {
    if delivery_failed.load(Ordering::Acquire) {
        fpss::PollOutcome::Failed
    } else {
        outcome
    }
}

/// Standalone streaming-only client.
///
/// Opens ONLY the streaming TLS transport, no historical data channel, no
/// Nexus HTTP authentication. Use when a parallel market-data process is
/// already running in the same environment and you need to stream
/// without the bundled `Client` taking over the Nexus session
/// at connect time.
///
/// ```ts
/// import { StreamingClient, Contract } from "thetadatadx-ts";
/// const streaming = StreamingClient.connectFromFile("creds.txt");
/// await streaming.startStreaming((event) => console.log(event.kind, event));
/// streaming.subscribe(Contract.stock("AAPL").quote());
/// // ... events arrive on the Node main thread ...
/// streaming.stopStreaming();
/// ```
#[napi]
pub struct StreamingClient {
    view: StreamView,
}

#[napi]
impl StreamingClient {
    // Lifecycle: intentionally hand-written. The connect factories validate
    // the config but do NOT open the streaming TLS connection: connection is
    // deferred to the first `startStreaming` call, matching the C ABI's
    // deferred-connect contract (`thetadatadx_streaming_connect` allocates the
    // handle, `thetadatadx_streaming_set_callback` opens the network) so the
    // same observable behaviour applies across every binding. No market-data
    // channel is opened and no Nexus request is issued by any factory.

    /// Allocate a standalone streaming handle with a `Credentials` handle.
    /// Streaming only — opens no historical data channel and issues no
    /// Nexus request. Pass an optional `Config` (`dev` / `stage` /
    /// `production`, plus any tuned streaming / reconnect setters) to override the
    /// production-default endpoint. The streaming TLS connection opens on the
    /// first `startStreaming` call.
    ///
    /// The config is snapshot at construction time: the `Config` handle
    /// may be reused or mutated afterward without affecting this client.
    #[napi(factory)]
    pub fn connect(creds: &Credentials, config: Option<&Config>) -> napi::Result<StreamingClient> {
        Self::from_config(&creds.inner, config)
    }

    /// Allocate a standalone streaming handle with a credentials file (line 1 =
    /// email, line 2 = password). Convenience wrapper over
    /// `Credentials.fromFile` + `connect`. Pass an optional `Config` to
    /// override the production-default endpoint.
    #[napi(factory, js_name = "connectFromFile")]
    pub fn connect_from_file(
        path: String,
        config: Option<&Config>,
    ) -> napi::Result<StreamingClient> {
        let creds = auth::Credentials::from_file(&path).map_err(to_napi_err)?;
        Self::from_config(&creds, config)
    }

    /// Start streaming and register a JS callback for incoming events.
    ///
    /// Opens the streaming connection and begins delivering events. Each typed
    /// streaming event is delivered to your `callback(event)` on the Node main
    /// thread, so the callback may use any JS API safely. Rust-side delivery
    /// panics are isolated and counted by `panicCount()`; a JavaScript
    /// exception follows Node's normal exception handling.
    ///
    /// Backpressure: a slow callback first fills a bounded delivery queue
    /// and then the event ring behind it, at which point the newest incoming
    /// events are dropped and counted by `droppedEventCount()` while
    /// `ringOccupancy()` reports the in-flight depth. Watch those two
    /// signals to detect a callback that cannot keep up. The receive path
    /// is never blocked by a slow callback, so the upstream connection
    /// stays healthy regardless of callback speed.
    #[napi(js_name = "startStreaming")]
    pub async fn start_streaming(
        &self,
        // The callback parameter is spelled with the inline
        // `ThreadsafeFunction<StreamEvent, …>` rather than the
        // `TsfnCallback` alias so napi-rs emits a typed
        // `(event: StreamEvent) => void` signature into `index.d.ts`. A bare
        // alias name would surface in the published types as an unresolved
        // identifier, leaving the callback parameter untyped for callers.
        callback: napi::threadsafe_function::ThreadsafeFunction<
            StreamEvent,
            (),
            StreamEvent,
            napi::Status,
            false,
            false,
            { crate::STREAMING_CALLBACK_QUEUE_DEPTH },
        >,
    ) -> napi::Result<()> {
        self.view.start_streaming(callback).await
    }

    /// Whether the streaming TLS connection is currently open. Returns `false`
    /// when the dispatcher thread has panicked — no events are arriving
    /// even though the TLS slot is still populated.
    #[napi(js_name = "isStreaming")]
    pub fn is_streaming(&self) -> bool {
        self.view.is_streaming()
    }

    /// Whether the streaming session is currently authenticated. Distinct from
    /// `isStreaming()`: the TLS slot can hold a client whose authenticated
    /// flag has flipped to `false` after a server disconnect, before the
    /// application has issued `reconnect()`. A panicked dispatcher also
    /// folds back to `false` here.
    #[napi(js_name = "isAuthenticated")]
    pub fn is_authenticated(&self) -> bool {
        self.view.is_authenticated()
    }

    /// Polymorphic subscribe — primary fluent entry point. Accepts the
    /// `Subscription` value returned by `Contract.quote()` /
    /// `Contract.trade()` / `Contract.openInterest()` (per-contract scope)
    /// or by `SecType.option().fullTrades()` /
    /// `SecType.option().fullOpenInterest()` (full-stream scope).
    #[napi]
    pub fn subscribe(&self, sub: &Subscription) -> napi::Result<()> {
        self.view.subscribe(sub)
    }

    /// Bulk-subscribe an array of `Subscription` values. Stops at the first
    /// error and returns it; previously-installed subscriptions are NOT
    /// rolled back.
    #[napi(js_name = "subscribeMany")]
    pub fn subscribe_many(&self, subs: Vec<&Subscription>) -> napi::Result<()> {
        self.view.subscribe_many(subs)
    }

    /// Polymorphic unsubscribe — fluent counterpart to `subscribe(sub)`.
    #[napi]
    pub fn unsubscribe(&self, sub: &Subscription) -> napi::Result<()> {
        self.view.unsubscribe(sub)
    }

    /// Bulk-unsubscribe an array of `Subscription` values.
    #[napi(js_name = "unsubscribeMany")]
    pub fn unsubscribe_many(&self, subs: Vec<&Subscription>) -> napi::Result<()> {
        self.view.unsubscribe_many(subs)
    }

    /// Snapshot of per-contract subscriptions on the live session as an
    /// array of `{ kind, contract }` objects (matching the unified
    /// client's `activeSubscriptions()` projection). Throws when
    /// streaming has not started.
    #[napi(js_name = "activeSubscriptions")]
    pub fn active_subscriptions(&self) -> napi::Result<serde_json::Value> {
        self.view.active_subscriptions()
    }

    /// Snapshot of full-stream subscriptions (e.g. `OPTION` /
    /// `full_trades`). Each entry has the same `{ kind, contract }` shape
    /// as the unified client's `activeFullSubscriptions()`, where `kind` is
    /// `"full_trades"` / `"full_open_interest"` and `contract` carries the
    /// wire-level security type. Quote is never a valid full-stream kind,
    /// so any such row is dropped. Throws when streaming has not started.
    #[napi(js_name = "activeFullSubscriptions")]
    pub fn active_full_subscriptions(&self) -> napi::Result<serde_json::Value> {
        self.view.active_full_subscriptions()
    }

    /// Count of streaming events the TLS reader could not publish into the
    /// event ring because the consumer fell behind, across every session this
    /// handle has run. A `reconnect()` rebuilds the inner client; the drops
    /// the previous session recorded stay in the total. Returned as `bigint`
    /// for the full 64-bit unsigned range.
    #[napi(js_name = "droppedEventCount")]
    pub fn dropped_event_count(&self) -> napi::bindgen_prelude::BigInt {
        self.view.dropped_event_count()
    }

    /// Point-in-time count of events published into the ring but not yet
    /// drained into your callback — the in-flight depth between the I/O
    /// thread and the dispatcher. The leading back-pressure signal: rises
    /// before `droppedEventCount()` moves. Returns `0n` when no session is
    /// live.
    #[napi(js_name = "ringOccupancy")]
    pub fn ring_occupancy(&self) -> napi::bindgen_prelude::BigInt {
        self.view.ring_occupancy()
    }

    /// Configured capacity of the event ring in slots (a power of two) —
    /// the fixed denominator for `ringOccupancy()`. Returns `0n` when no
    /// session is live.
    #[napi(js_name = "ringCapacity")]
    pub fn ring_capacity(&self) -> napi::bindgen_prelude::BigInt {
        self.view.ring_capacity()
    }

    /// Cumulative count of user-callback panics caught at the per-event
    /// isolation boundary. A panic is caught, recorded here, and does not
    /// stop event delivery. Returned as `bigint` for the full 64-bit unsigned range.
    #[napi(js_name = "panicCount")]
    pub fn panic_count(&self) -> napi::bindgen_prelude::BigInt {
        self.view.panic_count()
    }

    /// Milliseconds since the most recent inbound streaming frame of any
    /// kind (data tick, heartbeat, control), or `null` when no session is
    /// live or no frame has been received yet. The operator-facing
    /// staleness clock.
    #[napi(js_name = "millisSinceLastEvent")]
    pub fn millis_since_last_event(&self) -> Option<napi::bindgen_prelude::BigInt> {
        self.view.millis_since_last_event()
    }

    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// streaming frame of any kind. Returns `0n` when no session is live or
    /// no frame has been received yet.
    #[napi(js_name = "lastEventReceivedAtUnixNanos")]
    pub fn last_event_received_at_unix_nanos(&self) -> napi::bindgen_prelude::BigInt {
        self.view.last_event_received_at_unix_nanos()
    }

    /// Address (`host:port`) of the streaming server the current session is
    /// connected to, following the session across auto-reconnects. `null`
    /// when no session is live.
    #[napi(js_name = "lastConnectedAddr")]
    pub fn last_connected_addr(&self) -> Option<String> {
        self.view.last_connected_addr()
    }

    /// Stop streaming and clear the registered callback. Same
    /// explicit-handoff semantics as the unified client: to resume after
    /// this returns, call `startStreaming(callback)` again with a freshly
    /// bound function; `reconnect()` throws because no callback is held.
    #[napi(js_name = "stopStreaming")]
    pub fn stop_streaming(&self) {
        self.view.stop_streaming();
    }

    /// Alias for `stopStreaming`. Mirrors the unified client's split surface
    /// where `shutdown` is documented as the terminal stop — on the
    /// standalone client both names are equivalent.
    #[napi(js_name = "shutdown")]
    pub fn shutdown(&self) {
        self.view.shutdown();
    }

    /// Re-open the streaming connection and re-register the previously installed
    /// callback. Requires a prior `startStreaming(callback)`; throws
    /// otherwise.
    ///
    /// Saves the active per-contract and full-stream subscriptions against
    /// the old session, opens a fresh streaming connection under the previously
    /// installed callback, and re-applies the saved subscriptions through
    /// the core's paced replay engine. Per-subscription failures surface as
    /// a single error naming every contract that did not re-subscribe — the
    /// streaming session itself is already up at that point. A retry after a
    /// failed reconnect replays the subscriptions the failed attempt saved.
    #[napi]
    pub async fn reconnect(&self) -> napi::Result<()> {
        self.view.reconnect().await
    }

    /// Block until every superseded streaming session's event-ring consumer
    /// has finished firing the registered callback. Resolves `true` once
    /// all retired generations have drained, `false` on timeout. Polls at
    /// 1 ms cadence on a worker so the Node event loop stays free.
    #[napi(js_name = "awaitDrain")]
    pub async fn await_drain(&self, timeout_ms: f64) -> napi::Result<bool> {
        self.view.await_drain(timeout_ms).await
    }
}

impl StreamingClient {
    /// Assemble an idle handle over a core streaming service of its own. The
    /// streaming TLS connection is not opened until `startStreaming`. Called
    /// from the factories, which run on the Node main thread.
    fn from_config(creds: &auth::Credentials, config: Option<&Config>) -> napi::Result<Self> {
        // Validated here, as the unified client's connect does, so an
        // out-of-range value is refused naming its field instead of reaching
        // the reconnect driver.
        let service = thetadatadx::StreamService::new(creds, config_or_production(config)?)
            .map_err(to_napi_err)?;
        // Seed the process-global runtime from this client's runtime config
        // so `workerThreads` is honored when this is the first client in
        // the process, even though the streaming connection is opened lazily by
        // `startStreaming`. A runtime-build failure surfaces here as a typed
        // error rather than being deferred to the first `startStreaming`.
        crate::runtime_from_config(&service.config().runtime)?;
        Ok(Self {
            view: StreamView {
                stream: Arc::new(service),
                callback: Arc::new(Mutex::new(None)),
                js_thread: std::thread::current().id(),
            },
        })
    }
}

/// Clears a freshly reserved `startStreaming` callback slot on any non-success
/// exit -- the `?` error return AND a panic between reserving the slot and
/// completing the connect.
///
/// The generated `StreamView::startStreaming` reserves the slot before its
/// lock-released blocking connect (holding a `MutexGuard` across the `.await`
/// is not allowed) and must not clear it unconditionally afterward: a
/// concurrent `stopStreaming` + restart may have replaced the reservation with
/// a newer callback that must keep its registration, and clearing
/// unconditionally would strand a live session with no registration.
/// `Arc::ptr_eq` gives each start a unique identity, so this clears ONLY when
/// the slot still holds this reservation. Disarmed by [`Self::disarm`] once the
/// connect succeeds. Mirrors the Python binding's `CallbackReservation`.
pub(crate) struct CallbackReservation<'a> {
    slot: &'a Mutex<Option<Arc<TsfnCallback>>>,
    reserved: &'a Arc<TsfnCallback>,
    armed: bool,
}

impl<'a> CallbackReservation<'a> {
    pub(crate) fn armed(
        slot: &'a Mutex<Option<Arc<TsfnCallback>>>,
        reserved: &'a Arc<TsfnCallback>,
    ) -> Self {
        Self {
            slot,
            reserved,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CallbackReservation<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if slot
            .as_ref()
            .is_some_and(|cb| Arc::ptr_eq(cb, self.reserved))
        {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod callback_delivery_failure_tests {
    use super::*;

    #[test]
    fn unexpected_closing_marks_dispatcher_delivery_failed() {
        let closing_expected = AtomicBool::new(false);
        let delivery_failed = AtomicBool::new(false);

        note_callback_delivery_status(napi::Status::Closing, &closing_expected, &delivery_failed);

        assert!(delivery_failed.load(Ordering::Acquire));
        assert_eq!(
            callback_delivery_outcome(fpss::PollOutcome::Drained(1), &delivery_failed),
            fpss::PollOutcome::Failed
        );
    }

    #[test]
    fn teardown_expected_closing_does_not_mark_delivery_failed() {
        let closing_expected = AtomicBool::new(true);
        let delivery_failed = AtomicBool::new(false);

        note_callback_delivery_status(napi::Status::Closing, &closing_expected, &delivery_failed);

        assert!(!delivery_failed.load(Ordering::Acquire));
        assert_eq!(
            callback_delivery_outcome(fpss::PollOutcome::Shutdown, &delivery_failed),
            fpss::PollOutcome::Shutdown
        );
    }

    #[test]
    fn successful_callback_delivery_keeps_scope_outcome() {
        let closing_expected = AtomicBool::new(false);
        let delivery_failed = AtomicBool::new(false);

        note_callback_delivery_status(napi::Status::Ok, &closing_expected, &delivery_failed);

        assert!(!delivery_failed.load(Ordering::Acquire));
        assert_eq!(
            callback_delivery_outcome(fpss::PollOutcome::Drained(2), &delivery_failed),
            fpss::PollOutcome::Drained(2)
        );
    }
}

#[cfg(test)]
mod teardown_deadlock_tests {
    //! Deterministic watchdog for the bounded-queue teardown deadlock.
    //!
    //! The bug: the per-event dispatcher hands each event to a napi
    //! [`crate::TsfnCallback`] via a `Blocking`
    //! [`call`](napi::threadsafe_function::ThreadsafeFunction::call) against a
    //! bounded call queue. Once the queue fills, the dispatcher blocks INSIDE
    //! `call` waiting for the Node main thread to drain it. A synchronous
    //! teardown (`stopStreaming`, or a drop that runs on the JS thread) can run
    //! on that same Node main thread and JOIN the dispatcher, so the main
    //! thread is parked in the join and can never drain the queue: the blocked
    //! `call` never returns, the dispatcher never exits, and the join hangs
    //! forever.
    //!
    //! The fix gives the dispatcher an `on_teardown` wake hook
    //! ([`super::abort_hook`]) that aborts the threadsafe function so the
    //! in-flight `Blocking` `call` returns [`napi::Status::Closing`]; the
    //! consumer then resumes, observes the client shutdown, exits its loop, and
    //! the join completes. The core's teardown fires the hook only as a
    //! FALLBACK, after a grace window, so the (permanent, function-killing)
    //! abort does not fire on a session that `reconnect` will re-use.
    //!
    //! These tests fire the hook and join against a
    //! stand-in for the napi abort primitive — a real `ThreadsafeFunction`
    //! cannot be built off-thread without a napi `Env`, which the test runner
    //! lacks, so the stand-in mirrors the shape that matters:
    //! [`napi::threadsafe_function::ThreadsafeFunctionHandle`]'s bounded queue
    //! plus a shared `aborted` flag whose write path mirrors `abort_hook`
    //! exactly, and a `call` that returns `Closing` the instant `aborted` is
    //! set. A [`Barrier`] makes the consumer DETERMINISTICALLY blocked-in-`call`
    //! before teardown runs, and an [`AtomicBool`] + deadline loop is the
    //! watchdog. The pair below pins both properties: the blocked dispatcher
    //! is released and the join completes (the fix), and with NO hook the same
    //! scenario hangs (the harness genuinely reproduces the bug).

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier, Condvar, Mutex, RwLock};
    use std::time::{Duration, Instant};

    /// Stand-in for a bounded napi `ThreadsafeFunction`, faithful to the parts
    /// the deadlock and its fix turn on:
    ///
    /// * a bounded call queue: `call` in `Blocking` mode waits while the queue
    ///   is full instead of dropping (the production back-pressure contract);
    /// * a shared `aborted` flag behind an `RwLock<bool>`, exactly as
    ///   [`napi::threadsafe_function::ThreadsafeFunctionHandle`] holds it;
    /// * `call` returns the equivalent of [`napi::Status::Closing`] the moment
    ///   `aborted` is observed, matching the real `call`'s first action.
    struct BoundedFn {
        capacity: usize,
        /// Current queued depth; drained by [`Self::drain_one`].
        depth: Mutex<usize>,
        /// Signalled when depth drops (a slot frees) OR when abort is released.
        space: Condvar,
        /// Shared abort flag — the field [`super::abort_hook`] writes through.
        aborted: RwLock<bool>,
        /// Models `napi_release_threadsafe_function(..., abort)`: a lock-free
        /// wake that makes an in-flight blocking call return `Closing`.
        released: AtomicBool,
    }

    /// Outcome of a [`BoundedFn::call`] — mirrors the only two `Status` values
    /// the production path distinguishes (`Ok` vs `Closing`).
    #[derive(Debug, PartialEq, Eq)]
    enum CallStatus {
        Ok,
        Closing,
    }

    impl BoundedFn {
        fn new(capacity: usize) -> Arc<Self> {
            Arc::new(Self {
                capacity,
                depth: Mutex::new(0),
                space: Condvar::new(),
                aborted: RwLock::new(false),
                released: AtomicBool::new(false),
            })
        }

        /// Enqueue one call in `Blocking` mode. Returns `Closing` if the
        /// function is (or becomes) aborted, else blocks until a queue slot is
        /// free and returns `Ok`. This is the stand-in for
        /// `ThreadsafeFunction::call(.., Blocking)`.
        fn call_blocking(&self) -> CallStatus {
            // The real napi-rs `call` holds this read guard across the
            // potentially blocking `napi_call_threadsafe_function`.
            let aborted = self.aborted.read().unwrap();
            if *aborted {
                return CallStatus::Closing;
            }
            let mut depth = self.depth.lock().unwrap();
            loop {
                if *aborted || self.released.load(Ordering::Acquire) {
                    return CallStatus::Closing;
                }
                if *depth < self.capacity {
                    *depth += 1;
                    return CallStatus::Ok;
                }
                // Queue full: park until a slot frees or the function is
                // aborted. This is the exact wait the production dispatcher is
                // stuck in when teardown runs.
                depth = self.space.wait(depth).unwrap();
            }
        }

        /// Abort the function: wake, then mark. The ordering comes from
        /// [`super::abort_waking_before_marking`], which production
        /// `abort_hook` also calls, so the scenario below exercises the real
        /// sequence rather than a second copy of it that could agree with a
        /// broken original.
        fn abort(&self) {
            let already = self.released.swap(true, Ordering::AcqRel);
            super::abort_waking_before_marking(
                already,
                || {
                    let _depth = self.depth.lock().unwrap();
                    self.space.notify_all();
                },
                || {
                    let mut aborted = self.aborted.write().unwrap();
                    *aborted = true;
                },
            );
        }

        fn is_aborted(&self) -> bool {
            *self.aborted.read().unwrap()
        }
    }

    /// Run the deadlock scenario once and return whether the dispatcher
    /// join completed within the watchdog budget. `wire_hook = true` installs
    /// the abort hook (the fix); `wire_hook = false` passes `None` (the pre-fix
    /// behaviour) so the same harness proves the test actually catches the bug.
    fn join_completes_with_hook(wire_hook: bool) -> bool {
        // Capacity 1 so a single un-drained call wedges the queue.
        let func = BoundedFn::new(1);

        // The dispatcher blocks in its SECOND call (the first fills the queue;
        // nothing drains it, so the second parks) — deterministically reaching
        // the blocked-in-`call` state before teardown via this barrier.
        let blocked_in_call = Arc::new(Barrier::new(2));
        let shutdown = Arc::new(AtomicBool::new(false));

        let dispatcher = {
            let func = Arc::clone(&func);
            let blocked_in_call = Arc::clone(&blocked_in_call);
            let shutdown = Arc::clone(&shutdown);
            std::thread::Builder::new()
                .name("test-bounded-dispatcher".into())
                .spawn(move || {
                    // First call fills the single queue slot and returns Ok.
                    assert_eq!(func.call_blocking(), CallStatus::Ok);
                    // Announce that the next call is about to block, then make
                    // it. `for_each_scoped` loops delivering events; this models
                    // the delivery that wedges once the queue is full.
                    blocked_in_call.wait();
                    loop {
                        // The blocking delivery. Pre-fix this never returns
                        // (queue full, nothing draining). Post-fix the abort
                        // makes it return `Closing`.
                        if func.call_blocking() == CallStatus::Closing {
                            // Resumes on `Closing`, observes the
                            // already-signalled shutdown, and exits its loop —
                            // exactly as `for_each_scoped` returns on
                            // `PollOutcome::Shutdown` once the blocked `call`
                            // unblocks.
                            assert!(
                                shutdown.load(Ordering::Acquire),
                                "dispatcher unblocked before shutdown was signalled",
                            );
                            return;
                        }
                        if shutdown.load(Ordering::Acquire) {
                            return;
                        }
                    }
                })
                .expect("spawn dispatcher")
        };

        // Wait until the dispatcher is provably parked inside the blocking call.
        blocked_in_call.wait();
        // Give it a beat to actually enter `space.wait` after the barrier
        // rendezvous. Not required for correctness (the watchdog covers any
        // ordering), but it tightens the reproduction so the abort lands on a
        // truly-parked waiter.
        std::thread::sleep(Duration::from_millis(20));

        // Teardown: signal shutdown, fire the hook, then join. The hook is the
        // same shape as `abort_hook`: it aborts the function so the blocked
        // `call` returns `Closing`.
        let done = Arc::new(AtomicBool::new(false));
        let teardown = {
            let func = Arc::clone(&func);
            let shutdown = Arc::clone(&shutdown);
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                // Equivalent of `client.shutdown()` raising the ring shutdown
                // signal the dispatcher observes once it unblocks.
                shutdown.store(true, Ordering::Release);
                let hook: Option<Box<dyn FnOnce() + Send>> = if wire_hook {
                    let func = Arc::clone(&func);
                    Some(Box::new(move || func.abort()))
                } else {
                    None
                };
                if let Some(hook) = hook {
                    hook();
                }
                let _ = dispatcher.join();
                done.store(true, Ordering::Release);
            })
        };

        // Watchdog: the teardown (and thus the join) must finish within budget.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                // Leave the dispatcher/teardown threads detached; the process
                // exits at test-binary teardown. Returning `false` lets the
                // caller assert the watchdog fired.
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        teardown.join().expect("teardown thread join");
        // The fix path must leave the function observably aborted; the no-hook
        // path never reaches here (it deadlocks).
        assert!(func.is_aborted() == wire_hook);
        true
    }

    /// With the teardown wake hook wired (the fix), a dispatcher blocked in a
    /// full bounded-queue `Blocking` call is released and the join completes
    /// well within the watchdog budget.
    #[test]
    fn teardown_join_completes_when_wake_hook_aborts_the_blocked_call() {
        assert!(
            join_completes_with_hook(true),
            "teardown deadlocked even WITH the wake hook: a dispatcher blocked \
             in a full bounded callback queue was not released before the join",
        );
    }

    /// Guard that the test above actually catches the bug: with the hook
    /// neutered (the pre-fix `on_teardown: None` behaviour), the very same
    /// scenario hangs and the watchdog fires. A deadlock test that passes
    /// without the fix is worthless, so this pins that the harness fails
    /// pre-fix.
    #[test]
    fn teardown_join_hangs_without_the_wake_hook() {
        assert!(
            !join_completes_with_hook(false),
            "teardown completed WITHOUT a wake hook — the harness does not \
             actually reproduce the bounded-queue join deadlock, so the \
             companion test proves nothing",
        );
    }

    #[test]
    fn aborted_function_rejects_blocking_calls_immediately() {
        let func = BoundedFn::new(1);
        // Fill the single slot so a further call would otherwise block forever.
        assert_eq!(func.call_blocking(), CallStatus::Ok);
        // Abort (what the wake hook does).
        func.abort();
        // The would-be-blocking call now returns Closing at once.
        let start = Instant::now();
        assert_eq!(func.call_blocking(), CallStatus::Closing);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "an aborted function must reject a Blocking call immediately, not \
             block on the full queue",
        );
    }
}
