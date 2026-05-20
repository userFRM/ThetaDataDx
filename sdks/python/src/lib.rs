//! Python bindings over the Rust `thetadatadx` core. Every call crosses the
//! PyO3 boundary into the same Rust code path used by the CLI and FFI.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use tdbe::types::tick;
use thetadatadx::auth;
use thetadatadx::config;
use thetadatadx::fpss;

mod async_runtime;
mod chunking;
mod coerce;
mod errors;
mod flatfile_methods;
mod fluent;
mod fpss_client;
mod logging_bridge;
mod mdds_client;
mod util_helpers;

// These imports look unused at source level — they are pulled in by
// the `include!("_generated/historical_methods.rs")` and
// `include!("_generated/streaming_methods.rs")` blocks below, which
// expand inside this module and reference these names without their
// own `use` declarations.
use async_runtime::spawn_awaitable;
use coerce::{PyDateArg, PyStringArg, PySymbols, PyTimeArg};
use errors::to_py_err;

/// Shared tokio runtime for running async Rust from sync Python.
///
/// The `pyo3-async-runtimes` layer consumes the same runtime handle via
/// `pyo3_async_runtimes::tokio::init_with_runtime(...)` at module init
/// time (see [`thetadatadx_py`]). No second runtime is ever constructed,
/// so the sync and async code paths share worker threads, connection
/// pools, and the request semaphore on the underlying `MddsClient`.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime")
    })
}

