//! End-to-end streaming throughput across binding shapes.
//!
//! `streaming_channels.rs` already pins down the Disruptor pipeline cost
//! with a no-op user callback. This bench extends the methodology to
//! the callback shapes that matter for production integrators: a
//! Rust closure that actually retains the event, a lock-free queue
//! push, an FFI-style indirection, and the two recommended Python
//! handover patterns (`collections.deque.append` and
//! `queue.Queue.put_nowait`). Every variant drives exactly the same
//! Disruptor pipeline as `disruptor_consumer_panic_isolated`; the only
//! thing that changes is what the user closure does inside
//! `catch_unwind`.
//!
//! # Methodology
//!
//! - One Disruptor + one consumer thread per Criterion iteration. Build
//!   cost (ring allocation, thread spawn, handler installation) is
//!   excluded via `b.iter_custom`: setup runs outside the timed region
//!   and only the publish loop + producer drop are inside.
//! - The producer retries `try_publish` on `RingBufferFull` so all
//!   `EVENTS_PER_ITER` events are delivered (no silent drops). The
//!   delivered count is asserted via `debug_assert_eq!`; the published
//!   numbers are per-DELIVERED-event by construction.
//! - Each variant emits a realistic `FpssEvent::Data(FpssData::Trade)`
//!   carrying an `Arc<Contract>`. The contract is allocated once and
//!   the publish closure clones the `Arc` (refcount bump, no
//!   allocation) — this matches the live decode path where the
//!   contract is interned in the FPSS contract cache and every event
//!   takes a refcount on the same pointer.
//! - Python variants pre-acquire the deque / Queue object once per
//!   sample and stash it as `Py<PyAny>` on the consumer closure. The
//!   timed region acquires the GIL per event (matches the recommended
//!   Python integration pattern from `sdks/python/README.md`).
//!
//! Run: `cargo bench --bench streaming_throughput -- --noplot`

use std::ffi::c_void;
use std::hint::black_box;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use crossbeam_queue::ArrayQueue;
use disruptor::{build_single_producer, BusySpin, Producer, Sequence};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use tdbe::types::price::Price;
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssData, FpssEvent};

/// Events shipped per Criterion sample. Sized so per-iteration wall
/// clock dwarfs Criterion measurement overhead and so the consumer
/// thread reaches steady state.
const EVENTS_PER_ITER: usize = 100_000;

/// Disruptor ring size. Matches `FpssConnectArgs::ring_size = 4096` so
/// these numbers translate directly to the live SDK configuration.
const RING_SIZE: usize = 4096;

#[derive(Default)]
struct RingSlot {
    event: Option<FpssEvent>,
}

// SAFETY: matches `RingEvent` in `crates/thetadatadx/src/fpss/ring.rs`
// — `FpssEvent: Clone + Send`, the Disruptor's sequencing guarantees
// exclusive write / shared read.
unsafe impl Sync for RingSlot {}

/// Mutable user-handler cell, shape-compatible with the live `io_loop`
/// wiring (`Mutex<Box<dyn FnMut>>` — single-locker, no contention).
type BoxedHandler = Mutex<Box<dyn FnMut(&FpssEvent) + Send>>;

// ─── Realistic event factory ──────────────────────────────────────────

/// Build a `Trade` event carrying a shared `Arc<Contract>`. The
/// contract is interned by the caller (allocate once, clone the Arc
/// per event) so the publish closure pays only a refcount bump — same
/// shape as the live FPSS decode path where the contract cache hands
/// out `Arc::clone(&cached)` for every tick.
#[inline]
fn make_event(contract: &Arc<Contract>, idx: u64) -> FpssEvent {
    FpssEvent::Data(FpssData::Trade {
        contract: Arc::clone(contract),
        ms_of_day: (idx % 86_400_000) as i32,
        sequence: idx as i32,
        ext_condition1: 0,
        ext_condition2: 0,
        ext_condition3: 0,
        ext_condition4: 0,
        condition: 0,
        size: 100,
        exchange: 0,
        price: Price::new(15025, 8).to_f64(),
        condition_flags: 0,
        price_flags: 0,
        volume_type: 0,
        records_back: 0,
        date: 20240315,
        received_at_ns: idx,
    })
}

