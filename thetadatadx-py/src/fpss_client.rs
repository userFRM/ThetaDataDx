//! Standalone Python `StreamingClient` pyclass.
//!
//! Opens ONLY the streaming TLS transport, no market-data channel, no Nexus
//! HTTP auth, no Treasury / Calendar / OHLCVC market-data surface.
//! Mirrors the C++ `thetadatadx::StreamingClient` (`thetadatadx-cpp/include/thetadatadx.hpp`)
//! and the standalone C ABI entry points (`thetadatadx_client_*` in
//! `thetadatadx-ffi/src/streaming.rs`), letting Python users run a streaming-only
//! session alongside an externally-managed market-data process without the
//! bundled [`crate::Client`] preempting the parallel market-data
//! work at the Nexus session layer.
//!
//! # Nexus session behaviour
//!
//! This pyclass does NOT issue a Nexus authentication. The streaming service speaks its
//! own protocol-level `CREDENTIALS` handshake (wire code `0`) on the
//! TLS connection itself; no separate Nexus session UUID is acquired.
//! The cross-binding contract here matches the standalone C ABI:
//! `thetadatadx_client_connect` accepts a `ThetaDataDxCredentials` handle without
//! touching Nexus. Run the bundled [`crate::Client`] (which
//! does authenticate against Nexus) when you need the market-data surface and
//! Nexus session machinery side-by-side.
//!
//! # Lifecycle
//!
//! 1. `StreamingClient(creds, config)` — snapshots the connect parameters.
//!    The streaming TLS connection is opened lazily by `start_streaming`
//!    (matching the FFI's deferred-connect contract).
//! 2. `start_streaming(callback)` — opens the streaming TLS connection and
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
use thetadatadx::fpss::{self, StreamingClient as RustStreamingClient};
use thetadatadx::DispatcherSession as PyFpssDispatcherSession;

use crate::errors::to_py_err;
use crate::fluent::{self, PySubscription};
use crate::fpss_event_to_typed;
use crate::streaming_session::{StreamableHandle, StreamingSession};
use crate::{Config, Credentials};

/// Snapshot of the parameters required to open a streaming TLS connection.
///
/// Cloned out of the user's `Config` at construction time so subsequent
/// Python-side mutations of the `Config` handle cannot retroactively
/// change reconnect behaviour for an already-running session — the same
/// snapshot semantics the FFI uses in
/// `thetadatadx-ffi/src/streaming.rs::StreamingConnectParams`.
///
/// The whole [`StreamingConfig`] and [`ReconnectConfig`] are snapshotted
/// wholesale rather than copied field by field, so a new tuning knob
/// added to either config cannot drift out of the standalone connect
/// path the way a hand-maintained subset did.
///
/// [`StreamingConfig`]: thetadatadx::config::StreamingConfig
/// [`ReconnectConfig`]: thetadatadx::config::ReconnectConfig
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
    /// unified client's connect path (`thetadatadx-rs/src/client.rs`)
    /// and the C ABI (`thetadatadx-ffi/src/streaming.rs::streaming_builder`) so the
    /// standalone client honours the full streaming and reconnect surface.
    fn builder(&self) -> fpss::StreamingClientBuilder<'_> {
        fpss::StreamingClientBuilder::new(&self.creds, self.streaming.hosts())
            .ring_size(self.streaming.ring_size)
            .consumer_cpu(self.streaming.consumer_cpu)
            .reconnect_policy(self.reconnect.policy.clone())
            .reconnect_wait_ms(self.reconnect.wait_ms)
            .reconnect_wait_max_ms(self.reconnect.wait_max_ms)
            .reconnect_wait_rate_limited_ms(self.reconnect.wait_rate_limited_ms)
            .reconnect_wait_server_restart_ms(self.reconnect.wait_server_restart_ms)
            .reconnect_jitter(self.reconnect.jitter)
            .reconnect_replay_burst_size(self.reconnect.replay_burst_size)
            .reconnect_replay_pace_ms(self.reconnect.replay_pace_ms)
            .connect_timeout_ms(self.streaming.connect_timeout_ms)
            .read_timeout_ms(self.streaming.timeout_ms)
            .ping_interval_ms(self.streaming.ping_interval_ms)
            .io_read_slice_ms(self.streaming.io_read_slice_ms)
            .keepalive_idle_secs(self.streaming.keepalive_idle_secs)
            .keepalive_interval_secs(self.streaming.keepalive_interval_secs)
            .keepalive_retries(self.streaming.keepalive_retries)
            .host_selection(self.streaming.host_selection)
            .host_shuffle_seed(self.streaming.host_shuffle_seed)
    }
}

