//! Fluent contract-first Python surface.
//!
//! Mirrors the Rust API laid out in
//! `report/ThetaDataDxClient_API_DX_Review_Rust_Python.md`:
//!
//! ```python
//! from thetadatadx import Contract, SecType
//!
//! stock  = Contract.stock("AAPL")
//! option = Contract.option("SPY", expiration="20260620", strike="550", right="C")
//!
//! with client.streaming(on_event) as session:
//!     session.subscribe(stock.quote())
//!     session.subscribe(option.trade())
//!     session.subscribe(SecType.OPTION.full_open_interest())
//!     session.subscribe_many([stock.quote(), option.quote()])
//! ```
//!
//! `Subscription` is a typed pyclass — the polymorphic
//! `client.subscribe(sub)` path on the Rust core dispatches on the
//! enum without any string parsing.

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use thetadatadx::fpss::protocol::{self, FullSubscriptionKind, SecTypeExt as _, SubscriptionKind};

/// Mirror of the Rust `tdbe::types::enums::SecType` value, exposed as
/// a Python class with class-level constants so users can write
/// `SecType.OPTION.full_trades()` without importing the underlying
/// `tdbe` crate.
#[pyclass(module = "thetadatadx", name = "SecType", frozen, skip_from_py_object)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct PySecType {
    pub(crate) inner: tdbe::types::enums::SecType,
}

