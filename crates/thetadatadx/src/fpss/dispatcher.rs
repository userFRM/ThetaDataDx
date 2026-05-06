//! Streaming dispatcher: lock-free queue + drain thread between the FPSS
//! reader and the user-registered callback.
//!
//! # Why this exists
//!
//! The FPSS reader thread owns the TLS socket exclusively. If a slow user
//! callback fires directly on that thread, the reader stalls, the kernel
//! receive buffer fills, and the vendor disconnects the session. The
//! historical mitigation was the LMAX Disruptor ring inside [`super::ring`],
//! which decouples the reader from the consumer thread but pre-allocates a
//! fixed-size ring of `RingEvent` slots and routes every event through the
//! Disruptor's adaptive wait strategy regardless of payload shape.
//!
//! [`StreamingDispatcher`] is a thinner alternative on the user-callback
//! boundary: a single `crossbeam_channel::bounded(8192)` queue and one
//! drain thread. The reader thread does `try_send`; on `Full` the event
//! is dropped and a per-dispatcher counter ticks. Slow callbacks back up
//! the queue, drops occur, but the reader thread never blocks.
//!
//! # SSOT
//!
//! This struct is the single source of truth for the queue + drain-thread
//! orchestration. Bindings (Python, TypeScript, FFI) consume the same
//! `StreamingDispatcher` rather than rebuilding their own variants. The
//! drain loop wraps every user-callback invocation in `catch_unwind` so
//! a panic from binding glue (PyO3 `Python::attach`, napi `ThreadsafeFunction`,
//! C `extern "C" fn`) does NOT take the dispatcher thread down — bindings
//! must NOT add their own panic handling around the same call path.
//!
//! # Capacity
//!
//! `bounded(8192)` matches the `crossbeam_bounded_8192` row in
//! `benches/streaming_channels.rs`. At the steady-state rate of ~10–30 k
//! events/s for a typical multi-contract subscription, an 8 192-slot
//! queue absorbs ~270–800 ms of consumer-side stall before drops begin —
//! enough headroom for a normal GC pause or a slow Python listener,
//! without holding so much memory that the dispatcher becomes the de
//! facto buffer for a multi-second stall.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{bounded, Sender, TrySendError};

use super::events::FpssEvent;

/// Bounded queue capacity between the FPSS reader thread and the
/// dispatcher drain thread. Verified against
/// `benches/streaming_channels.rs::crossbeam_bounded_8192`.
const QUEUE_CAPACITY: usize = 8_192;

/// Lock-free queue + dedicated drain thread between the FPSS reader and
/// the user-registered callback.
///
/// The reader thread calls [`Self::send`] (non-blocking `try_send`); the
/// drain thread owns the receive end of the queue and invokes the
/// user's callback for every event it dequeues. On overflow, [`Self::send`]
/// drops the event and increments a counter exposed via
/// [`Self::dropped_count`].
pub struct StreamingDispatcher {
    /// Producer-side handle to the bounded queue. Cloned per send-site
    /// so the FPSS reader and the ping thread share the same drain
    /// thread without coordinating directly.
    ///
    /// Wrapped in `Option` so [`Self::shutdown`] can drop the sender
    /// while the rest of `self` is still being moved into the
    /// destructor: `Drop` requires `&mut self`, which precludes
    /// partial-moves of bare fields.
    sender: Option<Sender<FpssEvent>>,
    /// Drain thread that owns the receiver end of the queue and invokes
    /// the user callback. `Some` until [`Self::shutdown`] joins it.
    handle: Option<JoinHandle<()>>,
    /// Count of events dropped because the bounded queue was full when
    /// the reader called [`Self::send`]. Snapshot via
    /// [`Self::dropped_count`]. This is the user-facing "queue overflow"
    /// metric; disconnected sends (post-shutdown race) are tracked
    /// separately on [`Self::disconnected`].
    dropped: Arc<AtomicU64>,
    /// Count of events that could not be enqueued because the receiver
    /// end was already disconnected. This is shutdown-race noise (drain
    /// thread already exited), tracked separately so the public drop
    /// metric stays a clean overflow signal. Snapshot via
    /// [`Self::disconnected_count`].
    disconnected: Arc<AtomicU64>,
    /// Count of user-callback panics caught by the drain loop. The
    /// drain thread keeps running on panic; this counter is the only
    /// surfacing mechanism on a long-lived stream. Snapshot via
    /// [`Self::panic_count`].
    panic_count: Arc<AtomicU64>,
}