/// Build the standard SPY contract used as the event payload across
/// every variant. Allocated once per sample, then shared via
/// `Arc::clone`.
fn make_contract() -> Arc<Contract> {
    Arc::new(Contract::stock("SPY"))
}

// ─── Disruptor harness with a user-supplied consumer body ─────────────

/// Builds the Disruptor + consumer thread, returns the producer plus a
/// shared delivery counter. `body` is invoked on the consumer thread
/// for every delivered event, wrapped in `catch_unwind` to match the
/// live SSOT pipeline.
fn build_pipeline<F>(body: F) -> (impl Producer<RingSlot>, Arc<AtomicU64>)
where
    F: FnMut(&FpssEvent) + Send + 'static,
{
    let delivered = Arc::new(AtomicU64::new(0));
    let delivered_consumer = Arc::clone(&delivered);
    let user_handler: BoxedHandler = Mutex::new(Box::new(body));

    let factory = || RingSlot { event: None };
    let producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
                delivered_consumer.fetch_add(1, Ordering::Relaxed);
                let mut h = user_handler
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let _ = catch_unwind(AssertUnwindSafe(|| h(evt)));
            }
        })
        .build();
    (producer, delivered)
}

/// Drive the publish loop + drain. `EVENTS_PER_ITER` deliveries are
/// guaranteed by the retry-on-overflow loop. Returns
/// `(delivered_count, elapsed)` where `elapsed` is the publish loop +
/// producer drop only (setup is excluded by the caller).
fn drive_publish<P: Producer<RingSlot>>(
    mut producer: P,
    delivered: Arc<AtomicU64>,
    contract: Arc<Contract>,
) -> (u64, Duration) {
    let start = Instant::now();
    for i in 0..EVENTS_PER_ITER as u64 {
        loop {
            let evt = make_event(&contract, i);
            if producer
                .try_publish(|slot| {
                    slot.event = Some(evt);
                })
                .is_ok()
            {
                break;
            }
            std::hint::spin_loop();
        }
    }
    drop(producer);
    let elapsed = start.elapsed();
    (delivered.load(Ordering::Relaxed), elapsed)
}

// NOTE on `make_event` placement vs the timed region: building the
// `FpssEvent` (one Arc-clone + struct fill) is part of every realistic
// publish path — the live decode loop materializes the event before
// `try_publish`. We deliberately count it inside the timed region so
// the bench reflects what an integrator actually pays. The variants
// differ only in what the *consumer* closure does; the producer side
// is identical across all six.

// ─── Variant 1: Rust no-op closure ────────────────────────────────────

fn run_rust_noop(contract: Arc<Contract>) -> (u64, Duration) {
    let (producer, delivered) = build_pipeline(|_evt: &FpssEvent| {
        // No-op — sanity check against `disruptor_consumer_panic_isolated`
        // in `streaming_channels.rs`.
    });
    drive_publish(producer, delivered, contract)
}

// ─── Variant 2: clone into pre-allocated Vec<FpssEvent> ───────────────

fn run_rust_vec_push(contract: Arc<Contract>) -> (u64, Duration) {
    // Pre-allocate so Vec growth is excluded from the timed cost.
    let buf: Arc<Mutex<Vec<FpssEvent>>> =
        Arc::new(Mutex::new(Vec::with_capacity(EVENTS_PER_ITER + 16)));
    let buf_consumer = Arc::clone(&buf);
    let (producer, delivered) = build_pipeline(move |evt: &FpssEvent| {
        let mut v = buf_consumer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        v.push(evt.clone());
    });
    let result = drive_publish(producer, delivered, contract);
    // Consume the Vec so the optimiser cannot elide the push.
    let v = buf
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    black_box(v.len());
    result
}

// ─── Variant 3: push to crossbeam_queue::ArrayQueue ───────────────────

fn run_rust_arrayqueue_push(contract: Arc<Contract>) -> (u64, Duration) {
    // Capacity > EVENTS_PER_ITER so push never fails. ArrayQueue is
    // bounded MPMC lock-free; we use it single-producer here.
    let queue: Arc<ArrayQueue<FpssEvent>> = Arc::new(ArrayQueue::new(EVENTS_PER_ITER + 16));
    let queue_consumer = Arc::clone(&queue);
    let (producer, delivered) = build_pipeline(move |evt: &FpssEvent| {
        // ArrayQueue::push returns Err(value) on full; capacity guarantees
        // success here, but we defensively spin if the assumption ever
        // breaks rather than panicking on the consumer thread.
        if queue_consumer.push(evt.clone()).is_err() {
            std::hint::spin_loop();
        }
    });
    let result = drive_publish(producer, delivered, contract);
    black_box(queue.len());
    result
}

