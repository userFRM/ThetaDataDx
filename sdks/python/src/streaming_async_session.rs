//! Asyncio-native streaming surface for FPSS.
//!
//! Sibling of the sync push-callback `StreamingSession` (event delivery on
//! the Disruptor consumer thread) and the sync pull-iter
//! `StreamingIterSession` (blocking `for event in iter:` drain on the user
//! thread). The async variant bridges the Disruptor consumer thread to
//! Python's asyncio event loop via FD-readiness: every successful
//! `queue.push` on the Rust side writes a coalesced byte to a self-pipe,
//! and the asyncio loop's `add_reader(read_fd, ...)` wakes the awaiting
//! coroutine. No polling, no 100 µs tick budget.
//!
//! The pattern mirrors Bloomberg's BLPAPI `USER_DISPATCH` mode and
//! Refinitiv RTSDK's `DispatchHandler` queue-wake hooks — both expose a
//! file descriptor the host event loop selects on. The Python idiom is
//! the asyncio context-manager + async-iterator pair the bindings in
//! this file implement.
//!
//! # Surface
//!
//! ```python
//! async with client.streaming_async() as session:
//!     await session.subscribe_many([
//!         Contract.stock("QQQ").quote(),
//!         Contract.stock("SPY").trade(),
//!     ])
//!     async for batch in session:
//!         for ev in batch:
//!             handle(ev)
//! ```
//!
//! `batch` is a `list[FpssEvent]` — 1..N events drained per OS wake.
//! Holding the GIL across a batch (instead of per event) is the same
//! throughput optimisation the sync pull-iter path makes; the async
//! path adds FD-readiness signalling on top so quiet periods cost zero
//! CPU.
//!
//! # Backpressure
//!
//! If the asyncio reader falls behind, the bounded Disruptor queue
//! fills. The wake FD coalesces (single byte per pending-not-yet-drained
//! batch) so the producer never blocks on a full pipe. When the queue
//! itself overflows, the consumer increments
//! [`FpssClient::dropped_event_count`] and emits a `tracing::warn!` —
//! identical policy to the sync path.

use std::sync::Arc;

use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};
use pyo3::prelude::*;
use pyo3::types::PyList;

use thetadatadx::fpss::wake::WakeFd;
use thetadatadx::{EventIterator as RustEventIterator, NextEvent};

use crate::buffered_event_to_typed;
use crate::errors::to_py_err;
use crate::fpss_event_to_buffered;

/// Drain timeout applied on `__aexit__`. Same 5 s budget as the sync
/// `StreamingSession` / `StreamingIterSession` so the three context
/// managers' teardown behaviour stays uniform.
const EXIT_DRAIN_TIMEOUT_MS: u64 = 5_000;

/// Typed handle to the underlying streaming client. Mirrors the
/// `StreamableHandle` enum in `streaming_session.rs` but specialised for
/// the async path: subscribe / unsubscribe go through `tdx` core methods
/// directly (no need to traverse the Python attribute proxy), and the
/// wake-fd plumbing requires a method shape (`start_streaming_async`)
/// that the sync surface does not have.
pub(crate) enum AsyncStreamableHandle {
    /// Unified `ThetaDataDxClient` (MDDS + FPSS).
    Tdx(Py<crate::ThetaDataDxClient>),
    /// Standalone FPSS-only client.
    Fpss(Py<crate::fpss_client::FpssClient>),
}

impl AsyncStreamableHandle {
    /// Bind the inner pyclass as a `Bound<PyAny>` for attribute / method
    /// dispatch via the Python proxy. Mirrors the `bind_any` helper on
    /// the sync [`crate::streaming_session::StreamableHandle`] — every
    /// streaming method we forward (subscribe / unsubscribe /
    /// stop_streaming / await_drain) is reachable via the Python
    /// surface on both pyclasses already, so the proxy is the simplest
    /// SSOT path.
    fn bind_any<'py>(&'py self, py: Python<'py>) -> Bound<'py, PyAny> {
        match self {
            Self::Tdx(handle) => handle.bind(py).clone().into_any(),
            Self::Fpss(handle) => handle.bind(py).clone().into_any(),
        }
    }

    /// Open the iterator + wake FD pair on the underlying client.
    /// Returns the Rust iterator and the shared `Arc<WakeFd>` so the
    /// session can `rearm()` from the asyncio reader path. Dispatched
    /// through the closed sum of the two streaming pyclasses — the
    /// Rust-side internal entry points (`start_streaming_async_inner`)
    /// live on inherent impls outside the `#[pymethods]` blocks so the
    /// Rust types (`EventIterator`, `Arc<WakeFd>`) round-trip without
    /// going through Python conversion.
    #[cfg(unix)]
    fn start(
        &self,
        py: Python<'_>,
        write_fd: i32,
    ) -> PyResult<(RustEventIterator, Arc<WakeFd>)> {
        match self {
            Self::Tdx(handle) => handle.borrow(py).start_streaming_async_inner(write_fd),
            Self::Fpss(handle) => handle.borrow(py).start_streaming_async_inner(write_fd),
        }
    }

