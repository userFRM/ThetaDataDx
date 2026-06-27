//! Python bindings over the Rust `thetadatadx` core. Every call crosses the
//! PyO3 boundary into the same Rust code path used by the CLI and FFI.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use thetadatadx as tick;
use thetadatadx::auth;
use thetadatadx::config;
use thetadatadx::fpss;

mod async_runtime;
mod bench_streaming;
mod chunking;
mod coerce;
mod errors;
mod flatfile_methods;
mod fluent;
mod fpss_client;
mod logging_bridge;
mod mdds_client;
mod streaming_batches;

// These imports look unused at source level — they are pulled in by
// the `include!("_generated/historical_methods.rs")` and
// `include!("_generated/streaming_methods.rs")` blocks below, which
// expand inside this module and reference these names without their
// own `use` declarations.
use async_runtime::spawn_awaitable;
use coerce::{PyDateArg, PyStringArg, PySymbols, PyTimeArg};
use errors::{config_err, to_py_err};

/// Shared tokio runtime for running async Rust from sync Python.
///
/// The `pyo3-async-runtimes` layer consumes the same runtime handle via
/// `pyo3_async_runtimes::tokio::init_with_runtime(...)`. No second runtime
/// is ever constructed, so the sync and async code paths share worker
/// threads, connection pools, and the request semaphore on the underlying
/// `HistoricalClient`.
///
/// The runtime is process-global and built exactly once. The first client
/// connected in the process seeds it from that client's `config.runtime`
/// via [`runtime_from_config`], so `Config.worker_threads` takes effect
/// for the first client created in the process. The runtime is built
/// lazily — never at module import — so a `worker_threads` value set on
/// the config before the first connect is honoured.
static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Teach `pyo3-async-runtimes` to reuse our runtime, exactly once, the
/// first time the runtime is resolved. Guarded by `Once` so the cost is
/// paid a single time and the registration lands before any
/// `future_into_py` runs on the async path.
fn register_async_runtime(rt: &'static tokio::runtime::Runtime) {
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| {
        let _ = pyo3_async_runtimes::tokio::init_with_runtime(rt);
    });
}

/// Build (or return the already-built) process-global runtime, sizing the
/// worker pool from the first client's [`thetadatadx::RuntimeConfig`].
///
/// The first connect in the process seeds the pool from its
/// `config.runtime`; later connects share the already-built runtime, so
/// their `runtime` config is a no-op by design.
fn runtime_from_config(cfg: &thetadatadx::RuntimeConfig) -> &'static tokio::runtime::Runtime {
    let rt = RT.get_or_init(|| cfg.build_runtime().expect("failed to create tokio runtime"));
    register_async_runtime(rt);
    rt
}

/// Return the process-global runtime, building it with tokio default
/// sizing if no client has seeded it from config yet.
///
/// Connect constructors seed the pool via [`runtime_from_config`] before
/// their first `run_blocking`; this accessor resolves the already-built
/// runtime for every subsequent call.
fn runtime() -> &'static tokio::runtime::Runtime {
    let rt = RT.get_or_init(|| {
        thetadatadx::RuntimeConfig::default()
            .build_runtime()
            .expect("failed to create tokio runtime")
    });
    register_async_runtime(rt);
    rt
}

