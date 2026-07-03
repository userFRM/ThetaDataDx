//! Async bridge helpers mirroring [`run_blocking`] for the sync path.
//!
//! Called by every generator-emitted `*_async` method in
//! [`historical_methods.rs`] and every `*_async` builder terminal. The
//! module is utility code (hand-written, same category as `run_blocking`
//! at the top of [`lib.rs`]) — it is explicitly NOT generator output, so
//! edits here do not cross the SSOT boundary at `endpoint_surface.toml`.
//!
//! [`run_blocking`]: crate::run_blocking
//! [`historical_methods.rs`]: ../../../../thetadatadx-py/src/_generated/historical_methods.rs
//! [`lib.rs`]: ../../../../thetadatadx-py/src/lib.rs

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
/// # GIL + scheduling contract
///
/// `convert` runs inside [`Python::attach`] on a thread from tokio's
/// blocking pool (via [`tokio::task::spawn_blocking`]), NOT on the
/// runtime worker thread that drove `fut` to completion. Running the
/// convert closure directly in the awaitable body would park the
/// runtime worker under GIL contention for the duration of heavy
/// materialization work (e.g. building a large `QuoteTickList`
/// pyclass) and block every other async task scheduled on the same
/// worker. Routing the conversion to the blocking pool keeps the
/// runtime worker free to service other endpoints while the current
/// call is synthesizing its Python payload.
///
/// `convert` can still touch pyclass instances, allocate Python
/// objects, and call Python code freely — it holds the GIL via the
/// inner `Python::attach`.
///
/// ## Why not return `T: IntoPyObject` (option A from the review)?
///
/// `pyo3_async_runtimes::tokio::future_into_py` already wraps the
/// final `IntoPyObject` materialization in `spawn_blocking` itself
/// (see `pyo3-async-runtimes-0.28.0/src/generic.rs` around line 643),
/// so "return the raw `T` and let the library convert" would also
/// land the work on the blocking pool. The problem is that our
/// convert helpers (`strings_to_string_list`,
/// `trade_ticks_to_pyclass_list`, `quote_ticks_to_pyclass_list`, …)
/// produce typed pyclass wrappers via functions that need the GIL —
/// they aren't plain `IntoPyObject` impls on `Vec<T>`. Refactoring
/// every helper plus the 122 generator-emitted callsites in
/// `historical_methods.rs` and the matching generator templates in
/// `thetadatadx-rs/build_support/endpoints/render/python.rs` is a
/// much larger change with the same final scheduling outcome.
/// `spawn_blocking` resolves the contention with zero ripple to the
/// helper surface and zero generator-template churn.
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
    pyo3_async_runtimes::tokio::future_into_py(py, resolve_then_convert(fut, convert))
}

/// Inner coroutine shared by [`spawn_awaitable`] and its unit tests:
/// await `fut`, then offload `convert` onto tokio's blocking pool.
///
/// Running `convert` on the runtime worker would acquire the GIL there
/// and park the worker for the duration of the Python-object build, so
/// two concurrent `*_async` calls on the same worker would serialize on
/// the GIL even with other workers free — heavy converts (building a
/// large `QuoteTickList` pyclass) are where this matters. `spawn_blocking`
/// confines the GIL contention to the pool thread that actually needs it
/// and returns the worker to its queue the instant the future resolves.
async fn resolve_then_convert<F, T, C>(fut: F, convert: C) -> PyResult<Py<PyAny>>
where
    F: Future<Output = Result<T, thetadatadx::Error>> + Send + 'static,
    T: Send + 'static,
    C: FnOnce(Python<'_>, T) -> PyResult<Py<PyAny>> + Send + 'static,
{
    let value = fut.await.map_err(to_py_err)?;
    tokio::task::spawn_blocking(move || Python::attach(|py| convert(py, value)))
        .await
        .map_err(|join_err| {
            // `JoinError::into_panic()` is the documented way to surface a
            // panicked blocking task; other JoinError causes (cancellation)
            // map to a generic RuntimeError.
            if join_err.is_panic() {
                let payload = join_err.into_panic();
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "convert closure panicked".to_string());
                pyo3::exceptions::PyRuntimeError::new_err(format!(
                    "convert closure panicked: {msg}"
                ))
            } else {
                pyo3::exceptions::PyRuntimeError::new_err(format!(
                    "convert task join failed: {join_err}"
                ))
            }
        })?
}

