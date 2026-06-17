//! Hand-written Python bindings for the FLATFILES surface.
//!
//! Unlike the historical / streaming surfaces, FLATFILES has only one
//! Rust public entry point (`flatfile_request_decoded`) whose schema is
//! determined at runtime by `(SecType, ReqType)`. Codegen via
//! `endpoint_surface.toml` / `tick_schema.toml` does not apply — those
//! pipelines target static-schema endpoints. This module is intentionally
//! hand-written and stays under 350 LOC.
//!
//! Surface shape:
//!
//! ```python
//! client.flat_files.option_trade_quote(date="20260428").to_polars()
//! client.flat_files.option_open_interest(date="20260428").to_arrow()
//! client.flat_files.stock_eod(date="20260428").to_pandas()
//! client.flatfile_to_path("OPTION", "TRADE_QUOTE", "20260428",
//!                      "/tmp/spy.csv", "csv")  # raw on-disk
//! ```
//!
//! Every typed terminal funnels through
//! `thetadatadx::flatfiles::arrow::rows_to_arrow` so the Arrow schema
//! lives in exactly one place and is shared with the TypeScript / C++
//! bindings.

use std::sync::Arc;

use pyo3::exceptions::{PyImportError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyList;

use thetadatadx::flatfiles::{self, FlatFileFormat, FlatFileRow, FlatFileValue, ReqType, SecType};

use crate::async_runtime::spawn_awaitable;
use crate::errors::to_py_err;
use crate::record_batch_to_pyarrow_table;
use crate::run_blocking;

// ── Helpers ────────────────────────────────────────────────────────────

fn parse_flatfile_sec_type(sec: &str) -> PyResult<SecType> {
    match sec.to_uppercase().as_str() {
        "OPTION" => Ok(SecType::Option),
        "STOCK" => Ok(SecType::Stock),
        "INDEX" => Ok(SecType::Index),
        other => Err(PyValueError::new_err(format!(
            "unknown flat-file sec_type: {other:?} (expected OPTION, STOCK, or INDEX)"
        ))),
    }
}

fn parse_flatfile_req_type(req: &str) -> PyResult<ReqType> {
    match req.to_uppercase().as_str() {
        "EOD" => Ok(ReqType::Eod),
        "QUOTE" => Ok(ReqType::Quote),
        "OPEN_INTEREST" | "OPENINTEREST" => Ok(ReqType::OpenInterest),
        "OHLC" => Ok(ReqType::Ohlc),
        "TRADE" => Ok(ReqType::Trade),
        "TRADE_QUOTE" | "TRADEQUOTE" => Ok(ReqType::TradeQuote),
        other => Err(PyValueError::new_err(format!(
            "unknown flat-file req_type: {other:?} (expected EOD, QUOTE, OPEN_INTEREST, OHLC, TRADE, TRADE_QUOTE)"
        ))),
    }
}

fn parse_flatfile_format(fmt: Option<&str>) -> PyResult<FlatFileFormat> {
    match fmt.unwrap_or("csv").to_lowercase().as_str() {
        "csv" => Ok(FlatFileFormat::Csv),
        "jsonl" | "json" => Ok(FlatFileFormat::Jsonl),
        other => Err(PyValueError::new_err(format!(
            "unknown flat-file format: {other:?} (expected csv or jsonl)"
        ))),
    }
}

fn rows_to_pyarrow_table(py: Python<'_>, rows: &[FlatFileRow]) -> PyResult<Py<PyAny>> {
    let batch = flatfiles::arrow::rows_to_arrow(rows).map_err(to_py_err)?;
    record_batch_to_pyarrow_table(py, batch)
}

fn pyarrow_table_to_pandas_local(py: Python<'_>, table: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let bound = table.bind(py);
    let df = bound.call_method0("to_pandas").map_err(|e| {
        if e.is_instance_of::<PyImportError>(py) {
            PyImportError::new_err(
                "pandas is required for .to_pandas(). Install with: pip install thetadatadx[pandas]",
            )
        } else {
            e
        }
    })?;
    Ok(df.unbind())
}

fn pyarrow_table_to_polars_local(py: Python<'_>, table: Py<PyAny>) -> PyResult<Py<PyAny>> {
    let polars = py.import("polars").map_err(|_| {
        PyImportError::new_err(
            "polars is not installed. Install it with: pip install thetadatadx[polars]",
        )
    })?;
    let df = polars.call_method1("from_arrow", (table,))?;
    Ok(df.unbind())
}

// ── FlatFileRowList ────────────────────────────────────────────────────

/// Result of a decoded flat-file pull. Wraps `Vec<FlatFileRow>` and
/// exposes the same terminal vocabulary as the typed `<TickName>List`
/// wrappers (`.to_arrow()`, `.to_pandas()`, `.to_polars()`, `.to_list()`)
/// despite the dynamic per-`(SecType, ReqType)` schema.
#[pyclass(module = "thetadatadx", frozen, name = "FlatFileRowList")]
pub struct FlatFileRowList {
    rows: Vec<FlatFileRow>,
}

#[pymethods]
impl FlatFileRowList {
    fn __len__(&self) -> usize {
        self.rows.len()
    }

    fn __bool__(&self) -> bool {
        !self.rows.is_empty()
    }

    fn __repr__(&self) -> String {
        format!("FlatFileRowList({} rows)", self.rows.len())
    }

    /// Return a `pyarrow.Table` whose schema is inferred from the first
    /// row's `(SecType, ReqType)` column shape.
    fn to_arrow(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        rows_to_pyarrow_table(py, &self.rows)
    }

    /// Return a `pandas.DataFrame`. Requires pandas + pyarrow.
    fn to_pandas(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let table = rows_to_pyarrow_table(py, &self.rows)?;
        pyarrow_table_to_pandas_local(py, table)
    }

    /// Return a `polars.DataFrame`. Requires polars + pyarrow.
    fn to_polars(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let table = rows_to_pyarrow_table(py, &self.rows)?;
        pyarrow_table_to_polars_local(py, table)
    }

    /// Return a plain Python list of dicts, one per row. Useful for
    /// quick inspection or when the caller wants to feed each row into
    /// a non-Arrow consumer (e.g. a backtester written against `dict`).
    fn to_list(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        for row in &self.rows {
            let dict = pyo3::types::PyDict::new(py);
            dict.set_item("symbol", &row.symbol)?;
            match row.expiration {
                Some(v) => dict.set_item("expiration", v)?,
                None => dict.set_item("expiration", py.None())?,
            }
            match row.strike {
                Some(v) => dict.set_item("strike", v)?,
                None => dict.set_item("strike", py.None())?,
            }
            match row.right {
                Some(c) => dict.set_item("right", c.to_string())?,
                None => dict.set_item("right", py.None())?,
            }
            for (name, value) in &row.fields {
                match value {
                    FlatFileValue::Int(v) => dict.set_item(name, *v)?,
                    FlatFileValue::Price(v) => dict.set_item(name, *v)?,
                }
            }
            list.append(dict)?;
        }
        Ok(list.into_any().unbind())
    }
}

// ── FlatFilesNamespace ─────────────────────────────────────────────────

/// Namespace handle returned by `client.flat_files`. Each method maps to
/// one `(SecType, ReqType)` tuple and runs `flatfile_request_decoded`
/// under the shared tokio runtime, yielding a [`FlatFileRowList`].
#[pyclass(module = "thetadatadx", frozen, name = "FlatFilesNamespace")]
pub struct FlatFilesNamespace {
    pub(crate) client: Arc<thetadatadx::Client>,
}

impl FlatFilesNamespace {
    fn pull_decoded(
        &self,
        py: Python<'_>,
        sec: SecType,
        req: ReqType,
        date: &str,
    ) -> PyResult<FlatFileRowList> {
        let client = Arc::clone(&self.client);
        let date_owned = date.to_string();
        let rows = run_blocking(py, async move {
            client.flatfile_request_decoded(sec, req, &date_owned).await
        })?;
        Ok(FlatFileRowList { rows })
    }

    fn pull_decoded_async<'py>(
        &self,
        py: Python<'py>,
        sec: SecType,
        req: ReqType,
        date: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let client = Arc::clone(&self.client);
        let date_owned = date.to_string();
        spawn_awaitable(
            py,
            async move { client.flatfile_request_decoded(sec, req, &date_owned).await },
            |py, rows| Ok(Py::new(py, FlatFileRowList { rows })?.into_any()),
        )
    }
}