    /// Proxy `subscribe` / `subscribe_many` / `unsubscribe` /
    /// `unsubscribe_many` through the Python attribute lookup. The
    /// underlying pyclass exposes these via `#[pymethods]` so the
    /// Python method dispatch path is the SSOT — Rust-side direct
    /// calls would require relaxing the methods' visibility to
    /// `pub(crate)`, duplicating the binding's source-of-truth on which
    /// type owns the contract.
    fn call_proxy(
        &self,
        py: Python<'_>,
        name: &str,
        arg: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let bound = self.bind_any(py);
        bound.call_method1(name, (arg.clone(),))?;
        Ok(())
    }

    fn stop_streaming(&self, py: Python<'_>) -> PyResult<()> {
        let bound = self.bind_any(py);
        bound.call_method0("stop_streaming")?;
        Ok(())
    }

    fn await_drain(&self, py: Python<'_>, timeout_ms: u64) -> PyResult<bool> {
        let bound = self.bind_any(py);
        bound.call_method1("await_drain", (timeout_ms,))?.extract()
    }
}

/// Asyncio-native context manager + async iterator for FPSS streaming.
///
/// Acquired via `client.streaming_async()` on either `ThetaDataDxClient`
/// or `FpssClient`. The session takes ownership of:
///
/// * a self-pipe `(read_fd, write_fd)` allocated in `__aenter__`. The
///   read-end is registered on the asyncio loop via `loop.add_reader`;
///   the write-end is wrapped in [`WakeFd`] and handed to the Rust
///   Disruptor consumer thread, which writes a coalesced byte to it on
///   every successful `queue.push`.
/// * a Rust [`EventIterator`] that drains the bounded queue on the
///   reader path. `__anext__` calls `try_next` in a loop until the
///   queue empties or terminal EOF lands.
/// * an `asyncio.Event` set from the `loop.add_reader` callback and
///   awaited inside `__anext__`. Coalesced wakes (the wake FD only
///   writes one byte per pending-not-yet-drained batch) keep the event
///   loop wake-count proportional to the number of batches the reader
///   actually processes, not to the number of events the producer
///   pushes.
#[pyclass(module = "thetadatadx", name = "StreamingAsyncSession", unsendable)]
pub(crate) struct StreamingAsyncSession {
    /// Underlying streaming pyclass. Held across the session lifetime
    /// so subscribe / unsubscribe / stop_streaming go through the same
    /// pyclass instance the caller obtained the session from.
    handle: AsyncStreamableHandle,
    /// Read-end FD. `-1` before `__aenter__` and after `__aexit__`.
    /// Registered on the asyncio loop's `add_reader` so the awaiting
    /// coroutine wakes on every wake-byte the Disruptor consumer
    /// writes.
    read_fd: i32,
    /// Reference to the shared `WakeFd` (write-end ownership lives
    /// inside the Disruptor consumer closure via the
    /// `Delivery::Queue::wake_fd` slot; this `Arc` is the reader-side
    /// clone used to call `rearm()` from the asyncio reader before
    /// draining the pipe). `None` outside the `__aenter__` /
    /// `__aexit__` window.
    wake: Option<Arc<WakeFd>>,
    /// Rust iterator handle. Created in `__aenter__` via
    /// `connect_iter_with_wake_keep_handle`. `None` outside the window.
    iterator: Option<Arc<RustEventIterator>>,
    /// Cached event-loop reference captured in `__aenter__`. Stored as
    /// a `Py<PyAny>` (rather than rebinding on every wake) so
    /// `__aexit__` can call `loop.remove_reader(read_fd)` on the same
    /// loop the reader was installed on, even if the user changed the
    /// running loop between enter and exit.
    event_loop: Option<Py<PyAny>>,
    /// Cached `asyncio.Event` instance the asyncio `add_reader`
    /// callback sets and `__anext__` awaits. Stored on the session so
    /// the same event survives across multiple `__anext__` calls.
    asyncio_event: Option<Py<PyAny>>,
    /// `True` once `__aexit__` has torn the session down. `__anext__`
    /// short-circuits to `StopAsyncIteration` afterward without
    /// touching the (now-closed) read FD or the dropped iterator.
    closed: bool,
}