/// Standalone streaming-only client.
///
/// Opens ONLY the streaming TLS transport, no market-data channel, no Nexus
/// HTTP authentication. Use when a parallel market-data process is already
/// running in the same environment and you need to test streaming without
/// the bundled [`crate::Client`] taking over the Nexus
/// session at construction time.
///
/// ```python
/// from thetadatadx import StreamingClient, Credentials, Config, Contract
///
/// creds = Credentials.from_file("creds.txt")
/// streaming = StreamingClient(creds, Config.production())
///
/// def on_event(event):
///     print(event.kind, event)
///
/// streaming.start_streaming(callback=on_event)
/// streaming.subscribe(Contract.stock("AAPL").quote())
/// # ... events arrive on the event-dispatch consumer thread ...
/// streaming.stop_streaming()
/// ```
// `frozen` — every `#[pymethods]` entry takes `&self` (never
// `&mut self`). The inner `Arc<Mutex<Option<fpss::StreamingClient>>>`
// carries its own interior mutability; the pyclass shell is
// immutable. A future `&mut self` regression surfaces as a
// `cargo check` failure rather than slipping silently.
#[pyclass(module = "thetadatadx", name = "StreamingClient", frozen)]
pub(crate) struct StreamingClient {
    /// Connect parameters captured at construction time. Reused on
    /// every `start_streaming*` / `reconnect`.
    params: FpssParams,
    /// Currently-open inner streaming client. `None` between construction
    /// and `start_streaming*`, and after `stop_streaming` / `shutdown`.
    inner: Mutex<Option<Arc<RustStreamingClient>>>,
    /// Most recently registered Python callable. Retained across
    /// `start_streaming` so `reconnect()` can re-register the same
    /// handler without the caller having to pass it again. Cleared on
    /// `stop_streaming` / `shutdown` so a teardown the application has
    /// already observed does not leak the closure's captured
    /// references — same explicit-handoff model as the unified
    /// [`crate::Client`].
    ///
    /// The callable is held behind an inner `Arc` so a freshly reserved slot
    /// carries a unique identity (`Arc::ptr_eq`): `start_streaming` releases the
    /// callback lock before its blocking connect, so a concurrent stop + restart
    /// can replace the reservation mid-connect; a failed start must clear ONLY
    /// its own slot, never the newer one. Shared with the unified client's
    /// [`crate::CallbackReservation`].
    callback: Mutex<Option<Arc<Py<PyAny>>>>,
    /// Quiescence flags of every superseded streaming session that has
    /// not yet drained. Mirrors the `prev_drained` field on the unified
    /// [`thetadatadx::Client`] — stacked stop/start cycles
    /// can layer multiple in-flight event-dispatch consumers, and
    /// `await_drain` must wait for all of them before reporting
    /// quiescence.
    prev_drained: Mutex<Vec<Arc<AtomicBool>>>,
    /// Dispatcher lifecycle — single mutex replacing
    /// `dispatcher_handle: Mutex<Option<JoinHandle<()>>>` and
    /// `dispatcher_failed: Arc<AtomicBool>`. Panic state is derived
    /// from `JoinHandle::join()` returning `Err(_)`.
    ///
    /// Wrapped in `Arc` so the spawned dispatcher thread can hold an owning
    /// handle to just this slot — the pyclass shell is not itself `Arc`-shared
    /// across the spawn — and publish `Failed` from its own catch-arm the
    /// instant an outer panic kills the event loop (see
    /// [`publish_failed_if_current`]).
    dispatcher: Arc<Mutex<thetadatadx::DispatcherSession>>,
}

impl Drop for StreamingClient {
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
            PyFpssDispatcherSession::Running { handle, .. } => Some(handle),
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

impl StreamingClient {
    /// Shared context-manager teardown for `__exit__` / `__aexit__`: stop
    /// streaming, then block on the drain barrier so the consumer thread has
    /// finished firing the registered callback. A drain timeout is a
    /// `RuntimeWarning` (best-effort observability), not a hard error, because
    /// the pipeline is already stopped and re-raising would swallow any
    /// exception from the `with` body.
    fn exit_teardown(&self, py: Python<'_>) -> PyResult<()> {
        self.stop_streaming(py);
        let drained = self.await_drain(py, crate::streaming_session::EXIT_DRAIN_TIMEOUT_MS);
        if !drained {
            let warnings = py.import("warnings")?;
            let msg = format!(
                "streaming drain timed out after {}ms; consumer callback may still be firing.",
                crate::streaming_session::EXIT_DRAIN_TIMEOUT_MS
            );
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("stacklevel", 2_u32)?;
            warnings.call_method(
                "warn",
                (msg, py.get_type::<pyo3::exceptions::PyRuntimeWarning>()),
                Some(&kwargs),
            )?;
        }
        Ok(())
    }

