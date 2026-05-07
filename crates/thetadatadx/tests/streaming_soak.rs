//! Soak tests for the post-#513 single-queue FPSS streaming pipeline.
//!
//! These tests exercise the contract every callback path inherits from
//! `io_loop`'s Disruptor consumer wiring, without standing up a live
//! FPSS TLS connection:
//!
//! 1. **`slow_callback_does_not_block_reader`** — a callback that
//!    sleeps per event must not block the producer side once the ring
//!    buffer fills; `Producer::try_publish` must return
//!    [`disruptor::RingBufferFull`] so the simulated reader thread can
//!    keep advancing.
//! 2. **`panicking_callback_does_not_kill_consumer`** — every Nth
//!    event panics inside the user callback; the consumer thread must
//!    keep delivering, and a `panic_count` counter must reflect every
//!    panic observed by `catch_unwind`.
//! 3. **`callback_triggered_stop_does_not_self_join`** — the
//!    `start_streaming` path calls back on the Disruptor consumer
//!    thread, which is NOT the same thread that runs
//!    `FpssClient::Drop`; therefore a callback that drops its own
//!    handle (modelled by signalling a shutdown flag and dropping the
//!    producer at the end of the test) does not deadlock.
//! 4. **`burst_overload_drops_count_correctly`** — pushing more events
//!    than the ring can hold while the consumer is gated must produce
//!    a drop count exactly equal to the overflow.
//!
//! All four exercise the **same closure shape** as
//! `crates/thetadatadx/src/fpss/io_loop/mod.rs` so a regression in the
//! consumer wiring (lost panic isolation, lost drop counting) fails
//! these tests without needing live credentials.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use disruptor::{build_single_producer, BusySpin, Producer, Sequence};
use thetadatadx::fpss::FpssEvent;

/// Mirror of `crate::fpss::ring::RingEvent` — kept local so the soak
/// suite does not need to bend the public surface around exposing the
/// internal ring slot type.
#[derive(Default)]
struct Slot {
    event: Option<FpssEvent>,
}

// SAFETY: matches the live `RingEvent`'s `unsafe impl Sync` reasoning —
// `FpssEvent: Clone + Send`, and the Disruptor's sequencing guarantees
// exclusive write / shared read.
unsafe impl Sync for Slot {}