impl PySecType {
    pub(crate) fn from_inner(inner: tdbe::types::enums::SecType) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PySecType {
    #[classattr]
    #[allow(non_snake_case)]
    fn STOCK() -> Self {
        Self::from_inner(tdbe::types::enums::SecType::Stock)
    }

    #[classattr]
    #[allow(non_snake_case)]
    fn OPTION() -> Self {
        Self::from_inner(tdbe::types::enums::SecType::Option)
    }

    #[classattr]
    #[allow(non_snake_case)]
    fn INDEX() -> Self {
        Self::from_inner(tdbe::types::enums::SecType::Index)
    }

    #[classattr]
    #[allow(non_snake_case)]
    fn RATE() -> Self {
        Self::from_inner(tdbe::types::enums::SecType::Rate)
    }

    /// Full-stream Trade subscription for this security type. Pair with
    /// `client.subscribe(sec_type.full_trades())`.
    fn full_trades(&self) -> PySubscription {
        PySubscription {
            inner: self.inner.full_trades(),
        }
    }

    /// Full-stream OpenInterest subscription for this security type.
    fn full_open_interest(&self) -> PySubscription {
        PySubscription {
            inner: self.inner.full_open_interest(),
        }
    }

    fn __repr__(&self) -> String {
        format!("SecType.{:?}", self.inner).to_uppercase()
    }

    fn __str__(&self) -> String {
        format!("{:?}", self.inner).to_uppercase()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> isize {
        self.inner as isize
    }
}

/// Fluent contract identifier exposed to Python.
///
/// ```python
/// stock  = Contract.stock("AAPL")
/// option = Contract.option("SPY", expiration="20260620", strike="550", right="C")
/// ```
#[pyclass(module = "thetadatadx", name = "Contract", frozen, skip_from_py_object)]
#[derive(Clone, Debug)]
pub(crate) struct PyContract {
    pub(crate) inner: protocol::Contract,
}

impl PyContract {
    pub(crate) fn from_inner(inner: protocol::Contract) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyContract {
    /// Construct a stock contract.
    #[staticmethod]
    fn stock(symbol: &str) -> Self {
        Self::from_inner(protocol::Contract::stock(symbol))
    }

    /// Construct an index contract.
    #[staticmethod]
    fn index(symbol: &str) -> Self {
        Self::from_inner(protocol::Contract::index(symbol))
    }

    /// Construct an option contract. `strike` is the price in dollars
    /// and accepts a number or a string (`550`, `550.0`, and `"550"`
    /// are equivalent).
    #[staticmethod]
    #[pyo3(signature = (symbol, *, expiration, strike, right))]
    fn option(symbol: &str, expiration: &str, strike: StrikeArg, right: &str) -> PyResult<Self> {
        protocol::Contract::option(symbol, expiration, &strike.into_string(), right)
            .map(Self::from_inner)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// Per-contract Quote subscription.
    fn quote(&self) -> PySubscription {
        PySubscription {
            inner: self.inner.quote(),
        }
    }

    /// Per-contract Trade subscription.
    fn trade(&self) -> PySubscription {
        PySubscription {
            inner: self.inner.trade(),
        }
    }

    /// Per-contract OpenInterest subscription.
    fn open_interest(&self) -> PySubscription {
        PySubscription {
            inner: self.inner.open_interest(),
        }
    }

    #[getter]
    fn symbol(&self) -> &str {
        &self.inner.symbol
    }

    /// Security type as a symbolic uppercase name (`"STOCK"` /
    /// `"OPTION"` / `"INDEX"` / `"RATE"`). A string — matching the
    /// streaming `ContractRef.sec_type` event surface and the
    /// TypeScript binding — so one concept reads as one type across the
    /// whole surface. Build full-stream subscriptions through the
    /// `SecType` class (`SecType.OPTION.full_trades()`), not this getter.
    #[getter]
    fn sec_type(&self) -> &'static str {
        self.inner.sec_type.as_str()
    }

    /// Expiration date as a `YYYYMMDD` integer; `None` for non-options.
    #[getter]
    fn expiration(&self) -> Option<i32> {
        self.inner.expiration
    }

    /// Strike price in dollars; `None` for non-options. Reads back the
    /// same notation `Contract.option(strike=...)` takes, and joins
    /// directly against historical-row `strike` columns.
    #[getter]
    fn strike(&self) -> Option<f64> {
        self.inner.strike_dollars()
    }

    /// Option right (`"C"` / `"P"`); `None` for non-options.
    #[getter]
    fn right(&self) -> Option<&'static str> {
        self.inner.is_call.map(|c| if c { "C" } else { "P" })
    }

    fn __repr__(&self) -> String {
        format!("Contract({})", self.inner)
    }

    fn __str__(&self) -> String {
        format!("{}", self.inner)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

/// Typed market-data subscription returned by `Contract.quote()` /
/// `Contract.trade()` / `Contract.open_interest()` and by
/// `SecType.OPTION.full_trades()` / `SecType.OPTION.full_open_interest()`.
///
/// Pass to `client.subscribe(sub)` or `client.subscribe_many([sub, ...])`.
#[pyclass(
    module = "thetadatadx",
    name = "Subscription",
    frozen,
    skip_from_py_object
)]
#[derive(Clone, Debug)]
pub(crate) struct PySubscription {
    pub(crate) inner: protocol::Subscription,
}

#[pymethods]
impl PySubscription {
    /// One of `"quote"`, `"trade"`, `"open_interest"`, `"full_trades"`,
    /// or `"full_open_interest"` — the wire-level kind for this
    /// subscription.
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            protocol::Subscription::Contract { kind, .. } => match kind {
                SubscriptionKind::Quote => "quote",
                SubscriptionKind::Trade => "trade",
                SubscriptionKind::OpenInterest => "open_interest",
                _ => "unknown",
            },
            protocol::Subscription::Full { kind, .. } => match kind {
                FullSubscriptionKind::Trades => "full_trades",
                FullSubscriptionKind::OpenInterest => "full_open_interest",
                _ => "unknown",
            },
            _ => "unknown",
        }
    }

    /// `True` for full-stream (security-type-scoped) subscriptions.
    #[getter]
    fn is_full(&self) -> bool {
        matches!(&self.inner, protocol::Subscription::Full { .. })
    }

