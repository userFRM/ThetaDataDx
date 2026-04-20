//! Python bindings for `thetadatadx` — wraps the Rust SDK via PyO3.
//!
//! This is NOT a reimplementation. Every call goes through the Rust crate,
//! giving Python users native performance for ThetaData market data access.

use pyo3::exceptions::{PyConnectionError, PyRuntimeError, PyTimeoutError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::sync::atomic::AtomicU64;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use tdbe::types::tick;
use thetadatadx::auth;
use thetadatadx::config;
use thetadatadx::fpss;

/// Shared tokio runtime for running async Rust from sync Python.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime")
    })
}

fn to_py_err(e: thetadatadx::Error) -> PyErr {
    match e {
        thetadatadx::Error::Auth { message, .. } => PyConnectionError::new_err(message),
        thetadatadx::Error::Config(msg) => PyValueError::new_err(msg),
        // `Error::Timeout` maps to Python's stdlib `builtins.TimeoutError`
        // (which inherits from `OSError` in 3.3+) so callers can write
        // `except TimeoutError`. Falls back through `except Exception`
        // for backward compat. Documented in
        // [docs/dev/w3-async-cancellation-design.md].
        thetadatadx::Error::Timeout { .. } => PyTimeoutError::new_err(e.to_string()),
        _ => PyRuntimeError::new_err(e.to_string()),
    }
}

/// Run an async future to completion while periodically honoring Python's
/// signal handlers. A blocking `runtime().block_on` inside `py.detach`
/// otherwise starves `KeyboardInterrupt` because the GIL is released and
/// signals can never be delivered.
///
/// Polls `Python::check_signals()` every 100ms. On Ctrl+C, returns the
/// `PyErr` raised by Python (typically `KeyboardInterrupt`); the in-flight
/// future is dropped and its gRPC channel is cancelled.
fn run_blocking<F, T>(py: Python<'_>, fut: F) -> PyResult<T>
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
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        Python::attach(|py| py.check_signals())?;
                    }
                }
            }
        })
    })
}

fn parse_sec_type(sec_type: &str) -> PyResult<tdbe::types::enums::SecType> {
    match sec_type.to_uppercase().as_str() {
        "STOCK" => Ok(tdbe::types::enums::SecType::Stock),
        "OPTION" => Ok(tdbe::types::enums::SecType::Option),
        "INDEX" => Ok(tdbe::types::enums::SecType::Index),
        other => Err(PyValueError::new_err(format!(
            "unknown sec_type: {other:?} (expected STOCK, OPTION, or INDEX)"
        ))),
    }
}

// ── Credentials ──
// Lifecycle: intentionally hand-written (language-specific constructor semantics).
//
// `skip_from_py_object` matches every generated pyclass: these are constructed
// on the Python side and passed to Rust by reference (`&Credentials` in
// `ThetaDataDx::new`), never extracted by value, so the auto-derived
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
// Python-side setters (`config.reconnect_policy = "auto"`) still mutate in
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
    /// - "auto" (default): auto-reconnect matching Java terminal behavior.
    /// - "manual": no auto-reconnect, user calls reconnect explicitly.
    #[setter]
    fn set_reconnect_policy(&self, policy: &str) -> PyResult<()> {
        let parsed = match policy.to_lowercase().as_str() {
            "manual" => config::ReconnectPolicy::Manual,
            "auto" => config::ReconnectPolicy::Auto,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown reconnect_policy: {other:?} (expected \"auto\" or \"manual\")"
                )))
            }
        };
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.reconnect_policy = parsed;
        Ok(())
    }

    /// Get the current reconnect policy as a string.
    #[getter]
    fn get_reconnect_policy(&self) -> &'static str {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match guard.reconnect_policy {
            config::ReconnectPolicy::Auto => "auto",
            config::ReconnectPolicy::Manual => "manual",
            config::ReconnectPolicy::Custom(_) => "custom",
        }
    }

    /// Set whether to derive OHLCVC bars locally from trade events.
    ///
    /// When ``False``, only server-sent OHLCVC frames are emitted,
    /// reducing per-trade throughput overhead.
    #[setter]
    fn set_derive_ohlcvc(&self, enabled: bool) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.derive_ohlcvc = enabled;
    }

    /// Get the current OHLCVC derivation setting.
    #[getter]
    fn get_derive_ohlcvc(&self) -> bool {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.derive_ohlcvc
    }

    fn __repr__(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        format!(
            "Config(mdds={}:{}, fpss_hosts={})",
            guard.mdds_host,
            guard.mdds_port,
            guard.fpss_hosts.len()
        )
    }
}

