//! Standalone Python `FpssClient` pyclass.
//!
//! Opens ONLY the FPSS TLS transport — no MDDS gRPC channel, no Nexus
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
//!    The FPSS TLS connection is opened lazily by `start_streaming*`
//!    (matching the FFI's deferred-connect contract).
//! 2. `start_streaming(callback)` or `start_streaming_iter()` — opens
//!    the FPSS TLS connection and starts the LMAX Disruptor consumer.
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
use thetadatadx::fpss::{self, FpssClient as RustFpssClient, FpssConnectArgs};

use crate::buffered_event_to_typed;
use crate::errors::to_py_err;
use crate::event_iterator::EventIterator;
use crate::fluent;
use crate::fpss_event_to_buffered;
use crate::streaming_iter_session::StreamingIterSession;
use crate::streaming_session::StreamingSession;
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
            derive_ohlcvc: config.fpss.derive_ohlcvc,
            connect_timeout_ms: config.fpss.connect_timeout_ms,
            read_timeout_ms: config.fpss.timeout_ms,
            ping_interval_ms: config.fpss.ping_interval_ms,
        }
    }

    fn args(&self) -> FpssConnectArgs<'_> {
        FpssConnectArgs {
            creds: &self.creds,
            hosts: &self.hosts,
            ring_size: self.ring_size,
            flush_mode: self.flush_mode,
            policy: self.policy.clone(),
            derive_ohlcvc: self.derive_ohlcvc,
            connect_timeout_ms: self.connect_timeout_ms,
            read_timeout_ms: self.read_timeout_ms,
            ping_interval_ms: self.ping_interval_ms,
        }
    }
}

/// Standalone FPSS-only streaming client.
///
/// Opens ONLY the FPSS TLS transport — no MDDS gRPC channel, no Nexus
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
/// # ... events arrive on the Disruptor consumer thread ...
/// fpss.stop_streaming()
/// ```
#[pyclass(module = "thetadatadx", name = "FpssClient")]
pub(crate) struct FpssClient {
    /// Connect parameters captured at construction time. Reused on
    /// every `start_streaming*` / `reconnect`.
    params: FpssParams,
    /// Currently-open inner FPSS client. `None` between construction
    /// and `start_streaming*`, and after `stop_streaming` / `shutdown`.
    inner: Mutex<Option<RustFpssClient>>,
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
    /// can layer multiple in-flight Disruptor consumers, and
    /// `await_drain` must wait for all of them before reporting
    /// quiescence.
    prev_drained: Mutex<Vec<Arc<AtomicBool>>>,
}

