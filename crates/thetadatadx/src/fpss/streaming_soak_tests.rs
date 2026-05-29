//! Soak tests for the post-#513 single-queue FPSS streaming pipeline.
//!
//! Lives inside the crate (rather than `tests/`) so the harness
//! constructor [`super::FpssClient::for_self_join_test`] and
//! [`super::HarnessPublishMode`] can be `#[cfg(test)]`-only without
//! widening the public surface through a downstream-flippable feature
//! flag.
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
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use super::FpssEvent;
use disruptor::{build_single_producer, BusySpin, Producer, Sequence};

/// Process-wide serializer for the two callback-triggered-stop soak
/// tests. Both stand up `BusySpin` Disruptor consumers, and on
/// CPU-constrained CI runners (or any host with
/// `--test-threads >= 2`) the BusySpin loops can starve each other
/// long enough for either test's "callback fired" observation budget
/// to expire, producing a flake that has nothing to do with the
/// self-join guard. Serialising them at the test boundary removes
/// the only source of contention without weakening either assertion.
fn callback_stop_serializer() -> &'static Mutex<()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

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
                let mut h = handler_cell
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if catch_unwind(AssertUnwindSafe(|| h(evt))).is_err() {
                    panic_count_consumer.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
        .build();

    (producer, panic_count)
}

