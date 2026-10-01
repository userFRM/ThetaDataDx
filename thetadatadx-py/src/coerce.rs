//! Argument-coercion newtypes for the generated endpoint bindings.
//!
//! Each newtype implements `FromPyObject` to accept the natural Python
//! shapes for a wire parameter (a bare string, an enum with a `.value`,
//! a `date` / `time` object) and normalizes them to the single string
//! form the Rust core expects. Centralizing the coercion here keeps the
//! generated method signatures free of per-argument extraction logic.

use pyo3::exceptions::{PyTypeError, PyValueError};
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
/// `date`/`datetime` object (formatted via `strftime("%Y%m%d")`). A naive
/// value is read as an Eastern Time calendar day; an aware `datetime` is first
/// converted to the Eastern day its instant falls on.
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
        if let Some((date, _)) = aware_to_eastern(&obj)? {
            return Ok(Self(date.to_string()));
        }
        let formatted = obj.call_method1("strftime", ("%Y%m%d",))?;
        Ok(Self(formatted.extract::<String>()?))
    }
}

/// The Eastern Time `(YYYYMMDD, milliseconds of day)` an aware `datetime`
/// names, or `None` when `obj` carries no UTC offset (a naive `datetime`, a
/// naive `time` or a `date`).
///
/// The server reads every date and time argument as Eastern wall-clock, and
/// `strftime` renders only an object's own wall-clock fields, so an aware
/// value in another zone would reach it hours away from the instant it names.
/// The instant is taken in UTC and converted with the SDK's own Eastern rules,
/// so the result does not depend on the host having a timezone database.
///
/// # Errors
///
/// A `time` with a `tzinfo` is refused: without a date, its distance from
/// Eastern Time is unknown across a daylight-saving change. An instant outside
/// the supported calendar range is refused too.
fn aware_to_eastern(obj: &Bound<'_, PyAny>) -> PyResult<Option<(i32, i32)>> {
    let Ok(tzinfo) = obj.getattr("tzinfo") else {
        return Ok(None);
    };
    if tzinfo.is_none() {
        return Ok(None);
    }
    if !obj.hasattr("date")? {
        return Err(PyValueError::new_err(
            "a time with a tzinfo has no date to place it in Eastern Time; pass a naive \
             Eastern wall-clock time or an aware datetime",
        ));
    }
    if obj.call_method0("utcoffset")?.is_none() {
        return Ok(None);
    }
    let utc = obj
        .py()
        .import("datetime")?
        .getattr("timezone")?
        .getattr("utc")?;
    let utc = obj.call_method1("astimezone", (utc,))?;
    let field = |name: &str| -> PyResult<i64> { utc.getattr(name)?.extract() };
    let days = thetadatadx::time::civil_to_epoch_days(
        utc.getattr("year")?.extract()?,
        utc.getattr("month")?.extract()?,
        utc.getattr("day")?.extract()?,
    );
    let seconds = (field("hour")? * 60 + field("minute")?) * 60 + field("second")?;
    let epoch_ms = days * 86_400_000 + seconds * 1_000 + field("microsecond")? / 1_000;
    let Some(epoch_ms) = u64::try_from(epoch_ms)
        .ok()
        .filter(|&ms| thetadatadx::time::epoch_ms_in_range(ms))
    else {
        return Err(PyValueError::new_err(format!(
            "{} is outside the supported date range",
            obj.repr()?
        )));
    };
    Ok(Some((
        thetadatadx::time::timestamp_to_date(epoch_ms),
        thetadatadx::time::timestamp_to_ms_of_day(epoch_ms),
    )))
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
///
/// A naive value is read as Eastern wall-clock time; an aware `datetime` is
/// first converted to the Eastern time of its instant.
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
        if let Some((_, ms)) = aware_to_eastern(&obj)? {
            return Ok(Self(format!(
                "{:02}:{:02}:{:02}.{:03}",
                ms / 3_600_000,
                ms / 60_000 % 60,
                ms / 1_000 % 60,
                ms % 1_000
            )));
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
    use super::{microseconds_to_milliseconds, PyDateArg, PyTimeArg};
    use pyo3::prelude::*;

    /// Aware values name an instant and reach the server as the Eastern
    /// wall-clock of that instant; naive values are already Eastern and pass
    /// through. Each row is `(Python expression, date, time)`.
    #[test]
    fn aware_datetimes_convert_to_eastern_time() {
        Python::initialize();
        Python::attach(|py| {
            let scope = pyo3::types::PyDict::new(py);
            py.run(
                c"from datetime import datetime, time, timedelta, timezone",
                Some(&scope),
                None,
            )
            .unwrap();
            for (expr, date, time) in [
                // 13:30 UTC is 09:30 EDT.
                (
                    c"datetime(2024, 3, 15, 13, 30, 0, 123456, tzinfo=timezone.utc)",
                    "20240315",
                    "09:30:00.123",
                ),
                // 01:00 UTC on the 16th is still the 15th in New York (EDT).
                (
                    c"datetime(2024, 3, 16, 1, 0, tzinfo=timezone.utc)",
                    "20240315",
                    "21:00:00.000",
                ),
                // Standard time: 16:00 at UTC+1 is 10:00 EST.
                (
                    c"datetime(2024, 1, 10, 16, 0, tzinfo=timezone(timedelta(hours=1)))",
                    "20240110",
                    "10:00:00.000",
                ),
                (c"datetime(2024, 3, 15, 13, 30)", "20240315", "13:30:00.000"),
            ] {
                let value = py.eval(expr, Some(&scope), None).unwrap();
                assert_eq!(
                    value.extract::<PyDateArg>().unwrap().as_str(),
                    date,
                    "{expr:?}"
                );
                assert_eq!(
                    value.extract::<PyTimeArg>().unwrap().as_str(),
                    time,
                    "{expr:?}"
                );
            }
            let aware_time = py
                .eval(c"time(9, 30, tzinfo=timezone.utc)", Some(&scope), None)
                .unwrap();
            let err = aware_time.extract::<PyTimeArg>().err().expect("refused");
            assert!(err.is_instance_of::<pyo3::exceptions::PyValueError>(py));
        });
    }

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