// ── Typed-pyclass tick definitions (generated from tick_schema.toml) ──
//
// `tick_classes.rs` is the sole return path for every historical
// endpoint — matches the typed-struct approach used by Rust core,
// TypeScript, Go, and C++ FFI. The previous specialized PyDict-returning
// `*_ticks_to_columnar` helpers (in a since-deleted `tick_columnar.rs`)
// were a second, redundant fast-path; users now get a pandas DataFrame
// by chaining `thetadatadx.to_dataframe(client.stock_history_eod(...))`.

include!("tick_classes.rs");

include!("utility_functions.rs");

// ── FPSS streaming client ──

// ── BufferedEvent + converter (generated from fpss_event_schema.toml) ──
//
// The intermediate flat event type that crosses the mpsc channel from the
// FPSS Disruptor callback to the Python polling thread. Generator output
// is identical to the TypeScript SDK copy; `fpss_event_schema.toml` is the
// single source of truth.
include!("buffered_event.rs");

// ── Unified ThetaDataDx client ──

/// Unified ThetaData client — single connection for both historical and streaming.
///
/// This is the recommended entry point. Connects historical (MDDS/gRPC)
/// with a single authentication. Streaming (FPSS/TCP) starts lazily via
/// ``start_streaming()``.
///
/// Usage::
///
///     tdx = ThetaDataDx(creds, config)
///     eod = tdx.stock_history_eod("AAPL", "20240101", "20240301")
///     tdx.start_streaming()
///     tdx.subscribe_quotes("AAPL")
///     event = tdx.next_event(100)
///     tdx.stop_streaming()
/// Shared event receiver for the streaming callback -> Python poll bridge.
type EventRx = Arc<Mutex<Option<Arc<Mutex<std::sync::mpsc::Receiver<BufferedEvent>>>>>>;

#[pyclass]
struct ThetaDataDx {
    /// The underlying Rust unified client (Deref to DirectClient for historical).
    tdx: thetadatadx::ThetaDataDx,
    /// Created lazily when `start_streaming()` is called.
    rx: EventRx,
    /// Count of FPSS events dropped because the Python polling side
    /// disconnected before the callback could hand the event off. Lives
    /// on the struct (not inside the `start_streaming` closure) so the
    /// counter survives reconnect and is visible to callers via
    /// [`Self::dropped_events`]. `Arc<AtomicU64>` so each closure gets
    /// its own clone while they all increment the same underlying
    /// counter.
    dropped_events: Arc<AtomicU64>,
}

#[pymethods]
impl ThetaDataDx {
    // Lifecycle: intentionally hand-written (language-specific constructor semantics).