/// Run an async future to completion while periodically honoring Python's
/// signal handlers. A blocking `runtime().block_on` inside `py.detach`
/// otherwise starves `KeyboardInterrupt` because the GIL is released and
/// signals can never be delivered.
///
/// Polls `Python::check_signals()` every 20ms. On Ctrl+C, returns the
/// `PyErr` raised by Python (typically `KeyboardInterrupt`); the in-flight
/// future is dropped and its gRPC channel is cancelled.
///
/// The 20 ms cadence (vs. the pre-v8.0.6 100 ms) reduces first-tick jitter
/// on sub-100 ms endpoint calls — an extra Python `check_signals()` call
/// is ~1 µs, so driving the ticker 5× as often has negligible steady-state
/// cost but collapses the worst-case select-wait on short calls from
/// ~100 ms down to ~20 ms. Long-running endpoints see no behavioural
/// change beyond a slightly finer-grained Ctrl+C cancellation window.
pub(crate) fn run_blocking<F, T>(py: Python<'_>, fut: F) -> PyResult<T>
where
    F: std::future::Future<Output = Result<T, thetadatadx::Error>> + Send,
    T: Send,
{
    py.detach(|| {
        runtime().block_on(async move {
            tokio::pin!(fut);
            loop {
                tokio::select! {
                    out = &mut fut => return out.map_err(to_py_err),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {
                        Python::attach(|py| py.check_signals())?;
                    }
                }
            }
        })
    })
}

/// Snapshot-endpoint fast path — runs a future under `runtime().block_on`
/// inside `py.detach` with a bounded `tokio::time::timeout`, skipping the
/// `run_blocking` signal-check polling loop entirely.
///
/// Snapshot-kind endpoints (`stock_snapshot_*`, `option_snapshot_*`,
/// `index_snapshot_*`, `calendar_*`) complete in under 200 ms on every
/// observed production call; the 5-second upper bound is a liveness
/// safeguard that adds zero steady-state cost. Dropping the `tokio::select!`
/// ticker removes the +1-5 ms first-tick-jitter tax that closes the
/// remaining latency gap vs. the vendor's v3 client on 90-100 ms calls.
///
/// Ctrl+C during a snapshot call is still honoured after the future
/// resolves or after the 5-second timeout fires, so the interpreter
/// cannot be wedged indefinitely.
pub(crate) const SNAPSHOT_UPPER_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

fn run_blocking_snapshot<F, T>(py: Python<'_>, fut: F) -> PyResult<T>
where
    F: std::future::Future<Output = Result<T, thetadatadx::Error>> + Send,
    T: Send,
{
    py.detach(|| {
        runtime().block_on(async move {
            match tokio::time::timeout(SNAPSHOT_UPPER_BOUND, fut).await {
                Ok(out) => out.map_err(to_py_err),
                Err(_) => Err(PyRuntimeError::new_err(format!(
                    "snapshot endpoint exceeded {} s upper bound",
                    SNAPSHOT_UPPER_BOUND.as_secs()
                ))),
            }
        })
    })
}


// ── Credentials ──
// Lifecycle: intentionally hand-written (language-specific constructor semantics).
//
// `skip_from_py_object` matches every generated pyclass: these are constructed
// on the Python side and passed to Rust by reference (`&Credentials` in
// `ThetaDataDxClient::new`), never extracted by value, so the auto-derived
// `FromPyObject` impl is dead weight (and deprecated for `Clone` pyclasses in
// pyo3 0.28+).

#[pyclass(module = "thetadatadx", frozen, skip_from_py_object)]
#[derive(Clone)]
struct Credentials {
    inner: auth::Credentials,
}

#[pymethods]
impl Credentials {
    /// Create credentials from email and password.
    #[new]
    fn new(email: String, password: String) -> Self {
        Self {
            inner: auth::Credentials::new(email, password),
        }
    }

    /// Load credentials from a file (line 1 = email, line 2 = password).
    #[staticmethod]
    fn from_file(path: &str) -> PyResult<Self> {
        let inner = auth::Credentials::from_file(path).map_err(to_py_err)?;
        Ok(Self { inner })
    }

    fn __repr__(&self) -> String {
        // Match the redacted Rust `Debug` impl on `auth::Credentials`
        // (`crates/thetadatadx/src/auth/creds.rs`). The Python binding
        // previously reached around the Debug impl by formatting
        // `self.inner.email` directly — that leaked the email into
        // Jupyter `repr()`, tracebacks, and any structured logger that
        // captures pyclass reprs.
        "Credentials(email=<redacted>)".to_string()
    }
}

// ── Config ──
// Lifecycle: intentionally hand-written (language-specific constructor semantics).
//
// `frozen` + `skip_from_py_object` matches every generated pyclass: the
// outer handle is immutable from Rust's perspective (no `&mut self` across
// the GIL), while the inner `DirectConfig` is guarded by a `Mutex` so
// Python-side setters (`config.reconnect_policy = "auto"`) still mutate the
// underlying nested `DirectConfig` in
// place. Python-side semantics are unchanged.

#[pyclass(module = "thetadatadx", frozen, skip_from_py_object)]
struct Config {
    inner: Mutex<config::DirectConfig>,
}

impl Config {
    fn from_direct(inner: config::DirectConfig) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }
}

#[pymethods]
impl Config {
    /// Production configuration (ThetaData NJ datacenter).
    #[staticmethod]
    fn production() -> Self {
        Self::from_direct(config::DirectConfig::production())
    }

    /// Dev FPSS configuration (port 20200, infinite historical replay).
    #[staticmethod]
    fn dev() -> Self {
        Self::from_direct(config::DirectConfig::dev())
    }

    /// Stage FPSS configuration (port 20100, testing, unstable).
    #[staticmethod]
    fn stage() -> Self {
        Self::from_direct(config::DirectConfig::stage())
    }

    /// Set the FPSS reconnect policy.
    ///
    /// - "auto" (default): auto-reconnect with split per-class attempt
    ///   budgets ([`config::ReconnectAttemptLimits`] defaults — 3
    ///   attempts for generic transients, 100 for rate-limited).
    /// - "manual": no auto-reconnect, user calls reconnect explicitly.
    ///
    /// Per-class attempt budgets and the stable-window timer are
    /// configured via the dedicated `reconnect_max_attempts`,
    /// `reconnect_max_rate_limited_attempts`, and
    /// `reconnect_stable_window_secs` setters.
    #[setter]
    fn set_reconnect_policy(&self, policy: &str) -> PyResult<()> {
        let parsed = match policy.to_lowercase().as_str() {
            "manual" => config::ReconnectPolicy::Manual,
            "auto" => {
                config::ReconnectPolicy::Auto(config::ReconnectAttemptLimits::default())
            }
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown reconnect_policy: {other:?} (expected \"auto\" or \"manual\")"
                )))
            }
        };
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.policy = parsed;
        Ok(())
    }

    /// Get the current reconnect policy as a string.
    #[getter]
    fn get_reconnect_policy(&self) -> &'static str {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(_) => "auto",
            config::ReconnectPolicy::Manual => "manual",
            config::ReconnectPolicy::Custom(_) => "custom",
        }
    }

    /// Set the per-class transient-failure attempt budget for the
    /// auto-reconnect path. Default `3`. Has no effect when the
    /// reconnect policy is `"manual"` or `"custom"`.
    #[setter]
    fn set_reconnect_max_attempts(&self, max_attempts: u32) -> PyResult<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.max_attempts = max_attempts;
        }
        Ok(())
    }

    /// Set the per-class rate-limited (`TooManyRequests`) attempt budget
    /// for the auto-reconnect path. Default `100`. Has no effect when
    /// the reconnect policy is `"manual"` or `"custom"`.
    #[setter]
    fn set_reconnect_max_rate_limited_attempts(
        &self,
        max_rate_limited_attempts: u32,
    ) -> PyResult<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.max_rate_limited_attempts = max_rate_limited_attempts;
        }
        Ok(())
    }

    /// Set the continuous successful-data-flow window (in seconds)
    /// after which the auto-reconnect attempt counters reset. Default
    /// `60`. Has no effect when the reconnect policy is `"manual"` or
    /// `"custom"`.
    #[setter]
    fn set_reconnect_stable_window_secs(&self, secs: u64) -> PyResult<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.stable_window = std::time::Duration::from_secs(secs);
        }
        Ok(())
    }

    /// Set whether to derive OHLCVC bars locally from trade events.
    ///
    /// When ``False``, only server-sent OHLCVC frames are emitted,
    /// reducing per-trade throughput overhead.
    #[setter]
    fn set_derive_ohlcvc(&self, enabled: bool) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.derive_ohlcvc = enabled;
    }

    /// Get the current OHLCVC derivation setting.
    #[getter]
    fn get_derive_ohlcvc(&self) -> bool {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.derive_ohlcvc
    }

    /// Override the MDDS gRPC host. Used by structural tests that need
    /// to point the MDDS channel at a known-refused endpoint to prove
    /// the FPSS-only surface never opens it; production code paths
    /// should keep the `Config::production()` default.
    #[setter]
    fn set_mdds_host(&self, host: String) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.host = host;
    }

    /// Current MDDS gRPC host.
    #[getter]
    fn get_mdds_host(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.host.clone()
    }

    /// Override the MDDS gRPC port. Companion to `mdds_host` — same
    /// rationale and same test-only usage.
    #[setter]
    fn set_mdds_port(&self, port: u16) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.port = port;
    }

    /// Current MDDS gRPC port.
    #[getter]
    fn get_mdds_port(&self) -> u16 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.port
    }

    fn __repr__(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        format!(
            "Config(mdds={}:{}, fpss_hosts={})",
            guard.mdds.host,
            guard.mdds.port,
            guard.fpss.hosts.len()
        )
    }
}

