//! Cross-language utility helpers — Python bindings.
//!
//! Wraps the lookup tables in `thetadatadx::utils::{conditions, exchange, sequences}`
//! and exposes them under the `thetadatadx.util` Python submodule:
//!
//! ```python
//! import thetadatadx.util as util
//! util.condition_name(0)            # "REGULAR"
//! util.exchange_name(3)             # "NewYorkStockExchange"
//! util.exchange_symbol(3)           # "NYSE"
//! util.calendar_status_name(1)      # "early_close"
//! util.timestamp_ms(20240102, 34200000)  # epoch ms, or None if out of domain
//! util.sequence_signed_to_unsigned(-1)
//! ```
//!
//! Hand-written rather than codegen'd. The function set is finite and
//! the codegen pipeline targets dynamic-schema endpoints.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

#[pyfunction]
fn condition_name(code: i32) -> &'static str {
    thetadatadx::utils::conditions::condition_name(code)
}

#[pyfunction]
fn condition_description(code: i32) -> &'static str {
    thetadatadx::utils::conditions::condition_description(code)
}

#[pyfunction]
fn is_cancel(code: i32) -> bool {
    thetadatadx::utils::conditions::is_cancel(code)
}

#[pyfunction]
fn updates_volume(code: i32) -> bool {
    thetadatadx::utils::conditions::updates_volume(code)
}

#[pyfunction]
fn quote_condition_name(code: i32) -> &'static str {
    thetadatadx::utils::conditions::quote_condition_name(code)
}

#[pyfunction]
fn quote_condition_description(code: i32) -> &'static str {
    thetadatadx::utils::conditions::quote_condition_description(code)
}

#[pyfunction]
fn is_firm(code: i32) -> bool {
    thetadatadx::utils::conditions::is_firm(code)
}

#[pyfunction]
fn is_halted(code: i32) -> bool {
    thetadatadx::utils::conditions::is_halted(code)
}

#[pyfunction]
fn exchange_name(code: i32) -> &'static str {
    thetadatadx::utils::exchange::exchange_name(code)
}

/// Vendor vocabulary text for a calendar-day `status` code (`0` ->
/// `"open"`, `1` -> `"early_close"`, `2` -> `"full_close"`, `3` ->
/// `"weekend"`). Returns the literal `"UNKNOWN"` for codes outside the
/// table. Mirrors the C++ `thetadatadx::calendar_status_name` and the C ABI
/// `thetadatadx_calendar_status_name`.
#[pyfunction]
fn calendar_status_name(code: i32) -> &'static str {
    thetadatadx::CalendarStatus::from_code(code)
        .map_or("UNKNOWN", thetadatadx::CalendarStatus::as_str)
}

/// Combine an Eastern-Time `YYYYMMDD` date and milliseconds-of-day into
/// Unix epoch milliseconds (UTC, DST-aware). Usable with any
/// `(date, *_ms_of_day)` pair on the tick structs. Returns `None` when
/// `date` is absent (`0`) or either input is out of domain — the same
/// `std::nullopt` contract the C++ `thetadatadx::timestamp_ms` returns (the C
/// ABI `thetadatadx_timestamp_ms` encodes that absence as the `-1` sentinel).
#[pyfunction]
fn timestamp_ms(date: i32, ms_of_day: i32) -> Option<i64> {
    thetadatadx::time::date_ms_to_epoch_ms(date, ms_of_day)
}

#[pyfunction]
fn exchange_symbol(code: i32) -> &'static str {
    thetadatadx::utils::exchange::exchange_symbol(code)
}

/// Convert a signed wire-encoded trade-sequence value to its unsigned
/// monotonic form. `signed_value` must lie in the i32 wire range
/// (`-2_147_483_648 ..= 2_147_483_647`): the upstream terminal encodes
/// trade sequences as i32, so a value outside that domain is not a wire
/// sequence and is rejected with `ValueError` rather than silently
/// reinterpreted into a look-correct-but-wrong id. A value that does not
/// fit the `i64` parameter type still surfaces as the built-in
/// `OverflowError` from argument coercion, unchanged.
#[pyfunction]
fn sequence_signed_to_unsigned(signed_value: i64) -> PyResult<u64> {
    if !(thetadatadx::utils::sequences::SEQUENCE_MIN..=thetadatadx::utils::sequences::SEQUENCE_MAX)
        .contains(&signed_value)
    {
        return Err(PyValueError::new_err(format!(
            "sequence_signed_to_unsigned: {signed_value} is outside the i32 wire range \
             (-2_147_483_648 ..= 2_147_483_647)"
        )));
    }
    Ok(thetadatadx::utils::sequences::signed_to_unsigned(
        signed_value,
    ))
}

/// Convert an unsigned monotonic trade-sequence value back to its signed
/// wire encoding. `unsigned_value` must lie in the unsigned wire range
/// (`0 ..= 2^32 - 1`): the monotonic sequence id is never wider than one
/// i32 cycle, so a value above that domain is rejected with `ValueError`
/// rather than silently reinterpreted. A negative argument still
/// surfaces as the built-in `OverflowError` from `u64` coercion,
/// unchanged.
#[pyfunction]
fn sequence_unsigned_to_signed(unsigned_value: u64) -> PyResult<i64> {
    if unsigned_value > u64::from(u32::MAX) {
        return Err(PyValueError::new_err(format!(
            "sequence_unsigned_to_signed: {unsigned_value} is above the unsigned wire range \
             (0 ..= 2^32 - 1)"
        )));
    }
    Ok(thetadatadx::utils::sequences::unsigned_to_signed(
        unsigned_value,
    ))
}

/// Register the `thetadatadx.util` submodule on the parent module.
///
/// All functions are added to a child PyModule named `util`, then that
/// child is registered both as a submodule of the parent and (so
/// `import thetadatadx.util` works) inserted into `sys.modules` under
/// the dotted name. This is the standard pyo3 idiom for native Python
/// submodules.
pub(crate) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = parent.py();
    let util = PyModule::new(py, "util")?;
    util.add_function(wrap_pyfunction!(condition_name, &util)?)?;
    util.add_function(wrap_pyfunction!(condition_description, &util)?)?;
    util.add_function(wrap_pyfunction!(is_cancel, &util)?)?;
    util.add_function(wrap_pyfunction!(updates_volume, &util)?)?;
    util.add_function(wrap_pyfunction!(quote_condition_name, &util)?)?;
    util.add_function(wrap_pyfunction!(quote_condition_description, &util)?)?;
    util.add_function(wrap_pyfunction!(is_firm, &util)?)?;
    util.add_function(wrap_pyfunction!(is_halted, &util)?)?;
    util.add_function(wrap_pyfunction!(exchange_name, &util)?)?;
    util.add_function(wrap_pyfunction!(exchange_symbol, &util)?)?;
    util.add_function(wrap_pyfunction!(calendar_status_name, &util)?)?;
    util.add_function(wrap_pyfunction!(timestamp_ms, &util)?)?;
    util.add_function(wrap_pyfunction!(sequence_signed_to_unsigned, &util)?)?;
    util.add_function(wrap_pyfunction!(sequence_unsigned_to_signed, &util)?)?;

    // Insert under the dotted name so `import thetadatadx.util` works
    // identically to a pure-Python submodule.
    let sys_modules = py.import("sys")?.getattr("modules")?;
    sys_modules.set_item("thetadatadx.util", &util)?;

    parent.add_submodule(&util)?;
    Ok(())
}