/// Synthetic non-`Empty` / non-`RawData` event so the consumer's
/// internal-event filter does not drop it.
fn event() -> FpssEvent {
    FpssEvent::Control(super::FpssControl::MarketOpen)
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
    //      live SDK does this via `ThetaDataDxClient::stop_streaming` swapping
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
    use super::{FpssClient, HarnessPublishMode};
    use std::sync::Mutex as StdMutex;

    let _serial = callback_stop_serializer()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

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

        let start_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let client = FpssClient::for_self_join_test(
            N_EVENTS,
            RING,
            HarnessPublishMode::BlockingPublish,
            Some(Arc::clone(&start_signal)),
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
        // the holder so the callback can drop it, THEN release the
        // io thread to publish the burst. The start-signal handshake
        // closes the TOCTOU window between `for_self_join_test`
        // returning and the harness's first publish: without it, the
        // consumer can fire on an event that landed before the test
        // thread finished stashing, taking `None` from the holder
        // and never reaching the self-join Drop path.
        *arc_holder.lock().expect("arc holder lock poisoned") = Some(client);
        start_signal.store(true, Ordering::Release);

        // Wait until the callback has observed at least one event +
        // taken the Arc. Use most of the watchdog budget here so the
        // soak suite is not flaky under heavy parallel-test CPU load
        // (BusySpin consumer threads compete for cores). The
        // self-join failure mode would manifest as the worker thread
        // never returning at all — bounded above by `WATCHDOG` on
        // `worker_done_rx.recv_timeout` — so the inner observation
        // budget can be generous without weakening the deadlock
        // assertion.
        let waited_until = Instant::now() + Duration::from_secs(4);
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
    use super::{FpssClient, HarnessPublishMode};
    use std::sync::Mutex as StdMutex;

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
        None,
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

/// Quiescence-barrier coverage for the new `await_drain` contract.
///
/// Models the same callback-thread-stop scenario as
/// `callback_triggered_stop_does_not_self_join`, but adds the
/// post-stop drain barrier: the test thread captures the
/// `Arc<AtomicBool>` from `FpssClient::drained_flag()` BEFORE
/// dropping its Arc on the consumer thread, then polls the flag from
/// a separate thread. The flag MUST flip to `true` within the budget
/// (proving the detach helper joined), and no additional callback
/// invocations may fire after the flag is observed `true`.
#[test]
fn callback_triggered_stop_then_await_drain_completes() {
    use super::{FpssClient, HarnessPublishMode};
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex as StdMutex;

    let _serial = callback_stop_serializer()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    const WATCHDOG: Duration = Duration::from_secs(5);
    const RING: usize = 64;
    const N_EVENTS: usize = 4;

    let dropped_observed = Arc::new(AtomicBool::new(false));
    let dropped_observed_main = Arc::clone(&dropped_observed);

    // Counts callback invocations so we can assert nothing fires
    // after `await_drain` returns true.
    let invocations = Arc::new(AtomicU64::new(0));
    let invocations_cb = Arc::clone(&invocations);

    // Outbound: capture the drain flag from outside the consumer
    // thread BEFORE we release the last `Arc<FpssClient>` on the
    // consumer thread. Done via a oneshot mpsc so the worker thread
    // can hand the flag to the main thread before the callback drops
    // the Arc.
    let (drain_flag_tx, drain_flag_rx) = std::sync::mpsc::channel::<Arc<AtomicBool>>();
    let (worker_done_tx, worker_done_rx) = std::sync::mpsc::channel::<()>();

    let worker = thread::spawn(move || {
        let arc_holder: Arc<StdMutex<Option<Arc<FpssClient>>>> = Arc::new(StdMutex::new(None));
        let arc_holder_cb = Arc::clone(&arc_holder);
        let dropped_observed_cb = Arc::clone(&dropped_observed);

        let start_signal = Arc::new(AtomicBool::new(false));
        let client = FpssClient::for_self_join_test(
            N_EVENTS,
            RING,
            HarnessPublishMode::BlockingPublish,
            Some(Arc::clone(&start_signal)),
            move |_event: &FpssEvent| {
                invocations_cb.fetch_add(1, Ordering::Relaxed);
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

        // Hand the drain flag to the main thread BEFORE the callback
        // can possibly drop the Arc. After this send returns, the
        // main thread can poll the flag while the consumer thread
        // tears down asynchronously.
        drain_flag_tx
            .send(client.drained_flag())
            .expect("drain_flag channel closed");

        *arc_holder.lock().expect("arc holder lock poisoned") = Some(client);
        // Release the io thread to publish; the consumer cannot fire
        // before the holder is populated.
        start_signal.store(true, Ordering::Release);

        // Generous observation budget — see the same loop in
        // `callback_triggered_stop_does_not_self_join` for the rationale
        // (BusySpin consumer threads compete for CPU under parallel
        // test load; the self-join failure mode is bounded separately
        // by the outer `WATCHDOG`).
        let waited_until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < waited_until {
            if dropped_observed.load(Ordering::Acquire) {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        worker_done_tx.send(()).expect("worker_done channel closed");
    });

    let drain_flag = drain_flag_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("worker did not surface drain flag in time");

    // Wait for the worker to finish its observation phase. This is
    // distinct from the drain flag flipping — drain depends on the
    // helper thread joining the I/O thread, which races against the
    // worker's observation loop.
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

    // Poll the drain flag for up to WATCHDOG. The detach helper has
    // to join the I/O thread, which in turn drops the Disruptor
    // producer and joins the consumer. A budget of seconds is
    // generous; in practice this completes in single-digit ms.
    let drain_deadline = Instant::now() + WATCHDOG;
    let mut drained = false;
    while Instant::now() < drain_deadline {
        if drain_flag.load(Ordering::Acquire) {
            drained = true;
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        drained,
        "drain flag never flipped within {WATCHDOG:?} — the detach helper failed to \
         join the I/O thread + Disruptor consumer in budget",
    );

    let invocations_at_drain = invocations.load(Ordering::Relaxed);
    // Sleep a small additional window so any straggler callback
    // invocation would observably increment the counter past
    // `invocations_at_drain`. The drain barrier promises no
    // invocations AFTER it returns true; a regression here would
    // show up as the post-sleep value exceeding the snapshot.
    thread::sleep(Duration::from_millis(50));
    let invocations_after = invocations.load(Ordering::Relaxed);
    assert_eq!(
        invocations_at_drain, invocations_after,
        "callback fired after drain flag returned true (before={invocations_at_drain}, \
         after={invocations_after})",
    );
}

/// FFI-free-contract coverage: the destruction sequence used by
/// `tdx_unified_free` / `tdx_fpss_free` (raise the shutdown signal,
/// poll `drained_flag` until it flips, only then release the handle)
/// must not return until the consumer thread has stopped invoking the
/// user callback.
///
/// The FFI handles wrap an `Arc<FpssClient>` (unified) or a
/// `Mutex<Option<FpssClient>>` (FPSS), so the relevant property is
/// "after the free-shaped sequence below returns, no further callback
/// invocation can be observed against the (now-released) callback
/// context". This soak test stages a slow callback that bumps a
/// counter on every fire, runs the same sequence the FFI free does,
/// and asserts:
///
/// - the simulated free does NOT return before the drain flag flips
///   (proving free is a real barrier, not a no-op);
/// - no callback invocation is observed AFTER the simulated free
///   returns (proving `ctx` is safe to release on return).
#[test]
fn free_blocks_until_drain_complete() {
    use super::{FpssClient, HarnessPublishMode};
    use std::sync::atomic::AtomicBool;

    // Serialise against the other BusySpin-consumer tests; on
    // CPU-constrained CI runners parallel BusySpin loops can starve
    // each other long enough that the per-event sleep budget below
    // becomes flaky. The test thread (not the consumer thread) runs
    // Drop here, so there is no self-join interaction — only the
    // shared "do not stand up two BusySpin consumers concurrently"
    // contention concern.
    let _serial = callback_stop_serializer()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    const WATCHDOG: Duration = Duration::from_secs(5);
    const RING: usize = 64;
    const N_EVENTS: usize = 16;
    // Per-event sleep to make the consumer's drain non-instant; this
    // is what gives the test budget to observe "free has not returned
    // yet, callback is still firing" rather than racing the consumer.
    const PER_EVENT_SLEEP: Duration = Duration::from_millis(10);

    // Counter incremented inside the user callback on the consumer
    // thread. The post-free assertion compares a snapshot taken
    // immediately after the simulated free returns against a value
    // sampled a small window later; if the consumer is still firing,
    // the second sample exceeds the first.
    let invocations = Arc::new(AtomicU64::new(0));
    let invocations_cb = Arc::clone(&invocations);

    // Captured during simulated free so the post-free assertion knows
    // exactly when to start the "no further fires" observation
    // window.
    let free_returned = Arc::new(AtomicBool::new(false));
    let free_returned_cb = Arc::clone(&free_returned);

    // Set if the consumer ever fires the callback after free has
    // returned. If the new free contract regresses (drops the drain
    // barrier or returns before the flag flips), this flips to
    // `true` and the test fails.
    let post_free_fire = Arc::new(AtomicBool::new(false));
    let post_free_fire_cb = Arc::clone(&post_free_fire);

    let client = FpssClient::for_self_join_test(
        N_EVENTS,
        RING,
        HarnessPublishMode::BlockingPublish,
        None,
        move |_event: &FpssEvent| {
            invocations_cb.fetch_add(1, Ordering::Relaxed);
            if free_returned_cb.load(Ordering::Acquire) {
                // Any fire observed after the simulated free returns
                // is a contract violation.
                post_free_fire_cb.store(true, Ordering::Release);
            }
            // Slow the consumer just enough that the simulated free
            // path actually has to wait on the drain flag rather than
            // observing an already-quiesced consumer.
            thread::sleep(PER_EVENT_SLEEP);
        },
    );

    // Capture the drain flag BEFORE we drop the last `Arc<FpssClient>`
    // so the test thread observes the same flag the FFI free path
    // captures via `prev_drained` before the inner `Mutex<Option<...>>`
    // slot is taken.
    let drain_flag = client.drained_flag();

    // Mirror the FFI free contract:
    //
    //   1. raise the shutdown signal (asynchronous teardown begins)
    //   2. release the last `Arc<FpssClient>` so `Drop` runs the
    //      I/O-thread join + producer-drop + consumer-join sequence
    //      (this is what `Mutex::take()` does inside `tdx_fpss_free`)
    //   3. poll the drain flag until it flips, with a 5 s budget
    //   4. only AFTER the flag flips do we mark `free_returned`,
    //      which the callback uses to detect a post-free fire
    let started = Instant::now();
    client.shutdown();
    drop(client); // matches the inner `take()` in `tdx_fpss_free`

    let drain_deadline = started + WATCHDOG;
    let mut drained = false;
    while Instant::now() < drain_deadline {
        if drain_flag.load(Ordering::Acquire) {
            drained = true;
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        drained,
        "drain flag never flipped within {WATCHDOG:?} -- the simulated free path \
         would have logged a tracing::error! and proceeded with destruction with \
         the consumer still firing the callback",
    );

    // The barrier is real: the simulated free took at least one
    // PER_EVENT_SLEEP interval to return (the consumer was sleeping
    // through events). Require at least PER_EVENT_SLEEP / 2 — any
    // less and the consumer was already quiesced before the barrier
    // started polling, which would mean the test scenario isn't
    // exercising the new contract.
    let elapsed = started.elapsed();
    assert!(
        elapsed >= PER_EVENT_SLEEP / 2,
        "simulated free returned in {elapsed:?} -- below the {PER_EVENT_SLEEP:?} \
         lower bound, the consumer was already quiesced before the barrier started, \
         so the test scenario never exercised the post-shutdown drain wait",
    );

    // Mark "free has returned"; from this point on, any callback fire
    // is a contract violation. Order matters: the flag must flip
    // BEFORE the post-free observation window starts so a stray
    // post-drain invocation would observably set `post_free_fire`.
    free_returned.store(true, Ordering::Release);

    // Observation window: if the drain barrier missed an in-flight
    // event, it would land in this window and flip
    // `post_free_fire`. The barrier promised quiescence, so this
    // must stay clean.
    thread::sleep(Duration::from_millis(100));
    assert!(
        !post_free_fire.load(Ordering::Acquire),
        "callback fired after the simulated free returned -- the new \
         free contract failed to wait for consumer quiescence; \
         invocations={}, drained_flag={}",
        invocations.load(Ordering::Relaxed),
        drain_flag.load(Ordering::Acquire),
    );
}

// ---------------------------------------------------------------------------
// callback-watchdog soak coverage
// ---------------------------------------------------------------------------

/// Replica of the `io_loop` consumer-closure wiring with the
/// slow-callback watchdog plumbed through. Identical structure to
/// `build_consumer` above plus the per-event timer + counter +
/// rate-limited `tracing::warn!`. Returns the producer plus the
/// shared atomics so the test can drive the producer side and assert
/// against the counters after `drop(producer)` joins the consumer.
fn build_consumer_with_watchdog<F>(
    ring_size: usize,
    threshold_ns: Arc<AtomicU64>,
    handler: F,
) -> (
    impl Producer<Slot>,
    Arc<AtomicU64>, // panic_count
    Arc<AtomicU64>, // slow_callback_count
)
where
    F: FnMut(&FpssEvent) + Send + 'static,
{
    let panic_count = Arc::new(AtomicU64::new(0));
    let panic_count_consumer = Arc::clone(&panic_count);
    let slow_count = Arc::new(AtomicU64::new(0));
    let slow_count_consumer = Arc::clone(&slow_count);
    let threshold_ns_consumer = Arc::clone(&threshold_ns);

    type BoxedHandler = Mutex<Box<dyn FnMut(&FpssEvent) + Send>>;
    let handler_cell: BoxedHandler = Mutex::new(Box::new(handler));

    let producer = build_single_producer(ring_size, || Slot { event: None }, BusySpin)
        .handle_events_with(move |slot: &Slot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
                let mut h = handler_cell
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let threshold = threshold_ns_consumer.load(Ordering::Relaxed);
                let start = if threshold > 0 {
                    Some(Instant::now())
                } else {
                    None
                };
                if catch_unwind(AssertUnwindSafe(|| h(evt))).is_err() {
                    panic_count_consumer.fetch_add(1, Ordering::Relaxed);
                }
                if let Some(start) = start {
                    let elapsed_ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
                    if elapsed_ns > threshold {
                        let prev = slow_count_consumer.fetch_add(1, Ordering::Relaxed);
                        if prev.is_multiple_of(1024) {
                            tracing::warn!(
                                target: "thetadatadx::fpss::io_loop",
                                elapsed_ns,
                                threshold_ns = threshold,
                                slow_callback_count = prev + 1,
                                "user callback exceeded slow-callback threshold",
                            );
                        }
                    }
                }
            }
        })
        .build();

    (producer, panic_count, slow_count)
}

/// The callback sleeps `100ms` for every 10th event with a `50ms`
/// threshold, runs 100 events, verifies the counter == 10. Mirrors
/// the wiring in `io_loop::io_loop` so a regression in the consumer
/// closure is caught here without needing a live FPSS connection.
#[test]
fn slow_callback_threshold_counts_overbudget_invocations() {
    const RING: usize = 256;
    const TOTAL: usize = 100;
    const THRESHOLD_NS: u64 = 50_000_000; // 50 ms
    const SLOW_PER: usize = 10; // every 10th event
    const SLOW_DURATION: Duration = Duration::from_millis(100);

    let threshold = Arc::new(AtomicU64::new(THRESHOLD_NS));
    let n = AtomicU64::new(0);

    let (mut producer, _panics, slow) =
        build_consumer_with_watchdog(RING, Arc::clone(&threshold), move |_event: &FpssEvent| {
            let i = n.fetch_add(1, Ordering::Relaxed);
            if i.is_multiple_of(SLOW_PER as u64) {
                thread::sleep(SLOW_DURATION);
            }
        });

    for _ in 0..TOTAL {
        producer.publish(|slot| {
            slot.event = Some(event());
        });
    }

    // Drop producer to drain + join the consumer. After this returns
    // every event has been processed.
    drop(producer);

    let observed = slow.load(Ordering::Relaxed);
    // One slow-callback emission per `SLOW_PER` over-budget invocations.
    let expected = TOTAL.div_ceil(SLOW_PER) as u64;
    assert_eq!(
        observed, expected,
        "slow_callback_count must reflect every over-budget invocation: \
         observed={observed}, expected={expected}",
    );
}

/// With the threshold disabled (== 0) the timer path is bypassed
/// entirely: even slow callbacks must not increment the counter.
#[test]
fn slow_callback_disabled_when_threshold_zero() {
    const RING: usize = 64;
    const TOTAL: usize = 16;

    let threshold = Arc::new(AtomicU64::new(0));

    let (mut producer, _panics, slow) =
        build_consumer_with_watchdog(RING, Arc::clone(&threshold), move |_event: &FpssEvent| {
            // Genuinely slow — but the threshold is 0 (disabled), so
            // the consumer must skip the timer.
            thread::sleep(Duration::from_millis(20));
        });

    for _ in 0..TOTAL {
        producer.publish(|slot| {
            slot.event = Some(event());
        });
    }
    drop(producer);

    assert_eq!(
        slow.load(Ordering::Relaxed),
        0,
        "slow_callback_count must stay zero when threshold is disabled",
    );
}

// ---------------------------------------------------------------------------
// Multi-generation drain barrier soak coverage
// ---------------------------------------------------------------------------

/// Drive three retired-generation drain flags through the same
/// `Mutex<Vec<Arc<AtomicBool>>>` storage `ThetaDataDxClient::prev_drained`
/// and `TdxFpssHandle::prev_drained` use. Stagger the drain order
/// (gen `c` flips first, then `a`, then `b`) so the barrier cannot
/// pass by inspecting only the most-recently pushed entry — a
/// single-slot tracker would observe `flag_c` flipping at 40ms and
/// report quiescence while `flag_a` / `flag_b` were still pending.
///
/// This is the direct multi-generation drive: 3 `Arc<AtomicBool>`
/// flags through `prev_drained: Vec<...>`, with `B`'s flag flipping
/// before `A`'s, asserting `await_drain` does not return until all 3
/// are flipped. The full live-`FpssClient` stacking path is
/// structurally tested by
/// `callback_triggered_stop_then_await_drain_completes` (single-gen);
/// this case covers the multi-gen sequencing of those flags into a Vec.
///
/// The barrier loop here is byte-identical to the production
/// `ThetaDataDxClient::await_drain` / `tdx_fpss_await_drain` poll: lock,
/// `retain` away drained entries, return on empty.
#[test]
fn multi_gen_drain_waits_for_all_retired_sessions() {
    use std::sync::atomic::AtomicBool;

    const WATCHDOG: Duration = Duration::from_secs(5);

    let prev_drained: Mutex<Vec<Arc<AtomicBool>>> = Mutex::new(Vec::new());

    // Three retired generations layered on top of each other before
    // any has drained — the stacked stop/start/stop scenario.
    let flag_a = Arc::new(AtomicBool::new(false));
    let flag_b = Arc::new(AtomicBool::new(false));
    let flag_c = Arc::new(AtomicBool::new(false));
    {
        let mut g = prev_drained.lock().unwrap();
        g.push(Arc::clone(&flag_a));
        g.push(Arc::clone(&flag_b));
        g.push(Arc::clone(&flag_c));
        assert_eq!(
            g.len(),
            3,
            "three retired generations must coexist; pre-fix single-slot \
             tracker would have lost flag_a and flag_b here"
        );
    }

    // Stagger the drain. flag_c (last-pushed) flips FIRST — under the
    // pre-fix code the barrier would have observed `c` and returned
    // immediately while `a` and `b` were still pending.
    let f_c = Arc::clone(&flag_c);
    let f_a = Arc::clone(&flag_a);
    let f_b = Arc::clone(&flag_b);
    let helper_c = thread::spawn(move || {
        thread::sleep(Duration::from_millis(40));
        f_c.store(true, Ordering::Release);
    });
    let helper_a = thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        f_a.store(true, Ordering::Release);
    });
    let helper_b = thread::spawn(move || {
        thread::sleep(Duration::from_millis(120));
        f_b.store(true, Ordering::Release);
    });

    // Production await_drain semantics: lock, GC drained entries,
    // return when empty.
    let started = Instant::now();
    let drained = loop {
        let all_drained = {
            let mut g = prev_drained.lock().unwrap();
            g.retain(|f| !f.load(Ordering::Acquire));
            g.is_empty()
        };
        if all_drained {
            break true;
        }
        if Instant::now() >= started + WATCHDOG {
            break false;
        }
        thread::sleep(Duration::from_millis(1));
    };
    let elapsed = started.elapsed();

    helper_a.join().unwrap();
    helper_b.join().unwrap();
    helper_c.join().unwrap();

    assert!(drained, "multi-gen drain timed out; barrier should have returned `true` once all three flags flipped");

    // The slowest helper flips at 120ms. The barrier MUST NOT return
    // before that point. The pre-fix single-slot tracker would have
    // returned when `flag_c` flipped at 40ms, so the lower bound below
    // is a strict regression gate against HIGH-001.
    assert!(
        elapsed >= Duration::from_millis(110),
        "barrier returned in {elapsed:?} -- below the slowest flag's \
         120ms flip delay. The pre-fix single-slot tracker would have \
         observed flag_c (last-pushed) flipping at 40ms and returned \
         immediately, leaving flag_a and flag_b's still-firing callbacks \
         to fire on freed FFI ctx. This is the HIGH-001 regression.",
    );

    // Every flag must be `true` and the Vec must be GC'd to empty —
    // a long-lived handle that cycles through many sessions cannot
    // accumulate entries past their drain.
    assert!(flag_a.load(Ordering::Acquire));
    assert!(flag_b.load(Ordering::Acquire));
    assert!(flag_c.load(Ordering::Acquire));
    assert!(
        prev_drained.lock().unwrap().is_empty(),
        "post-drain Vec must be empty -- lazy GC walked every flag"
    );
}

/// `await_drain` returns `false` exactly when the barrier expires
/// with at least one generation still pending. Guards the timeout
/// path that the FFI `_free` log line surfaces to operators.
#[test]
fn multi_gen_await_drain_times_out_with_stuck_generation() {
    use std::sync::atomic::AtomicBool;

    let prev_drained: Mutex<Vec<Arc<AtomicBool>>> = Mutex::new(Vec::new());
    let stuck = Arc::new(AtomicBool::new(false));
    let drained_already = Arc::new(AtomicBool::new(true));
    prev_drained.lock().unwrap().push(Arc::clone(&stuck));
    prev_drained
        .lock()
        .unwrap()
        .push(Arc::clone(&drained_already));

    let started = Instant::now();
    let result = loop {
        let all_drained = {
            let mut g = prev_drained.lock().unwrap();
            g.retain(|f| !f.load(Ordering::Acquire));
            g.is_empty()
        };
        if all_drained {
            break true;
        }
        if Instant::now() >= started + Duration::from_millis(50) {
            break false;
        }
        thread::sleep(Duration::from_millis(1));
    };

    assert!(
        !result,
        "stuck generation must drive the barrier into a timeout"
    );
    let g = prev_drained.lock().unwrap();
    assert_eq!(
        g.len(),
        1,
        "drained entry GC'd on the way down; stuck flag remains"
    );
}

// ---------------------------------------------------------------------------
// Pull-iter delivery — drain semantics, shutdown observability
// ---------------------------------------------------------------------------

/// Pull-iter EventIterator must drain every event the producer side
/// pushed onto the queue, in arrival order, even after the upstream
/// `client_shutdown` flag flips. Asserts the queue → iterator
/// round-trip without standing up a live FPSS connection.
///
/// 1000 synthetic `MarketOpen` control events into a 4096-slot queue;
/// flip the shutdown flag; drain the iterator and count.
#[test]
fn iter_mode_drains_all_events() {
    use std::sync::atomic::AtomicBool;

    use super::events::FpssControl;
    use super::EventIterator;

    const N: usize = 1000;
    let queue: Arc<crossbeam_queue::ArrayQueue<FpssEvent>> =
        Arc::new(crossbeam_queue::ArrayQueue::new(4096));
    let shutdown = Arc::new(AtomicBool::new(false));

    // Push N events synchronously — capacity 4096 > N so no overflow
    // is possible. Use `MarketOpen` control variants because they have
    // no payload and clone is a discriminant-only copy.
    for _ in 0..N {
        queue
            .push(FpssEvent::Control(FpssControl::MarketOpen))
            .expect("push must succeed under capacity");
    }
    assert_eq!(queue.len(), N, "all events queued before shutdown signal");

    // Flip shutdown so the iterator's wait loop returns once the
    // queue empties. The iterator must still yield every queued event
    // before observing the empty queue + shutdown combination and
    // returning None.
    shutdown.store(true, Ordering::Release);

    let iter = EventIterator::for_test(Arc::clone(&queue), Arc::clone(&shutdown));
    let mut count = 0usize;
    while let Some(evt) = iter.next() {
        match evt {
            FpssEvent::Control(FpssControl::MarketOpen) => count += 1,
            other => panic!("unexpected event variant in drain: {other:?}"),
        }
    }
    assert_eq!(
        count, N,
        "iterator must yield every queued event before signalling end"
    );
    assert_eq!(
        queue.len(),
        0,
        "queue must be empty once iterator returned None"
    );
}

/// `EventIterator::close` retires the iterator without forcing the
/// upstream client to shut down. Asserts that subsequent `next()`
/// drains residual events and then returns `None`.
#[test]
fn iter_close_retires_without_upstream_shutdown() {
    use std::sync::atomic::AtomicBool;

    use super::events::FpssControl;
    use super::EventIterator;

    let queue: Arc<crossbeam_queue::ArrayQueue<FpssEvent>> =
        Arc::new(crossbeam_queue::ArrayQueue::new(64));
    let shutdown = Arc::new(AtomicBool::new(false));

    // Three events queued; iterator does NOT see upstream shutdown.
    for _ in 0..3 {
        queue
            .push(FpssEvent::Control(FpssControl::MarketClose))
            .expect("push must succeed under capacity");
    }

    let iter = EventIterator::for_test(Arc::clone(&queue), Arc::clone(&shutdown));
    iter.close();

    // After close + with shutdown still false, the iterator must
    // drain the residual events and then return None — without
    // waiting on the upstream shutdown signal.
    let drained: Vec<FpssEvent> = std::iter::from_fn(|| iter.next()).collect();
    assert_eq!(
        drained.len(),
        3,
        "close-then-drain must yield every queued event"
    );
    assert!(
        !shutdown.load(Ordering::Acquire),
        "iterator close must NOT touch the upstream shutdown flag"
    );
}

/// `EventIterator::try_next` returns immediately on an empty queue
/// even if the upstream is still live, and reports the typed
/// `NextEvent::Timeout` outcome so the polling caller can distinguish
/// quiet-but-live from terminal end-of-stream. Pins the non-blocking
/// polling contract used by the C ABI's `tdx_*_event_iter_next` with
/// `timeout_ms = 0`.
#[test]
fn iter_try_next_does_not_block_on_empty_queue() {
    use std::sync::atomic::AtomicBool;

    use super::{EventIterator, NextEvent};

    let queue: Arc<crossbeam_queue::ArrayQueue<FpssEvent>> =
        Arc::new(crossbeam_queue::ArrayQueue::new(8));
    let shutdown = Arc::new(AtomicBool::new(false));
    let iter = EventIterator::for_test(queue, shutdown);

    let started = Instant::now();
    match iter.try_next() {
        NextEvent::Timeout => {}
        other => panic!("empty-but-live queue must return Timeout, got {other:?}"),
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(5),
        "try_next must not pay the polling cadence; took {elapsed:?}",
    );
}

/// `EventIterator::try_next` must return `NextEvent::Closed` once the
/// queue has been drained on a stopped session. Earlier it returned
/// `Option<FpssEvent>` and overloaded `None` to mean both
/// "empty-but-live" AND "drained + shutdown". The C ABI's non-blocking
/// poll path inherited that ambiguity so a C client calling
/// `tdx_fpss_event_iter_next(.., 0)` after `stop_streaming()` would
/// see rc `1` (timeout) forever instead of rc `-1` (terminal). Pins
/// the typed-enum contract that lets every binding map `Closed` to
/// its terminal sentinel.
#[test]
fn iter_try_next_returns_closed_after_drain() {
    use std::sync::atomic::AtomicBool;

    use super::events::FpssControl;
    use super::{EventIterator, NextEvent};

    let queue: Arc<crossbeam_queue::ArrayQueue<FpssEvent>> =
        Arc::new(crossbeam_queue::ArrayQueue::new(8));
    let shutdown = Arc::new(AtomicBool::new(false));

    // One residual event the iterator should drain BEFORE seeing the
    // terminal Closed signal — the contract is "drain residuals, then
    // close" so subscribers don't lose tail events on shutdown.
    queue
        .push(FpssEvent::Control(FpssControl::MarketClose))
        .expect("push must succeed under capacity");

    // Simulate `stop_streaming()` flipping the upstream shutdown
    // signal while the queue still has the residual.
    shutdown.store(true, Ordering::Release);

    let iter = EventIterator::for_test(Arc::clone(&queue), Arc::clone(&shutdown));

    // First call drains the residual.
    match iter.try_next() {
        NextEvent::Ready(FpssEvent::Control(FpssControl::MarketClose)) => {}
        other => panic!("expected residual MarketClose, got {other:?}"),
    }

    // Second call: queue empty AND shutdown asserted → Closed
    // (NOT Timeout). This is the case the FFI's non-blocking poll
    // path was previously collapsing to rc `1`.
    match iter.try_next() {
        NextEvent::Closed => {}
        other => panic!("drained + shutdown must return Closed, got {other:?}"),
    }
    // Repeat: still Closed (idempotent terminal state). A C client
    // calling `tdx_fpss_event_iter_next(.., 0)` in a polling loop
    // after `stop_streaming()` must see rc `-1` on every subsequent
    // call instead of bouncing between rc `1` and rc `-1`.
    match iter.try_next() {
        NextEvent::Closed => {}
        other => panic!("Closed must be sticky once asserted, got {other:?}"),
    }
}

/// Pull-iter `next_timeout` must distinguish a timeout (no event in
/// the wait window, upstream still live) from a terminal close
/// (upstream shut down + queue drained). Earlier both cases were
/// `None`, which forced every language binding to guess and led to
/// false EOFs on quiet-but-live streams.
///
/// Drives an iterator with no events for 200 ms, asserts the timeout
/// outcome; then pushes one event and asserts the ready outcome on
/// the next call.
#[test]
fn iter_returns_timeout_then_event_on_quiet_then_active_stream() {
    use std::sync::atomic::AtomicBool;

    use super::events::FpssControl;
    use super::{EventIterator, NextEvent};

    let queue: Arc<crossbeam_queue::ArrayQueue<FpssEvent>> =
        Arc::new(crossbeam_queue::ArrayQueue::new(8));
    let shutdown = Arc::new(AtomicBool::new(false));
    let iter = EventIterator::for_test(Arc::clone(&queue), Arc::clone(&shutdown));

    // Quiet-but-live: no events queued, no shutdown signal. The
    // call MUST return Timeout — NOT Closed — so a downstream loop
    // re-polls instead of false-EOF'ing.
    let outcome = iter.next_timeout(Duration::from_millis(200));
    match outcome {
        NextEvent::Timeout => {}
        other => panic!("quiet-but-live stream must return Timeout, got {other:?}"),
    }
    assert!(
        !shutdown.load(Ordering::Acquire),
        "Timeout must not flip the upstream shutdown signal"
    );

    // Now publish an event and assert Ready surfaces it.
    queue
        .push(FpssEvent::Control(FpssControl::MarketOpen))
        .expect("push must succeed under capacity");
    let outcome = iter.next_timeout(Duration::from_millis(200));
    match outcome {
        NextEvent::Ready(FpssEvent::Control(FpssControl::MarketOpen)) => {}
        other => panic!("active stream must return Ready(MarketOpen), got {other:?}"),
    }
}

/// Pull-iter `next_timeout` must return `Closed` once the upstream
/// `client_shutdown` flag has flipped AND the queue is drained.
/// Mirrors the language-binding contract: Python `__next__` raises
/// `StopIteration` on `Closed`, TypeScript `next()` resolves to
/// `null`, the C ABI returns `-1`. The blocking `Iterator::next` impl
/// continues to return `None` on the same condition.
#[test]
fn iter_returns_closed_after_stop_streaming() {
    use std::sync::atomic::AtomicBool;

    use super::events::FpssControl;
    use super::{EventIterator, NextEvent};

    let queue: Arc<crossbeam_queue::ArrayQueue<FpssEvent>> =
        Arc::new(crossbeam_queue::ArrayQueue::new(8));
    let shutdown = Arc::new(AtomicBool::new(false));

    // Two residual events the iterator should drain BEFORE seeing
    // the terminal Closed signal — the contract is "drain residuals,
    // then close" so subscribers don't lose tail events on shutdown.
    queue
        .push(FpssEvent::Control(FpssControl::MarketClose))
        .expect("push must succeed under capacity");
    queue
        .push(FpssEvent::Control(FpssControl::MarketOpen))
        .expect("push must succeed under capacity");

    // Simulate `stop_streaming()` flipping the upstream shutdown
    // signal while the queue still has pending items.
    shutdown.store(true, Ordering::Release);

    let iter = EventIterator::for_test(Arc::clone(&queue), Arc::clone(&shutdown));

    // First two calls drain the residuals — in the order they were
    // pushed (`MarketClose` then `MarketOpen`).
    match iter.next_timeout(Duration::from_millis(200)) {
        NextEvent::Ready(FpssEvent::Control(FpssControl::MarketClose)) => {}
        other => panic!("expected residual MarketClose, got {other:?}"),
    }
    match iter.next_timeout(Duration::from_millis(200)) {
        NextEvent::Ready(FpssEvent::Control(FpssControl::MarketOpen)) => {}
        other => panic!("expected residual MarketOpen, got {other:?}"),
    }

    // Third call: queue empty AND shutdown asserted → Closed.
    match iter.next_timeout(Duration::from_millis(200)) {
        NextEvent::Closed => {}
        other => panic!("drained + shutdown must return Closed, got {other:?}"),
    }
    // Repeat: still Closed (idempotent terminal state).
    match iter.next_timeout(Duration::from_millis(200)) {
        NextEvent::Closed => {}
        other => panic!("Closed must be sticky once asserted, got {other:?}"),
    }

    // Blocking `Iterator::next` reports the terminal state as `None`
    // on the same condition.
    let mut iter_again = EventIterator::for_test(Arc::clone(&queue), shutdown);
    assert!(
        Iterator::next(&mut iter_again).is_none(),
        "blocking Iterator::next must return None once queue is drained + shutdown asserted"
    );
}

/// Empty Vec must short-circuit. This is the `Idle` / "never streamed"
/// path the FFI free contract relies on to skip the timeout-log
/// emission.
#[test]
fn multi_gen_await_drain_short_circuits_when_no_generations_retired() {
    use std::sync::atomic::AtomicBool;

    let prev_drained: Mutex<Vec<Arc<AtomicBool>>> = Mutex::new(Vec::new());
    let started = Instant::now();
    let drained = {
        let mut g = prev_drained.lock().unwrap();
        g.retain(|f| !f.load(Ordering::Acquire));
        g.is_empty()
    };
    let elapsed = started.elapsed();

    assert!(drained, "empty Vec is the fully-drained state");
    assert!(
        elapsed < Duration::from_millis(5),
        "empty-Vec short-circuit must NOT pay the poll cadence"
    );
}

/// Pull-iter `EventIterator` MUST NOT false-EOF while the consumer
/// thread is still pushing tail events into the queue.
///
/// Earlier the iterator's terminal predicate keyed off the global
/// I/O-thread shutdown flag. `stop_streaming()` flipped that flag
/// BEFORE the Disruptor consumer thread had finished draining the ring
/// buffer into the `ArrayQueue` shared with the iterator, so any
/// `next_timeout` call observed between those two moments saw an
/// empty queue + asserted shutdown and returned `Closed`, dropping the
/// tail of events on the floor.
///
/// The fix moved the predicate to a dedicated `iter_closed` flag
/// flipped by a drop guard captured inside the consumer closure. The
/// closure is dropped only after the producer is dropped at io_loop
/// exit, which only happens after the consumer thread has joined and
/// every in-flight event has been pushed onto the queue.
///
/// This test simulates the race directly without standing up a live
/// FPSS session:
///
/// 1. Pre-fill the queue with `N` events (the "tail" the consumer is
///    still flushing in the real path).
/// 2. Flip the global `shutdown` flag — modelling `stop_streaming()`.
/// 3. Drain via `next_timeout` and count `Ready` events until `Closed`.
///
/// Pre-fix this returned `Closed` immediately (count = 0). Post-fix
/// the iterator drains all `N` events first because its predicate
/// (`iter_closed`) is still `false` — the consumer drop guard hasn't
/// fired yet. We flip `iter_closed = true` AFTER the test has
/// observed the events, mirroring the production wiring.
#[test]
fn iter_does_not_false_eof_during_drain() {
    use std::sync::atomic::AtomicBool;

    use super::events::FpssControl;
    use super::{EventIterator, NextEvent};

    const N: usize = 100;
    let queue: Arc<crossbeam_queue::ArrayQueue<FpssEvent>> =
        Arc::new(crossbeam_queue::ArrayQueue::new(256));
    // Models the consumer-side drain-guard flag. Stays `false` until
    // we explicitly flip it below — even though the simulated global
    // shutdown has already fired.
    let iter_closed = Arc::new(AtomicBool::new(false));

    // Pre-fill the queue with N tail events. In the real path these
    // are the events the Disruptor consumer thread is still flushing
    // into the queue at the moment `stop_streaming()` is called.
    for _ in 0..N {
        queue
            .push(FpssEvent::Control(FpssControl::MarketOpen))
            .expect("push must succeed under capacity");
    }
    assert_eq!(queue.len(), N, "tail events queued before drain");

    // The simulated client.shutdown flag — earlier the iterator
    // would terminate on this. Post-9.1.0 it is irrelevant to the
    // iterator and only acts as scenario flavour.
    let global_shutdown = Arc::new(AtomicBool::new(true));
    assert!(
        global_shutdown.load(Ordering::Acquire),
        "global shutdown is asserted while tail events still pending in queue"
    );

    let iter = EventIterator::for_test(Arc::clone(&queue), Arc::clone(&iter_closed));

    // Drain via `next_timeout` and count `Ready`. With the new
    // predicate (`iter_closed = false`), the iterator MUST surface
    // every queued event as `Ready`. Pre-fix it would surface
    // `Closed` immediately because the global shutdown flag was
    // asserted.
    let mut ready_count: usize = 0;
    let drain_started = Instant::now();
    loop {
        match iter.next_timeout(Duration::from_millis(50)) {
            NextEvent::Ready(_) => ready_count += 1,
            NextEvent::Timeout => {
                // Queue empty but iterator still live — this is the
                // window between "consumer pushed last event" and
                // "drop guard fires". In production the drop guard
                // is automatic; here we flip it manually to release
                // the iterator.
                iter_closed.store(true, Ordering::Release);
            }
            NextEvent::Closed => break,
        }
        // Soak hard cap so a regression that loops forever fails
        // loudly instead of hanging CI.
        assert!(
            drain_started.elapsed() < Duration::from_secs(5),
            "drain exceeded 5s safety cap; ready_count={ready_count}"
        );
    }

    assert_eq!(
        ready_count, N,
        "iterator must drain every queued event before Closed; earlier false-EOF \
         dropped tail events on shutdown"
    );
    assert_eq!(
        queue.len(),
        0,
        "queue fully drained once iterator surfaced Closed"
    );
}

// ---------------------------------------------------------------------------
// Direct-consumer (single-ring) poller soak tests
// ---------------------------------------------------------------------------
//
// These drive `FpssEventPoller` against a producer built by
// `io_loop::build_poller_producer` and published into by hand — no TLS,
// no I/O thread — so the poller's drain ordering, the EOF-drain
// guarantee, and the terminal `PollOutcome::Shutdown` contract are
// pinned without live credentials. Dropping the producer is the test's
// stand-in for the I/O thread exiting and dropping the ring producer at
// `io_loop` scope exit, which is what stores the ring's shutdown
// sequence.

/// `poll_batch` drains the currently-available batch in publish order,
/// then reports `Shutdown` only after the producer has been dropped AND
/// every published event has been drained — the EOF-drain guarantee.
#[test]
fn poller_poll_batch_drains_then_reports_shutdown() {
    use super::events::FpssControl;
    use super::{FpssEventPoller, PollOutcome};
    use disruptor::Producer;

    let (mut producer, poller) = super::io_loop::build_poller_producer(64);
    let mut poller = FpssEventPoller::for_test(poller);

    // Publish three control frames the projection surfaces publicly.
    let kinds = [
        FpssControl::MarketOpen,
        FpssControl::MarketClose,
        FpssControl::Reconnected,
    ];
    for k in &kinds {
        let k = k.clone();
        producer.publish(|slot| {
            slot.event = super::events::FpssEventInternal::Control(k);
        });
    }

    // Drain the available batch. With a single producer publishing
    // three events before any poll, the batch should carry all three.
    let mut seen: Vec<FpssEvent> = Vec::new();
    let outcome = poller.poll_batch(|evt| seen.push(evt.clone()));
    assert_eq!(
        outcome,
        PollOutcome::Drained(3),
        "poll_batch must drain the full available batch"
    );
    assert!(
        matches!(
            seen.as_slice(),
            [
                FpssEvent::Control(FpssControl::MarketOpen),
                FpssEvent::Control(FpssControl::MarketClose),
                FpssEvent::Control(FpssControl::Reconnected),
            ]
        ),
        "events must arrive in publish order, got {seen:?}"
    );

    // Empty ring, producer still live → zero-length drain, NOT shutdown.
    let outcome = poller.poll_batch(|_| panic!("no events expected on empty live ring"));
    assert_eq!(
        outcome,
        PollOutcome::Drained(0),
        "empty-but-live ring must report Drained(0), not Shutdown"
    );

    // Drop the producer: this stores the ring's shutdown sequence (no
    // consumer thread to join in poller mode). The poller now reports
    // terminal shutdown.
    drop(producer);
    let outcome = poller.poll_batch(|_| panic!("no events expected after shutdown"));
    assert_eq!(
        outcome,
        PollOutcome::Shutdown,
        "poll_batch must report Shutdown once the producer is dropped and the ring is drained"
    );
    // Idempotent terminal.
    assert_eq!(poller.poll_batch(|_| {}), PollOutcome::Shutdown);
}

/// A tail of events published right before the producer drop must all be
/// delivered before `poll_batch` reports `Shutdown` — the drain happens
/// on the poll that follows the last publish, never lost to a premature
/// terminal.
#[test]
fn poller_poll_batch_drains_tail_published_before_drop() {
    use super::events::FpssControl;
    use super::{FpssEventPoller, PollOutcome};
    use disruptor::Producer;

    let (mut producer, poller) = super::io_loop::build_poller_producer(64);
    let mut poller = FpssEventPoller::for_test(poller);

    const TAIL: usize = 10;
    for _ in 0..TAIL {
        producer.publish(|slot| {
            slot.event = super::events::FpssEventInternal::Control(FpssControl::MarketOpen);
        });
    }
    // Drop BEFORE the first poll: the shutdown sequence is set, but the
    // tail is still in the ring and must drain first.
    drop(producer);

    let mut delivered = 0usize;
    while let PollOutcome::Drained(n) = poller.poll_batch(|_| {}) {
        delivered += n;
    }
    assert_eq!(
        delivered, TAIL,
        "every event published before the drop must drain before Shutdown"
    );
}

/// `run` blocks the calling thread, drains every event a separate
/// producer thread publishes (in order), and returns once the producer
/// is dropped and the ring is empty.
#[test]
fn poller_run_receives_events_in_order_then_returns() {
    use super::events::FpssControl;
    use super::FpssEventPoller;
    use disruptor::Producer;

    let (mut producer, poller) = super::io_loop::build_poller_producer(256);
    let poller = FpssEventPoller::for_test(poller);

    const N: i32 = 500;
    // Publish on a separate thread; `run` drives the ring on this one.
    let producer_thread = thread::spawn(move || {
        for i in 0..N {
            // Encode the sequence number in `ServerError.message` so the
            // consumer can assert exact ordering.
            let msg = i.to_string();
            producer.publish(|slot| {
                slot.event = super::events::FpssEventInternal::Control(FpssControl::ServerError {
                    message: msg,
                });
            });
        }
        // Drop the producer to set the ring shutdown sequence so `run`
        // returns after draining.
        drop(producer);
    });

    let received: Arc<Mutex<Vec<i32>>> = Arc::new(Mutex::new(Vec::new()));
    let received_run = Arc::clone(&received);
    poller.run(move |evt| {
        if let FpssEvent::Control(FpssControl::ServerError { message }) = evt {
            received_run
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(message.parse().expect("sequence message must parse"));
        }
    });

    producer_thread.join().expect("producer thread must join");

    let got = received
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(
        got.len(),
        N as usize,
        "run must deliver every published event"
    );
    let expected: Vec<i32> = (0..N).collect();
    assert_eq!(*got, expected, "run must preserve publish order");
}

/// A handler that panics mid-batch must not lose the unread tail of that
/// batch. The Disruptor `EventGuard` advances the consumer cursor to the
/// end of the available batch on drop regardless of how many events were
/// read, so an unwind across the drain loop would silently skip the
/// remaining events. `poll_batch` isolates each handler call under
/// `catch_unwind`, so the panicking event is the only one dropped and the
/// rest of the batch is still delivered.
#[test]
fn poller_poll_batch_isolates_handler_panic_and_keeps_batch_tail() {
    use super::events::FpssControl;
    use super::{FpssEventPoller, PollOutcome};
    use disruptor::Producer;

    let (mut producer, poller) = super::io_loop::build_poller_producer(256);
    let mut poller = FpssEventPoller::for_test(poller);

    // Publish a 5-event batch BEFORE polling so a single `poll_batch`
    // call drains all five under one `EventGuard`.
    for i in 0..5i32 {
        let msg = i.to_string();
        producer.publish(|slot| {
            slot.event = super::events::FpssEventInternal::Control(FpssControl::ServerError {
                message: msg,
            });
        });
    }

    let mut received: Vec<i32> = Vec::new();
    let outcome = poller.poll_batch(|evt| {
        if let FpssEvent::Control(FpssControl::ServerError { message }) = evt {
            let seq: i32 = message.parse().expect("sequence message must parse");
            // Panic on the middle event of the batch.
            assert_ne!(seq, 2, "handler panics on event 2");
            received.push(seq);
        }
    });

    // The four non-panicking events were delivered; only event 2 was lost.
    assert_eq!(
        received,
        vec![0, 1, 3, 4],
        "tail after the panic must survive"
    );
    assert_eq!(
        outcome,
        PollOutcome::Drained(4),
        "drained count excludes the panicking event"
    );
    assert_eq!(poller.panic_count(), 1, "one caught handler panic");

    drop(producer);
}

/// `run` returns promptly once the producer is dropped on another
/// thread mid-stream, after draining the in-flight tail. Guards against
/// a `run` loop that hangs when the producer goes away.
#[test]
fn poller_run_terminates_when_producer_dropped() {
    use super::events::FpssControl;
    use super::FpssEventPoller;
    use disruptor::Producer;

    let (mut producer, poller) = super::io_loop::build_poller_producer(64);
    let poller = FpssEventPoller::for_test(poller);

    let producer_thread = thread::spawn(move || {
        for _ in 0..5 {
            producer.publish(|slot| {
                slot.event = super::events::FpssEventInternal::Control(FpssControl::MarketOpen);
            });
        }
        // Hold the producer briefly so `run` observes an idle ring and
        // exercises its adaptive-wait branch, then drop to terminate.
        thread::sleep(Duration::from_millis(20));
        drop(producer);
    });

    let count = Arc::new(AtomicU64::new(0));
    let count_run = Arc::clone(&count);
    let start = Instant::now();
    poller.run(move |_evt| {
        count_run.fetch_add(1, Ordering::Relaxed);
    });
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "run must return promptly after the producer is dropped"
    );
    assert_eq!(
        count.load(Ordering::Relaxed),
        5,
        "run must deliver the in-flight events before returning"
    );
    producer_thread.join().expect("producer thread must join");
}

/// The projection filters the internal-only `Empty` / `Unparseable`
/// ring slots: a published `Unparseable` event is never surfaced to the
/// poller closure, only `Data` / `Control` reach it. Pins the same
/// internal/public split the managed consumer enforces, on the poller
/// drain path.
#[test]
fn poller_poll_batch_filters_internal_only_variants() {
    use super::events::{FpssControl, FpssEventInternal};
    use super::{FpssEventPoller, PollOutcome};
    use disruptor::Producer;

    let (mut producer, poller) = super::io_loop::build_poller_producer(64);
    let mut poller = FpssEventPoller::for_test(poller);

    // Interleave a public Control with an internal-only Unparseable.
    producer.publish(|slot| {
        slot.event = FpssEventInternal::Control(FpssControl::MarketOpen);
    });
    producer.publish(|slot| {
        slot.event = FpssEventInternal::Unparseable;
    });
    producer.publish(|slot| {
        slot.event = FpssEventInternal::Control(FpssControl::MarketClose);
    });

    let mut seen: Vec<FpssEvent> = Vec::new();
    let outcome = poller.poll_batch(|evt| seen.push(evt.clone()));
    // Three slots advanced, but only the two public ones are delivered.
    assert_eq!(
        outcome,
        PollOutcome::Drained(2),
        "Unparseable must not count toward delivered events"
    );
    assert!(
        matches!(
            seen.as_slice(),
            [
                FpssEvent::Control(FpssControl::MarketOpen),
                FpssEvent::Control(FpssControl::MarketClose),
            ]
        ),
        "internal-only variants must be filtered before the closure, got {seen:?}"
    );
}

/// `FpssEventPoller` is `Send` so a caller can build it on the connect
/// thread and move it to the thread that will own the drain loop.
/// Single-consumer safety does not rely on `!Sync`: both drive methods
/// take `&mut self` (`poll_batch`) or `self` (`run`), so the borrow
/// checker already forbids two threads draining the same poller at once
/// without unsynchronised aliasing the caller would have to construct
/// deliberately.
#[test]
fn poller_is_send() {
    use super::FpssEventPoller;
    fn assert_send<T: Send>() {}
    assert_send::<FpssEventPoller>();
}