// ─── Variant 4: FFI-style extern "C" callback indirection ─────────────

/// Counter used as the FFI context — pointer-only, mimics what a C or
/// Go shim would pass as `void* ctx`.
struct FfiCtx {
    counter: AtomicU64,
}

/// `extern "C"` callback shape, matching the FFI surface a Python /
/// Node native binding would dispatch through. Takes a `*const c_void`
/// for the event pointer so the indirection cost (function-pointer
/// call + opaque pointer cast) is observable.
extern "C" fn ffi_trampoline(ctx: *mut c_void, event_ptr: *const c_void) {
    // SAFETY: `ctx` points to an `FfiCtx` that outlives the bench
    // (held inside `run_ffi_simulated` for the duration of the
    // `drive_publish` call). `event_ptr` is a `*const FpssEvent` cast
    // to `*const c_void` by the caller.
    let ctx = unsafe { &*(ctx as *const FfiCtx) };
    let evt = unsafe { &*(event_ptr as *const FpssEvent) };
    // Touch the event so the read is not elided. Match-arm chosen for
    // its uniqueness so the optimiser cannot collapse the load.
    if let FpssEvent::Data(FpssData::Trade { sequence, .. }) = evt {
        ctx.counter
            .fetch_add((*sequence as u64) & 1, Ordering::Relaxed);
    }
}

fn run_ffi_simulated(contract: Arc<Contract>) -> (u64, Duration) {
    let ctx = Arc::new(FfiCtx {
        counter: AtomicU64::new(0),
    });
    let ctx_consumer = Arc::clone(&ctx);
    // `extern "C" fn` coerces to a function pointer — same indirection
    // an FFI binding pays per event.
    let cb: extern "C" fn(*mut c_void, *const c_void) = ffi_trampoline;
    let (producer, delivered) = build_pipeline(move |evt: &FpssEvent| {
        let ctx_ptr = (Arc::as_ptr(&ctx_consumer)) as *mut c_void;
        let evt_ptr = (evt as *const FpssEvent) as *const c_void;
        cb(ctx_ptr, evt_ptr);
    });
    let result = drive_publish(producer, delivered, contract);
    black_box(ctx.counter.load(Ordering::Relaxed));
    result
}

// ─── Variant 5: PyO3 + collections.deque.append(tuple) ────────────────

fn run_pyo3_deque_append(contract: Arc<Contract>) -> (u64, Duration) {
    // Acquire the deque object once per sample. Two `Py<PyAny>`
    // handles: one held by the bench thread (for length read-out
    // after the run), one moved into the consumer closure.
    let (deque, deque_consumer) = Python::attach(|py| -> PyResult<(Py<PyAny>, Py<PyAny>)> {
        let collections = py.import("collections")?;
        let deque_cls = collections.getattr("deque")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("maxlen", EVENTS_PER_ITER + 16)?;
        let dq = deque_cls.call((), Some(&kwargs))?.unbind();
        let dq_clone = dq.clone_ref(py);
        Ok((dq, dq_clone))
    })
    .expect("pyo3 deque construction failed");

    let (producer, delivered) = build_pipeline(move |evt: &FpssEvent| {
        Python::attach(|py| {
            // Mirror the recommended Python integration pattern: build
            // a small tuple (5 fields) and append it to the deque.
            // Tuple construction is the dominant cost on the Python
            // side because it allocates a PyObject per event.
            if let FpssEvent::Data(FpssData::Trade {
                ms_of_day,
                price,
                size,
                received_at_ns,
                ..
            }) = evt
            {
                if let Ok(tup) = (*ms_of_day, *price, *size, *received_at_ns).into_pyobject(py) {
                    let _ = deque_consumer.bind(py).call_method1("append", (tup,));
                }
            }
        });
    });
    let result = drive_publish(producer, delivered, contract);
    let len = Python::attach(|py| deque.bind(py).len().expect("deque len"));
    black_box(len);
    result
}