#[pymethods]
impl FlatFilesNamespace {
    fn __repr__(&self) -> &'static str {
        "FlatFilesNamespace(option_*, stock_*; .to_arrow/.to_pandas/.to_polars/.to_list)"
    }

    /// Decoded option-trade-quote flat file for `date` (YYYYMMDD).
    fn option_trade_quote(&self, py: Python<'_>, date: &str) -> PyResult<FlatFileRowList> {
        self.pull_decoded(py, SecType::Option, ReqType::TradeQuote, date)
    }

    /// Decoded option-open-interest flat file for `date` (YYYYMMDD).
    fn option_open_interest(&self, py: Python<'_>, date: &str) -> PyResult<FlatFileRowList> {
        self.pull_decoded(py, SecType::Option, ReqType::OpenInterest, date)
    }

    /// Decoded option-EOD flat file for `date` (YYYYMMDD).
    fn option_eod(&self, py: Python<'_>, date: &str) -> PyResult<FlatFileRowList> {
        self.pull_decoded(py, SecType::Option, ReqType::Eod, date)
    }

    /// Decoded stock-trade-quote flat file for `date` (YYYYMMDD).
    fn stock_trade_quote(&self, py: Python<'_>, date: &str) -> PyResult<FlatFileRowList> {
        self.pull_decoded(py, SecType::Stock, ReqType::TradeQuote, date)
    }

    /// Decoded stock-EOD flat file for `date` (YYYYMMDD).
    fn stock_eod(&self, py: Python<'_>, date: &str) -> PyResult<FlatFileRowList> {
        self.pull_decoded(py, SecType::Stock, ReqType::Eod, date)
    }

    /// Generic dispatcher — `sec_type` and `req_type` accept the same
    /// strings as the per-method shortcuts, e.g. `"OPTION"` / `"QUOTE"`.
    /// Useful when the call shape comes from config and the user does
    /// not want to switch on the static method.
    fn request(
        &self,
        py: Python<'_>,
        sec_type: &str,
        req_type: &str,
        date: &str,
    ) -> PyResult<FlatFileRowList> {
        let sec = parse_flatfile_sec_type(sec_type)?;
        let req = parse_flatfile_req_type(req_type)?;
        self.pull_decoded(py, sec, req, date)
    }

    // ── Awaitable terminals ─────────────────────────────────────────
    //
    // Each `*_async` method is the awaitable twin of the sync method
    // above. A flat-file pull is a full-day blob download — seconds of
    // network plus a decode pass. The sync methods drive that to
    // completion on the calling thread, which is correct for a plain
    // `Client` call but would stall a running asyncio event loop when
    // reached through `AsyncClient.flat_files`. The awaitable terminals
    // resolve the download off the event loop so other coroutines keep
    // running while the day's data arrives, then yield the same
    // `FlatFileRowList`. Pick the suffix that matches your call site:
    // sync `flat_files.option_eod(date)` on a `Client`, awaitable
    // `await flat_files.option_eod_async(date)` inside a coroutine.

    /// Awaitable option-trade-quote flat file for `date` (YYYYMMDD).
    fn option_trade_quote_async<'py>(
        &self,
        py: Python<'py>,
        date: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.pull_decoded_async(py, SecType::Option, ReqType::TradeQuote, date)
    }

    /// Awaitable option-open-interest flat file for `date` (YYYYMMDD).
    fn option_open_interest_async<'py>(
        &self,
        py: Python<'py>,
        date: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.pull_decoded_async(py, SecType::Option, ReqType::OpenInterest, date)
    }

    /// Awaitable option-EOD flat file for `date` (YYYYMMDD).
    fn option_eod_async<'py>(&self, py: Python<'py>, date: &str) -> PyResult<Bound<'py, PyAny>> {
        self.pull_decoded_async(py, SecType::Option, ReqType::Eod, date)
    }

    /// Awaitable stock-trade-quote flat file for `date` (YYYYMMDD).
    fn stock_trade_quote_async<'py>(
        &self,
        py: Python<'py>,
        date: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.pull_decoded_async(py, SecType::Stock, ReqType::TradeQuote, date)
    }

    /// Awaitable stock-EOD flat file for `date` (YYYYMMDD).
    fn stock_eod_async<'py>(&self, py: Python<'py>, date: &str) -> PyResult<Bound<'py, PyAny>> {
        self.pull_decoded_async(py, SecType::Stock, ReqType::Eod, date)
    }

    /// Awaitable generic dispatcher. `sec_type` and `req_type` accept the
    /// same strings as `request(...)`, e.g. `"OPTION"` / `"QUOTE"`.
    fn request_async<'py>(
        &self,
        py: Python<'py>,
        sec_type: &str,
        req_type: &str,
        date: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let sec = parse_flatfile_sec_type(sec_type)?;
        let req = parse_flatfile_req_type(req_type)?;
        self.pull_decoded_async(py, sec, req, date)
    }
}

