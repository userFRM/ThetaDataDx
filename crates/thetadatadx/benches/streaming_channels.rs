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
//! than sketches. Five variants are timed; the fifth is compiled only
//! under the private `__test-helpers` feature (it reaches the crate's
//! production ring constructor, which is not part of the public API).
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
//! 5. `disruptor_production_ctor` — same cross-thread topology as
//!    variant 4, but the pipeline is built through the crate's
//!    production ring constructor (`build_poller_producer`) instead of
//!    a raw `build_single_producer` ring. That constructor installs the
//!    sequence-recording producer adapter and returns the matching
//!    poller, so this variant measures the instrumented publish path the
//!    live client ships — one relaxed occupancy store per publish on the
//!    producer side, one per drained batch on the consumer side — rather
//!    than the bare ring. Compiled only under the private
//!    `__test-helpers` feature because the constructor is crate-internal.
//!
//! Every variant drives exactly `EVENTS_PER_ITER` (= 100_000)
//! `StreamEvent::Control(StreamControl::Connected)` deliveries per Criterion sample.
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
use thetadatadx::fpss::{StreamControl, StreamEvent};

// Variant 5 drives the crate's production ring constructor, which is
// crate-internal and only re-exported under the private `__test-helpers`
// feature (see `fpss::__test_internals`). `build_poller_producer` is the
// constructor that installs the sequence-recording producer adapter.
// VOCAB-OK: `build_poller_producer` / `RingProducer` are bench symbols
// from the crate under test, not user-facing prose.
#[cfg(feature = "__test-helpers")]
use thetadatadx::fpss::__test_internals::{
    build_poller_producer, AdaptiveWaitStrategy, Polling, RingCursors, RingProducer,
};

/// Number of events shipped through the pipeline per criterion sample.
/// Sized so the per-iteration wall-clock dwarfs criterion's measurement
/// overhead and so p50/p99 reflect steady-state behaviour, not warm-up.
const EVENTS_PER_ITER: usize = 100_000;

/// Disruptor ring size for the bench harness. Matches the production
/// default (`FpssConfig::ring_size = 131_072`, see
/// `crates/thetadatadx/src/config/fpss.rs`) so the bench numbers reflect
/// the out-of-the-box live SDK configuration.
const RING_SIZE: usize = 131_072;

#[derive(Default)]
struct RingSlot {
    event: Option<StreamEvent>,
}

/// Type alias for the mutable user-handler cell shared with the
/// Disruptor consumer closure. Factored out so each variant doesn't
/// repeat the `Mutex<Box<dyn FnMut(...)>>` shape and so clippy's
/// `type_complexity` lint stays quiet at `-D warnings`.
type BoxedHandler = Mutex<Box<dyn FnMut(&StreamEvent) + Send>>;