    fn lock_inner(&self) -> MutexGuard<'_, Option<Arc<RustStreamingClient>>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_callback(&self) -> MutexGuard<'_, Option<Arc<Py<PyAny>>>> {
        self.callback.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run a closure with a borrow of the live streaming client, raising
    /// `RuntimeError` when nothing is connected.
    ///
    /// The user closure runs with the GIL RELEASED: the live client is
    /// an `Arc<RustStreamingClient>` cloned out from under the binding
    /// mutex, so the closure body (a blocking streaming socket write) holds
    /// no Python object and no binding lock. The inner mutex is taken
    /// only briefly to clone the handle, then dropped before the
    /// detached blocking section, so a concurrent `stop_streaming` on a
    /// sibling thread cannot deadlock against an in-flight write. The
    /// `thetadatadx::Error` is mapped to the typed Python exception
    /// AFTER the GIL is re-acquired, leaving the error surface
    /// unchanged.
    fn with_live<R>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&RustStreamingClient) -> Result<R, thetadatadx::Error> + Send,
    ) -> PyResult<R>
    where
        R: Send,
    {
        let client = {
            let guard = self.lock_inner();
            guard.as_ref().map(Arc::clone).ok_or_else(|| {
                PyRuntimeError::new_err(
                    "streaming not started -- call start_streaming(callback) first",
                )
            })?
        };
        py.detach(move || f(&client)).map_err(to_py_err)
    }
}

/// Downcast a thread-panic payload to a human-readable string.
fn downcast_py_panic_payload(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_owned();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "dispatcher panicked with non-string payload".to_owned()
}

/// Publish `Failed` from the dispatcher thread's OWN catch-arm after an outer
/// panic in the event-iteration machinery, so `is_streaming` / `is_authenticated`
/// report the dead loop immediately rather than only after teardown joins the
/// corpse.
///
/// `dispatcher_thread_id` is the id of the thread the session's `JoinHandle`
/// names; the caller passes its own [`std::thread::current`] id. The store
/// happens ONLY when the slot still holds the matching `Running` session: a
/// concurrent `stop_streaming` may have extracted it to `Idle`, or a fresh
/// session (different thread id) may occupy it. Overwriting either would
/// resurrect a torn-down session or clobber a live one's `JoinHandle`.
///
/// Orthogonal to teardown: a mutate-UNDER-lock-then-RELEASE with no join held
/// across the guard, matching the publish in `stop_streaming`'s join path.
fn publish_failed_if_current(
    dispatcher: &Mutex<thetadatadx::DispatcherSession>,
    dispatcher_thread_id: std::thread::ThreadId,
    reason: String,
) {
    let mut guard = dispatcher.lock().unwrap_or_else(|e| e.into_inner());
    if let PyFpssDispatcherSession::Running { handle, .. } = &*guard {
        if handle.thread().id() == dispatcher_thread_id {
            *guard = PyFpssDispatcherSession::Failed { reason };
        }
    }
}

#[pymethods]
impl StreamingClient {
    /// Allocate a standalone streaming handle.
    ///
    /// Snapshots the connect parameters out of the supplied `Config`
    /// but does NOT open the streaming TLS connection. Connection is
    /// deferred to the first `start_streaming*` call. This matches the
    /// C ABI's deferred-connect contract (`thetadatadx_client_connect` allocates
    /// the handle, `thetadatadx_client_set_callback` opens the network) so the
    /// same observable behaviour applies across every binding.
    ///
    /// No market-data channel is opened. No Nexus HTTP request is issued.
    /// A parallel market-data process under the same credentials is unaffected
    /// by this constructor.
    #[new]
    fn new(_py: Python<'_>, creds: &Credentials, config: &Config) -> PyResult<Self> {
        let direct = {
            let guard = config.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        if direct.streaming_hosts().is_empty() {
            return Err(PyValueError::new_err(
                "StreamingClient: config.streaming.hosts is empty (set THETADATA_STREAMING_HOSTS or use Config::production())",
            ));
        }
        // Seed the process-global runtime from this client's runtime config
        // so `worker_threads` is honoured when this is the first client in
        // the process, even though the streaming TLS connection itself is
        // deferred to `start_streaming`.
        crate::runtime_from_config(&direct.runtime);
        Ok(Self {
            params: FpssParams::from_config(&creds.inner, &direct),
            inner: Mutex::new(None),
            callback: Mutex::new(None),
            prev_drained: Mutex::new(Vec::new()),
            dispatcher: Arc::new(Mutex::new(PyFpssDispatcherSession::Idle)),
        })
    }

    /// Convenience constructor: `StreamingClient.from_file("creds.txt")`.
    /// Loads credentials from a two-line file and connects with the
    /// supplied `config`, defaulting to `Config.production()`.
    ///
    /// Parity with `Client.from_file()`,
    /// `AsyncClient.from_file()`, and
    /// `MarketDataClient.from_file()` — every standalone Python client
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
        // Match the bundled `Client.__repr__` key/value vocabulary
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
        let hosts = self.params.streaming.hosts().len();
        format!("StreamingClient(streaming={streaming}, hosts={hosts})")
    }