/// Fold a blocking-task [`JoinError`] into the shared per-chunk
/// callback-error slot so a panicked streaming handler surfaces as a
/// re-raised `PyErr` instead of being swallowed.
///
/// The generated `*_stream_async` terminals run each user handler on a
/// `spawn_blocking` task (the handler holds the GIL, so keeping it off
/// the async workers is what stops one slow handler from starving every
/// other in-flight historical call). A panic inside that task must not
/// vanish: it is captured here as a `RuntimeError` carrying the panic
/// payload and re-raised by the terminal's post-await converter. An
/// already-captured handler `PyErr` is left in place — a panicked task
/// cannot also have captured an error, and clobbering would drop the
/// proximate cause.
///
/// [`JoinError`]: tokio::task::JoinError
pub(crate) fn capture_join_error(
    cb_err: &std::sync::Mutex<Option<PyErr>>,
    join_err: tokio::task::JoinError,
) {
    let msg = if join_err.is_panic() {
        let payload = join_err.into_panic();
        payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "stream handler panicked".to_string())
    } else {
        format!("stream handler task did not complete: {join_err}")
    };
    let mut slot = cb_err.lock().unwrap();
    if slot.is_none() {
        *slot = Some(pyo3::exceptions::PyRuntimeError::new_err(msg));
    }
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
        //
        // Note the `py.detach(|| block_on(...))` envelope: the
        // `spawn_awaitable` body offloads `convert` onto a
        // `spawn_blocking` task that calls `Python::attach` from the
        // blocking-pool thread. If the outer test thread were still
        // holding the GIL when the block_on fires, the blocking-pool
        // thread would wait forever for the GIL — deadlock. Releasing
        // the GIL around `block_on` lets the blocking task acquire it,
        // matching the production flow where the awaitable is polled
        // by the Python event loop (which has already released the
        // GIL to its workers).
        Python::initialize();
        Python::attach(|py| {
            install_runtime();
            let runtime = pyo3_async_runtimes::tokio::get_runtime();
            let err = py
                .detach(|| {
                    runtime.block_on(resolve_then_convert(
                        async { Err::<i64, _>(thetadatadx::Error::Timeout { duration_ms: 250 }) },
                        |_py: Python<'_>, _value: i64| -> PyResult<Py<PyAny>> {
                            unreachable!("convert runs only on Ok path")
                        },
                    ))
                })
                .expect_err("Timeout must map to a PyErr");

            let type_obj = err.get_type(py);
            let name = type_obj
                .qualname()
                .and_then(|q| q.extract::<String>())
                .expect("every pyo3 exception class has a qualname");
            // `DeadlineExceededError` is the canonical deadline leaf;
            // `TimeoutError` is registered as a same-object back-compat
            // alias (`thetadatadx.TimeoutError is DeadlineExceededError`),
            // so the raised class reports the canonical qualname while an
            // `except thetadatadx.TimeoutError` clause still catches it.
            assert_eq!(
                name, "DeadlineExceededError",
                "Error::Timeout must route to the DeadlineExceededError leaf, got {name}"
            );
        });
    }

    #[test]
    fn spawn_awaitable_runs_convert_on_gil() {
        // Drives the Ok path through the same inner coroutine and
        // asserts that `convert` sees a live GIL state — constructing a
        // Python object there must succeed. If `convert` ever ran
        // without the GIL (a regression in the helper) this would panic
        // inside `Python::attach`.
        //
        // See the `detach(|| block_on(...))` note in the sibling error
        // test — the same deadlock-avoidance applies here because
        // `spawn_blocking` will try to acquire the GIL on a different
        // thread.
        Python::initialize();
        Python::attach(|py| {
            install_runtime();
            let runtime = pyo3_async_runtimes::tokio::get_runtime();
            let obj = py
                .detach(|| {
                    runtime.block_on(resolve_then_convert(
                        async { Ok::<i64, thetadatadx::Error>(42) },
                        |py: Python<'_>, value: i64| -> PyResult<Py<PyAny>> {
                            // Allocating a Python int is the cheapest
                            // observable GIL operation — it fails
                            // immediately without a held GIL.
                            Ok(value.into_pyobject(py)?.unbind().into_any())
                        },
                    ))
                })
                .expect("Ok path converts successfully");

            let extracted: i64 = obj.extract(py).expect("int round-trip");
            assert_eq!(extracted, 42);
        });
    }

    /// Concurrency regression test: two `spawn_awaitable` calls with
    /// slow convert closures must overlap in wall-clock time rather
    /// than serialize. Running convert on the runtime worker would park
    /// it under GIL for the duration of the conversion, so two
    /// concurrent calls on the same worker would serialize end-to-end.
    /// Running convert on the blocking pool keeps the runtime worker
    /// free to drive both futures concurrently, so their wall-clock
    /// durations overlap.
    ///
    /// We measure overlap by the standard "total time < sum of
    /// individual times" test: if two 150ms tasks run in parallel the
    /// combined wall-clock is close to 150ms, not 300ms. Use 1.5x
    /// single-task time as the ceiling — generous enough to absorb
    /// scheduler jitter on a loaded CI box while still catching a full
    /// serialize regression.
    #[test]
    fn concurrent_spawn_awaitable_calls_overlap_in_wall_time() {
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        Python::initialize();
        Python::attach(|py| {
            install_runtime();

            // Per-task convert cost. Pick 100ms to dwarf tokio scheduler
            // jitter; the assertion only needs the difference between
            // serial (2 * DELAY) and parallel (~ 1 * DELAY) to be
            // unambiguous.
            const DELAY: Duration = Duration::from_millis(100);

            let runtime = pyo3_async_runtimes::tokio::get_runtime();
            let observed = Arc::new(Mutex::new(Vec::<Duration>::new()));

            let spawn_one = |tag: i64, recorder: Arc<Mutex<Vec<Duration>>>| {
                resolve_then_convert(
                    async move { Ok::<i64, thetadatadx::Error>(tag) },
                    move |py: Python<'_>, value: i64| -> PyResult<Py<PyAny>> {
                        // Heavy convert synthesised via a blocking
                        // sleep + a Python-object allocation, matching
                        // the shape of the real helpers
                        // (`quote_ticks_to_pyclass_list` et al.) that
                        // spend most of their time building pyclass
                        // instances under the GIL. `detach` inside
                        // the blocking section is what lets the other
                        // concurrent convert make progress — with the
                        // GIL released, two blocking-pool threads can
                        // run their sleeps in parallel. Without the
                        // detach they would serialize on the GIL,
                        // which is exactly the contention that running
                        // convert on the runtime worker would produce.
                        let t0 = std::time::Instant::now();
                        py.detach(|| std::thread::sleep(DELAY));
                        let obj = value.into_pyobject(py)?.unbind().into_any();
                        recorder.lock().unwrap().push(t0.elapsed());
                        Ok(obj)
                    },
                )
            };

            // Drop the GIL around `block_on` so the `spawn_blocking`
            // tasks inside `resolve_then_convert` can acquire it
            // — see the sibling-test comment for the full explanation.
            let (elapsed, (a, b)) = py.detach(|| {
                let rec_a = observed.clone();
                let rec_b = observed.clone();
                let start = std::time::Instant::now();
                let results = runtime
                    .block_on(async { tokio::join!(spawn_one(1, rec_a), spawn_one(2, rec_b)) });
                (start.elapsed(), results)
            });
            a.expect("first call ok");
            b.expect("second call ok");

            // Sanity: each convert closure ran for at least DELAY.
            let per_task_times = observed.lock().unwrap().clone();
            assert_eq!(per_task_times.len(), 2, "both converts should have fired");
            for d in &per_task_times {
                assert!(
                    *d >= DELAY,
                    "each convert should run for at least {DELAY:?}; got {d:?}"
                );
            }

            // Overlap assertion: parallel execution keeps combined
            // wall-clock below 1.5 * DELAY. Serial execution (convert
            // running on the runtime worker under GIL) would be
            // ~ 2 * DELAY = 200ms. 1.5x = 150ms leaves
            // headroom for scheduler jitter while still catching a
            // full-serialize regression.
            let ceiling = DELAY.mul_f64(1.5);
            assert!(
                elapsed < ceiling,
                "concurrent spawn_awaitable calls must overlap: elapsed {elapsed:?} should be < {ceiling:?} (single-task {DELAY:?}, serial would be ~{:?})",
                DELAY * 2
            );
        });
    }
}