/// Run an async future to completion while periodically honoring Python's
/// signal handlers. A blocking runtime execution inside `py.detach`
/// otherwise starves `KeyboardInterrupt` because the GIL is released and
/// signals can never be delivered.
///
/// Polls `Python::check_signals()` every 20ms. On Ctrl+C, returns the
/// `PyErr` raised by Python (typically `KeyboardInterrupt`); the in-flight
/// future is dropped and its gRPC channel is cancelled.
///
/// # Errors
///
/// Returns the future's `thetadatadx::Error` mapped via [`to_py_err`], or
/// the `PyErr` raised by a pending Python signal (e.g. `KeyboardInterrupt`).
///
/// The 20 ms cadence reduces first-tick jitter on sub-100 ms endpoint
/// calls — an extra Python `check_signals()` call is ~1 µs, so driving
/// the ticker more often has negligible steady-state cost but collapses
/// the worst-case select-wait on short calls to ~20 ms. Long-running
/// endpoints see no behavioural change beyond a slightly finer-grained
/// Ctrl+C cancellation window.
pub(crate) fn run_blocking<F, T>(py: Python<'_>, fut: F) -> PyResult<T>
where
    F: std::future::Future<Output = Result<T, thetadatadx::Error>> + Send,
    T: Send,
{
    py.detach(|| {
        // VOCAB-OK: tokio Runtime::block_on in PyO3 bridge, not PyO3 allow_threads GIL-hold pattern
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

/// Snapshot-endpoint fast path — runs a future under the runtime
/// inside `py.detach` with a bounded `tokio::time::timeout`, skipping the
/// `run_blocking` signal-check polling loop entirely.
///
/// Snapshot-kind endpoints (`stock_snapshot_*`, `option_snapshot_*`,
/// `index_snapshot_*`, `calendar_*`) complete in under 200 ms on every
/// observed production call; the 5-second upper bound is a liveness
/// safeguard that adds zero steady-state cost. Dropping the `tokio::select!`
/// ticker removes the +1-5 ms first-tick-jitter tax on 90-100 ms calls.
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
        // VOCAB-OK: tokio Runtime::block_on in PyO3 bridge, not PyO3 allow_threads GIL-hold pattern
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
// `Client::new`), never extracted by value, so the auto-derived
// `FromPyObject` impl is dead weight (and deprecated for `Clone` pyclasses in
// pyo3 0.28+).

#[pyclass(module = "thetadatadx", frozen, skip_from_py_object)]
#[derive(Clone)]
struct Credentials {
    pub(crate) inner: auth::Credentials,
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

    /// Authenticate with an API key instead of an email + password.
    ///
    /// The key is trimmed and held as secret material; the repr never
    /// exposes it.
    #[staticmethod]
    fn from_api_key(api_key: String) -> Self {
        Self {
            inner: auth::Credentials::api_key(api_key),
        }
    }

    /// Authenticate with an API key paired with an account email.
    ///
    /// The email is lowercased and trimmed; an empty email is dropped.
    /// The key is trimmed and held as secret material.
    #[staticmethod]
    fn from_api_key_with_email(email: String, api_key: String) -> Self {
        Self {
            inner: auth::Credentials::api_key_with_email(email, api_key),
        }
    }

    /// Source credentials strictly from the ``THETADATA_API_KEY``
    /// environment variable.
    ///
    /// Strict: an unset or whitespace-only value raises ``ConfigError``
    /// rather than falling back, and there is no ``creds.txt`` file
    /// fallback. Use :meth:`from_env_or_file` when a file fallback is
    /// wanted instead.
    #[staticmethod]
    fn from_env() -> PyResult<Self> {
        let inner = auth::Credentials::from_env().map_err(to_py_err)?;
        Ok(Self { inner })
    }

    /// Source credentials from the environment, falling back to a
    /// credentials file.
    ///
    /// When ``THETADATA_API_KEY`` is set and non-empty an API key is
    /// used; otherwise the two-line ``creds.txt`` file at ``path`` is
    /// read.
    #[staticmethod]
    fn from_env_or_file(path: &str) -> PyResult<Self> {
        let inner = auth::Credentials::from_env_or_file(path).map_err(to_py_err)?;
        Ok(Self { inner })
    }

    /// Source credentials from a ``.env``-format file.
    ///
    /// The file uses the common ``.env`` grammar (one ``KEY=VALUE`` per
    /// line, optional ``export`` prefix, ``#`` comments, optional quotes).
    /// ``THETADATA_API_KEY`` selects an API key; otherwise
    /// ``THETADATA_EMAIL`` + ``THETADATA_PASSWORD`` build email +
    /// password credentials.
    #[staticmethod]
    fn from_dotenv(path: &str) -> PyResult<Self> {
        let inner = auth::Credentials::from_dotenv(path).map_err(to_py_err)?;
        Ok(Self { inner })
    }

    fn __repr__(&self) -> String {
        // Match the redacted Rust `Debug` impl on `auth::Credentials`
        // (`crates/thetadatadx/src/auth/creds.rs`). Never interpolate
        // `self.inner.email` here: a repr that prints the email leaks it
        // into Jupyter `repr()`, tracebacks, and any structured logger
        // that captures pyclass reprs.
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

    /// Dev streaming configuration (port 20200, infinite historical replay).
    #[staticmethod]
    fn dev() -> Self {
        Self::from_direct(config::DirectConfig::dev())
    }

    /// Historical-staging configuration (historical staging cluster + auth marker;
    /// streaming stays on production). Testing, unstable.
    #[staticmethod]
    fn stage() -> Self {
        Self::from_direct(config::DirectConfig::stage())
    }

    /// Source the target environment from a ``.env``-format file.
    ///
    /// Starts from the production configuration and applies the cluster
    /// keys carried by the file: ``THETADATA_HISTORICAL_TYPE`` (``PROD`` /
    /// ``STAGE``, case-insensitive) selects the environment, and the
    /// optional ``THETADATA_HISTORICAL_HOST`` / ``THETADATA_STREAMING_HOST``
    /// keys override the hosts (an explicit host wins over the environment
    /// default).
    ///
    /// This reads the same file format and keys as
    /// :meth:`Credentials.from_dotenv`, so a single ``.env`` file can carry
    /// both ``THETADATA_API_KEY`` and ``THETADATA_HISTORICAL_TYPE``.
    #[staticmethod]
    fn from_dotenv(path: &str) -> PyResult<Self> {
        let inner = config::DirectConfig::from_dotenv(path).map_err(to_py_err)?;
        Ok(Self::from_direct(inner))
    }

    /// Set the streaming reconnect policy.
    ///
    /// - "auto" (default): auto-reconnect with split per-class attempt
    ///   budgets ([`config::ReconnectAttemptLimits`] defaults — 30
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
            "auto" => config::ReconnectPolicy::Auto(config::ReconnectAttemptLimits::default()),
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
            _ => "custom",
        }
    }

    /// Install a custom reconnect policy driven by a Python callable.
    ///
    /// ``callback(reason: int, attempt: int)`` is invoked on the
    /// streaming I/O thread after each retriable involuntary
    /// disconnect; return the reconnect delay in milliseconds, or
    /// ``None`` to stop reconnecting (the stream then emits the
    /// terminal ``ReconnectsExhausted`` event). Permanent disconnect
    /// reasons (bad credentials, account conflicts) never reach the
    /// callable. Pass ``None`` to restore the default ``Auto`` policy.
    ///
    /// The callable runs off the main thread: it must be thread-safe
    /// and should return quickly — the I/O thread performs the actual
    /// delay sleep after the callable returns, without holding the
    /// interpreter lock.
    #[setter]
    fn set_reconnect_callback(&self, callback: Option<Py<PyAny>>) -> PyResult<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(callback) = callback else {
            guard.reconnect.policy =
                config::ReconnectPolicy::Auto(config::ReconnectAttemptLimits::default());
            return Ok(());
        };
        guard.reconnect.policy =
            config::ReconnectPolicy::Custom(std::sync::Arc::new(move |reason, attempt| {
                Python::attach(|py| {
                    let result = callback.call1(py, (reason as i32, attempt));
                    match result {
                        Ok(value) => {
                            if value.is_none(py) {
                                return None;
                            }
                            match value.extract::<u64>(py) {
                                Ok(ms) => Some(std::time::Duration::from_millis(ms)),
                                Err(e) => {
                                    e.write_unraisable(py, None);
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            // A raising callback cannot decide a delay;
                            // surface via the unraisable hook and stop
                            // reconnecting rather than looping silently.
                            e.write_unraisable(py, None);
                            None
                        }
                    }
                })
            }));
        Ok(())
    }

    // ── Streaming transport knobs ─────────────────────────────────────
    //
    // Scalar tuning on ``DirectConfig.streaming`` mirroring the FFI /
    // C++ / TypeScript surface. Out-of-range values are rejected by the
    // core validator at connect time.

    /// Set the streaming event ring buffer size (slots). Must be a power
    /// of two ``>= 64`` (rejected at connect otherwise). Default
    /// ``131_072``.
    #[setter]
    fn set_streaming_ring_size(&self, n: usize) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.streaming.ring_size = n;
    }

    /// Set the async worker-thread count for embedded bindings that own
    /// their runtime (Python / FFI / napi). ``None`` (the default)
    /// defers to the default sizing (one worker per logical CPU);
    /// ``Some(n)`` pins the worker pool to ``n``. ``Some(0)`` is
    /// preserved across the binding boundary and clamps to ``1`` so the
    /// runtime always has at least one worker.
    ///
    /// The async worker pool is process-global: it is built once, from the
    /// config of the first client connected in the process. This setting
    /// is therefore honoured when the first client in the process is
    /// created; clients connected later share the already-built pool, so
    /// setting it on a subsequent ``Config`` has no effect.
    #[setter]
    fn set_worker_threads(&self, n: Option<usize>) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.runtime.tokio_worker_threads = n;
    }

    /// Current ``worker_threads`` setting (``None`` = auto).
    #[getter]
    fn get_worker_threads(&self) -> Option<usize> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.runtime.tokio_worker_threads
    }

    // ``DirectConfig.retry`` ms getters (``initial_delay`` / ``max_delay``)
    // and the ``DirectConfig.auth`` string fields (``nexus_url`` /
    // ``client_type``) are the generated ``ms`` / ``string`` accessors in
    // config_surface.toml.

    // ``DirectConfig.metrics.port`` (``Optional[int]``, exporter port),
    // the ``streaming.flush_mode`` / ``wait_strategy`` enums, and the
    // ``reconnect.jitter`` / ``streaming.host_selection`` enums are the
    // generated ``enum`` / ``option`` accessors in config_surface.toml.

    /// Target historical environment carried by this configuration:
    /// ``"PROD"`` for the production cluster or ``"STAGE"`` for staging.
    /// The historical and streaming channels are selected independently;
    /// :meth:`Config.production` / :meth:`Config.stage` (and the
    /// ``THETADATA_HISTORICAL_TYPE`` key on :meth:`Config.from_dotenv`) set the
    /// historical channel, and this is the readback of that selection.
    /// Mirrors the ``historical_type`` string the inline ``Client`` constructor
    /// accepts.
    #[getter]
    fn get_historical_environment(&self) -> &'static str {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.historical_environment().as_str()
    }

    /// Target streaming environment carried by this configuration:
    /// ``"PROD"`` for the production cluster or ``"DEV"`` for the dev
    /// cluster. The streaming and historical channels are selected
    /// independently; :meth:`Config.production` / :meth:`Config.dev` (and
    /// the ``THETADATA_STREAMING_TYPE`` key on :meth:`Config.from_dotenv`) set
    /// the streaming channel, and this is the readback of that selection.
    /// Mirrors the ``streaming_type`` string the inline ``Client`` constructor
    /// accepts.
    #[getter]
    fn get_streaming_environment(&self) -> &'static str {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.streaming_environment().as_str()
    }

    /// Spin iterations the wait strategy busy-waits before yielding /
    /// parking. Higher trades idle CPU for lower wake latency.
    #[setter]
    fn set_wait_spin_iters(&self, iters: u32) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.streaming.wait_spin_iters = iters;
    }

    /// Current wait-strategy spin iteration count.
    #[getter]
    fn get_wait_spin_iters(&self) -> u32 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.streaming.wait_spin_iters
    }

    /// ``yield_now`` iterations after the spin phase, before any park.
    #[setter]
    fn set_wait_yield_iters(&self, iters: u32) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.streaming.wait_yield_iters = iters;
    }

    /// Current wait-strategy yield iteration count.
    #[getter]
    fn get_wait_yield_iters(&self) -> u32 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.streaming.wait_yield_iters
    }

    /// Park interval (microseconds) for the parking wait strategies
    /// (``"balanced"`` / ``"efficient"``). Inert for the never-sleep
    /// strategies.
    #[setter]
    fn set_wait_park_us(&self, park_us: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.streaming.wait_park_us = park_us;
    }

    /// Current wait-strategy park interval in microseconds.
    #[getter]
    fn get_wait_park_us(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.streaming.wait_park_us
    }

    /// Set the CPU core to pin the streaming consumer thread to, or
    /// ``None`` to leave it under the OS scheduler (default).
    ///
    /// Pinning the tick-consumer thread to an isolated core gives
    /// deterministic, low-jitter delivery. An out-of-range or offline
    /// core is a best-effort no-op rather than an error.
    #[setter]
    fn set_consumer_cpu(&self, core: Option<usize>) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.streaming.consumer_cpu = core;
    }

    /// Current streaming consumer-thread CPU pin, or ``None`` if
    /// unpinned.
    #[getter]
    fn get_consumer_cpu(&self) -> Option<usize> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.streaming.consumer_cpu
    }

    // `historical_host` (string) is the generated `string` accessor in
    // config_surface.toml.

    fn __repr__(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        format!(
            "Config(historical={}:{}, streaming_hosts={})",
            guard.historical_host(),
            guard.historical.port,
            guard.streaming_hosts().len()
        )
    }
}