#[pymethods]
impl StreamingAsyncSession {
    /// Async context-manager entry.
    ///
    /// Allocates a non-blocking self-pipe via `pipe2(O_CLOEXEC |
    /// O_NONBLOCK)`, hands the write-end to the Rust core, registers
    /// the read-end on the running asyncio loop, and creates the
    /// asyncio.Event the reader awaits. Returns `self` so the caller
    /// reaches subscribe / unsubscribe / async-iter methods via the
    /// `async with` binding.
    ///
    /// Returns a coroutine (the Python `async with` protocol expects
    /// `__aenter__` to be awaitable). Inside the coroutine we run on
    /// the calling event loop so `asyncio.get_running_loop()` finds the
    /// right one.
    fn __aenter__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        // Materialise an unbound `Py<Self>` and drop the borrow before
        // entering the async block — the awaitable will reborrow on the
        // event-loop thread.
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let mut session = session_handle.borrow_mut(py);
                session.aenter_inner(py)?;
                Ok::<Py<PyAny>, PyErr>(session_handle.clone_ref(py).into_any())
            })
        })
    }

    /// Async context-manager exit. Mirrors the sync `StreamingSession`
    /// teardown: removes the asyncio reader, closes the read FD, stops
    /// streaming, and awaits the drain barrier (5 s budget). Returns a
    /// coroutine resolving to `False` so exceptions raised inside the
    /// `async with` body propagate.
    #[pyo3(signature = (exc_type=None, exc_value=None, traceback=None))]
    fn __aexit__<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        exc_type: Option<Py<PyAny>>,
        exc_value: Option<Py<PyAny>>,
        traceback: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let _ = (exc_type, exc_value, traceback);
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let mut session = session_handle.borrow_mut(py);
                session.aexit_inner(py)?;
                Ok::<Py<PyAny>, PyErr>(py.None())
            })
        })
    }

    /// `async for batch in session:` — returns self as the async
    /// iterator. Idiomatic Python async-iterator contract.
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Drain the next batch of events.
    ///
    /// Returns a coroutine that:
    ///
    /// 1. checks the iterator's `try_next` once — if events are already
    ///    queued (because the producer pushed between two `__anext__`
    ///    calls and the asyncio reader fired before we awaited), drain
    ///    them immediately without awaiting the asyncio.Event.
    /// 2. otherwise awaits the asyncio.Event the `loop.add_reader`
    ///    callback sets.
    /// 3. once awakened, calls `wake.rearm()` so the next producer
    ///    push re-fires the wake, drains the read-end pipe to clear
    ///    `epoll`'s pending state, drains the iterator queue into a
    ///    Python list, and returns the list.
    /// 4. if the iterator hit terminal EOF (`NextEvent::Closed` and the
    ///    batch is empty), raises `StopAsyncIteration`.
    fn __anext__<'py>(slf: PyRef<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            anext_step(session_handle).await
        })
    }

    /// Awaitable wrapper around `subscribe`. The underlying FPSS
    /// `subscribe` is a channel send (non-blocking already); we wrap it
    /// in `future_into_py` so the surface stays uniformly async on
    /// `async with` consumers — `await session.subscribe(sub)`.
    fn subscribe<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        sub: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let session = session_handle.borrow(py);
                session.handle.call_proxy(py, "subscribe", sub.bind(py))?;
                Ok::<Py<PyAny>, PyErr>(py.None())
            })
        })
    }

    /// Awaitable bulk-subscribe. Same shape as
    /// [`Self::subscribe`] — sequential per-spec subscribe with the
    /// FPSS protocol's per-spec round-trip semantics.
    fn subscribe_many<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        subs: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let session = session_handle.borrow(py);
                session
                    .handle
                    .call_proxy(py, "subscribe_many", subs.bind(py))?;
                Ok::<Py<PyAny>, PyErr>(py.None())
            })
        })
    }

    /// Awaitable unsubscribe.
    fn unsubscribe<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        sub: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let session = session_handle.borrow(py);
                session
                    .handle
                    .call_proxy(py, "unsubscribe", sub.bind(py))?;
                Ok::<Py<PyAny>, PyErr>(py.None())
            })
        })
    }

    /// Awaitable bulk-unsubscribe.
    fn unsubscribe_many<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        subs: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            Python::attach(|py| {
                let session = session_handle.borrow(py);
                session
                    .handle
                    .call_proxy(py, "unsubscribe_many", subs.bind(py))?;
                Ok::<Py<PyAny>, PyErr>(py.None())
            })
        })
    }

    /// Run a callback for every batch the session yields, with
    /// async-aware backpressure: if `callback` is `async def`, the
    /// iterator awaits its return before draining the next batch, so a
    /// slow consumer naturally throttles upstream.
    ///
    /// The callback receives a `list[FpssEvent]` per call.
    ///
    /// Sync callbacks (`def callback(batch):`) are invoked directly on
    /// the asyncio loop — keep them fast or the loop blocks. Use an
    /// `async def` callback when the work is non-trivial.
    fn streaming_async_for_each<'py>(
        slf: PyRef<'py, Self>,
        py: Python<'py>,
        callback: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let session_handle: Py<Self> = slf.into();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            for_each_loop(session_handle, callback).await
        })
    }

    /// Snapshot count of events currently buffered between the
    /// Disruptor consumer thread and this session. Diagnostic only —
    /// racy because the consumer pushes concurrently.
    fn queue_len(&self) -> usize {
        self.iterator.as_ref().map_or(0, |it| it.queue_len())
    }
}

