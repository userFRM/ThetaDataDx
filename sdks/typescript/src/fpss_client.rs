//! Standalone TypeScript (napi-rs) `StreamingClient` — FPSS streaming only.
//!
//! Opens ONLY the FPSS TLS transport — no MDDS channel, no Nexus HTTP
//! authentication, no historical / Treasury / Calendar surface. Mirrors
//! the Python `StreamingClient` (`sdks/python/src/fpss_client.rs`), the C++
//! `thetadatadx::StreamingClient` (`sdks/cpp/include/thetadatadx.hpp`), and the standalone
//! C ABI entry points (`thetadatadx_fpss_*` in `ffi/src/streaming.rs`), letting a
//! Node.js caller run an FPSS-only session alongside an externally
//! managed MDDS process without the bundled
//! [`crate::Client`] preempting the parallel MDDS work at the
//! Nexus session layer.
//!
//! # Why a hand-written module
//!
//! The unified [`crate::Client`] drives the high-level
//! `thetadatadx::Client::start_streaming` convenience, which
//! owns its own dispatcher thread and `DispatcherSession`. The standalone
//! client wraps `thetadatadx::fpss::StreamingClient` directly — the lower-level
//! FPSS primitive that exposes `for_each_scoped` / `subscribe` / `shutdown`
//! but no dispatcher management — so this module spins the dispatcher
//! thread itself, exactly as the Python and C ABI standalone clients do.
//! The event-delivery path is the same `ThreadsafeFunction` mechanism the
//! unified TS streaming uses: the dispatcher thread converts each event to
//! the typed napi object and routes it onto the Node main thread.
//!
//! # Nexus session behaviour
//!
//! This client does NOT issue a Nexus authentication. FPSS speaks its own
//! protocol-level `CREDENTIALS` handshake on the TLS connection itself; no
//! separate Nexus session is acquired. Run the bundled
//! [`crate::Client`] when you need the MDDS surface and Nexus
//! session machinery side by side.
//!
//! # Lifecycle
//!
//! 1. `StreamingClient.connect(...)` / `connectFromFile(...)` — snapshots the
//!    connect parameters. The FPSS TLS connection is opened lazily by
//!    `startStreaming` (matching the FFI's deferred-connect contract).
//! 2. `startStreaming(callback)` — opens the FPSS TLS connection and
//!    starts the background dispatcher driving the ring iterator.
//! 3. `subscribe(...)` / `unsubscribe(...)` — fluent subscription.
//! 4. `stopStreaming()` / `shutdown()` — atomic stop with drain barrier.
//! 5. `reconnect()` — re-open under the same callback.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use thetadatadx::auth::{self, Credentials as RustCredentials};
use thetadatadx::config::DirectConfig;
use thetadatadx::fpss::protocol::SubscriptionKind;
use thetadatadx::fpss::{self, StreamingClient as RustStreamingClient};
use thetadatadx::DispatcherSession;

use crate::fluent::Subscription;
use crate::{
    buffered_event_to_typed, config_or_production, fpss_event_to_buffered, runtime, to_napi_err,
    Config, Credentials, StreamEvent, TsfnCallback,
};

/// Snapshot of the parameters required to open an FPSS TLS connection.
///
/// Cloned out of the user's `Config` at construction time so subsequent
/// mutations of the `Config` handle cannot retroactively change reconnect
/// behaviour for an already-running session — the same snapshot semantics
/// the Python binding (`FpssParams`) and the FFI
/// (`ffi/src/streaming.rs::StreamingConnectParams`) use.
///
/// The whole [`StreamingConfig`] and [`ReconnectConfig`] are snapshotted
/// wholesale rather than copied field by field, so a new tuning knob
/// added to either config cannot drift out of the standalone connect
/// path the way a hand-maintained subset did.
///
/// [`StreamingConfig`]: thetadatadx::config::StreamingConfig
/// [`ReconnectConfig`]: thetadatadx::config::ReconnectConfig
#[derive(Clone)]
struct FpssParams {
    creds: RustCredentials,
    streaming: thetadatadx::config::StreamingConfig,
    reconnect: thetadatadx::config::ReconnectConfig,
}

impl FpssParams {
    fn from_config(creds: &RustCredentials, config: &DirectConfig) -> Self {
        Self {
            creds: creds.clone(),
            streaming: config.streaming.clone(),
            reconnect: config.reconnect.clone(),
        }
    }