// Mechanical config setters/getters (`config_surface.toml`), in a second
// `#[pymethods] impl Config` block enabled by `multiple-pymethods`: the
// scalar / duration pairs plus the `policy_limit` (reconnect `Auto`-limit)
// and `string` carve-outs. The divergent accessors above (enum string
// labels, `Option`, policy selector) stay hand-written; only the
// assign/read pairs are projected from the SSOT.
include!("_generated/config_accessors.rs");

// ── Typed-pyclass tick definitions (generated from tick_schema.toml) ──
//
// `tick_arrow.rs` is the schema-generated Arrow pipeline used by the
// DataFrame adapter -- zero-copy handoff to pyarrow via the Arrow C
// Data Interface. `tick_classes.rs` is the primary return path for
// all historical endpoints -- matches the typed-struct approach used
// by Rust core, TypeScript, and C++ FFI.

include!("_generated/tick_classes.rs");

include!("_generated/tick_arrow.rs");

include!("_generated/utility_functions.rs");

// ── Streaming client ──

// ── Unified Client client ──

/// Unified ThetaData client — single connection for both historical and streaming.
///
/// This is the recommended entry point. Connects historical (gRPC over
/// HTTP/2 + TLS) with a single authentication. Real-time streaming
/// starts lazily via ``start_streaming(callback)``.
///
/// Usage::
///
///     client = Client(creds, config)
///     eod = client.historical.stock_history_eod("AAPL", "20240101", "20240301")
///
///     def on_event(event):
///         print(event.kind, event)
///
///     client.stream.start_streaming(callback=on_event)
///     client.stream.subscribe(Contract.stock("AAPL").quote())
///     # ... events arrive on the dispatcher's drain thread ...
///     client.stream.stop_streaming()
// `frozen` — every `#[pymethods]` entry on this pyclass takes
// `&self` (never `&mut self`). The inner `client: Arc<...>` carries its
// own mutex / atomic state for transient surfaces; the pyclass shell
// is immutable from Rust's perspective, which lets PyO3 elide the
// `RefCell` borrow-check overhead on every attribute / method
// dispatch under the free-threaded interpreter. A future `&mut self`
// regression surfaces as a `cargo check` failure rather than slipping
// silently through.
/// Resolve the inline authentication kwargs into a single
/// [`auth::Credentials`], enforcing that exactly one source was given.
///
/// The API key is first-class and mutually exclusive with the email +
/// password pair and with a pre-built `credentials` handle. Conflicts and
/// the empty case raise `ConfigError` before any network round-trip, so a
/// bad call fails fast and locally.
fn resolve_credentials(
    credentials: Option<&Credentials>,
    api_key: Option<String>,
    email: Option<String>,
    password: Option<String>,
) -> PyResult<auth::Credentials> {
    // Count the distinct auth methods supplied. `email` + `password`
    // together count as the single "email/password" method.
    let has_api_key = api_key.is_some();
    let has_email_pw = email.is_some() || password.is_some();
    let has_credentials = credentials.is_some();
    let set_count = u8::from(has_api_key) + u8::from(has_email_pw) + u8::from(has_credentials);

    if set_count == 0 {
        return Err(config_err(
            "no authentication argument given — pass api_key=..., the email=... and \
             password=... pair, or credentials=...",
        ));
    }
    if set_count > 1 {
        return Err(config_err(
            "conflicting authentication arguments — pass exactly one of api_key, the \
             email/password pair, or credentials",
        ));
    }

    if let Some(key) = api_key {
        return Ok(auth::Credentials::api_key(key));
    }
    if has_email_pw {
        match (email, password) {
            (Some(email), Some(password)) => return Ok(auth::Credentials::new(email, password)),
            _ => {
                return Err(config_err(
                    "email/password authentication needs both email= and password=",
                ));
            }
        }
    }
    // Exactly one source remained: the pre-built credentials handle.
    Ok(credentials
        .expect("set_count == 1 with no api_key / email-password leaves credentials")
        .inner
        .clone())
}

/// Resolve the environment selection into a [`config::DirectConfig`].
///
/// A full `config` handle wins (its environments and hosts are taken
/// verbatim). Otherwise the historical and streaming
/// channels are selected independently on top of the production defaults:
/// `historical_type` (`"PROD"` / `"STAGE"`, case-insensitive) selects the
/// historical channel and `streaming_type` (`"PROD"` / `"DEV"`,
/// case-insensitive) the streaming channel. Either absent keeps that
/// channel on production. An unrecognized value raises ``ValueError``
/// naming the valid set, never a silent fallback.
fn resolve_direct_config(
    config: Option<&Config>,
    historical_type: Option<&str>,
    streaming_type: Option<&str>,
) -> PyResult<config::DirectConfig> {
    if let Some(cfg) = config {
        // Snapshot under the mutex — connect() takes ownership and the
        // outer handle may still be mutated Python-side afterward.
        let guard = cfg.inner.lock().unwrap_or_else(|e| e.into_inner());
        return Ok(guard.clone());
    }
    let mut direct = config::DirectConfig::production();
    if let Some(raw) = historical_type {
        let environment = config::HistoricalEnvironment::parse(raw).ok_or_else(|| {
            config_err(format!(
                "historical_type must be \"PROD\" or \"STAGE\" (case-insensitive); got {raw:?}"
            ))
        })?;
        direct = direct.with_historical_environment(environment);
    }
    if let Some(raw) = streaming_type {
        let environment = config::StreamingEnvironment::parse(raw).ok_or_else(|| {
            config_err(format!(
                "streaming_type must be \"PROD\" or \"DEV\" (case-insensitive); got {raw:?}"
            ))
        })?;
        direct = direct.with_streaming_environment(environment);
    }
    Ok(direct)
}