// ── Typed-pyclass tick definitions (generated from tick_schema.toml) ──
//
// `tick_arrow.rs` is the schema-generated Arrow pipeline used by the
// DataFrame adapter -- zero-copy handoff to pyarrow via the Arrow C
// Data Interface. `tick_classes.rs` is the primary return path for
// all historical endpoints -- matches the typed-struct approach used
// by Rust core, TypeScript, Go, and C++ FFI.

include!("_generated/tick_classes.rs");

include!("_generated/tick_arrow.rs");

include!("_generated/utility_functions.rs");

// ── FPSS streaming client ──

// ── BufferedEvent + converter (generated from fpss_event_schema.toml) ──
//
// Flat owned form of `fpss::FpssEvent`, materialised inside the
// dispatcher's drain-thread closure before we acquire the GIL and
// build the typed pyclass. Cheaper than calling
// `buffered_event_to_typed` directly on a borrowed `&FpssEvent`
// because the typed-pyclass conversion takes owned strings/bytes.
// Generator output is identical to the TypeScript SDK copy;
// `fpss_event_schema.toml` is the single source of truth.
include!("_generated/buffered_event.rs");

// ── Unified ThetaDataDxClient client ──

/// Unified ThetaData client — single connection for both historical and streaming.
///
/// This is the recommended entry point. Connects historical (MDDS/gRPC)
/// with a single authentication. Streaming (FPSS/TCP) starts lazily via
/// ``start_streaming(callback)``.
///
/// Usage::
///
///     tdx = ThetaDataDxClient(creds, config)
///     eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
///
///     def on_event(event):
///         print(event.kind, event)
///
///     tdx.start_streaming(callback=on_event)
///     tdx.subscribe(Contract.stock("AAPL").quote())
///     # ... events arrive on the dispatcher's drain thread ...
///     tdx.stop_streaming()
#[pyclass]
struct ThetaDataDxClient {
    /// The underlying Rust unified client (Deref to MddsClient for historical).
    ///
    /// Wrapped in `Arc<>` so the per-endpoint fluent builder pyclasses
    /// emitted by the generator (`<Endpoint>Builder`) can clone a cheap
    /// handle into the awaitable returned by `*_async()` terminals. The
    /// inner `thetadatadx::ThetaDataDxClient` is not `Clone` — its FPSS mutex
    /// and subscription-tier state forbid it — so the builder cannot
    /// hold the value directly without Arc ref-counting.
    tdx: std::sync::Arc<thetadatadx::ThetaDataDxClient>,
    /// User-registered Python callable that receives every FPSS event
    /// after `start_streaming(callback)` succeeds. The dispatcher's
    /// drain thread acquires the GIL via `Python::attach` to invoke
    /// `callback(event)`; the FPSS reader thread itself never touches
    /// Python. `None` before any `start_streaming` and after every
    /// `stop_streaming` / `shutdown`. `reconnect()` re-uses the
    /// stored handle so callers do not have to re-pass the callable.
    callback: Mutex<Option<Py<PyAny>>>,
}

