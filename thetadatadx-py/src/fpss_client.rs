//! Standalone Python `StreamingClient` pyclass.
//!
//! Opens ONLY the streaming TLS transport, no market-data channel, no Nexus
//! HTTP auth, no Treasury / Calendar / OHLCVC market-data surface.
//! Mirrors the C++ `thetadatadx::StreamingClient` (`thetadatadx-cpp/include/thetadatadx.hpp`)
//! and the standalone C ABI entry points (`thetadatadx_streaming_*` in
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
//!
//! # Lifecycle
//!
//! The client is a [`crate::StreamView`] over a core
//! [`thetadatadx::StreamService`] it owns alone: every streaming method runs
//! the body `client.stream` runs, over the same core lifecycle.

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use std::sync::{Arc, Mutex};

use crate::errors::to_py_err;
use crate::fluent::PySubscription;
use crate::streaming_session::{StreamableHandle, StreamingSession};
use crate::{Config, Credentials, StreamView};

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
#[pyclass(module = "thetadatadx", name = "StreamingClient", frozen)]
pub(crate) struct StreamingClient {
    view: StreamView,
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
}

#[pymethods]
impl StreamingClient {
    /// Allocate a standalone streaming handle.
    ///
    /// Snapshots the connect parameters out of the supplied `Config`
    /// but does NOT open the streaming TLS connection. Connection is
    /// deferred to the first `start_streaming*` call. This matches the
    /// C ABI's deferred-connect contract (`thetadatadx_streaming_connect` allocates
    /// the handle, `thetadatadx_streaming_set_callback` opens the network) so the
    /// same observable behaviour applies across every binding.
    ///
    /// No market-data channel is opened. No Nexus HTTP request is issued.
    /// A parallel market-data process under the same credentials is unaffected
    /// by this constructor.
    #[new]
    fn new(_py: Python<'_>, creds: &Credentials, config: &Config) -> PyResult<Self> {
        let direct = config
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        // Validated here, as the unified client's connect validates it, so an
        // out-of-range value is refused naming its field.
        let service = thetadatadx::StreamService::new(&creds.inner, direct).map_err(to_py_err)?;
        // Seed the process-global runtime from this client's runtime config
        // so `worker_threads` is honoured when this is the first client in
        // the process, even though the streaming TLS connection itself is
        // deferred to `start_streaming`.
        crate::runtime_from_config(&service.config().runtime);
        Ok(Self {
            view: StreamView {
                stream: Arc::new(service),
                callback: Arc::new(Mutex::new(None)),
            },
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
                owned_default = Config::production()?;
                &owned_default
            }
        };
        Self::new(py, &creds, cfg)
    }

    fn __repr__(&self) -> String {
        // Match the bundled `Client.__repr__` key/value vocabulary
        // (`streaming=connected` / `streaming=none`). `is_streaming()` is
        // failure-aware, so a panicked dispatcher reports `streaming=none`.
        let streaming = if self.is_streaming() {
            "connected"
        } else {
            "none"
        };
        let hosts = self.view.stream.config().streaming.hosts().len();
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
        self.view.start_streaming(py, callback)
    }

    /// Whether the streaming TLS connection is currently open.
    ///
    /// Returns `false` when the dispatcher thread panicked — no events
    /// are arriving even though the TLS slot is still populated, so
    /// callers must observe the failed state.
    fn is_streaming(&self) -> bool {
        self.view.is_streaming()
    }

    /// Whether the streaming session is currently authenticated.
    ///
    /// Mirrors the C++ `thetadatadx::StreamingClient::is_authenticated()` getter and
    /// the C ABI `thetadatadx_streaming_is_authenticated`. Distinct from
    /// `is_streaming()`: the live session can have its `authenticated` flag
    /// flipped to `false` after a server disconnect, before the application
    /// has issued `reconnect()`.
    ///
    /// A panicked dispatcher thread also folds back to `false` here so
    /// the failed state is uniformly visible across every status reader,
    /// not just `is_streaming()`.
    fn is_authenticated(&self) -> bool {
        self.view.is_authenticated()
    }

    /// Snapshot of per-contract subscriptions on the live session.
    ///
    /// Returns the same typed `Subscription` values the caller passes to
    /// `subscribe()` — round-trippable rather than a debug-format
    /// string projection. Raises `NetworkError` when streaming has not
    /// started.
    fn active_subscriptions(&self) -> PyResult<Vec<PySubscription>> {
        self.view.active_subscriptions()
    }

    /// Snapshot of full-stream subscriptions (e.g. `SecType.OPTION.full_trades()`).
    ///
    /// Returns the same typed `Subscription` values the caller passes to
    /// `subscribe()`. Quote is never a valid full-stream kind on the
    /// streaming wire, so any such row is dropped from the projection.
    /// Raises `NetworkError` when streaming has not started.
    fn active_full_subscriptions(&self) -> PyResult<Vec<PySubscription>> {
        self.view.active_full_subscriptions()
    }