#[pyclass(frozen)]
struct Client {
    /// The underlying Rust unified client (Deref to HistoricalClient for historical).
    ///
    /// Wrapped in `Arc<>` so the per-endpoint fluent builder pyclasses
    /// emitted by the generator (`<Endpoint>Builder`) can clone a cheap
    /// handle into the awaitable returned by `*_async()` terminals. The
    /// inner `thetadatadx::Client` is not `Clone` — its
    /// streaming mutex and subscription-tier state forbid it — so the
    /// builder cannot hold the value directly without Arc ref-counting.
    ///
    /// Shutdown contract: when the pyclass auto-drops while the GIL is
    /// held, the final `Arc::drop` may trigger the core
    /// `Client::Drop` chain, which joins the streaming dispatcher
    /// thread that itself re-acquires the GIL via `Python::attach`.
    /// Holding the GIL across that join would deadlock. Callers MUST
    /// invoke `stop_streaming()` (the generated method uses `py.detach`
    /// around the teardown so the dispatcher exits cleanly) before
    /// letting the pyclass fall out of scope. The `with client.streaming(cb)`
    /// context manager pairs `start_streaming(cb)` with
    /// `stop_streaming() + await_drain(5000)` on exit to enforce this
    /// ordering automatically. The fully-shared `Arc<>` (cloned into
    /// every fluent builder pyclass) cannot enforce the contract at the
    /// `Drop` site without restructuring every accessor, so the
    /// invariant is enforced by documentation plus the explicit
    /// `stop_streaming(py)` path.
    client: std::sync::Arc<thetadatadx::Client>,
    /// User-registered Python callable that receives every streaming
    /// event after `start_streaming(callback)` succeeds. The dispatcher's
    /// drain thread acquires the GIL via `Python::attach` to invoke
    /// `callback(event)`; the streaming reader thread itself never
    /// touches Python. `None` before any `start_streaming` and after
    /// every `stop_streaming` / `shutdown`. `reconnect()` re-uses the
    /// stored handle so callers do not have to re-pass the callable.
    ///
    /// `Arc<Mutex<...>>` so the same callback slot can be shared with the
    /// [`StreamView`] returned by `client.stream`: both the `Client` shell
    /// and every `StreamView` handle observe and mutate one registration,
    /// keeping `start_streaming` / `stop_streaming` / `reconnect`
    /// idempotent regardless of which surface the caller reaches through.
    callback: Arc<Mutex<Option<Py<PyAny>>>>,
}

impl Client {
    /// Connect a resolved credential + config, blocking on the
    /// process-global runtime via [`run_blocking`] so a hung handshake
    /// stays cancellable via Ctrl+C. Shared by every Python constructor
    /// (`__new__`, `from_file`, `from_env`, `from_dotenv`).
    fn connect_blocking(
        py: Python<'_>,
        creds: auth::Credentials,
        direct_config: config::DirectConfig,
    ) -> PyResult<Self> {
        // Seed the process-global runtime from this client's runtime
        // config before the first `run_blocking` resolves it, so
        // `worker_threads` takes effect on the first connect.
        runtime_from_config(&direct_config.runtime);
        let client = run_blocking(py, async move {
            thetadatadx::Client::connect(&creds, direct_config).await
        })?;
        Ok(Self {
            client: std::sync::Arc::new(client),
            callback: Arc::new(Mutex::new(None)),
        })
    }
}

/// User-facing historical-data sub-namespace returned by
/// `client.historical`.
///
/// Holds a cheap `Arc` clone of the inner unified client; constructing it
/// performs no auth round-trip and mutates no streaming state. Every
/// historical endpoint method (sync, `*_async`, and `*_builder`) is
/// generated onto this view from `endpoint_surface.toml`, so the surface
/// stays a single generated source of truth.
#[pyclass(frozen)]
struct HistoricalView {
    client: std::sync::Arc<thetadatadx::Client>,
}

/// User-facing real-time-streaming sub-namespace returned by
/// `client.stream`.
///
/// Shares the parent client's `Arc<thetadatadx::Client>` and the parent's
/// `Arc<Mutex<Option<Py<PyAny>>>>` callback slot, so `start_streaming`,
/// `stop_streaming`, `reconnect`, and the subscription methods observe the
/// same registration the unified client does. Constructing it is a pair of
/// `Arc::clone`s — no auth round-trip, no streaming state mutation.
#[pyclass(frozen)]
struct StreamView {
    client: std::sync::Arc<thetadatadx::Client>,
    callback: Arc<Mutex<Option<Py<PyAny>>>>,
}

#[pymethods]
impl Client {
    // Lifecycle: intentionally hand-written (language-specific constructor semantics).