#[pymethods]
impl ThetaDataDxClient {
    // Lifecycle: intentionally hand-written (language-specific constructor semantics).

    /// Connect to ThetaData (historical only -- FPSS is NOT started).
    ///
    /// Authenticates once, opens gRPC channel. Call
    /// ``start_streaming(callback)`` to begin FPSS real-time data —
    /// the dispatcher invokes ``callback(event)`` under the GIL for
    /// every typed event.
    ///
    /// Routed through [`run_blocking`] so a hung TLS handshake or slow
    /// auth round-trip stays cancellable via Ctrl+C — a plain
    /// `runtime().block_on(connect(...))` would swallow `SIGINT` until
    /// the network returned (signals can't fire while the GIL is
    /// released inside `block_on`).
    #[new]
    fn new(py: Python<'_>, creds: &Credentials, config: &Config) -> PyResult<Self> {
        // Snapshot the DirectConfig under the mutex — connect() takes
        // ownership, and the outer `Config` handle may still be mutated
        // Python-side after construction.
        let direct_config = {
            let guard = config.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        let inner_creds = creds.inner.clone();
        let tdx = run_blocking(py, async move {
            thetadatadx::ThetaDataDxClient::connect(&inner_creds, direct_config).await
        })?;

        Ok(Self {
            tdx: std::sync::Arc::new(tdx),
            callback: Mutex::new(None),
        })
    }

    // No per-endpoint `_df` / `_arrow` / `_polars` convenience wrappers.
    // Every historical endpoint returns `Py<<TickName>List>` (or
    // `Py<StringList>` for list endpoints); chain `.to_polars()` /
    // `.to_pandas()` / `.to_arrow()` / `.to_list()` on the return
    // value for the Arrow-backed conversion. One code path, one SSOT,
    // one place to audit.

    fn __repr__(&self) -> String {
        let streaming = if self.tdx.is_streaming() {
            "streaming=connected"
        } else {
            "streaming=none"
        };
        format!("ThetaDataDxClient(historical=connected, {streaming})")
    }

    /// Cumulative count of FPSS events the TLS reader could not
    /// publish into the Disruptor ring because the consumer fell
    /// behind and the ring was full (`Producer::try_publish` returned
    /// `RingBufferFull`).
    ///
    /// Forwarded directly to
    /// [`thetadatadx::ThetaDataDxClient::dropped_event_count`] so the count
    /// matches every other binding (C ABI, TypeScript, C++). The
    /// counter lives on the live `FpssClient`, not on this Python
    /// wrapper, which has two consequences:
    ///
    /// * `reconnect()` calls `stop_streaming()` + `start_streaming()`
    ///   internally; that rebuilds the FPSS client and the counter
    ///   resets to zero. Snapshot the value BEFORE reconnect if you
    ///   need to accumulate drops across session boundaries.
    /// * After `stop_streaming()` the slot is empty and the getter
    ///   returns 0. The same is true before `start_streaming()` is
    ///   ever called.
    ///
    /// Returns 0 before `start_streaming`, the running total while
    /// streaming, and 0 again after `stop_streaming`. Consumers
    /// should poll this on a periodic timer and emit a log on any
    /// non-zero delta within a single streaming session.
    fn dropped_event_count(&self) -> u64 {
        self.tdx.dropped_event_count()
    }
}

// ── Fluent contract-first API on the unified client ──────────────────────
//
// Polymorphic `subscribe(Subscription)` / `unsubscribe(Subscription)` /
// `subscribe_many([Subscription, ...])`. Routes through the same Rust
// core paths used by the typed compat helpers in
// `streaming_methods.rs`, but takes the typed `Subscription` value
// returned by `Contract.quote()` / `SecType.OPTION.full_trades()` —
// no string parsing, no kwarg gymnastics.
//
// Second `#[pymethods]` impl block enabled by the `multiple-pymethods`
// PyO3 feature flag (also used by `streaming_session.rs`).
#[pymethods]
impl ThetaDataDxClient {
    /// Polymorphic subscribe — primary fluent entry point.
    ///
    /// Accepts the `Subscription` value returned by `Contract.quote()`
    /// / `Contract.trade()` / `Contract.open_interest()` (per-contract
    /// scope) or by `SecType.OPTION.full_trades()` /
    /// `SecType.OPTION.full_open_interest()` (full-stream scope).
    ///
    /// ```python
    /// stock  = Contract.stock("AAPL")
    /// option = Contract.option("SPY", expiration="20260620", strike="550", right="C")
    /// client.subscribe(stock.quote())
    /// client.subscribe(option.trade())
    /// client.subscribe(SecType.OPTION.full_trades())
    /// ```
    fn subscribe(&self, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        let inner = fluent::coerce_subscription(sub)?;
        self.tdx.subscribe(inner).map_err(to_py_err)
    }

