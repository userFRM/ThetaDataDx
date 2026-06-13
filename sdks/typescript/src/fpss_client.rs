//! Standalone TypeScript (napi-rs) `FpssClient` — FPSS streaming only.
//!
//! Opens ONLY the FPSS TLS transport — no MDDS channel, no Nexus HTTP
//! authentication, no historical / Treasury / Calendar surface. Mirrors
//! the Python `FpssClient` (`sdks/python/src/fpss_client.rs`), the C++
//! `tdx::FpssClient` (`sdks/cpp/include/thetadx.hpp`), and the standalone
//! C ABI entry points (`tdx_fpss_*` in `ffi/src/streaming.rs`), letting a
//! Node.js caller run an FPSS-only session alongside an externally
//! managed MDDS process without the bundled
//! [`crate::ThetaDataDxClient`] preempting the parallel MDDS work at the
//! Nexus session layer.
//!
//! # Why a hand-written module
//!
//! The unified [`crate::ThetaDataDxClient`] drives the high-level
//! `thetadatadx::ThetaDataDxClient::start_streaming` convenience, which
//! owns its own dispatcher thread and `DispatcherSession`. The standalone
//! client wraps `thetadatadx::fpss::FpssClient` directly — the lower-level
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
//! [`crate::ThetaDataDxClient`] when you need the MDDS surface and Nexus
//! session machinery side by side.
//!
//! # Lifecycle
//!
//! 1. `FpssClient.connect(...)` / `connectFromFile(...)` — snapshots the
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
use thetadatadx::config::{self, DirectConfig};
use thetadatadx::fpss::protocol::SubscriptionKind;
use thetadatadx::fpss::{self, FpssClient as RustFpssClient};
use thetadatadx::DispatcherSession;

use crate::fluent::Subscription;
use crate::{
    buffered_event_to_typed, fpss_event_to_buffered, runtime, to_napi_err, Config, TsfnCallback,
};

/// Snapshot of the parameters required to open an FPSS TLS connection.
///
/// Cloned out of the user's `Config` at construction time so subsequent
/// mutations of the `Config` handle cannot retroactively change reconnect
/// behaviour for an already-running session — the same snapshot semantics
/// the Python binding (`FpssParams`) and the FFI
/// (`ffi/src/streaming.rs::FpssConnectParams`) use.
struct FpssParams {
    creds: RustCredentials,
    hosts: Vec<(String, u16)>,
    ring_size: usize,
    flush_mode: thetadatadx::config::FpssFlushMode,
    policy: thetadatadx::config::ReconnectPolicy,
    wait_ms: u64,
    wait_rate_limited_ms: u64,
    derive_ohlcvc: bool,
    connect_timeout_ms: u64,
    read_timeout_ms: u64,
    ping_interval_ms: u64,
}

impl FpssParams {
    fn from_config(creds: &RustCredentials, config: &DirectConfig) -> Self {
        Self {
            creds: creds.clone(),
            hosts: config.fpss.hosts.clone(),
            ring_size: config.fpss.ring_size,
            flush_mode: config.fpss.flush_mode,
            policy: config.reconnect.policy.clone(),
            wait_ms: config.reconnect.wait_ms,
            wait_rate_limited_ms: config.reconnect.wait_rate_limited_ms,
            derive_ohlcvc: config.fpss.derive_ohlcvc,
            connect_timeout_ms: config.fpss.connect_timeout_ms,
            read_timeout_ms: config.fpss.timeout_ms,
            ping_interval_ms: config.fpss.ping_interval_ms,
        }
    }

    fn builder(&self) -> fpss::FpssClientBuilder<'_> {
        fpss::FpssClientBuilder::new(&self.creds, &self.hosts)
            .ring_size(self.ring_size)
            .flush_mode(self.flush_mode)
            .reconnect_policy(self.policy.clone())
            .reconnect_wait_ms(self.wait_ms)
            .reconnect_wait_rate_limited_ms(self.wait_rate_limited_ms)
            .derive_ohlcvc(self.derive_ohlcvc)
            .connect_timeout_ms(self.connect_timeout_ms)
            .read_timeout_ms(self.read_timeout_ms)
            .ping_interval_ms(self.ping_interval_ms)
    }
}

