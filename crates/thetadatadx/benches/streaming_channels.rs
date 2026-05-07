//! Streaming hot-path channel benchmarks (issues #482, #513).
//!
//! After the #513 single-queue rewrite there is exactly ONE queue between
//! the FPSS TLS reader and the user callback — the LMAX Disruptor ring
//! buffer. The reader publishes events via `Producer::try_publish`; a
//! single Disruptor consumer thread invokes the user callback wrapped in
//! `catch_unwind`. The previous `StreamingDispatcher` (a second
//! `crossbeam_channel::bounded(8192)` queue plus drain thread) has been
//! deleted along with its `crossbeam-channel` dependency.
//!
//! These benches exercise the post-#513 pipeline end-to-end so the
//! release notes can quote a numbers-against-numbers comparison rather
//! than sketches. Three variants are timed:
//!
//! 1. `disruptor_consumer_panic_isolated` — the live
//!    `start_streaming` path: `Producer::try_publish` on the producer
//!    thread, `handle_events_with` on the consumer thread, each callback
//!    invocation wrapped in `catch_unwind`. This is what the SDK ships.
//! 2. `disruptor_consumer_no_catch_unwind` — same Disruptor pipeline but
//!    without the `catch_unwind` boundary, so the cost of the panic
//!    isolation is observable as a delta against variant 1.
//! 3. `direct_callback` — the prospective inline path the
//!    `expert-mode` feature flag reserves for: the producer invokes the
//!    user callback in-place via a `Box<dyn Fn>` adapter, no ring, no
//!    consumer thread. Models a true TLS-reader-direct dispatch.
//!
//! All three variants ship 100k `FpssEvent::Empty` payloads per
//! criterion sample so the per-event wall-clock is large enough to
//! dwarf the harness overhead.
//!
//! Run: `cargo bench --bench streaming_channels`

use std::hint::black_box;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use disruptor::{build_single_producer, BusySpin, Producer, Sequence};
use thetadatadx::fpss::FpssEvent;

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

// SAFETY: matches the live `RingEvent` impl in
// `crates/thetadatadx/src/fpss/ring.rs` — `FpssEvent: Clone + Send`,
// the Disruptor's sequencing guarantees exclusive write / shared read.
unsafe impl Sync for RingSlot {}

// ─── Variant 1: live SSOT (Disruptor consumer + catch_unwind) ──────────

fn run_disruptor_consumer_panic_isolated() {
    let counter = Arc::new(AtomicU64::new(0));
    let counter_consumer = Arc::clone(&counter);
    let panics = Arc::new(AtomicU64::new(0));
    let panics_consumer = Arc::clone(&panics);

    // Mirror the live `io_loop` wiring: `FnMut` user callback wrapped in
    // a `Mutex<F>` so the Disruptor consumer (which expects `Fn`) can
    // call it mutably across the boundary. Single-locker pattern — no
    // contention because only the consumer thread takes the lock.
    let user_handler: Mutex<Box<dyn FnMut(&FpssEvent) + Send>> =
        Mutex::new(Box::new(move |_event: &FpssEvent| {
            counter_consumer.fetch_add(1, Ordering::Relaxed);
        }));

    let factory = || RingSlot { event: None };
    let mut producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
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
        if producer
            .try_publish(|slot| {
                slot.event = Some(FpssEvent::Empty);
            })
            .is_err()
        {
            dropped += 1;
        }
    }

    // Drop the producer so the consumer drains and the worker thread
    // joins before this sample finishes — the criterion timer wraps
    // exactly this call site.
    drop(producer);
    black_box((counter.load(Ordering::Relaxed), dropped));
}

// ─── Variant 2: Disruptor consumer without catch_unwind ────────────────

fn run_disruptor_consumer_no_catch_unwind() {
    let counter = Arc::new(AtomicU64::new(0));
    let counter_consumer = Arc::clone(&counter);

    let user_handler: Mutex<Box<dyn FnMut(&FpssEvent) + Send>> =
        Mutex::new(Box::new(move |_event: &FpssEvent| {
            counter_consumer.fetch_add(1, Ordering::Relaxed);
        }));

    let factory = || RingSlot { event: None };
    let mut producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
                let mut h = user_handler
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                h(evt);
            }
        })
        .build();

    let mut dropped: u64 = 0;
    for _ in 0..EVENTS_PER_ITER {
        if producer
            .try_publish(|slot| {
                slot.event = Some(FpssEvent::Empty);
            })
            .is_err()
        {
            dropped += 1;
        }
    }

    drop(producer);
    black_box((counter.load(Ordering::Relaxed), dropped));
}

// ─── Variant 3: direct TLS-reader-thread callback (prospective inline) ─

fn run_direct_callback() {
    let counter = Arc::new(AtomicU64::new(0));
    let counter_cb = Arc::clone(&counter);

    // Same shape the eventual `expert-mode` reader-thread path would
    // use: a `Box<dyn Fn>` invoked by the producer in-place. Models a
    // future TLS-reader-direct dispatch with no ring, no consumer
    // thread, no `catch_unwind`.
    let trampoline: Box<dyn Fn(&FpssEvent)> = Box::new(move |_event: &FpssEvent| {
        counter_cb.fetch_add(1, Ordering::Relaxed);
    });

    for _ in 0..EVENTS_PER_ITER {
        let event = FpssEvent::Empty;
        trampoline(&event);
    }

    black_box(counter.load(Ordering::Relaxed));
}

// ─── Variant 4: cross-thread Disruptor publish (multi-thread topology) ─
//
// The first three variants run the producer on the bench thread; the
// live SDK runs the producer on the FPSS reader thread and the
// consumer on a different OS thread spawned by the Disruptor builder.
// This variant pins down that the cross-thread cost is dominated by
// the same publish + consumer pair, with the producer thread spawned
// explicitly so the topology matches the live deployment.

fn run_disruptor_cross_thread() {
    let counter = Arc::new(AtomicU64::new(0));
    let counter_consumer = Arc::clone(&counter);

    let user_handler: Mutex<Box<dyn FnMut(&FpssEvent) + Send>> =
        Mutex::new(Box::new(move |_event: &FpssEvent| {
            counter_consumer.fetch_add(1, Ordering::Relaxed);
        }));

    let factory = || RingSlot { event: None };
    let mut producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
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
            if producer
                .try_publish(|slot| {
                    slot.event = Some(FpssEvent::Empty);
                })
                .is_err()
            {
                dropped += 1;
            }
        }
        // Drop the producer to drain + join the consumer.
        drop(producer);
        dropped
    });

    let dropped = producer_thread
        .join()
        .expect("disruptor cross-thread producer panicked");
    black_box((counter.load(Ordering::Relaxed), dropped));
}

// ─── Criterion driver ──────────────────────────────────────────────────

fn bench_disruptor_consumer_panic_isolated(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/disruptor_consumer_panic_isolated");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| {
        b.iter(run_disruptor_consumer_panic_isolated);
    });
    group.finish();
}

fn bench_disruptor_consumer_no_catch_unwind(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/disruptor_consumer_no_catch_unwind");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| {
        b.iter(run_disruptor_consumer_no_catch_unwind);
    });
    group.finish();
}

fn bench_direct_callback(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/direct_callback");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| b.iter(run_direct_callback));
    group.finish();
}

fn bench_disruptor_cross_thread(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_channels/disruptor_cross_thread");
    group.throughput(Throughput::Elements(EVENTS_PER_ITER as u64));
    group.sample_size(10);
    group.bench_function("100k_events", |b| b.iter(run_disruptor_cross_thread));
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
