//! Python bindings for `thetadatadx` — wraps the Rust SDK via PyO3.
//!
//! This is NOT a reimplementation. Every call goes through the Rust crate,
//! giving Python users native performance for ThetaData market data access.

use pyo3::exceptions::{PyConnectionError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::sync::OnceLock;
use thetadatadx::auth;
use thetadatadx::config;
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
                    None => {
                        dict.set_item(key, py.None()).unwrap();
                    }
                }
            }
            dict.into_any().unbind()
        })
        .collect()
}

// ── DirectClient ──

#[pyclass]
struct DirectClient {
    inner: thetadatadx::DirectClient,
}

#[pymethods]
impl DirectClient {
    /// Connect to ThetaData servers (authenticates via Nexus API).
    #[new]
    fn new(creds: &Credentials, config: &Config) -> PyResult<Self> {
        let inner = runtime()
            .block_on(thetadatadx::DirectClient::connect(
                &creds.inner,
                config.inner.clone(),
            ))
            .map_err(to_py_err)?;
        Ok(Self { inner })
    }

    // ── Stock — List (2) ──

    fn stock_list_symbols(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        py.detach(|| { runtime().block_on(self.inner.stock_list_symbols()).map_err(to_py_err) })
    }
    fn stock_list_dates(&self, py: Python<'_>, request_type: &str, symbol: &str) -> PyResult<Vec<String>> {
        py.detach(|| { runtime().block_on(self.inner.stock_list_dates(request_type, symbol)).map_err(to_py_err) })
    }

    // ── Stock — Snapshot (4) ──