    /// Thread every connection-side knob from the snapshot into a
    /// [`fpss::StreamingClientBuilder`]. Kept in lockstep with the
    /// unified client's connect path (`crates/thetadatadx/src/client.rs`)
    /// and the C ABI (`ffi/src/streaming.rs::streaming_builder`) so the
    /// standalone client honours the full streaming and reconnect surface.
    fn builder(&self) -> fpss::StreamingClientBuilder<'_> {
        fpss::StreamingClientBuilder::new(&self.creds, self.streaming.hosts())
            .ring_size(self.streaming.ring_size)
            .flush_mode(self.streaming.flush_mode)
            .wait_strategy(self.streaming.wait_strategy)
            .wait_strategy_tuning(
                self.streaming.wait_spin_iters,
                self.streaming.wait_yield_iters,
                self.streaming.wait_park_us,
            )
            .consumer_cpu(self.streaming.consumer_cpu)
            .reconnect_policy(self.reconnect.policy.clone())
            .reconnect_wait_ms(self.reconnect.wait_ms)
            .reconnect_wait_max_ms(self.reconnect.wait_max_ms)
            .reconnect_wait_rate_limited_ms(self.reconnect.wait_rate_limited_ms)
            .reconnect_wait_server_restart_ms(self.reconnect.wait_server_restart_ms)
            .reconnect_jitter(self.reconnect.jitter)
            .reconnect_replay_burst_size(self.reconnect.replay_burst_size)
            .reconnect_replay_pace_ms(self.reconnect.replay_pace_ms)
            .derive_ohlcvc(self.streaming.derive_ohlcvc)
            .connect_timeout_ms(self.streaming.connect_timeout_ms)
            .read_timeout_ms(self.streaming.timeout_ms)
            .ping_interval_ms(self.streaming.ping_interval_ms)
            .io_read_slice_ms(self.streaming.io_read_slice_ms)
            .data_watchdog_ms(self.streaming.data_watchdog_ms)
            .keepalive_idle_secs(self.streaming.keepalive_idle_secs)
            .keepalive_interval_secs(self.streaming.keepalive_interval_secs)
            .keepalive_retries(self.streaming.keepalive_retries)
            .host_selection(self.streaming.host_selection)
            .host_shuffle_seed(self.streaming.host_shuffle_seed)
    }
}

/// Build the snapshot from an owned [`DirectConfig`], rejecting a config
/// with no FPSS hosts before any TLS work begins. Mirrors the Python
/// `StreamingClient.__new__` empty-hosts guard.
fn params_from_direct(creds: &RustCredentials, direct: &DirectConfig) -> napi::Result<FpssParams> {
    if direct.streaming_hosts().is_empty() {
        return Err(crate::invalid_parameter_err(
            "StreamingClient: config.streaming.hosts is empty (use Config.production() or set the streaming hosts)",
        ));
    }
    Ok(FpssParams::from_config(creds, direct))
}

/// Standalone FPSS-only streaming client.
///
/// Opens ONLY the FPSS TLS transport — no historical data channel, no
/// Nexus HTTP authentication. Use when a parallel historical process is
/// already running in the same environment and you need to stream FPSS
/// without the bundled `Client` taking over the Nexus session
/// at connect time.
///
/// ```ts
/// import { StreamingClient, Contract } from "thetadatadx";
/// const fpss = StreamingClient.connectFromFile("creds.txt");
/// await fpss.startStreaming((event) => console.log(event.kind, event));
/// fpss.subscribe(Contract.stock("AAPL").quote());
/// // ... events arrive on the Node main thread ...
/// fpss.stopStreaming();
/// ```
#[napi]
pub struct StreamingClient {
    /// Connect parameters captured at construction time. Reused on every
    /// `startStreaming` / `reconnect`.
    params: FpssParams,
    /// Currently-open inner FPSS client. `None` between construction and
    /// `startStreaming`, and after `stopStreaming` / `shutdown`.
    inner: Mutex<Option<Arc<RustStreamingClient>>>,
    /// Most recently registered JS callback, behind an `Arc` so the
    /// dispatcher closure can hold its own ref-counted clone. Retained
    /// across `startStreaming` so `reconnect()` can re-register the same
    /// handler without the caller passing it again; cleared on
    /// `stopStreaming` / `shutdown` so a teardown the application has
    /// already observed releases the napi reference back to V8 — the same
    /// explicit-handoff model as the unified [`crate::Client`].
    callback: Mutex<Option<Arc<TsfnCallback>>>,
    /// Quiescence flags of every superseded streaming session that has not
    /// yet drained. Mirrors the unified client's `prev_drained` field:
    /// stacked stop/start cycles can layer multiple in-flight ring
    /// consumers, and `awaitDrain` waits for all of them.
    prev_drained: Mutex<Vec<Arc<AtomicBool>>>,
    /// Dispatcher thread lifecycle. Panic state is derived from
    /// `JoinHandle::join()` returning `Err(_)`.
    dispatcher: Mutex<DispatcherSession>,
}

impl Drop for StreamingClient {
    /// Signal shutdown and join the dispatcher thread so a callback in
    /// flight does not race destruction. Unlike the Python binding there
    /// is no GIL to release here: the dispatcher hands events to a
    /// `ThreadsafeFunction` (which queues onto the Node main thread) and
    /// never blocks on a Rust lock the destructor holds.
    fn drop(&mut self) {
        let taken_client = self.inner.lock().unwrap_or_else(|e| e.into_inner()).take();
        let prev_session = std::mem::replace(
            &mut *self.dispatcher.lock().unwrap_or_else(|e| e.into_inner()),
            DispatcherSession::Idle,
        );
        if let Some(ref client) = taken_client {
            client.shutdown();
        }
        drop(taken_client);
        if let DispatcherSession::Running { handle } = prev_session {
            if handle.thread().id() != std::thread::current().id() {
                let _ = handle.join();
            }
        }
    }
}