    /// Bulk-subscribe a list / iterable of `Subscription` values.
    ///
    /// Stops at the first error and re-raises it; previously-installed
    /// subscriptions are NOT rolled back (the FPSS protocol does not
    /// support batched transactions).
    fn subscribe_many(&self, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        let list = fluent::coerce_subscription_list(subs)?;
        self.tdx.subscribe_many(list).map_err(to_py_err)
    }

    /// Polymorphic unsubscribe — fluent counterpart to `subscribe(sub)`.
    fn unsubscribe(&self, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        let inner = fluent::coerce_subscription(sub)?;
        self.tdx.unsubscribe(inner).map_err(to_py_err)
    }

    /// Bulk-unsubscribe a list / iterable of `Subscription` values.
    fn unsubscribe_many(&self, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        let list = fluent::coerce_subscription_list(subs)?;
        self.tdx.unsubscribe_many(list).map_err(to_py_err)
    }
}

// `ThetaDataDxClient` is THE pyclass name. No alias, no compat wrapper.

// ── AsyncThetaDataDxClient — async-only sibling (Phase 3b) ─────────────
//
// Minimum-viable async/sync split: the underlying `ThetaDataDxClient`
// exposes both sync and `*_async` historical methods today. This thin
// wrapper holds a `ThetaDataDxClient` handle and proxies attribute
// access through `__getattr__`, but raises on access to non-`async_`
// methods so users that opt into the async surface do not
// accidentally call a blocking sync path.
//
// The full split (separate codegen pass that emits async-only
// builders, no sync surface) lands in v9.2.0. Today the wrapper is a
// disciplined façade — same Rust core, narrower public Python
// surface.

/// Async-only sibling of [`ThetaDataDxClient`].
///
/// ```python
/// import asyncio
/// from thetadatadx import AsyncThetaDataDxClient, Credentials, Config
///
/// async def main():
///     creds = Credentials.from_file("creds.txt")
///     client = AsyncThetaDataDxClient(creds, Config.production())
///     ticks = await client.stock_history_eod_async("AAPL", "20240101", "20240301")
///     print(ticks.to_pandas().head())
///
/// asyncio.run(main())
/// ```
///
/// Attribute access is restricted to async-suffixed methods plus a
/// safelisted set of synchronous lifecycle methods that have no
/// async counterpart on the wrapped surface
/// (`subscribe`/`unsubscribe`/`stop_streaming`/...).
#[pyclass(module = "thetadatadx", name = "AsyncThetaDataDxClient")]
struct AsyncThetaDataDxClient {
    inner: Py<ThetaDataDxClient>,
}

#[pymethods]
impl AsyncThetaDataDxClient {
    #[new]
    fn new(py: Python<'_>, creds: &Credentials, config: &Config) -> PyResult<Self> {
        let tdx = Py::new(py, ThetaDataDxClient::new(py, creds, config)?)?;
        Ok(Self { inner: tdx })
    }

