//! Python bindings for `thetadatadx` — wraps the Rust SDK via PyO3.
//!
//! This is NOT a reimplementation. Every call goes through the Rust crate,
//! giving Python users native performance for ThetaData market data access.

use pyo3::exceptions::{PyConnectionError, PyRuntimeError, PyTimeoutError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
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
                        if let Err(e) = Python::attach(|py| py.check_signals()) {
                            return Err(e);
                        }
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

#[pyclass(from_py_object)]
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
        format!("Credentials(email={:?})", self.inner.email)
    }
}

// ── Config ──
// Lifecycle: intentionally hand-written (language-specific constructor semantics).

#[pyclass(from_py_object)]
#[derive(Clone)]
struct Config {
    inner: config::DirectConfig,
}

#[pymethods]
impl Config {
    /// Production configuration (ThetaData NJ datacenter).
    #[staticmethod]
    fn production() -> Self {
        Self {
            inner: config::DirectConfig::production(),
        }
    }

    /// Dev FPSS configuration (port 20200, infinite historical replay).
    #[staticmethod]
    fn dev() -> Self {
        Self {
            inner: config::DirectConfig::dev(),
        }
    }

    /// Stage FPSS configuration (port 20100, testing, unstable).
    #[staticmethod]
    fn stage() -> Self {
        Self {
            inner: config::DirectConfig::stage(),
        }
    }