impl StreamingClient {
    fn lock_inner(&self) -> MutexGuard<'_, Option<Arc<RustStreamingClient>>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_callback(&self) -> MutexGuard<'_, Option<Arc<TsfnCallback>>> {
        self.callback.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_dispatcher(&self) -> MutexGuard<'_, DispatcherSession> {
        self.dispatcher.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run a closure with a borrow of the live FPSS client, rejecting with
    /// a typed napi error when nothing is connected.
    fn with_live<R>(
        &self,
        f: impl FnOnce(&RustStreamingClient) -> Result<R, thetadatadx::Error>,
    ) -> napi::Result<R> {
        let guard = self.lock_inner();
        let client = guard.as_ref().ok_or_else(|| {
            napi::Error::from_reason("streaming not started -- call startStreaming(callback) first")
        })?;
        f(client).map_err(to_napi_err)
    }

    /// Open the FPSS TLS connection under `callback` and spawn the
    /// dispatcher thread. Shared by `startStreaming` and `reconnect`.
    ///
    /// The TLS connect and the protocol `CREDENTIALS` handshake are
    /// network-bound and run synchronously inside `builder().build()`. That
    /// work is moved onto a blocking worker via `spawn_blocking` so the
    /// single libuv thread is never frozen for the handshake. The callback
    /// slot is reserved before the handshake (and released on failure) so
    /// the double-registration check stays correct across the `.await`,
    /// where two `startStreaming` calls could otherwise both pass it while
    /// the first is still connecting.
    ///
    /// Lock ordering: `callback` BEFORE `inner`, matching `stopStreaming`.
    async fn start_with_callback(&self, callback: Arc<TsfnCallback>) -> napi::Result<()> {
        {
            let mut cb_guard = self.lock_callback();
            if cb_guard.is_some() || self.lock_inner().is_some() {
                return Err(napi::Error::from_reason(
                    "streaming already started -- call stopStreaming() before startStreaming() again",
                ));
            }
            // Reserve the slot so a concurrent call is rejected while the
            // handshake below is in flight.
            *cb_guard = Some(Arc::clone(&callback));
        }

        let dispatch_cb = Arc::clone(&callback);

        let params = self.params.clone();
        let join_result = runtime()
            .spawn_blocking(move || params.builder().build())
            .await;
        let build_result = match join_result {
            Ok(build_result) => build_result,
            Err(e) => {
                // The connect task itself panicked. Release the slot
                // reserved above, mirroring the handshake-failure path
                // below, so the handle returns to a usable state and a
                // later startStreaming retry sees a clean registration
                // instead of a stuck "streaming already started".
                *self.lock_callback() = None;
                return Err(napi::Error::from_reason(format!(
                    "start_streaming task panicked: {e}"
                )));
            }
        };
        let client = match build_result {
            Ok(client) => client,
            Err(e) => {
                // Release the slot reserved above so a later retry sees a
                // clean registration.
                *self.lock_callback() = None;
                return Err(to_napi_err(thetadatadx::Error::from(e)));
            }
        };
        let client_arc = Arc::new(client);

        // Publish the client BEFORE spawning the dispatcher so the first
        // delivered event sees a fully initialised handle. The callback
        // slot was already reserved above, so a re-entrant call from inside
        // the user callback to `subscribe()` / `isStreaming()` observes a
        // populated registration. `drop(callback)` releases this scope's
        // last owning handle; the reserved slot and `dispatch_cb` keep the
        // ref-count alive.
        *self.lock_inner() = Some(Arc::clone(&client_arc));
        drop(callback);

        let dispatcher_client = Arc::clone(&client_arc);
        let dispatcher = std::thread::Builder::new()
            .name("thetadatadx-ts-fpss-dispatcher".into())
            .spawn(move || {
                // `for_each_scoped` drives `poll_batch`, which wraps each
                // callback invocation in its own `catch_unwind`; a panic in
                // the per-event machinery here is caught by the outer guard
                // and recorded as `Failed`. There is no GIL to bracket, so
                // the scope is the identity closure — the wait between
                // batches happens outside it as usual.
                // A panic escaping the event-iteration machinery (NOT a
                // user-callback panic — those are caught per-invocation
                // inside `poll_batch`) ends the thread. `stopStreaming`
                // observes it through `JoinHandle::join()` returning
                // `Err(_)` and records `DispatcherSession::Failed`, which
                // folds `isStreaming()` / `isAuthenticated()` back to
                // `false`; that state is the observable signal, so no
                // logging dependency is pulled into this binding crate.
                let _outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    dispatcher_client.for_each_scoped(
                        |event: &fpss::StreamEvent| {
                            // Convert the borrowed event to the typed napi
                            // object on the dispatcher thread, then hand it
                            // to the `ThreadsafeFunction`, which routes the
                            // call onto the Node main thread (the only
                            // thread allowed to execute V8). `Blocking`
                            // applies bounded back-pressure when the tsfn
                            // queue is full instead of dropping the event;
                            // the FPSS TLS reader is never blocked, so a
                            // slow JS callback at most fills the ring and
                            // bumps `droppedEventCount()`.
                            let buffered = fpss_event_to_buffered(event);
                            let typed = buffered_event_to_typed(buffered);
                            dispatch_cb.call(
                                typed,
                                napi::threadsafe_function::ThreadsafeFunctionCallMode::Blocking,
                            );
                        },
                        |drain| drain(),
                    );
                }));
            });
        match dispatcher {
            Ok(h) => {
                *self.lock_dispatcher() = DispatcherSession::Running { handle: h };
                Ok(())
            }
            Err(e) => {
                let taken = self.lock_inner().take();
                *self.lock_callback() = None;
                if let Some(client) = taken {
                    client.shutdown();
                }
                Err(napi::Error::from_reason(format!(
                    "failed to spawn FPSS dispatcher thread: {e}"
                )))
            }
        }
    }
}