    /// Connect to ThetaData (historical only -- streaming is NOT started).
    ///
    /// Authenticates once, opens gRPC channel. Call
    /// ``start_streaming(callback)`` to begin real-time streaming —
    /// the dispatcher invokes ``callback(event)`` under the GIL for
    /// every typed event.
    ///
    /// Routed through [`run_blocking`] so a hung TLS handshake or slow
    /// auth round-trip stays cancellable via Ctrl+C — a plain
    /// runtime-driven `connect()` would swallow `SIGINT` until
    /// the network returned (signals can't fire while the GIL is
    /// released inside the runtime executor).
    ///
    /// The API key is a first-class, directly-passed argument:
    /// ``Client(api_key="td1_...")`` and ``Client(api_key="td1_...",
    /// historical_type="STAGE")`` select the credential and the environment
    /// inline. Email + password is the parallel method:
    /// ``Client(email="user@example.com", password="secret")``. The
    /// lower-level typed path stays a clean superset:
    /// ``Client(credentials=creds, config=cfg)`` (and the historical
    /// positional ``Client(creds, config)``) still work.
    ///
    /// Exactly one authentication argument must be given — ``api_key``,
    /// the ``email`` + ``password`` pair, or ``credentials``. Passing
    /// none, or two different ones, raises ``ConfigError`` before any
    /// network round-trip. ``historical_type`` (``"PROD"`` / ``"STAGE"``,
    /// case-insensitive) selects the historical environment and
    /// ``streaming_type`` (``"PROD"`` / ``"DEV"``, case-insensitive) the
    /// streaming environment, independently; ``config`` supplies a full
    /// :class:`Config` whose environments and hosts win.
    #[new]
    #[pyo3(signature = (
        credentials=None,
        config=None,
        *,
        api_key=None,
        email=None,
        password=None,
        historical_type=None,
        streaming_type=None,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        credentials: Option<&Credentials>,
        config: Option<&Config>,
        api_key: Option<String>,
        email: Option<String>,
        password: Option<String>,
        historical_type: Option<&str>,
        streaming_type: Option<&str>,
    ) -> PyResult<Self> {
        let creds = resolve_credentials(credentials, api_key, email, password)?;
        let direct_config = resolve_direct_config(config, historical_type, streaming_type)?;
        Self::connect_blocking(py, creds, direct_config)
    }

    /// Connect with the API key sourced strictly from the environment
    /// (``THETADATA_API_KEY``).
    ///
    /// Strict, with no file fallback: an unset or whitespace-only
    /// ``THETADATA_API_KEY`` raises ``ConfigError`` before any network
    /// round-trip, mirroring the Rust ``ClientBuilder::api_key_from_env``
    /// and the C++ ``ClientBuilder::api_key_from_env`` so the same-named
    /// capability behaves identically across bindings. For the
    /// env-or-file convenience read a ``.env`` file with
    /// :meth:`from_dotenv` instead.
    ///
    /// ``historical_type`` selects the historical environment (``"PROD"`` /
    /// ``"STAGE"``) and ``streaming_type`` the streaming environment (``"PROD"``
    /// / ``"DEV"``), independently; ``config`` supplies a full
    /// :class:`Config` whose environments and hosts win. The key is read
    /// once, immediately before the network round-trip.
    #[staticmethod]
    #[pyo3(signature = (config=None, *, historical_type=None, streaming_type=None))]
    fn from_env(
        py: Python<'_>,
        config: Option<&Config>,
        historical_type: Option<&str>,
        streaming_type: Option<&str>,
    ) -> PyResult<Self> {
        let creds = auth::Credentials::from_env().map_err(to_py_err)?;
        let direct_config = resolve_direct_config(config, historical_type, streaming_type)?;
        Self::connect_blocking(py, creds, direct_config)
    }

    /// Connect with the credential (and optionally the environment)
    /// sourced from a ``.env``-format file.
    ///
    /// ``THETADATA_API_KEY`` selects an API key; otherwise
    /// ``THETADATA_EMAIL`` + ``THETADATA_PASSWORD`` build email +
    /// password credentials. When ``config`` is omitted the same file is
    /// also read for ``THETADATA_HISTORICAL_TYPE`` and ``THETADATA_STREAMING_TYPE``,
    /// so one ``.env`` can carry both the credential and the
    /// environments. An explicit ``config``, ``historical_type``, or
    /// ``streaming_type`` overrides the file's environment selection.
    #[staticmethod]
    #[pyo3(signature = (path, config=None, *, historical_type=None, streaming_type=None))]
    fn from_dotenv(
        py: Python<'_>,
        path: &str,
        config: Option<&Config>,
        historical_type: Option<&str>,
        streaming_type: Option<&str>,
    ) -> PyResult<Self> {
        let creds = auth::Credentials::from_dotenv(path).map_err(to_py_err)?;
        // With no explicit config / historical_type / streaming_type, read both
        // environment selectors from the same file; otherwise honour the
        // explicit override.
        let direct_config = match (config, historical_type, streaming_type) {
            (None, None, None) => config::DirectConfig::from_dotenv(path).map_err(to_py_err)?,
            _ => resolve_direct_config(config, historical_type, streaming_type)?,
        };
        Self::connect_blocking(py, creds, direct_config)
    }

    /// Convenience constructor: `Client.from_file("creds.txt")`.
    /// Loads credentials from a two-line file and connects with the
    /// supplied `config`, defaulting to `Config.production()`.
    ///
    /// The `config` kwarg is optional: with no kwarg the constructor
    /// targets the production endpoint. Tests and dev / stage
    /// environments reach a single-arg constructor shape via
    /// `Client.from_file("creds.txt", config=Config.dev())`.
    /// Parity with `AsyncClient.from_file()`,
    /// `HistoricalClient.from_file()`, and `StreamingClient.from_file()` — every
    /// Python client exposes the same one-call file-construction shape.
    #[staticmethod]
    #[pyo3(signature = (path, config=None))]
    fn from_file(py: Python<'_>, path: &str, config: Option<&Config>) -> PyResult<Self> {
        let creds = auth::Credentials::from_file(path).map_err(to_py_err)?;
        let direct_config = resolve_direct_config(config, None, None)?;
        Self::connect_blocking(py, creds, direct_config)
    }

    // No per-endpoint `_df` / `_arrow` / `_polars` convenience wrappers.
    // Every historical endpoint returns `Py<<TickName>List>` (or
    // `Py<StringList>` for list endpoints); chain `.to_polars()` /
    // `.to_pandas()` / `.to_arrow()` / `.to_list()` on the return
    // value for the Arrow-backed conversion. One code path, one SSOT,
    // one place to audit.

    fn __repr__(&self) -> String {
        let streaming = if self.client.stream().is_streaming() {
            "streaming=connected"
        } else {
            "streaming=none"
        };
        format!("Client(historical=connected, {streaming})")
    }

    /// Historical-data sub-namespace: `client.historical.stock_eod(...)`.
    ///
    /// Returns a fresh [`HistoricalView`] over a cheap `Arc` clone of the
    /// inner client. No auth round-trip, no streaming-state mutation;
    /// storing `hist = client.historical` is identical to calling
    /// `client.historical.<endpoint>(...)` inline.
    #[getter]
    fn historical(&self) -> HistoricalView {
        HistoricalView {
            client: Arc::clone(&self.client),
        }
    }

    /// Real-time-streaming sub-namespace: `client.stream.subscribe(...)`,
    /// `client.stream.start_streaming(cb)`, …
    ///
    /// Returns a fresh [`StreamView`] sharing the inner client and the
    /// parent's callback slot, so the streaming lifecycle observed through
    /// the view is the same one the unified client manages.
    #[getter]
    fn stream(&self) -> StreamView {
        StreamView {
            client: Arc::clone(&self.client),
            callback: Arc::clone(&self.callback),
        }
    }
}

#[pymethods]
impl StreamView {
    /// Whether the live streaming session is currently authenticated.
    ///
    /// Distinct from :meth:`is_streaming`: the session can be live yet
    /// briefly unauthenticated mid-reconnect (the authenticated flag is
    /// cleared on disconnect and restored on a successful re-auth).
    /// Returns ``False`` before streaming starts and after it stops.
    /// Mirrors the standalone
    /// [`crate::fpss_client::StreamingClient::is_authenticated`] and the
    /// C++ `Stream::is_authenticated()` getter.
    fn is_authenticated(&self) -> bool {
        self.client.stream().is_authenticated()
    }

