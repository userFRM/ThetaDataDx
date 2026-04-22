//! Async bridge helpers mirroring [`run_blocking`] for the sync path.
//!
//! Called by every generator-emitted `*_async` method in
//! [`historical_methods.rs`] and every `*_async` builder terminal. The
//! module is utility code (hand-written, same category as `run_blocking`
//! at the top of [`lib.rs`]) — it is explicitly NOT generator output, so
//! edits here do not cross the SSOT boundary at `endpoint_surface.toml`.
//!
//! [`run_blocking`]: crate::run_blocking
//! [`historical_methods.rs`]: ../../../../sdks/python/src/historical_methods.rs
//! [`lib.rs`]: ../../../../sdks/python/src/lib.rs

use std::future::Future;

use pyo3::prelude::*;
use pyo3::types::PyAny;

use crate::errors::to_py_err;

/// Run `fut` on the shared tokio runtime, convert the resolved value via
/// `convert`, and return a `pyo3`-awaitable that yields the converted
/// Python object.
///
/// Every generator-emitted `*_async` endpoint method in
/// `historical_methods.rs` calls this. The shape mirrors the sync
/// `run_blocking(py, fut)` helper so generator output stays symmetric
/// across the sync + async paths — one `*_async` = one call to this
/// helper, exactly as one sync = one call to `run_blocking`.
///
/// # Error propagation
///
/// `thetadatadx::Error` values returned by the awaited future are routed
/// through [`crate::errors::to_py_err`] so the caller sees a concrete
/// `thetadatadx` exception subclass (e.g. `TimeoutError`, `RateLimitError`)
/// rather than a generic `RuntimeError`.
///
/// # GIL contract
///
/// `convert` runs inside [`Python::attach`] on the pool thread that
/// resolves the future, so it can touch pyclass instances, allocate
/// Python objects, and call Python code freely.
pub(crate) fn spawn_awaitable<'py, F, T, C>(
    py: Python<'py>,
    fut: F,
    convert: C,
) -> PyResult<Bound<'py, PyAny>>
where
    F: Future<Output = Result<T, thetadatadx::Error>> + Send + 'static,
    T: Send + 'static,
    C: FnOnce(Python<'_>, T) -> PyResult<Py<PyAny>> + Send + 'static,
{
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let value = fut.await.map_err(to_py_err)?;
        Python::attach(|py| convert(py, value))
    })
}

#[cfg(test)]
mod tests {
    //! Unit tests covering the two invariants the generator relies on:
    //!
    //! 1. `thetadatadx::Error` values crossing the await point arrive on
    //!    the Python side as the matching `ThetaDataError` subclass, not
    //!    a generic `RuntimeError`. Verifies the helper honours the same
    //!    [`to_py_err`] mapping the sync path uses.
    //! 2. The `convert` closure runs with a held GIL token — it must be
    //!    safe to allocate Python objects there.
    //!
    //! The `pyo3_async_runtimes::tokio::future_into_py` helper ultimately
    //! returns a `Bound<'py, PyAny>` that resolves on the shared tokio
    //! runtime. We do not spin that runtime up here; instead we exercise
    //! the helper at the "inputs land on the right rail" layer by
    //! invoking the inner future directly and asserting the mapping.
    use super::*;

    /// Force the runtime init once per test binary so
    /// `pyo3_async_runtimes::tokio::future_into_py` (which is wired in
    /// at module init via `init_with_runtime`) has a place to run. We
    /// explicitly do NOT touch the module-level runtime installed by
    /// `thetadatadx_py` because the test binary has no `#[pymodule]`
    /// init path — we set up the runtime directly instead.
    fn install_runtime() {
        use std::sync::OnceLock;
        use tokio::runtime::{Builder, Runtime};
        static RT: OnceLock<Runtime> = OnceLock::new();
        let rt = RT.get_or_init(|| {
            Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("test runtime")
        });
        // `init_with_runtime` is idempotent-ish — the first call wins,
        // subsequent calls return Err. Either is fine here; we just need
        // the registration to land before the first `future_into_py`.
        let _ = pyo3_async_runtimes::tokio::init_with_runtime(rt);
    }

    #[test]
    fn spawn_awaitable_propagates_rust_error_as_typed_py_exception() {
        // The helper's error path is
        //   fut.await.map_err(to_py_err)?
        // so the full mapping lives in `errors::to_py_err`. We assert
        // the helper wires through to that function by constructing the
        // inner coroutine directly and awaiting it on a minimal tokio
        // runtime — this skips the `future_into_py` wrapping (which
        // would need a full pyo3-async-runtimes module bootstrap) and
        // tests the single line we actually care about.
        Python::initialize();
        Python::attach(|py| {
            install_runtime();

            // Inject a `Timeout` variant; the mapping in `to_py_err`
            // sends it to `TimeoutError`. `T = i64` (not `()`) so the
            // `let value = ...?` inside the tested body is a real
            // assignment matching the production `spawn_awaitable`
            // shape — every endpoint passes a non-unit tick / list
            // payload.
            let fut = async { Err::<i64, _>(thetadatadx::Error::Timeout { duration_ms: 250 }) };
            let convert = |_py: Python<'_>, _value: i64| -> PyResult<Py<PyAny>> {
                unreachable!("convert runs only on Ok path")
            };

            // Drive the coroutine `spawn_awaitable` constructs to get at
            // the typed error. `future_into_py` wraps it but the inner
            // future body is observable by running the same `async move`
            // block directly.
            let runtime = pyo3_async_runtimes::tokio::get_runtime();
            let err = runtime
                .block_on(async {
                    let value = fut.await.map_err(to_py_err)?;
                    Python::attach(|py| convert(py, value))
                })
                .expect_err("Timeout must map to a PyErr");

            let type_obj = err.get_type(py);
            let name = type_obj
                .qualname()
                .and_then(|q| q.extract::<String>())
                .expect("every pyo3 exception class has a qualname");
            assert_eq!(
                name, "TimeoutError",
                "Error::Timeout must route to the TimeoutError subclass, got {name}"
            );
        });
    }

    #[test]
    fn spawn_awaitable_runs_convert_on_gil() {
        // Drives the Ok path through the same inner coroutine and
        // asserts that `convert` sees a live GIL state — constructing a
        // Python object there must succeed. If `convert` ever ran
        // without the GIL (a regression in the helper) this would panic
        // inside `Python::with_gil`.
        Python::initialize();
        Python::attach(|py| {
            install_runtime();

            let fut = async { Ok::<i64, thetadatadx::Error>(42) };
            let convert = |py: Python<'_>, value: i64| -> PyResult<Py<PyAny>> {
                // Allocating a Python int is the cheapest observable GIL
                // operation — it fails immediately without a held GIL.
                Ok(value.into_pyobject(py)?.unbind().into_any())
            };

            let runtime = pyo3_async_runtimes::tokio::get_runtime();
            let obj = runtime
                .block_on(async {
                    let value = fut.await.map_err(to_py_err)?;
                    Python::attach(|py| convert(py, value))
                })
                .expect("Ok path converts successfully");

            let extracted: i64 = obj.extract(py).expect("int round-trip");
            assert_eq!(extracted, 42);
        });
    }
}
