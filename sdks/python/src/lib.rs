//! Python bindings for `thetadatadx` — wraps the Rust SDK via PyO3.
//!
//! This is NOT a reimplementation. Every call goes through the Rust crate,
//! giving Python users native performance for ThetaData market data access.

use pyo3::exceptions::{PyConnectionError, PyRuntimeError, PyTimeoutError, PyValueError};
use pyo3::prelude::*;
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
// `tick_arrow.rs` is the schema-generated Arrow pipeline used by the
// DataFrame adapter -- zero-copy handoff to pyarrow via the Arrow C
// Data Interface. `tick_classes.rs` is the primary return path for
// all historical endpoints -- matches the typed-struct approach used
// by Rust core, TypeScript, Go, and C++ FFI.

include!("tick_classes.rs");

include!("tick_arrow.rs");

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

    // No per-endpoint `_df` / `_arrow` / `_polars` convenience wrappers.
    // Every historical endpoint returns `list[TickClass]`; chain
    // `thetadatadx.to_dataframe(ticks)` / `.to_polars(ticks)` /
    // `.to_arrow(ticks)` for the Arrow-backed conversion. One code
    // path, one SSOT, one place to audit. See `sdks/python/README.md`
    // "Historical endpoints → DataFrames" for the usage recipe.

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

// ── DataFrame adapter: Arrow columnar pipeline ──
//
// The adapter is built on the schema-generated Arrow surface in
// `tick_arrow.rs` (see `pyclass_list_to_arrow_table`,
// `arrow_schema_for_qualname`, and the per-class Arrow readers).
// Every user-facing entry point goes through that single
// dispatcher; this file owns only the thin pyarrow->{pandas, polars}
// bridges and the `to_arrow` / `to_dataframe` / `to_polars` pyo3
// signatures.
//
// Zero-copy path:
//   Vec<tick::T>     -- Rust-side (historical endpoints)
//     -> RecordBatch -- schema-generated arrow builders
//     -> arrow_pyarrow::Table
//     -> pyarrow.Table  (Arrow C Data Interface, zero-copy buffers)
//     -> pandas.DataFrame | polars.DataFrame | user code

/// Cast a `Py<PyAny>` user input into a typed `PyList` of tick pyclasses.
/// Shared by `to_arrow` / `to_dataframe` / `to_polars` so the error
/// message is consistent across entry points.
fn as_pyclass_list<'py>(
    py: Python<'py>,
    ticks: &'py Py<PyAny>,
    entry_point: &str,
) -> PyResult<Bound<'py, pyo3::types::PyList>> {
    let bound = ticks.bind(py);
    bound.cast::<pyo3::types::PyList>().cloned().map_err(|_| {
        PyValueError::new_err(format!(
            "{entry_point}() expects a list of typed tick objects (got `{}`)",
            bound
                .get_type()
                .qualname()
                .and_then(|q| q.extract::<String>())
                .unwrap_or_else(|_| "<unknown>".to_string())
        ))
    })
}

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
                "pandas is required for to_dataframe. Install with: pip install thetadatadx[pandas]",
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

/// Convert a list of typed tick pyclasses to a `pyarrow.Table` with a
/// zero-copy Arrow C Data Interface handoff.
///
/// This is the "raw Arrow" entry point: users wanting DuckDB /
/// Arrow-Flight / cuDF / polars-arrow pipelines can consume this
/// directly without a pandas / polars roundtrip.
///
/// Requires pyarrow: ``pip install thetadatadx[arrow]``
///
/// # Empty-list behaviour
///
/// * Non-empty list: schema is inferred from the pyclass qualname; all
///   items must be the same type (mixed types raise `RuntimeError`).
/// * Empty list: returns a zero-column `pyarrow.Table`. Pass the
///   concrete tick class via the `*_df` helpers on `ThetaDataDx` (or
///   build your own list of at least one item) if you need the typed
///   schema on empty results.
///
/// Example::
///
///     ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
///     table = thetadatadx.to_arrow(ticks)  # pyarrow.Table
///     con.register("eod", table)           # zero-copy into DuckDB
#[pyfunction]
fn to_arrow(py: Python<'_>, ticks: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let list = as_pyclass_list(py, &ticks, "to_arrow")?;
    pyclass_list_to_arrow_table(py, &list, None)
}

/// Convert a list of typed tick pyclasses to a pandas DataFrame.
///
/// Requires pandas + pyarrow: ``pip install thetadatadx[pandas]``
///
/// The DataFrame is backed by the Arrow columnar pipeline -- on pandas
/// 2.x the numeric columns alias the underlying Arrow buffers (zero
/// copy). Benchmarks at 100k EodTick rows show a ~6x wall-clock
/// speedup over the legacy dict-of-lists path.
///
/// Example::
///
///     ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
///     df = thetadatadx.to_dataframe(ticks)
#[pyfunction]
fn to_dataframe(py: Python<'_>, ticks: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let list = as_pyclass_list(py, &ticks, "to_dataframe")?;
    let table = pyclass_list_to_arrow_table(py, &list, None)?;
    pyarrow_table_to_pandas(py, table)
}

/// Convert a list of typed tick pyclasses to a polars DataFrame via
/// `polars.from_arrow` -- zero-copy at the Arrow boundary.
///
/// Requires polars + pyarrow: ``pip install thetadatadx[polars]``
///
/// Example::
///
///     ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
///     df = thetadatadx.to_polars(ticks)
#[pyfunction]
fn to_polars(py: Python<'_>, ticks: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let list = as_pyclass_list(py, &ticks, "to_polars")?;
    let table = pyclass_list_to_arrow_table(py, &list, None)?;
    pyarrow_table_to_polars(py, table)
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
    m.add_function(wrap_pyfunction!(to_arrow, m)?)?;
    m.add_function(wrap_pyfunction!(to_dataframe, m)?)?;
    m.add_function(wrap_pyfunction!(to_polars, m)?)?;
    Ok(())
}