    /// Convenience constructor: `AsyncThetaDataDxClient.from_file("creds.txt")`.
    #[staticmethod]
    fn from_file(py: Python<'_>, path: &str) -> PyResult<Self> {
        let creds = Credentials::from_file(path)?;
        let config = Config::production();
        let tdx = Py::new(py, ThetaDataDxClient::new(py, &creds, &config)?)?;
        Ok(Self { inner: tdx })
    }

    /// Forward attribute access to the wrapped `ThetaDataDxClient`.
    /// Async-suffixed methods plus the safelisted lifecycle / streaming
    /// methods are reachable; everything else raises `AttributeError`
    /// so callers who picked the async surface stay on the async path.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        const ALLOWED: &[&str] = &[
            "subscribe",
            "subscribe_many",
            "unsubscribe",
            "unsubscribe_many",
            "start_streaming",
            "stop_streaming",
            "shutdown",
            "streaming",
            "is_streaming",
            "is_authenticated",
            "active_subscriptions",
            "reconnect",
            "await_drain",
            "dropped_event_count",
            "session_uuid",
            "subscription_info",
            "config",
            "flat_files",
        ];
        if !name.ends_with("_async") && !ALLOWED.contains(&name) {
            return Err(pyo3::exceptions::PyAttributeError::new_err(format!(
                "AsyncThetaDataDxClient surfaces only `*_async` historical methods plus \
                 streaming lifecycle helpers; `{name}` is not on the async surface. \
                 Use `ThetaDataDxClient` for the synchronous historical methods."
            )));
        }
        let bound = self.inner.bind(py);
        Ok(bound.getattr(name)?.unbind())
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let bound = self.inner.bind(py);
        let inner_repr: String = bound.call_method0("__repr__")?.extract()?;
        Ok(inner_repr.replace("ThetaDataDxClient", "AsyncThetaDataDxClient"))
    }
}

