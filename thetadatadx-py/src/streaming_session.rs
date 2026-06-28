//! Hand-written Python context manager that mirrors the C++ RAII
//! lifecycle for streaming.
//!
//! `with client.streaming(callback) as session:` enters by calling
//! `start_streaming(callback)` and exits by calling `stop_streaming()`
//! followed by `await_drain(5_000)`. The drain barrier matches the
//! C ABI / C++ wrapper contract: by the time control returns to the
//! caller the consumer thread has finished firing the callback, so
//! the closure stack the callback closed over can be released without
//! a use-after-free race against the LMAX Disruptor consumer.
//!
//! SSOT: every public method on `Client` is reachable on the
//! `StreamingSession` by virtue of `__getattr__` proxying. There is
//! NO hand-listed mirror of `subscribe_*` / `unsubscribe_*` /
//! `active_subscriptions` here -- adding a new public method to
//! `Client` automatically makes it callable through the session,
//! with zero drift between the wrapper and the wrapped surface.

use pyo3::exceptions::PyRuntimeWarning;
use pyo3::prelude::*;

use crate::fpss_client::StreamingClient;

/// Drain timeout applied on `__exit__`. Matches the C++ destructor's
/// 5 s budget in `thetadatadx-cpp/src/thetadatadx.cpp` and the FFI free-path
/// budget in `thetadatadx-ffi/src/streaming.rs::FREE_DRAIN_TIMEOUT`. Cross-binding
/// parity matters more than tunability here -- a slow Python callback
/// that needs >5 s to drain is already a contract violation worth
/// surfacing.
const EXIT_DRAIN_TIMEOUT_MS: u64 = 5_000;

/// Typed handle carried by the context-manager pyclasses. Replaces a
/// bare `Py<PyAny>` so the streaming lifecycle calls
/// (`start_streaming` / `stop_streaming` / `await_drain`) dispatch
/// through a closed sum of the two supported pyclasses rather than
/// duck-typed Python attribute lookup. The fluent `__getattr__` proxy
/// for non-lifecycle attributes still goes through PyAny — `subscribe`
/// and the historical surface live on `Client` only, so the
/// proxy carries that asymmetry rather than enumerating it here.
pub(crate) enum StreamableHandle {
    Unified(Py<crate::Client>),
    Fpss(Py<StreamingClient>),
}

impl StreamableHandle {
    /// Bind the inner pyclass as a `Bound<PyAny>` for fluent
    /// `__getattr__` forwarding of non-lifecycle attributes.
    pub(crate) fn bind_any<'py>(&'py self, py: Python<'py>) -> Bound<'py, PyAny> {
        match self {
            Self::Unified(handle) => handle.bind(py).clone().into_any(),
            Self::Fpss(handle) => handle.bind(py).clone().into_any(),
        }
    }

    /// Invoke `start_streaming(callback)` through the typed enum. The
    /// unified-client streaming lifecycle lives on the `client.stream`
    /// `StreamView` surface, so the `Unified` arm dispatches through it.
    pub(crate) fn start_streaming(&self, py: Python<'_>, callback: Py<PyAny>) -> PyResult<()> {
        match self {
            Self::Unified(handle) => handle.borrow(py).stream().start_streaming(py, callback),
            Self::Fpss(handle) => handle.borrow(py).start_streaming(py, callback),
        }
    }

    /// Invoke `stop_streaming()` through the typed enum.
    pub(crate) fn stop_streaming(&self, py: Python<'_>) {
        match self {
            Self::Unified(handle) => handle.borrow(py).stream().stop_streaming(py),
            Self::Fpss(handle) => handle.borrow(py).stop_streaming(py),
        }
    }

    /// Invoke `await_drain(timeout_ms)` through the typed enum. Both
    /// pyclasses release the GIL internally before polling, so the
    /// PyO3 dispatcher is the only frame holding the GIL during the
    /// wait.
    pub(crate) fn await_drain(&self, py: Python<'_>, timeout_ms: u64) -> bool {
        match self {
            Self::Unified(handle) => handle.borrow(py).stream().await_drain(py, timeout_ms),
            Self::Fpss(handle) => handle.borrow(py).await_drain(py, timeout_ms),
        }
    }
}

