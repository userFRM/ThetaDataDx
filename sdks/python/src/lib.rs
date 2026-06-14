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
/// `pyo3_async_runtimes::tokio::init_with_runtime(...)`. No second runtime
/// is ever constructed, so the sync and async code paths share worker
/// threads, connection pools, and the request semaphore on the underlying
/// `MddsClient`.
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

    /// Stage streaming configuration (port 20100, testing, unstable).
    #[staticmethod]
    fn stage() -> Self {
        Self::from_direct(config::DirectConfig::stage())
    }

    /// Set the streaming reconnect policy.
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

    /// Set the per-class transient-failure attempt budget for the
    /// auto-reconnect path. Default `3`. No effect unless the
    /// reconnect policy is `Auto`.
    #[setter]
    fn set_reconnect_max_attempts(&self, max_attempts: u32) -> PyResult<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.max_attempts = max_attempts;
        }
        Ok(())
    }

    /// Set the per-class rate-limited (`TooManyRequests`) attempt budget
    /// for the auto-reconnect path. Default `100`. No effect unless the
    /// reconnect policy is `Auto`.
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
    /// `60`. No effect unless the reconnect policy is `Auto`.
    #[setter]
    fn set_reconnect_stable_window_secs(&self, secs: u64) -> PyResult<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.stable_window = std::time::Duration::from_secs(secs);
        }
        Ok(())
    }

    /// Set the reconnect delay (ms) honoured for generic transient
    /// disconnects (TimedOut, ServerRestarting, Unspecified, …).
    /// Plumbed through to the streaming I/O loop at connect time.
    /// Default ``2_000``.
    #[setter]
    fn set_reconnect_wait_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.wait_ms = ms;
    }

    /// Current reconnect ``wait_ms`` value (default ``2_000``).
    #[getter]
    fn get_reconnect_wait_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.wait_ms
    }

    /// Set the reconnect delay (ms) honoured for ``TooManyRequests``
    /// rate-limited disconnects. Default ``130_000`` (matches the
    /// Java terminal's 130 s rate-limit cooldown).
    #[setter]
    fn set_reconnect_wait_rate_limited_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.wait_rate_limited_ms = ms;
    }

    /// Current reconnect ``wait_rate_limited_ms`` value (default ``130_000``).
    #[getter]
    fn get_reconnect_wait_rate_limited_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.wait_rate_limited_ms
    }

    /// Set the cap (ms) on the exponential generic-transient reconnect
    /// ladder. The ladder starts at ``reconnect_wait_ms`` and doubles
    /// per consecutive attempt up to this value. Default ``30_000``.
    #[setter]
    fn set_reconnect_wait_max_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.wait_max_ms = ms;
    }

    /// Current reconnect ``wait_max_ms`` value (default ``30_000``).
    #[getter]
    fn get_reconnect_wait_max_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.wait_max_ms
    }

    /// Set the flat reconnect cadence (ms) for ``ServerRestarting``
    /// disconnects. Default ``5_000``.
    #[setter]
    fn set_reconnect_wait_server_restart_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.wait_server_restart_ms = ms;
    }

    /// Current reconnect ``wait_server_restart_ms`` value (default ``5_000``).
    #[getter]
    fn get_reconnect_wait_server_restart_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.wait_server_restart_ms
    }

    /// Set the jitter strategy applied to every reconnect delay.
    /// Accepts ``"full"`` (default), ``"equal"``, ``"decorrelated"``,
    /// or ``"none"`` (case-insensitive).
    #[setter]
    fn set_reconnect_jitter(&self, mode: &str) -> PyResult<()> {
        let parsed = config::JitterMode::parse(mode).ok_or_else(|| {
            PyValueError::new_err(format!(
                "unknown reconnect_jitter: {mode:?} (expected \"full\", \"equal\", \"decorrelated\", or \"none\")"
            ))
        })?;
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.jitter = parsed;
        Ok(())
    }

    /// Current reconnect jitter mode as a lowercase string.
    #[getter]
    fn get_reconnect_jitter(&self) -> &'static str {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.jitter.as_str()
    }

    /// Set the wall-clock reconnect envelope (seconds) for the
    /// generic-transient and server-restart classes, measured from the
    /// first attempt of a consecutive-reconnect sequence. ``0``
    /// disables the envelope (attempt budgets only). Default ``300``.
    /// No effect unless the reconnect policy is ``Auto``.
    #[setter]
    fn set_reconnect_max_elapsed_secs(&self, secs: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.max_elapsed = std::time::Duration::from_secs(secs);
        }
    }

    /// Current wall-clock reconnect envelope in seconds (default
    /// ``300``; ``0`` = disabled). Reads the default-limits value when
    /// the policy is not ``Auto``.
    #[getter]
    fn get_reconnect_max_elapsed_secs(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(limits) => limits.max_elapsed.as_secs(),
            _ => config::ReconnectAttemptLimits::default()
                .max_elapsed
                .as_secs(),
        }
    }

    /// Set the ``ServerRestarting`` reconnect attempt budget. Default
    /// ``60``. No effect unless the reconnect policy is ``Auto``.
    #[setter]
    fn set_reconnect_max_server_restart_attempts(&self, n: u32) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {
            limits.max_server_restart_attempts = n;
        }
    }

    /// Current ``ServerRestarting`` reconnect attempt budget (default
    /// ``60``). Reads the default-limits value when the policy is not
    /// ``Auto``.
    #[getter]
    fn get_reconnect_max_server_restart_attempts(&self) -> u32 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(limits) => limits.max_server_restart_attempts,
            _ => config::ReconnectAttemptLimits::default().max_server_restart_attempts,
        }
    }

    /// Current generic-transient reconnect attempt budget (default
    /// ``30``). Reads the default-limits value when the policy is not
    /// ``Auto``.
    #[getter]
    fn get_reconnect_max_attempts(&self) -> u32 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(limits) => limits.max_attempts,
            _ => config::ReconnectAttemptLimits::default().max_attempts,
        }
    }

    /// Current rate-limited reconnect attempt budget (default ``100``).
    /// Reads the default-limits value when the policy is not ``Auto``.
    #[getter]
    fn get_reconnect_max_rate_limited_attempts(&self) -> u32 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(limits) => limits.max_rate_limited_attempts,
            _ => config::ReconnectAttemptLimits::default().max_rate_limited_attempts,
        }
    }

    /// Current stable-window reset interval in seconds (default ``60``).
    /// Reads the default-limits value when the policy is not ``Auto``.
    #[getter]
    fn get_reconnect_stable_window_secs(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &guard.reconnect.policy {
            config::ReconnectPolicy::Auto(limits) => limits.stable_window.as_secs(),
            _ => config::ReconnectAttemptLimits::default()
                .stable_window
                .as_secs(),
        }
    }

    /// Set the subscription-replay burst size used after an
    /// auto-reconnect: frames are written in bursts of this many, each
    /// burst flushed and followed by a jittered ``replay_pace_ms``
    /// pause. Minimum ``1`` (validated at connect). Default ``50``.
    #[setter]
    fn set_reconnect_replay_burst_size(&self, n: u32) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.replay_burst_size = n;
    }

    /// Current ``replay_burst_size`` value (default ``50``).
    #[getter]
    fn get_reconnect_replay_burst_size(&self) -> u32 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.replay_burst_size
    }

    /// Set the pause (ms) between subscription-replay bursts after an
    /// auto-reconnect. ``0`` removes the pause. Default ``5``.
    #[setter]
    fn set_reconnect_replay_pace_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.replay_pace_ms = ms;
    }

    /// Current ``replay_pace_ms`` value (default ``5``).
    #[getter]
    fn get_reconnect_replay_pace_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect.replay_pace_ms
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

    // ── FPSS transport knobs ──────────────────────────────────────────
    //
    // Scalar tuning on ``DirectConfig.fpss`` mirroring the FFI / C++ /
    // TypeScript surface. Out-of-range values are rejected by the core
    // validator at connect time.

    /// Set the FPSS read timeout (ms): the no-frames deadline after
    /// which the streaming I/O loop declares the session dead and
    /// reconnects. Default ``3_000``; validated to ``[100, 60_000]``.
    #[setter]
    fn set_fpss_timeout_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.timeout_ms = ms;
    }

    /// Current ``fpss.timeout_ms`` value (default ``3_000``).
    #[getter]
    fn get_fpss_timeout_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.timeout_ms
    }

    /// Set the per-server connect timeout (ms) for the streaming
    /// connection. Default
    /// ``2_000``; validated to ``[1_000, 60_000]``.
    #[setter]
    fn set_fpss_connect_timeout_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.connect_timeout_ms = ms;
    }

    /// Current ``fpss.connect_timeout_ms`` value (default ``2_000``).
    #[getter]
    fn get_fpss_connect_timeout_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.connect_timeout_ms
    }

    /// Set the FPSS heartbeat ping interval (ms). Default ``250``;
    /// validated to ``[100, 300_000]``.
    #[setter]
    fn set_fpss_ping_interval_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.ping_interval_ms = ms;
    }

    /// Current ``fpss.ping_interval_ms`` value (default ``250``).
    #[getter]
    fn get_fpss_ping_interval_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.ping_interval_ms
    }

    /// Set the FPSS event ring buffer size (slots). Must be a power of
    /// two ``>= 64`` (rejected at connect otherwise). Default
    /// ``131_072``.
    #[setter]
    fn set_fpss_ring_size(&self, n: usize) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.ring_size = n;
    }

    /// Current ``fpss.ring_size`` value (default ``131_072``).
    #[getter]
    fn get_fpss_ring_size(&self) -> usize {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.ring_size
    }

    /// Set the per-iteration blocking-read slice (ms) for the
    /// streaming I/O loop. Default ``25``; validated to ``[10, 500]``.
    #[setter]
    fn set_fpss_io_read_slice_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.io_read_slice_ms = ms;
    }

    /// Current ``fpss.io_read_slice_ms`` value (default ``25``).
    #[getter]
    fn get_fpss_io_read_slice_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.io_read_slice_ms
    }

    /// Set the last-frame watchdog (ms): when no frame of any kind has
    /// arrived for this long the I/O loop force-reconnects. ``0``
    /// disables. Default ``30_000``.
    #[setter]
    fn set_fpss_data_watchdog_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.data_watchdog_ms = ms;
    }

    /// Current ``fpss.data_watchdog_ms`` value (default ``30_000``;
    /// ``0`` = disabled).
    #[getter]
    fn get_fpss_data_watchdog_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.data_watchdog_ms
    }

    /// Set the TCP keepalive idle time (seconds) before the first
    /// kernel probe on a silent FPSS socket. Default ``5``; validated
    /// to ``[1, 7_200]``.
    #[setter]
    fn set_fpss_keepalive_idle_secs(&self, secs: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.keepalive_idle_secs = secs;
    }

    /// Current ``fpss.keepalive_idle_secs`` value (default ``5``).
    #[getter]
    fn get_fpss_keepalive_idle_secs(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.keepalive_idle_secs
    }

    /// Set the interval (seconds) between TCP keepalive probes.
    /// Default ``2``; validated to ``[1, 75]``.
    #[setter]
    fn set_fpss_keepalive_interval_secs(&self, secs: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.keepalive_interval_secs = secs;
    }

    /// Current ``fpss.keepalive_interval_secs`` value (default ``2``).
    #[getter]
    fn get_fpss_keepalive_interval_secs(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.keepalive_interval_secs
    }

    /// Set the number of unanswered TCP keepalive probes after which
    /// the kernel declares the FPSS connection dead (where the
    /// platform exposes the knob). Default ``2``; validated to
    /// ``[1, 10]``.
    #[setter]
    fn set_fpss_keepalive_retries(&self, n: u32) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.keepalive_retries = n;
    }

    /// Current ``fpss.keepalive_retries`` value (default ``2``).
    #[getter]
    fn get_fpss_keepalive_retries(&self) -> u32 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.keepalive_retries
    }

    /// Set the FPSS host-selection policy. Accepts ``"shuffled"``
    /// (default — fault-domain-aware per-client shuffle) or
    /// ``"fixed_order"`` (declared order verbatim), case-insensitive.
    #[setter]
    fn set_fpss_host_selection(&self, policy: &str) -> PyResult<()> {
        let parsed = config::HostSelectionPolicy::parse(policy).ok_or_else(|| {
            PyValueError::new_err(format!(
                "unknown fpss_host_selection: {policy:?} (expected \"shuffled\" or \"fixed_order\")"
            ))
        })?;
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.host_selection = parsed;
        Ok(())
    }

    /// Current FPSS host-selection policy as a lowercase string.
    #[getter]
    fn get_fpss_host_selection(&self) -> &'static str {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.host_selection.as_str()
    }

    /// Set the FPSS host-shuffle seed. ``None`` (default) derives a
    /// fresh per-client seed so a fleet shuffles independently; an
    /// explicit value makes the shuffled order deterministic — useful
    /// for fleet sharding and tests. Ignored under ``"fixed_order"``.
    #[setter]
    fn set_fpss_host_shuffle_seed(&self, seed: Option<u64>) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.host_shuffle_seed = seed;
    }

    /// Current ``fpss.host_shuffle_seed`` value (``None`` = per-client
    /// entropy).
    #[getter]
    fn get_fpss_host_shuffle_seed(&self) -> Option<u64> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.host_shuffle_seed
    }

    /// Set the wall-clock envelope (seconds) for one
    /// historical-channel retry sequence, measured from the first
    /// attempt. ``0`` disables the envelope (attempt budget only).
    /// Default ``300``.
    #[setter]
    fn set_retry_max_elapsed_secs(&self, secs: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.retry.max_elapsed = std::time::Duration::from_secs(secs);
    }

    /// Current ``retry.max_elapsed`` value in seconds (default ``300``;
    /// ``0`` = disabled).
    #[getter]
    fn get_retry_max_elapsed_secs(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.retry.max_elapsed.as_secs()
    }

    /// Toggle AWS-style full jitter on the flatfile retry ladder.
    /// Default ``True``; ``False`` gives the deterministic schedule,
    /// useful for tests that assert exact timings.
    #[setter]
    fn set_flatfiles_jitter(&self, jitter: bool) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.flatfiles.jitter = jitter;
    }

    /// Current ``flatfiles.jitter`` value (default ``True``).
    #[getter]
    fn get_flatfiles_jitter(&self) -> bool {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.flatfiles.jitter
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

    // ── RetryPolicy field setters/getters ─────────────────────────────
    //
    // Per-field access on ``DirectConfig.retry`` mirrors the FFI / C++
    // / TypeScript surface. The ``delay_for_attempt`` / ``capped_backoff``
    // methods stay Rust-only — they are method-shape helpers that
    // callers can recompute from the four field values if needed.

    /// Set the initial backoff delay (ms) for the historical-channel retry policy.
    /// Default ``250``. Subsequent retries double from here, capped
    /// at :attr:`retry_max_delay_ms`.
    #[setter]
    fn set_retry_initial_delay_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.retry.initial_delay = std::time::Duration::from_millis(ms);
    }

    /// Current ``retry.initial_delay`` value in ms.
    #[getter]
    fn get_retry_initial_delay_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        u64::try_from(guard.retry.initial_delay.as_millis()).unwrap_or(u64::MAX)
    }

    /// Set the upper-bound backoff delay (ms) for the
    /// historical-channel retry policy. Default ``30_000`` (30 s).
    #[setter]
    fn set_retry_max_delay_ms(&self, ms: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.retry.max_delay = std::time::Duration::from_millis(ms);
    }

    /// Current ``retry.max_delay`` value in ms.
    #[getter]
    fn get_retry_max_delay_ms(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        u64::try_from(guard.retry.max_delay.as_millis()).unwrap_or(u64::MAX)
    }

    /// Set the total attempt budget for the historical-channel retry policy. ``1``
    /// disables retry (single call only); higher values permit retries
    /// up to ``max_attempts - 1`` after the initial call. Default ``5``.
    #[setter]
    fn set_retry_max_attempts(&self, n: u32) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.retry.max_attempts = n;
    }

    /// Current ``retry.max_attempts`` value.
    #[getter]
    fn get_retry_max_attempts(&self) -> u32 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.retry.max_attempts
    }

    /// Toggle AWS-style full-jitter on the historical-channel retry policy. Default
    /// ``True``. ``False`` gives the deterministic backoff schedule
    /// ``min(max_delay, initial * 2^attempt)``, useful for tests that
    /// need to assert exact timings.
    #[setter]
    fn set_retry_jitter(&self, jitter: bool) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.retry.jitter = jitter;
    }

    /// Current ``retry.jitter`` value.
    #[getter]
    fn get_retry_jitter(&self) -> bool {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.retry.jitter
    }

    // ── FlatFilesConfig field setters/getters ─────────────────────────
    //
    // Per-field access on ``DirectConfig.flatfiles`` mirrors the FFI /
    // C++ / TypeScript surface. The two ``Duration`` fields cross the
    // binding boundary as ``u64`` seconds (matching the
    // human-meaningful units ``FlatFilesConfig`` documents); the
    // ``backoff_for_attempt`` / ``production_defaults`` helpers stay
    // Rust-only.

    /// Set the total attempt budget for the flatfile driver retry loop.
    /// ``1`` disables retry (single call only); higher values permit
    /// retries up to ``max_attempts - 1`` after the initial call.
    /// Default ``3``. Validated to the range ``[1, 10]`` at connect
    /// time.
    #[setter]
    fn set_flatfiles_max_attempts(&self, n: u32) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.flatfiles.max_attempts = n;
    }

    /// Current ``flatfiles.max_attempts`` value.
    #[getter]
    fn get_flatfiles_max_attempts(&self) -> u32 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.flatfiles.max_attempts
    }

    /// Set the initial backoff delay (seconds) for the flatfile driver
    /// retry loop. Doubles per attempt up to
    /// :attr:`flatfiles_max_backoff_secs`. Default ``1``.
    #[setter]
    fn set_flatfiles_initial_backoff_secs(&self, secs: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.flatfiles.initial_backoff = std::time::Duration::from_secs(secs);
    }

    /// Current ``flatfiles.initial_backoff`` value in seconds.
    #[getter]
    fn get_flatfiles_initial_backoff_secs(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.flatfiles.initial_backoff.as_secs()
    }

    /// Set the upper-bound backoff delay (seconds) for the flatfile
    /// driver retry loop. The doubling schedule never exceeds this
    /// value regardless of attempt number. Default ``4``. Must be
    /// greater than or equal to :attr:`flatfiles_initial_backoff_secs`
    /// (rejected at connect-time validate otherwise).
    #[setter]
    fn set_flatfiles_max_backoff_secs(&self, secs: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.flatfiles.max_backoff = std::time::Duration::from_secs(secs);
    }

    /// Current ``flatfiles.max_backoff`` value in seconds.
    #[getter]
    fn get_flatfiles_max_backoff_secs(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.flatfiles.max_backoff.as_secs()
    }

    // ── AuthConfig field setters/getters ──────────────────────────────
    //
    // Per-field access on ``DirectConfig.auth``. Both fields are
    // ``str``. Defaults: the upstream production Nexus URL and the
    // canonical ``"rust-thetadatadx"`` client type.

    /// Set the Nexus auth URL. Default matches the upstream production
    /// endpoint; override to redirect at a staging cluster.
    #[setter]
    fn set_nexus_url(&self, url: String) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.auth.nexus_url = url;
    }

    /// Current ``auth.nexus_url`` value.
    #[getter]
    fn get_nexus_url(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.auth.nexus_url.clone()
    }

    /// Set the ``QueryInfo.client_type`` identifier. Default is
    /// ``"rust-thetadatadx"``; override to identify a deployment fleet
    /// in server-side dashboards.
    #[setter]
    fn set_client_type(&self, client_type: String) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.auth.client_type = client_type;
    }

    /// Current ``auth.client_type`` value.
    #[getter]
    fn get_client_type(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.auth.client_type.clone()
    }

    // ── MetricsConfig field setter/getter ─────────────────────────────
    //
    // ``DirectConfig.metrics.port`` is ``Optional[int]``. ``None``
    // (the default) leaves the Prometheus exporter disabled even when
    // the ``metrics-prometheus`` cargo feature is compiled in; an
    // ``int`` binds the exporter on ``0.0.0.0:<port>``.

    /// Set the Prometheus exporter port. ``None`` (the default) keeps
    /// the exporter disabled; an ``int`` binds an HTTP listener whose
    /// ``/metrics`` endpoint exposes every counter and histogram.
    ///
    /// Raises ``ValueError`` if the value is outside the ``u16`` range
    /// (``0..=65535``).
    #[setter]
    fn set_metrics_port(&self, port: Option<u32>) -> PyResult<()> {
        let resolved = match port {
            Some(v) => Some(u16::try_from(v).map_err(|_| {
                PyValueError::new_err(format!("metrics_port must be in 0..=65535; got {v}"))
            })?),
            None => None,
        };
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.metrics.port = resolved;
        Ok(())
    }

    /// Current ``metrics.port`` setting. ``None`` means the exporter is
    /// disabled; an ``int`` is the bound port.
    #[getter]
    fn get_metrics_port(&self) -> Option<u16> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.metrics.port
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

    /// Set the streaming write-flush policy.
    ///
    /// Accepts ``"batched"`` (default, flushes on the PING heartbeat,
    /// roughly every 100 ms — best throughput) or ``"immediate"``
    /// (flushes after every wire write — lowest latency, higher
    /// per-frame syscall cost).
    #[setter]
    fn set_flush_mode(&self, mode: &str) -> pyo3::PyResult<()> {
        let parsed = match mode.to_ascii_lowercase().as_str() {
            "batched" => config::FpssFlushMode::Batched,
            "immediate" => config::FpssFlushMode::Immediate,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "flush_mode must be \"batched\" or \"immediate\"; got {other:?}"
                )));
            }
        };
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.fpss.flush_mode = parsed;
        Ok(())
    }

    /// Current streaming write-flush policy (``"batched"`` or
    /// ``"immediate"``).
    #[getter]
    fn get_flush_mode(&self) -> &'static str {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match guard.fpss.flush_mode {
            config::FpssFlushMode::Batched => "batched",
            config::FpssFlushMode::Immediate => "immediate",
            _ => "unknown",
        }
    }

    /// Override the historical gRPC host. Used by structural tests that
    /// need to point the historical channel at a known-refused endpoint
    /// to prove the streaming-only surface never opens it; production
    /// code paths should keep the `Config::production()` default.
    #[setter]
    fn set_mdds_host(&self, host: String) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.host = host;
    }

    /// Current historical gRPC host.
    #[getter]
    fn get_mdds_host(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.host.clone()
    }

    /// Override the historical gRPC port. Companion to `mdds_host` —
    /// same rationale and same test-only usage.
    #[setter]
    fn set_mdds_port(&self, port: u16) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.port = port;
    }

    /// Current historical gRPC port.
    #[getter]
    fn get_mdds_port(&self) -> u16 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.port
    }

    // ── Historical pool sizing ─────────────────────────────────────

    /// Set the number of concurrent in-flight gRPC requests.
    ///
    /// ``0`` (default) auto-detects from the Nexus subscription tier:
    /// FREE=1 / VALUE=2 / STANDARD=4 / PRO=8. Explicit values above
    /// the tier cap are clamped to the cap at connect time with a
    /// ``tracing::warn!`` — set ``override_tier_clamp = True`` to
    /// bypass (tests only).
    ///
    /// Examples
    /// --------
    /// Multi-day backfill on a PRO subscription::
    ///
    ///     cfg = Config.production()
    ///     cfg.concurrent_requests = 8
    ///     client = ThetaDataDxClient(creds, cfg)
    #[setter]
    fn set_concurrent_requests(&self, n: usize) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.concurrent_requests = n;
    }

    /// Current `concurrent_requests` setting (``0`` = auto-detect).
    #[getter]
    fn get_concurrent_requests(&self) -> usize {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.concurrent_requests
    }

    /// Set the warning threshold (in bytes) for buffered (non-streaming)
    /// historical responses. Endpoints whose decoded total exceeds this
    /// value log a `tracing::warn!` pointing the caller at the
    /// ``.stream()`` surface; the data is still delivered. ``0``
    /// disables the warning entirely. Default is ``100 * 1024 * 1024``
    /// (100 MiB).
    ///
    /// Examples
    /// --------
    ///     cfg = Config.production()
    ///     cfg.warn_on_buffered_threshold_bytes = 50 * 1024 * 1024
    #[setter]
    fn set_warn_on_buffered_threshold_bytes(&self, n: usize) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.warn_on_buffered_threshold_bytes = n;
    }

    /// Current ``warn_on_buffered_threshold_bytes`` setting (bytes).
    #[getter]
    fn get_warn_on_buffered_threshold_bytes(&self) -> usize {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.mdds.warn_on_buffered_threshold_bytes
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

// ── Unified ThetaDataDxClient client ──

/// Unified ThetaData client — single connection for both historical and streaming.
///
/// This is the recommended entry point. Connects historical (gRPC over
/// HTTP/2 + TLS) with a single authentication. Real-time streaming
/// starts lazily via ``start_streaming(callback)``.
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
// `frozen` — every `#[pymethods]` entry on this pyclass takes
// `&self` (never `&mut self`). The inner `tdx: Arc<...>` carries its
// own mutex / atomic state for transient surfaces; the pyclass shell
// is immutable from Rust's perspective, which lets PyO3 elide the
// `RefCell` borrow-check overhead on every attribute / method
// dispatch under the free-threaded interpreter. A future `&mut self`
// regression surfaces as a `cargo check` failure rather than slipping
// silently through.
#[pyclass(frozen)]
struct ThetaDataDxClient {
    /// The underlying Rust unified client (Deref to MddsClient for historical).
    ///
    /// Wrapped in `Arc<>` so the per-endpoint fluent builder pyclasses
    /// emitted by the generator (`<Endpoint>Builder`) can clone a cheap
    /// handle into the awaitable returned by `*_async()` terminals. The
    /// inner `thetadatadx::ThetaDataDxClient` is not `Clone` — its
    /// streaming mutex and subscription-tier state forbid it — so the
    /// builder cannot hold the value directly without Arc ref-counting.
    ///
    /// Shutdown contract: when the pyclass auto-drops while the GIL is
    /// held, the final `Arc::drop` may trigger the core
    /// `ThetaDataDxClient::Drop` chain, which joins the FPSS dispatcher
    /// thread that itself re-acquires the GIL via `Python::attach`.
    /// Holding the GIL across that join would deadlock. Callers MUST
    /// invoke `stop_streaming()` (the generated method uses `py.detach`
    /// around the teardown so the dispatcher exits cleanly) before
    /// letting the pyclass fall out of scope. The `with tdx.streaming(cb)`
    /// context manager pairs `start_streaming(cb)` with
    /// `stop_streaming() + await_drain(5000)` on exit to enforce this
    /// ordering automatically. The fully-shared `Arc<>` (cloned into
    /// every fluent builder pyclass) cannot enforce the contract at the
    /// `Drop` site without restructuring every accessor, so the
    /// invariant is enforced by documentation plus the explicit
    /// `stop_streaming(py)` path.
    tdx: std::sync::Arc<thetadatadx::ThetaDataDxClient>,
    /// User-registered Python callable that receives every streaming
    /// event after `start_streaming(callback)` succeeds. The dispatcher's
    /// drain thread acquires the GIL via `Python::attach` to invoke
    /// `callback(event)`; the streaming reader thread itself never
    /// touches Python. `None` before any `start_streaming` and after
    /// every `stop_streaming` / `shutdown`. `reconnect()` re-uses the
    /// stored handle so callers do not have to re-pass the callable.
    callback: Mutex<Option<Py<PyAny>>>,
}

#[pymethods]
impl ThetaDataDxClient {
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
    #[new]
    fn new(py: Python<'_>, creds: &Credentials, config: &Config) -> PyResult<Self> {
        // Snapshot the DirectConfig under the mutex — connect() takes
        // ownership, and the outer `Config` handle may still be mutated
        // Python-side after construction.
        let direct_config = {
            let guard = config.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        // Seed the process-global runtime from this client's runtime config
        // before the first `run_blocking` resolves it, so `worker_threads`
        // takes effect when the first client in the process connects.
        runtime_from_config(&direct_config.runtime);
        let inner_creds = creds.inner.clone();
        let tdx = run_blocking(py, async move {
            thetadatadx::ThetaDataDxClient::connect(&inner_creds, direct_config).await
        })?;

        Ok(Self {
            tdx: std::sync::Arc::new(tdx),
            callback: Mutex::new(None),
        })
    }

    /// Convenience constructor: `ThetaDataDxClient.from_file("creds.txt")`.
    /// Loads credentials from a two-line file and connects with the
    /// supplied `config`, defaulting to `Config.production()`.
    ///
    /// The `config` kwarg is optional: with no kwarg the constructor
    /// targets the production endpoint. Tests and dev / stage
    /// environments reach a single-arg constructor shape via
    /// `ThetaDataDxClient.from_file("creds.txt", config=Config.dev())`.
    /// Parity with `AsyncThetaDataDxClient.from_file()`,
    /// `MddsClient.from_file()`, and `FpssClient.from_file()` — every
    /// Python client exposes the same one-call file-construction shape.
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

    /// Cumulative count of streaming events the TLS reader could not
    /// publish into the bounded ring because the consumer fell behind
    /// and the ring was full.
    ///
    /// Forwarded directly to
    /// [`thetadatadx::ThetaDataDxClient::dropped_event_count`] so the count
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
        self.tdx.dropped_event_count()
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
    /// [`thetadatadx::ThetaDataDxClient::ring_occupancy`] so the value
    /// matches every other binding (C ABI, TypeScript, C++). Returns
    /// 0 before `start_streaming` and after `stop_streaming`.
    fn ring_occupancy(&self) -> usize {
        self.tdx.ring_occupancy()
    }

    /// Configured capacity of the streaming event ring in slots (the
    /// ``fpss_ring_size`` setting, a power of two).
    ///
    /// The fixed denominator for :meth:`ring_occupancy`: when the
    /// occupancy sample approaches this value the ring is saturating
    /// and further events will be dropped (counted by
    /// :meth:`dropped_event_count`). Returns 0 before
    /// `start_streaming` and after `stop_streaming`.
    fn ring_capacity(&self) -> usize {
        self.tdx.ring_capacity()
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
        self.tdx.millis_since_last_event()
    }

    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// streaming frame of any kind. Returns ``0`` when streaming has
    /// not started or no frame has been received yet. Raw feed for
    /// :meth:`millis_since_last_event`, exposed for callers
    /// correlating against their own pipeline timestamps.
    fn last_event_received_at_unix_nanos(&self) -> i64 {
        self.tdx.last_event_received_at_unix_nanos()
    }

    /// Address (``host:port``) of the streaming server the current
    /// session is connected to, following the session across
    /// auto-reconnects. ``None`` when streaming has not started.
    fn last_connected_addr(&self) -> Option<String> {
        self.tdx.last_connected_addr()
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
    /// subscriptions are NOT rolled back (the upstream streaming
    /// protocol does not support batched transactions).
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

// ── AsyncThetaDataDxClient — async-only sibling ───────────────────────
//
// The underlying `ThetaDataDxClient` exposes both sync and `*_async`
// historical methods. This thin wrapper holds a `ThetaDataDxClient`
// handle and proxies attribute access through `__getattr__`, but raises
// on access to non-`async_` methods so users that opt into the async
// surface do not accidentally call a blocking sync path.
//
// The wrapper is a disciplined façade over the same Rust core, exposing
// a narrower public Python surface.

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
/// Sync method names safelisted for proxy access on
/// [`AsyncThetaDataDxClient`]. Every name MUST exist as a
/// `#[pymethods]` entry on [`ThetaDataDxClient`]; the const-eval
/// assertion below pins that invariant at compile time so we cannot
/// promise a method that the inner pyclass does not implement.
///
/// `is_authenticated` lives only on `FpssClient` (not on the unified
/// client) and `config` has no such getter, so neither appears in
/// this list. The remaining names map 1:1 to public methods on
/// `ThetaDataDxClient` reachable via `bound.getattr(name)`.
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
    // FLATFILES namespace getter.
    "flat_files",
    // NOTE: `session_uuid` / `subscription_info` are NOT on
    // `ThetaDataDxClient` — they live on `StreamingSession` (returned
    // by `client.streaming(callback)`) per their natural lifecycle
    // scope. Reaching for them through the unified async surface
    // raises `AttributeError` via the runtime `bound.getattr` after
    // the allowlist check, identical to the sync client.
];

/// Hand-written `#[pymethods]` entries on `ThetaDataDxClient` outside
/// the generator-emitted streaming surface (`PYTHON_UNIFIED_FPSS_METHODS`).
/// Pairs with the generator-emitted set in the
/// `ALLOWED_UNIFIED_PROXY_METHODS` const-eval assertion below — every
/// name in `ALLOWED_UNIFIED_PROXY_METHODS` must appear in either this
/// list or `PYTHON_UNIFIED_FPSS_METHODS`, otherwise the build fails.
const HANDWRITTEN_UNIFIED_PYMETHODS: &[&str] = &[
    // Hand-written streaming-session factory.
    "streaming",
    // FLATFILES namespace getter (lives in `flatfile_methods.rs`).
    "flat_files",
    // Subscription management (hand-written on the unified client to
    // accept polymorphic `Subscription` PyAny inputs).
    "subscribe",
    "subscribe_many",
    "unsubscribe",
    "unsubscribe_many",
    "active_full_subscriptions",
    // Diagnostic getters — `dropped_event_count`, `panic_count`,
    // `ring_occupancy`, and `ring_capacity` live directly on
    // `ThetaDataDxClient` (lib.rs) and forward to the core
    // `thetadatadx::ThetaDataDxClient` accessors so the count matches
    // every other binding.
    "dropped_event_count",
    "panic_count",
    "ring_occupancy",
    "ring_capacity",
];

/// `const fn` byte-equal helper for the compile-time guard below.
/// PyO3 attribute names are ASCII; byte equality is exact.
const fn const_str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
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
/// resolve to a real `#[pymethods]` entry on `ThetaDataDxClient`.
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
            if const_str_eq(HANDWRITTEN_UNIFIED_PYMETHODS[j], needle) {
                found = true;
                break;
            }
            j += 1;
        }
        if !found {
            let mut k = 0;
            while k < PYTHON_UNIFIED_FPSS_METHODS.len() {
                if const_str_eq(PYTHON_UNIFIED_FPSS_METHODS[k], needle) {
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
             `HANDWRITTEN_UNIFIED_PYMETHODS` — the AsyncThetaDataDxClient \
             would promise a method ThetaDataDxClient does not implement."
        );
        i += 1;
    }
};

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
    /// Accepts an optional `config` kwarg defaulting to
    /// `Config.production()` for non-production environment tests.
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
        let tdx = Py::new(py, ThetaDataDxClient::new(py, &creds, cfg)?)?;
        Ok(Self { inner: tdx })
    }

    /// Forward attribute access to the wrapped `ThetaDataDxClient`.
    /// Async-suffixed methods plus the safelisted lifecycle / streaming
    /// methods are reachable; everything else raises `AttributeError`
    /// so callers who picked the async surface stay on the async path.
    ///
    /// Every name in `ALLOWED` is checked at compile time (see the
    /// `_ALLOWED_NAMES_ON_UNIFIED` const-eval block below) to actually
    /// exist on `ThetaDataDxClient`. The list is verified by
    /// `_ALLOWED_NAMES`; `is_authenticated` (only on `FpssClient`)
    /// and `config` (no such getter) are intentionally excluded so
    /// the proxy does not advertise methods the inner client lacks.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        if !name.ends_with("_async") && !ALLOWED_UNIFIED_PROXY_METHODS.contains(&name) {
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
// All FPSS `#[pyclass]` definitions and the `fpss_event_to_typed`
// dispatcher (borrowed `&FpssEvent` → pyclass, single pass, no
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
    m.add_class::<ThetaDataDxClient>()?;
    m.add_class::<AsyncThetaDataDxClient>()?;
    m.add_class::<fpss_client::FpssClient>()?;
    m.add_class::<mdds_client::MddsClient>()?;
    m.add_class::<StreamingSession>()?;
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