    /// The bound contract for per-contract subscriptions, `None` for
    /// full-stream subscriptions.
    #[getter]
    fn contract(&self) -> Option<PyContract> {
        match &self.inner {
            protocol::Subscription::Contract { contract, .. } => {
                Some(PyContract::from_inner(contract.clone()))
            }
            _ => None,
        }
    }

    /// The security type for full-stream subscriptions, `None` for
    /// per-contract subscriptions.
    #[getter]
    fn sec_type(&self) -> Option<PySecType> {
        match &self.inner {
            protocol::Subscription::Full { sec_type, .. } => Some(PySecType::from_inner(*sec_type)),
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            protocol::Subscription::Contract { contract, kind } => {
                format!("Subscription({kind:?}, {contract})")
            }
            protocol::Subscription::Full { sec_type, kind } => {
                format!("Subscription(full {kind:?}, {sec_type:?})")
            }
            _ => "Subscription(<unknown>)".to_string(),
        }
    }
}

/// Strike argument accepted by `Contract.option`: a number (int or
/// float, dollars) or a string (dollars; the historical-endpoint
/// wildcard `"*"` is NOT valid here — streaming contracts address one
/// strike). Converted to the canonical string form the core builder
/// parses.
#[derive(FromPyObject)]
pub(crate) enum StrikeArg {
    /// Integer dollars (`550`).
    Int(i64),
    /// Float dollars (`550.5`).
    Float(f64),
    /// String dollars (`"550"` / `"550.50"`).
    Text(String),
}

impl StrikeArg {
    fn into_string(self) -> String {
        match self {
            Self::Int(v) => v.to_string(),
            Self::Float(v) => v.to_string(),
            Self::Text(s) => s,
        }
    }
}

/// Coerce a `Subscription`, `Contract`, or symbol-style string from
/// Python into a `protocol::Subscription`. Bare strings are accepted
/// for ergonomics but always interpreted as a stock-quote
/// subscription — that is the most common shorthand in the existing
/// compat helpers.
pub(crate) fn coerce_subscription(obj: &Bound<'_, PyAny>) -> PyResult<protocol::Subscription> {
    if let Ok(sub) = obj.extract::<PyRef<PySubscription>>() {
        return Ok(sub.inner.clone());
    }
    Err(PyTypeError::new_err(
        "expected a Subscription value (use Contract.quote() / .trade() / .open_interest() or \
         SecType.OPTION.full_trades())",
    ))
}

/// Coerce a Python iterable of `Subscription` into a `Vec`.
pub(crate) fn coerce_subscription_list(
    obj: &Bound<'_, PyAny>,
) -> PyResult<Vec<protocol::Subscription>> {
    // Accept any iterable (list / tuple / generator).
    let mut out = Vec::new();
    let iter = obj.try_iter()?;
    for item in iter {
        let item = item?;
        out.push(coerce_subscription(&item)?);
    }
    Ok(out)
}

/// Register the fluent classes (Contract / Subscription / SecType) on
/// the module.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyContract>()?;
    m.add_class::<PySubscription>()?;
    m.add_class::<PySecType>()?;

    // Module-level diagnostic — gives Python users a one-shot probe to
    // discover the fluent surface.
    let dict = PyDict::new(m.py());
    dict.set_item("contract", "Contract.stock(...) | Contract.option(...)")?;
    dict.set_item(
        "subscription",
        "contract.quote() / .trade() / .open_interest()",
    )?;
    dict.set_item(
        "full_stream",
        "SecType.OPTION.full_trades() / .full_open_interest()",
    )?;
    dict.set_item(
        "subscribe",
        "client.subscribe(sub) | client.subscribe_many([sub, ...])",
    )?;
    m.add("__fluent_api__", dict)?;

    // Module-level constant for the canonical fluent symbols. Ordered
    // list mirrors the docstring in `report/...md`.
    let names = PyList::new(
        m.py(),
        ["Contract", "Subscription", "SecType"].iter().copied(),
    )?;
    m.add("__fluent_classes__", names)?;
    Ok(())
}