#[napi]
impl StreamingClient {
    // Lifecycle: intentionally hand-written. The connect factories snapshot
    // the connect parameters but do NOT open the FPSS TLS connection —
    // connection is deferred to the first `startStreaming` call, matching
    // the C ABI's deferred-connect contract (`thetadatadx_fpss_connect` allocates
    // the handle, `thetadatadx_fpss_set_callback` opens the network) so the same
    // observable behaviour applies across every binding. No MDDS channel is
    // opened and no Nexus request is issued by any factory.

    /// Allocate a standalone FPSS handle with a `Credentials` handle.
    /// Streaming only — opens no historical data channel and issues no
    /// Nexus request. Pass an optional `Config` (`dev` / `stage` /
    /// `production`, plus any tuned FPSS / reconnect setters) to override the
    /// production-default endpoint. The FPSS TLS connection opens on the
    /// first `startStreaming` call.
    ///
    /// The config is snapshot at construction time: the `Config` handle
    /// may be reused or mutated afterward without affecting this client.
    #[napi(factory)]
    pub fn connect(creds: &Credentials, config: Option<&Config>) -> napi::Result<StreamingClient> {
        let direct = config_or_production(config);
        // Seed the process-global runtime from this client's runtime config
        // so `workerThreads` is honored when this is the first client in
        // the process, even though the FPSS connection is opened lazily by
        // `startStreaming`.
        crate::runtime_from_config(&direct.runtime);
        let params = params_from_direct(&creds.inner, &direct)?;
        Ok(StreamingClient::from_params(params))
    }

    /// Allocate a standalone FPSS handle with a credentials file (line 1 =
    /// email, line 2 = password). Convenience wrapper over
    /// `Credentials.fromFile` + `connect`. Pass an optional `Config` to
    /// override the production-default endpoint.
    #[napi(factory, js_name = "connectFromFile")]
    pub fn connect_from_file(
        path: String,
        config: Option<&Config>,
    ) -> napi::Result<StreamingClient> {
        let creds = auth::Credentials::from_file(&path).map_err(to_napi_err)?;
        let direct = config_or_production(config);
        // Seed the process-global runtime from this client's runtime config
        // so `workerThreads` is honored when this is the first client in
        // the process, even though the FPSS connection is opened lazily by
        // `startStreaming`.
        crate::runtime_from_config(&direct.runtime);
        let params = params_from_direct(&creds, &direct)?;
        Ok(StreamingClient::from_params(params))
    }