// ─── Variant 6: PyO3 + queue.Queue.put_nowait(tuple) ──────────────────

fn run_pyo3_queue_put_nowait(contract: Arc<Contract>) -> (u64, Duration) {
    let (q, q_consumer) = Python::attach(|py| -> PyResult<(Py<PyAny>, Py<PyAny>)> {
        let queue_mod = py.import("queue")?;
        let queue_cls = queue_mod.getattr("Queue")?;
        // maxsize=0 -> unbounded. Matches the typical "drain on
        // worker thread" pattern for Python integrators.
        let q = queue_cls.call((0i32,), None)?.unbind();
        let q_clone = q.clone_ref(py);
        Ok((q, q_clone))
    })
    .expect("pyo3 Queue construction failed");

    let (producer, delivered) = build_pipeline(move |evt: &FpssEvent| {
        Python::attach(|py| {
            if let FpssEvent::Data(FpssData::Trade {
                ms_of_day,
                price,
                size,
                received_at_ns,
                ..
            }) = evt
            {
                if let Ok(tup) = (*ms_of_day, *price, *size, *received_at_ns).into_pyobject(py) {
                    let _ = q_consumer.bind(py).call_method1("put_nowait", (tup,));
                }
            }
        });
    });
    let result = drive_publish(producer, delivered, contract);
    let qsize = Python::attach(|py| {
        q.bind(py)
            .call_method0("qsize")
            .and_then(|v| v.extract::<usize>())
            .unwrap_or(0)
    });
    black_box(qsize);
    result
}

// ─── Variant 7: PyO3 zero-copy lazy-getter pyclass + deque.append ─────
//
// Mirrors the Python SDK's `FpssEvent` zero-copy wrapper. A
// `#[pyclass(frozen, freelist = 256)]` is reused across events via
// PyO3's per-class freelist (no heap alloc per event). The class
// holds a raw `*const FpssEvent` borrowed from the consumer closure
// scope plus a per-instance `AtomicBool` lifetime guard. Field access
// through the lazy getters is what the Python user pays — the
// 5-field tuple build of variant 5 is gone.
//
// The bench callback only touches one field (`price`) on the
// event, matching the typical "filter by price band" pattern; the
// throughput floor is the cost of crossing the binding once
// regardless of how many fields the user reads.

#[pyclass(frozen, freelist = 256, name = "BenchFpssEvent")]
struct BenchPyEvent {
    /// Raw pointer to a `FpssEvent` borrowed from the Disruptor consumer
    /// closure scope. Valid only while `valid` is true; flipped to false
    /// after the synchronous user callback returns.
    inner: *const FpssEvent,
    /// Lifetime guard. `Acquire` on every getter, `Release` after the
    /// callback returns. A retained handle (e.g., the user pushed the
    /// event into a list) raises `ValueError` on subsequent field access
    /// rather than reading freed memory.
    valid: AtomicBool,
}

// SAFETY: `BenchPyEvent` is constructed and dropped on the same
// (Disruptor consumer) thread; the Python interpreter may move the
// PyObject across threads, but every field access goes through the
// `valid: AtomicBool` gate so a stale pointer can never be
// dereferenced after the closure returns. The underlying `FpssEvent`
// is `Send + Sync` (`Arc<Contract>` + scalars). The bench mirrors the
// SDK's contract here exactly.
unsafe impl Send for BenchPyEvent {}
unsafe impl Sync for BenchPyEvent {}

impl BenchPyEvent {
    #[inline]
    fn evt(&self) -> PyResult<&FpssEvent> {
        if !self.valid.load(Ordering::Acquire) {
            return Err(PyValueError::new_err(
                "BenchFpssEvent accessed outside callback scope",
            ));
        }
        // SAFETY: `valid` is true, which the consumer closure guarantees
        // until it sets `valid = false` on closure exit. The pointer
        // therefore points at a `FpssEvent` borrowed from a stack-pinned
        // closure scope that strictly outlives the synchronous getter
        // call.
        Ok(unsafe { &*self.inner })
    }
}