    /// Connect to ThetaData (historical only -- FPSS is NOT started).
    ///
    /// Authenticates once, opens gRPC channel. Call ``start_streaming()``
    /// to begin FPSS real-time data.
    #[new]
    fn new(creds: &Credentials, config: &Config) -> PyResult<Self> {
        // Snapshot the DirectConfig under the mutex — connect() takes
        // ownership, and the outer `Config` handle may still be mutated
        // Python-side after construction.
        let direct_config = {
            let guard = config.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        let tdx = runtime()
            .block_on(thetadatadx::ThetaDataDx::connect(
                &creds.inner,
                direct_config,
            ))
            .map_err(to_py_err)?;

        Ok(Self {
            tdx,
            rx: Arc::new(Mutex::new(None)),
            dropped_events: Arc::new(AtomicU64::new(0)),
        })
    }

    // ── DataFrame helper ──
    //
    // Previously offered per-endpoint `*_df(...)` convenience methods
    // (stock_history_eod_df / ohlc_df / trade_df / quote_df) that
    // bypassed the typed pyclass layer via specialized `Py<PyDict>`
    // (dict-of-lists) helpers in a generated `tick_columnar.rs`. That
    // path was an SSOT violation (two typed/untyped paths) and a
    // PyDict leak into the public Python API. Removed. The unified
    // path is:
    //
    //   ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
    //   df    = thetadatadx.to_dataframe(ticks)
    //
    // Same work, one clear path, typed pyclass as the sole endpoint
    // return — exactly what the cross-SDK contract promises.

    fn __repr__(&self) -> String {
        let streaming = if self.tdx.is_streaming() {
            "streaming=connected"
        } else {
            "streaming=none"
        };
        format!("ThetaDataDx(historical=connected, {streaming})")
    }

    // ── Typed-pyclass event streaming ──
    //
    // `next_event` is the single implementation (generated in
    // `streaming_methods.rs` from `sdk_surface.toml`). `next_event_typed`
    // is a public alias documented in the README for consumers that
    // prefer the explicit naming — it simply delegates so there's only
    // one code path to audit.

    /// Pull the next FPSS event as a typed Python object (alias for
    /// [`Self::next_event`]).
    ///
    /// Every variant returns a concrete `#[pyclass]` — `Quote`, `Trade`,
    /// `OpenInterest`, `Ohlcvc` for market data; `Simple` for control /
    /// diagnostic events (login, contract_assigned, disconnected, ...);
    /// `RawData` for unrecognized wire frames. No `PyDict` path anywhere.
    /// One allocation per event (the pyclass instance), field access via
    /// attribute (direct C-offset lookup).
    ///
    /// # Parity contract with the TypeScript SDK
    ///
    /// The `event.kind` discriminator is the stable cross-language tag:
    /// `"ohlcvc"`, `"open_interest"`, `"quote"`, `"trade"`, `"simple"`,
    /// `"raw_data"`. Concrete control-event names (`"login_success"`,
    /// `"contract_assigned"`, `"disconnected"`, `"market_open"`,
    /// `"market_close"`, `"server_error"`, `"reconnecting"`,
    /// `"reconnected"`, `"error"`, `"unknown_frame"`, `"unknown_data"`,
    /// `"unknown_control"`) live on `Simple.event_type`, mirroring
    /// `FpssSimplePayload.eventType` on the TS side. Payload field names
    /// match byte-for-byte (modulo snake_case ↔ camelCase). Both surfaces
    /// are generated from `fpss_event_schema.toml` — adding a field
    /// regenerates both SDKs in lockstep, so the discriminator and
    /// payload shape cannot drift.
    ///
    /// Idiomatic nesting differs by design: TS exposes a
    /// discriminated-union struct (`event.simple.eventType`), Python
    /// dispatches on pyclass (`event.event_type` where
    /// `isinstance(event, Simple)`). Consumer code ports across
    /// languages with a `.kind` switch and identical field names.
    fn next_event_typed(&self, py: Python<'_>, timeout_ms: u64) -> PyResult<Option<Py<PyAny>>> {
        self.next_event(py, timeout_ms)
    }

    /// Cumulative count of FPSS events dropped because the Python polling
    /// side disconnected before the FPSS callback could hand them off.
    ///
    /// Counter lives on the client instance (not inside the
    /// `start_streaming` / `reconnect` closures), so:
    ///
    /// * the value survives reconnect (otherwise every reconnect would
    ///   reset observability to zero), and
    /// * consumers can call ``tdx.dropped_events()`` at any point —
    ///   before streaming starts (returns 0), during (live count), or
    ///   after stop/shutdown (post-mortem count).
    ///
    /// Enabling ``RUST_LOG=thetadatadx::sdk::streaming=debug`` emits
    /// per-drop log lines; this getter is the cheap path to sample the
    /// total without scraping logs.
    fn dropped_events(&self) -> u64 {
        self.dropped_events.load(std::sync::atomic::Ordering::Relaxed)
    }
}

// ── Typed-pyclass FPSS event path ─────────────────────────────────────────
//
// All FPSS `#[pyclass]` definitions and the `BufferedEvent` → typed
// dispatch live in a generated file whose SSOT is
// `crates/thetadatadx/fpss_event_schema.toml`. The generator is
// `crates/thetadatadx/build_support/fpss_events.rs`; regenerate via
// `cargo run --bin generate_sdk_surfaces --features config-file -- --write`.

include!("fpss_event_classes.rs");

include!("streaming_methods.rs");

include!("historical_methods.rs");

// ── pandas DataFrame / polars adapter ──
//
// Single source of `PyDict` in the Python SDK. Scope: `pandas.DataFrame`
// and `polars.DataFrame` both consume dict-of-lists natively as their
// constructor input. There is no public-API PyDict — every endpoint
// return is a typed `#[pyclass]`, every FPSS event is a typed
// `#[pyclass]`, every utility (greeks / IV) is a typed `#[pyclass]`.
// The dict below exists exactly between `list[TickClass]` and pandas'
// `pd.DataFrame(dict)` entry point and is never surfaced to Python
// users.

/// Internal-only pandas / polars adapter.
///
/// Pivots `list[TickClass]` into the `dict[str, list]` shape that
/// `pandas.DataFrame(...)` / `polars.DataFrame(...)` accept as native
/// constructor input. **This is the sole `PyDict` allocation in the
/// Python SDK** — it is private, never exposed to Python users, and
/// the intermediate dict is immediately consumed by pandas / polars.
///
/// Column names and order come from `columns_for_qualname` (generated
/// from `crates/thetadatadx/tick_schema.toml`), not from Python-side
/// reflection. This guarantees:
///
/// 1. **Deterministic column order** across PyO3 versions — `__dir__`
///    ordering is interpreter-dependent.
/// 2. **Empty-list schema preservation** — if the caller knows the expected
///    tick type at compile time, it can pass `empty_qualname_hint` so
///    pandas/polars still see the correct column set (and can infer dtypes
///    on insert) when the result set is empty.
fn tick_list_to_pandas_input<'py>(
    py: Python<'py>,
    ticks: &Bound<'py, pyo3::types::PyList>,
    empty_qualname_hint: Option<&str>,
) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    let columns: &[&str] = if ticks.len() == 0 {
        match empty_qualname_hint.and_then(columns_for_qualname) {
            Some(cols) => cols,
            // No hint and empty list -> empty dict. Matches legacy behaviour
            // for callers that don't know the tick type statically.
            None => return Ok(out),
        }
    } else {
        let first = ticks.get_item(0)?;
        let qualname: String = first.get_type().qualname()?.extract()?;
        match columns_for_qualname(&qualname) {
            Some(cols) => cols,
            None => {
                return Err(PyRuntimeError::new_err(format!(
                    "tick_list_to_pandas_input: unknown tick type `{qualname}` — \
                     expected a value from `thetadatadx`'s typed tick classes"
                )));
            }
        }
    };
    for name in columns {
        let col = pyo3::types::PyList::empty(py);
        for i in 0..ticks.len() {
            let item = ticks.get_item(i)?;
            col.append(item.getattr(*name)?)?;
        }
        out.set_item(*name, col)?;
    }
    Ok(out)
}