/// Build the snapshot from an owned [`DirectConfig`], rejecting a config
/// with no FPSS hosts before any TLS work begins. Mirrors the Python
/// `FpssClient.__new__` empty-hosts guard.
fn params_from_direct(creds: &RustCredentials, direct: &DirectConfig) -> napi::Result<FpssParams> {
    if direct.fpss.hosts.is_empty() {
        return Err(crate::invalid_parameter_err(
            "FpssClient: config.fpss.hosts is empty (use Config.production() or set the FPSS hosts)",
        ));
    }
    Ok(FpssParams::from_config(creds, direct))
}

/// Standalone FPSS-only streaming client.
///
/// Opens ONLY the FPSS TLS transport — no MDDS channel, no Nexus HTTP
/// authentication. Use when a parallel MDDS process is already running in
/// the same environment and you need to stream FPSS without the bundled
/// [`crate::ThetaDataDxClient`] taking over the Nexus session at connect
/// time.
///
/// ```js
/// const { FpssClient, Config, Contract } = require("@thetadatadx/sdk");
/// const fpss = FpssClient.connectFromFile("creds.txt");
/// fpss.startStreaming((event) => console.log(event.kind, event));
/// fpss.subscribe(Contract.stock("AAPL").quote());
/// // ... events arrive on the Node main thread ...
/// fpss.stopStreaming();
/// ```
#[napi]
pub struct FpssClient {
    /// Connect parameters captured at construction time. Reused on every
    /// `startStreaming` / `reconnect`.
    params: FpssParams,
    /// Currently-open inner FPSS client. `None` between construction and
    /// `startStreaming`, and after `stopStreaming` / `shutdown`.
    inner: Mutex<Option<Arc<RustFpssClient>>>,
    /// Most recently registered JS callback, behind an `Arc` so the
    /// dispatcher closure can hold its own ref-counted clone. Retained
    /// across `startStreaming` so `reconnect()` can re-register the same
    /// handler without the caller passing it again; cleared on
    /// `stopStreaming` / `shutdown` so a teardown the application has
    /// already observed releases the napi reference back to V8 — the same
    /// explicit-handoff model as the unified [`crate::ThetaDataDxClient`].
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

impl Drop for FpssClient {
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

impl FpssClient {
    fn lock_inner(&self) -> MutexGuard<'_, Option<Arc<RustFpssClient>>> {
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
        f: impl FnOnce(&RustFpssClient) -> Result<R, thetadatadx::Error>,
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
    /// Lock ordering: `callback` BEFORE `inner`, matching `stopStreaming`.
    fn start_with_callback(&self, callback: Arc<TsfnCallback>) -> napi::Result<()> {
        let mut cb_guard = self.lock_callback();
        if self.lock_inner().is_some() {
            return Err(napi::Error::from_reason(
                "streaming already started -- call stopStreaming() before startStreaming() again",
            ));
        }

        let dispatch_cb = Arc::clone(&callback);

        let client = self
            .params
            .builder()
            .build()
            .map_err(|e| to_napi_err(thetadatadx::Error::from(e)))?;
        let client_arc = Arc::new(client);

        // Publish the client and the stored callback BEFORE spawning the
        // dispatcher so the first delivered event sees a fully initialised
        // handle. A re-entrant call from inside the user callback to
        // `subscribe()` / `isStreaming()` would otherwise race the late
        // publish and observe `inner = None`.
        *self.lock_inner() = Some(Arc::clone(&client_arc));
        *cb_guard = Some(callback);
        drop(cb_guard);

        let dispatcher_client = Arc::clone(&client_arc);
        let dispatcher = std::thread::Builder::new()
            .name("tdx-ts-fpss-dispatcher".into())
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
                        |event: &fpss::FpssEvent| {
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
impl FpssClient {
    // Lifecycle: intentionally hand-written. The connect factories snapshot
    // the connect parameters but do NOT open the FPSS TLS connection —
    // connection is deferred to the first `startStreaming` call, matching
    // the C ABI's deferred-connect contract (`tdx_fpss_connect` allocates
    // the handle, `tdx_fpss_set_callback` opens the network) so the same
    // observable behaviour applies across every binding. No MDDS channel is
    // opened and no Nexus request is issued by any factory.

    /// Allocate a standalone FPSS handle against the production endpoint.
    /// Streaming only — opens no MDDS channel and issues no Nexus request.
    /// The FPSS TLS connection opens on the first `startStreaming` call.
    #[napi(factory)]
    pub fn connect(email: String, password: String) -> napi::Result<FpssClient> {
        let creds = auth::Credentials::new(email, password);
        let direct = config::DirectConfig::production();
        let params = params_from_direct(&creds, &direct)?;
        Ok(FpssClient::from_params(params))
    }

    /// Allocate a standalone FPSS handle with a credentials file (line 1 =
    /// email, line 2 = password) against the production endpoint.
    #[napi(factory)]
    pub fn connect_from_file(path: String) -> napi::Result<FpssClient> {
        let creds = auth::Credentials::from_file(&path).map_err(to_napi_err)?;
        let direct = config::DirectConfig::production();
        let params = params_from_direct(&creds, &direct)?;
        Ok(FpssClient::from_params(params))
    }

    /// Allocate a standalone FPSS handle against an explicit [`Config`]
    /// (`dev` / `stage` / `production`, plus any tuned FPSS / reconnect
    /// setters). Use `connect` for the production-default endpoint.
    ///
    /// The config is snapshot at construction time: the `Config` handle may
    /// be reused or mutated afterward without affecting this client.
    #[napi(factory)]
    pub fn connect_with_config(
        email: String,
        password: String,
        config: &Config,
    ) -> napi::Result<FpssClient> {
        let creds = auth::Credentials::new(email, password);
        let direct = config.snapshot();
        let params = params_from_direct(&creds, &direct)?;
        Ok(FpssClient::from_params(params))
    }

    /// Allocate a standalone FPSS handle with a credentials file against an
    /// explicit [`Config`]. Use `connectFromFile` for the
    /// production-default endpoint.
    ///
    /// The config is snapshot at construction time.
    #[napi(factory)]
    pub fn connect_from_file_with_config(
        path: String,
        config: &Config,
    ) -> napi::Result<FpssClient> {
        let creds = auth::Credentials::from_file(&path).map_err(to_napi_err)?;
        let direct = config.snapshot();
        let params = params_from_direct(&creds, &direct)?;
        Ok(FpssClient::from_params(params))
    }

    /// Start FPSS streaming and register a JS callback for incoming events.
    ///
    /// Opens the FPSS TLS connection and starts the background dispatcher.
    /// The dispatcher converts every typed FPSS event and routes it through
    /// a napi-rs `ThreadsafeFunction` to the Node main thread, where
    /// `callback(event)` runs. The FPSS TLS reader thread itself never
    /// touches V8: events cross the streaming ring first, with the consumer
    /// thread invoking the callback under `catch_unwind`.
    ///
    /// Backpressure: a slow callback fills the streaming ring and overflow
    /// events are dropped, observable via `droppedEventCount()`. The FPSS
    /// TLS reader is never blocked — vendor disconnects on slow consumers
    /// cannot happen on this path.
    #[napi(js_name = "startStreaming")]
    pub fn start_streaming(&self, callback: TsfnCallback) -> napi::Result<()> {
        self.start_with_callback(Arc::new(callback))
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
    /// resets. Returned as `bigint` for the full `u64` range.
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

    /// Cumulative count of user-callback panics caught by the
    /// per-invocation `catch_unwind` boundary. A panic is caught, recorded
    /// here, and does not stop event delivery. Returned as `bigint` for the
    /// full `u64` range.
    #[napi(js_name = "panicCount")]
    pub fn panic_count(&self) -> napi::bindgen_prelude::BigInt {
        let guard = self.lock_inner();
        napi::bindgen_prelude::BigInt::from(guard.as_ref().map_or(0, |c| c.panic_count()))
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
    pub fn reconnect(&self) -> napi::Result<()> {
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

        // Stop + restart under the same callback.
        self.stop_streaming();
        self.start_with_callback(stored)?;

        // Re-apply every saved subscription against the freshly reconnected
        // session through the core's paced replay engine.
        let inner = {
            let guard = self.lock_inner();
            guard.as_ref().map(Arc::clone)
        };
        let Some(inner) = inner else {
            return Err(napi::Error::from_reason(
                "streaming not started -- call startStreaming(callback) first",
            ));
        };
        inner
            .restore_subscriptions(&per_contract, &full_stream)
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

impl FpssClient {
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