#[pymethods]
impl BenchPyEvent {
    #[getter]
    fn price(&self) -> PyResult<Option<f64>> {
        match self.evt()? {
            FpssEvent::Data(FpssData::Trade { price, .. }) => Ok(Some(*price)),
            _ => Ok(None),
        }
    }
    #[getter]
    fn size(&self) -> PyResult<Option<i32>> {
        match self.evt()? {
            FpssEvent::Data(FpssData::Trade { size, .. }) => Ok(Some(*size)),
            _ => Ok(None),
        }
    }
    #[getter]
    fn ms_of_day(&self) -> PyResult<Option<i32>> {
        match self.evt()? {
            FpssEvent::Data(FpssData::Trade { ms_of_day, .. }) => Ok(Some(*ms_of_day)),
            _ => Ok(None),
        }
    }
}

fn run_pyo3_zerocopy_class_deque_append(contract: Arc<Contract>) -> (u64, Duration) {
    let (deque, deque_consumer) = Python::attach(|py| -> PyResult<(Py<PyAny>, Py<PyAny>)> {
        let collections = py.import("collections")?;
        let deque_cls = collections.getattr("deque")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("maxlen", EVENTS_PER_ITER + 16)?;
        let dq = deque_cls.call((), Some(&kwargs))?.unbind();
        let dq_clone = dq.clone_ref(py);
        Ok((dq, dq_clone))
    })
    .expect("pyo3 deque construction failed");

    let (producer, delivered) = build_pipeline(move |evt: &FpssEvent| {
        Python::attach(|py| {
            // SAFETY: `evt` is borrowed from the consumer closure
            // scope and lives for the synchronous duration of this
            // `Python::attach` block — the lifetime guard below
            // invalidates the wrapper immediately on scope exit, so
            // the user code (here `deque.append(py_evt)`) cannot read
            // the pointer after we leave.
            let py_evt = match Py::new(
                py,
                BenchPyEvent {
                    inner: evt as *const FpssEvent,
                    valid: AtomicBool::new(true),
                },
            ) {
                Ok(p) => p,
                Err(e) => {
                    e.write_unraisable(py, None);
                    return;
                }
            };
            // The realistic Python integrator pattern: hand the event
            // to `deque.append`. The deque retains the wrapper, but
            // any later field access raises `ValueError` since we
            // invalidate below.
            let bound = py_evt.bind(py).clone();
            let _ = deque_consumer.bind(py).call_method1("append", (bound,));
            // Invalidate so the retained wrapper safe-fails on later
            // access. Matches the SDK contract exactly.
            py_evt.borrow(py).valid.store(false, Ordering::Release);
        });
    });
    let result = drive_publish(producer, delivered, contract);
    let len = Python::attach(|py| deque.bind(py).len().expect("deque len"));
    black_box(len);
    result
}

// ─── Criterion driver ──────────────────────────────────────────────────

/// Run `runner` exactly `iters` times, summing the in-loop wall clock
/// (publish loop + producer drop). Setup (Disruptor build, thread
/// spawn, deque/Queue construction) is excluded.
fn time_iters<R>(iters: u64, mut runner: R) -> Duration
where
    R: FnMut(Arc<Contract>) -> (u64, Duration),
{
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        let contract = make_contract();
        let (delivered, elapsed) = runner(contract);
        debug_assert_eq!(
            delivered, EVENTS_PER_ITER as u64,
            "retry-on-overflow loop must deliver every attempted event \
             (delivered={delivered}, expected={EVENTS_PER_ITER})",
        );
        black_box(delivered);
        total += elapsed;
    }
    total
}

fn bench_variant<R>(c: &mut Criterion, name: &str, runner: R)
where
    R: FnMut(Arc<Contract>) -> (u64, Duration) + Copy,
{
    let mut group = c.benchmark_group(format!("streaming_throughput/{name}"));
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| {
        b.iter_custom(|iters| time_iters(iters, runner));
    });
    group.finish();
}

fn bench_rust_noop(c: &mut Criterion) {
    bench_variant(c, "rust_noop", run_rust_noop);
}

fn bench_rust_vec_push(c: &mut Criterion) {
    bench_variant(c, "rust_vec_push", run_rust_vec_push);
}

fn bench_rust_arrayqueue_push(c: &mut Criterion) {
    bench_variant(c, "rust_arrayqueue_push", run_rust_arrayqueue_push);
}

fn bench_ffi_simulated(c: &mut Criterion) {
    bench_variant(c, "ffi_simulated", run_ffi_simulated);
}

fn bench_pyo3_deque_append(c: &mut Criterion) {
    bench_variant(c, "pyo3_deque_append", run_pyo3_deque_append);
}

