//! Standalone Python `FpssClient` pyclass.
//!
//! Opens ONLY the FPSS TLS transport — no MDDS channel, no Nexus
//! HTTP auth, no Treasury / Calendar / OHLCVC historical surface.
//! Mirrors the C++ `tdx::FpssClient` (`sdks/cpp/include/thetadx.hpp`)
//! and the standalone C ABI entry points (`tdx_fpss_*` in
//! `ffi/src/streaming.rs`), letting Python users run an FPSS-only
//! session alongside an externally-managed MDDS process without the
//! bundled [`crate::ThetaDataDxClient`] preempting the parallel MDDS
//! work at the Nexus session layer.
//!
//! # Nexus session behaviour
//!
//! This pyclass does NOT issue a Nexus authentication. FPSS speaks its
//! own protocol-level `CREDENTIALS` handshake (wire code `0`) on the
//! TLS connection itself; no separate Nexus session UUID is acquired.
//! The cross-binding contract here matches the standalone C ABI:
//! `tdx_fpss_connect` accepts a `TdxCredentials` handle without
//! touching Nexus. Run the bundled [`crate::ThetaDataDxClient`] (which
//! does authenticate against Nexus) when you need the MDDS surface and
//! Nexus session machinery side-by-side.
//!
//! # Lifecycle
//!
//! 1. `FpssClient(creds, config)` — snapshots the connect parameters.
//!    The FPSS TLS connection is opened lazily by `start_streaming`
//!    (matching the FFI's deferred-connect contract).
//! 2. `start_streaming(callback)` — opens the FPSS TLS connection and
//!    starts the background dispatcher that drives the ring iterator.
//! 3. `subscribe(...)` / `unsubscribe(...)` — fluent subscription.
//! 4. `stop_streaming()` / `shutdown()` — atomic stop with drain barrier.
//! 5. `reconnect()` — re-open under the same callback.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use thetadatadx::auth::Credentials as RustCredentials;
use thetadatadx::config::DirectConfig;
use thetadatadx::fpss::protocol::{FullSubscriptionKind, SubscriptionKind};
use thetadatadx::fpss::{self, FpssClient as RustFpssClient};
use thetadatadx::DispatcherSession as PyFpssDispatcherSession;

use crate::buffered_event_to_typed;
use crate::errors::to_py_err;
use crate::fluent::{self, PySubscription};
use crate::fpss_event_to_buffered;
use crate::streaming_session::{StreamableHandle, StreamingSession};
use crate::{Config, Credentials};

/// Snapshot of the parameters required to open an FPSS TLS connection.
///
/// Cloned out of the user's `Config` at construction time so subsequent
/// Python-side mutations of the `Config` handle cannot retroactively
/// change reconnect behaviour for an already-running session — the same
/// snapshot semantics the FFI uses in
/// `ffi/src/streaming.rs::FpssConnectParams`.
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

