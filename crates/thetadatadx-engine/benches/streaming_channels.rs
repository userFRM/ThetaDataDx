//! Streaming hot-path channel benchmarks (issues #482, #513).
//!
//! After the #513 single-queue rewrite there is exactly ONE queue between
//! the FPSS TLS reader and the user callback — the LMAX Disruptor ring
//! buffer. The reader publishes events via `Producer::try_publish`; a
//! single Disruptor consumer thread invokes the user callback wrapped in
//! `catch_unwind`. The previous `StreamingDispatcher` (a second-queue
//! drain thread on top of the ring) has been deleted along with its
//! runtime dependency.
//!
//! These benches exercise the post-#513 pipeline end-to-end so the
//! release notes can quote a numbers-against-numbers comparison rather
//! than sketches. Four variants are timed.
//!
//! # Methodology
//!
//! Earlier revisions of this file divided the Criterion-measured wall
//! clock by `EVENTS_PER_ITER`, which silently inflated the
//! `try_publish` variants whenever the consumer fell behind: a publish
//! attempt that returned `RingBufferFull` was free, so dividing by the
//! attempt count understated per-DELIVERED-event cost. Each Disruptor
//! variant now retries `Producer::try_publish` on overflow until
//! exactly `EVENTS_PER_ITER` successful publishes have landed per
//! Criterion sample, so `delivered == EVENTS_PER_ITER` holds by
//! construction in every iteration. The bench reports
//! `wall_time / EVENTS_PER_ITER` via Criterion's fixed
//! `Throughput::Elements(EVENTS_PER_ITER as u64)`; the
//! per-callback `delivered_events: AtomicU64` snapshot taken inside
//! each consumer closure (or trampoline) is fed to a
//! `debug_assert_eq!` so any future regression in the retry-on-overflow
//! loop trips a debug-build failure rather than silently understating
//! cost. The `debug_assert!` is a no-op in release-mode bench runs;
//! the published numbers are per-DELIVERED-event by construction of
//! the retry loop, not by runtime assertion.
//!
//! For the `direct_callback` variant there is no ring and every
//! invocation is delivered, so `delivered == EVENTS_PER_ITER`
//! trivially. The Disruptor variants would drop on overflow without
//! the retry loop; with it, every attempted publish lands.
//!
//! 1. `disruptor_consumer_panic_isolated` — the live
//!    `start_streaming` path: `Producer::try_publish` on the producer
//!    thread, `handle_events_with` on the consumer thread, each callback
//!    invocation wrapped in `catch_unwind`. This is what the SDK ships.
//! 2. `disruptor_consumer_no_catch_unwind` — same Disruptor pipeline but
//!    without the `catch_unwind` boundary, so the cost of the panic
//!    isolation is observable as a delta against variant 1.
//! 3. `direct_callback` — a prospective inline-callback variant: the
//!    producer invokes the user callback in-place via a `Box<dyn Fn>`
//!    adapter, no ring, no consumer thread. Models a true
//!    TLS-reader-direct dispatch as a future option.
//! 4. `disruptor_cross_thread` — same as variant 1 but the producer
//!    runs on a worker thread spawned per iteration so the topology
//!    matches the live deployment (TLS reader thread != Disruptor
//!    consumer thread).
//!
//! All four variants drive exactly `EVENTS_PER_ITER` (= 100_000)
//! `FpssEvent::Control(FpssControl::Connected)` deliveries per Criterion sample.
//! `Throughput::Elements(EVENTS_PER_ITER as u64)` is exact by
//! construction; the reported ns/event are callback-delivery cost.
//!
//! Run: `cargo bench --bench streaming_channels`

use std::hint::black_box;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use disruptor::{build_single_producer, BusySpin, Producer, Sequence};
use thetadatadx_engine::fpss::{FpssControl, FpssEvent};

/// Number of events shipped through the pipeline per criterion sample.
/// Sized so the per-iteration wall-clock dwarfs criterion's measurement
/// overhead and so p50/p99 reflect steady-state behaviour, not warm-up.
const EVENTS_PER_ITER: usize = 100_000;

/// Disruptor ring size for the bench harness. Matches the production
/// default (`FpssConnectArgs::ring_size = 4096`) so the bench numbers
/// translate directly to the live SDK configuration.
const RING_SIZE: usize = 4096;