/// Context manager returned by `Client.streaming(callback)`.
///
/// Holds a strong reference to the underlying streaming pyclass (either
/// `Client` or the standalone `StreamingClient`) plus the user
/// callback. `__enter__` registers the callback via `start_streaming`,
/// `__exit__` calls `stop_streaming` + `await_drain`. Every other
/// method call is forwarded through `__getattr__` to the wrapped
/// pyclass instance.
#[pyclass(module = "thetadatadx", name = "StreamingSession")]
pub(crate) struct StreamingSession {
    /// Typed handle to the streaming pyclass. Closed sum of the two
    /// transports the session knows how to drive, so the lifecycle path
    /// compiles only against pyclasses that actually implement it. The
    /// non-lifecycle `__getattr__` proxy still erases the type for
    /// downstream attribute lookup (e.g. `subscribe` / historical
    /// methods).
    pub(crate) client: StreamableHandle,
    pub(crate) callback: Option<Py<PyAny>>,
}

#[pymethods]
impl StreamingSession {
    /// Register the stored callback via the typed `StreamableHandle`
    /// dispatch. Returns `self` so users access subscribe/unsubscribe
    /// methods through the session (which proxies via `__getattr__`).
    fn __enter__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<PyRef<'py, Self>> {
        let callback = slf.callback.as_ref().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err(
                "StreamingSession callback already consumed -- one session enters at most once",
            )
        })?;
        let cb = callback.clone_ref(py);
        slf.client.start_streaming(py, cb)?;
        Ok(slf)
    }

    /// Stop streaming + block on the drain barrier so the consumer
    /// thread is guaranteed to have finished firing the registered
    /// callback before this returns. Returns `False` so the `with`
    /// block does NOT swallow exceptions raised inside the body.
    #[pyo3(signature = (exc_type=None, exc_value=None, traceback=None))]
    fn __exit__(
        &mut self,
        py: Python<'_>,
        exc_type: Option<Py<PyAny>>,
        exc_value: Option<Py<PyAny>>,
        traceback: Option<Py<PyAny>>,
    ) -> PyResult<bool> {
        // The three exception args are part of the context-manager
        // protocol; we don't inspect them but accept them unconditionally
        // so Python's `with` machinery can pass `None` triplets.
        let _ = (exc_type, exc_value, traceback);

        self.client.stop_streaming(py);
        // `await_drain` releases the GIL internally (see the
        // generated `streaming_methods.rs` and the StreamingClient
        // hand-written equivalent), so the Disruptor consumer can
        // acquire the GIL to finish firing any in-flight callback
        // before flipping the drain bit.
        let drained = self.client.await_drain(py, EXIT_DRAIN_TIMEOUT_MS);
        // Drop the stored callback now that the consumer is quiesced.
        // Holding it longer would leak a Python reference until the
        // session itself is collected.
        self.callback = None;
        if !drained {
            // RuntimeWarning rather than a hard exception: the streaming
            // pipeline is already torn down (`stop_streaming` ran), and
            // the drain is best-effort observability. Re-raising here
            // would swallow any exception from the `with` body, which
            // breaks the standard context-manager contract.
            let warnings = py.import("warnings")?;
            let msg = format!(
                "streaming drain timed out after {EXIT_DRAIN_TIMEOUT_MS}ms; \
                 consumer callback may still be firing."
            );
            // `warnings.warn(msg, RuntimeWarning, stacklevel=2)` so the
            // warning point-of-blame is the caller's `with` exit, not
            // this Rust frame.
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("stacklevel", 2_u32)?;
            warnings.call_method(
                "warn",
                (msg, py.get_type::<PyRuntimeWarning>()),
                Some(&kwargs),
            )?;
        }
        // Returning `false` from `__exit__` tells the Python `with`
        // protocol NOT to swallow exceptions raised inside the body.
        Ok(false)
    }

    /// Forward unknown attribute access to the wrapped streaming
    /// pyclass.
    ///
    /// This is the SSOT proxy: every public method on the underlying
    /// pyclass (`subscribe(sub)` / `subscribe_many([...])` /
    /// `unsubscribe(sub)` / `unsubscribe_many([...])`,
    /// `active_subscriptions`, `dropped_event_count`, `reconnect`, …)
    /// is reachable on the session without duplication here. Adding a
    /// new method to the wrapped pyclass makes it callable through
    /// the session automatically — zero drift surface.
    ///
    /// PyO3 calls `__getattr__` only after the C-level attribute
    /// lookup fails, so `__enter__` / `__exit__` / `client` / `callback`
    /// defined on this class take precedence and never reach this
    /// proxy.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let bound = self.client.bind_any(py);
        // The unified client's subscription / diagnostic surface moved onto
        // the `client.stream` `StreamView`, so the `Unified` session arm resolves
        // a name there first (e.g. `session.subscribe(...)`,
        // `session.active_subscriptions`, `session.active_full_subscriptions`,
        // `session.panic_count`) before falling back to the methods that stay
        // on `Client` (`session_uuid`, `subscription_info`). The standalone
        // `StreamingClient` arm keeps its flat surface and has no `stream`
        // accessor, so the fallback path handles it unchanged.
        if let Ok(stream) = bound.getattr("stream") {
            if let Ok(attr) = stream.getattr(name) {
                return Ok(attr.unbind());
            }
        }
        Ok(bound.getattr(name)?.unbind())
    }
}

