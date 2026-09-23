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
    /// Borrow the normalized string.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume into the owned string.
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

/// Narrow a `%f` microsecond fraction to the milliseconds the wire carries.
///
/// Truncates rather than rounds: the value names an instant, and rounding
/// `16:00:00.9996` up to `16:00:01.000` asks for a different second than the
/// caller did.
///
/// Anything without a six-digit fraction passes through, so a caller's own
/// string is never rewritten.
fn microseconds_to_milliseconds(rendered: &str) -> String {
    match rendered.split_once('.') {
        Some((head, frac)) if frac.len() >= 3 && frac.bytes().all(|b| b.is_ascii_digit()) => {
            format!("{head}.{}", &frac[..3])
        }
        _ => rendered.to_string(),
    }
}

/// A time-valued endpoint argument: accepts an `HH:MM:SS[.mmm]` `str`, or a
/// `time` / `datetime` object.
///
/// `strftime("%H:%M:%S")` would drop sub-second precision, and the
/// at-time endpoints answer with the tick nearest the requested instant, so a
/// second-aligned request returns a different row from the one the caller
/// asked for. `time(9, 30, 0, 123_000)` and the string `"09:30:00.123"` are
/// the same instant and used to reach the server as different ones.
///
/// The fraction is milliseconds, not the six digits `%f` emits: the core
/// accepts one, two or three fractional digits and passes anything longer
/// through untouched, so emitting microseconds would hand the server a string
/// it was never given a rule for.
#[derive(Clone)]
pub(crate) struct PyTimeArg(String);

impl PyTimeArg {
    /// Borrow the normalized string.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume into the owned string.
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
        let formatted = obj.call_method1("strftime", ("%H:%M:%S.%f",))?;
        Ok(Self(microseconds_to_milliseconds(
            &formatted.extract::<String>()?,
        )))
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

#[cfg(test)]
mod time_arg_tests {
    use super::microseconds_to_milliseconds;

    /// Python renders a `time` with `%f`, which is microseconds. The wire
    /// carries milliseconds and the core parses one, two or three fractional
    /// digits, passing anything longer through untouched, so six digits would
    /// reach the server as a string it has no rule for.
    #[test]
    fn a_microsecond_fraction_narrows_to_milliseconds() {
        assert_eq!(
            microseconds_to_milliseconds("09:30:00.123000"),
            "09:30:00.123"
        );
        assert_eq!(
            microseconds_to_milliseconds("09:30:00.000000"),
            "09:30:00.000"
        );
    }

    /// Truncation, not rounding. The value names an instant: rounding up asks
    /// for a later second than the caller did, and an at-time query answers
    /// with the tick nearest the instant it was given.
    #[test]
    fn a_fraction_truncates_rather_than_rounds() {
        assert_eq!(
            microseconds_to_milliseconds("16:00:00.999999"),
            "16:00:00.999"
        );
        assert_eq!(
            microseconds_to_milliseconds("16:00:00.999600"),
            "16:00:00.999"
        );
    }

    /// A caller's own string is never rewritten, whatever shape it is in.
    #[test]
    fn a_string_the_caller_supplied_passes_through() {
        for given in [
            "09:30:00",
            "09:30:00.1",
            "09:30:00.12",
            "09:30:00.123",
            "34200000",
        ] {
            assert_eq!(microseconds_to_milliseconds(given), given, "{given}");
        }
    }
}
