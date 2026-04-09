//! Python bindings for `thetadatadx` — wraps the Rust SDK via PyO3.
//!
//! This is NOT a reimplementation. Every call goes through the Rust crate,
//! giving Python users native performance for ThetaData market data access.

use pyo3::exceptions::{PyConnectionError, PyRuntimeError, PyValueError};
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

// ── Tick types as Python dicts ──

macro_rules! set_contract_id {
    ($dict:expr, $tick:expr) => {
        if $tick.expiration != 0 {
            $dict.set_item("expiration", $tick.expiration).unwrap();
            $dict.set_item("strike", $tick.strike).unwrap();
            $dict
                .set_item(
                    "right",
                    if $tick.is_call() {
                        "C"
                    } else if $tick.is_put() {
                        "P"
                    } else {
                        ""
                    },
                )
                .unwrap();
        }
    };
}

fn trade_tick_to_dict(py: Python<'_>, t: &tick::TradeTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", t.ms_of_day).unwrap();
    dict.set_item("sequence", t.sequence).unwrap();
    dict.set_item("ext_condition1", t.ext_condition1).unwrap();
    dict.set_item("ext_condition2", t.ext_condition2).unwrap();
    dict.set_item("ext_condition3", t.ext_condition3).unwrap();
    dict.set_item("ext_condition4", t.ext_condition4).unwrap();
    dict.set_item("condition", t.condition).unwrap();
    dict.set_item("size", t.size).unwrap();
    dict.set_item("exchange", t.exchange).unwrap();
    dict.set_item("price", t.price).unwrap();
    dict.set_item("condition_flags", t.condition_flags).unwrap();
    dict.set_item("price_flags", t.price_flags).unwrap();
    dict.set_item("volume_type", t.volume_type).unwrap();
    dict.set_item("records_back", t.records_back).unwrap();
    dict.set_item("date", t.date).unwrap();
    set_contract_id!(dict, t);
    dict.into_any().unbind()
}

fn quote_tick_to_dict(py: Python<'_>, q: &tick::QuoteTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", q.ms_of_day).unwrap();
    dict.set_item("bid_size", q.bid_size).unwrap();
    dict.set_item("bid_exchange", q.bid_exchange).unwrap();
    dict.set_item("bid", q.bid).unwrap();
    dict.set_item("bid_condition", q.bid_condition).unwrap();
    dict.set_item("ask_size", q.ask_size).unwrap();
    dict.set_item("ask_exchange", q.ask_exchange).unwrap();
    dict.set_item("ask", q.ask).unwrap();
    dict.set_item("ask_condition", q.ask_condition).unwrap();
    dict.set_item("midpoint", q.midpoint).unwrap();
    dict.set_item("date", q.date).unwrap();
    set_contract_id!(dict, q);
    dict.into_any().unbind()
}

fn ohlc_tick_to_dict(py: Python<'_>, o: &tick::OhlcTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", o.ms_of_day).unwrap();
    dict.set_item("open", o.open).unwrap();
    dict.set_item("high", o.high).unwrap();
    dict.set_item("low", o.low).unwrap();
    dict.set_item("close", o.close).unwrap();
    dict.set_item("volume", o.volume).unwrap();
    dict.set_item("count", o.count).unwrap();
    dict.set_item("date", o.date).unwrap();
    set_contract_id!(dict, o);
    dict.into_any().unbind()
}

fn eod_tick_to_dict(py: Python<'_>, e: &tick::EodTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", e.ms_of_day).unwrap();
    dict.set_item("ms_of_day2", e.ms_of_day2).unwrap();
    dict.set_item("open", e.open).unwrap();
    dict.set_item("high", e.high).unwrap();
    dict.set_item("low", e.low).unwrap();
    dict.set_item("close", e.close).unwrap();
    dict.set_item("volume", e.volume).unwrap();
    dict.set_item("count", e.count).unwrap();
    dict.set_item("bid_size", e.bid_size).unwrap();
    dict.set_item("bid_exchange", e.bid_exchange).unwrap();
    dict.set_item("bid", e.bid).unwrap();
    dict.set_item("bid_condition", e.bid_condition).unwrap();
    dict.set_item("ask_size", e.ask_size).unwrap();
    dict.set_item("ask_exchange", e.ask_exchange).unwrap();
    dict.set_item("ask", e.ask).unwrap();
    dict.set_item("ask_condition", e.ask_condition).unwrap();
    dict.set_item("date", e.date).unwrap();
    set_contract_id!(dict, e);
    dict.into_any().unbind()
}

