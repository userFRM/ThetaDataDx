//! Python bindings over the Rust `thetadatadx` core. Every call crosses the
//! PyO3 boundary into the same Rust code path used by the CLI and FFI.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::sync::atomic::AtomicU64;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use tdbe::types::tick;
use thetadatadx::auth;
use thetadatadx::config;
use thetadatadx::fpss;

mod async_runtime;
mod chunking;
mod errors;
mod logging_bridge;

use async_runtime::spawn_awaitable;
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
    /// The underlying Rust unified client (Deref to MddsClient for historical).
    ///
    /// Wrapped in `Arc<>` so the per-endpoint fluent builder pyclasses
    /// emitted by the generator (`<Endpoint>Builder`) can clone a cheap
    /// handle into the awaitable returned by `*_async()` terminals. The
    /// inner `thetadatadx::ThetaDataDx` is not `Clone` — its FPSS mutex
    /// and subscription-tier state forbid it — so the builder cannot
    /// hold the value directly without Arc ref-counting.
    tdx: std::sync::Arc<thetadatadx::ThetaDataDx>,
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
            thetadatadx::ThetaDataDx::connect(&inner_creds, direct_config).await
        })?;

        Ok(Self {
            tdx: std::sync::Arc::new(tdx),
            rx: Arc::new(Mutex::new(None)),
            dropped_events: Arc::new(AtomicU64::new(0)),
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
        self.dropped_events
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

// ── Typed-pyclass FPSS event path ─────────────────────────────────────────
//
// All FPSS `#[pyclass]` definitions and the `BufferedEvent` → typed
// dispatch live in a generated file whose SSOT is
// `crates/thetadatadx/fpss_event_schema.toml`. The generator is
// `crates/thetadatadx/build_support/fpss_events/`; regenerate via
// `cargo run --bin generate_sdk_surfaces --features config-file -- --write`.

include!("fpss_event_classes.rs");

include!("streaming_methods.rs");

include!("historical_methods.rs");

// `decode_response_bytes(endpoint, chunks)` hook used by the external
// parity bench harness. Generator-emitted from `endpoint_surface.toml`
// so every new endpoint is auto-wired — no manual edits here. See
// `crates/thetadatadx/build_support/endpoints/render/python.rs::render_python_decode_bench`.
include!("decode_bench.rs");

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
///     [('20200101', '20201230'), ('20201231', '20211230'),
///      ('20211231', '20221230'), ('20221231', '20231231')]
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
    m.add_class::<ThetaDataDx>()?;
    register_fpss_event_classes(m)?;
    register_tick_classes(m)?;
    register_generated_utility_functions(m)?;
    register_generated_historical_builders(m)?;

    // Typed exception hierarchy — exports `thetadatadx.ThetaDataError`,
    // `thetadatadx.AuthenticationError`, etc. See [`errors`] for the
    // full tree + mapping from `thetadatadx::Error` variants.
    errors::register_exceptions(py, m)?;

    m.add_function(wrap_pyfunction!(decode_response_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(split_date_range, m)?)?;
    Ok(())
}