    /// Set the FPSS reconnect policy.
    ///
    /// - "auto" (default): auto-reconnect matching Java terminal behavior.
    /// - "manual": no auto-reconnect, user calls reconnect explicitly.
    #[setter]
    fn set_reconnect_policy(&mut self, policy: &str) -> PyResult<()> {
        self.inner.reconnect_policy = match policy.to_lowercase().as_str() {
            "manual" => config::ReconnectPolicy::Manual,
            "auto" => config::ReconnectPolicy::Auto,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown reconnect_policy: {other:?} (expected \"auto\" or \"manual\")"
                )))
            }
        };
        Ok(())
    }

    /// Get the current reconnect policy as a string.
    #[getter]
    fn get_reconnect_policy(&self) -> &str {
        match self.inner.reconnect_policy {
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
    fn set_derive_ohlcvc(&mut self, enabled: bool) {
        self.inner.derive_ohlcvc = enabled;
    }

    /// Get the current OHLCVC derivation setting.
    #[getter]
    fn get_derive_ohlcvc(&self) -> bool {
        self.inner.derive_ohlcvc
    }

    fn __repr__(&self) -> String {
        format!(
            "Config(mdds={}:{}, fpss_hosts={})",
            self.inner.mdds_host,
            self.inner.mdds_port,
            self.inner.fpss_hosts.len()
        )
    }
}

// ── Tick converters + typed pyclasses (generated from tick_schema.toml) ──
//
// `tick_columnar.rs` is used internally by DataFrame wrappers (pandas
// ingest path). `tick_classes.rs` is the primary return path for all
// historical endpoints — matches the typed-struct approach used by Rust
// core, TypeScript, Go, and C++ FFI.

include!("tick_columnar.rs");

include!("tick_classes.rs");

include!("utility_functions.rs");

// ── FPSS streaming client ──

/// A buffered FPSS event ready for Python consumption.
///
/// Buffered FPSS event that can travel through an `mpsc` channel from the
/// Disruptor callback thread to the Python polling thread.
///
/// Tick data events carry decoded, named fields as key-value pairs.
/// Price fields are pre-converted to `f64` using `Price::to_f64()`.
#[derive(Clone, Debug)]
enum BufferedEvent {
    /// Quote tick with decoded fields.
    Quote {
        contract_id: i32,
        ms_of_day: i32,
        bid_size: i32,
        bid_exchange: i32,
        bid: f64,
        bid_condition: i32,
        ask_size: i32,
        ask_exchange: i32,
        ask: f64,
        ask_condition: i32,
        date: i32,
        received_at_ns: u64,
    },
    /// Trade tick with decoded fields.
    Trade {
        contract_id: i32,
        ms_of_day: i32,
        sequence: i32,
        ext_condition1: i32,
        ext_condition2: i32,
        ext_condition3: i32,
        ext_condition4: i32,
        condition: i32,
        size: i32,
        exchange: i32,
        price: f64,
        condition_flags: i32,
        price_flags: i32,
        volume_type: i32,
        records_back: i32,
        date: i32,
        received_at_ns: u64,
    },
    /// Open interest tick.
    OpenInterest {
        contract_id: i32,
        ms_of_day: i32,
        open_interest: i32,
        date: i32,
        received_at_ns: u64,
    },
    /// OHLCVC bar with decoded fields.
    Ohlcvc {
        contract_id: i32,
        ms_of_day: i32,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: i64,
        count: i64,
        date: i32,
        received_at_ns: u64,
    },
    /// Raw undecoded data (fallback).
    RawData { code: u8, payload: Vec<u8> },
    /// Non-tick events (login, contract, response, errors, etc.).
    Simple {
        kind: String,
        detail: Option<String>,
        id: Option<i32>,
    },
}

fn fpss_event_to_buffered(event: &fpss::FpssEvent) -> BufferedEvent {
    match event {
        fpss::FpssEvent::Data(data) => match data {
            fpss::FpssData::Quote {
                contract_id,
                ms_of_day,
                bid_size,
                bid_exchange,
                bid,
                bid_condition,
                ask_size,
                ask_exchange,
                ask,
                ask_condition,
                date,
                received_at_ns,
                ..
            } => BufferedEvent::Quote {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                bid_size: *bid_size,
                bid_exchange: *bid_exchange,
                bid: *bid,
                bid_condition: *bid_condition,
                ask_size: *ask_size,
                ask_exchange: *ask_exchange,
                ask: *ask,
                ask_condition: *ask_condition,
                date: *date,
                received_at_ns: *received_at_ns,
            },
            fpss::FpssData::Trade {
                contract_id,
                ms_of_day,
                sequence,
                ext_condition1,
                ext_condition2,
                ext_condition3,
                ext_condition4,
                condition,
                size,
                exchange,
                price,
                condition_flags,
                price_flags,
                volume_type,
                records_back,
                date,
                received_at_ns,
                ..
            } => BufferedEvent::Trade {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                sequence: *sequence,
                ext_condition1: *ext_condition1,
                ext_condition2: *ext_condition2,
                ext_condition3: *ext_condition3,
                ext_condition4: *ext_condition4,
                condition: *condition,
                size: *size,
                exchange: *exchange,
                price: *price,
                condition_flags: *condition_flags,
                price_flags: *price_flags,
                volume_type: *volume_type,
                records_back: *records_back,
                date: *date,
                received_at_ns: *received_at_ns,
            },
            fpss::FpssData::OpenInterest {
                contract_id,
                ms_of_day,
                open_interest,
                date,
                received_at_ns,
                ..
            } => BufferedEvent::OpenInterest {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                open_interest: *open_interest,
                date: *date,
                received_at_ns: *received_at_ns,
            },
            fpss::FpssData::Ohlcvc {
                contract_id,
                ms_of_day,
                open,
                high,
                low,
                close,
                volume,
                count,
                date,
                received_at_ns,
                ..
            } => BufferedEvent::Ohlcvc {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                open: *open,
                high: *high,
                low: *low,
                close: *close,
                volume: *volume,
                count: *count,
                date: *date,
                received_at_ns: *received_at_ns,
            },
            _ => BufferedEvent::Simple {
                kind: "unknown_data".to_string(),
                detail: None,
                id: None,
            },
        },
        fpss::FpssEvent::Control(ctrl) => match ctrl {
            fpss::FpssControl::LoginSuccess { permissions } => BufferedEvent::Simple {
                kind: "login_success".to_string(),
                detail: Some(permissions.clone()),
                id: None,
            },
            fpss::FpssControl::ContractAssigned { id, contract } => BufferedEvent::Simple {
                kind: "contract_assigned".to_string(),
                detail: Some(format!("{contract}")),
                id: Some(*id),
            },
            fpss::FpssControl::ReqResponse { req_id, result } => BufferedEvent::Simple {
                kind: "req_response".to_string(),
                detail: Some(format!("{result:?}")),
                id: Some(*req_id),
            },
            fpss::FpssControl::MarketOpen => BufferedEvent::Simple {
                kind: "market_open".to_string(),
                detail: None,
                id: None,
            },
            fpss::FpssControl::MarketClose => BufferedEvent::Simple {
                kind: "market_close".to_string(),
                detail: None,
                id: None,
            },
            fpss::FpssControl::ServerError { message } => BufferedEvent::Simple {
                kind: "server_error".to_string(),
                detail: Some(message.clone()),
                id: None,
            },
            fpss::FpssControl::Disconnected { reason } => BufferedEvent::Simple {
                kind: "disconnected".to_string(),
                detail: Some(format!("{reason:?}")),
                id: None,
            },
            fpss::FpssControl::Error { message } => BufferedEvent::Simple {
                kind: "error".to_string(),
                detail: Some(message.clone()),
                id: None,
            },
            fpss::FpssControl::Reconnecting {
                reason,
                attempt,
                delay_ms,
            } => BufferedEvent::Simple {
                kind: "reconnecting".to_string(),
                detail: Some(format!(
                    "reason={reason:?} attempt={attempt} delay_ms={delay_ms}"
                )),
                id: None,
            },
            fpss::FpssControl::Reconnected => BufferedEvent::Simple {
                kind: "reconnected".to_string(),
                detail: None,
                id: None,
            },
            fpss::FpssControl::UnknownFrame { code, payload } => BufferedEvent::Simple {
                kind: "unknown_frame".to_string(),
                detail: Some(format!(
                    "code={code} payload_hex={}",
                    payload
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                )),
                id: None,
            },
            _ => BufferedEvent::Simple {
                kind: "unknown_control".to_string(),
                detail: None,
                id: None,
            },
        },
        fpss::FpssEvent::RawData { code, payload } => BufferedEvent::RawData {
            code: *code,
            payload: payload.clone(),
        },
        _ => BufferedEvent::Simple {
            kind: "unknown".to_string(),
            detail: None,
            id: None,
        },
    }
}

// PyO3: set_item is infallible for primitive types (str, int, float, bool, bytes).
fn buffered_event_to_py(py: Python<'_>, event: &BufferedEvent) -> Py<PyAny> {
    let dict = PyDict::new(py);
    match event {
        BufferedEvent::Quote {
            contract_id,
            ms_of_day,
            bid_size,
            bid_exchange,
            bid,
            bid_condition,
            ask_size,
            ask_exchange,
            ask,
            ask_condition,
            date,
            received_at_ns,
        } => {
            dict.set_item("kind", "quote").unwrap();
            dict.set_item("contract_id", contract_id).unwrap();
            dict.set_item("ms_of_day", ms_of_day).unwrap();
            dict.set_item("bid_size", bid_size).unwrap();
            dict.set_item("bid_exchange", bid_exchange).unwrap();
            dict.set_item("bid", bid).unwrap();
            dict.set_item("bid_condition", bid_condition).unwrap();
            dict.set_item("ask_size", ask_size).unwrap();
            dict.set_item("ask_exchange", ask_exchange).unwrap();
            dict.set_item("ask", ask).unwrap();
            dict.set_item("ask_condition", ask_condition).unwrap();
            dict.set_item("date", date).unwrap();
            dict.set_item("received_at_ns", received_at_ns).unwrap();
        }
        BufferedEvent::Trade {
            contract_id,
            ms_of_day,
            sequence,
            ext_condition1,
            ext_condition2,
            ext_condition3,
            ext_condition4,
            condition,
            size,
            exchange,
            price,
            condition_flags,
            price_flags,
            volume_type,
            records_back,
            date,
            received_at_ns,
        } => {
            dict.set_item("kind", "trade").unwrap();
            dict.set_item("contract_id", contract_id).unwrap();
            dict.set_item("ms_of_day", ms_of_day).unwrap();
            dict.set_item("sequence", sequence).unwrap();
            dict.set_item("ext_condition1", ext_condition1).unwrap();
            dict.set_item("ext_condition2", ext_condition2).unwrap();
            dict.set_item("ext_condition3", ext_condition3).unwrap();
            dict.set_item("ext_condition4", ext_condition4).unwrap();
            dict.set_item("condition", condition).unwrap();
            dict.set_item("size", size).unwrap();
            dict.set_item("exchange", exchange).unwrap();
            dict.set_item("price", price).unwrap();
            dict.set_item("condition_flags", condition_flags).unwrap();
            dict.set_item("price_flags", price_flags).unwrap();
            dict.set_item("volume_type", volume_type).unwrap();
            dict.set_item("records_back", records_back).unwrap();
            dict.set_item("date", date).unwrap();
            dict.set_item("received_at_ns", received_at_ns).unwrap();
        }
        BufferedEvent::OpenInterest {
            contract_id,
            ms_of_day,
            open_interest,
            date,
            received_at_ns,
        } => {
            dict.set_item("kind", "open_interest").unwrap();
            dict.set_item("contract_id", contract_id).unwrap();
            dict.set_item("ms_of_day", ms_of_day).unwrap();
            dict.set_item("open_interest", open_interest).unwrap();
            dict.set_item("date", date).unwrap();
            dict.set_item("received_at_ns", received_at_ns).unwrap();
        }
        BufferedEvent::Ohlcvc {
            contract_id,
            ms_of_day,
            open,
            high,
            low,
            close,
            volume,
            count,
            date,
            received_at_ns,
        } => {
            dict.set_item("kind", "ohlcvc").unwrap();
            dict.set_item("contract_id", contract_id).unwrap();
            dict.set_item("ms_of_day", ms_of_day).unwrap();
            dict.set_item("open", open).unwrap();
            dict.set_item("high", high).unwrap();
            dict.set_item("low", low).unwrap();
            dict.set_item("close", close).unwrap();
            dict.set_item("volume", volume).unwrap();
            dict.set_item("count", count).unwrap();
            dict.set_item("date", date).unwrap();
            dict.set_item("received_at_ns", received_at_ns).unwrap();
        }
        BufferedEvent::RawData { code, payload } => {
            dict.set_item("kind", "raw_data").unwrap();
            dict.set_item("code", code).unwrap();
            dict.set_item("payload", pyo3::types::PyBytes::new(py, payload))
                .unwrap();
        }
        BufferedEvent::Simple { kind, detail, id } => {
            dict.set_item("kind", kind.as_str()).unwrap();
            if let Some(ref d) = detail {
                dict.set_item("detail", d.as_str()).unwrap();
            } else {
                dict.set_item("detail", py.None()).unwrap();
            }
            if let Some(i) = id {
                dict.set_item("id", i).unwrap();
            } else {
                dict.set_item("id", py.None()).unwrap();
            }
        }
    }
    dict.into_any().unbind()
}

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
        let tdx = runtime()
            .block_on(thetadatadx::ThetaDataDx::connect(
                &creds.inner,
                config.inner.clone(),
            ))
            .map_err(to_py_err)?;

        Ok(Self {
            tdx,
            rx: Arc::new(Mutex::new(None)),
        })
    }

    // ── DataFrame convenience wrappers ──
    //
    // Intentional exception: hand-written convenience methods wrapping generated
    // endpoints + dicts_to_dataframe(). Only the most common endpoints get _df
    // variants; users can call to_dataframe() on any endpoint result themselves.
    // Not generated because emitting _df for all 44 tick-returning endpoints
    // would be API noise with no SSOT benefit.

    /// Fetch stock EOD history and return a pandas DataFrame.
    fn stock_history_eod_df(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
    ) -> PyResult<Py<PyAny>> {
        // Go straight through the Rust SDK → `*_to_columnar` helper so we
        // avoid the pyclass allocation + __dir__ pivot round-trip.
        let ticks = run_blocking(py, async move {
            self.tdx.stock_history_eod(symbol, start_date, end_date).await
        })?;
        let columnar = eod_ticks_to_columnar(py, &ticks);
        columnar_to_dataframe(py, columnar)
    }

    /// Fetch stock OHLC history and return a pandas DataFrame.
    fn stock_history_ohlc_df(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Py<PyAny>> {
        let ticks = run_blocking(py, async move {
            self.tdx.stock_history_ohlc(symbol, date, interval).await
        })?;
        let columnar = ohlc_ticks_to_columnar(py, &ticks);
        columnar_to_dataframe(py, columnar)
    }

    /// Fetch stock trade history and return a pandas DataFrame.
    fn stock_history_trade_df(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
    ) -> PyResult<Py<PyAny>> {
        let ticks = run_blocking(py, async move {
            self.tdx.stock_history_trade(symbol, date).await
        })?;
        let columnar = trade_ticks_to_columnar(py, &ticks);
        columnar_to_dataframe(py, columnar)
    }

    /// Fetch stock quote history and return a pandas DataFrame.
    fn stock_history_quote_df(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Py<PyAny>> {
        let ticks = run_blocking(py, async move {
            self.tdx.stock_history_quote(symbol, date, interval).await
        })?;
        let columnar = quote_ticks_to_columnar(py, &ticks);
        columnar_to_dataframe(py, columnar)
    }

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
    // Intentional exception: hand-written high-throughput API that
    // preserves the mental model of `next_event` (one event at a time,
    // attribute/field access) while avoiding the per-event `PyDict`
    // allocation. Matches the typed-record approach used by comparable
    // market-data SDKs. Respects SSOT: generated methods still live in
    // `streaming_methods.rs`.

    /// Pull the next FPSS event as a typed Python object (`Quote`, `Trade`,
    /// `OpenInterest`, `Ohlcvc`) instead of a `PyDict`.
    ///
    /// Drop-in replacement for `next_event` with the same loop shape. One
    /// allocation per event (the pyclass instance), field access via
    /// attribute (direct C-offset lookup), and no 14-way `set_item` dance
    /// per event. Measured ~24x faster across the FFI boundary than the
    /// dict path.
    ///
    /// Non-tick events (login, contract_assigned, disconnected, ...) fall
    /// back to the dict shape so lifecycle handling doesn't need a second
    /// API.
    fn next_event_typed(
        &self,
        py: Python<'_>,
        timeout_ms: u64,
    ) -> PyResult<Option<Py<PyAny>>> {
        let rx_outer = self.rx.lock().unwrap_or_else(|e| e.into_inner());
        let rx_arc = match rx_outer.as_ref() {
            Some(arc) => Arc::clone(arc),
            None => {
                return Err(PyRuntimeError::new_err(
                    "streaming not started -- call start_streaming() first",
                ))
            }
        };
        drop(rx_outer);
        let timeout = std::time::Duration::from_millis(timeout_ms);
        // Three outcomes: event, benign timeout, or fatal disconnect.
        // Collapsing disconnect to None spins consumer while-loops at
        // 100% CPU on a dead socket.
        enum PollOutcome {
            Event(BufferedEvent),
            Timeout,
            Disconnected,
        }
        let result = py.detach(move || {
            let deadline = std::time::Instant::now() + timeout;
            let poll_interval = std::time::Duration::from_millis(1);
            loop {
                {
                    let rx = rx_arc.lock().unwrap_or_else(|e| e.into_inner());
                    match rx.try_recv() {
                        Ok(event) => return PollOutcome::Event(event),
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            return PollOutcome::Disconnected
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    }
                }
                let now = std::time::Instant::now();
                if now >= deadline {
                    return PollOutcome::Timeout;
                }
                std::thread::sleep(poll_interval.min(deadline - now));
            }
        });
        match result {
            PollOutcome::Event(event) => Ok(Some(buffered_event_to_typed(py, &event)?)),
            PollOutcome::Timeout => Ok(None),
            PollOutcome::Disconnected => Err(PyRuntimeError::new_err(
                "streaming channel disconnected -- call reconnect() or start_streaming() again",
            )),
        }
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

// ── pandas DataFrame helpers ──

/// Convert a list of typed tick pyclasses into a columnar dict of lists.
///
/// Historical endpoints return `list[TickClass]` (typed pyclass objects,
/// matching Rust/TS/Go/C++). For DataFrame construction we need the same
/// dict-of-lists shape pandas consumes natively; this helper does the
/// one pivot, skipping private attributes and anything non-field (bound
/// methods, `kind`, `__repr__`).
fn pyclass_list_to_columnar<'py>(
    py: Python<'py>,
    ticks: &Bound<'py, pyo3::types::PyList>,
) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    let n = ticks.len();
    if n == 0 {
        return Ok(out);
    }
    let first = ticks.get_item(0)?;
    let attrs_list = first.call_method0("__dir__")?;
    let attrs: Vec<String> = attrs_list.extract()?;
    for name in attrs.iter().filter(|a| !a.starts_with('_')) {
        // Skip callables and the synthetic `kind` accessor if any.
        let probe = first.getattr(name.as_str())?;
        if probe.is_callable() {
            continue;
        }
        let col = pyo3::types::PyList::empty(py);
        for i in 0..n {
            let item = ticks.get_item(i)?;
            col.append(item.getattr(name.as_str())?)?;
        }
        out.set_item(name.as_str(), col)?;
    }
    Ok(out)
}

/// Internal helper: convert a generated columnar dict (dict-of-lists, as
/// produced by the schema-generated `*_to_columnar` converters) into a
/// pandas DataFrame. Used by the `*_df` convenience wrappers so they can
/// skip the pyclass-list round-trip and the slower `__dir__` pivot.
fn columnar_to_dataframe(py: Python<'_>, columnar: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let pandas = py.import("pandas").map_err(|_| {
        pyo3::exceptions::PyImportError::new_err(
            "pandas is required for DataFrame conversion. Install with: pip install pandas",
        )
    })?;
    let df = pandas.call_method1("DataFrame", (columnar,))?;
    Ok(df.unbind())
}