#[derive(Default)]
struct RingSlot {
    event: Option<FpssEvent>,
}

/// Type alias for the mutable user-handler cell shared with the
/// Disruptor consumer closure. Factored out so each variant doesn't
/// repeat the `Mutex<Box<dyn FnMut(...)>>` shape and so clippy's
/// `type_complexity` lint stays quiet at `-D warnings`.
type BoxedHandler = Mutex<Box<dyn FnMut(&FpssEvent) + Send>>;

// SAFETY: matches the live `RingEvent` impl in
// `crates/thetadatadx/src/fpss/ring.rs` — `FpssEvent: Clone + Send`,
// the Disruptor's sequencing guarantees exclusive write / shared read.
unsafe impl Sync for RingSlot {}

// ─── Variant 1: live SSOT (Disruptor consumer + catch_unwind) ──────────

/// Returns `(delivered_events, dropped_publishes)`.
fn run_disruptor_consumer_panic_isolated() -> (u64, u64) {
    let delivered = Arc::new(AtomicU64::new(0));
    let delivered_consumer = Arc::clone(&delivered);
    let panics = Arc::new(AtomicU64::new(0));
    let panics_consumer = Arc::clone(&panics);

    // Mirror the live `io_loop` wiring: `FnMut` user callback wrapped in
    // a `Mutex<F>` so the Disruptor consumer (which expects `Fn`) can
    // call it mutably across the boundary. Single-locker pattern — no
    // contention because only the consumer thread takes the lock.
    let user_handler: BoxedHandler = Mutex::new(Box::new(move |_event: &FpssEvent| {
        // Per-event delivered counter increments BEFORE the user
        // closure body so a panic inside the callback still counts
        // as a delivery (matches the bench's per-callback-entry
        // ns/event semantic).
    }));

    let factory = || RingSlot { event: None };
    let mut producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
                delivered_consumer.fetch_add(1, Ordering::Relaxed);
                let mut h = user_handler
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if catch_unwind(AssertUnwindSafe(|| h(evt))).is_err() {
                    panics_consumer.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
        .build();

    let mut dropped: u64 = 0;
    for _ in 0..EVENTS_PER_ITER {
        // Retry-on-overflow so the bench measures ns-per-DELIVERED-event,
        // not ns-per-publish-attempt (where rejections are free and
        // would understate the real callback-delivery cost).
        loop {
            if producer
                .try_publish(|slot| {
                    slot.event = Some(FpssEvent::Control(FpssControl::Connected));
                })
                .is_ok()
            {
                break;
            }
            dropped += 1;
            std::hint::spin_loop();
        }
    }

    // Drop the producer so the consumer drains and the worker thread
    // joins before this sample finishes — the criterion timer wraps
    // exactly this call site.
    drop(producer);
    (delivered.load(Ordering::Relaxed), dropped)
}

// ─── Variant 2: Disruptor consumer without catch_unwind ────────────────

fn run_disruptor_consumer_no_catch_unwind() -> (u64, u64) {
    let delivered = Arc::new(AtomicU64::new(0));
    let delivered_consumer = Arc::clone(&delivered);

    let user_handler: BoxedHandler = Mutex::new(Box::new(|_event: &FpssEvent| {}));

    let factory = || RingSlot { event: None };
    let mut producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
                delivered_consumer.fetch_add(1, Ordering::Relaxed);
                let mut h = user_handler
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                h(evt);
            }
        })
        .build();

    let mut dropped: u64 = 0;
    for _ in 0..EVENTS_PER_ITER {
        loop {
            if producer
                .try_publish(|slot| {
                    slot.event = Some(FpssEvent::Control(FpssControl::Connected));
                })
                .is_ok()
            {
                break;
            }
            dropped += 1;
            std::hint::spin_loop();
        }
    }

    drop(producer);
    (delivered.load(Ordering::Relaxed), dropped)
}

// ─── Variant 3: direct TLS-reader-thread callback (prospective inline) ─

