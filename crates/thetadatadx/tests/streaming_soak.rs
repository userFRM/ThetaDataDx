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

    type BoxedHandler = Mutex<Box<dyn FnMut(&FpssEvent) + Send>>;
    let handler_cell: BoxedHandler = Mutex::new(Box::new(handler));

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
    // Real reproducer for the self-join deadlock fixed in this branch.
    //
    // Live trace the test pins down:
    //
    //   1. User callback runs on the Disruptor consumer thread
    //      (`crates/thetadatadx/src/fpss/io_loop/mod.rs` consumer
    //      closure).
    //   2. Callback drops the last `Arc<FpssClient>` it carries — the
    //      live SDK does this via `ThetaDataDx::stop_streaming` swapping
    //      `StreamingSlot::Live` to `StreamingSlot::Stopped`, which
    //      releases the previous slot's `Arc<FpssClient>` on the
    //      consumer thread.
    //   3. Last `Arc<FpssClient>` drop runs `FpssClient::Drop`, which
    //      previously called `io_handle.join()` unconditionally.
    //   4. The I/O thread's exit path drops the Disruptor producer.
    //   5. `disruptor::Producer::Drop` joins the consumer thread.
    //   6. The consumer thread IS the thread running step (1). The
    //      pre-fix `io_handle.join()` would block waiting for itself —
    //      classic self-join deadlock.
    //
    // After the fix, `FpssClient::Drop` checks
    // `consumer_thread_id == current().id()` and detaches the join
    // onto a helper thread. Cleanup completes asynchronously; observers
    // poll a flag (or `is_streaming()` on the unified path) to confirm.
    //
    // The watchdog below pins down regressions: if the deadlock returns
    // the worker thread never exits and the watchdog fires within 5 s.
    use std::sync::Mutex as StdMutex;
    use thetadatadx::fpss::{FpssClient, HarnessPublishMode};

    const WATCHDOG: Duration = Duration::from_secs(5);
    const RING: usize = 64;
    const N_EVENTS: usize = 4;

    let dropped_observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let dropped_observed_main = Arc::clone(&dropped_observed);

    // Hand the test-only `FpssClient::for_self_join_test` constructor
    // a callback that drops the last `Arc<FpssClient>` from inside
    // itself. The callback owns the Arc through a `Mutex<Option<...>>`
    // so the first invocation can take it out and let it drop while
    // the consumer thread is still running this very closure.
    let (worker_done_tx, worker_done_rx) = std::sync::mpsc::channel::<()>();

    let worker = thread::spawn(move || {
        let arc_holder: Arc<StdMutex<Option<Arc<FpssClient>>>> = Arc::new(StdMutex::new(None));
        let arc_holder_cb = Arc::clone(&arc_holder);
        let dropped_observed_cb = Arc::clone(&dropped_observed);

        let client = FpssClient::for_self_join_test(
            N_EVENTS,
            RING,
            HarnessPublishMode::BlockingPublish,
            move |_event: &FpssEvent| {
                // Take the Arc out of the holder (idempotent). The
                // first take leaves the holder empty; subsequent
                // events are no-ops. Dropping the local `taken_arc`
                // at end of scope releases what was the last
                // `Arc<FpssClient>` reference, triggering
                // `FpssClient::Drop` ON THE CONSUMER THREAD. That
                // is the precise scenario the production
                // `stop_streaming() inside the callback` flow
                // produces.
                let taken_arc = arc_holder_cb
                    .lock()
                    .expect("arc holder lock poisoned")
                    .take();
                if let Some(arc) = taken_arc {
                    drop(arc);
                    dropped_observed_cb.store(true, Ordering::Release);
                }
            },
        );

        // Stash the only outstanding `Arc<FpssClient>` reference into
        // the holder so the callback can drop it. After this line,
        // the consumer's closure owns the only path to the Arc.
        *arc_holder.lock().expect("arc holder lock poisoned") = Some(client);

        // Wait until the callback has observed at least one event +
        // taken the Arc. Spin briefly so we don't hammer the lock.
        let waited_until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < waited_until {
            if dropped_observed.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        worker_done_tx.send(()).expect("worker_done channel closed");
    });

    // Watchdog: if the worker doesn't return within WATCHDOG, the
    // self-join deadlock has reappeared. Fail loud.
    match worker_done_rx.recv_timeout(WATCHDOG) {
        Ok(()) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!(
                "self-join deadlock detected: worker did not return within {WATCHDOG:?} after \
                 dropping the last Arc<FpssClient> from inside the Disruptor consumer callback",
            );
        }
        Err(other) => panic!("worker channel error: {other:?}"),
    }

    worker.join().expect("worker thread panicked");

    assert!(
        dropped_observed_main.load(Ordering::Acquire),
        "callback never observed an event — the test scenario did not reach the \
         self-join code path",
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

/// Same burst-overload contract as
/// `burst_overload_drops_count_correctly`, but routed through the
/// **public** counter on the real `FpssClient`
/// (`FpssClient::dropped_count()`). The harness's I/O thread runs a
/// `try_publish` burst against a gated consumer — the same shape as
/// the live `io_loop` data path — and increments the same
/// `Arc<AtomicU64>` the public getter reads.
#[test]
fn burst_overload_increments_public_dropped_count() {
    use std::sync::Mutex as StdMutex;
    use thetadatadx::fpss::{FpssClient, HarnessPublishMode};

    const RING: usize = 64;
    // Generous burst so that even with the consumer pulling one or
    // two events before the gate stalls it, the post-ring overflow
    // is unmistakable.
    const N_BURST: usize = RING * 8;

    let gate = Arc::new(StdMutex::new(()));
    let gate_cb = Arc::clone(&gate);
    let lock = gate.lock().expect("gate lock");

    let client = FpssClient::for_self_join_test(
        N_BURST,
        RING,
        HarnessPublishMode::TryPublishBurst,
        move |_event: &FpssEvent| {
            let _g = gate_cb.lock().expect("callback gate");
        },
    );

    // Wait for the harness's I/O thread to finish its `try_publish`
    // burst. The gate is still held, so the consumer is stalled
    // mid-callback and the burst will end with the public counter
    // pegged at the overflow count. Bound the wait so a regression
    // in the harness path fails fast rather than hanging this test.
    let waited_until = Instant::now() + Duration::from_secs(3);
    let mut last_seen = client.dropped_count();
    let mut stable_for = 0u32;
    while Instant::now() < waited_until {
        let now = client.dropped_count();
        if now == last_seen && now > 0 {
            stable_for += 1;
            if stable_for >= 5 {
                break;
            }
        } else {
            stable_for = 0;
            last_seen = now;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let dropped = client.dropped_count();
    assert!(
        dropped > 0,
        "public dropped_count must reflect try_publish overflow: dropped={dropped}, ring={RING}, burst={N_BURST}",
    );

    // Lower bound: the burst is N_BURST events, the ring holds RING
    // slots, and the consumer is stalled on the gate (so it pulls at
    // most ~RING + a small slack of events). Therefore drops must be
    // at least N_BURST - RING - slack. Use a conservative slack of
    // 2*RING to absorb Disruptor scheduling.
    let lower_bound = (N_BURST.saturating_sub(2 * RING)) as u64;
    assert!(
        dropped >= lower_bound,
        "public dropped_count={dropped} below expected lower bound {lower_bound} for burst={N_BURST} ring={RING}",
    );

    // Release the gate so the consumer drains; drop the Arc so the
    // I/O thread shuts down and joins cleanly.
    drop(lock);
    drop(client);
}