fn trade_quote_tick_to_dict(py: Python<'_>, t: &tick::TradeQuoteTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", t.ms_of_day).unwrap();
    dict.set_item("sequence", t.sequence).unwrap();
    dict.set_item("ext_condition1", t.ext_condition1).unwrap();
    dict.set_item("ext_condition2", t.ext_condition2).unwrap();
    dict.set_item("ext_condition3", t.ext_condition3).unwrap();
    dict.set_item("ext_condition4", t.ext_condition4).unwrap();
    dict.set_item("condition", t.condition).unwrap();
    dict.set_item("size", t.size).unwrap();
    dict.set_item("exchange", t.exchange).unwrap();
    dict.set_item("price", t.price).unwrap();
    dict.set_item("condition_flags", t.condition_flags).unwrap();
    dict.set_item("price_flags", t.price_flags).unwrap();
    dict.set_item("volume_type", t.volume_type).unwrap();
    dict.set_item("records_back", t.records_back).unwrap();
    dict.set_item("quote_ms_of_day", t.quote_ms_of_day).unwrap();
    dict.set_item("bid_size", t.bid_size).unwrap();
    dict.set_item("bid_exchange", t.bid_exchange).unwrap();
    dict.set_item("bid", t.bid).unwrap();
    dict.set_item("bid_condition", t.bid_condition).unwrap();
    dict.set_item("ask_size", t.ask_size).unwrap();
    dict.set_item("ask_exchange", t.ask_exchange).unwrap();
    dict.set_item("ask", t.ask).unwrap();
    dict.set_item("ask_condition", t.ask_condition).unwrap();
    dict.set_item("date", t.date).unwrap();
    set_contract_id!(dict, t);
    dict.into_any().unbind()
}

fn open_interest_tick_to_dict(py: Python<'_>, t: &tick::OpenInterestTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", t.ms_of_day).unwrap();
    dict.set_item("open_interest", t.open_interest).unwrap();
    dict.set_item("date", t.date).unwrap();
    set_contract_id!(dict, t);
    dict.into_any().unbind()
}

fn market_value_tick_to_dict(py: Python<'_>, t: &tick::MarketValueTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", t.ms_of_day).unwrap();
    dict.set_item("market_cap", t.market_cap).unwrap();
    dict.set_item("shares_outstanding", t.shares_outstanding)
        .unwrap();
    dict.set_item("enterprise_value", t.enterprise_value)
        .unwrap();
    dict.set_item("book_value", t.book_value).unwrap();
    dict.set_item("free_float", t.free_float).unwrap();
    dict.set_item("date", t.date).unwrap();
    set_contract_id!(dict, t);
    dict.into_any().unbind()
}