// ── Typed-pyclass FPSS event path ─────────────────────────────────────────
//
// All FPSS `#[pyclass]` definitions and the `BufferedEvent` → typed
// dispatch live in a generated file whose SSOT is
// `crates/thetadatadx/fpss_event_schema.toml`. The generator is
// `crates/thetadatadx/build_support/fpss_events/`; regenerate via
// `cargo run --bin generate_sdk_surfaces --features config-file -- --write`.

include!("_generated/fpss_event_classes.rs");

include!("_generated/streaming_methods.rs");

mod streaming_session;
use streaming_session::StreamingSession;

// Pull-iter delivery mode: hand-written PyO3 wrappers around
// `thetadatadx::EventIterator`. Two pyclasses — `EventIterator` (the
// drain handle) and `StreamingIterSession` (context-manager). Both
// live in a second `#[pymethods] impl ThetaDataDxClient` block.
mod event_iterator;
mod streaming_iter_session;
use event_iterator::EventIterator;
use streaming_iter_session::StreamingIterSession;

include!("_generated/historical_methods.rs");

// `decode_response_bytes(endpoint, chunks)` hook used by the external
// parity bench harness. Generator-emitted from `endpoint_surface.toml`
// so every new endpoint is auto-wired — no manual edits here. See
// `crates/thetadatadx/build_support/endpoints/render/python.rs::render_python_decode_bench`.
include!("_generated/decode_bench.rs");

// ── DataFrame adapter: Arrow columnar pipeline ──
//
// Every historical endpoint returns a typed `<TickName>List` (or
// `StringList` for list endpoints), generator-emitted from
// `tick_schema.toml` + `endpoint_surface.toml`. Terminals live on the
// wrapper: `ticks.to_list()`, `ticks.to_arrow()`, `ticks.to_pandas()`,
// `ticks.to_polars()` — one surface, no free-function round-trip.
//
// This file owns only the thin pyarrow -> {pandas, polars} bridges
// consumed by the generated terminals; the conversion machinery itself
// (schema, per-tick Arrow builders, `StringList`, and the list wrappers)
// lives in `tick_arrow.rs` + `tick_classes.rs`.
//
// Zero-copy path:
//   Vec<tick::T>     -- Rust-side (historical endpoints)
//     -> `<TickName>List` (decoder-owned Vec, no copy)
//     -> RecordBatch  -- schema-generated arrow builders
//     -> arrow_pyarrow::Table
//     -> pyarrow.Table (Arrow C Data Interface, zero-copy buffers)
//     -> pandas.DataFrame | polars.DataFrame | user code

/// pyarrow.Table -> pandas.DataFrame. pandas 2.x is required for the
/// numpy-backed zero-copy path (see `pyproject.toml` extras for the
/// version pin).
fn pyarrow_table_to_pandas(py: Python<'_>, table: Py<PyAny>) -> PyResult<Py<PyAny>> {
    // We don't import `pandas` explicitly -- `pyarrow.Table.to_pandas()`
    // does that internally and raises its own ImportError if pandas is
    // missing, which we re-wrap so the message guides users to the right
    // `pip install` command.
    let bound = table.bind(py);
    let df = bound.call_method0("to_pandas").map_err(|e| {
        // Re-wrap ImportError (raised by pyarrow when pandas is absent)
        // so users know which extra to install. Other errors pass through
        // untouched.
        if e.is_instance_of::<pyo3::exceptions::PyImportError>(py) {
            pyo3::exceptions::PyImportError::new_err(
                "pandas is required for .to_pandas(). Install with: pip install thetadatadx[pandas]",
            )
        } else {
            e
        }
    })?;
    Ok(df.unbind())
}

/// pyarrow.Table -> polars.DataFrame via `polars.from_arrow`. Requires
/// polars >= 0.20 (see `pyproject.toml`).
fn pyarrow_table_to_polars(py: Python<'_>, table: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let polars = py.import("polars").map_err(|_| {
        pyo3::exceptions::PyImportError::new_err(
            "polars is not installed. Install it with: pip install thetadatadx[polars]",
        )
    })?;
    let df = polars.call_method1("from_arrow", (table,))?;
    Ok(df.unbind())
}

