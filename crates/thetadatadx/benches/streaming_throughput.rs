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
//! - Each variant emits a realistic `StreamEvent::Data(StreamData::Trade)`
//!   carrying an `Arc<Contract>`. The contract is allocated once and
//!   the publish closure clones the `Arc` (refcount bump, no
//!   allocation) — this matches the live decode path where the
//!   contract is interned in the FPSS contract cache and every event
//!   takes a refcount on the same pointer.
//!
//! Run: `cargo bench --bench streaming_throughput -- --noplot`

use std::ffi::c_void;
use std::hint::black_box;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use disruptor::{build_single_producer, BusySpin, Producer, Sequence};
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{StreamData, StreamEvent};
use thetadatadx::Price;

/// Events shipped per Criterion sample. Sized so per-iteration wall
/// clock dwarfs Criterion measurement overhead and so the consumer
/// thread reaches steady state.
const EVENTS_PER_ITER: usize = 100_000;

/// Disruptor ring size. Matches `FpssConnectArgs::ring_size = 4096` so
/// these numbers translate directly to the live SDK configuration.
const RING_SIZE: usize = 4096;

#[derive(Default)]
struct RingSlot {
    event: Option<StreamEvent>,
}

// SAFETY: matches `RingEvent` in `crates/thetadatadx/src/fpss/ring.rs`
// — `StreamEvent: Clone + Send`, the Disruptor's sequencing guarantees
// exclusive write / shared read.
unsafe impl Sync for RingSlot {}

/// Mutable user-handler cell, shape-compatible with the live `io_loop`
/// wiring (`Mutex<Box<dyn FnMut>>` — single-locker, no contention).
type BoxedHandler = Mutex<Box<dyn FnMut(&StreamEvent) + Send>>;

// ─── Realistic event factory ──────────────────────────────────────────

/// Build a `Trade` event carrying a shared `Arc<Contract>`. The
/// contract is interned by the caller (allocate once, clone the Arc
/// per event) so the publish closure pays only a refcount bump — same
/// shape as the live FPSS decode path where the contract cache hands
/// out `Arc::clone(&cached)` for every tick.
#[inline]
fn make_event(contract: &Arc<Contract>, idx: u64) -> StreamEvent {
    StreamEvent::Data(StreamData::Trade {
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
    F: FnMut(&StreamEvent) + Send + 'static,
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
// `StreamEvent` (one Arc-clone + struct fill) is part of every realistic
// publish path — the live decode loop materializes the event before
// `try_publish`. We deliberately count it inside the timed region so
// the bench reflects what an integrator actually pays. The variants
// differ only in what the *consumer* closure does; the producer side
// is identical across all six.

// ─── Variant 1: Rust no-op closure ────────────────────────────────────

fn run_rust_noop(contract: Arc<Contract>) -> (u64, Duration) {
    let (producer, delivered) = build_pipeline(|_evt: &StreamEvent| {
        // No-op — sanity check against `disruptor_consumer_panic_isolated`
        // in `streaming_channels.rs`.
    });
    drive_publish(producer, delivered, contract)
}

// ─── Variant 2: clone into pre-allocated Vec<StreamEvent> ───────────────

fn run_rust_vec_push(contract: Arc<Contract>) -> (u64, Duration) {
    // Pre-allocate so Vec growth is excluded from the timed cost.
    let buf: Arc<Mutex<Vec<StreamEvent>>> =
        Arc::new(Mutex::new(Vec::with_capacity(EVENTS_PER_ITER + 16)));
    let buf_consumer = Arc::clone(&buf);
    let (producer, delivered) = build_pipeline(move |evt: &StreamEvent| {
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

// ─── Variant 4: FFI-style extern "C" callback indirection ─────────────

/// Counter used as the FFI context — pointer-only, mimics what a C or
/// C++ shim would pass as `void* ctx`.
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
    // `drive_publish` call). `event_ptr` is a `*const StreamEvent` cast
    // to `*const c_void` by the caller.
    let ctx = unsafe { &*(ctx as *const FfiCtx) };
    // SAFETY: `event_ptr` is a `*const StreamEvent` produced by the bench
    // harness one stack frame above; the referent lives until
    // `drive_publish` returns, which is strictly after this callback
    // ends. No aliasing — the consumer is the sole reader.
    let evt = unsafe { &*(event_ptr as *const StreamEvent) };
    // Touch the event so the read is not elided. Match-arm chosen for
    // its uniqueness so the optimiser cannot collapse the load.
    if let StreamEvent::Data(StreamData::Trade { sequence, .. }) = evt {
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
    let (producer, delivered) = build_pipeline(move |evt: &StreamEvent| {
        let ctx_ptr = (Arc::as_ptr(&ctx_consumer)) as *mut c_void;
        let evt_ptr = (evt as *const StreamEvent) as *const c_void;
        cb(ctx_ptr, evt_ptr);
    });
    let result = drive_publish(producer, delivered, contract);
    black_box(ctx.counter.load(Ordering::Relaxed));
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

fn bench_ffi_simulated(c: &mut Criterion) {
    bench_variant(c, "ffi_simulated", run_ffi_simulated);
}

criterion_group!(
    streaming_throughput,
    bench_rust_noop,
    bench_rust_vec_push,
    bench_ffi_simulated,
);
criterion_main!(streaming_throughput);