fn run_direct_callback() -> (u64, u64) {
    let delivered = Arc::new(AtomicU64::new(0));
    let delivered_cb = Arc::clone(&delivered);

    // Models a future TLS-reader-direct dispatch: a `Box<dyn Fn>`
    // invoked by the producer in-place, with no ring, no consumer
    // thread, no `catch_unwind`. Reserved for an inline-callback
    // variant if it ever lands.
    let trampoline: Box<dyn Fn(&FpssEvent)> = Box::new(move |_event: &FpssEvent| {
        delivered_cb.fetch_add(1, Ordering::Relaxed);
    });

    for _ in 0..EVENTS_PER_ITER {
        let event = FpssEvent::Control(FpssControl::Connected);
        trampoline(&event);
    }

    // No ring, every invocation is delivered, no drops possible.
    (delivered.load(Ordering::Relaxed), 0)
}

// ─── Variant 4: cross-thread Disruptor publish (multi-thread topology) ─
//
// The first three variants run the producer on the bench thread; the
// live SDK runs the producer on the FPSS reader thread and the
// consumer on a different OS thread spawned by the Disruptor builder.
// This variant pins down that the cross-thread cost is dominated by
// the same publish + consumer pair, with the producer thread spawned
// explicitly so the topology matches the live deployment.

fn run_disruptor_cross_thread() -> (u64, u64) {
    let delivered = Arc::new(AtomicU64::new(0));
    let delivered_consumer = Arc::clone(&delivered);

    let user_handler: BoxedHandler = Mutex::new(Box::new(|_event: &FpssEvent| {}));

    let factory = || RingSlot { event: None };
    let mut producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
                delivered_consumer.fetch_add(1, Ordering::Relaxed);
                let mut h = user_handler
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if catch_unwind(AssertUnwindSafe(|| h(evt))).is_err() {}
            }
        })
        .build();

    let producer_thread = thread::spawn(move || {
        let mut dropped: u64 = 0;
        for _ in 0..EVENTS_PER_ITER {
            // Retry-on-overflow so the bench measures cost
            // per-DELIVERED-event, matching the other variants.
            loop {
                if producer
                    .try_publish(|slot| {
                        slot.event = Some(FpssEvent::Control(FpssControl::Connected));
                    })
                    .is_ok()
                {
                    break;
                }
                dropped += 1;
                std::hint::spin_loop();
            }
        }
        // Drop the producer to drain + join the consumer.
        drop(producer);
        dropped
    });

    let dropped = producer_thread
        .join()
        .expect("disruptor cross-thread producer panicked");
    (delivered.load(Ordering::Relaxed), dropped)
}

// ─── Criterion driver ──────────────────────────────────────────────────

/// Wrap a `() -> (delivered, dropped)` runner so the per-iteration
/// return is fed to `black_box`, the delivered count is asserted to
/// equal `EVENTS_PER_ITER` (so `Throughput::Elements(EVENTS_PER_ITER)`
/// gives correct ns-per-DELIVERED-event semantics), and a regression
/// in the retry-on-overflow loop produces a loud bench failure
/// instead of silent under-counting.
fn drive(b: &mut criterion::Bencher<'_>, runner: impl Fn() -> (u64, u64)) {
    b.iter(|| {
        let (delivered, dropped) = runner();
        debug_assert_eq!(
            delivered, EVENTS_PER_ITER as u64,
            "bench retry-on-overflow loop must deliver every attempted event \
             (delivered={delivered}, expected={EVENTS_PER_ITER}, dropped-attempts={dropped})",
        );
        black_box((delivered, dropped));
    });
}

fn bench_disruptor_consumer_panic_isolated(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/disruptor_consumer_panic_isolated");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| {
        drive(b, run_disruptor_consumer_panic_isolated);
    });
    group.finish();
}

fn bench_disruptor_consumer_no_catch_unwind(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/disruptor_consumer_no_catch_unwind");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| {
        drive(b, run_disruptor_consumer_no_catch_unwind);
    });
    group.finish();
}

fn bench_direct_callback(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/direct_callback");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| drive(b, run_direct_callback));
    group.finish();
}

fn bench_disruptor_cross_thread(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/disruptor_cross_thread");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| drive(b, run_disruptor_cross_thread));
    group.finish();
}

criterion_group!(
    streaming_channels,
    bench_disruptor_consumer_panic_isolated,
    bench_disruptor_consumer_no_catch_unwind,
    bench_direct_callback,
    bench_disruptor_cross_thread,
);
criterion_main!(streaming_channels);