impl StreamingAsyncSession {
    /// Construct a session bound to the unified client.
    pub(crate) fn from_tdx(handle: Py<crate::ThetaDataDxClient>) -> Self {
        Self {
            handle: AsyncStreamableHandle::Tdx(handle),
            read_fd: -1,
            wake: None,
            iterator: None,
            event_loop: None,
            asyncio_event: None,
            closed: false,
        }
    }

    /// Construct a session bound to the standalone FPSS client.
    pub(crate) fn from_fpss(handle: Py<crate::fpss_client::FpssClient>) -> Self {
        Self {
            handle: AsyncStreamableHandle::Fpss(handle),
            read_fd: -1,
            wake: None,
            iterator: None,
            event_loop: None,
            asyncio_event: None,
            closed: false,
        }
    }

    /// `__aenter__` body — runs under the asyncio coroutine, holds the
    /// GIL via the outer `Python::attach`. Splits out the sync work so
    /// the awaitable body stays small.
    #[cfg(unix)]
    fn aenter_inner(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.iterator.is_some() {
            return Err(PyRuntimeError::new_err(
                "StreamingAsyncSession is already entered -- one session enters at most once",
            ));
        }

        // Allocate a non-blocking self-pipe. `pipe2(O_CLOEXEC | O_NONBLOCK)`
        // is the atomic Linux primitive but macOS / BSD only ship plain
        // `pipe(2)` — we fall through to `fcntl(F_SETFD/F_SETFL)` to
        // set the same flags. The non-atomic path has a fork-window
        // hazard (a `fork()` after `pipe()` but before `fcntl()` would
        // inherit the FDs without `O_CLOEXEC`), but the Python SDK is
        // single-threaded at `__aenter__` time and never forks across
        // this window, so the gap is benign.
        let (read_fd, write_fd) = alloc_wake_pipe()?;

        // Hand the write-end to the Rust core via the wake-fd-keeping
        // constructor. The shared `Arc<WakeFd>` returned lets us call
        // `rearm()` from the asyncio reader path on this thread.
        let (rust_iter, wake) = match self.handle.start(py, write_fd) {
            Ok(pair) => pair,
            Err(err) => {
                // Close the FDs we allocated — the wake never took
                // ownership because the start failed.
                // SAFETY: both FDs are open, owned by this scope, and
                // not yet shared.
                unsafe {
                    libc::close(read_fd);
                    libc::close(write_fd);
                }
                return Err(err);
            }
        };

        // Capture the running asyncio loop and create the wake event.
        let asyncio = py.import("asyncio")?;
        let event_loop = asyncio.call_method0("get_running_loop")?;
        let asyncio_event = asyncio.call_method0("Event")?;

        // Build the wake callback: `lambda: asyncio_event.set()`. We
        // can't pass an async-aware callable; `add_reader` invokes its
        // callback synchronously on the loop thread. Setting the
        // asyncio.Event from there is the canonical pattern.
        let set_event = asyncio_event.getattr("set")?;
        event_loop.call_method1("add_reader", (read_fd, set_event))?;

        // Commit state. Storing in self only AFTER the asyncio
        // registration succeeds keeps `__aexit__` cleanup safe — a
        // partial enter never leaves a registered reader behind.
        self.read_fd = read_fd;
        self.wake = Some(wake);
        self.iterator = Some(Arc::new(rust_iter));
        self.event_loop = Some(event_loop.unbind());
        self.asyncio_event = Some(asyncio_event.unbind());
        self.closed = false;
        Ok(())
    }

    /// Non-Unix stub. asyncio's `add_reader` does not work on Windows
    /// ProactorEventLoop with pipes; rather than ship a broken surface
    /// we raise loudly so users pick the sync path.
    #[cfg(not(unix))]
    fn aenter_inner(&mut self, _py: Python<'_>) -> PyResult<()> {
        Err(PyRuntimeError::new_err(
            "streaming_async() requires a POSIX platform (Linux / macOS / BSD); \
             Windows asyncio's ProactorEventLoop does not support add_reader on pipes. \
             Use client.streaming(callback) or client.streaming_iter() instead.",
        ))
    }