/// Standalone FPSS-only streaming client.
///
/// Opens ONLY the FPSS TLS transport — no MDDS channel, no Nexus
/// HTTP authentication. Use when a parallel MDDS process is already
/// running in the same environment and you need to test FPSS without
/// the bundled [`crate::ThetaDataDxClient`] taking over the Nexus
/// session at construction time.
///
/// ```python
/// from thetadatadx import FpssClient, Credentials, Config, Contract
///
/// creds = Credentials.from_file("creds.txt")
/// fpss = FpssClient(creds, Config.production())
///
/// def on_event(event):
///     print(event.kind, event)
///
/// fpss.start_streaming(callback=on_event)
/// fpss.subscribe(Contract.stock("AAPL").quote())
/// # ... events arrive on the event-dispatch consumer thread ...
/// fpss.stop_streaming()
/// ```
// N5: `frozen` — every `#[pymethods]` entry takes `&self` (never
// `&mut self`). The inner `Arc<Mutex<Option<fpss::FpssClient>>>`
// carries its own interior mutability; the pyclass shell is
// immutable. A future `&mut self` regression surfaces as a
// `cargo check` failure rather than slipping silently.
#[pyclass(module = "thetadatadx", name = "FpssClient", frozen)]
pub(crate) struct FpssClient {
    /// Connect parameters captured at construction time. Reused on
    /// every `start_streaming*` / `reconnect`.
    params: FpssParams,
    /// Currently-open inner FPSS client. `None` between construction
    /// and `start_streaming*`, and after `stop_streaming` / `shutdown`.
    inner: Mutex<Option<Arc<RustFpssClient>>>,
    /// Most recently registered Python callable. Retained across
    /// `start_streaming` so `reconnect()` can re-register the same
    /// handler without the caller having to pass it again. Cleared on
    /// `stop_streaming` / `shutdown` so a teardown the application has
    /// already observed does not leak the closure's captured
    /// references — same explicit-handoff model as the unified
    /// [`crate::ThetaDataDxClient`].
    callback: Mutex<Option<Py<PyAny>>>,
    /// Quiescence flags of every superseded streaming session that has
    /// not yet drained. Mirrors the `prev_drained` field on the unified
    /// [`thetadatadx::ThetaDataDxClient`] — stacked stop/start cycles
    /// can layer multiple in-flight event-dispatch consumers, and
    /// `await_drain` must wait for all of them before reporting
    /// quiescence.
    prev_drained: Mutex<Vec<Arc<AtomicBool>>>,
    /// Dispatcher lifecycle — single mutex replacing
    /// `dispatcher_handle: Mutex<Option<JoinHandle<()>>>` and
    /// `dispatcher_failed: Arc<AtomicBool>`. Panic state is derived
    /// from `JoinHandle::join()` returning `Err(_)`.
    dispatcher: Mutex<thetadatadx::DispatcherSession>,
}

impl Drop for FpssClient {
    /// Release the GIL across the inner drop and join the dispatcher
    /// thread so a callback in flight does not race destruction.
    ///
    /// The dispatcher re-acquires the GIL via `Python::attach` on every
    /// event, so holding the GIL across the join would deadlock. Take
    /// the inner Arc and the dispatcher handle out under the binding
    /// mutexes, signal shutdown so the iterator loop drains and exits,
    /// then detach to drop them on the dispatcher-friendly path.
    fn drop(&mut self) {
        let taken_client = self.inner.lock().unwrap_or_else(|e| e.into_inner()).take();
        let prev_session = std::mem::replace(
            &mut *self.dispatcher.lock().unwrap_or_else(|e| e.into_inner()),
            PyFpssDispatcherSession::Idle,
        );
        let taken_handle = match prev_session {
            PyFpssDispatcherSession::Running { handle } => Some(handle),
            _ => None,
        };
        if taken_client.is_some() || taken_handle.is_some() {
            Python::attach(|py| {
                py.detach(move || {
                    if let Some(ref client) = taken_client {
                        client.shutdown();
                    }
                    drop(taken_client);
                    if let Some(h) = taken_handle {
                        if h.thread().id() != std::thread::current().id() {
                            let _ = h.join();
                        }
                    }
                });
            });
        }
    }
}

