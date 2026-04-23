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

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[pyclass(module = "thetadatadx", frozen, eq, hash, skip_from_py_object)]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub(crate) struct $name {
            value: &'static str,
        }

        #[pymethods]
        impl $name {
            $(
                #[classattr]
                const $variant: Self = Self { value: $value };
            )+

            #[getter]
            fn value(&self) -> &'static str {
                self.value
            }

            fn __str__(&self) -> &'static str {
                self.value
            }

            fn __repr__(&self) -> String {
                format!("{}.{}", stringify!($name), self.value)
            }
        }
    };
}

string_enum!(Right {
    CALL => "call",
    PUT => "put",
    BOTH => "both",
});

string_enum!(Venue {
    NQB => "nqb",
    UTP_CTA => "utp_cta",
});

string_enum!(Interval {
    TICK => "tick",
    MS_10 => "10ms",
    MS_100 => "100ms",
    MS_500 => "500ms",
    S_1 => "1s",
    S_5 => "5s",
    S_10 => "10s",
    S_15 => "15s",
    S_30 => "30s",
    M_1 => "1m",
    M_5 => "5m",
    M_10 => "10m",
    M_15 => "15m",
    M_30 => "30m",
    H_1 => "1h",
});

string_enum!(RateType {
    SOFR => "sofr",
    TREASURY_M1 => "treasury_m1",
    TREASURY_M3 => "treasury_m3",
    TREASURY_M6 => "treasury_m6",
    TREASURY_Y1 => "treasury_y1",
    TREASURY_Y2 => "treasury_y2",
    TREASURY_Y3 => "treasury_y3",
    TREASURY_Y5 => "treasury_y5",
    TREASURY_Y7 => "treasury_y7",
    TREASURY_Y10 => "treasury_y10",
    TREASURY_Y20 => "treasury_y20",
    TREASURY_Y30 => "treasury_y30",
});

string_enum!(RequestType {
    TRADE => "trade",
    QUOTE => "quote",
    EOD => "eod",
    OHLC => "ohlc",
});

string_enum!(Version {
    LATEST => "latest",
    V1 => "1",
});

pub(crate) fn register_string_enums(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Right>()?;
    m.add_class::<Venue>()?;
    m.add_class::<Interval>()?;
    m.add_class::<RateType>()?;
    m.add_class::<RequestType>()?;
    m.add_class::<Version>()?;
    Ok(())
}