    /// Start FPSS streaming and register a JS callback for incoming events.
    ///
    /// Opens the FPSS connection and begins delivering events. Each typed
    /// FPSS event is delivered to your `callback(event)` on the Node main
    /// thread, so the callback may use any JS API safely. A callback that
    /// panics or throws is isolated and does not interrupt the stream.
    ///
    /// Backpressure: a slow callback causes incoming events to queue and,
    /// once the buffer is full, newly arriving events are dropped, observable
    /// via `droppedEventCount()`. The receive path is never blocked by a
    /// slow callback, so the upstream connection stays healthy regardless
    /// of callback speed.
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
        >,
    ) -> napi::Result<()> {
        self.start_with_callback(Arc::new(callback)).await
    }

    /// Whether the FPSS TLS connection is currently open. Returns `false`
    /// when the dispatcher thread has panicked — no events are arriving
    /// even though the TLS slot is still populated.
    #[napi(js_name = "isStreaming")]
    pub fn is_streaming(&self) -> bool {
        let guard = self.lock_inner();
        if guard.as_ref().is_none() {
            return false;
        }
        !matches!(*self.lock_dispatcher(), DispatcherSession::Failed { .. })
    }

    /// Whether the FPSS session is currently authenticated. Distinct from
    /// `isStreaming()`: the TLS slot can hold a client whose authenticated
    /// flag has flipped to `false` after a server disconnect, before the
    /// application has issued `reconnect()`. A panicked dispatcher also
    /// folds back to `false` here.
    #[napi(js_name = "isAuthenticated")]
    pub fn is_authenticated(&self) -> bool {
        let guard = self.lock_inner();
        let Some(client) = guard.as_ref() else {
            return false;
        };
        let dispatcher_failed = matches!(*self.lock_dispatcher(), DispatcherSession::Failed { .. });
        client.is_authenticated() && !dispatcher_failed
    }

    /// Polymorphic subscribe — primary fluent entry point. Accepts the
    /// `Subscription` value returned by `Contract.quote()` /
    /// `Contract.trade()` / `Contract.openInterest()` (per-contract scope)
    /// or by `SecType.option().fullTrades()` /
    /// `SecType.option().fullOpenInterest()` (full-stream scope).
    #[napi]
    pub fn subscribe(&self, sub: &Subscription) -> napi::Result<()> {
        let inner = sub.snapshot();
        self.with_live(|c| c.subscribe(inner))
    }

    /// Bulk-subscribe an array of `Subscription` values. Stops at the first
    /// error and returns it; previously-installed subscriptions are NOT
    /// rolled back.
    #[napi(js_name = "subscribeMany")]
    pub fn subscribe_many(&self, subs: Vec<&Subscription>) -> napi::Result<()> {
        let snaps: Vec<_> = subs.iter().map(|s| s.snapshot()).collect();
        for snap in snaps {
            self.with_live(|c| c.subscribe(snap))?;
        }
        Ok(())
    }

    /// Polymorphic unsubscribe — fluent counterpart to `subscribe(sub)`.
    #[napi]
    pub fn unsubscribe(&self, sub: &Subscription) -> napi::Result<()> {
        let inner = sub.snapshot();
        self.with_live(|c| c.unsubscribe(inner))
    }

    /// Bulk-unsubscribe an array of `Subscription` values.
    #[napi(js_name = "unsubscribeMany")]
    pub fn unsubscribe_many(&self, subs: Vec<&Subscription>) -> napi::Result<()> {
        let snaps: Vec<_> = subs.iter().map(|s| s.snapshot()).collect();
        for snap in snaps {
            self.with_live(|c| c.unsubscribe(snap))?;
        }
        Ok(())
    }

    /// Snapshot of per-contract subscriptions on the live session as an
    /// array of `{ kind, contract }` objects (matching the unified
    /// client's `activeSubscriptions()` projection). Empty array when
    /// streaming has not started.
    #[napi(js_name = "activeSubscriptions")]
    pub fn active_subscriptions(&self) -> napi::Result<serde_json::Value> {
        let guard = self.lock_inner();
        let Some(client) = guard.as_ref() else {
            return Ok(serde_json::json!([]));
        };
        Ok(serde_json::json!(client
            .active_subscriptions()
            .into_iter()
            .map(|(kind, contract)| {
                serde_json::json!({ "kind": format!("{kind:?}"), "contract": format!("{contract}") })
            })
            .collect::<Vec<_>>()))
    }

    /// Snapshot of full-stream subscriptions (e.g. `OPTION` /
    /// `full_trades`). Each entry has the same `{ kind, contract }` shape
    /// as the unified client's `activeFullSubscriptions()`, where `kind` is
    /// `"full_trades"` / `"full_open_interest"` and `contract` carries the
    /// wire-level security type. Quote is never a valid full-stream kind,
    /// so any such row is dropped. Empty array when streaming has not
    /// started.
    #[napi(js_name = "activeFullSubscriptions")]
    pub fn active_full_subscriptions(&self) -> napi::Result<serde_json::Value> {
        let guard = self.lock_inner();
        let Some(client) = guard.as_ref() else {
            return Ok(serde_json::json!([]));
        };
        Ok(serde_json::json!(client
            .active_full_subscriptions()
            .into_iter()
            .filter_map(|(kind, sec_type)| {
                let kind_str = match kind {
                    SubscriptionKind::Trade => "full_trades",
                    SubscriptionKind::OpenInterest => "full_open_interest",
                    SubscriptionKind::Quote => return None,
                    _ => return None,
                };
                Some(serde_json::json!({
                    "kind": kind_str,
                    "contract": format!("{sec_type:?}"),
                }))
            })
            .collect::<Vec<_>>()))
    }

    /// Cumulative count of FPSS events the TLS reader could not publish into
    /// the event ring because the consumer fell behind. Snapshot the value
    /// BEFORE `reconnect()` if you need to accumulate drops across session
    /// boundaries — `reconnect` rebuilds the inner client and the counter
    /// resets. Returned as `bigint` for the full 64-bit unsigned range.
    #[napi(js_name = "droppedEventCount")]
    pub fn dropped_event_count(&self) -> napi::bindgen_prelude::BigInt {
        let guard = self.lock_inner();
        napi::bindgen_prelude::BigInt::from(guard.as_ref().map_or(0, |c| c.dropped_count()))
    }

    /// Point-in-time count of events published into the ring but not yet
    /// drained into your callback — the in-flight depth between the I/O
    /// thread and the dispatcher. The leading back-pressure signal: rises
    /// before `droppedEventCount()` moves. Returns `0n` when no session is
    /// live.
    #[napi(js_name = "ringOccupancy")]
    pub fn ring_occupancy(&self) -> napi::bindgen_prelude::BigInt {
        let guard = self.lock_inner();
        napi::bindgen_prelude::BigInt::from(guard.as_ref().map_or(0, |c| c.ring_occupancy()) as u64)
    }

    /// Configured capacity of the event ring in slots (a power of two) —
    /// the fixed denominator for `ringOccupancy()`. Returns `0n` when no
    /// session is live.
    #[napi(js_name = "ringCapacity")]
    pub fn ring_capacity(&self) -> napi::bindgen_prelude::BigInt {
        let guard = self.lock_inner();
        napi::bindgen_prelude::BigInt::from(guard.as_ref().map_or(0, |c| c.ring_capacity()) as u64)
    }

    /// Cumulative count of user-callback panics caught at the per-event
    /// isolation boundary. A panic is caught, recorded here, and does not
    /// stop event delivery. Returned as `bigint` for the full 64-bit unsigned range.
    #[napi(js_name = "panicCount")]
    pub fn panic_count(&self) -> napi::bindgen_prelude::BigInt {
        let guard = self.lock_inner();
        napi::bindgen_prelude::BigInt::from(guard.as_ref().map_or(0, |c| c.panic_count()))
    }

    /// Set the slow-callback wall-clock threshold in microseconds. When a
    /// callback invocation runs longer than `thresholdUs`,
    /// `slowCallbackCount()` increments and a rate-limited warning is
    /// logged. Pass `0n` to disable the watchdog (the default).
    /// Observability only: the watchdog never cancels the callback. No-op
    /// when no session is live. Accepts `bigint` for the full 64-bit
    /// unsigned range.
    #[napi(js_name = "setSlowCallbackThresholdUs")]
    pub fn set_slow_callback_threshold_us(
        &self,
        threshold_us: napi::bindgen_prelude::BigInt,
    ) -> napi::Result<()> {
        let (_signed, value, _lossless) = threshold_us.get_u64();
        let guard = self.lock_inner();
        if let Some(c) = guard.as_ref() {
            c.set_slow_callback_threshold(std::time::Duration::from_micros(value));
        }
        Ok(())
    }

    /// Cumulative count of user-callback invocations whose wall-clock
    /// duration exceeded the threshold set by `setSlowCallbackThresholdUs()`.
    /// Returns `0n` when the watchdog is disabled or no session is live.
    /// Returned as `bigint` for the full 64-bit unsigned range.
    #[napi(js_name = "slowCallbackCount")]
    pub fn slow_callback_count(&self) -> napi::bindgen_prelude::BigInt {
        let guard = self.lock_inner();
        napi::bindgen_prelude::BigInt::from(guard.as_ref().map_or(0, |c| c.slow_callback_count()))
    }

    /// Milliseconds since the most recent inbound streaming frame of any
    /// kind (data tick, heartbeat, control), or `null` when no session is
    /// live or no frame has been received yet. The operator-facing
    /// staleness clock.
    #[napi(js_name = "millisSinceLastEvent")]
    pub fn millis_since_last_event(&self) -> Option<napi::bindgen_prelude::BigInt> {
        let guard = self.lock_inner();
        guard
            .as_ref()
            .and_then(|c| c.millis_since_last_event())
            .map(napi::bindgen_prelude::BigInt::from)
    }

    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// streaming frame of any kind. Returns `0n` when no session is live or
    /// no frame has been received yet.
    #[napi(js_name = "lastEventReceivedAtUnixNanos")]
    pub fn last_event_received_at_unix_nanos(&self) -> napi::bindgen_prelude::BigInt {
        let guard = self.lock_inner();
        napi::bindgen_prelude::BigInt::from(
            guard
                .as_ref()
                .map_or(0, |c| c.last_event_received_at_unix_nanos()),
        )
    }

    /// Address (`host:port`) of the streaming server the current session is
    /// connected to, following the session across auto-reconnects. `null`
    /// when no session is live.
    #[napi(js_name = "lastConnectedAddr")]
    pub fn last_connected_addr(&self) -> Option<String> {
        let guard = self.lock_inner();
        guard.as_ref().map(|c| c.last_connected_addr())
    }

    /// Stop streaming and clear the registered callback. Same
    /// explicit-handoff semantics as the unified client: to resume after
    /// this returns, call `startStreaming(callback)` again with a freshly
    /// bound function; `reconnect()` throws because no callback is held.
    ///
    /// Lock ordering: `callback` BEFORE `inner`, matching `startStreaming`.
    #[napi(js_name = "stopStreaming")]
    pub fn stop_streaming(&self) {
        // Take the client and stored callback out under the binding mutexes,
        // then release both before signalling shutdown so a dispatcher
        // re-entering any method via the callback never sees a lock held.
        let (taken_client, prev_session) = {
            let mut cb_guard = self.lock_callback();
            let taken = self.lock_inner().take();
            *cb_guard = None;
            let session = std::mem::replace(&mut *self.lock_dispatcher(), DispatcherSession::Idle);
            (taken, session)
        };
        if let Some(client) = taken_client {
            self.prev_drained
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(client.drained_flag());
            client.shutdown();
            drop(client);
            if let DispatcherSession::Running { handle } = prev_session {
                if handle.thread().id() != std::thread::current().id() {
                    if let Err(payload) = handle.join() {
                        // Record the panic reason so `isStreaming()` /
                        // `isAuthenticated()` report the failed state if
                        // streaming is restarted without re-checking. The
                        // `Failed` state is the observable signal; no
                        // logging dependency is pulled into this crate.
                        let reason = if let Some(s) = payload.downcast_ref::<&str>() {
                            (*s).to_owned()
                        } else if let Some(s) = payload.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "dispatcher panicked with non-string payload".to_owned()
                        };
                        *self.lock_dispatcher() = DispatcherSession::Failed { reason };
                    }
                }
            }
        }
    }

    /// Alias for `stopStreaming`. Mirrors the unified client's split surface
    /// where `shutdown` is documented as the terminal stop — on the
    /// standalone client both names are equivalent.
    #[napi(js_name = "shutdown")]
    pub fn shutdown(&self) {
        self.stop_streaming();
    }

    /// Re-open the FPSS connection and re-register the previously installed
    /// callback. Requires a prior `startStreaming(callback)`; throws
    /// otherwise.
    ///
    /// Saves the active per-contract and full-stream subscriptions against
    /// the old session, opens a fresh FPSS connection under the previously
    /// installed callback, and re-applies the saved subscriptions through
    /// the core's paced replay engine. Per-subscription failures surface as
    /// a single error naming every contract that did not re-subscribe — the
    /// streaming session itself is already up at that point.
    #[napi]
    pub async fn reconnect(&self) -> napi::Result<()> {
        let stored = {
            let guard = self.lock_callback();
            match guard.as_ref() {
                Some(cb) => Arc::clone(cb),
                None => {
                    return Err(napi::Error::from_reason(
                        "no callback registered -- call startStreaming(callback) before reconnect()",
                    ));
                }
            }
        };

        // Snapshot the active subscriptions BEFORE stopping.
        let (per_contract, full_stream) =
            self.with_live(|c| Ok((c.active_subscriptions(), c.active_full_subscriptions())))?;

        // Stop + restart under the same callback. The restart re-runs the
        // FPSS connect and authentication handshake off the libuv thread.
        self.stop_streaming();
        self.start_with_callback(stored).await?;

        // Re-apply every saved subscription against the freshly reconnected
        // session through the core's paced replay engine. The replay is
        // network-bound and paced, so it runs on a blocking worker to keep
        // the Node event loop free for the whole restore.
        let inner = {
            let guard = self.lock_inner();
            guard.as_ref().map(Arc::clone)
        };
        let Some(inner) = inner else {
            return Err(napi::Error::from_reason(
                "streaming not started -- call startStreaming(callback) first",
            ));
        };
        runtime()
            .spawn_blocking(move || inner.restore_subscriptions(&per_contract, &full_stream))
            .await
            .map_err(|e| napi::Error::from_reason(format!("reconnect task panicked: {e}")))?
            .map_err(|e| napi::Error::from_reason(format!("reconnect succeeded but {e}")))
    }

    /// Block until every superseded streaming session's event-ring consumer
    /// has finished firing the registered callback. Resolves `true` once
    /// all retired generations have drained, `false` on timeout. Polls at
    /// 1 ms cadence on a worker so the Node event loop stays free.
    #[napi(js_name = "awaitDrain")]
    pub async fn await_drain(&self, timeout_ms: u32) -> napi::Result<bool> {
        let timeout = Duration::from_millis(u64::from(timeout_ms));
        // Snapshot the retired-generation flags; the poll loop is a cheap
        // sleep loop that owns its own `Arc`s, so it can run on a blocking
        // worker without borrowing `&self` for `'static`.
        let flags: Vec<Arc<AtomicBool>> = self
            .prev_drained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let drained = runtime()
            .spawn_blocking(move || {
                let deadline = Instant::now() + timeout;
                let mut pending = flags;
                loop {
                    pending.retain(|f| !f.load(Ordering::Acquire));
                    if pending.is_empty() {
                        return true;
                    }
                    if Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
            .await
            .map_err(|e| napi::Error::from_reason(format!("await_drain task panicked: {e}")))?;
        // Prune drained generations the poll observed so a later call does
        // not re-wait on them.
        if drained {
            self.prev_drained
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|f| !f.load(Ordering::Acquire));
        }
        Ok(drained)
    }
}

impl StreamingClient {
    /// Assemble an idle handle from a parameter snapshot. The FPSS TLS
    /// connection is not opened until `startStreaming`.
    fn from_params(params: FpssParams) -> Self {
        Self {
            params,
            inner: Mutex::new(None),
            callback: Mutex::new(None),
            prev_drained: Mutex::new(Vec::new()),
            dispatcher: Mutex::new(DispatcherSession::Idle),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thetadatadx::config::{
        HostSelectionPolicy, JitterMode, ReconnectPolicy, StreamingFlushMode, StreamingWaitStrategy,
    };

    /// Anti-drift guard for the standalone connect path.
    ///
    /// `FpssParams` snapshots the whole `StreamingConfig` + `ReconnectConfig`
    /// and `builder()` threads every field into the `StreamingClientBuilder`,
    /// so the standalone TypeScript `StreamingClient` honours the same
    /// streaming and reconnect surface as the unified client and the C ABI.
    /// This test sets every streaming and reconnect knob to a non-default
    /// value and asserts each one survives the snapshot. A future field that
    /// `from_config` forgets to carry makes this fail rather than silently
    /// dropping a user's tuning.
    #[test]
    fn from_config_preserves_every_streaming_and_reconnect_knob() {
        let creds = RustCredentials::new("user@example.com", "secret");
        let mut config = DirectConfig::production();

        // Streaming: flip every knob away from its production default.
        config.set_streaming_hosts(vec![("stream.example.com".to_owned(), 12345)]);
        config.streaming.host_selection = HostSelectionPolicy::FixedOrder;
        config.streaming.host_shuffle_seed = Some(0xABCD_1234);
        config.streaming.timeout_ms = 111_111;
        config.streaming.ring_size = 1 << 20;
        config.streaming.ping_interval_ms = 22_222;
        config.streaming.connect_timeout_ms = 33_333;
        config.streaming.io_read_slice_ms = 44;
        config.streaming.data_watchdog_ms = 55_555;
        config.streaming.keepalive_idle_secs = 66;
        config.streaming.keepalive_interval_secs = 77;
        config.streaming.keepalive_retries = 8;
        config.streaming.flush_mode = StreamingFlushMode::Immediate;
        config.streaming.wait_strategy = StreamingWaitStrategy::Balanced;
        config.streaming.wait_spin_iters = 123;
        config.streaming.wait_yield_iters = 456;
        config.streaming.wait_park_us = 789;
        config.streaming.consumer_cpu = Some(3);
        config.streaming.derive_ohlcvc = false;

        // Reconnect: flip every knob away from its production default.
        config.reconnect.wait_ms = 1_010;
        config.reconnect.wait_max_ms = 2_020;
        config.reconnect.wait_rate_limited_ms = 3_030;
        config.reconnect.wait_server_restart_ms = 4_040;
        config.reconnect.jitter = JitterMode::None;
        config.reconnect.replay_burst_size = 51;
        config.reconnect.replay_pace_ms = 62;
        config.reconnect.policy = ReconnectPolicy::Manual;

        let params = FpssParams::from_config(&creds, &config);

        let s = &params.streaming;
        assert_eq!(s.hosts(), config.streaming_hosts());
        assert_eq!(s.host_selection, HostSelectionPolicy::FixedOrder);
        assert_eq!(s.host_shuffle_seed, Some(0xABCD_1234));
        assert_eq!(s.timeout_ms, 111_111);
        assert_eq!(s.ring_size, 1 << 20);
        assert_eq!(s.ping_interval_ms, 22_222);
        assert_eq!(s.connect_timeout_ms, 33_333);
        assert_eq!(s.io_read_slice_ms, 44);
        assert_eq!(s.data_watchdog_ms, 55_555);
        assert_eq!(s.keepalive_idle_secs, 66);
        assert_eq!(s.keepalive_interval_secs, 77);
        assert_eq!(s.keepalive_retries, 8);
        assert_eq!(s.flush_mode, StreamingFlushMode::Immediate);
        assert_eq!(s.wait_strategy, StreamingWaitStrategy::Balanced);
        assert_eq!(s.wait_spin_iters, 123);
        assert_eq!(s.wait_yield_iters, 456);
        assert_eq!(s.wait_park_us, 789);
        assert_eq!(s.consumer_cpu, Some(3));
        assert!(!s.derive_ohlcvc);

        let r = &params.reconnect;
        assert_eq!(r.wait_ms, 1_010);
        assert_eq!(r.wait_max_ms, 2_020);
        assert_eq!(r.wait_rate_limited_ms, 3_030);
        assert_eq!(r.wait_server_restart_ms, 4_040);
        assert_eq!(r.jitter, JitterMode::None);
        assert_eq!(r.replay_burst_size, 51);
        assert_eq!(r.replay_pace_ms, 62);
        assert!(
            matches!(r.policy, ReconnectPolicy::Manual),
            "reconnect policy must survive the snapshot"
        );

        // The snapshot must build without panicking with every knob set.
        let _ = params.builder();
    }
}