impl FpssClient {
    fn lock_inner(&self) -> MutexGuard<'_, Option<Arc<RustFpssClient>>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_callback(&self) -> MutexGuard<'_, Option<Py<PyAny>>> {
        self.callback.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run a closure with a borrow of the live FPSS client, raising
    /// `RuntimeError` when nothing is connected.
    fn with_live<R>(
        &self,
        f: impl FnOnce(&RustFpssClient) -> Result<R, thetadatadx::Error>,
    ) -> PyResult<R> {
        let guard = self.lock_inner();
        let client = guard.as_ref().ok_or_else(|| {
            PyRuntimeError::new_err("streaming not started -- call start_streaming(callback) first")
        })?;
        f(client).map_err(to_py_err)
    }
}

#[pymethods]
impl FpssClient {
    /// Allocate a standalone FPSS handle.
    ///
    /// Snapshots the connect parameters out of the supplied `Config`
    /// but does NOT open the FPSS TLS connection — connection is
    /// deferred to the first `start_streaming*` call. This matches the
    /// C ABI's deferred-connect contract (`tdx_fpss_connect` allocates
    /// the handle, `tdx_fpss_set_callback` opens the network) so the
    /// same observable behaviour applies across every binding.
    ///
    /// No MDDS channel is opened. No Nexus HTTP request is issued.
    /// A parallel MDDS process under the same credentials is unaffected
    /// by this constructor.
    #[new]
    fn new(_py: Python<'_>, creds: &Credentials, config: &Config) -> PyResult<Self> {
        let direct = {
            let guard = config.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        if direct.fpss.hosts.is_empty() {
            return Err(PyValueError::new_err(
                "FpssClient: config.fpss.hosts is empty (set THETADATA_FPSS_HOSTS or use Config::production())",
            ));
        }
        Ok(Self {
            params: FpssParams::from_config(&creds.inner, &direct),
            inner: Mutex::new(None),
            callback: Mutex::new(None),
            prev_drained: Mutex::new(Vec::new()),
            dispatcher: Mutex::new(PyFpssDispatcherSession::Idle),
        })
    }

    /// Convenience constructor: `FpssClient.from_file("creds.txt")`.
    /// Loads credentials from a two-line file and connects with the
    /// supplied `config`, defaulting to `Config.production()`.
    ///
    /// Parity with `ThetaDataDxClient.from_file()`,
    /// `AsyncThetaDataDxClient.from_file()`, and
    /// `MddsClient.from_file()` — every standalone Python client
    /// surfaces the same one-shot constructor shape.
    #[staticmethod]
    #[pyo3(signature = (path, config=None))]
    fn from_file(py: Python<'_>, path: &str, config: Option<&Config>) -> PyResult<Self> {
        let creds = Credentials::from_file(path)?;
        let owned_default;
        let cfg = match config {
            Some(c) => c,
            None => {
                owned_default = Config::production();
                &owned_default
            }
        };
        Self::new(py, &creds, cfg)
    }

    fn __repr__(&self) -> String {
        // Match the bundled `ThetaDataDxClient.__repr__` key/value vocabulary
        // (`streaming=connected` / `streaming=none`) so cross-class repr
        // strings parse the same way.
        // Derive the `streaming=` label from the failure-aware
        // `is_streaming()` gate so a panicked dispatcher reports
        // `streaming=none` consistently with `is_streaming()` and
        // `is_authenticated()`.
        let streaming = if self.is_streaming() {
            "connected"
        } else {
            "none"
        };
        let hosts = self.params.hosts.len();
        format!("FpssClient(streaming={streaming}, hosts={hosts})")
    }

    /// Open the FPSS TLS connection and register the Python callback
    /// for incoming events.
    ///
    /// The event-dispatch consumer thread acquires the GIL via
    /// `Python::attach` to invoke `callback(event)` for every typed
    /// FPSS event. Each invocation is individually wrapped in
    /// `catch_unwind`: a panic on event N is caught, recorded via
    /// `panic_count()`, and does not stop event delivery — event N+1
    /// continues normally. `callback` must accept exactly one positional
    /// argument — a typed FPSS event class (`Quote`, `Trade`, `Ohlcvc`,
    /// … the same hierarchy emitted on the unified client's callback path).
    ///
    /// The reader never blocks on user code; on ring overflow events
    /// are dropped and counted via `dropped_event_count()`. User
    /// callback panics are caught and counted via `panic_count()`.
    pub(crate) fn start_streaming(&self, callback: Py<PyAny>) -> PyResult<()> {
        let mut cb_guard = self.lock_callback();
        if self.lock_inner().is_some() {
            return Err(PyRuntimeError::new_err(
                "streaming already started -- call stop_streaming() before start_streaming() again",
            ));
        }

        let callback_arc: Arc<Py<PyAny>> = Arc::new(callback);
        let dispatch_cb = Arc::clone(&callback_arc);

        let client = self
            .params
            .builder()
            .build()
            .map_err(|e| to_py_err(thetadatadx::Error::from(e)))?;
        let client_arc = Arc::new(client);

        // Publish the client and the stored callback BEFORE spawning
        // the dispatcher so the first delivered event sees a fully
        // initialised handle. A re-entrant call from inside the user
        // callback to `subscribe()` / `with_live()` / `is_streaming()`
        // would otherwise race the late publish and observe
        // `inner = None`, raising `RuntimeError("streaming not
        // started")`.
        *self.lock_inner() = Some(Arc::clone(&client_arc));
        *cb_guard = Some(Python::attach(|py| callback_arc.clone_ref(py)));
        drop(cb_guard);

        let dispatcher_client = Arc::clone(&client_arc);
        // Clone a handle for counting Python exceptions inside the closure.
        // A `PyErr` raised by the callback does not unwind through Rust's
        // `catch_unwind`; `poll_batch` never sees it as a panic. The
        // binding must increment the counter explicitly so `panic_count()`
        // reflects both Rust panics and Python exceptions.
        let panic_recorder = Arc::clone(&client_arc);
        let dispatcher = std::thread::Builder::new()
            .name("tdx-py-fpss-dispatcher".into())
            .spawn(move || {
                // `FpssClient::for_each` drives `poll_batch`, which wraps
                // each callback invocation in its own `catch_unwind`.  A
                // Rust panic in the handler is caught, recorded via
                // `panic_count()`, and does not stop event delivery for
                // subsequent events.  The outer `catch_unwind` below
                // guards only the event-iteration machinery itself.
                //
                // `PyErr` raised by the user callback is NOT a Rust panic;
                // it is caught by `call1` returning `Err(err)` and is
                // written as an unraisable exception via `write_unraisable`
                // (the same channel Python uses for `__del__` errors).
                // `panic_recorder.record_panic()` ensures the counter is
                // also bumped for Python exceptions.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    dispatcher_client.for_each(|event| {
                        Python::attach(|py| {
                            let buffered = fpss_event_to_buffered(event);
                            let typed = match buffered_event_to_typed(py, &buffered) {
                                Ok(obj) => obj,
                                Err(err) => {
                                    err.write_unraisable(py, None);
                                    return;
                                }
                            };
                            if let Err(err) = dispatch_cb.call1(py, (typed,)) {
                                err.write_unraisable(py, None);
                                panic_recorder.record_panic();
                            }
                        });
                    });
                }));
                if outcome.is_err() {
                    tracing::error!(
                        target: "thetadatadx::python",
                        "tdx-py-fpss-dispatcher panicked in event iteration machinery; FpssClient transitioning to failed state",
                    );
                }
            });
        match dispatcher {
            Ok(h) => {
                *self.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) =
                    PyFpssDispatcherSession::Running { handle: h };
            }
            Err(e) => {
                let taken = self.lock_inner().take();
                *self.lock_callback() = None;
                if let Some(client) = taken {
                    client.shutdown();
                }
                return Err(PyRuntimeError::new_err(format!(
                    "failed to spawn FPSS dispatcher thread: {e}"
                )));
            }
        }
        Ok(())
    }

    /// Whether the FPSS TLS connection is currently open.
    ///
    /// Returns `false` when the dispatcher thread panicked — no events
    /// are arriving even though the TLS slot is still populated, so
    /// callers must observe the failed state.
    fn is_streaming(&self) -> bool {
        let guard = self.lock_inner();
        if guard.as_ref().is_none() {
            return false;
        }
        let session = self.dispatcher.lock().unwrap_or_else(|e| e.into_inner());
        if let PyFpssDispatcherSession::Failed { reason } = &*session {
            tracing::debug!(
                target: "thetadatadx::python",
                reason = %reason,
                "is_streaming: dispatcher failed",
            );
            return false;
        }
        true
    }

    /// Whether the FPSS session is currently authenticated.
    ///
    /// Mirrors the C++ `tdx::FpssClient::is_authenticated()` getter and
    /// the C ABI `tdx_fpss_is_authenticated`. Distinct from
    /// `is_streaming()`: the TLS slot can hold an `RustFpssClient` whose
    /// `authenticated` flag has been flipped to `false` after a server
    /// disconnect, before the application has issued `reconnect()`.
    ///
    /// A panicked dispatcher thread also folds back to `false` here so
    /// the failed state is uniformly visible across every status reader,
    /// not just `is_streaming()`.
    fn is_authenticated(&self) -> bool {
        let guard = self.lock_inner();
        let Some(client) = guard.as_ref() else {
            return false;
        };
        let dispatcher_failed = matches!(
            *self.dispatcher.lock().unwrap_or_else(|e| e.into_inner()),
            PyFpssDispatcherSession::Failed { .. }
        );
        client.is_authenticated() && !dispatcher_failed
    }

    /// Snapshot of per-contract subscriptions on the live session.
    ///
    /// Returns the same typed `Subscription` values the caller passes to
    /// `subscribe()` — round-trippable rather than a debug-format
    /// string projection. Empty list when streaming has not started.
    fn active_subscriptions(&self) -> Vec<PySubscription> {
        let guard = self.lock_inner();
        let Some(client) = guard.as_ref() else {
            return Vec::new();
        };
        client
            .active_subscriptions()
            .into_iter()
            .map(|(kind, contract)| PySubscription {
                inner: fpss::protocol::Subscription::Contract { contract, kind },
            })
            .collect()
    }

    /// Snapshot of full-stream subscriptions (e.g. `SecType.OPTION.full_trades()`).
    ///
    /// Returns the same typed `Subscription` values the caller passes to
    /// `subscribe()`. Quote is never a valid full-stream kind on the
    /// FPSS wire, so the core's `active_full_subscriptions` only ever
    /// returns `Trade` / `OpenInterest`; any other variant is dropped
    /// from the projection. Empty list when streaming has not started.
    fn active_full_subscriptions(&self) -> Vec<PySubscription> {
        let guard = self.lock_inner();
        let Some(client) = guard.as_ref() else {
            return Vec::new();
        };
        client
            .active_full_subscriptions()
            .into_iter()
            .filter_map(|(kind, sec_type)| {
                let full_kind = match kind {
                    SubscriptionKind::Trade => FullSubscriptionKind::Trades,
                    SubscriptionKind::OpenInterest => FullSubscriptionKind::OpenInterest,
                    SubscriptionKind::Quote => return None,
                    _ => return None,
                };
                Some(PySubscription {
                    inner: fpss::protocol::Subscription::Full {
                        sec_type,
                        kind: full_kind,
                    },
                })
            })
            .collect()
    }

    /// Cumulative count of FPSS events the TLS reader could not
    /// publish into the event ring because the consumer fell
    /// behind. Snapshot the value BEFORE `reconnect()` if you need to
    /// accumulate drops across session boundaries — `reconnect`
    /// rebuilds the inner client and the counter resets.
    fn dropped_event_count(&self) -> u64 {
        let guard = self.lock_inner();
        guard.as_ref().map_or(0, |c| c.dropped_count())
    }

    /// Cumulative count of user-callback faults: Rust panics caught by the
    /// per-invocation `catch_unwind` boundary, and Python exceptions raised
    /// inside the callback (surfaced via `sys.unraisablehook`). Both kinds
    /// are counted atomically so callers observe a single unified fault
    /// counter. A fault does not stop event delivery — the next event
    /// continues normally. Incremented atomically; safe to read from any
    /// thread.
    fn panic_count(&self) -> u64 {
        let guard = self.lock_inner();
        guard.as_ref().map_or(0, |c| c.panic_count())
    }

    /// Milliseconds since the most recent inbound streaming frame of
    /// any kind (data tick, heartbeat, control), or ``None`` when no
    /// session is live or no frame has been received yet. The
    /// operator-facing staleness clock: a steadily growing value is
    /// the earliest external signal of a dead or wedged connection.
    fn millis_since_last_event(&self) -> Option<u64> {
        let guard = self.lock_inner();
        guard.as_ref().and_then(|c| c.millis_since_last_event())
    }

    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// streaming frame of any kind. Returns ``0`` when no session is
    /// live or no frame has been received yet.
    fn last_event_received_at_unix_nanos(&self) -> i64 {
        let guard = self.lock_inner();
        guard
            .as_ref()
            .map_or(0, |c| c.last_event_received_at_unix_nanos())
    }

    /// Address (``host:port``) of the streaming server the current
    /// session is connected to, following the session across
    /// auto-reconnects. ``None`` when no session is live.
    fn last_connected_addr(&self) -> Option<String> {
        let guard = self.lock_inner();
        guard.as_ref().map(|c| c.last_connected_addr())
    }

    /// Polymorphic subscribe — primary fluent entry point. Accepts
    /// any value returned by `Contract.quote()` / `Contract.trade()` /
    /// `Contract.open_interest()` (per-contract scope) or
    /// `SecType.OPTION.full_trades()` (full-stream scope).
    fn subscribe(&self, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        let inner = fluent::coerce_subscription(sub)?;
        self.with_live(|c| c.subscribe(inner))
    }

    /// Bulk-subscribe a list of `Subscription` values.
    ///
    /// Releases the inner mutex between iterations so a concurrent
    /// `stop_streaming` on a sibling thread (today still serialised by
    /// the GIL, but defensive against a future `py.detach` in the FPSS
    /// connect path) cannot deadlock against a long batch. The core
    /// FPSS protocol has no batched-subscribe wire frame today; a
    /// future single-command `subscribe_many` on
    /// `crates/thetadatadx/src/fpss/mod.rs` is tracked as a follow-up
    /// and would route through here without an API change.
    fn subscribe_many(&self, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        let list = fluent::coerce_subscription_list(subs)?;
        for sub in list {
            self.with_live(|c| c.subscribe(sub))?;
        }
        Ok(())
    }

    /// Polymorphic unsubscribe.
    fn unsubscribe(&self, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        let inner = fluent::coerce_subscription(sub)?;
        self.with_live(|c| c.unsubscribe(inner))
    }

    /// Bulk-unsubscribe.
    ///
    /// Releases the inner mutex between iterations — same rationale as
    /// `subscribe_many`.
    fn unsubscribe_many(&self, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        let list = fluent::coerce_subscription_list(subs)?;
        for sub in list {
            self.with_live(|c| c.unsubscribe(sub))?;
        }
        Ok(())
    }

    /// Stop streaming and clear the registered callback. Same
    /// explicit-handoff semantics as the unified client: to resume
    /// streaming after this returns, call `start_streaming(callback)`
    /// again with a freshly bound callable; `reconnect()` raises
    /// `RuntimeError` because no callback is held.
    ///
    /// Lock ordering: `callback` BEFORE `inner`, matching
    /// `start_streaming`. The two methods MUST agree on the order
    /// they acquire the two `Mutex` slots — Python's GIL serialises
    /// pyclass-method dispatch today, but a future revision that
    /// releases the GIL across the FPSS connect (e.g. via
    /// `py.detach` inside `start_streaming`) would let concurrent
    /// `start_streaming` / `stop_streaming` interleave on the same
    /// handle. Pinning the ordering here closes that race
    /// proactively.
    pub(crate) fn stop_streaming(&self, py: Python<'_>) {
        // Take the `Arc<RustFpssClient>` out of `inner` and the stored
        // Python callable out of `callback` under the binding mutexes,
        // then release both before signalling shutdown so a dispatcher
        // re-entering any pyclass method via the callback never sees a
        // lock held.
        let (taken_client, prev_session) = {
            let mut cb_guard = self.lock_callback();
            let taken = self.lock_inner().take();
            *cb_guard = None;
            let session = std::mem::replace(
                &mut *self.dispatcher.lock().unwrap_or_else(|e| e.into_inner()),
                PyFpssDispatcherSession::Idle,
            );
            (taken, session)
        };
        if let Some(client) = taken_client {
            self.prev_drained
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(client.drained_flag());
            client.shutdown();
            // Detach the GIL while dropping the Arc and joining the
            // dispatcher so the dispatcher thread (which re-acquires the
            // GIL via `Python::attach` on every event) can make progress.
            // Dispatcher panic state is derived from `JoinHandle::join()`
            // returning `Err(_)` — record as `Failed` so subsequent
            // `is_streaming()` / `is_authenticated()` calls see the
            // correct state if streaming is restarted without re-checking.
            let dispatcher_ref = &self.dispatcher;
            py.detach(move || {
                drop(client);
                if let PyFpssDispatcherSession::Running { handle } = prev_session {
                    if handle.thread().id() != std::thread::current().id() {
                        if let Err(payload) = handle.join() {
                            let reason = if let Some(s) = payload.downcast_ref::<&str>() {
                                (*s).to_owned()
                            } else if let Some(s) = payload.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "dispatcher panicked with non-string payload".to_owned()
                            };
                            tracing::error!(
                                target: "thetadatadx::python",
                                reason = %reason,
                                "tdx-py-fpss-dispatcher panicked; FpssClient marked as failed",
                            );
                            *dispatcher_ref.lock().unwrap_or_else(|e| e.into_inner()) =
                                PyFpssDispatcherSession::Failed { reason };
                        }
                    }
                }
            });
        }
    }

    /// Alias for `stop_streaming`. Mirrors the unified client's split
    /// surface where `shutdown` is documented as the terminal stop —
    /// on the standalone client both names are equivalent.
    fn shutdown(&self, py: Python<'_>) {
        self.stop_streaming(py);
    }

    /// Re-open the FPSS connection and re-register the previously
    /// installed callback. Requires a prior `start_streaming(callback)`;
    /// raises `RuntimeError` otherwise.
    ///
    /// Mirrors [`thetadatadx::ThetaDataDxClient::reconnect_streaming`]:
    /// saves the active per-contract and full-stream subscriptions
    /// against the old session, opens a fresh FPSS connection under
    /// the previously installed callback, and re-applies the saved
    /// subscriptions. Per-subscription failures during restore are
    /// surfaced as a single `RuntimeError` that names every contract
    /// that did not re-subscribe — the streaming session itself is
    /// already up at that point. Without this restore step a Python
    /// caller observing a transient disconnect would lose every
    /// subscription, breaking parity with the unified client and the
    /// C ABI (`tdx_fpss_reconnect`).
    fn reconnect(&self, py: Python<'_>) -> PyResult<()> {
        let stored = {
            let guard = self.lock_callback();
            match guard.as_ref() {
                Some(cb) => Python::attach(|py| cb.clone_ref(py)),
                None => {
                    return Err(PyRuntimeError::new_err(
                        "no callback registered -- call start_streaming(callback) before reconnect()",
                    ));
                }
            }
        };

        // 1. Snapshot the active subscriptions BEFORE stopping. The
        //    `with_live` borrow handles the not-yet-started case by
        //    returning `RuntimeError` — `reconnect()` only makes
        //    sense on a previously-started session anyway, so the
        //    error message is the same shape the unified client uses.
        let (per_contract, full_stream) =
            self.with_live(|c| Ok((c.active_subscriptions(), c.active_full_subscriptions())))?;

        // 2. Stop + restart under the same callback. `start_streaming`
        //    repopulates `self.callback` with a freshly owned handle
        //    so subsequent `reconnect()` calls find the same state
        //    shape.
        self.stop_streaming(py);
        self.start_streaming(stored)?;

        // 3. Re-apply every saved subscription against the freshly
        //    reconnected session through the core's paced replay
        //    engine — bursts with a jittered pause between them, the
        //    same cadence the auto-reconnect path uses, so a large
        //    saved set is not fired at a recovering upstream
        //    back-to-back. Failures accumulate (the FPSS protocol has
        //    no batched-transaction semantic) and surface as a single
        //    error naming everything that did not restore; the
        //    streaming session itself is already up at that point.
        //    The replay sleeps between bursts, so it runs detached
        //    from the interpreter lock.
        let outcome = {
            let inner = {
                let guard = self.lock_inner();
                guard.as_ref().map(std::sync::Arc::clone)
            };
            let Some(inner) = inner else {
                return Err(PyRuntimeError::new_err(
                    "streaming not started -- call start_streaming(callback) first",
                ));
            };
            py.detach(move || inner.restore_subscriptions(&per_contract, &full_stream))
        };
        outcome.map_err(|e| PyRuntimeError::new_err(format!("reconnect succeeded but {e}")))
    }

    /// Block until every superseded streaming session's event ring
    /// consumer has finished firing the registered callback. Returns
    /// `true` once all retired generations have drained, `false` on
    /// timeout. Polls at 1 ms cadence.
    pub(crate) fn await_drain(&self, py: Python<'_>, timeout_ms: u64) -> bool {
        let timeout = Duration::from_millis(timeout_ms);
        py.detach(|| {
            let deadline = Instant::now() + timeout;
            loop {
                let all_drained = {
                    let mut guard = self
                        .prev_drained
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard.retain(|f| !f.load(Ordering::Acquire));
                    guard.is_empty()
                };
                if all_drained {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        })
    }

    /// Open a context-managed FPSS streaming session.
    ///
    /// `with fpss.streaming(callback) as session:` registers
    /// `callback` via `start_streaming` on enter and pairs
    /// `stop_streaming()` + `await_drain(5000)` on exit — same RAII
    /// semantics as the unified client's `streaming()` helper.
    fn streaming(
        slf: Py<Self>,
        py: Python<'_>,
        callback: Py<PyAny>,
    ) -> PyResult<Py<StreamingSession>> {
        Py::new(
            py,
            StreamingSession {
                tdx: StreamableHandle::Fpss(slf),
                callback: Some(callback),
            },
        )
    }
}