    /// `__aexit__` body — runs under the asyncio coroutine. Removes
    /// the asyncio reader, closes the read FD, stops streaming, and
    /// awaits the drain barrier. Idempotent so a double-exit (e.g.
    /// nested `async with`) is safe.
    fn aexit_inner(&mut self, py: Python<'_>) -> PyResult<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;

        // 1. Remove the asyncio reader BEFORE closing the FD so the
        //    loop doesn't try to read from a closed descriptor on its
        //    next iteration.
        if let (Some(loop_obj), read_fd) = (self.event_loop.as_ref(), self.read_fd) {
            if read_fd >= 0 {
                let bound = loop_obj.bind(py);
                // `remove_reader` returns `True` if it actually removed
                // a reader; we don't care about the return value, but
                // we do want to surface real errors (e.g. event loop
                // closed before the session exited).
                let _ = bound.call_method1("remove_reader", (read_fd,))?;
            }
        }

        // 2. Close the read-end FD. The write-end is owned by the
        //    `WakeFd` Arc inside the iterator's `Delivery::Queue`
        //    variant — it closes itself on Drop when the iterator and
        //    the wake Arc on this session are both released.
        #[cfg(unix)]
        if self.read_fd >= 0 {
            // SAFETY: `self.read_fd` was allocated in `aenter_inner`,
            // owned by this session, and not yet closed (the asyncio
            // reader was removed in step 1 so no other thread touches
            // it). The `-1` sentinel below prevents a double-close.
            unsafe {
                libc::close(self.read_fd);
            }
            self.read_fd = -1;
        }

        // 3. Stop streaming + await drain. Same teardown semantics as
        //    the sync StreamingSession / StreamingIterSession context
        //    managers — operators see uniform behaviour across all
        //    three.
        self.handle.stop_streaming(py)?;
        let drained = self.handle.await_drain(py, EXIT_DRAIN_TIMEOUT_MS)?;

        // 4. Drop the iterator + wake Arc so the wake-fd Drop fires
        //    and the inner client refcount releases. The wake-fd Drop
        //    closes the write-end FD; once both sides are closed the
        //    self-pipe is fully torn down.
        self.iterator = None;
        self.wake = None;
        self.event_loop = None;
        self.asyncio_event = None;

        if !drained {
            // Match the sync surfaces' RuntimeWarning rather than
            // raising — the streaming pipeline is already torn down,
            // and the drain is best-effort observability. Raising
            // here would suppress any exception from the `async with`
            // body.
            let warnings = py.import("warnings")?;
            let msg = format!(
                "streaming_async drain timed out after {EXIT_DRAIN_TIMEOUT_MS}ms; \
                 the consumer thread may still be draining residual events."
            );
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("stacklevel", 2_u32)?;
            let runtime_warning = py.get_type::<pyo3::exceptions::PyRuntimeWarning>();
            warnings.call_method("warn", (msg, runtime_warning), Some(&kwargs))?;
        }
        Ok(())
    }
}

/// Drain the iterator's queue once into a typed Python list.
///
/// Returns the list (possibly empty) and a `terminal` flag. `terminal`
/// is `true` when the iterator's `try_next` reported `Closed` — the
/// caller surfaces that as `StopAsyncIteration` once the batch (which
/// may still contain the last tail of events) is delivered.
fn drain_batch(
    py: Python<'_>,
    iterator: &RustEventIterator,
) -> PyResult<(Py<PyList>, bool)> {
    // `Bound<PyList>::new` returns the list bound to `py`; we'll
    // append the typed pyclass objects in-place and unbind at the end.
    let batch = PyList::empty(py);
    let mut terminal = false;
    loop {
        match iterator.try_next() {
            NextEvent::Ready(evt) => {
                let buffered = fpss_event_to_buffered(&evt);
                let typed = buffered_event_to_typed(py, &buffered).map_err(|e| {
                    PyRuntimeError::new_err(format!(
                        "failed to convert FPSS event to typed Python class: {e}"
                    ))
                })?;
                batch.append(typed)?;
            }
            NextEvent::Timeout => break,
            NextEvent::Closed => {
                terminal = true;
                break;
            }
        }
    }
    Ok((batch.unbind(), terminal))
}

