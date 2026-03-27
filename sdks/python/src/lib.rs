//! Python bindings for `thetadatadx` — wraps the Rust SDK via PyO3.
//!
//! This is NOT a reimplementation. Every call goes through the Rust crate,
//! giving Python users native performance for ThetaData market data access.

use pyo3::exceptions::{PyConnectionError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use thetadatadx::auth;
use thetadatadx::config;
use thetadatadx::fpss;
use thetadatadx::types::tick;

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
        thetadatadx::Error::Auth(msg) => PyConnectionError::new_err(msg),
        thetadatadx::Error::Config(msg) => PyValueError::new_err(msg),
        _ => PyRuntimeError::new_err(e.to_string()),
    }
}

// ── Credentials ──

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

    /// Dev configuration (shorter timeouts).
    #[staticmethod]
    fn dev() -> Self {
        Self {
            inner: config::DirectConfig::dev(),
        }
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

// ── Tick types as Python dicts ──

fn trade_tick_to_dict(py: Python<'_>, t: &tick::TradeTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", t.ms_of_day).unwrap();
    dict.set_item("sequence", t.sequence).unwrap();
    dict.set_item("condition", t.condition).unwrap();
    dict.set_item("size", t.size).unwrap();
    dict.set_item("exchange", t.exchange).unwrap();
    dict.set_item("price", t.get_price().to_f64()).unwrap();
    dict.set_item("price_raw", t.price).unwrap();
    dict.set_item("price_type", t.price_type).unwrap();
    dict.set_item("condition_flags", t.condition_flags).unwrap();
    dict.set_item("price_flags", t.price_flags).unwrap();
    dict.set_item("volume_type", t.volume_type).unwrap();
    dict.set_item("records_back", t.records_back).unwrap();
    dict.set_item("date", t.date).unwrap();
    dict.into_any().unbind()
}

fn quote_tick_to_dict(py: Python<'_>, q: &tick::QuoteTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", q.ms_of_day).unwrap();
    dict.set_item("bid_size", q.bid_size).unwrap();
    dict.set_item("bid_exchange", q.bid_exchange).unwrap();
    dict.set_item("bid", q.bid_price().to_f64()).unwrap();
    dict.set_item("bid_condition", q.bid_condition).unwrap();
    dict.set_item("ask_size", q.ask_size).unwrap();
    dict.set_item("ask_exchange", q.ask_exchange).unwrap();
    dict.set_item("ask", q.ask_price().to_f64()).unwrap();
    dict.set_item("ask_condition", q.ask_condition).unwrap();
    dict.set_item("date", q.date).unwrap();
    dict.into_any().unbind()
}

fn ohlc_tick_to_dict(py: Python<'_>, o: &tick::OhlcTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", o.ms_of_day).unwrap();
    dict.set_item("open", o.open_price().to_f64()).unwrap();
    dict.set_item("high", o.high_price().to_f64()).unwrap();
    dict.set_item("low", o.low_price().to_f64()).unwrap();
    dict.set_item("close", o.close_price().to_f64()).unwrap();
    dict.set_item("volume", o.volume).unwrap();
    dict.set_item("count", o.count).unwrap();
    dict.set_item("date", o.date).unwrap();
    dict.into_any().unbind()
}

fn eod_tick_to_dict(py: Python<'_>, e: &tick::EodTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", e.ms_of_day).unwrap();
    dict.set_item("open", e.open_price().to_f64()).unwrap();
    dict.set_item("high", e.high_price().to_f64()).unwrap();
    dict.set_item("low", e.low_price().to_f64()).unwrap();
    dict.set_item("close", e.close_price().to_f64()).unwrap();
    dict.set_item("volume", e.volume).unwrap();
    dict.set_item("count", e.count).unwrap();
    dict.set_item("bid", e.bid_price().to_f64()).unwrap();
    dict.set_item("ask", e.ask_price().to_f64()).unwrap();
    dict.set_item("date", e.date).unwrap();
    dict.into_any().unbind()
}

/// Convert a `proto::DataTable` into a Python list of dicts.
///
/// Each dict has the column headers as keys and the corresponding row
/// values as values (strings, ints, or nested dicts for price/timestamp).
fn data_table_to_dicts(py: Python<'_>, table: &thetadatadx::proto::DataTable) -> Vec<Py<PyAny>> {
    use thetadatadx::proto::data_value::DataType;

    table
        .data_table
        .iter()
        .map(|row| {
            let dict = PyDict::new(py);
            for (i, val) in row.values.iter().enumerate() {
                let key = table
                    .headers
                    .get(i)
                    .map(|s| s.as_str())
                    .unwrap_or("unknown");
                match &val.data_type {
                    Some(DataType::Text(s)) => {
                        dict.set_item(key, s.as_str()).unwrap();
                    }
                    Some(DataType::Number(n)) => {
                        dict.set_item(key, *n).unwrap();
                    }
                    Some(DataType::Price(p)) => {
                        let d = PyDict::new(py);
                        d.set_item("value", p.value).unwrap();
                        d.set_item("type", p.r#type).unwrap();
                        dict.set_item(key, d).unwrap();
                    }
                    Some(DataType::Timestamp(ts)) => {
                        let d = PyDict::new(py);
                        d.set_item("epoch_ms", ts.epoch_ms).unwrap();
                        d.set_item("zone", ts.zone).unwrap();
                        dict.set_item(key, d).unwrap();
                    }
                    Some(DataType::NullValue(_)) | None => {
                        dict.set_item(key, py.None()).unwrap();
                    }
                }
            }
            dict.into_any().unbind()
        })
        .collect()
}

// ── Greeks ──

/// Compute all 22 Black-Scholes Greeks + IV in one call.
///
/// Args:
///     spot: Underlying spot price
///     strike: Option strike price
///     rate: Risk-free interest rate
///     div_yield: Continuous dividend yield
///     tte: Time to expiration in years
///     option_price: Market price of the option
///     is_call: True for call, False for put
///
/// Returns:
///     Dict with keys: value, delta, gamma, theta, vega, rho, iv, iv_error,
///     vanna, charm, vomma, veta, speed, zomma, color, ultima,
///     d1, d2, dual_delta, dual_gamma, epsilon, lambda
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn all_greeks(
    py: Python<'_>,
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    is_call: bool,
) -> Py<PyAny> {
    let g =
        thetadatadx::greeks::all_greeks(spot, strike, rate, div_yield, tte, option_price, is_call);
    let dict = PyDict::new(py);
    dict.set_item("value", g.value).unwrap();
    dict.set_item("delta", g.delta).unwrap();
    dict.set_item("gamma", g.gamma).unwrap();
    dict.set_item("theta", g.theta).unwrap();
    dict.set_item("vega", g.vega).unwrap();
    dict.set_item("rho", g.rho).unwrap();
    dict.set_item("iv", g.iv).unwrap();
    dict.set_item("iv_error", g.iv_error).unwrap();
    dict.set_item("vanna", g.vanna).unwrap();
    dict.set_item("charm", g.charm).unwrap();
    dict.set_item("vomma", g.vomma).unwrap();
    dict.set_item("veta", g.veta).unwrap();
    dict.set_item("speed", g.speed).unwrap();
    dict.set_item("zomma", g.zomma).unwrap();
    dict.set_item("color", g.color).unwrap();
    dict.set_item("ultima", g.ultima).unwrap();
    dict.set_item("d1", g.d1).unwrap();
    dict.set_item("d2", g.d2).unwrap();
    dict.set_item("dual_delta", g.dual_delta).unwrap();
    dict.set_item("dual_gamma", g.dual_gamma).unwrap();
    dict.set_item("epsilon", g.epsilon).unwrap();
    dict.set_item("lambda", g.lambda).unwrap();
    dict.into_any().unbind()
}

/// Compute implied volatility via bisection.
///
/// Returns:
///     Tuple of (iv, error)
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn implied_volatility(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    is_call: bool,
) -> (f64, f64) {
    thetadatadx::greeks::implied_volatility(
        spot,
        strike,
        rate,
        div_yield,
        tte,
        option_price,
        is_call,
    )
}

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
    },
    /// Trade tick with decoded fields.
    Trade {
        contract_id: i32,
        ms_of_day: i32,
        sequence: i32,
        condition: i32,
        size: i32,
        exchange: i32,
        price: f64,
        price_raw: i32,
        price_type: i32,
        condition_flags: i32,
        price_flags: i32,
        volume_type: i32,
        records_back: i32,
        date: i32,
    },
    /// Open interest tick.
    OpenInterest {
        contract_id: i32,
        ms_of_day: i32,
        open_interest: i32,
        date: i32,
    },
    /// OHLCVC bar with decoded fields.
    Ohlcvc {
        contract_id: i32,
        ms_of_day: i32,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: i32,
        count: i32,
        date: i32,
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

/// Convert raw integer price to f64 using ThetaData's price_type encoding.
fn price_to_f64(value: i32, price_type: i32) -> f64 {
    thetadatadx::types::price::Price::new(value, price_type).to_f64()
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
                price_type,
                date,
            } => BufferedEvent::Quote {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                bid_size: *bid_size,
                bid_exchange: *bid_exchange,
                bid: price_to_f64(*bid, *price_type),
                bid_condition: *bid_condition,
                ask_size: *ask_size,
                ask_exchange: *ask_exchange,
                ask: price_to_f64(*ask, *price_type),
                ask_condition: *ask_condition,
                date: *date,
            },
            fpss::FpssData::Trade {
                contract_id,
                ms_of_day,
                sequence,
                condition,
                size,
                exchange,
                price,
                condition_flags,
                price_flags,
                volume_type,
                records_back,
                price_type,
                date,
                ..
            } => BufferedEvent::Trade {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                sequence: *sequence,
                condition: *condition,
                size: *size,
                exchange: *exchange,
                price: price_to_f64(*price, *price_type),
                price_raw: *price,
                price_type: *price_type,
                condition_flags: *condition_flags,
                price_flags: *price_flags,
                volume_type: *volume_type,
                records_back: *records_back,
                date: *date,
            },
            fpss::FpssData::OpenInterest {
                contract_id,
                ms_of_day,
                open_interest,
                date,
            } => BufferedEvent::OpenInterest {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                open_interest: *open_interest,
                date: *date,
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
                price_type,
                date,
            } => BufferedEvent::Ohlcvc {
                contract_id: *contract_id,
                ms_of_day: *ms_of_day,
                open: price_to_f64(*open, *price_type),
                high: price_to_f64(*high, *price_type),
                low: price_to_f64(*low, *price_type),
                close: price_to_f64(*close, *price_type),
                volume: *volume,
                count: *count,
                date: *date,
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
        }
        BufferedEvent::Trade {
            contract_id,
            ms_of_day,
            sequence,
            condition,
            size,
            exchange,
            price,
            price_raw,
            price_type,
            condition_flags,
            price_flags,
            volume_type,
            records_back,
            date,
        } => {
            dict.set_item("kind", "trade").unwrap();
            dict.set_item("contract_id", contract_id).unwrap();
            dict.set_item("ms_of_day", ms_of_day).unwrap();
            dict.set_item("sequence", sequence).unwrap();
            dict.set_item("condition", condition).unwrap();
            dict.set_item("size", size).unwrap();
            dict.set_item("exchange", exchange).unwrap();
            dict.set_item("price", price).unwrap();
            dict.set_item("price_raw", price_raw).unwrap();
            dict.set_item("price_type", price_type).unwrap();
            dict.set_item("condition_flags", condition_flags).unwrap();
            dict.set_item("price_flags", price_flags).unwrap();
            dict.set_item("volume_type", volume_type).unwrap();
            dict.set_item("records_back", records_back).unwrap();
            dict.set_item("date", date).unwrap();
        }
        BufferedEvent::OpenInterest {
            contract_id,
            ms_of_day,
            open_interest,
            date,
        } => {
            dict.set_item("kind", "open_interest").unwrap();
            dict.set_item("contract_id", contract_id).unwrap();
            dict.set_item("ms_of_day", ms_of_day).unwrap();
            dict.set_item("open_interest", open_interest).unwrap();
            dict.set_item("date", date).unwrap();
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

    /// Start FPSS streaming. Events are buffered; poll with ``next_event()``.
    fn start_streaming(&self) -> PyResult<()> {
        let (tx, rx) = std::sync::mpsc::channel::<BufferedEvent>();

        self.tdx
            .start_streaming(move |event: &fpss::FpssEvent| {
                let buffered = fpss_event_to_buffered(event);
                let _ = tx.send(buffered);
            })
            .map_err(to_py_err)?;

        if let Ok(mut guard) = self.rx.lock() {
            *guard = Some(Arc::new(Mutex::new(rx)));
        }
        Ok(())
    }

    /// Start FPSS streaming with OHLCVC derivation disabled.
    fn start_streaming_no_ohlcvc(&self) -> PyResult<()> {
        let (tx, rx) = std::sync::mpsc::channel::<BufferedEvent>();

        self.tdx
            .start_streaming_no_ohlcvc(move |event: &fpss::FpssEvent| {
                let buffered = fpss_event_to_buffered(event);
                let _ = tx.send(buffered);
            })
            .map_err(to_py_err)?;

        if let Ok(mut guard) = self.rx.lock() {
            *guard = Some(Arc::new(Mutex::new(rx)));
        }
        Ok(())
    }

    /// Whether the streaming connection is active.
    fn is_streaming(&self) -> bool {
        self.tdx.is_streaming()
    }

    // ── Streaming methods ──

    /// Subscribe to quote data for a stock symbol.
    fn subscribe_quotes(&self, symbol: &str) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.subscribe_quotes(&contract).map_err(to_py_err)
    }

    /// Subscribe to trade data for a stock symbol.
    fn subscribe_trades(&self, symbol: &str) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.subscribe_trades(&contract).map_err(to_py_err)
    }

    /// Subscribe to open interest data for a stock symbol.
    fn subscribe_open_interest(&self, symbol: &str) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx
            .subscribe_open_interest(&contract)
            .map_err(to_py_err)
    }

    /// Subscribe to quote data for an option contract.
    fn subscribe_option_quotes(
        &self,
        symbol: &str,
        exp_date: i32,
        is_call: bool,
        strike: i32,
    ) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, is_call, strike);
        self.tdx.subscribe_quotes(&contract).map_err(to_py_err)
    }

    /// Subscribe to trade data for an option contract.
    fn subscribe_option_trades(
        &self,
        symbol: &str,
        exp_date: i32,
        is_call: bool,
        strike: i32,
    ) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, is_call, strike);
        self.tdx.subscribe_trades(&contract).map_err(to_py_err)
    }

    /// Subscribe to open interest data for an option contract.
    fn subscribe_option_open_interest(
        &self,
        symbol: &str,
        exp_date: i32,
        is_call: bool,
        strike: i32,
    ) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, is_call, strike);
        self.tdx
            .subscribe_open_interest(&contract)
            .map_err(to_py_err)
    }

    /// Subscribe to all trades for a security type (full trade stream).
    fn subscribe_full_trades(&self, sec_type: &str) -> PyResult<i32> {
        let st = match sec_type.to_uppercase().as_str() {
            "STOCK" => thetadatadx::types::enums::SecType::Stock,
            "OPTION" => thetadatadx::types::enums::SecType::Option,
            "INDEX" => thetadatadx::types::enums::SecType::Index,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown sec_type: {other:?} (expected STOCK, OPTION, or INDEX)"
                )))
            }
        };
        self.tdx.subscribe_full_trades(st).map_err(to_py_err)
    }

    /// Unsubscribe from quote data for a stock symbol.
    fn unsubscribe_quotes(&self, symbol: &str) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.unsubscribe_quotes(&contract).map_err(to_py_err)
    }

    /// Unsubscribe from trade data for a stock symbol.
    fn unsubscribe_trades(&self, symbol: &str) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.unsubscribe_trades(&contract).map_err(to_py_err)
    }

    /// Unsubscribe from open interest data for a stock symbol.
    fn unsubscribe_open_interest(&self, symbol: &str) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx
            .unsubscribe_open_interest(&contract)
            .map_err(to_py_err)
    }

    /// Unsubscribe from quote data for an option contract.
    fn unsubscribe_option_quotes(
        &self,
        symbol: &str,
        exp_date: i32,
        is_call: bool,
        strike: i32,
    ) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, is_call, strike);
        self.tdx.unsubscribe_quotes(&contract).map_err(to_py_err)
    }

    /// Unsubscribe from trade data for an option contract.
    fn unsubscribe_option_trades(
        &self,
        symbol: &str,
        exp_date: i32,
        is_call: bool,
        strike: i32,
    ) -> PyResult<i32> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, is_call, strike);
        self.tdx.unsubscribe_trades(&contract).map_err(to_py_err)
    }

    /// Get the current contract map (server-assigned IDs -> contract strings).
    fn contract_map(&self) -> PyResult<std::collections::HashMap<i32, String>> {
        self.tdx
            .contract_map()
            .map(|m| m.into_iter().map(|(id, c)| (id, format!("{c}"))).collect())
            .map_err(to_py_err)
    }

    /// Look up a single contract by its server-assigned ID.
    fn contract_lookup(&self, id: i32) -> PyResult<Option<String>> {
        self.tdx
            .contract_lookup(id)
            .map(|opt| opt.map(|c| format!("{c}")))
            .map_err(to_py_err)
    }

    /// Get a snapshot of currently active subscriptions.
    fn active_subscriptions(&self) -> PyResult<Vec<std::collections::HashMap<String, String>>> {
        self.tdx
            .active_subscriptions()
            .map(|subs| {
                subs.into_iter()
                    .map(|(kind, contract)| {
                        let mut m = std::collections::HashMap::new();
                        m.insert("kind".to_string(), format!("{kind:?}"));
                        m.insert("contract".to_string(), format!("{contract}"));
                        m
                    })
                    .collect()
            })
            .map_err(to_py_err)
    }

    /// Poll for the next FPSS event.
    ///
    /// Args:
    ///     timeout_ms: Maximum time to wait in milliseconds.
    ///
    /// Returns:
    ///     A dict with ``kind`` key indicating event type, or ``None`` if timeout.
    ///     Raises ``RuntimeError`` if streaming has not been started.
    fn next_event(&self, py: Python<'_>, timeout_ms: u64) -> PyResult<Option<Py<PyAny>>> {
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
        let result = py.detach(move || {
            let rx = rx_arc.lock().unwrap();
            rx.recv_timeout(timeout).ok()
        });
        match result {
            Some(event) => Ok(Some(buffered_event_to_py(py, &event))),
            None => Ok(None),
        }
    }

    /// Stop streaming (historical remains active).
    fn stop_streaming(&self) {
        self.tdx.stop_streaming();
        if let Ok(mut guard) = self.rx.lock() {
            *guard = None;
        }
    }

    /// Shut down everything (streaming + drop historical).
    fn shutdown(&self) {
        self.tdx.stop_streaming();
    }

    // ── Historical methods (delegate through Deref to DirectClient) ──

    // Stock — List (2)

    fn stock_list_symbols(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_list_symbols())
                .map_err(to_py_err)
        })
    }
    fn stock_list_dates(
        &self,
        py: Python<'_>,
        request_type: &str,
        symbol: &str,
    ) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_list_dates(request_type, symbol))
                .map_err(to_py_err)
        })
    }

    // Stock — Snapshot (4)

    fn stock_snapshot_ohlc(
        &self,
        py: Python<'_>,
        symbols: Vec<String>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_snapshot_ohlc(&refs))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn stock_snapshot_trade(
        &self,
        py: Python<'_>,
        symbols: Vec<String>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_snapshot_trade(&refs))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn stock_snapshot_quote(
        &self,
        py: Python<'_>,
        symbols: Vec<String>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_snapshot_quote(&refs))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }
    fn stock_snapshot_market_value(
        &self,
        py: Python<'_>,
        symbols: Vec<String>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_snapshot_market_value(&refs))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // Stock — History (5 + bonus)

    fn stock_history_eod(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_history_eod(symbol, start_date, end_date))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| eod_tick_to_dict(py, t)).collect())
    }
    fn stock_history_ohlc(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_history_ohlc(symbol, date, interval))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn stock_history_ohlc_range(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .stock_history_ohlc_range(symbol, start_date, end_date, interval),
                )
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn stock_history_trade(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_history_trade(symbol, date))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn stock_history_quote(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_history_quote(symbol, date, interval))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }
    fn stock_history_trade_quote(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.stock_history_trade_quote(symbol, date))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // Stock — At-Time (2)

    fn stock_at_time_trade(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
        time_of_day: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .stock_at_time_trade(symbol, start_date, end_date, time_of_day),
                )
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn stock_at_time_quote(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
        time_of_day: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .stock_at_time_quote(symbol, start_date, end_date, time_of_day),
                )
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }

    // Option — List (5)

    fn option_list_symbols(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(self.tdx.option_list_symbols())
                .map_err(to_py_err)
        })
    }
    fn option_list_dates(
        &self,
        py: Python<'_>,
        request_type: &str,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_list_dates(request_type, symbol, expiration, strike, right),
                )
                .map_err(to_py_err)
        })
    }
    fn option_list_expirations(&self, py: Python<'_>, symbol: &str) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(self.tdx.option_list_expirations(symbol))
                .map_err(to_py_err)
        })
    }
    fn option_list_strikes(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
    ) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(self.tdx.option_list_strikes(symbol, expiration))
                .map_err(to_py_err)
        })
    }
    fn option_list_contracts(
        &self,
        py: Python<'_>,
        request_type: &str,
        symbol: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_list_contracts(request_type, symbol, date))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // Option — Snapshot (10)

    fn option_snapshot_ohlc(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_snapshot_ohlc(symbol, expiration, strike, right),
                )
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn option_snapshot_trade(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_snapshot_trade(symbol, expiration, strike, right),
                )
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn option_snapshot_quote(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_snapshot_quote(symbol, expiration, strike, right),
                )
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }
    fn option_snapshot_open_interest(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_snapshot_open_interest(symbol, expiration, strike, right),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_market_value(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_snapshot_market_value(symbol, expiration, strike, right),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_greeks_implied_volatility(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_snapshot_greeks_implied_volatility(
                    symbol, expiration, strike, right,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_greeks_all(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_snapshot_greeks_all(symbol, expiration, strike, right),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_greeks_first_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_snapshot_greeks_first_order(symbol, expiration, strike, right),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_greeks_second_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_snapshot_greeks_second_order(symbol, expiration, strike, right),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_greeks_third_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_snapshot_greeks_third_order(symbol, expiration, strike, right),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // Option — History (6)

    fn option_history_eod(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        start_date: &str,
        end_date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_eod(
                    symbol, expiration, strike, right, start_date, end_date,
                ))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| eod_tick_to_dict(py, t)).collect())
    }
    fn option_history_ohlc(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_history_ohlc(symbol, expiration, strike, right, date, interval),
                )
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn option_history_trade(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_history_trade(symbol, expiration, strike, right, date),
                )
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn option_history_quote(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_history_quote(symbol, expiration, strike, right, date, interval),
                )
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }
    fn option_history_trade_quote(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_history_trade_quote(symbol, expiration, strike, right, date),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_open_interest(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_history_open_interest(symbol, expiration, strike, right, date),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // Option — History Greeks (11)

    fn option_history_greeks_eod(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        start_date: &str,
        end_date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_greeks_eod(
                    symbol, expiration, strike, right, start_date, end_date,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_greeks_all(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_greeks_all(
                    symbol, expiration, strike, right, date, interval,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_trade_greeks_all(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .option_history_trade_greeks_all(symbol, expiration, strike, right, date),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_greeks_first_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_greeks_first_order(
                    symbol, expiration, strike, right, date, interval,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_trade_greeks_first_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_trade_greeks_first_order(
                    symbol, expiration, strike, right, date,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_greeks_second_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_greeks_second_order(
                    symbol, expiration, strike, right, date, interval,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_trade_greeks_second_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_trade_greeks_second_order(
                    symbol, expiration, strike, right, date,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_greeks_third_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_greeks_third_order(
                    symbol, expiration, strike, right, date, interval,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_trade_greeks_third_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_trade_greeks_third_order(
                    symbol, expiration, strike, right, date,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_greeks_implied_volatility(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_greeks_implied_volatility(
                    symbol, expiration, strike, right, date, interval,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_trade_greeks_implied_volatility(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_history_trade_greeks_implied_volatility(
                    symbol, expiration, strike, right, date,
                ))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // Option — At-Time (2)

    #[allow(clippy::too_many_arguments)]
    fn option_at_time_trade(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        start_date: &str,
        end_date: &str,
        time_of_day: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_at_time_trade(
                    symbol,
                    expiration,
                    strike,
                    right,
                    start_date,
                    end_date,
                    time_of_day,
                ))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    #[allow(clippy::too_many_arguments)]
    fn option_at_time_quote(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
        start_date: &str,
        end_date: &str,
        time_of_day: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.option_at_time_quote(
                    symbol,
                    expiration,
                    strike,
                    right,
                    start_date,
                    end_date,
                    time_of_day,
                ))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }

    // Index — List (2)

    fn index_list_symbols(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(self.tdx.index_list_symbols())
                .map_err(to_py_err)
        })
    }
    fn index_list_dates(&self, py: Python<'_>, symbol: &str) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(self.tdx.index_list_dates(symbol))
                .map_err(to_py_err)
        })
    }

    // Index — Snapshot (3)

    fn index_snapshot_ohlc(
        &self,
        py: Python<'_>,
        symbols: Vec<String>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.index_snapshot_ohlc(&refs))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn index_snapshot_price(
        &self,
        py: Python<'_>,
        symbols: Vec<String>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.index_snapshot_price(&refs))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn index_snapshot_market_value(
        &self,
        py: Python<'_>,
        symbols: Vec<String>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.index_snapshot_market_value(&refs))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // Index — History (3)

    fn index_history_eod(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(self.tdx.index_history_eod(symbol, start_date, end_date))
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| eod_tick_to_dict(py, t)).collect())
    }
    fn index_history_ohlc(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .index_history_ohlc(symbol, start_date, end_date, interval),
                )
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn index_history_price(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.index_history_price(symbol, date, interval))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // Index — At-Time (1)

    fn index_at_time_price(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
        time_of_day: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .index_at_time_price(symbol, start_date, end_date, time_of_day),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // Calendar (3)

    fn calendar_open_today(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.calendar_open_today())
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn calendar_on_date(&self, py: Python<'_>, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.calendar_on_date(date))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn calendar_year(&self, py: Python<'_>, year: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(self.tdx.calendar_year(year))
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // Interest Rate (1)

    fn interest_rate_history_eod(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| {
            runtime()
                .block_on(
                    self.tdx
                        .interest_rate_history_eod(symbol, start_date, end_date),
                )
                .map_err(to_py_err)
        })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── DataFrame convenience wrappers ──
    //
    // These call the underlying method and wrap the result in a pandas DataFrame.
    // Requires pandas to be installed (`pip install pandas`).

    /// Fetch stock EOD history and return a pandas DataFrame.
    fn stock_history_eod_df(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
    ) -> PyResult<Py<PyAny>> {
        let ticks = self.stock_history_eod(py, symbol, start_date, end_date)?;
        dicts_to_dataframe(py, ticks)
    }

    /// Fetch stock OHLC history and return a pandas DataFrame.
    fn stock_history_ohlc_df(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Py<PyAny>> {
        let ticks = self.stock_history_ohlc(py, symbol, date, interval)?;
        dicts_to_dataframe(py, ticks)
    }

    /// Fetch stock trade history and return a pandas DataFrame.
    fn stock_history_trade_df(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
    ) -> PyResult<Py<PyAny>> {
        let ticks = self.stock_history_trade(py, symbol, date)?;
        dicts_to_dataframe(py, ticks)
    }

    /// Fetch stock quote history and return a pandas DataFrame.
    fn stock_history_quote_df(
        &self,
        py: Python<'_>,
        symbol: &str,
        date: &str,
        interval: &str,
    ) -> PyResult<Py<PyAny>> {
        let ticks = self.stock_history_quote(py, symbol, date, interval)?;
        dicts_to_dataframe(py, ticks)
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

// ── pandas DataFrame helpers ──

/// Internal helper: convert a Vec of Python dicts into a pandas DataFrame.
fn dicts_to_dataframe(py: Python<'_>, dicts: Vec<Py<PyAny>>) -> PyResult<Py<PyAny>> {
    let pandas = py.import("pandas").map_err(|_| {
        PyRuntimeError::new_err(
            "pandas is required for DataFrame conversion. Install with: pip install pandas",
        )
    })?;
    let df = pandas.call_method1("DataFrame", (dicts,))?;
    Ok(df.unbind())
}

/// Convert a list of tick dicts to a pandas DataFrame.
///
/// Requires pandas to be installed (``pip install pandas``).
///
/// Example::
///
///     ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
///     df = thetadatadx.to_dataframe(ticks)
#[pyfunction]
fn to_dataframe(py: Python<'_>, ticks: Vec<Py<PyAny>>) -> PyResult<Py<PyAny>> {
    dicts_to_dataframe(py, ticks)
}

/// Convert a list of tick dicts to a polars DataFrame.
///
/// Requires polars: `pip install thetadatadx[polars]`
///
/// Example:
///
///     ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
///     df = thetadatadx.to_polars(ticks)
#[pyfunction]
fn to_polars(py: Python<'_>, ticks: Vec<Py<PyAny>>) -> PyResult<Py<PyAny>> {
    let polars = py.import("polars").map_err(|_| {
        PyRuntimeError::new_err(
            "polars is not installed. Install it with: pip install thetadatadx[polars]",
        )
    })?;
    let df = polars.call_method1("from_dicts", (ticks,))?;
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
    m.add_function(wrap_pyfunction!(all_greeks, m)?)?;
    m.add_function(wrap_pyfunction!(implied_volatility, m)?)?;
    m.add_function(wrap_pyfunction!(to_dataframe, m)?)?;
    m.add_function(wrap_pyfunction!(to_polars, m)?)?;
    Ok(())
}