    /// Cumulative count of streaming events the TLS reader could not
    /// publish into the bounded ring because the consumer fell behind
    /// and the ring was full.
    ///
    /// Forwarded directly to
    /// [`thetadatadx::Client::dropped_event_count`] so the count
    /// matches every other binding (C ABI, TypeScript, C++). The
    /// counter lives on the live streaming client, not on this Python
    /// wrapper, which has two consequences:
    ///
    /// * `reconnect()` calls `stop_streaming()` + `start_streaming()`
    ///   internally; that rebuilds the streaming client and the counter
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
        self.client.stream().dropped_event_count()
    }

    /// Point-in-time count of streaming events published into the
    /// event ring but not yet drained into your callback — the
    /// in-flight depth between the I/O thread and the dispatcher.
    ///
    /// The leading back-pressure signal: :meth:`dropped_event_count`
    /// only moves AFTER data has been lost, while a rising occupancy
    /// that approaches :meth:`ring_capacity` predicts those drops
    /// while there is still time to react. Sampling never blocks the
    /// feed; poll it from your own thread at any cadence.
    ///
    /// Forwarded to
    /// [`thetadatadx::Client::ring_occupancy`] so the value
    /// matches every other binding (C ABI, TypeScript, C++). Returns
    /// 0 before `start_streaming` and after `stop_streaming`.
    fn ring_occupancy(&self) -> usize {
        self.client.stream().ring_occupancy()
    }

    /// Configured capacity of the streaming event ring in slots (the
    /// ``streaming_ring_size`` setting, a power of two).
    ///
    /// The fixed denominator for :meth:`ring_occupancy`: when the
    /// occupancy sample approaches this value the ring is saturating
    /// and further events will be dropped (counted by
    /// :meth:`dropped_event_count`). Returns 0 before
    /// `start_streaming` and after `stop_streaming`.
    fn ring_capacity(&self) -> usize {
        self.client.stream().ring_capacity()
    }

    /// Milliseconds since the most recent inbound streaming frame of
    /// any kind (data tick, heartbeat, control), or ``None`` when
    /// streaming has not started or no frame has been received yet.
    ///
    /// The operator-facing staleness clock: a healthy session stays in
    /// the low hundreds of milliseconds (the upstream heartbeats even
    /// when no market data flows), so a steadily growing value is the
    /// earliest external signal of a dead or wedged connection.
    fn millis_since_last_event(&self) -> Option<u64> {
        self.client.stream().millis_since_last_event()
    }

    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// streaming frame of any kind. Returns ``0`` when streaming has
    /// not started or no frame has been received yet. Raw feed for
    /// :meth:`millis_since_last_event`, exposed for callers
    /// correlating against their own pipeline timestamps.
    fn last_event_received_at_unix_nanos(&self) -> i64 {
        self.client.stream().last_event_received_at_unix_nanos()
    }

    /// Address (``host:port``) of the streaming server the current
    /// session is connected to, following the session across
    /// auto-reconnects. ``None`` when streaming has not started.
    fn last_connected_addr(&self) -> Option<String> {
        self.client.stream().last_connected_addr()
    }

    /// Snapshot of full-stream subscriptions (e.g.
    /// `SecType.OPTION.full_trades()`).
    ///
    /// Returns the same typed `Subscription` values the caller passes
    /// to `subscribe()`. Quote is never a valid full-stream kind on
    /// the streaming wire, so any such row from the core is dropped from
    /// the projection. Empty list when streaming has not started.
    ///
    /// Mirrors the cross-binding contract on the C++
    /// `Stream::active_full_subscriptions` (see
    /// `sdks/cpp/include/thetadatadx.hpp`) and the standalone
    /// [`crate::fpss_client::StreamingClient::active_full_subscriptions`].
    fn active_full_subscriptions(&self) -> pyo3::PyResult<Vec<crate::fluent::PySubscription>> {
        use crate::errors::to_py_err;
        use thetadatadx::fpss::protocol::{FullSubscriptionKind, SubscriptionKind};
        self.client
            .stream()
            .active_full_subscriptions()
            .map(|subs| {
                subs.into_iter()
                    .filter_map(|(kind, sec_type)| {
                        let full_kind = match kind {
                            SubscriptionKind::Trade => FullSubscriptionKind::Trades,
                            SubscriptionKind::OpenInterest => FullSubscriptionKind::OpenInterest,
                            SubscriptionKind::Quote => return None,
                            _ => return None,
                        };
                        Some(crate::fluent::PySubscription {
                            inner: thetadatadx::fpss::protocol::Subscription::Full {
                                sec_type,
                                kind: full_kind,
                            },
                        })
                    })
                    .collect()
            })
            .map_err(to_py_err)
    }

    /// Cumulative count of user-callback panics caught by the
    /// Disruptor consumer's `catch_unwind` boundary. Mirrors the
    /// `panic_count()` getter on the standalone
    /// [`crate::fpss_client::StreamingClient`] and the upstream
    /// [`thetadatadx::Client::panic_count`].
    fn panic_count(&self) -> u64 {
        self.client.stream().panic_count()
    }

    /// Set the slow-callback wall-clock threshold in microseconds.
    ///
    /// When a callback invocation runs longer than ``threshold_us``,
    /// :meth:`slow_callback_count` increments and a rate-limited warning
    /// is logged. Pass ``0`` to disable the watchdog (the default).
    ///
    /// Observability only: the watchdog never cancels or kills the
    /// callback. The counter and log let operators detect a callback
    /// that has outgrown its budget and decide how to respond. No-op
    /// when streaming has not started.
    fn set_slow_callback_threshold_us(&self, threshold_us: u64) {
        self.client
            .stream()
            .set_slow_callback_threshold(std::time::Duration::from_micros(threshold_us));
    }

    /// Cumulative count of user-callback invocations whose wall-clock
    /// duration exceeded the threshold set by
    /// :meth:`set_slow_callback_threshold_us`. Returns 0 when the
    /// watchdog is disabled or streaming has not started. Mirrors the
    /// `slow_callback_count()` getter on the standalone
    /// [`crate::fpss_client::StreamingClient`] and the upstream
    /// [`thetadatadx::Client::slow_callback_count`].
    fn slow_callback_count(&self) -> u64 {
        self.client.stream().slow_callback_count()
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
impl StreamView {
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
    /// client.stream.subscribe(stock.quote())
    /// client.stream.subscribe(option.trade())
    /// client.stream.subscribe(SecType.OPTION.full_trades())
    /// ```
    fn subscribe(&self, py: Python<'_>, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        // `coerce_subscription` reads the Python object, so it runs with
        // the GIL held; the subscribe is a blocking wire write, so the
        // GIL is released across it. Never hold the GIL across blocking
        // network I/O.
        let inner = fluent::coerce_subscription(sub)?;
        py.detach(|| self.client.stream().subscribe(inner))
            .map_err(to_py_err)
    }

    /// Bulk-subscribe a list / iterable of `Subscription` values.
    ///
    /// Stops at the first error and re-raises it; previously-installed
    /// subscriptions are NOT rolled back (the upstream streaming
    /// protocol does not support batched transactions).
    fn subscribe_many(&self, py: Python<'_>, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        // Coerce the list under the GIL, then release it across the
        // blocking wire writes — never hold the GIL across network I/O.
        let list = fluent::coerce_subscription_list(subs)?;
        py.detach(|| self.client.stream().subscribe_many(list))
            .map_err(to_py_err)
    }

    /// Polymorphic unsubscribe — fluent counterpart to `subscribe(sub)`.
    fn unsubscribe(&self, py: Python<'_>, sub: &Bound<'_, PyAny>) -> PyResult<()> {
        let inner = fluent::coerce_subscription(sub)?;
        py.detach(|| self.client.stream().unsubscribe(inner))
            .map_err(to_py_err)
    }

    /// Bulk-unsubscribe a list / iterable of `Subscription` values.
    fn unsubscribe_many(&self, py: Python<'_>, subs: &Bound<'_, PyAny>) -> PyResult<()> {
        let list = fluent::coerce_subscription_list(subs)?;
        py.detach(|| self.client.stream().unsubscribe_many(list))
            .map_err(to_py_err)
    }
}

// `Client` is THE pyclass name. No alias, no compat wrapper.

// ── AsyncClient — async-only sibling ───────────────────────
//
// The underlying `Client` exposes both sync and `*_async`
// historical methods. This thin wrapper holds a `Client`
// handle and proxies attribute access through `__getattr__`, but raises
// on access to non-`async_` methods so users that opt into the async
// surface do not accidentally call a blocking sync path.
//
// The wrapper is a disciplined façade over the same Rust core, exposing
// a narrower public Python surface.

/// Async-only sibling of [`Client`].
///
/// ```python
/// import asyncio
/// from thetadatadx import AsyncClient, Credentials, Config
///
/// async def main():
///     creds = Credentials.from_file("creds.txt")
///     client = await AsyncClient.connect(creds, Config.production())
///     ticks = await client.stock_history_eod_async("AAPL", "20240101", "20240301")
///     print(ticks.to_pandas().head())
///
/// asyncio.run(main())
/// ```
///
/// Construct with `await AsyncClient.connect(...)` (or
/// `await AsyncClient.connect_from_file("creds.txt")`) from inside a
/// coroutine so the auth + connect handshake resolves off the event loop
/// instead of stalling it. The synchronous `AsyncClient(creds, config)`
/// constructor stays available for building outside a running loop.
///
/// Attribute access is restricted to async-suffixed methods plus a
/// safelisted set of synchronous lifecycle methods that have no
/// async counterpart on the wrapped surface
/// (`subscribe`/`unsubscribe`/`stop_streaming`/...).
/// Sync method names safelisted for proxy access on
/// [`AsyncClient`]. Every name MUST exist as a
/// `#[pymethods]` entry on [`Client`]; the const-eval
/// assertion below pins that invariant at compile time so we cannot
/// promise a method that the inner pyclass does not implement.
///
/// `is_authenticated` lives only on `StreamingClient` (not on the unified
/// client) and `config` has no such getter, so neither appears in
/// this list. The remaining names map 1:1 to public methods on
/// `Client` reachable via `bound.getattr(name)`.
pub(crate) const ALLOWED_UNIFIED_PROXY_METHODS: &[&str] = &[
    // Subscription management.
    "subscribe",
    "subscribe_many",
    "unsubscribe",
    "unsubscribe_many",
    "active_subscriptions",
    "active_full_subscriptions",
    // Streaming lifecycle.
    "start_streaming",
    "stop_streaming",
    "shutdown",
    "reconnect",
    "streaming",
    "is_streaming",
    "await_drain",
    // Diagnostics.
    "dropped_event_count",
    "panic_count",
    "ring_occupancy",
    "ring_capacity",
    "slow_callback_count",
    "set_slow_callback_threshold_us",
    // FLATFILES namespace getter.
    "flat_files",
    // NOTE: `session_uuid` / `subscription_info` are NOT on
    // `Client` — they live on `StreamingSession` (returned
    // by `client.streaming(callback)`) per their natural lifecycle
    // scope. Reaching for them through the unified async surface
    // raises `AttributeError` via the runtime `bound.getattr` after
    // the allowlist check, identical to the sync client.
];

/// Allowlisted names that stay on `Client` and resolve directly rather
/// than via the `client.stream` [`StreamView`] surface. The stream-view
/// proxy set is [`ALLOWED_UNIFIED_PROXY_METHODS`] minus these two, derived
/// inline in `AsyncClient.__getattr__` so the lists cannot drift.
const DIRECT_ON_CLIENT: [&str; 2] = ["streaming", "flat_files"];

/// Hand-written `#[pymethods]` entries on `Client` outside
/// the generator-emitted streaming surface (`PYTHON_UNIFIED_FPSS_METHODS`).
/// Pairs with the generator-emitted set in the
/// `ALLOWED_UNIFIED_PROXY_METHODS` const-eval assertion below — every
/// name in `ALLOWED_UNIFIED_PROXY_METHODS` must appear in either this
/// list or `PYTHON_UNIFIED_FPSS_METHODS`, otherwise the build fails.
const HANDWRITTEN_UNIFIED_PYMETHODS: &[&str] = &[
    // Hand-written streaming-session factory (stays on `Client`).
    "streaming",
    // FLATFILES namespace getter (lives in `flatfile_methods.rs`,
    // stays on `Client`).
    "flat_files",
    // Subscription management — hand-written to accept polymorphic
    // `Subscription` PyAny inputs; lives on the `client.stream`
    // `StreamView` surface (lib.rs).
    "subscribe",
    "subscribe_many",
    "unsubscribe",
    "unsubscribe_many",
    // Full-stream subscription snapshot lives on the `client.stream`
    // `StreamView` surface (lib.rs).
    "active_full_subscriptions",
    // Diagnostic getters — `dropped_event_count`, `panic_count`,
    // `ring_occupancy`, `ring_capacity`, and `slow_callback_count`, plus
    // the slow-callback threshold setter, all live on the
    // `client.stream` `StreamView` surface (lib.rs). All forward to the
    // core `thetadatadx::Client` accessors so the counts match every
    // other binding.
    "dropped_event_count",
    "panic_count",
    "ring_occupancy",
    "ring_capacity",
    "slow_callback_count",
    "set_slow_callback_threshold_us",
];

/// `const fn` byte-equal helper for the compile-time guards in this
/// crate. PyO3 attribute names are ASCII, so byte equality on the
/// `str` bytes is an exact name compare.
pub(crate) const fn const_bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Compile-time assertion: every safelisted proxy name must
/// resolve to a real `#[pymethods]` entry on `Client`.
/// Without this check a name that passes the allowlist but has no
/// matching method would raise a confusing `AttributeError` from the
/// inner `getattr` at call time — pinning the inventory here surfaces
/// the mismatch at compile time instead.
const _: () = {
    let mut i = 0;
    while i < ALLOWED_UNIFIED_PROXY_METHODS.len() {
        let needle = ALLOWED_UNIFIED_PROXY_METHODS[i];
        let mut found = false;
        let mut j = 0;
        while j < HANDWRITTEN_UNIFIED_PYMETHODS.len() {
            if const_bytes_eq(
                HANDWRITTEN_UNIFIED_PYMETHODS[j].as_bytes(),
                needle.as_bytes(),
            ) {
                found = true;
                break;
            }
            j += 1;
        }
        if !found {
            let mut k = 0;
            while k < PYTHON_UNIFIED_FPSS_METHODS.len() {
                if const_bytes_eq(PYTHON_UNIFIED_FPSS_METHODS[k].as_bytes(), needle.as_bytes()) {
                    found = true;
                    break;
                }
                k += 1;
            }
        }
        assert!(
            found,
            "ALLOWED_UNIFIED_PROXY_METHODS contains a name not present \
             in `PYTHON_UNIFIED_FPSS_METHODS` (generated) nor in \
             `HANDWRITTEN_UNIFIED_PYMETHODS` — the AsyncClient \
             would promise a method Client does not implement."
        );
        i += 1;
    }
};

#[pyclass(module = "thetadatadx", name = "AsyncClient")]
struct AsyncClient {
    inner: Py<Client>,
}

#[pymethods]
impl AsyncClient {
    /// Synchronous constructor that runs the auth + connect handshake to
    /// completion before returning.
    ///
    /// Suitable for construction OUTSIDE a running event loop (module
    /// import, a worker thread, a `__main__` body before `asyncio.run`).
    /// Inside a coroutine, prefer ``await AsyncClient.connect(...)`` so
    /// the handshake does not stall the event loop.
    #[new]
    fn new(py: Python<'_>, creds: &Credentials, config: &Config) -> PyResult<Self> {
        let direct_config = resolve_direct_config(Some(config), None, None)?;
        let inner = Client::connect_blocking(py, creds.inner.clone(), direct_config)?;
        let client = Py::new(py, inner)?;
        Ok(Self { inner: client })
    }

    /// Awaitable constructor that yields a connected ``AsyncClient``
    /// without stalling the running event loop.
    ///
    /// The auth round-trip and gRPC channel setup resolve off the event
    /// loop, so other coroutines keep running while the connection is
    /// established. This is the preferred way to build an ``AsyncClient``
    /// from inside a coroutine::
    ///
    ///     client = await AsyncClient.connect(creds, config)
    ///
    /// The synchronous ``AsyncClient(creds, config)`` constructor remains
    /// available for construction outside a running loop.
    #[staticmethod]
    fn connect<'py>(
        py: Python<'py>,
        creds: &Credentials,
        config: &Config,
    ) -> PyResult<Bound<'py, PyAny>> {
        // Snapshot the config + credentials under the GIL before handing
        // the connect future to the runtime. `connect()` takes ownership
        // of the `DirectConfig`, and the outer `Config` handle may still
        // be mutated Python-side after this call returns its awaitable.
        let direct_config = {
            let guard = config.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        // Seed the process-global runtime from this client's runtime
        // config before the awaitable resolves, so `worker_threads` takes
        // effect when the first client in the process connects.
        runtime_from_config(&direct_config.runtime);
        let inner_creds = creds.inner.clone();
        spawn_awaitable(
            py,
            async move { thetadatadx::Client::connect(&inner_creds, direct_config).await },
            |py, client| {
                let wrapped = Client {
                    client: std::sync::Arc::new(client),
                    callback: Arc::new(Mutex::new(None)),
                };
                let inner = Py::new(py, wrapped)?;
                Ok(Py::new(py, Self { inner })?.into_any())
            },
        )
    }

    /// Convenience constructor: `AsyncClient.from_file("creds.txt")`.
    /// Accepts an optional `config` kwarg defaulting to
    /// `Config.production()` for non-production environment tests.
    #[staticmethod]
    #[pyo3(signature = (path, config=None))]
    fn from_file(py: Python<'_>, path: &str, config: Option<&Config>) -> PyResult<Self> {
        let creds = auth::Credentials::from_file(path).map_err(to_py_err)?;
        let direct_config = resolve_direct_config(config, None, None)?;
        let inner = Client::connect_blocking(py, creds, direct_config)?;
        let client = Py::new(py, inner)?;
        Ok(Self { inner: client })
    }

    /// Awaitable file constructor that yields a connected ``AsyncClient``
    /// without stalling the running event loop.
    ///
    /// Loads credentials from a two-line file (line 1 = email, line 2 =
    /// password) and connects off the event loop, defaulting to the
    /// production endpoint when no ``config`` is supplied::
    ///
    ///     client = await AsyncClient.connect_from_file("creds.txt")
    #[staticmethod]
    #[pyo3(signature = (path, config=None))]
    fn connect_from_file<'py>(
        py: Python<'py>,
        path: &str,
        config: Option<&Config>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let creds = Credentials::from_file(path)?;
        let owned_default;
        let cfg = match config {
            Some(c) => c,
            None => {
                owned_default = Config::production();
                &owned_default
            }
        };
        Self::connect(py, &creds, cfg)
    }

    /// Forward attribute access to the wrapped `Client`.
    /// Async-suffixed methods plus the safelisted lifecycle / streaming
    /// methods are reachable; everything else raises `AttributeError`
    /// so callers who picked the async surface stay on the async path.
    ///
    /// Every name in `ALLOWED` is checked at compile time (see the
    /// `_ALLOWED_NAMES_ON_UNIFIED` const-eval block below) to actually
    /// exist on `Client`. The list is verified by
    /// `_ALLOWED_NAMES`; `is_authenticated` (only on `StreamingClient`)
    /// and `config` (no such getter) are intentionally excluded so
    /// the proxy does not advertise methods the inner client lacks.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        if !name.ends_with("_async") && !ALLOWED_UNIFIED_PROXY_METHODS.contains(&name) {
            return Err(pyo3::exceptions::PyAttributeError::new_err(format!(
                "AsyncClient surfaces only `*_async` historical methods plus \
                 streaming lifecycle helpers; `{name}` is not on the async surface. \
                 Use `Client` for the synchronous historical methods."
            )));
        }
        let bound = self.inner.bind(py);
        // The historical and streaming surfaces moved off `Client` onto the
        // `client.historical` / `client.stream` sub-namespace views. Resolve
        // each proxied name against the surface that actually owns it so the
        // async façade keeps a flat call shape
        // (`await async_client.stock_history_eod_async(...)`,
        // `async_client.subscribe(...)`) over the restructured client.
        if name.ends_with("_async") {
            let historical = bound.getattr("historical")?;
            return Ok(historical.getattr(name)?.unbind());
        }
        if ALLOWED_UNIFIED_PROXY_METHODS.contains(&name) && !DIRECT_ON_CLIENT.contains(&name) {
            let stream = bound.getattr("stream")?;
            return Ok(stream.getattr(name)?.unbind());
        }
        Ok(bound.getattr(name)?.unbind())
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let bound = self.inner.bind(py);
        let inner_repr: String = bound.call_method0("__repr__")?.extract()?;
        Ok(inner_repr.replace("Client", "AsyncClient"))
    }
}