impl FpssClient {
    fn lock_inner(&self) -> MutexGuard<'_, Option<RustFpssClient>> {
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
            PyRuntimeError::new_err(
                "streaming not started -- call start_streaming(callback) or start_streaming_iter() first",
            )
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
    /// No MDDS gRPC channel is opened. No Nexus HTTP request is issued.
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
        })
    }

    fn __repr__(&self) -> String {
        let connected = self.lock_inner().is_some();
        let hosts = self.params.hosts.len();
        format!("FpssClient(connected={connected}, hosts={hosts})")
    }

    /// Open the FPSS TLS connection and register the Python callback
    /// for incoming events.
    ///
    /// The LMAX Disruptor consumer thread acquires the GIL via
    /// `Python::attach` to invoke `callback(event)` for every typed
    /// FPSS event, with each invocation wrapped in `catch_unwind`.
    /// `callback` must accept exactly one positional argument — a
    /// typed FPSS event class (`Quote`, `Trade`, `Ohlcvc`, … the same
    /// hierarchy emitted on the unified client's callback path).
    ///
    /// The reader never blocks on user code; on ring overflow events
    /// are dropped and counted via `dropped_event_count()`. User
    /// callback panics are caught and counted via `panic_count()`.
    fn start_streaming(&self, callback: Py<PyAny>) -> PyResult<()> {
        let mut cb_guard = self.lock_callback();
        if self.lock_inner().is_some() {
            return Err(PyRuntimeError::new_err(
                "streaming already started -- call stop_streaming() before start_streaming() again",
            ));
        }

        let callback_arc: Arc<Py<PyAny>> = Arc::new(callback);
        let dispatch_cb = Arc::clone(&callback_arc);

        let client = RustFpssClient::connect(self.params.args(), move |event: &fpss::FpssEvent| {
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
                }
            });
        })
        .map_err(to_py_err)?;

        *self.lock_inner() = Some(client);
        // The Disruptor consumer closure has already captured a clone
        // of `callback_arc` (via `dispatch_cb`), so the refcount on
        // `callback_arc` is guaranteed to be at least 2 by the time
        // we reach this line — `Arc::try_unwrap` would always fail.
        // Lift a fresh owned `Py<PyAny>` handle under the GIL for
        // storage on `cb_guard`; the cost is one refcount bump per
        // `start_streaming` call, which is the same lifetime
        // accounting the unified client does.
        *cb_guard = Some(Python::attach(|py| callback_arc.clone_ref(py)));
        Ok(())
    }

    /// Open the FPSS TLS connection in pull-iter delivery mode and
    /// return an [`EventIterator`] handle the caller drains on its own
    /// thread.
    ///
    /// Push-callback and pull-iter are mutually exclusive — calling
    /// this while streaming is already running raises `RuntimeError`.
    fn start_streaming_iter(&self) -> PyResult<EventIterator> {
        if self.lock_inner().is_some() {
            return Err(PyRuntimeError::new_err(
                "streaming already started -- call stop_streaming() before start_streaming_iter()",
            ));
        }
        let (client, iter) = RustFpssClient::connect_iter(self.params.args()).map_err(to_py_err)?;
        *self.lock_inner() = Some(client);
        Ok(EventIterator::new(iter))
    }

    /// Whether the FPSS TLS connection is currently open.
    fn is_streaming(&self) -> bool {
        self.lock_inner().is_some()
    }

    /// Snapshot of per-contract subscriptions on the live session.
    /// Returns an empty list when streaming has not started.
    fn active_subscriptions(&self) -> Vec<std::collections::HashMap<String, String>> {
        let guard = self.lock_inner();
        let Some(client) = guard.as_ref() else {
            return Vec::new();
        };
        client
            .active_subscriptions()
            .into_iter()
            .map(|(kind, contract)| {
                let mut m = std::collections::HashMap::new();
                m.insert("kind".to_string(), format!("{kind:?}"));
                m.insert("contract".to_string(), format!("{contract}"));
                m
            })
            .collect()
    }

    /// Snapshot of full-stream subscriptions (e.g. `SecType.OPTION.full_trades()`).
    /// Returns an empty list when streaming has not started.
    fn active_full_subscriptions(&self) -> Vec<std::collections::HashMap<String, String>> {
        let guard = self.lock_inner();
        let Some(client) = guard.as_ref() else {
            return Vec::new();
        };
        client
            .active_full_subscriptions()
            .into_iter()
            .map(|(kind, sec_type)| {
                let mut m = std::collections::HashMap::new();
                m.insert("kind".to_string(), format!("{kind:?}"));
                m.insert("sec_type".to_string(), format!("{sec_type:?}"));
                m
            })
            .collect()
    }

    /// Cumulative count of FPSS events the TLS reader could not
    /// publish into the Disruptor ring because the consumer fell
    /// behind. Snapshot the value BEFORE `reconnect()` if you need to
    /// accumulate drops across session boundaries — `reconnect`
    /// rebuilds the inner client and the counter resets.
    fn dropped_event_count(&self) -> u64 {
        let guard = self.lock_inner();
        guard.as_ref().map_or(0, |c| c.dropped_count())
    }

    /// Cumulative count of user-callback panics caught by the
    /// Disruptor consumer's `catch_unwind` boundary.
    fn panic_count(&self) -> u64 {
        let guard = self.lock_inner();
        guard.as_ref().map_or(0, |c| c.panic_count())
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
    fn subscribe_many(&self, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        let list = fluent::coerce_subscription_list(subs)?;
        self.with_live(|c| {
            for sub in list {
                c.subscribe(sub)?;
            }
            Ok(())
        })
    }

    /// Polymorphic unsubscribe.
    fn unsubscribe(&self, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        let inner = fluent::coerce_subscription(sub)?;
        self.with_live(|c| c.unsubscribe(inner))
    }

    /// Bulk-unsubscribe.
    fn unsubscribe_many(&self, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        let list = fluent::coerce_subscription_list(subs)?;
        self.with_live(|c| {
            for sub in list {
                c.unsubscribe(sub)?;
            }
            Ok(())
        })
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
    fn stop_streaming(&self) {
        let mut cb_guard = self.lock_callback();
        let mut inner_guard = self.lock_inner();
        if let Some(client) = inner_guard.as_ref() {
            self.prev_drained
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(client.drained_flag());
            client.shutdown();
        }
        *inner_guard = None;
        *cb_guard = None;
    }

    /// Alias for `stop_streaming`. Mirrors the unified client's split
    /// surface where `shutdown` is documented as the terminal stop —
    /// on the standalone client both names are equivalent.
    fn shutdown(&self) {
        self.stop_streaming();
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
    fn reconnect(&self) -> PyResult<()> {
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
        self.stop_streaming();
        self.start_streaming(stored)?;

        // 3. Re-apply every saved subscription against the freshly
        //    reconnected session. Accumulate failures rather than
        //    aborting on the first one — the FPSS protocol has no
        //    batched-transaction semantic, so any per-subscription
        //    failure leaves a partial state that the caller decides
        //    how to handle. Per-contract Quote / Trade / OpenInterest
        //    plus full-stream Trade / OpenInterest cover every active
        //    subscription shape the core client tracks.
        let mut failed: Vec<String> = Vec::new();
        for (kind, contract) in per_contract {
            let sub = fpss::protocol::Subscription::Contract {
                contract: contract.clone(),
                kind,
            };
            let outcome = self.with_live(|c| c.subscribe(sub.clone()));
            if outcome.is_err() {
                failed.push(format!("per-contract {kind:?} {contract}"));
            }
        }
        for (kind, sec_type) in full_stream {
            let full_kind = match kind {
                SubscriptionKind::Trade => Some(FullSubscriptionKind::Trades),
                SubscriptionKind::OpenInterest => Some(FullSubscriptionKind::OpenInterest),
                // Quote is never a full-stream subscription kind on
                // the FPSS wire — the core's `active_full_subscriptions`
                // never returns it, so this arm is unreachable in
                // practice and we drop it silently rather than
                // misclassifying it as a restore failure.
                SubscriptionKind::Quote => None,
            };
            if let Some(full_kind) = full_kind {
                let sub = fpss::protocol::Subscription::Full {
                    sec_type,
                    kind: full_kind,
                };
                let outcome = self.with_live(|c| c.subscribe(sub.clone()));
                if outcome.is_err() {
                    failed.push(format!("full-stream {kind:?} {sec_type:?}"));
                }
            }
        }
        if failed.is_empty() {
            Ok(())
        } else {
            Err(PyRuntimeError::new_err(format!(
                "reconnect succeeded but {} subscription(s) failed to restore: {}",
                failed.len(),
                failed.join(", "),
            )))
        }
    }

    /// Block until every superseded streaming session's Disruptor
    /// consumer has finished firing the registered callback. Returns
    /// `true` once all retired generations have drained, `false` on
    /// timeout. Polls at 1 ms cadence.
    fn await_drain(&self, py: Python<'_>, timeout_ms: u64) -> bool {
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
                tdx: slf.into_any(),
                callback: Some(callback),
            },
        )
    }

    /// Open a context-managed pull-iter streaming session.
    ///
    /// `with fpss.streaming_iter() as it: for event in it: ...`
    /// opens the FPSS connection in pull-iter mode on enter, drains
    /// the iterator inside the body, and pairs `stop_streaming()` +
    /// `await_drain(5000)` on exit.
    fn streaming_iter(slf: Py<Self>, py: Python<'_>) -> PyResult<Py<StreamingIterSession>> {
        Py::new(
            py,
            StreamingIterSession {
                tdx: slf.into_any(),
                iterator: None,
            },
        )
    }
}