fn greeks_tick_to_dict(py: Python<'_>, t: &tick::GreeksTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", t.ms_of_day).unwrap();
    dict.set_item("implied_volatility", t.implied_volatility)
        .unwrap();
    dict.set_item("delta", t.delta).unwrap();
    dict.set_item("gamma", t.gamma).unwrap();
    dict.set_item("theta", t.theta).unwrap();
    dict.set_item("vega", t.vega).unwrap();
    dict.set_item("rho", t.rho).unwrap();
    dict.set_item("iv_error", t.iv_error).unwrap();
    dict.set_item("vanna", t.vanna).unwrap();
    dict.set_item("charm", t.charm).unwrap();
    dict.set_item("vomma", t.vomma).unwrap();
    dict.set_item("veta", t.veta).unwrap();
    dict.set_item("speed", t.speed).unwrap();
    dict.set_item("zomma", t.zomma).unwrap();
    dict.set_item("color", t.color).unwrap();
    dict.set_item("ultima", t.ultima).unwrap();
    dict.set_item("d1", t.d1).unwrap();
    dict.set_item("d2", t.d2).unwrap();
    dict.set_item("dual_delta", t.dual_delta).unwrap();
    dict.set_item("dual_gamma", t.dual_gamma).unwrap();
    dict.set_item("epsilon", t.epsilon).unwrap();
    dict.set_item("lambda", t.lambda).unwrap();
    dict.set_item("vera", t.vera).unwrap();
    dict.set_item("date", t.date).unwrap();
    set_contract_id!(dict, t);
    dict.into_any().unbind()
}

fn iv_tick_to_dict(py: Python<'_>, t: &tick::IvTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", t.ms_of_day).unwrap();
    dict.set_item("implied_volatility", t.implied_volatility)
        .unwrap();
    dict.set_item("iv_error", t.iv_error).unwrap();
    dict.set_item("date", t.date).unwrap();
    set_contract_id!(dict, t);
    dict.into_any().unbind()
}

fn price_tick_to_dict(py: Python<'_>, t: &tick::PriceTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", t.ms_of_day).unwrap();
    dict.set_item("price", t.price).unwrap();
    dict.set_item("date", t.date).unwrap();
    dict.into_any().unbind()
}

fn calendar_day_to_dict(py: Python<'_>, d: &tick::CalendarDay) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("date", d.date).unwrap();
    dict.set_item("is_open", d.is_open).unwrap();
    dict.set_item("open_time", d.open_time).unwrap();
    dict.set_item("close_time", d.close_time).unwrap();
    dict.set_item("status", d.status).unwrap();
    dict.into_any().unbind()
}

fn interest_rate_tick_to_dict(py: Python<'_>, t: &tick::InterestRateTick) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("ms_of_day", t.ms_of_day).unwrap();
    dict.set_item("rate", t.rate).unwrap();
    dict.set_item("date", t.date).unwrap();
    dict.into_any().unbind()
}