// ── Typed-pyclass streaming event path ─────────────────────────────────────────
//
// All streaming `#[pyclass]` definitions and the `fpss_event_to_typed`
// dispatcher (borrowed `&StreamEvent` → pyclass, single pass, no
// intermediate) live in a generated file whose SSOT is
// `crates/thetadatadx/fpss_event_schema.toml`. The generator is
// `crates/thetadatadx/build_support/fpss_events/`; regenerate via
// `cargo run --bin generate_sdk_surfaces --features config-file -- --write`.

include!("_generated/fpss_event_classes.rs");

include!("_generated/streaming_methods.rs");

mod streaming_session;
use streaming_session::StreamingSession;

// `start_streaming(cb)` plus the `StreamingSession` context manager is
// the sole streaming surface on the bundled client.

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
//     -> FFI_ArrowArrayStream  -- Arrow C Stream Interface export
//     -> pyarrow.Table (imported via RecordBatchReader._import_from_c, zero-copy buffers)
//     -> pandas.DataFrame | polars.DataFrame | user code

/// pyarrow.Table -> pandas.DataFrame. pandas 2.x is required for the
/// numpy-backed zero-copy path (see `pyproject.toml` extras for the
/// version pin).
pub(crate) fn pyarrow_table_to_pandas(py: Python<'_>, table: Py<PyAny>) -> PyResult<Py<PyAny>> {
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
pub(crate) fn pyarrow_table_to_polars(py: Python<'_>, table: Py<PyAny>) -> PyResult<Py<PyAny>> {
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
///
/// `gil_used = false` opts the module into PEP 703 free-threaded
/// interpreters (`python3.14t`). Without this attribute
/// the free-threaded build automatically re-enables the GIL on the
/// first import of this module — which would defeat the entire purpose
/// of shipping nogil wheels. Every `#[pyclass]` carries either
/// `frozen` (immutable, safe-by-construction), interior `Mutex` /
/// `RwLock` / atomic primitives, or `unsendable` (single-thread
/// affinity); see the per-pyclass audit in `feat/python-nogil-wheels`
/// PR body for the full matrix.
#[pymodule(gil_used = false)]
#[pyo3(name = "thetadatadx")]
fn thetadatadx_py(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Pin the ring rustls `CryptoProvider` as the process-wide default
    // before any TLS handshake. See the docstring on
    // `__internal_install_ring_crypto_provider` for the rationale.
    let _ = thetadatadx::__internal_install_ring_crypto_provider();

    // Install the tracing → Python logging bridge FIRST so any `tracing`
    // events emitted during the subsequent connect / config setup reach
    // user-configured `logging.getLogger("thetadatadx")` handlers.
    logging_bridge::install_logging_bridge();

    // The tokio runtime is built lazily on the first client connect, not
    // here, so a `Config.worker_threads` value set before that connect is
    // honoured (the runtime is sized from the first client's
    // `config.runtime`). `pyo3-async-runtimes` is taught to reuse that
    // same runtime at build time via `register_async_runtime`, keeping
    // the sync and async paths on one runtime, one request semaphore, one
    // connection pool. Building it here would freeze the worker count at
    // import, before the user can set it.

    m.add_class::<Credentials>()?;
    m.add_class::<Config>()?;
    m.add_class::<Client>()?;
    m.add_class::<HistoricalView>()?;
    m.add_class::<StreamView>()?;
    m.add_class::<streaming_batches::RecordBatchStream>()?;
    m.add_class::<AsyncClient>()?;
    m.add_class::<fpss_client::StreamingClient>()?;
    m.add_class::<mdds_client::HistoricalClient>()?;
    m.add_class::<StreamingSession>()?;
    fluent::register(m)?;
    m.add_class::<flatfile_methods::FlatFilesNamespace>()?;
    m.add_class::<flatfile_methods::FlatFileRowList>()?;
    register_fpss_event_classes(m)?;
    register_tick_classes(m)?;
    register_generated_utility_functions(m)?;
    register_generated_historical_builders(m)?;
    coerce::register_string_enums(m)?;
    register_generated_util_submodule(m)?;

    // Typed exception hierarchy — exports `thetadatadx.ThetaDataError`,
    // `thetadatadx.AuthenticationError`, etc. See [`errors`] for the
    // full tree + mapping from `thetadatadx::Error` variants.
    errors::register_exceptions(py, m)?;

    m.add_function(wrap_pyfunction!(decode_response_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(split_date_range, m)?)?;
    // Introspection helper for the offline `HistoricalClient` block-list
    // coverage test. Mirrors `mdds_client::FPSS_TOUCHING_METHODS`.
    m.add_function(wrap_pyfunction!(mdds_client::blocked_fpss_methods, m)?)?;
    // Offline streaming-saturation bench hooks (no network). Drive the real
    // Disruptor pipeline + the production per-event GIL/marshal/callback path,
    // plus the batched-delivery and Arrow-columnar throughput levers.
    // Bench-only; enrolled in `PY_NON_UTILITY_PYFUNCTIONS` in the parity gate.
    m.add_function(wrap_pyfunction!(bench_streaming::__bench_flood_events, m)?)?;
    m.add_function(wrap_pyfunction!(
        bench_streaming::__bench_flood_events_batched_calls,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        bench_streaming::__bench_flood_events_batched_list,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        bench_streaming::__bench_flood_events_arrow,
        m
    )?)?;
    Ok(())
}