fn bench_pyo3_queue_put_nowait(c: &mut Criterion) {
    bench_variant(c, "pyo3_queue_put_nowait", run_pyo3_queue_put_nowait);
}

fn bench_pyo3_zerocopy_class_deque_append(c: &mut Criterion) {
    bench_variant(
        c,
        "pyo3_zerocopy_class_deque_append",
        run_pyo3_zerocopy_class_deque_append,
    );
}

// ─── Variant 8: PyO3 pull-iter drain under one GIL acquire ───
//
// Models the v9.1.0 pull-iter delivery shape. Pre-populates a
// crossbeam ArrayQueue with `EVENTS_PER_ITER` events (the producer
// side runs UNTIMED — bench owns it deterministically) and then
// drains the queue from a single Python::attach scope, building a
// 5-field tuple per event and appending to a deque under that one
// GIL hold. Compares directly against `pyo3_deque_append` (push-
// callback) on the same per-event Python work; the only difference is
// where the GIL acquire boundary sits.
//
// Caveat: the producer half is intentionally outside the timed region.
// That mirrors the live behaviour — under sustained load the
// Disruptor consumer thread fills the queue independently of the
// caller's drain cadence, so the integrator's wall-clock cost IS the
// drain loop. The push-callback variant pays the GIL acquire on every
// event because the consumer thread itself crosses the binding; the
// pull-iter variant amortises one acquire across the whole batch.
fn run_pyo3_iter_next_drain(contract: Arc<Contract>) -> (u64, Duration) {
    let queue: Arc<ArrayQueue<FpssEvent>> = Arc::new(ArrayQueue::new(EVENTS_PER_ITER + 16));
    // Producer side — UNTIMED. Pre-populate the queue exactly the way
    // the live FPSS Disruptor consumer would, but synchronously on the
    // bench thread so the timed region only measures the pull-iter
    // drain cost (the push-callback variant times push + drain
    // together because the user's per-event work runs synchronously
    // inside the consumer thread).
    for i in 0..EVENTS_PER_ITER as u64 {
        let evt = make_event(&contract, i);
        queue.push(evt).expect("queue capacity > EVENTS_PER_ITER");
    }
    let delivered = u64::try_from(queue.len()).unwrap_or(u64::MAX);

    // Acquire the deque target once, before the timed drain. Same
    // shape as `run_pyo3_deque_append`'s deque so the numbers compare
    // apples-to-apples.
    let deque = Python::attach(|py| -> PyResult<Py<PyAny>> {
        let collections = py.import("collections")?;
        let deque_cls = collections.getattr("deque")?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("maxlen", EVENTS_PER_ITER + 16)?;
        Ok(deque_cls.call((), Some(&kwargs))?.unbind())
    })
    .expect("pyo3 deque construction failed");

    let start = Instant::now();
    // Single GIL acquire across the entire drain — this is the
    // canonical pull-iter delivery path on the Python
    // binding. `for event in iter:` holds the GIL across every pop
    // until the loop exits.
    Python::attach(|py| {
        let bound_deque = deque.bind(py);
        while let Some(evt) = queue.pop() {
            if let FpssEvent::Data(FpssData::Trade {
                ms_of_day,
                price,
                size,
                received_at_ns,
                ..
            }) = evt
            {
                if let Ok(tup) = (ms_of_day, price, size, received_at_ns).into_pyobject(py) {
                    let _ = bound_deque.call_method1("append", (tup,));
                }
            }
        }
    });
    let elapsed = start.elapsed();
    let len = Python::attach(|py| deque.bind(py).len().expect("deque len"));
    black_box(len);
    (delivered, elapsed)
}

fn bench_pyo3_iter_next_drain(c: &mut Criterion) {
    bench_variant(c, "pyo3_iter_next_drain", run_pyo3_iter_next_drain);
}

criterion_group!(
    streaming_throughput,
    bench_rust_noop,
    bench_rust_vec_push,
    bench_rust_arrayqueue_push,
    bench_ffi_simulated,
    bench_pyo3_deque_append,
    bench_pyo3_queue_put_nowait,
    bench_pyo3_zerocopy_class_deque_append,
    bench_pyo3_iter_next_drain,
);
criterion_main!(streaming_throughput);
