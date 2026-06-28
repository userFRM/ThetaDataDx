//! Argument-coercion newtypes for the generated endpoint bindings.
//!
//! Each newtype implements `FromPyObject` to accept the natural Python
//! shapes for a wire parameter (a bare string, an enum with a `.value`,
//! a `date` / `time` object) and normalizes them to the single string
//! form the Rust core expects. Centralizing the coercion here keeps the
//! generated method signatures free of per-argument extraction logic.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;

/// A string-valued endpoint argument: accepts a `str` or any enum whose
/// `.value` is a `str`.
#[derive(Clone)]
pub(crate) struct PyStringArg(String);

impl PyStringArg {
    /// Borrow the normalized string.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume into the owned string.
    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl<'py> FromPyObject<'_, 'py> for PyStringArg {
    type Error = PyErr;

    fn extract(obj: pyo3::Borrowed<'_, 'py, PyAny>) -> Result<Self, Self::Error> {
        if let Ok(value) = obj.extract::<String>() {
            return Ok(Self(value));
        }
        if let Ok(value_attr) = obj.getattr("value") {
            return Ok(Self(value_attr.extract::<String>()?));
        }
        Err(PyTypeError::new_err("expected str or enum value"))
    }
}

/// A date-valued endpoint argument: accepts a `YYYYMMDD` `str` or a
/// `date`/`datetime` object (formatted via `strftime("%Y%m%d")`).
#[derive(Clone)]
pub(crate) struct PyDateArg(String);

impl PyDateArg {
    /// Borrow the normalized `YYYYMMDD` string.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume into the owned `YYYYMMDD` string.
    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl<'py> FromPyObject<'_, 'py> for PyDateArg {
    type Error = PyErr;

    fn extract(obj: pyo3::Borrowed<'_, 'py, PyAny>) -> Result<Self, Self::Error> {
        if let Ok(value) = obj.extract::<String>() {
            return Ok(Self(value));
        }
        let formatted = obj.call_method1("strftime", ("%Y%m%d",))?;
        Ok(Self(formatted.extract::<String>()?))
    }
}

/// A time-valued endpoint argument: accepts an `HH:MM:SS` `str` or a
/// `time`/`datetime` object (formatted via `strftime("%H:%M:%S")`).
#[derive(Clone)]
pub(crate) struct PyTimeArg(String);

impl PyTimeArg {
    /// Borrow the normalized `HH:MM:SS` string.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume into the owned `HH:MM:SS` string.
    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl<'py> FromPyObject<'_, 'py> for PyTimeArg {
    type Error = PyErr;

    fn extract(obj: pyo3::Borrowed<'_, 'py, PyAny>) -> Result<Self, Self::Error> {
        if let Ok(value) = obj.extract::<String>() {
            return Ok(Self(value));
        }
        let formatted = obj.call_method1("strftime", ("%H:%M:%S",))?;
        Ok(Self(formatted.extract::<String>()?))
    }
}

/// A multi-symbol endpoint argument: accepts a single symbol `str`
/// (wrapped into a one-element list) or a sequence of symbol strings.
#[derive(Clone)]
pub(crate) struct PySymbols(Vec<String>);

impl PySymbols {
    /// Iterate over the collected symbols.
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, String> {
        self.0.iter()
    }

    /// Consume into the owned symbol vector.
    pub(crate) fn into_vec(self) -> Vec<String> {
        self.0
    }
}

impl<'py> FromPyObject<'_, 'py> for PySymbols {
    type Error = PyErr;

    fn extract(obj: pyo3::Borrowed<'_, 'py, PyAny>) -> Result<Self, Self::Error> {
        if let Ok(value) = obj.extract::<String>() {
            return Ok(Self(vec![value]));
        }
        Ok(Self(obj.extract::<Vec<String>>()?))
    }
}

include!("_generated/enums_generated.rs");