/// Drain the read-end of the self-pipe with non-blocking reads until
/// `EAGAIN`. Clears `epoll`'s pending wake-up state so the next
/// `add_reader` fire requires a fresh wake byte from the producer.
///
/// The producer's `WakeFd::signal()` coalesces — at most one byte is in
/// the pipe under steady state — so this drain typically loops once.
/// Under burst conditions where multiple wake bytes accumulated, we
/// drain them all in one call to avoid `epoll` firing twice for the
/// same logical batch.
/// Allocate a self-pipe with `O_CLOEXEC` + `O_NONBLOCK` on both ends.
///
/// Returns `(read_fd, write_fd)`. Uses `libc::pipe(2)` plus
/// `fcntl(F_SETFD, FD_CLOEXEC)` and `fcntl(F_SETFL, O_NONBLOCK)`
/// because macOS (and other BSDs) don't ship the atomic `pipe2(2)`
/// extension. The non-atomic path is correct for our single-threaded
/// `__aenter__` callsite; see the call-site comment for the
/// fork-window analysis.
///
/// Errors are mapped to Python `RuntimeError` with the kernel errno
/// surfaced through `Error::last_os_error()`.
#[cfg(unix)]
fn alloc_wake_pipe() -> PyResult<(i32, i32)> {
    let mut fds = [0_i32; 2];
    // SAFETY: `pipe(2)` writes two FDs into `fds`; documented safe
    // to invoke from any thread context.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        return Err(PyRuntimeError::new_err(format!(
            "pipe(2) failed allocating wake FDs for streaming_async: {err}"
        )));
    }
    let read_fd = fds[0];
    let write_fd = fds[1];

    // Set `O_CLOEXEC` and `O_NONBLOCK` via `fcntl(2)` on both ends.
    // Closing the FDs on early-exit so a partial-failure path doesn't
    // leak descriptors.
    let setup = |fd: i32| -> Result<(), std::io::Error> {
        // SAFETY: `fd` is open and owned by this function for the
        // duration of the call; the fcntl commands are all `i32` ABI
        // and don't touch user memory.
        let cloexec = unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        if cloexec < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: same as above. `F_GETFL` returns the current flag
        // bitmask, which we OR with `O_NONBLOCK` and write back via
        // `F_SETFL`.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `flags | O_NONBLOCK` is a valid `F_SETFL` argument.
        let nonblock = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        if nonblock < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    };
    if let Err(err) = setup(read_fd).and_then(|()| setup(write_fd)) {
        // SAFETY: both FDs are owned by this scope; close before
        // returning the error.
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err(PyRuntimeError::new_err(format!(
            "fcntl failed configuring wake FDs for streaming_async: {err}"
        )));
    }
    Ok((read_fd, write_fd))
}

#[cfg(unix)]
fn drain_read_pipe(read_fd: i32) {
    if read_fd < 0 {
        return;
    }
    let mut buf = [0_u8; 64];
    loop {
        // SAFETY: `read_fd` is owned by the session for the duration of
        // this call (the `add_reader` callback runs on the asyncio loop
        // thread which holds the GIL, and we're holding it too via
        // `Python::attach` upstream). `buf` is a valid 64-byte stack
        // buffer.
        let n = unsafe {
            libc::read(
                read_fd,
                buf.as_mut_ptr().cast::<libc::c_void>(),
                buf.len(),
            )
        };
        if n > 0 {
            continue;
        }
        // EAGAIN / EWOULDBLOCK is the success condition — pipe drained.
        // Any other error is logged via tracing; the session continues
        // because the wake protocol is self-healing (next producer
        // push re-fires the wake).
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if !matches!(err.kind(), std::io::ErrorKind::WouldBlock) {
                tracing::warn!(
                    target: "thetadatadx::streaming_async",
                    errno = err.raw_os_error().unwrap_or(0),
                    "wake-fd read failed; next batch may need a redundant wake"
                );
            }
        }
        break;
    }
}

