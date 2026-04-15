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

// ── Tick columnar converters (generated from tick_schema.toml) ──

include!("tick_columnar.rs");

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
        let columnar = self.stock_history_eod(py, symbol, start_date, end_date, None)?;
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
        let columnar = self.stock_history_ohlc(
            py, symbol, date, interval, None, None, None, None, None, None,
        )?;
        columnar_to_dataframe(py, columnar)
    }

    /// Fetch stock trade history and return a pandas DataFrame.
    fn stock_history_trade_df(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
    ) -> PyResult<Py<PyAny>> {
        let columnar =
            self.stock_history_trade(py, symbol, date, None, None, None, None, None, None)?;
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
        let columnar = self.stock_history_quote(
            py, symbol, date, interval, None, None, None, None, None, None,
        )?;
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
}

include!("streaming_methods.rs");

include!("historical_methods.rs");

// ── pandas DataFrame helpers ──

/// Internal helper: convert a columnar dict (dict-of-lists) into a pandas DataFrame.
///
/// `pd.DataFrame(dict_of_lists)` is the fastest DataFrame constructor --
/// it accepts a dict where each key maps to a list of values directly.
fn columnar_to_dataframe(py: Python<'_>, columnar: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let pandas = py.import("pandas").map_err(|_| {
        PyRuntimeError::new_err(
            "pandas is required for DataFrame conversion. Install with: pip install pandas",
        )
    })?;
    let df = pandas.call_method1("DataFrame", (columnar,))?;
    Ok(df.unbind())
}

/// Convert a columnar dict (dict-of-lists) to a pandas DataFrame.
///
/// Requires pandas to be installed (``pip install pandas``).
///
/// Historical endpoints return columnar dicts (one list per field).
/// This is the fastest input format for ``pd.DataFrame()``.
///
/// Example::
///
///     ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
///     df = thetadatadx.to_dataframe(ticks)
#[pyfunction]
fn to_dataframe(py: Python<'_>, ticks: Py<PyAny>) -> PyResult<Py<PyAny>> {
    columnar_to_dataframe(py, ticks)
}

/// Convert a columnar dict (dict-of-lists) to a polars DataFrame.
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
        PyRuntimeError::new_err(
            "polars is not installed. Install it with: pip install thetadatadx[polars]",
        )
    })?;
    let df = polars.call_method1("DataFrame", (ticks,))?;
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
    register_generated_utility_functions(m)?;
    m.add_function(wrap_pyfunction!(to_dataframe, m)?)?;
    m.add_function(wrap_pyfunction!(to_polars, m)?)?;
    Ok(())
}