/// Internal helper: convert a list of tick pyclasses into a pandas DataFrame.
fn pyclass_list_to_dataframe(py: Python<'_>, ticks: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let pandas = py.import("pandas").map_err(|_| {
        pyo3::exceptions::PyImportError::new_err(
            "pandas is required for DataFrame conversion. Install with: pip install pandas",
        )
    })?;
    let bound = ticks.bind(py);
    let list = bound.downcast::<pyo3::types::PyList>().map_err(|_| {
        PyValueError::new_err("to_dataframe() expects a list of typed tick objects")
    })?;
    let columnar = pyclass_list_to_columnar(py, list)?;
    let df = pandas.call_method1("DataFrame", (columnar,))?;
    Ok(df.unbind())
}

// ── Synthetic FFI-boundary microbenches ──
//
// Isolate the "one trade event -> Python object" cost from all I/O,
// server pacing, and mpsc overhead. Each bench constructs N copies of a
// single representative trade so the only variable is the PyDict vs
// typed-pyclass path through PyO3.

fn synthetic_trade_buffered(i: i32) -> BufferedEvent {
    BufferedEvent::Trade {
        contract_id: i,
        ms_of_day: 34_200_000,
        sequence: i,
        ext_condition1: 0,
        ext_condition2: 0,
        ext_condition3: 0,
        ext_condition4: 0,
        condition: 50,
        size: 100,
        exchange: 1,
        price: 450.26,
        condition_flags: 0,
        price_flags: 0,
        volume_type: 0,
        records_back: 0,
        date: 20_260_418,
        received_at_ns: 1_700_000_000_000_000_000,
    }
}