    /// Cumulative count of streaming events the TLS reader could not
    /// publish into the event ring because the consumer fell behind,
    /// counted across every session this handle has run. A `reconnect()`
    /// rebuilds the inner client; the drops the previous session recorded
    /// stay in the total.
    fn dropped_event_count(&self) -> u64 {
        self.view.dropped_event_count()
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
        self.view.ring_occupancy()
    }

    /// Configured capacity of the event ring in slots (the
    /// ``streaming_ring_size`` setting, a power of two) — the fixed
    /// denominator for :meth:`ring_occupancy`. Returns 0 when no
    /// session is live.
    fn ring_capacity(&self) -> usize {
        self.view.ring_capacity()
    }

    /// Cumulative count of user-callback faults: Rust panics caught by the
    /// per-invocation `catch_unwind` boundary, and Python exceptions raised
    /// inside the callback (surfaced via `sys.unraisablehook`). Both kinds
    /// are counted atomically so callers observe a single unified fault
    /// counter. A fault does not stop event delivery — the next event
    /// continues normally. Incremented atomically; safe to read from any
    /// thread.
    fn panic_count(&self) -> u64 {
        self.view.panic_count()
    }

    /// Milliseconds since the most recent inbound streaming frame of
    /// any kind (data tick, heartbeat, control), or ``None`` when no
    /// session is live or no frame has been received yet. The
    /// operator-facing staleness clock: a steadily growing value is
    /// the earliest external signal of a dead or wedged connection.
    fn millis_since_last_event(&self) -> Option<u64> {
        self.view.millis_since_last_event()
    }

    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// streaming frame of any kind. Returns ``0`` when no session is
    /// live or no frame has been received yet.
    fn last_event_received_at_unix_nanos(&self) -> i64 {
        self.view.last_event_received_at_unix_nanos()
    }

    /// Address (``host:port``) of the streaming server the current
    /// session is connected to, following the session across
    /// auto-reconnects. ``None`` when no session is live.
    fn last_connected_addr(&self) -> Option<String> {
        self.view.last_connected_addr()
    }

    /// Polymorphic subscribe — primary fluent entry point. Accepts
    /// any value returned by `Contract.quote()` / `Contract.trade()` /
    /// `Contract.open_interest()` (per-contract scope) or
    /// `SecType.OPTION.full_trades()` (full-stream scope).
    fn subscribe(&self, py: Python<'_>, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        self.view.subscribe(py, sub)
    }

    /// Bulk-subscribe a list of `Subscription` values.
    ///
    /// The GIL is released across the blocking wire writes, so a
    /// concurrent `stop_streaming` on a sibling thread cannot deadlock
    /// against a long batch. Stops at the first error and re-raises it.
    fn subscribe_many(&self, py: Python<'_>, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        self.view.subscribe_many(py, subs)
    }

    /// Polymorphic unsubscribe.
    fn unsubscribe(&self, py: Python<'_>, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        self.view.unsubscribe(py, sub)
    }

    /// Bulk-unsubscribe.
    ///
    /// Releases the GIL across each blocking wire write, for the same reason
    /// as `subscribe_many`.
    fn unsubscribe_many(&self, py: Python<'_>, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        self.view.unsubscribe_many(py, subs)
    }

    /// Stop streaming and clear the registered callback. Same
    /// explicit-handoff semantics as the unified client: to resume
    /// streaming after this returns, call `start_streaming(callback)`
    /// again with a freshly bound callable; `reconnect()` raises
    /// `RuntimeError` because no callback is held.
    pub(crate) fn stop_streaming(&self, py: Python<'_>) {
        self.view.stop_streaming(py);
    }

    /// Alias for `stop_streaming`. Mirrors the unified client's split
    /// surface where `shutdown` is documented as the terminal stop —
    /// on the standalone client both names are equivalent.
    fn shutdown(&self, py: Python<'_>) {
        self.view.shutdown(py);
    }

    /// Re-open the streaming connection and re-register the previously
    /// installed callback. Requires a prior `start_streaming(callback)`;
    /// raises `RuntimeError` otherwise.
    ///
    /// Same as `client.stream.reconnect()`: saves the active per-contract
    /// and full-stream subscriptions against the old session, opens a fresh
    /// streaming connection under the previously installed callback, and
    /// re-applies the saved subscriptions. Per-subscription failures during
    /// restore are surfaced as a single `StreamError` that names every
    /// contract that did not re-subscribe; the streaming session itself is
    /// already up at that point. A retry after a failed reconnect replays the
    /// subscriptions the failed attempt saved.
    fn reconnect(&self, py: Python<'_>) -> PyResult<()> {
        self.view.reconnect(py)
    }

    /// Block until every superseded streaming session's event ring
    /// consumer has finished firing the registered callback. Returns
    /// `true` once all retired generations have drained, `false` on
    /// timeout. Polls at 1 ms cadence.
    pub(crate) fn await_drain(&self, py: Python<'_>, timeout_ms: u64) -> bool {
        self.view.await_drain(py, timeout_ms)
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