    fn stock_snapshot_ohlc(&self, py: Python<'_>, symbols: Vec<String>) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let ticks = py.detach(|| { runtime().block_on(self.inner.stock_snapshot_ohlc(&refs)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn stock_snapshot_trade(&self, py: Python<'_>, symbols: Vec<String>) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let ticks = py.detach(|| { runtime().block_on(self.inner.stock_snapshot_trade(&refs)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn stock_snapshot_quote(&self, py: Python<'_>, symbols: Vec<String>) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let ticks = py.detach(|| { runtime().block_on(self.inner.stock_snapshot_quote(&refs)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }
    fn stock_snapshot_market_value(&self, py: Python<'_>, symbols: Vec<String>) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let table = py.detach(|| { runtime().block_on(self.inner.stock_snapshot_market_value(&refs)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── Stock — History (5 + bonus) ──

    fn stock_history_eod(&self, py: Python<'_>, symbol: &str, start_date: &str, end_date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.stock_history_eod(symbol, start_date, end_date)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| eod_tick_to_dict(py, t)).collect())
    }
    fn stock_history_ohlc(&self, py: Python<'_>, symbol: &str, date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.stock_history_ohlc(symbol, date, interval)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn stock_history_ohlc_range(&self, py: Python<'_>, symbol: &str, start_date: &str, end_date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.stock_history_ohlc_range(symbol, start_date, end_date, interval)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn stock_history_trade(&self, py: Python<'_>, symbol: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.stock_history_trade(symbol, date)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn stock_history_quote(&self, py: Python<'_>, symbol: &str, date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.stock_history_quote(symbol, date, interval)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }
    fn stock_history_trade_quote(&self, py: Python<'_>, symbol: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.stock_history_trade_quote(symbol, date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── Stock — At-Time (2) ──

    fn stock_at_time_trade(&self, py: Python<'_>, symbol: &str, start_date: &str, end_date: &str, time_of_day: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.stock_at_time_trade(symbol, start_date, end_date, time_of_day)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn stock_at_time_quote(&self, py: Python<'_>, symbol: &str, start_date: &str, end_date: &str, time_of_day: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.stock_at_time_quote(symbol, start_date, end_date, time_of_day)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }

    // ── Option — List (5) ──

    fn option_list_symbols(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        py.detach(|| { runtime().block_on(self.inner.option_list_symbols()).map_err(to_py_err) })
    }
    fn option_list_dates(&self, py: Python<'_>, request_type: &str, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<String>> {
        py.detach(|| { runtime().block_on(self.inner.option_list_dates(request_type, symbol, expiration, strike, right)).map_err(to_py_err) })
    }
    fn option_list_expirations(&self, py: Python<'_>, symbol: &str) -> PyResult<Vec<String>> {
        py.detach(|| { runtime().block_on(self.inner.option_list_expirations(symbol)).map_err(to_py_err) })
    }
    fn option_list_strikes(&self, py: Python<'_>, symbol: &str, expiration: &str) -> PyResult<Vec<String>> {
        py.detach(|| { runtime().block_on(self.inner.option_list_strikes(symbol, expiration)).map_err(to_py_err) })
    }
    fn option_list_contracts(&self, py: Python<'_>, request_type: &str, symbol: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_list_contracts(request_type, symbol, date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── Option — Snapshot (10) ──

    fn option_snapshot_ohlc(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.option_snapshot_ohlc(symbol, expiration, strike, right)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn option_snapshot_trade(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.option_snapshot_trade(symbol, expiration, strike, right)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn option_snapshot_quote(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.option_snapshot_quote(symbol, expiration, strike, right)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }
    fn option_snapshot_open_interest(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_snapshot_open_interest(symbol, expiration, strike, right)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_market_value(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_snapshot_market_value(symbol, expiration, strike, right)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_greeks_implied_volatility(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_snapshot_greeks_implied_volatility(symbol, expiration, strike, right)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_greeks_all(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_snapshot_greeks_all(symbol, expiration, strike, right)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_greeks_first_order(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_snapshot_greeks_first_order(symbol, expiration, strike, right)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_greeks_second_order(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_snapshot_greeks_second_order(symbol, expiration, strike, right)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_snapshot_greeks_third_order(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_snapshot_greeks_third_order(symbol, expiration, strike, right)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── Option — History (6) ──

    fn option_history_eod(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, start_date: &str, end_date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.option_history_eod(symbol, expiration, strike, right, start_date, end_date)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| eod_tick_to_dict(py, t)).collect())
    }
    fn option_history_ohlc(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.option_history_ohlc(symbol, expiration, strike, right, date, interval)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn option_history_trade(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.option_history_trade(symbol, expiration, strike, right, date)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn option_history_quote(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.option_history_quote(symbol, expiration, strike, right, date, interval)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }
    fn option_history_trade_quote(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_trade_quote(symbol, expiration, strike, right, date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_open_interest(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_open_interest(symbol, expiration, strike, right, date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── Option — History Greeks (11) ──

    fn option_history_greeks_eod(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, start_date: &str, end_date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_greeks_eod(symbol, expiration, strike, right, start_date, end_date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_greeks_all(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_greeks_all(symbol, expiration, strike, right, date, interval)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_trade_greeks_all(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_trade_greeks_all(symbol, expiration, strike, right, date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_greeks_first_order(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_greeks_first_order(symbol, expiration, strike, right, date, interval)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_trade_greeks_first_order(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_trade_greeks_first_order(symbol, expiration, strike, right, date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_greeks_second_order(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_greeks_second_order(symbol, expiration, strike, right, date, interval)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_trade_greeks_second_order(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_trade_greeks_second_order(symbol, expiration, strike, right, date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_greeks_third_order(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_greeks_third_order(symbol, expiration, strike, right, date, interval)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_trade_greeks_third_order(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_trade_greeks_third_order(symbol, expiration, strike, right, date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_greeks_implied_volatility(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_greeks_implied_volatility(symbol, expiration, strike, right, date, interval)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn option_history_trade_greeks_implied_volatility(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.option_history_trade_greeks_implied_volatility(symbol, expiration, strike, right, date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── Option — At-Time (2) ──

    fn option_at_time_trade(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, start_date: &str, end_date: &str, time_of_day: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.option_at_time_trade(symbol, expiration, strike, right, start_date, end_date, time_of_day)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| trade_tick_to_dict(py, t)).collect())
    }
    fn option_at_time_quote(&self, py: Python<'_>, symbol: &str, expiration: &str, strike: &str, right: &str, start_date: &str, end_date: &str, time_of_day: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.option_at_time_quote(symbol, expiration, strike, right, start_date, end_date, time_of_day)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| quote_tick_to_dict(py, t)).collect())
    }

    // ── Index — List (2) ──

    fn index_list_symbols(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        py.detach(|| { runtime().block_on(self.inner.index_list_symbols()).map_err(to_py_err) })
    }
    fn index_list_dates(&self, py: Python<'_>, symbol: &str) -> PyResult<Vec<String>> {
        py.detach(|| { runtime().block_on(self.inner.index_list_dates(symbol)).map_err(to_py_err) })
    }

    // ── Index — Snapshot (3) ──

    fn index_snapshot_ohlc(&self, py: Python<'_>, symbols: Vec<String>) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let ticks = py.detach(|| { runtime().block_on(self.inner.index_snapshot_ohlc(&refs)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn index_snapshot_price(&self, py: Python<'_>, symbols: Vec<String>) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let table = py.detach(|| { runtime().block_on(self.inner.index_snapshot_price(&refs)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn index_snapshot_market_value(&self, py: Python<'_>, symbols: Vec<String>) -> PyResult<Vec<Py<PyAny>>> {
        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
        let table = py.detach(|| { runtime().block_on(self.inner.index_snapshot_market_value(&refs)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── Index — History (3) ──

    fn index_history_eod(&self, py: Python<'_>, symbol: &str, start_date: &str, end_date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.index_history_eod(symbol, start_date, end_date)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| eod_tick_to_dict(py, t)).collect())
    }
    fn index_history_ohlc(&self, py: Python<'_>, symbol: &str, start_date: &str, end_date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let ticks = py.detach(|| { runtime().block_on(self.inner.index_history_ohlc(symbol, start_date, end_date, interval)).map_err(to_py_err) })?;
        Ok(ticks.iter().map(|t| ohlc_tick_to_dict(py, t)).collect())
    }
    fn index_history_price(&self, py: Python<'_>, symbol: &str, date: &str, interval: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.index_history_price(symbol, date, interval)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── Index — At-Time (1) ──

    fn index_at_time_price(&self, py: Python<'_>, symbol: &str, start_date: &str, end_date: &str, time_of_day: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.index_at_time_price(symbol, start_date, end_date, time_of_day)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── Calendar (3) ──

    fn calendar_open_today(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.calendar_open_today()).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn calendar_on_date(&self, py: Python<'_>, date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.calendar_on_date(date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }
    fn calendar_year(&self, py: Python<'_>, year: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.calendar_year(year)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    // ── Interest Rate (1) ──

    fn interest_rate_history_eod(&self, py: Python<'_>, symbol: &str, start_date: &str, end_date: &str) -> PyResult<Vec<Py<PyAny>>> {
        let table = py.detach(|| { runtime().block_on(self.inner.interest_rate_history_eod(symbol, start_date, end_date)).map_err(to_py_err) })?;
        Ok(data_table_to_dicts(py, &table))
    }

    fn __repr__(&self) -> String {
        "DirectClient(connected)".to_string()
    }
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
    let g = thetadatadx::greeks::all_greeks(spot, strike, rate, div_yield, tte, option_price, is_call);
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
    thetadatadx::greeks::implied_volatility(spot, strike, rate, div_yield, tte, option_price, is_call)
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
    m.add_class::<DirectClient>()?;
    m.add_function(wrap_pyfunction!(all_greeks, m)?)?;
    m.add_function(wrap_pyfunction!(implied_volatility, m)?)?;
    Ok(())
}