    /// Sync context-manager entry: returns ``self`` so the ``with`` body can
    /// call ``start_streaming`` / ``subscribe`` on it.
    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    /// Sync context-manager exit: stop streaming and block on the drain
    /// barrier so the consumer thread has finished firing the callback before
    /// this returns, mirroring the TypeScript ``Symbol.dispose`` behavior.
    /// Returns ``False`` so an exception raised inside the ``with`` body is not
    /// swallowed.
    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: Option<Py<PyAny>>,
        _exc_value: Option<Py<PyAny>>,
        _traceback: Option<Py<PyAny>>,
    ) -> PyResult<bool> {
        self.exit_teardown(py)?;
        Ok(false)
    }

    /// Async context-manager entry: returns ``self``.
    fn __aenter__<'py>(slf: Py<Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        pyo3_async_runtimes::tokio::future_into_py(py, async move { Ok(slf) })
            .map(pyo3::Bound::into_any)
    }

    /// Async context-manager exit: stop streaming and drain on a blocking
    /// worker. Resolves to ``False`` so an exception raised in the
    /// ``async with`` body is not swallowed.
    #[pyo3(signature = (_exc_type=None, _exc_value=None, _traceback=None))]
    fn __aexit__<'py>(
        slf: Py<Self>,
        py: Python<'py>,
        _exc_type: Option<Py<PyAny>>,
        _exc_value: Option<Py<PyAny>>,
        _traceback: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            tokio::task::spawn_blocking(move || {
                Python::attach(|py| slf.borrow(py).exit_teardown(py))
            })
            .await
            .map_err(|e| PyRuntimeError::new_err(format!("streaming close task failed: {e}")))??;
            Ok(false)
        })
        .map(pyo3::Bound::into_any)
    }

    /// Open the streaming TLS connection and register the Python callback
    /// for incoming events.
    ///
    /// The event-dispatch consumer thread acquires the GIL via
    /// `Python::attach` to invoke `callback(event)` for every typed
    /// streaming event. Each invocation is individually wrapped in
    /// `catch_unwind`: a panic on event N is caught, recorded via
    /// `panic_count()`, and does not stop event delivery — event N+1
    /// continues normally. `callback` must accept exactly one positional
    /// argument — a typed streaming event class (`Quote`, `Trade`, `Ohlcvc`,
    /// … the same hierarchy emitted on the unified client's callback path).
    ///
    /// The reader never blocks on user code; on ring overflow events
    /// are dropped and counted via `dropped_event_count()`. User
    /// callback panics are caught and counted via `panic_count()`.
    pub(crate) fn start_streaming(&self, py: Python<'_>, callback: Py<PyAny>) -> PyResult<()> {
        let callback_arc: Arc<Py<PyAny>> = Arc::new(callback);
        // Reserve the callback slot in an inner scope and DROP the guard before
        // the GIL-released connect below. Holding the callback mutex across
        // `py.detach` (whose dispatcher re-acquires the GIL) lets a concurrent
        // `stop_streaming` / `reconnect` / second `start_streaming` park on the
        // same mutex WITH the GIL held while this thread blocks on the GIL --
        // a whole-interpreter deadlock. Reject a double-start (either a live
        // session or an in-flight reservation) here, then release the lock.
        {
            let mut cb_guard = self.lock_callback();
            if self.lock_inner().is_some() || cb_guard.is_some() {
                return Err(PyRuntimeError::new_err(
                    "streaming already started -- call stop_streaming() before start_streaming() again",
                ));
            }
            *cb_guard = Some(Arc::clone(&callback_arc));
        }
        // Own the reservation until the connect AND dispatcher spawn succeed. On
        // any non-success exit -- the `?` on a handshake failure, the spawn
        // failure below, or a panic before the session is live -- the guard
        // clears the slot, but ONLY if it still holds THIS reservation
        // (`Arc::ptr_eq`): the lock is dropped across the detached connect, so a
        // concurrent stop + restart may have replaced the slot with a newer
        // callback that must keep its registration. Disarmed on success.
        let mut reservation = crate::CallbackReservation::armed(&self.callback, &callback_arc);
        let dispatch_cb = Arc::clone(&callback_arc);

        // The streaming TLS connect (`StreamingClientBuilder::build`) performs
        // the blocking socket connect + `CREDENTIALS` handshake on the
        // calling thread. Release the GIL across it so a sibling Python
        // thread keeps running while the handshake is in flight. The
        // builder snapshot and the resulting `Result` are pure Rust — no
        // Python object is touched inside the detached region. The
        // `StreamError` is mapped to the typed Python exception AFTER the
        // GIL is re-acquired, leaving the error surface unchanged.
        let client = py
            .detach(|| self.params.builder().build())
            .map_err(|e| to_py_err(thetadatadx::Error::from(e)))?;
        let client_arc = Arc::new(client);

        // Ownership gate: the callback lock was dropped across the connect, so a
        // concurrent `stop_streaming` (clears the slot) or a newer
        // `start_streaming` (replaces it) may have superseded this start. Take
        // the callback lock and publish ONLY while the slot still holds THIS
        // reservation (`Arc::ptr_eq`). Publishing unconditionally would resurrect
        // streaming after a completed stop, or clobber a newer session's inner +
        // dispatcher + callback. The lock is HELD across the publish AND the
        // dispatcher install so the whole handoff is atomic against a concurrent
        // stop (which takes the callback lock first); the dispatcher closure
        // never takes the callback lock, so holding it here cannot re-enter.
        let cb_guard = self.lock_callback();
        if !cb_guard
            .as_ref()
            .is_some_and(|cb| Arc::ptr_eq(cb, &callback_arc))
        {
            // Superseded. Do not publish. Shut the freshly built client down and
            // return; the reservation drops as a no-op (the slot is not ours).
            // Drop the sole client Arc OFF the GIL: `StreamingClient::Drop`
            // inline-joins the I/O thread, which re-acquires the GIL via
            // `Python::attach` if a `Custom` reconnect callback fires on a
            // transient disconnect in this window — joining under the held GIL
            // would deadlock the interpreter. Mirrors `stop_streaming` / `Drop`.
            drop(cb_guard);
            py.detach(move || {
                client_arc.shutdown();
                drop(client_arc);
            });
            return Err(PyRuntimeError::new_err(
                "streaming start superseded by a concurrent stop/start",
            ));
        }
        // Still ours: publish the client BEFORE spawning the dispatcher so the
        // first delivered event sees a fully initialised handle. A re-entrant
        // call from inside the user callback to `subscribe()` / `with_live()` /
        // `is_streaming()` would otherwise race a late publish and observe
        // `inner = None`, raising `RuntimeError("streaming not started")`.
        *self.lock_inner() = Some(Arc::clone(&client_arc));

        let dispatcher_client = Arc::clone(&client_arc);
        // Clone a handle for counting Python exceptions inside the closure.
        // A `PyErr` raised by the callback does not unwind through Rust's
        // `catch_unwind`; `poll_batch` never sees it as a panic. The
        // binding must increment the counter explicitly so `panic_count()`
        // reflects both Rust panics and Python exceptions.
        let panic_recorder = Arc::clone(&client_arc);
        let dispatcher_slot = Arc::clone(&self.dispatcher);
        // Hold the dispatcher lock across the spawn AND the `Running` install so
        // a dispatcher that reaches its fault arm before the parent installs
        // `Running` blocks on this lock (its `publish_failed_if_current` takes
        // it) instead of observing `Idle` and dropping the fault against a slot
        // the parent then overwrites with `Running` for an already-exited
        // thread. The callback lock is already held (order callback -> dispatcher,
        // matching `stop_streaming`); the dispatcher's normal drain never takes
        // this lock, so holding it across the spawn cannot stall delivery.
        let mut dispatcher_guard = self.dispatcher.lock().unwrap_or_else(|e| e.into_inner());
        let dispatcher = std::thread::Builder::new()
            .name("thetadatadx-py-fpss-dispatcher".into())
            .spawn(move || {
                // `StreamingClient::for_each` drives `poll_batch`, which wraps
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
                //
                // GIL once per batch, not per event: `for_each_scoped`
                // brackets each batch drain in the `Python::attach` scope
                // below, so the GIL is acquired once and held across every
                // event in the batch, then released across the idle
                // inter-batch wait. The per-event `Python::attach` inside
                // `dispatch_one` is then the cheap reentrant fast path
                // (the GIL is already held by the scope), not a full
                // acquire-from-detached. This holds the no-GIL discipline:
                // the lock never spans the blocking wait.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let dispatch_one = |event: &fpss::StreamEvent| {
                        Python::attach(|py| {
                            // Borrowed `&StreamEvent` → typed pyclass in one
                            // pass; the contract is stored inline, so the
                            // nested `ContractRef` Python object is built
                            // only when the callback reads `event.contract`.
                            // `call1` with a 1-tuple takes pyo3's
                            // vectorcall fast path (no heap argument tuple).
                            let typed = match fpss_event_to_typed(py, event) {
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
                    };
                    dispatcher_client
                        .for_each_scoped(dispatch_one, |drain| Python::attach(|_py| drain()))
                }));
                match outcome {
                    Err(payload) => {
                        let reason = downcast_py_panic_payload(payload);
                        tracing::error!(
                            target: "thetadatadx::python",
                            reason = %reason,
                            "thetadatadx-py-fpss-dispatcher panicked in event iteration machinery; StreamingClient transitioning to failed state",
                        );
                        // Publish `Failed` from this thread before it exits so
                        // health checks reflect the dead loop immediately, not only
                        // once teardown joins.
                        publish_failed_if_current(
                            &dispatcher_slot,
                            std::thread::current().id(),
                            reason,
                        );
                    }
                    // The FPSS I/O thread unwound: the drain ended on a fault, not
                    // a clean stop. Flip the session to `Failed` so `is_streaming`
                    // reflects the dead loop immediately, matching the Rust core
                    // and the pull path's `DispatcherFailed`.
                    Ok(fpss::PollOutcome::Failed) => {
                        publish_failed_if_current(
                            &dispatcher_slot,
                            std::thread::current().id(),
                            "fpss io thread terminated abnormally".to_string(),
                        );
                    }
                    Ok(_) => {}
                }
            });
        match dispatcher {
            Ok(h) => {
                // The callback dispatcher parks only on the event ring, which
                // the client shutdown signals on teardown, so no wake hook.
                // Written through the guard held across the spawn, so a racing
                // fault publish serialises AFTER this `Running` install.
                *dispatcher_guard = PyFpssDispatcherSession::Running {
                    handle: h,
                    on_teardown: None,
                    // Python runs its own teardown and never reads this flag.
                    registers_drain_flag: true,
                };
                drop(dispatcher_guard);
                // Connect + spawn succeeded: hand the reservation off to the
                // live session so the guard does not clear it on return, then
                // release the callback lock.
                reservation.disarm();
                drop(cb_guard);
            }
            Err(e) => {
                // Owner: the callback lock is held from the gate above, so no
                // concurrent stop could have run and `inner` still holds this
                // start's client. Clear it and shut the client down. Release the
                // callback lock BEFORE returning so the reservation can clear the
                // slot on drop (ownership-checked) without a double-lock.
                drop(dispatcher_guard);
                // Drops a non-last clone (`client_arc` still holds a ref), so
                // this is cheap and never joins the I/O thread under the GIL.
                *self.lock_inner() = None;
                drop(cb_guard);
                // Drop the last client Arc OFF the GIL so `StreamingClient::Drop`
                // inline-joins the I/O thread without holding the GIL a `Custom`
                // reconnect callback's `Python::attach` would block on. Mirrors
                // the superseded path above and `stop_streaming` / `Drop`.
                py.detach(move || {
                    client_arc.shutdown();
                    drop(client_arc);
                });
                return Err(PyRuntimeError::new_err(format!(
                    "failed to spawn streaming dispatcher thread: {e}"
                )));
            }
        }
        Ok(())
    }

    /// Whether the streaming TLS connection is currently open.
    ///
    /// Returns `false` when the dispatcher thread panicked — no events
    /// are arriving even though the TLS slot is still populated, so
    /// callers must observe the failed state.
    fn is_streaming(&self) -> bool {
        if self.lock_inner().as_ref().is_none() {
            return false;
        }
        // Copy the failure reason out and DROP both binding mutexes before
        // tracing: a user log handler that re-enters a pyclass method would
        // otherwise deadlock on the non-reentrant mutex still held here.
        let failed_reason = match &*self.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) {
            PyFpssDispatcherSession::Failed { reason } => Some(reason.clone()),
            _ => None,
        };
        match failed_reason {
            Some(reason) => {
                tracing::debug!(
                    target: "thetadatadx::python",
                    reason = %reason,
                    "is_streaming: dispatcher failed",
                );
                false
            }
            None => true,
        }
    }

    /// Whether the streaming session is currently authenticated.
    ///
    /// Mirrors the C++ `thetadatadx::StreamingClient::is_authenticated()` getter and
    /// the C ABI `thetadatadx_client_is_authenticated`. Distinct from
    /// `is_streaming()`: the TLS slot can hold an `RustStreamingClient` whose
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
    /// streaming wire, so the core's `active_full_subscriptions` only ever
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

    /// Cumulative count of streaming events the TLS reader could not
    /// publish into the event ring because the consumer fell
    /// behind. Snapshot the value BEFORE `reconnect()` if you need to
    /// accumulate drops across session boundaries — `reconnect`
    /// rebuilds the inner client and the counter resets.
    fn dropped_event_count(&self) -> u64 {
        let guard = self.lock_inner();
        guard.as_ref().map_or(0, |c| c.dropped_count())
    }

    /// Point-in-time count of events published into the event ring
    /// but not yet drained into your callback — the in-flight depth
    /// between the I/O thread and the dispatcher. The leading
    /// back-pressure signal: :meth:`dropped_event_count` only moves
    /// AFTER data has been lost, while a rising occupancy that
    /// approaches :meth:`ring_capacity` predicts those drops while
    /// there is still time to react. Sampling never blocks the feed.
    /// Returns 0 when no session is live.
    fn ring_occupancy(&self) -> usize {
        let guard = self.lock_inner();
        guard.as_ref().map_or(0, |c| c.ring_occupancy())
    }

    /// Configured capacity of the event ring in slots (the
    /// ``streaming_ring_size`` setting, a power of two) — the fixed
    /// denominator for :meth:`ring_occupancy`. Returns 0 when no
    /// session is live.
    fn ring_capacity(&self) -> usize {
        let guard = self.lock_inner();
        guard.as_ref().map_or(0, |c| c.ring_capacity())
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
    fn subscribe(&self, py: Python<'_>, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        let inner = fluent::coerce_subscription(sub)?;
        self.with_live(py, move |c| c.subscribe(inner))
    }

    /// Bulk-subscribe a list of `Subscription` values.
    ///
    /// Each iteration clones the live handle out from under the inner
    /// mutex and releases the GIL across the blocking wire write (via
    /// `with_live`), so a concurrent `stop_streaming` on a sibling
    /// thread cannot deadlock against a long batch. The core streaming
    /// protocol has no batched-subscribe wire frame today; a future
    /// single-command `subscribe_many` on
    /// `thetadatadx-rs/src/fpss/mod.rs` is tracked as a follow-up
    /// and would route through here without an API change.
    fn subscribe_many(&self, py: Python<'_>, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        let list = fluent::coerce_subscription_list(subs)?;
        for sub in list {
            self.with_live(py, move |c| c.subscribe(sub))?;
        }
        Ok(())
    }

    /// Polymorphic unsubscribe.
    fn unsubscribe(&self, py: Python<'_>, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        let inner = fluent::coerce_subscription(sub)?;
        self.with_live(py, move |c| c.unsubscribe(inner))
    }

    /// Bulk-unsubscribe.
    ///
    /// Clones the live handle out and releases the GIL across each
    /// blocking wire write — same rationale as `subscribe_many`.
    fn unsubscribe_many(&self, py: Python<'_>, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        let list = fluent::coerce_subscription_list(subs)?;
        for sub in list {
            self.with_live(py, move |c| c.unsubscribe(sub))?;
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
    /// they acquire the two `Mutex` slots — `start_streaming`
    /// releases the GIL across the streaming connect (via `py.detach`), so
    /// concurrent `start_streaming` / `stop_streaming` can interleave
    /// on the same handle. Pinning the ordering here keeps that
    /// interleaving deadlock-free.
    pub(crate) fn stop_streaming(&self, py: Python<'_>) {
        // Take the `Arc<RustStreamingClient>` out of `inner` and the stored
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
            {
                let mut prev = self
                    .prev_drained
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                // Prune prior sessions that have finished draining before
                // pushing this one, so repeated stop/start cycles that never
                // call `await_drain` cannot grow this vec unboundedly. Bounds it
                // to the sessions still draining (mirrors `await_drain`'s retain).
                prev.retain(|f| !f.load(Ordering::Acquire));
                prev.push(client.drained_flag());
            }
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
                if let PyFpssDispatcherSession::Running { handle, .. } = prev_session {
                    if handle.thread().id() != std::thread::current().id() {
                        if let Err(payload) = handle.join() {
                            let reason = downcast_py_panic_payload(payload);
                            tracing::error!(
                                target: "thetadatadx::python",
                                reason = %reason,
                                "thetadatadx-py-fpss-dispatcher panicked; StreamingClient marked as failed",
                            );
                            // Record `Failed` ONLY if the slot is still `Idle`:
                            // the dispatcher lock was released across the join,
                            // so a concurrent restart may have installed a fresh
                            // `Running` in that window. The panic belongs to the
                            // superseded OLD session; overwriting unconditionally
                            // would clobber the new session's handle (orphaning
                            // its thread) and falsely fail a healthy live session.
                            // Matches the FFI `join_extracted_session` guard.
                            let mut guard =
                                dispatcher_ref.lock().unwrap_or_else(|e| e.into_inner());
                            if matches!(*guard, PyFpssDispatcherSession::Idle) {
                                *guard = PyFpssDispatcherSession::Failed { reason };
                            }
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

    /// Re-open the streaming connection and re-register the previously
    /// installed callback. Requires a prior `start_streaming(callback)`;
    /// raises `RuntimeError` otherwise.
    ///
    /// Mirrors [`thetadatadx::Client::reconnect_streaming`]:
    /// saves the active per-contract and full-stream subscriptions
    /// against the old session, opens a fresh streaming connection under
    /// the previously installed callback, and re-applies the saved
    /// subscriptions. Per-subscription failures during restore are
    /// surfaced as a single `RuntimeError` that names every contract
    /// that did not re-subscribe — the streaming session itself is
    /// already up at that point. Without this restore step a Python
    /// caller observing a transient disconnect would lose every
    /// subscription, breaking parity with the unified client and the
    /// C ABI (`thetadatadx_client_reconnect`).
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
        let (per_contract, full_stream) = self.with_live(py, |c| {
            Ok((c.active_subscriptions(), c.active_full_subscriptions()))
        })?;

        // 2. Stop + restart under the same callback. `start_streaming`
        //    repopulates `self.callback` with a freshly owned handle
        //    so subsequent `reconnect()` calls find the same state
        //    shape.
        self.stop_streaming(py);
        self.start_streaming(py, stored)?;

        // 3. Re-apply every saved subscription against the freshly
        //    reconnected session through the core's paced replay
        //    engine — bursts with a jittered pause between them, the
        //    same cadence the auto-reconnect path uses, so a large
        //    saved set is not fired at a recovering upstream
        //    back-to-back. Failures accumulate (the streaming protocol has
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
        // Route the partial-restore failure through the typed-exception
        // mapper so a `PartialReconnect` surfaces as `StreamError` — the
        // same leaf the unified `StreamView.reconnect` raises — rather
        // than a bare `RuntimeError` that `except StreamError` misses.
        outcome.map_err(to_py_err)
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

    /// Open a context-managed streaming session.
    ///
    /// `with streaming_client.streaming(callback) as session:` registers
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
                client: StreamableHandle::Fpss(slf),
                callback: Some(callback),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thetadatadx::config::{
        HostSelectionPolicy, JitterMode, ReconnectPolicy,
    };

    /// Anti-drift guard for the standalone connect path.
    ///
    /// `FpssParams` snapshots the whole `StreamingConfig` + `ReconnectConfig`
    /// and `builder()` threads every field into the `StreamingClientBuilder`,
    /// so the standalone Python `StreamingClient` honours the same streaming
    /// and reconnect surface as the unified client and the C ABI. This test
    /// sets every streaming and reconnect knob to a non-default value and
    /// asserts each one survives the snapshot. A future field that
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
        config.streaming.keepalive_idle_secs = 66;
        config.streaming.keepalive_interval_secs = 77;
        config.streaming.keepalive_retries = 8;
        config.streaming.consumer_cpu = Some(3);

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
        assert_eq!(s.keepalive_idle_secs, 66);
        assert_eq!(s.keepalive_interval_secs, 77);
        assert_eq!(s.keepalive_retries, 8);
        assert_eq!(s.consumer_cpu, Some(3));

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

    /// An OUTER dispatcher panic (the event-iteration machinery, not a user
    /// callback) must flip `is_streaming()` / `is_authenticated()` to `false`
    /// IMMEDIATELY — from the dispatcher thread's own catch-arm — rather than
    /// staying healthy until teardown joins the dead thread. Pins
    /// [`super::publish_failed_if_current`], the catch-arm publish
    /// `start_streaming` routes through.
    #[test]
    fn outer_panic_flips_health_checks_to_failed_immediately() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use thetadatadx::fpss::HarnessPublishMode;

        // A live harness client: `for_self_join_test` flips its `authenticated`
        // flag `true`, so absent a `Failed` dispatcher both readers report
        // healthy.
        let client = RustStreamingClient::for_self_join_test(
            1,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            |_event| {},
        );
        let drained = client.drained_flag();

        // A stand-in for the dispatcher thread whose `JoinHandle` the `Running`
        // session carries; it parks until released so the handle stays joinable
        // while the test drives the catch-arm publish.
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

        let creds = RustCredentials::new("user@example.com", "secret");
        let config = DirectConfig::production();
        let sc = StreamingClient {
            params: FpssParams::from_config(&creds, &config),
            inner: Mutex::new(Some(client)),
            callback: Mutex::new(None),
            prev_drained: Mutex::new(Vec::new()),
            dispatcher: Arc::new(Mutex::new(PyFpssDispatcherSession::Running {
                handle: dispatcher_handle,
                on_teardown: None,
                registers_drain_flag: true,
            })),
        };

        // Healthy before the panic.
        assert!(
            sc.is_streaming(),
            "a live Running session must report streaming"
        );
        assert!(
            sc.is_authenticated(),
            "a live authenticated session must report authenticated before any panic"
        );

        // A non-matching thread id must NOT publish (models a fresh session
        // installed by a concurrent restart in the lock-release window).
        publish_failed_if_current(
            &sc.dispatcher,
            std::thread::current().id(),
            "wrong-thread panic must not clobber".to_owned(),
        );
        assert!(
            sc.is_streaming(),
            "publish_failed_if_current must not overwrite a session owned by a different thread"
        );

        // The dispatcher thread's own catch-arm publishes `Failed`.
        publish_failed_if_current(
            &sc.dispatcher,
            dispatcher_thread_id,
            "intentional outer-machinery panic".to_owned(),
        );

        // Both status readers now report the dead loop, with no teardown join.
        assert!(
            !sc.is_streaming(),
            "is_streaming must return false immediately after an outer dispatcher panic"
        );
        assert!(
            !sc.is_authenticated(),
            "is_authenticated must return false immediately after an outer dispatcher panic"
        );

        // Release the parked stand-in (its handle was detached into `Failed`),
        // shut the harness client down, and wait for its consumer to drain so
        // nothing outlives the test.
        release.store(true, Ordering::Release);
        if let Some(client) = sc.inner.lock().unwrap_or_else(|e| e.into_inner()).take() {
            client.shutdown();
            drop(client);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !drained.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "harness client did not drain within 5 s"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