/// One `__anext__` step. Splits out the awaitable body so the pyclass
/// method stays small and the async logic is testable in isolation.
async fn anext_step(session_handle: Py<StreamingAsyncSession>) -> PyResult<Py<PyAny>> {
    loop {
        // Step 1: try to drain WITHOUT awaiting. Covers the case where
        // the producer pushed between two `__anext__` calls and the
        // asyncio reader fired before we awaited — the asyncio event
        // is already set, but we'd rather return the batch immediately
        // than do a round-trip through the loop.
        let drained = Python::attach(|py| -> PyResult<Option<(Py<PyAny>, bool)>> {
            let session = session_handle.borrow(py);
            if session.closed {
                return Ok(Some((PyList::empty(py).unbind().into_any(), true)));
            }
            let iterator = session.iterator.as_ref().ok_or_else(|| {
                PyRuntimeError::new_err(
                    "StreamingAsyncSession not entered -- call `async with session:` first",
                )
            })?;
            let wake = session.wake.as_ref().ok_or_else(|| {
                PyRuntimeError::new_err(
                    "StreamingAsyncSession wake handle missing -- enter the context manager first",
                )
            })?;
            // Rearm BEFORE draining so any producer push observed
            // between the rearm and the drain re-fires the wake.
            wake.rearm();
            drain_read_pipe(session.read_fd);
            let (batch, terminal) = drain_batch(py, iterator)?;
            let len = batch.bind(py).len();
            if len > 0 || terminal {
                Ok(Some((batch.into_any(), terminal)))
            } else {
                Ok(None)
            }
        })?;

        if let Some((batch, terminal)) = drained {
            // `PyList::len` returns `usize` directly (the pyo3 0.28 API
            // for `PyList::len` is infallible on `&Bound<PyList>`), but
            // we're holding a `Py<PyAny>` here — go through `len_bound`
            // via `PyAnyMethods::len()` which IS fallible. The fallible
            // path is benign in this codepath because the value was
            // just constructed from `drain_batch` and is guaranteed to
            // be a real `PyList` instance.
            let len = Python::attach(|py| batch.bind(py).len())?;
            if terminal && len == 0 {
                return Err(PyStopAsyncIteration::new_err(()));
            }
            return Ok(batch);
        }

        // Step 2: queue empty and not terminal — await the asyncio
        // event. Capture the event reference under the GIL, then await
        // it OUTSIDE the GIL via the pyo3-async-runtimes coro bridge.
        let event_awaitable: Py<PyAny> = Python::attach(|py| -> PyResult<Py<PyAny>> {
            let session = session_handle.borrow(py);
            let event = session.asyncio_event.as_ref().ok_or_else(|| {
                PyRuntimeError::new_err(
                    "StreamingAsyncSession asyncio.Event missing -- enter the context manager first",
                )
            })?;
            // `event.wait()` returns a coroutine; we hand it through
            // `into_future` to integrate with the rust async runtime.
            let bound = event.bind(py);
            let coro = bound.call_method0("wait")?;
            Ok(coro.unbind())
        })?;

        // Bridge the Python coroutine into a Rust future and await it.
        let await_fut = Python::attach(|py| {
            pyo3_async_runtimes::tokio::into_future(event_awaitable.bind(py).clone())
        })?;
        let _ = await_fut.await?;

        // Step 3: clear the asyncio.Event so the next iteration waits
        // for a fresh wake. Done AFTER the await landed, so a producer
        // push that races with the clear still re-fires the wake (the
        // wake-fd protocol covers the race — the reader rearms before
        // draining inside step 1, and any push between the rearm and
        // the drain writes a fresh wake byte the asyncio loop will
        // observe on the next iteration).
        Python::attach(|py| -> PyResult<()> {
            let session = session_handle.borrow(py);
            if let Some(event) = session.asyncio_event.as_ref() {
                event.bind(py).call_method0("clear")?;
            }
            Ok(())
        })?;
        // Loop back to step 1 to drain.
    }
}

/// `streaming_async_for_each(callback)` body. Drives the async iterator
/// from inside Rust, invoking `callback(batch)` per drain. Honours
/// async-callback backpressure by awaiting the callback's return value
/// when it produces an awaitable.
async fn for_each_loop(
    session_handle: Py<StreamingAsyncSession>,
    callback: Py<PyAny>,
) -> PyResult<Py<PyAny>> {
    loop {
        let next_outcome = anext_step(session_handle.clone_ref_attached()).await;
        let batch = match next_outcome {
            Ok(batch) => batch,
            Err(err) => {
                // Treat StopAsyncIteration as terminal, propagate
                // everything else. Have to acquire the GIL to do the
                // typecheck (`is_instance_of` needs a `Python<'_>`
                // token); `Python::attach` is the safe primitive.
                let is_stop = Python::attach(|py| err.is_instance_of::<PyStopAsyncIteration>(py));
                if is_stop {
                    return Ok(Python::attach(|py| py.None()));
                }
                return Err(err);
            }
        };
        // Invoke the callback. If it returns an awaitable, await it
        // before draining the next batch — that is the
        // async-aware-backpressure contract advertised on the docstring.
        let result = Python::attach(|py| -> PyResult<Py<PyAny>> {
            let cb = callback.bind(py);
            Ok(cb.call1((batch,))?.unbind())
        })?;
        let maybe_awaitable = Python::attach(|py| -> PyResult<Option<Py<PyAny>>> {
            let bound = result.bind(py);
            // `inspect.isawaitable` is the canonical test — handles
            // both native coroutines and any user object exposing
            // `__await__`.
            let inspect = py.import("inspect")?;
            let is_awaitable: bool = inspect
                .call_method1("isawaitable", (bound.clone(),))?
                .extract()?;
            if is_awaitable {
                Ok(Some(result.clone_ref(py)))
            } else {
                Ok(None)
            }
        })?;
        if let Some(awaitable) = maybe_awaitable {
            let fut = Python::attach(|py| {
                pyo3_async_runtimes::tokio::into_future(awaitable.bind(py).clone())
            })?;
            let _ = fut.await?;
        }
    }
}

/// Small helper trait — clone the `Py<T>` while we already hold a GIL
/// token captured upstream. Keeps `for_each_loop` from re-attaching for
/// a trivial refcount bump.
trait CloneRefAttached {
    fn clone_ref_attached(&self) -> Self;
}

impl<T> CloneRefAttached for Py<T> {
    fn clone_ref_attached(&self) -> Self {
        Python::attach(|py| self.clone_ref(py))
    }
}