// SAFETY: matches the live `RingEvent` impl in
// `crates/thetadatadx/src/fpss/ring.rs` — `StreamEvent: Clone + Send`,
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
    let user_handler: BoxedHandler = Mutex::new(Box::new(move |_event: &StreamEvent| {
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
                    slot.event = Some(StreamEvent::Control(StreamControl::Connected));
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

    let user_handler: BoxedHandler = Mutex::new(Box::new(|_event: &StreamEvent| {}));

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
                    slot.event = Some(StreamEvent::Control(StreamControl::Connected));
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
    let trampoline: Box<dyn Fn(&StreamEvent)> = Box::new(move |_event: &StreamEvent| {
        delivered_cb.fetch_add(1, Ordering::Relaxed);
    });

    for _ in 0..EVENTS_PER_ITER {
        let event = StreamEvent::Control(StreamControl::Connected);
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

    let user_handler: BoxedHandler = Mutex::new(Box::new(|_event: &StreamEvent| {}));

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
                        slot.event = Some(StreamEvent::Control(StreamControl::Connected));
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

// ─── Variant 5: production ring constructor (instrumented adapter) ─────
//
// Variants 1-4 build a raw `build_single_producer` ring, so the
// before/after guard for the ring-occupancy feature pinned the shared
// ring machinery rather than the sequence-recording producer adapter the
// live client installs. This variant routes the whole pipeline through
// `build_poller_producer` — the production constructor — so the adapter's
// documented cost model (one relaxed occupancy store per successful
// publish, one per drained batch) has a standing measurement. The
// topology matches variant 4: the producer runs on a spawned worker
// thread, the consumer drains the returned poller on the bench thread,
// mirroring the live "TLS reader thread != consumer thread" deployment.

#[cfg(feature = "__test-helpers")]
fn run_disruptor_production_ctor() -> (u64, u64) {
    // Shared occupancy cursors the production adapter records into — the
    // exact pair `StreamingClient::ring_occupancy` samples in the live client.
    let cursors = Arc::new(RingCursors::new());
    let (mut producer, mut poller) = build_poller_producer(
        RING_SIZE,
        Arc::clone(&cursors),
        AdaptiveWaitStrategy::low_latency(),
    );

    // Producer thread: publish via the instrumented adapter. Each
    // successful `try_publish` records the published sequence into the
    // shared cursors (the adapter's per-publish relaxed store). Dropping
    // the producer at thread exit stores the shutdown sequence so the
    // consumer's poll observes `Polling::Shutdown` once the ring drains.
    let producer_thread = thread::spawn(move || {
        let mut dropped: u64 = 0;
        for _ in 0..EVENTS_PER_ITER {
            // Retry-on-overflow so the bench measures cost
            // per-DELIVERED-event, matching the other variants.
            loop {
                if producer
                    .try_publish(|slot| {
                        slot.set_public(StreamEvent::Control(StreamControl::Connected));
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
        dropped
    });

    // Consumer: drain the poller on this thread exactly as the live
    // `poll_batch` path does — count every released slot, record one
    // consumed-cursor store per drained batch (never per event), and
    // stop once the producer has dropped and the ring is fully drained.
    let mut delivered: u64 = 0;
    let mut consumed_seq: i64 = -1;
    loop {
        match poller.poll() {
            Ok(mut batch) => {
                let mut batch_len: i64 = 0;
                for ring_event in &mut batch {
                    batch_len += 1;
                    if ring_event.as_public().is_some() {
                        delivered += 1;
                    }
                }
                // One relaxed store per drained batch — the consumer side
                // of the adapter's documented cost model.
                consumed_seq += batch_len;
                cursors.record_consumed(consumed_seq);
            }
            Err(Polling::NoEvents) => std::hint::spin_loop(),
            Err(Polling::Shutdown) => break,
        }
    }

    let dropped = producer_thread
        .join()
        .expect("disruptor production-constructor producer panicked");
    (delivered, dropped)
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

#[cfg(feature = "__test-helpers")]
fn bench_disruptor_production_ctor(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/disruptor_production_ctor");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| drive(b, run_disruptor_production_ctor));
    group.finish();
}

// Variant 5 is only compiled under `__test-helpers` (it reaches the
// crate-internal production ring constructor), so the group it belongs to
// is selected by feature: with the feature off the four public-symbol
// variants run; with it on the production-constructor variant is appended.
#[cfg(not(feature = "__test-helpers"))]
criterion_group!(
    streaming_channels,
    bench_disruptor_consumer_panic_isolated,
    bench_disruptor_consumer_no_catch_unwind,
    bench_direct_callback,
    bench_disruptor_cross_thread,
);
#[cfg(feature = "__test-helpers")]
criterion_group!(
    streaming_channels,
    bench_disruptor_consumer_panic_isolated,
    bench_disruptor_consumer_no_catch_unwind,
    bench_direct_callback,
    bench_disruptor_cross_thread,
    bench_disruptor_production_ctor,
);
criterion_main!(streaming_channels);
