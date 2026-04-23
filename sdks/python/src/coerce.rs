use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;

#[derive(Clone)]
pub(crate) struct PyStringArg(String);

impl PyStringArg {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

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

#[derive(Clone)]
pub(crate) struct PyDateArg(String);

impl PyDateArg {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

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

#[derive(Clone)]
pub(crate) struct PyTimeArg(String);

impl PyTimeArg {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

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

#[derive(Clone)]
pub(crate) struct PySymbols(Vec<String>);

impl PySymbols {
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, String> {
        self.0.iter()
    }

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

include!("enums_generated.rs");