// ── Inherent helpers on the streaming pyclasses ──────────────────────────
//
// Inherent (non-`#[pymethods]`) impl blocks so the methods return raw
// Rust types (`EventIterator`, `Arc<WakeFd>`) without the pyo3 macro
// trying to convert them to Python objects. The corresponding pymethod
// surfaces (`streaming_async`) live in the matching `#[pymethods]` block
// at the bottom of this file.

impl crate::ThetaDataDxClient {
    /// Internal helper consumed by `StreamingAsyncSession::__aenter__`.
    /// Routes through the core's
    /// [`thetadatadx::ThetaDataDxClient::start_streaming_iter_with_wake`]
    /// so we never bypass the streaming-slot generation guard that
    /// rejects concurrent `start_streaming` / `streaming_async` /
    /// `streaming_iter` calls on the same client.
    #[cfg(unix)]
    pub(crate) fn start_streaming_async_inner(
        &self,
        write_fd: i32,
    ) -> PyResult<(RustEventIterator, Arc<WakeFd>)> {
        let inner = Arc::clone(&self.tdx);
        inner
            .start_streaming_iter_with_wake(WakeFd::from_raw_write_fd(write_fd))
            .map_err(to_py_err)
    }
}

impl crate::fpss_client::FpssClient {
    /// Internal helper consumed by `StreamingAsyncSession::__aenter__`.
    /// Splits the wake-fd plumbing from the `__aenter__` body so the
    /// asyncio-loop registration and the Rust streaming start stay
    /// in lockstep — partial-failure paths in `__aenter__` rely on
    /// this returning `Result` so the FD pair can be closed without
    /// reaching the asyncio side.
    #[cfg(unix)]
    pub(crate) fn start_streaming_async_inner(
        &self,
        write_fd: i32,
    ) -> PyResult<(RustEventIterator, Arc<WakeFd>)> {
        self.start_streaming_iter_with_wake_internal(write_fd)
    }
}

// ── PyO3 surfaces: `streaming_async()` on each streaming pyclass ────────

#[pymethods]
impl crate::ThetaDataDxClient {
    /// Open the FPSS connection in pull-iter mode with an asyncio FD
    /// wake-up signal, and return the [`StreamingAsyncSession`] context
    /// manager that drives it.
    ///
    /// ```python
    /// async with client.streaming_async() as session:
    ///     await session.subscribe(Contract.stock("QQQ").quote())
    ///     async for batch in session:
    ///         for ev in batch:
    ///             handle(ev)
    /// ```
    ///
    /// Comparison with the sibling streaming surfaces:
    ///
    /// * [`crate::ThetaDataDxClient::streaming`] (sync callback) — Disruptor
    ///   consumer thread invokes the user callback under the GIL. Lowest
    ///   latency for a callback-style consumer; cannot be `await`ed.
    /// * `streaming_iter()` (sync iterator) — `for event in iter:` drain
    ///   on the user thread. Polls at 100 µs; suitable for sync code
    ///   paths.
    /// * `streaming_async()` (this method) — asyncio-native drain with
    ///   FD-readiness signalling. Zero polling cost during quiet
    ///   periods; one OS wake per coalesced burst.
    ///
    /// # Backpressure
    ///
    /// If the asyncio consumer falls behind, the bounded Disruptor
    /// queue fills. The wake FD coalesces — exactly one byte sits in
    /// the pipe between batches — so the producer never blocks on a
    /// full pipe. Queue overflow increments
    /// [`Self::dropped_event_count`] and emits a `tracing::warn!`,
    /// matching the sync paths.
    ///
    /// # Disposal
    ///
    /// `__aexit__` removes the asyncio reader, closes the read-end FD,
    /// calls `stop_streaming` + `await_drain(5000ms)` on the underlying
    /// client (same semantics as the sync `streaming()` context
    /// manager), and drops the wake handle so the write-end FD closes.
    /// A drain timeout fires a `RuntimeWarning` rather than raising,
    /// so exceptions raised inside the `async with` body propagate.
    fn streaming_async(slf: Py<Self>, py: Python<'_>) -> PyResult<Py<StreamingAsyncSession>> {
        Py::new(py, StreamingAsyncSession::from_tdx(slf))
    }
}

#[pymethods]
impl crate::fpss_client::FpssClient {
    /// Open the FPSS connection in pull-iter mode with an asyncio FD
    /// wake-up signal, and return the [`StreamingAsyncSession`] context
    /// manager.
    ///
    /// Same surface as
    /// [`crate::ThetaDataDxClient::streaming_async`] but bound to the
    /// standalone FPSS-only client. The standalone client opens NO
    /// MDDS / Nexus surface, so an asyncio app that wants only the
    /// real-time stream (e.g. coexisting with a Java MDDS process)
    /// pays no MDDS connection cost.
    fn streaming_async(slf: Py<Self>, py: Python<'_>) -> PyResult<Py<StreamingAsyncSession>> {
        Py::new(py, StreamingAsyncSession::from_fpss(slf))
    }
}