/// Factory method on `Client` -- second `#[pymethods]` impl block
/// enabled by the `multiple-pymethods` PyO3 feature flag (see
/// `Cargo.toml`). The generated `streaming_methods.rs` owns the
/// rest of the streaming surface; the context-manager constructor lives
/// here because it is hand-written and references the hand-written
/// `StreamingSession` pyclass.
#[pymethods]
impl crate::Client {
    /// Open a context-managed streaming session.
    ///
    /// `with client.streaming(callback) as session:` registers `callback`
    /// via `start_streaming` on enter and pairs `stop_streaming()` +
    /// `await_drain(5_000)` on exit, mirroring the C++ RAII destructor
    /// in `thetadatadx-cpp/src/thetadatadx.cpp`. Subscription methods on the bound
    /// `session` forward to the underlying `Client` via
    /// `StreamingSession.__getattr__`, so the public surface stays a
    /// single source of truth rooted in the wrapped class.
    ///
    /// If the drain barrier times out (5000 ms), a `RuntimeWarning`
    /// fires but the `with` block exits normally. A timeout means the
    /// consumer thread is still firing the registered callback; the
    /// callback closure remains referenced by the consumer until it
    /// finishes.
    fn streaming(
        slf: Py<Self>,
        py: Python<'_>,
        callback: Py<PyAny>,
    ) -> PyResult<Py<StreamingSession>> {
        Py::new(
            py,
            StreamingSession {
                client: StreamableHandle::Unified(slf),
                callback: Some(callback),
            },
        )
    }

    /// Current historical session UUID. Reads through the shared session
    /// token so the returned value reflects any mid-session refresh.
    ///
    /// Backs the `session_uuid` entry on `AsyncClient`'s
    /// `__getattr__` allowlist so that proxy resolves to a working call.
    fn session_uuid(&self, py: Python<'_>) -> pyo3::PyResult<String> {
        let inner = self.client.clone();
        crate::run_blocking(py, async move { Ok(inner.session_uuid().await) })
    }

    /// Subscription-tier snapshot captured at authentication time.
    ///
    /// Returns one `(asset_class, tier)` tuple per asset class the
    /// Nexus auth payload carries, in stable declaration order:
    /// `stock`, `options`, `indices`, `interest_rate`. Missing fields
    /// surface as the string `"Unknown"`. Returning an ordered list
    /// (rather than a `dict`) pins iteration order across binding
    /// versions and across Python implementations — `HashMap` is only
    /// insertion-ordered by accident in CPython 3.7+, and that
    /// accident has been observably wrong on PyPy in the past.
    ///
    /// Mirrors the upstream
    /// [`thetadatadx::Client::subscription_info`] shape.
    fn subscription_info(&self) -> Vec<(String, String)> {
        let info = self.client.subscription_info();
        vec![
            ("stock".to_string(), info.stock),
            ("options".to_string(), info.options),
            ("indices".to_string(), info.indices),
            ("interest_rate".to_string(), info.interest_rate),
        ]
    }
}