/// Internal helper: convert a list of tick pyclasses into a pandas DataFrame.
fn pyclass_list_to_dataframe(py: Python<'_>, ticks: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let pandas = py.import("pandas").map_err(|_| {
        pyo3::exceptions::PyImportError::new_err(
            "pandas is required for DataFrame conversion. Install with: pip install pandas",
        )
    })?;
    let bound = ticks.bind(py);
    let list = bound.cast::<pyo3::types::PyList>().map_err(|_| {
        PyValueError::new_err("to_dataframe() expects a list of typed tick objects")
    })?;
    // `to_dataframe` / `to_polars` are generic entry points — they receive a
    // list of tick pyclasses without knowing the tick type at compile time.
    // Pass `None` for `empty_qualname_hint` so empty input lists yield an
    // empty dict (legacy behaviour). Callers that know the type statically
    // (e.g. per-endpoint helpers) can pass the qualname to preserve the
    // schema on empty results.
    let columnar = tick_list_to_pandas_input(py, list, None)?;
    let df = pandas.call_method1("DataFrame", (columnar,))?;
    Ok(df.unbind())
}

/// Convert a list of typed tick pyclasses to a pandas DataFrame.
///
/// Requires pandas to be installed (``pip install pandas``).
///
/// Historical endpoints return ``list[TickClass]`` (typed pyclass objects).
/// This helper pivots to the dict-of-lists shape pandas consumes natively.
///
/// Example::
///
///     ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
///     df = thetadatadx.to_dataframe(ticks)
#[pyfunction]
fn to_dataframe(py: Python<'_>, ticks: Py<PyAny>) -> PyResult<Py<PyAny>> {
    pyclass_list_to_dataframe(py, ticks)
}

/// Convert a list of typed tick pyclasses to a polars DataFrame.
///
/// Requires polars: ``pip install thetadatadx[polars]``
///
/// Example::
///
///     ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
///     df = thetadatadx.to_polars(ticks)
#[pyfunction]
fn to_polars(py: Python<'_>, ticks: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let polars = py.import("polars").map_err(|_| {
        pyo3::exceptions::PyImportError::new_err(
            "polars is not installed. Install it with: pip install thetadatadx[polars]",
        )
    })?;
    let bound = ticks.bind(py);
    let list = bound
        .cast::<pyo3::types::PyList>()
        .map_err(|_| PyValueError::new_err("to_polars() expects a list of typed tick objects"))?;
    // See `pyclass_list_to_dataframe` for the `None` rationale — generic
    // entry point, tick type is not known at compile time.
    let columnar = tick_list_to_pandas_input(py, list, None)?;
    let df = polars.call_method1("DataFrame", (columnar,))?;
    Ok(df.unbind())
}

// ── Module ──

/// thetadatadx — Native ThetaData SDK powered by Rust.
///
/// This Python package wraps the thetadatadx Rust crate via PyO3.
/// All data parsing, gRPC communication, and TCP streaming
/// happens in compiled Rust — Python is just the interface.
#[pymodule]
#[pyo3(name = "thetadatadx")]
fn thetadatadx_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Credentials>()?;
    m.add_class::<Config>()?;
    m.add_class::<ThetaDataDx>()?;
    register_fpss_event_classes(m)?;
    register_tick_classes(m)?;
    register_generated_utility_functions(m)?;
    m.add_function(wrap_pyfunction!(to_dataframe, m)?)?;
    m.add_function(wrap_pyfunction!(to_polars, m)?)?;
    Ok(())
}