/// Split a date range `(start, end)` into chunks that each fit under
/// the server's 365-day cap.
///
/// Used internally by the auto-chunk pre-flight; exposed publicly so
/// tooling / test harnesses can verify the split boundaries without
/// reaching into Rust internals. Each chunk's boundaries are inclusive
/// `YYYYMMDD` strings identical to the ones every history endpoint
/// accepts.
///
/// Returns `[(start, end)]` unchanged when the span is ≤365 days.
/// Raises `ValueError` on malformed input.
///
/// Example::
///
///     >>> import thetadatadx
///     >>> thetadatadx.split_date_range("20200101", "20231231")
///     [('20200101', '20201230'), ('20201231', '20211230'), ('20211231', '20221230'), ('20221231', '20231230'), ('20231231', '20231231')]
#[pyfunction]
fn split_date_range(start: &str, end: &str) -> PyResult<Vec<(String, String)>> {
    chunking::split_date_range(start, end).map_err(|e| PyValueError::new_err(e.to_string()))
}

// ── Module ──

/// thetadatadx — Native ThetaData SDK powered by Rust.
///
/// This Python package wraps the thetadatadx Rust crate via PyO3.
/// All data parsing, gRPC communication, and TCP streaming
/// happens in compiled Rust — Python is just the interface.
#[pymodule]
#[pyo3(name = "thetadatadx")]
fn thetadatadx_py(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Install the tracing → Python logging bridge FIRST so any `tracing`
    // events emitted during the subsequent connect / config setup reach
    // user-configured `logging.getLogger("thetadatadx")` handlers.
    logging_bridge::install_logging_bridge();

    // Teach pyo3-async-runtimes to reuse our shared tokio runtime. This is
    // the critical coupling that keeps `runtime()` — the one the sync
    // path uses — and the async path aligned: one runtime, one request
    // semaphore, one connection pool. Failing to call this early would
    // cause `pyo3_async_runtimes::tokio::future_into_py` to spin up its
    // own runtime on first use.
    //
    // `init_with_runtime` requires a `&'static tokio::runtime::Runtime`
    // handle. Our `runtime()` singleton satisfies that contract — the
    // `OnceLock` leaks the runtime for the lifetime of the process, so
    // the borrow holds.
    let _ = pyo3_async_runtimes::tokio::init_with_runtime(runtime());

    m.add_class::<Credentials>()?;
    m.add_class::<Config>()?;
    m.add_class::<ThetaDataDxClient>()?;
    m.add_class::<AsyncThetaDataDxClient>()?;
    m.add_class::<fpss_client::FpssClient>()?;
    m.add_class::<mdds_client::MddsClient>()?;
    m.add_class::<StreamingSession>()?;
    m.add_class::<StreamingIterSession>()?;
    m.add_class::<EventIterator>()?;
    fluent::register(m)?;
    m.add_class::<flatfile_methods::FlatFilesNamespace>()?;
    m.add_class::<flatfile_methods::FlatFileRowList>()?;
    register_fpss_event_classes(m)?;
    register_tick_classes(m)?;
    register_generated_utility_functions(m)?;
    register_generated_historical_builders(m)?;
    coerce::register_string_enums(m)?;
    util_helpers::register(m)?;

    // Typed exception hierarchy — exports `thetadatadx.ThetaDataError`,
    // `thetadatadx.AuthenticationError`, etc. See [`errors`] for the
    // full tree + mapping from `thetadatadx::Error` variants.
    errors::register_exceptions(py, m)?;

    m.add_function(wrap_pyfunction!(decode_response_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(split_date_range, m)?)?;
    // Introspection helper for the offline `MddsClient` block-list
    // coverage test. Mirrors `mdds_client::FPSS_TOUCHING_METHODS`.
    m.add_function(wrap_pyfunction!(mdds_client::blocked_fpss_methods, m)?)?;
    Ok(())
}