impl StreamingDispatcher {
    /// Spawn a drain thread bound to the given user callback.
    ///
    /// The drain thread runs until the producer side of the queue is
    /// closed (either through [`Self::shutdown`] or because every
    /// [`Sender`] clone has been dropped), at which point [`Receiver::iter`]
    /// terminates and the thread exits.
    ///
    /// Each callback invocation is wrapped in [`std::panic::catch_unwind`]
    /// so a panic from user code (Rust closure / PyO3 callable / napi
    /// `ThreadsafeFunction` / C `extern "C" fn`) does NOT kill the
    /// dispatcher thread. Panics are counted on [`Self::panic_count`]
    /// and logged at `error` level, then draining continues.
    ///
    /// # Panics
    ///
    /// Panics if the OS refuses to spawn the named drain thread. This
    /// matches the existing FPSS reader/ping thread spawns: a failure
    /// here means the host is out of thread budget and recovery from
    /// the streaming layer is not meaningful.
    #[must_use]
    pub fn spawn(callback: Box<dyn Fn(&FpssEvent) + Send + 'static>) -> Self {
        let (sender, receiver) = bounded::<FpssEvent>(QUEUE_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let disconnected = Arc::new(AtomicU64::new(0));
        let panic_count = Arc::new(AtomicU64::new(0));

        let panic_count_thread = Arc::clone(&panic_count);
        let handle = thread::Builder::new()
            .name("fpss-dispatcher".to_owned())
            .spawn(move || {
                // `Receiver::iter` blocks until a value is ready or all
                // senders are dropped; the loop exits cleanly when the
                // last sender goes away (i.e., on `shutdown` or when the
                // owning `StreamingDispatcher` is dropped).
                //
                // Each invocation is wrapped in `catch_unwind` so a
                // panic from user code (or from binding glue such as
                // PyO3's `Python::attach` during interpreter teardown)
                // does NOT kill the dispatcher thread. A killed
                // dispatcher would surface only later as a join panic
                // in `shutdown()` and every queued event after the
                // panic would be silently lost.
                //
                // `AssertUnwindSafe` is sound here because the
                // callback's captured state lives behind the
                // `Box<dyn Fn>`; any user-visible side effects observable
                // across a panic boundary are the user's own
                // responsibility, not the dispatcher's.
                for event in receiver.iter() {
                    if catch_unwind(AssertUnwindSafe(|| callback(&event))).is_err() {
                        panic_count_thread.fetch_add(1, Ordering::Relaxed);
                        tracing::error!(
                            target: "thetadatadx::fpss::dispatcher",
                            "user callback panicked; dispatcher continuing — \
                             panic_count incremented",
                        );
                    }
                }
            })
            .expect("spawn fpss-dispatcher thread");

        Self {
            sender: Some(sender),
            handle: Some(handle),
            dropped,
            disconnected,
            panic_count,
        }
    }

    /// Enqueue an event for the drain thread.
    ///
    /// Non-blocking. On a full queue the event is dropped and the
    /// queue-full counter ([`Self::dropped_count`]) is incremented;
    /// callers should log the counter delta at `warn` level on a
    /// periodic timer rather than per-drop to avoid log amplification
    /// under sustained overflow. Disconnected sends (drain thread
    /// already exited / shutdown race) feed
    /// [`Self::disconnected_count`] instead so the public drop metric
    /// stays a pure overflow signal.
    pub fn send(&self, event: FpssEvent) {
        let Some(sender) = self.sender.as_ref() else {
            // Sender has already been dropped via `shutdown`. Track
            // as a disconnect (lifecycle noise), not a queue-full drop.
            self.disconnected.fetch_add(1, Ordering::Relaxed);
            return;
        };
        match sender.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                // Drain thread has exited (shutdown completed). Track
                // separately from queue-full drops — disconnect is
                // expected at lifecycle boundaries and should not
                // inflate the user-facing drop metric. No log:
                // shutdown is expected.
                self.disconnected.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Return a producer-side handle that pushes onto the same
    /// bounded queue and shares the same dropped/disconnected counters.
    ///
    /// Used to give the FPSS reader thread its own send handle without
    /// exposing the [`StreamingDispatcher`] itself, which is owned by
    /// the unified client. The returned [`DispatcherProducer`] is
    /// `Send + Sync + Clone`.
    ///
    /// # Panics
    ///
    /// Panics if called after [`Self::shutdown`] has dropped the
    /// sender. In normal use this cannot happen: `shutdown` consumes
    /// `self`, so any live `&self` reference statically guarantees the
    /// sender is still present.
    #[must_use]
    pub fn producer(&self) -> DispatcherProducer {
        DispatcherProducer {
            sender: self
                .sender
                .as_ref()
                .expect("sender present until shutdown consumes self")
                .clone(),
            dropped: Arc::clone(&self.dropped),
            disconnected: Arc::clone(&self.disconnected),
        }
    }

    /// Snapshot the number of events dropped because the bounded queue
    /// was full when the producer called [`Self::send`].
    ///
    /// Uses `Relaxed` ordering — this counter is observational only,
    /// it does not gate any other memory access. Disconnected-sender
    /// events (lifecycle race) are tracked separately on
    /// [`Self::disconnected_count`] and are NOT included here.
    #[must_use]
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Snapshot the number of events that could not be enqueued
    /// because the receiver end was already disconnected (drain thread
    /// exited / shutdown race). In practice this counter typically
    /// stays at 0 — disconnect only happens during the brief window
    /// between [`Self::shutdown`] consuming the dispatcher and the
    /// upstream producer noticing.
    #[must_use]
    pub fn disconnected_count(&self) -> u64 {
        self.disconnected.load(Ordering::Relaxed)
    }

    /// Snapshot the number of user-callback panics observed by the
    /// drain loop since [`Self::spawn`].
    ///
    /// The drain loop wraps every callback invocation in
    /// `catch_unwind`; on panic the counter ticks, an `error`-level
    /// `tracing` event is emitted, and the next event is processed.
    /// The dispatcher thread NEVER dies from a user-code panic.
    #[must_use]
    pub fn panic_count(&self) -> u64 {
        self.panic_count.load(Ordering::Relaxed)
    }

    /// Close the queue and join the drain thread.
    ///
    /// Drops the [`Sender`]; the drain thread sees `Receiver::iter`
    /// terminate, processes any events still in the queue, then
    /// returns. This call blocks until the drain thread has exited.
    ///
    /// User-callback panics encountered during draining are caught by
    /// the loop and counted on [`Self::panic_count`]; they do NOT
    /// propagate out of `shutdown`. The only path that can panic here
    /// is an internal Rust bug in the drain wiring itself, which
    /// would re-raise on join.
    pub fn shutdown(mut self) {
        // Drop the sender so the drain thread sees `Receiver::iter`
        // terminate, then join. `take` is safe to use here even though
        // `Drop` will run after this method returns: the destructor
        // tolerates `None` for both fields.
        let handle = self.handle.take();
        drop(self.sender.take());
        if let Some(h) = handle {
            h.join().expect("fpss-dispatcher drain thread panicked");
        }
    }
}

/// Producer-side handle to a [`StreamingDispatcher`]'s queue.
///
/// Cheap to clone (one [`Sender`] clone + two `Arc::clone`). The FPSS
/// reader thread holds one of these and calls [`Self::send`] for every
/// decoded event; the unified client retains the [`StreamingDispatcher`]
/// itself for `shutdown` and `dropped_count` access.
#[derive(Clone)]
pub struct DispatcherProducer {
    sender: Sender<FpssEvent>,
    dropped: Arc<AtomicU64>,
    disconnected: Arc<AtomicU64>,
}

impl DispatcherProducer {
    /// Enqueue an event for the drain thread. Same overflow semantics
    /// as [`StreamingDispatcher::send`]: non-blocking `try_send`,
    /// queue-full drops tick `dropped`, disconnected sends (shutdown
    /// race) tick `disconnected`.
    pub fn send(&self, event: FpssEvent) {
        match self.sender.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.disconnected.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl Drop for StreamingDispatcher {
    fn drop(&mut self) {
        // If `shutdown` was not called explicitly, drop the join
        // handle; the drain thread will still exit cleanly when the
        // sender is dropped (right after this `Drop` runs), and the
        // OS will reap the detached thread. We intentionally do NOT
        // join here: a panicking user callback should not poison
        // the destructor.
        let _ = self.handle.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AOrdering};
    use std::sync::Mutex;
    use std::thread::ThreadId;
    use std::time::{Duration, Instant};

    /// Drain a parking-park busy-wait until `cond` returns true or the
    /// deadline elapses. Used by tests that need to observe asynchronous
    /// drain-thread progress without sleeping a fixed wall-clock time.
    fn wait_until<F: Fn() -> bool>(cond: F, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(1));
        }
        cond()
    }

    #[test]
    fn dispatcher_invokes_callback_for_every_event() {
        let received = Arc::new(AtomicUsize::new(0));
        let received_cb = Arc::clone(&received);
        let dispatcher = StreamingDispatcher::spawn(Box::new(move |_event: &FpssEvent| {
            received_cb.fetch_add(1, AOrdering::Relaxed);
        }));

        const N: usize = 1_000;
        for _ in 0..N {
            dispatcher.send(FpssEvent::Empty);
        }

        dispatcher.shutdown();

        assert_eq!(
            received.load(AOrdering::Relaxed),
            N,
            "every send must reach the callback when the queue never fills",
        );
    }

    #[test]
    fn dispatcher_drops_overflow_events_and_increments_counter() {
        // Block the drain thread inside the callback so the queue fills.
        // A simple `Mutex` held by the test thread does the trick: the
        // callback acquires the mutex on first invocation and blocks
        // until the test releases it after flooding the queue.
        let gate = Arc::new(Mutex::new(()));
        let gate_cb = Arc::clone(&gate);
        let lock = gate.lock().expect("gate lock");

        let dispatcher = StreamingDispatcher::spawn(Box::new(move |_event: &FpssEvent| {
            // First call blocks here until the test thread drops the
            // outer guard. Subsequent calls acquire and release in O(ns).
            let _g = gate_cb.lock().expect("callback gate");
        }));

        // Push enough events to exceed the bounded queue capacity by a
        // healthy margin. With the drain thread blocked, the first
        // QUEUE_CAPACITY + 1 sends fill the channel (the +1 is the event
        // currently held by the callback on the drain thread), and every
        // event past that increments `dropped`.
        let total = QUEUE_CAPACITY + 1_000;
        for _ in 0..total {
            dispatcher.send(FpssEvent::Empty);
        }

        let dropped = dispatcher.dropped_count();
        assert!(
            dropped > 0,
            "queue overflow must register on the dropped counter, got {dropped}",
        );
        assert!(
            dropped <= total as u64,
            "dropped count {dropped} must not exceed total sent {total}",
        );
        // Disconnected counter must not have ticked at all -- the
        // receiver is still very much alive (just blocked on the
        // gate). Splits the metric cleanly along its public contract.
        assert_eq!(
            dispatcher.disconnected_count(),
            0,
            "disconnected counter must remain 0 while the receiver is live",
        );

        // Release the gate so the drain thread can finish and shutdown
        // joins cleanly without leaking the thread.
        drop(lock);
        dispatcher.shutdown();
    }

    #[test]
    fn dispatcher_shutdown_joins_thread_cleanly() {
        let received = Arc::new(AtomicUsize::new(0));
        let received_cb = Arc::clone(&received);
        let dispatcher = StreamingDispatcher::spawn(Box::new(move |_event: &FpssEvent| {
            received_cb.fetch_add(1, AOrdering::Relaxed);
        }));

        for _ in 0..16 {
            dispatcher.send(FpssEvent::Empty);
        }

        // shutdown must not panic, must join the drain thread, and must
        // process every queued event before returning.
        dispatcher.shutdown();
        assert_eq!(received.load(AOrdering::Relaxed), 16);
    }

    /// Inline-mode contract test: a callback registered through the
    /// inline path runs on the same thread as the producer, no
    /// dispatcher thread involved. The dispatcher path, by contrast,
    /// always invokes on the drain thread.
    ///
    /// This test pins the dispatcher's contract — the inline-path
    /// equivalent for the unified `start_streaming_inline` API is
    /// covered in `crates/thetadatadx/tests/streaming_inline.rs` against
    /// the live FPSS reader thread.
    #[test]
    fn dispatcher_callback_runs_on_drain_thread_not_caller() {
        let caller_thread = thread::current().id();
        let observed: Arc<Mutex<Option<ThreadId>>> = Arc::new(Mutex::new(None));
        let observed_cb = Arc::clone(&observed);

        let dispatcher = StreamingDispatcher::spawn(Box::new(move |_event: &FpssEvent| {
            let mut slot = observed_cb.lock().expect("observed lock");
            if slot.is_none() {
                *slot = Some(thread::current().id());
            }
        }));

        dispatcher.send(FpssEvent::Empty);

        let saw_other_thread = wait_until(
            || {
                observed
                    .lock()
                    .expect("observed lock (poll)")
                    .is_some_and(|tid| tid != caller_thread)
            },
            Duration::from_secs(2),
        );

        dispatcher.shutdown();

        assert!(
            saw_other_thread,
            "dispatcher callback must run on the drain thread, not the caller's thread",
        );
    }

    /// HIGH 1 follow-up: a panicking user callback must NOT kill the
    /// dispatcher thread. Subsequent events must still fire, and the
    /// `panic_count` snapshot must reflect every panic observed.
    #[test]
    fn dispatcher_survives_panicking_callback() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_cb = Arc::clone(&calls);
        let panics_to_throw = Arc::new(AtomicUsize::new(3));
        let panics_to_throw_cb = Arc::clone(&panics_to_throw);

        let dispatcher = StreamingDispatcher::spawn(Box::new(move |_event: &FpssEvent| {
            calls_cb.fetch_add(1, AOrdering::Relaxed);
            // Panic on the first three events; succeed afterwards.
            if panics_to_throw_cb
                .fetch_update(AOrdering::Relaxed, AOrdering::Relaxed, |n| {
                    if n > 0 {
                        Some(n - 1)
                    } else {
                        None
                    }
                })
                .is_ok()
            {
                panic!("synthetic user-callback panic for dispatcher resilience test");
            }
        }));

        // Three guaranteed-panic events.
        for _ in 0..3 {
            dispatcher.send(FpssEvent::Empty);
        }
        // Three follow-on events that must still fire.
        for _ in 0..3 {
            dispatcher.send(FpssEvent::Empty);
        }

        // Wait until the callback has been invoked the expected six
        // times (three panicking, three successful) before sampling
        // counters. Avoids racing the drain thread.
        let drained = wait_until(
            || calls.load(AOrdering::Relaxed) >= 6,
            Duration::from_secs(2),
        );
        assert!(drained, "drain thread must process all 6 events");

        assert_eq!(
            dispatcher.panic_count(),
            3,
            "every panicking invocation must tick panic_count",
        );
        assert_eq!(
            dispatcher.dropped_count(),
            0,
            "no overflow should occur on a 6-event burst",
        );
        assert_eq!(
            calls.load(AOrdering::Relaxed),
            6,
            "subsequent non-panicking callbacks must still fire",
        );

        dispatcher.shutdown();
    }
}