/// Build N PyDict trade events. Returns the wall-clock ns elapsed.
///
/// Hidden (leading-underscore) Python-level function. Only used by the
/// internal bench script to compare FFI-boundary costs between paths.
#[pyfunction]
fn _bench_synthetic_dict(py: Python<'_>, n: usize) -> PyResult<u64> {
    let ev = synthetic_trade_buffered(0);
    let start = std::time::Instant::now();
    for _ in 0..n {
        let _dict = buffered_event_to_py(py, &ev);
    }
    Ok(u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX))
}

/// Build N typed-pyclass trade events. Returns the wall-clock ns elapsed.
#[pyfunction]
fn _bench_synthetic_typed(py: Python<'_>, n: usize) -> PyResult<u64> {
    let ev = synthetic_trade_buffered(0);
    let start = std::time::Instant::now();
    for _ in 0..n {
        let _obj = buffered_event_to_typed(py, &ev)?;
    }
    Ok(u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX))
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
    let list = bound.downcast::<pyo3::types::PyList>().map_err(|_| {
        PyValueError::new_err("to_polars() expects a list of typed tick objects")
    })?;
    let columnar = pyclass_list_to_columnar(py, list)?;
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
    m.add_function(wrap_pyfunction!(_bench_synthetic_dict, m)?)?;
    m.add_function(wrap_pyfunction!(_bench_synthetic_typed, m)?)?;
    register_generated_utility_functions(m)?;
    m.add_function(wrap_pyfunction!(to_dataframe, m)?)?;
    m.add_function(wrap_pyfunction!(to_polars, m)?)?;
    Ok(())
}