// ── Client pymethods extension ────────────────────────────────────
//
// Adding a second `#[pymethods]` block on `Client` requires the
// `multiple-pymethods` PyO3 feature, already enabled in
// `sdks/python/Cargo.toml` for the same reason the streaming /
// historical includes use it.

#[pymethods]
impl crate::Client {
    /// Namespace handle exposing the FLATFILES surface.
    ///
    /// Lazily constructed on each access. Internally clones the inner
    /// `Arc<thetadatadx::Client>` — no auth round-trip, no FPSS
    /// state mutation. Each call returns a fresh handle so that storing
    /// `flat_files = client.flat_files` in user code is identical to
    /// calling `client.flat_files.option_eod(...)` inline.
    #[getter]
    fn flat_files(&self) -> FlatFilesNamespace {
        FlatFilesNamespace {
            client: Arc::clone(&self.client),
        }
    }

    /// Pull a flat-file blob and write the requested format directly to
    /// `path`. Skips the typed-row decode step — useful when the caller
    /// only wants the vendor byte-format CSV / JSONL on disk and will
    /// load it into their own pipeline later.
    ///
    /// `sec_type` / `req_type` accept the same strings as
    /// `flat_files.request(...)`. `format` is `"csv"` (default) or
    /// `"jsonl"`. Returns the final on-disk path (with the extension
    /// auto-appended if absent).
    #[pyo3(signature = (sec_type, req_type, date, path, format=None))]
    fn flatfile_to_path(
        &self,
        py: Python<'_>,
        sec_type: &str,
        req_type: &str,
        date: &str,
        path: &str,
        format: Option<&str>,
    ) -> PyResult<String> {
        let sec = parse_flatfile_sec_type(sec_type)?;
        let req = parse_flatfile_req_type(req_type)?;
        let fmt = parse_flatfile_format(format)?;
        let client = Arc::clone(&self.client);
        let date_owned = date.to_string();
        let path_owned = std::path::PathBuf::from(path);
        let final_path = run_blocking(py, async move {
            client
                .flatfile_request(sec, req, &date_owned, &path_owned, fmt)
                .await
        })?;
        Ok(final_path.to_string_lossy().into_owned())
    }

    /// Awaitable twin of [`flatfile_to_path`](Self::flatfile_to_path).
    ///
    /// Writes the requested format straight to `path` without decoding
    /// into rows, resolving the blob download off the calling thread so a
    /// running event loop keeps servicing other coroutines while the
    /// day's file streams to disk. Yields the final on-disk path.
    #[pyo3(signature = (sec_type, req_type, date, path, format=None))]
    fn flatfile_to_path_async<'py>(
        &self,
        py: Python<'py>,
        sec_type: &str,
        req_type: &str,
        date: &str,
        path: &str,
        format: Option<&str>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let sec = parse_flatfile_sec_type(sec_type)?;
        let req = parse_flatfile_req_type(req_type)?;
        let fmt = parse_flatfile_format(format)?;
        let client = Arc::clone(&self.client);
        let date_owned = date.to_string();
        let path_owned = std::path::PathBuf::from(path);
        spawn_awaitable(
            py,
            async move {
                client
                    .flatfile_request(sec, req, &date_owned, &path_owned, fmt)
                    .await
            },
            |py, final_path| {
                Ok(final_path
                    .to_string_lossy()
                    .into_owned()
                    .into_pyobject(py)?
                    .into_any()
                    .unbind())
            },
        )
    }
}