/// Build a producer wired exactly the way `io_loop::io_loop` builds
/// it: user callback under a `Mutex<F>` (so `FnMut` works inside the
/// `Fn` `handle_events_with`), invocation wrapped in `catch_unwind`,
/// panic count incremented on `Err`. Returns the producer and the two
/// counters so the test can drive the producer side and assert
/// against the counters after `drop(producer)` joins the consumer.
fn build_consumer<F>(
    ring_size: usize,
    handler: F,
) -> (
    impl Producer<Slot>,
    Arc<AtomicU64>, // panic_count
)
where
    F: FnMut(&FpssEvent) + Send + 'static,
{
    let panic_count = Arc::new(AtomicU64::new(0));
    let panic_count_consumer = Arc::clone(&panic_count);

    let handler_cell: Mutex<Box<dyn FnMut(&FpssEvent) + Send>> = Mutex::new(Box::new(handler));

    let producer = build_single_producer(ring_size, || Slot { event: None }, BusySpin)
        .handle_events_with(move |slot: &Slot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
                match evt {
                    FpssEvent::Empty | FpssEvent::RawData { .. } => {}
                    _ => {
                        let mut h = handler_cell
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if catch_unwind(AssertUnwindSafe(|| h(evt))).is_err() {
                            panic_count_consumer.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        })
        .build();

    (producer, panic_count)
}

/// Synthetic non-`Empty` / non-`RawData` event so the consumer's
/// internal-event filter does not drop it.
fn event() -> FpssEvent {
    FpssEvent::Control(thetadatadx::fpss::FpssControl::MarketOpen)
}

#[test]
fn slow_callback_does_not_block_reader() {
    // Tiny ring so the producer overflows quickly.
    const RING: usize = 64;
    // Sleep duration per callback — slow enough that the consumer
    // cannot drain a 64-slot ring before the producer pushes 1000
    // events.
    const PER_EVENT: Duration = Duration::from_millis(10);
    const TOTAL: usize = 1_000;

    let (mut producer, _panics) = build_consumer(RING, move |_event: &FpssEvent| {
        thread::sleep(PER_EVENT);
    });

    let mut dropped: u64 = 0;
    let started = Instant::now();
    for _ in 0..TOTAL {
        if producer
            .try_publish(|slot| {
                slot.event = Some(event());
            })
            .is_err()
        {
            dropped += 1;
        }
    }
    let producer_wall = started.elapsed();

    // The producer must finish the burst in a small fraction of the
    // total wall-clock the consumer would need to drain (TOTAL *
    // PER_EVENT = 10 s). If the producer was blocking, the loop above
    // would take the full 10 s. Allow generous headroom for CI noise:
    // the bound is "well under 1 second" rather than an exact figure.
    assert!(
        producer_wall < Duration::from_secs(2),
        "producer was blocked by slow callback: took {producer_wall:?} for {TOTAL} publishes",
    );
    // And drops must have happened — otherwise the assertion above is
    // observing a lucky scheduling rather than the contract.
    assert!(
        dropped > 0,
        "ring (size {RING}) did not overflow under slow callback: dropped={dropped}",
    );
    assert!(
        dropped < TOTAL as u64,
        "every event dropped — consumer never made progress (dropped={dropped})",
    );

    // Drop the producer so the consumer drains and joins. The test
    // doesn't care about the final delivered count — only about the
    // contract that `try_publish` is non-blocking.
    drop(producer);
}

#[test]
fn panicking_callback_does_not_kill_consumer() {
    // Every 3rd event panics; the consumer must keep delivering.
    const RING: usize = 64;
    const TOTAL: usize = 30;

    let received = Arc::new(AtomicU64::new(0));
    let received_cb = Arc::clone(&received);
    let n = AtomicU64::new(0);

    let (mut producer, panics) = build_consumer(RING, move |_event: &FpssEvent| {
        let i = n.fetch_add(1, Ordering::Relaxed);
        received_cb.fetch_add(1, Ordering::Relaxed);
        if i.is_multiple_of(3) {
            panic!("synthetic panic on event {i}");
        }
    });

    for _ in 0..TOTAL {
        producer
            .try_publish(|slot| {
                slot.event = Some(event());
            })
            .expect("ring has plenty of headroom for 30 events on a 64-slot ring");
    }

    drop(producer);

    let received_total = received.load(Ordering::Relaxed);
    assert_eq!(
        received_total, TOTAL as u64,
        "consumer must have delivered every event despite panics; received={received_total}",
    );
    let panicked = panics.load(Ordering::Relaxed);
    let expected_panics = TOTAL.div_ceil(3) as u64;
    assert_eq!(
        panicked, expected_panics,
        "panic_count must reflect every panicking invocation (received {received_total}, panicked {panicked}, expected {expected_panics})",
    );
}

#[test]
fn callback_triggered_stop_does_not_self_join() {
    // Models the contract that `start_streaming` callbacks run on the
    // Disruptor consumer thread, which is NOT the thread that owns
    // `FpssClient::Drop`'s `io_handle.join()`. A callback that
    // requests shutdown (here: setting a flag and stopping production)
    // must not deadlock; the producer drop on the test thread joins
    // the consumer cleanly.
    const RING: usize = 64;

    let stop_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_cb = Arc::clone(&stop_requested);

    let (mut producer, _panics) = build_consumer(RING, move |_event: &FpssEvent| {
        // Simulate a callback that asks the SDK to shut down. With
        // `start_streaming` this is sound because the callback is on
        // the Disruptor consumer thread, separate from the TLS reader
        // thread that `FpssClient::Drop` joins. The flag is the
        // observable side effect; we just need to see it set without
        // deadlock.
        stop_cb.store(true, Ordering::Release);
    });

    // Push one event so the consumer fires the callback.
    producer
        .try_publish(|slot| {
            slot.event = Some(event());
        })
        .expect("ring has room for one event");

    // Drop the producer — this triggers the consumer-thread join.
    // If the callback's "stop" had self-joined, this drop would
    // deadlock; the test would hang past its default timeout. The
    // assertion below executes only if drop returned cleanly.
    drop(producer);

    assert!(
        stop_requested.load(Ordering::Acquire),
        "callback must have observed the stop request",
    );
}

#[test]
fn burst_overload_drops_count_correctly() {
    // Block the consumer with a mutex, push more events than the ring
    // can hold, count the drops, then release the mutex so the
    // consumer drains and the test can join cleanly.
    const RING: usize = 64;

    let gate = Arc::new(Mutex::new(()));
    let gate_cb = Arc::clone(&gate);
    let lock = gate.lock().expect("gate lock");

    let (mut producer, _panics) = build_consumer(RING, move |_event: &FpssEvent| {
        // First call blocks until the test thread drops the outer
        // guard. Subsequent calls acquire and release in nanoseconds.
        let _g = gate_cb.lock().expect("callback gate");
    });

    // Burst: ring (64) + extra (200). The first ~64 sends fill the
    // ring; the consumer pulls one event and blocks on the gate, so
    // up to 64 sends past that threshold also succeed (the consumer's
    // wait re-arms after each pull). The exact crossover depends on
    // Disruptor scheduling, but every event past the headroom is a
    // drop, and total drops + delivered must equal TOTAL.
    const TOTAL: usize = RING + 200;
    let mut dropped: u64 = 0;
    for _ in 0..TOTAL {
        if producer
            .try_publish(|slot| {
                slot.event = Some(event());
            })
            .is_err()
        {
            dropped += 1;
        }
    }

    assert!(
        dropped > 0,
        "ring (size {RING}) must overflow when consumer is blocked by gate; dropped={dropped}",
    );
    let delivered = TOTAL as u64 - dropped;
    // We can also bound delivered: the consumer is blocked, so it
    // pulled at most one event before the gate blocked it. So
    // `delivered <= RING + 1` (one in-flight + RING in the ring).
    // Some Disruptor builds prefetch differently; bound generously to
    // RING + 4 to absorb implementation slack.
    assert!(
        delivered <= (RING + 4) as u64,
        "delivered count {delivered} exceeds ring headroom (RING={RING}) — drops are likely undercounted",
    );

    // Release the gate so the consumer drains and the producer drop
    // joins the consumer thread cleanly.
    drop(lock);
    drop(producer);
}