fn option_contract_to_dict(py: Python<'_>, c: &tick::OptionContract) -> Py<PyAny> {
    let dict = PyDict::new(py);
    dict.set_item("root", &c.root).unwrap();
    dict.set_item("expiration", c.expiration).unwrap();
    dict.set_item("strike", c.strike).unwrap();
    dict.set_item("right", c.right).unwrap();
    dict.into_any().unbind()
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
#[allow(clippy::too_many_arguments)] // Reason: mirrors Black-Scholes parameter set expected by Python callers
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
    let g = tdbe::greeks::all_greeks(spot, strike, rate, div_yield, tte, option_price, is_call);
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
#[allow(clippy::too_many_arguments)] // Reason: mirrors Black-Scholes parameter set expected by Python callers
fn implied_volatility(
    spot: f64,
    strike: f64,
    rate: f64,
    div_yield: f64,
    tte: f64,
    option_price: f64,
    is_call: bool,
) -> (f64, f64) {
    tdbe::greeks::implied_volatility(spot, strike, rate, div_yield, tte, option_price, is_call)
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

    /// Whether the streaming connection is active.
    fn is_streaming(&self) -> bool {
        self.tdx.is_streaming()
    }

    // ── Streaming methods ──

    /// Subscribe to quote data for a stock symbol.
    fn subscribe_quotes(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.subscribe_quotes(&contract).map_err(to_py_err)
    }

    /// Subscribe to trade data for a stock symbol.
    fn subscribe_trades(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.subscribe_trades(&contract).map_err(to_py_err)
    }

    /// Subscribe to open interest data for a stock symbol.
    fn subscribe_open_interest(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx
            .subscribe_open_interest(&contract)
            .map_err(to_py_err)
    }

    /// Subscribe to quote data for an option contract.
    fn subscribe_option_quotes(
        &self,
        symbol: &str,
        exp_date: &str,
        right: &str,
        strike: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, strike, right);
        self.tdx.subscribe_quotes(&contract).map_err(to_py_err)
    }

    /// Subscribe to trade data for an option contract.
    fn subscribe_option_trades(
        &self,
        symbol: &str,
        exp_date: &str,
        right: &str,
        strike: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, strike, right);
        self.tdx.subscribe_trades(&contract).map_err(to_py_err)
    }

    /// Subscribe to open interest data for an option contract.
    fn subscribe_option_open_interest(
        &self,
        symbol: &str,
        exp_date: &str,
        right: &str,
        strike: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, strike, right);
        self.tdx
            .subscribe_open_interest(&contract)
            .map_err(to_py_err)
    }

    /// Subscribe to all trades for a security type (full trade stream).
    fn subscribe_full_trades(&self, sec_type: &str) -> PyResult<()> {
        let st = parse_sec_type(sec_type)?;
        self.tdx.subscribe_full_trades(st).map_err(to_py_err)
    }

    /// Subscribe to all open interest for a security type (full OI stream).
    fn subscribe_full_open_interest(&self, sec_type: &str) -> PyResult<()> {
        let st = parse_sec_type(sec_type)?;
        self.tdx.subscribe_full_open_interest(st).map_err(to_py_err)
    }

    /// Unsubscribe from all trades for a security type (full trade stream).
    fn unsubscribe_full_trades(&self, sec_type: &str) -> PyResult<()> {
        let st = parse_sec_type(sec_type)?;
        self.tdx.unsubscribe_full_trades(st).map_err(to_py_err)
    }

    /// Unsubscribe from all open interest for a security type (full OI stream).
    fn unsubscribe_full_open_interest(&self, sec_type: &str) -> PyResult<()> {
        let st = parse_sec_type(sec_type)?;
        self.tdx.unsubscribe_full_open_interest(st).map_err(to_py_err)
    }

    /// Unsubscribe from quote data for a stock symbol.
    fn unsubscribe_quotes(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.unsubscribe_quotes(&contract).map_err(to_py_err)
    }

    /// Unsubscribe from trade data for a stock symbol.
    fn unsubscribe_trades(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx.unsubscribe_trades(&contract).map_err(to_py_err)
    }

    /// Unsubscribe from open interest data for a stock symbol.
    fn unsubscribe_open_interest(&self, symbol: &str) -> PyResult<()> {
        let contract = fpss::protocol::Contract::stock(symbol);
        self.tdx
            .unsubscribe_open_interest(&contract)
            .map_err(to_py_err)
    }

    /// Unsubscribe from quote data for an option contract.
    fn unsubscribe_option_quotes(
        &self,
        symbol: &str,
        exp_date: &str,
        right: &str,
        strike: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, strike, right);
        self.tdx.unsubscribe_quotes(&contract).map_err(to_py_err)
    }

    /// Unsubscribe from trade data for an option contract.
    fn unsubscribe_option_trades(
        &self,
        symbol: &str,
        exp_date: &str,
        right: &str,
        strike: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, strike, right);
        self.tdx.unsubscribe_trades(&contract).map_err(to_py_err)
    }

    /// Unsubscribe from open interest data for an option contract.
    fn unsubscribe_option_open_interest(
        &self,
        symbol: &str,
        exp_date: &str,
        right: &str,
        strike: &str,
    ) -> PyResult<()> {
        let contract = fpss::protocol::Contract::option(symbol, exp_date, strike, right);
        self.tdx
            .unsubscribe_open_interest(&contract)
            .map_err(to_py_err)
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
            let rx = rx_arc.lock().unwrap_or_else(|e| e.into_inner());
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
                .block_on(async { self.tdx.stock_list_symbols().await })
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
                .block_on(async { self.tdx.stock_list_dates(request_type, symbol).await })
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
                .block_on(async { self.tdx.stock_snapshot_ohlc(&refs).await })
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
                .block_on(async { self.tdx.stock_snapshot_trade(&refs).await })
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
                .block_on(async { self.tdx.stock_snapshot_quote(&refs).await })
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async {
                    self.tdx.stock_snapshot_market_value(&refs).await
                })
                .map_err(to_py_err)
        })?;
        Ok(ticks
            .iter()
            .map(|t| market_value_tick_to_dict(py, t))
            .collect())
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
                .block_on(async { self.tdx.stock_history_eod(symbol, start_date, end_date).await })
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
                .block_on(async { self.tdx.stock_history_ohlc(
                    symbol,
                    date,
                    interval,
                ).await })
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
                .block_on(async { self.tdx.stock_history_ohlc_range(
                    symbol,
                    start_date,
                    end_date,
                    interval,
                ).await })
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
                .block_on(async { self.tdx.stock_history_trade(
                    symbol,
                    date,
                ).await })
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
                .block_on(async { self.tdx.stock_history_quote(
                    symbol,
                    date,
                    interval,
                ).await })
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.stock_history_trade_quote(
                    symbol,
                    date,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks
            .iter()
            .map(|t| trade_quote_tick_to_dict(py, t))
            .collect())
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
                .block_on(async { self.tdx.stock_at_time_trade(
                    symbol,
                    start_date,
                    end_date,
                    time_of_day,
                ).await })
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
                .block_on(async { self.tdx.stock_at_time_quote(
                    symbol,
                    start_date,
                    end_date,
                    time_of_day,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }

    // Option — List (5)

    fn option_list_symbols(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_list_symbols().await })
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
                .block_on(async { self.tdx.option_list_dates(
                    request_type,
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
                .map_err(to_py_err)
        })
    }
    fn option_list_expirations(&self, py: Python<'_>, symbol: &str) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_list_expirations(symbol).await })
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
                .block_on(async { self.tdx.option_list_strikes(symbol, expiration).await })
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_list_contracts(
                    request_type,
                    symbol,
                    date,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks
            .iter()
            .map(|t| option_contract_to_dict(py, t))
            .collect())
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
                .block_on(async { self.tdx.option_snapshot_ohlc(
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
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
                .block_on(async { self.tdx.option_snapshot_trade(
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
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
                .block_on(async { self.tdx.option_snapshot_quote(
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_snapshot_open_interest(
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks
            .iter()
            .map(|t| open_interest_tick_to_dict(py, t))
            .collect())
    }
    fn option_snapshot_market_value(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_snapshot_market_value(
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks
            .iter()
            .map(|t| market_value_tick_to_dict(py, t))
            .collect())
    }
    fn option_snapshot_greeks_implied_volatility(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_snapshot_greeks_implied_volatility(
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| iv_tick_to_dict(py, t)).collect())
    }
    fn option_snapshot_greeks_all(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_snapshot_greeks_all(
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
    }
    fn option_snapshot_greeks_first_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_snapshot_greeks_first_order(
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
    }
    fn option_snapshot_greeks_second_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_snapshot_greeks_second_order(
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
    }
    fn option_snapshot_greeks_third_order(
        &self,
        py: Python<'_>,
        symbol: &str,
        expiration: &str,
        strike: &str,
        right: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_snapshot_greeks_third_order(
                    symbol,
                    expiration,
                    strike,
                    right,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
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
        let ticks =
            py.detach(|| {
                runtime()
                    .block_on(async { self.tdx.option_history_eod(
                        symbol, expiration, strike, right, start_date, end_date,
                    ).await })
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
                .block_on(async { self.tdx.option_history_ohlc(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                    interval,
                ).await })
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
                .block_on(async { self.tdx.option_history_trade(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                ).await })
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
                .block_on(async { self.tdx.option_history_quote(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                    interval,
                ).await })
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_trade_quote(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks
            .iter()
            .map(|t| trade_quote_tick_to_dict(py, t))
            .collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_open_interest(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks
            .iter()
            .map(|t| open_interest_tick_to_dict(py, t))
            .collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_greeks_eod(
                    symbol,
                    expiration,
                    strike,
                    right,
                    start_date,
                    end_date,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_greeks_all(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                    interval,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_trade_greeks_all(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_greeks_first_order(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                    interval,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_trade_greeks_first_order(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_greeks_second_order(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                    interval,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_trade_greeks_second_order(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_greeks_third_order(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                    interval,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_trade_greeks_third_order(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| greeks_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_greeks_implied_volatility(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                    interval,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| iv_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.option_history_trade_greeks_implied_volatility(
                    symbol,
                    expiration,
                    strike,
                    right,
                    date,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| iv_tick_to_dict(py, t)).collect())
    }

    // Option — At-Time (2)

    #[allow(clippy::too_many_arguments)] // Reason: ThetaData API requires all option identifiers + date range + time
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
                .block_on(async { self.tdx.option_at_time_trade(
                    symbol,
                    expiration,
                    strike,
                    right,
                    start_date,
                    end_date,
                    time_of_day,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    #[allow(clippy::too_many_arguments)] // Reason: ThetaData API requires all option identifiers + date range + time
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
                .block_on(async { self.tdx.option_at_time_quote(
                    symbol,
                    expiration,
                    strike,
                    right,
                    start_date,
                    end_date,
                    time_of_day,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }

    // Index — List (2)

    fn index_list_symbols(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(async { self.tdx.index_list_symbols().await })
                .map_err(to_py_err)
        })
    }
    fn index_list_dates(&self, py: Python<'_>, symbol: &str) -> PyResult<Vec<String>> {
        py.detach(|| {
            runtime()
                .block_on(async { self.tdx.index_list_dates(symbol).await })
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
                .block_on(async { self.tdx.index_snapshot_ohlc(&refs).await })
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.index_snapshot_price(&refs).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| price_tick_to_dict(py, t)).collect())
    }
    fn index_snapshot_market_value(
        &self,
        py: Python<'_>,
        symbols: Vec<String>,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let ticks = py.detach(|| {
            runtime()
                .block_on(async {
                    self.tdx.index_snapshot_market_value(&refs).await
                })
                .map_err(to_py_err)
        })?;
        Ok(ticks
            .iter()
            .map(|t| market_value_tick_to_dict(py, t))
            .collect())
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
                .block_on(async { self.tdx.index_history_eod(symbol, start_date, end_date).await })
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
                .block_on(async {
                    self.tdx.index_history_ohlc(symbol, start_date, end_date, interval).await
                })
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.index_history_price(
                    symbol,
                    date,
                    interval,
                ).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| price_tick_to_dict(py, t)).collect())
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
        let ticks = py.detach(|| {
            runtime()
                .block_on(async {
                    self.tdx.index_at_time_price(symbol, start_date, end_date, time_of_day).await
                })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| price_tick_to_dict(py, t)).collect())
    }

    // Calendar (3)

    fn calendar_open_today(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.calendar_open_today().await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| calendar_day_to_dict(py, t)).collect())
    }
    fn calendar_on_date(&self, py: Python<'_>, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.calendar_on_date(date).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| calendar_day_to_dict(py, t)).collect())
    }
    fn calendar_year(&self, py: Python<'_>, year: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(async { self.tdx.calendar_year(year).await })
                .map_err(to_py_err)
        })?;
        Ok(ticks.iter().map(|t| calendar_day_to_dict(py, t)).collect())
    }

    // Interest Rate (1)

    fn interest_rate_history_eod(
        &self,
        py: Python<'_>,
        symbol: &str,
        start_date: &str,
        end_date: &str,
    ) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| {
            runtime()
                .block_on(async {
                    self.tdx.interest_rate_history_eod(symbol, start_date, end_date).await
                })
                .map_err(to_py_err)
        })?;
        Ok(ticks
            .iter()
            .map(|t| interest_rate_tick_to_dict(py, t))
            .collect())
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
