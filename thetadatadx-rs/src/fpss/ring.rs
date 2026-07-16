//! Pre-allocated ring buffer for lock-free FPSS event dispatch.
//!
//! # Architecture
//!
//! ```text
//!  +--------------------+                  +--------------------+
//!  | Blocking TLS       |  publish()       | Event Ring         |
//!  | read thread        |----------------->| (pre-allocated,    |
//!  | (std::thread)      |                  |  lock-free SPSC)   |
//!  +--------------------+                  +---------+----------+
//!                                                    | consumer
//!                                                    v
//!                                          +--------------------+
//!                                          | User handler(F)    |
//!                                          | (runs on           |
//!                                          |  consumer thread)  |
//!                                          +--------------------+
//! ```
//!
//! Pipeline: blocking TLS `read` -> event ring -> user's
//! `FnMut(&StreamEvent)` callback.
//!
//! No tokio, no channels, no async. The blocking read thread IS the ring
//! producer. Events are pre-allocated in the ring buffer (zero allocation on
//! the hot path), and the single-producer barrier uses a plain store (no CAS).
//!
//! # Publish policy
//!
//! Every publish from the io_loop thread — TLS-read data frames AND
//! handshake / reconnect / control frames — goes through
//! `Producer::try_publish`. The blocking `Producer::publish` is
//! **never** called on the io_loop thread: a slow callback that lets
//! the ring fill must NOT wedge the TLS reader, because a wedged
//! reader stops servicing PING heartbeats and the vendor session
//! drops on the wire. On overflow the event is dropped, the shared
//! `dropped` counter increments, and a `warn` is logged.
//!
//! # Wait Strategy
//!
//! [`AdaptiveWaitStrategy`] carries the selected [`WaitMode`] and drives
//! the consumer's per-empty-poll wait. The default (`Spin`) is the fixed
//! low-latency spin+yield ramp tuned for FPSS tick intervals (~100us
//! during active trading); [`DrainWaiter`] adds the stateful `Backoff`
//! idle escalation on top.

use std::hint;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use disruptor::{Producer, RingBufferFull, Sequence};

use super::events::FpssEventInternal;
use crate::config::WaitMode;
use crate::util::cache_padded::CachePadded;

/// Idle-wait strategy for the FPSS event-ring consumer.
///
/// A small `Copy` carrier — the disruptor's `WaitStrategy` trait is
/// `Copy + Send`, so it is passed by value with no indirection. It holds
/// the selected [`WaitMode`] and the sleep interval used by
/// [`WaitMode::Park`] and [`WaitMode::Backoff`]; the drain loop reads it
/// on every ring-empty poll.
///
/// # CPU vs latency
///
/// [`WaitMode::Spin`] and [`WaitMode::BusySpin`] **both hold ~100% of one
/// core** while the stream is connected — they differ only in scheduler
/// jitter, not CPU. Only [`WaitMode::Park`] and [`WaitMode::Backoff`]
/// lower idle CPU, by sleeping between polls.
///
/// The stateless `wait_for` below covers `Spin` / `BusySpin` / `Park`
/// exactly. `Backoff`'s idle escalation is stateful and lives in
/// [`DrainWaiter`], which the drain loop constructs; `wait_for`'s
/// `Backoff` arm falls back to the low-latency active wait so the
/// disruptor-facing impl stays total.
#[derive(Copy, Clone, Debug)]
pub struct AdaptiveWaitStrategy {
    /// Selected wait behaviour.
    pub(crate) mode: WaitMode,
    /// Sleep length for `Park` and `Backoff`; ignored by `Spin` /
    /// `BusySpin`.
    pub(crate) park: Duration,
}

impl AdaptiveWaitStrategy {
    /// Number of busy-wait spins before the yield phase (active ramp).
    const SPIN_ITERS: u32 = 100;
    /// Number of `thread::yield_now()` calls after the spin phase.
    const YIELD_ITERS: u32 = 10;

    /// The default low-latency strategy: [`WaitMode::Spin`] with a 1 ms
    /// park interval (unused by `Spin`). Byte-identical to the historical
    /// fixed wait.
    #[must_use]
    pub fn low_latency() -> Self {
        Self::from_mode(WaitMode::Spin, Duration::from_millis(1))
    }

    /// Build the carrier from a selected mode and park interval.
    #[must_use]
    pub fn from_mode(mode: WaitMode, park: Duration) -> Self {
        Self { mode, park }
    }

    /// Phases 1-2 of the active ramp: busy-spin then `yield_now`. Never
    /// sleeps. Shared by the `Spin` / `Backoff`-active wait and by `Park`
    /// (which follows it with a sleep instead of the trailing hint).
    #[inline]
    fn spin_yield(&self) {
        for _ in 0..Self::SPIN_ITERS {
            hint::spin_loop();
        }
        for _ in 0..Self::YIELD_ITERS {
            thread::yield_now();
        }
    }

    /// Low-latency active wait: the spin+yield ramp plus a trailing
    /// `spin_loop` hint; never sleeps. The `Spin` behaviour, and the
    /// active phase of `Backoff`.
    #[inline]
    fn active_wait(&self) {
        self.spin_yield();
        hint::spin_loop();
    }
}

impl disruptor::wait_strategies::WaitStrategy for AdaptiveWaitStrategy {
    #[inline]
    fn wait_for(&self, _sequence: Sequence) {
        match self.mode {
            // Pure busy-spin: one hint, re-poll immediately. Lowest jitter,
            // still ~100% of one core.
            WaitMode::BusySpin => hint::spin_loop(),
            // Adaptive spin+yield ramp (the default). `Backoff`'s stateless
            // fallback matches `Spin`; its idle escalation is applied by
            // `DrainWaiter`, which owns the consecutive-idle state.
            WaitMode::Spin | WaitMode::Backoff => self.active_wait(),
            // Ramp, then park the thread for the configured interval.
            WaitMode::Park => {
                self.spin_yield();
                thread::sleep(self.park);
            }
        }
    }
}

/// Idle window after which [`WaitMode::Backoff`] escalates from the
/// low-latency active wait to parking.
///
/// ~2 ms: comfortably longer than the active spin+yield ramp (which
/// resolves in microseconds on an idle core), so a brief lull between
/// bursts of live ticks keeps spinning at full responsiveness, while a
/// genuinely idle session (a closed market) crosses it well within one
/// ~100 ms ping cycle and drops to the park sleep. Not user-configurable:
/// the user knob is the park interval; this is the fixed hand-off point.
const BACKOFF_IDLE_THRESHOLD: Duration = Duration::from_millis(2);

/// Drain-side wait driver.
///
/// Wraps the `Copy` [`AdaptiveWaitStrategy`] with the per-drain-loop state
/// [`WaitMode::Backoff`] needs — the start of the current uninterrupted
/// idle run. The disruptor `WaitStrategy` itself stays `Copy` and
/// stateless; this is the SDK drain loop's own waiter, constructed fresh
/// for each pull (`next_event`) call and once per callback drain
/// (`for_each_scoped`).
pub(crate) struct DrainWaiter {
    strategy: AdaptiveWaitStrategy,
    /// `Backoff` only: when the current idle run began, or `None` while
    /// events are flowing (or when the mode is not `Backoff`).
    idle_since: Option<Instant>,
}

impl DrainWaiter {
    #[inline]
    pub(crate) fn new(strategy: AdaptiveWaitStrategy) -> Self {
        Self {
            strategy,
            idle_since: None,
        }
    }

    /// Wait after a poll found the ring momentarily empty.
    #[inline]
    pub(crate) fn on_empty(&mut self) {
        use disruptor::wait_strategies::WaitStrategy as _;
        match self.strategy.mode {
            WaitMode::Backoff => {
                // Spin at full responsiveness until the idle run passes the
                // threshold, then park until an event resets the run.
                let since = *self.idle_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= BACKOFF_IDLE_THRESHOLD {
                    thread::sleep(self.strategy.park);
                } else {
                    self.strategy.active_wait();
                }
            }
            // Spin / BusySpin / Park are stateless — defer to the carrier.
            _ => self.strategy.wait_for(0),
        }
    }

    /// Record that a poll delivered at least one event, ending the idle
    /// run so `Backoff` returns to the low-latency active wait.
    #[inline]
    pub(crate) fn on_progress(&mut self) {
        self.idle_since = None;
    }
}

// ---------------------------------------------------------------------------
// Ring event -- the pre-allocated slot in the disruptor ring buffer
// ---------------------------------------------------------------------------

/// FPSS event stored in the event ring buffer.
///
/// Slots are pre-allocated by the ring buffer and reused. The `event`
/// field is an [`FpssEventInternal`] — its `Empty` variant marks an
/// unwritten / drained slot, while `Data`, `Control`, and `Unparseable`
/// carry decoder output. The Disruptor consumer reborrows `Data` /
/// `Control` slots to a public `&StreamEvent` via
/// [`FpssEventInternal::as_public`] and skips the internal-only
/// (`Empty`, `Unparseable`) discriminants.
///
/// # Why not store `StreamEvent` directly?
///
/// The public `StreamEvent` enum hides the ring-buffer pre-allocation
/// placeholder and the decode-failure fallback by design;
/// only `FpssEventInternal` carries those slots. `FpssEventInternal`
/// also dispenses with the `Option<StreamEvent>` discriminant by folding
/// the `None` case into its `Empty` variant, so the consumer pays one
/// branch instead of two.
// `RingEvent` is `pub` (and re-exported under `__test-helpers`) so the
// out-of-crate streaming bench can hold a ring slot; the `event` field
// stays `pub(crate)` — out-of-crate callers go through the typed
// `set_public` / `as_public` seam below. The enclosing `pub(crate) mod
// ring` keeps the type crate-internal in shipped builds.
#[derive(Default)]
pub struct RingEvent {
    /// The FPSS event occupying this slot. Defaults to
    /// [`FpssEventInternal::Empty`] for unwritten / drained slots.
    pub(crate) event: FpssEventInternal,
}

// SAFETY: `FpssEventInternal` is `Send` + holds no thread-affine state
// (only `Arc<Contract>` + owned `Vec<u8>` + POD primitives). Concurrent
// shared access is gated by the disruptor's sequencing protocol:
// exactly one publisher writes a slot before publishing the sequence,
// and consumers only read slots whose sequence has been published
// (memory-ordered Acquire on the cursor read). No reader observes an
// in-flight write, so the lack of an internal lock on `RingEvent`
// is safe; the `unsafe impl Sync` records the contract.
unsafe impl Sync for RingEvent {}

/// Typed accessors over the ring slot for out-of-crate benches and
/// integration tests that drive the production ring constructor.
///
/// The `event` field is `pub(crate)`, so an external bench crate cannot
/// touch it even through a re-exported `RingEvent`. These two methods are
/// the typed seam: write a published [`StreamEvent`] into the slot and read
/// the public projection back, without exposing the internal
/// [`FpssEventInternal`] discriminant. Feature-gated on `__test-helpers`
/// so they never enter a shipped build.
#[cfg(any(test, feature = "__test-helpers"))]
impl RingEvent {
    /// Place a published [`crate::fpss::StreamEvent`] into this slot,
    /// folding it into the internal representation the consumer reads.
    pub fn set_public(&mut self, event: crate::fpss::StreamEvent) {
        self.event = event.into();
    }

    /// Borrow the slot's payload as a public [`crate::fpss::StreamEvent`],
    /// or `None` for the pre-allocation placeholder / decode-failure
    /// fallback slots that never surface to a consumer.
    #[must_use]
    pub fn as_public(&self) -> Option<&crate::fpss::StreamEvent> {
        self.event.as_public()
    }
}

// ---------------------------------------------------------------------------
// Ring producer surface -- the publishing seam the I/O thread drives
// ---------------------------------------------------------------------------

/// The event-ring publishing surface the I/O thread (and the test
/// harness) drives: `Send` (moved into the spawned I/O thread) and
/// `'static` (outlives the thread it is moved into).
///
/// This is the crate's own seam over the underlying
/// [`Producer<RingEvent>`] so every publish path can be instrumented
/// uniformly — the shape returned by
/// [`super::io_loop::build_poller_producer`] records the published
/// ring sequence into [`RingCursors`] on each successful
/// `try_publish`, which feeds the public
/// `StreamingClient::ring_occupancy` sample without touching any call
/// site.
// Declared `pub` so the `__test-helpers`-gated `fpss::__test_internals`
// re-export can name the publish trait the out-of-crate streaming bench
// drives. The enclosing `pub(crate) mod ring` keeps it crate-internal in
// shipped builds — it never reaches the public API.
pub trait RingProducer: Send + 'static {
    /// Non-blocking publish. Returns the published slot's ring
    /// sequence, or an error when the ring is full (the event is NOT
    /// enqueued and the caller decides drop policy).
    ///
    /// This is the only publish path the I/O thread uses: a slow
    /// consumer that lets the ring fill must never wedge the TLS
    /// reader (see the publish-policy contract in the module header).
    fn try_publish<F>(&mut self, update: F) -> Result<Sequence, RingBufferFull>
    where
        F: FnOnce(&mut RingEvent);

    /// Blocking publish: spins until a slot frees. Test-harness
    /// pre-fill only — never called on the I/O thread, so the method
    /// only exists on test builds.
    #[cfg(any(test, feature = "__test-helpers"))]
    fn publish<F>(&mut self, update: F)
    where
        F: FnOnce(&mut RingEvent);
}

/// [`RingProducer`] adapter that records each successfully published
/// ring sequence into the shared [`RingCursors`].
///
/// The store is one plain `Relaxed` write of a value `try_publish`
/// already returns in a register — no read-modify-write, no local
/// counter, no branch beyond the existing `Result` discriminant. The
/// cursor pair is cache-padded so this producer-side store stream
/// never shares a line with the consumer's cursor.
///
/// The blocking `publish` path deliberately does not advance the
/// published cursor: the underlying ring's blocking publish does not
/// return the slot sequence, and the only blocking callers are
/// harness pre-fills — immaterial to occupancy, and the next
/// `try_publish` store re-synchronises the cursor because ring
/// sequences are globally monotone.
pub(in crate::fpss) struct SequencedProducer<P> {
    inner: P,
    cursors: Arc<RingCursors>,
}

impl<P> SequencedProducer<P> {
    pub(in crate::fpss) fn new(inner: P, cursors: Arc<RingCursors>) -> Self {
        Self { inner, cursors }
    }
}

impl<P> RingProducer for SequencedProducer<P>
where
    P: Producer<RingEvent> + Send + 'static,
{
    #[inline]
    fn try_publish<F>(&mut self, update: F) -> Result<Sequence, RingBufferFull>
    where
        F: FnOnce(&mut RingEvent),
    {
        let seq = self.inner.try_publish(update)?;
        self.cursors.record_published(seq);
        Ok(seq)
    }

    #[cfg(any(test, feature = "__test-helpers"))]
    #[inline]
    fn publish<F>(&mut self, update: F)
    where
        F: FnOnce(&mut RingEvent),
    {
        self.inner.publish(update);
    }
}

// ---------------------------------------------------------------------------
// Ring cursors -- producer/consumer progress for occupancy sampling
// ---------------------------------------------------------------------------

/// Producer and consumer progress cursors for the FPSS event ring,
/// sampled by the public occupancy surface
/// ([`crate::fpss::StreamingClient::ring_occupancy`]).
///
/// Both cursors hold the ring sequence (`i64`, `-1` = nothing yet) of
/// the most recent slot the respective side has finished with:
///
/// * `published` — written by the I/O thread with one plain `Relaxed`
///   store per successful publish. The sequence is already in a
///   register when `try_publish` returns, so there is no
///   read-modify-write and no local counter on the hot path.
/// * `consumed` — written by the consumer with one plain `Relaxed`
///   store per **drained batch** (never per event), carrying the
///   sequence of the last slot the batch released.
///
/// Each cursor is padded to its own cache line so the producer's
/// store stream never contends with the consumer's
/// (`CachePadded`). Reads are cold: only operator polling touches
/// them.
// `RingCursors` is `pub` (re-exported under `__test-helpers`) so the
// out-of-crate streaming bench can build the shared occupancy cursor pair
// the production constructor records into; the enclosing `pub(crate) mod
// ring` keeps it crate-internal in shipped builds.
#[derive(Debug)]
pub struct RingCursors {
    /// Sequence of the most recently published slot (`-1` = none).
    published: CachePadded<AtomicI64>,
    /// Sequence of the most recently consumed slot (`-1` = none).
    consumed: CachePadded<AtomicI64>,
}

impl RingCursors {
    /// Fresh cursor pair: nothing published, nothing consumed.
    pub const fn new() -> Self {
        Self {
            published: CachePadded::new(AtomicI64::new(-1)),
            consumed: CachePadded::new(AtomicI64::new(-1)),
        }
    }

    /// Record the sequence returned by a successful publish. One plain
    /// store of a register-resident value; producer-thread only.
    #[inline]
    pub(crate) fn record_published(&self, seq: Sequence) {
        self.published.store(seq, Ordering::Relaxed);
    }

    /// Record the last sequence released by a drained batch. One plain
    /// store per batch; consumer-thread only.
    #[inline]
    pub fn record_consumed(&self, seq: Sequence) {
        self.consumed.store(seq, Ordering::Relaxed);
    }

    /// Point-in-time count of published-but-not-yet-consumed slots.
    ///
    /// The two loads are independent `Relaxed` reads, so a sample
    /// racing a concurrent drain can observe the consumed cursor
    /// ahead of the published cursor it paired with; the difference
    /// is clamped to zero rather than wrapping. The value is a
    /// monitoring sample, not a synchronisation primitive.
    pub(crate) fn occupancy(&self) -> usize {
        let published = self.published.load(Ordering::Relaxed);
        let consumed = self.consumed.load(Ordering::Relaxed);
        usize::try_from(published.saturating_sub(consumed).max(0)).unwrap_or(0)
    }
}

impl Default for RingCursors {
    /// The fresh `-1 / -1` "nothing published, nothing consumed" pair.
    /// A derived `Default` would zero both cursors, which would misreport
    /// the very first slot (sequence `0`) as already consumed; delegate
    /// to [`RingCursors::new`] so the sentinel stays `-1`.
    fn default() -> Self {
        Self::new()
    }
}

// Ring-size validation lives in [`crate::util::ring`] so any other
// ring consumer can share the same contract. Re-export the items here
// under their historical FPSS paths so existing consumers do not have
// to change.
pub(crate) use crate::util::ring::{check_ring_size, MIN_RING_SIZE};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::events::FpssEventInternal;
    use super::*;
    use crate::fpss::{StreamControl, StreamData, StreamEvent};
    use crate::tdbe::types::enums::RemoveReason;
    use disruptor::{build_single_producer, Producer};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn adaptive_wait_strategy_is_copy_send() {
        fn assert_copy_send<T: Copy + Send>() {}
        assert_copy_send::<AdaptiveWaitStrategy>();
    }

    #[test]
    fn low_latency_wait_for_returns() {
        use disruptor::wait_strategies::WaitStrategy;
        // Smoke: the fixed low-latency `wait_for` completes promptly and
        // never parks.
        AdaptiveWaitStrategy::low_latency().wait_for(0);
    }

    #[test]
    fn park_mode_sleeps_each_empty() {
        // Park sleeps the configured interval on every empty poll.
        let strat = AdaptiveWaitStrategy::from_mode(WaitMode::Park, Duration::from_millis(5));
        let mut w = DrainWaiter::new(strat);
        let t = Instant::now();
        w.on_empty();
        assert!(
            t.elapsed() >= Duration::from_millis(4),
            "park must sleep ~the interval"
        );
    }

    #[test]
    fn backoff_escalates_to_park_after_idle_window_then_resets_on_progress() {
        // Backoff spins while the idle run is short, parks once it crosses
        // the threshold, and returns to spinning after a delivered event.
        let strat = AdaptiveWaitStrategy::from_mode(WaitMode::Backoff, Duration::from_millis(5));
        let mut w = DrainWaiter::new(strat);

        // Pre-threshold: the first empty spins, it does not park.
        let t0 = Instant::now();
        w.on_empty();
        assert!(
            t0.elapsed() < Duration::from_millis(5),
            "a fresh idle run must spin, not park"
        );

        // Let the idle run cross the threshold; the next empty parks.
        thread::sleep(BACKOFF_IDLE_THRESHOLD);
        let t1 = Instant::now();
        w.on_empty();
        assert!(
            t1.elapsed() >= Duration::from_millis(4),
            "past the idle window, an empty must park"
        );

        // A delivered event resets the run back to spinning.
        w.on_progress();
        let t2 = Instant::now();
        w.on_empty();
        assert!(
            t2.elapsed() < Duration::from_millis(5),
            "after progress, the idle run restarts and spins"
        );
    }

    #[test]
    fn ring_event_default_is_empty() {
        let e = RingEvent::default();
        assert!(matches!(e.event, FpssEventInternal::Empty));
    }

    #[test]
    fn disruptor_direct_publish_dispatches_events() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let factory = RingEvent::default;
        let wait_strategy = AdaptiveWaitStrategy::low_latency();

        let mut producer = build_single_producer(64, factory, wait_strategy)
            .handle_events_with(
                move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                    if !matches!(ring_event.event, FpssEventInternal::Empty) {
                        counter_clone.fetch_add(1, Ordering::Relaxed);
                    }
                },
            )
            .build();

        producer.publish(|slot| {
            slot.event = FpssEventInternal::Control(StreamControl::MarketOpen);
        });
        producer.publish(|slot| {
            slot.event = FpssEventInternal::Control(StreamControl::MarketClose);
        });
        producer.publish(|slot| {
            slot.event = FpssEventInternal::Control(StreamControl::ServerError {
                message: "test".to_string(),
            });
        });

        // Drop the producer to drain the ring and join consumer thread.
        drop(producer);

        assert_eq!(counter.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn disruptor_direct_publish_receives_payload() {
        use std::sync::Mutex as StdMutex;

        let received = Arc::new(StdMutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);

        let factory = RingEvent::default;
        let wait_strategy = AdaptiveWaitStrategy::low_latency();

        let mut producer = build_single_producer(64, factory, wait_strategy)
            .handle_events_with(
                move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                    if let Some(evt) = ring_event.event.as_public() {
                        received_clone.lock().unwrap().push(evt.clone());
                    }
                },
            )
            .build();

        let ring_contract = std::sync::Arc::new(crate::fpss::protocol::Contract::stock("AAPL"));
        producer.publish(|slot| {
            slot.event = FpssEventInternal::Data(StreamData::Quote {
                contract: std::sync::Arc::clone(&ring_contract),
                ms_of_day: 34200000,
                bid_size: 100,
                bid_exchange: 1,
                bid: 150.25,
                bid_condition: 0,
                ask_size: 200,
                ask_exchange: 1,
                ask: 150.30,
                ask_condition: 0,
                date: 20240315,
                received_at_ns: 0,
            });
        });

        drop(producer);

        let events = received.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::Data(StreamData::Quote {
                contract, bid, ask, ..
            }) => {
                assert_eq!(&*contract.symbol, "AAPL");
                // Both sides round-trip exact decimal-ms quotes
                // (Price::new(15025, 4) and Price::new(15030, 4)) so
                // an `assert_eq!` is sound and tighter than an
                // EPSILON tolerance.
                assert_eq!(*bid, 150.25);
                assert_eq!(*ask, 150.30);
            }
            other => panic!("expected Data(Quote), got {other:?}"),
        }
    }

    #[test]
    fn disruptor_direct_publish_disconnect_event() {
        use std::sync::Mutex as StdMutex;

        let received = Arc::new(StdMutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);

        let factory = RingEvent::default;
        let wait_strategy = AdaptiveWaitStrategy::low_latency();

        let mut producer = build_single_producer(64, factory, wait_strategy)
            .handle_events_with(
                move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                    if let Some(evt) = ring_event.event.as_public() {
                        received_clone.lock().unwrap().push(evt.clone());
                    }
                },
            )
            .build();

        producer.publish(|slot| {
            slot.event = FpssEventInternal::Control(StreamControl::Disconnected {
                reason: RemoveReason::ServerRestarting,
            });
        });

        drop(producer);

        let events = received.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::Control(StreamControl::Disconnected { reason }) => {
                assert_eq!(*reason, RemoveReason::ServerRestarting);
            }
            other => panic!("expected Control(Disconnected), got {other:?}"),
        }
    }

    #[test]
    fn disruptor_high_throughput() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let factory = RingEvent::default;
        let wait_strategy = AdaptiveWaitStrategy::low_latency();

        let mut producer = build_single_producer(4096, factory, wait_strategy)
            .handle_events_with(
                move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                    if !matches!(ring_event.event, FpssEventInternal::Empty) {
                        counter_clone.fetch_add(1, Ordering::Relaxed);
                    }
                },
            )
            .build();

        let count = 1000usize;
        // One Arc<Contract>, reused across all published events so the
        // throughput test doesn't allocate per event (matches the
        // hot-path behaviour where the contract is shared).
        let throughput_contract = std::sync::Arc::new(crate::fpss::protocol::Contract::stock(""));
        for _ in 0..count {
            let contract_clone = std::sync::Arc::clone(&throughput_contract);
            producer.publish(|slot| {
                slot.event = FpssEventInternal::Data(StreamData::Quote {
                    contract: contract_clone,
                    ms_of_day: 0,
                    bid_size: 0,
                    bid_exchange: 0,
                    bid: 0.0,
                    bid_condition: 0,
                    ask_size: 0,
                    ask_exchange: 0,
                    ask: 0.0,
                    ask_condition: 0,
                    date: 0,
                    received_at_ns: 0,
                });
            });
        }

        drop(producer);

        // All events should be processed (disruptor blocks if ring is full).
        assert_eq!(counter.load(Ordering::Relaxed), count);
    }

    #[test]
    fn disruptor_shutdown_flag_pattern() {
        // Verify the shutdown flag pattern used by the read thread works.
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let factory = RingEvent::default;
        let wait_strategy = AdaptiveWaitStrategy::low_latency();

        let mut producer = build_single_producer(64, factory, wait_strategy)
            .handle_events_with(
                move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                    if !matches!(ring_event.event, FpssEventInternal::Empty) {
                        counter_clone.fetch_add(1, Ordering::Relaxed);
                    }
                },
            )
            .build();

        // Simulate the read loop publishing a few events then shutting down.
        let handle = std::thread::spawn(move || {
            for _ in 0..5 {
                if shutdown_clone.load(Ordering::Relaxed) {
                    break;
                }
                producer.publish(|slot| {
                    slot.event = FpssEventInternal::Control(StreamControl::MarketOpen);
                });
            }
            // Producer dropped here -> consumer drains and joins.
        });

        handle.join().unwrap();
        assert_eq!(counter.load(Ordering::Relaxed), 5);
    }

    /// Fresh cursors report an empty ring: both sides at `-1`.
    #[test]
    fn ring_cursors_start_empty() {
        let cursors = RingCursors::new();
        assert_eq!(cursors.occupancy(), 0);
    }

    /// `occupancy = published - consumed` over the plain cursor stores.
    #[test]
    fn ring_cursors_track_published_minus_consumed() {
        let cursors = RingCursors::new();
        cursors.record_published(9); // sequences 0..=9 published
        assert_eq!(cursors.occupancy(), 10);
        cursors.record_consumed(3); // sequences 0..=3 consumed
        assert_eq!(cursors.occupancy(), 6);
        cursors.record_consumed(9); // fully drained
        assert_eq!(cursors.occupancy(), 0);
    }

    /// The transient race: two independent `Relaxed` loads can pair a
    /// stale published cursor with a fresher consumed cursor. The
    /// negative difference must clamp to zero, never wrap into a huge
    /// unsigned value.
    #[test]
    fn ring_cursors_clamp_when_consumed_reads_ahead() {
        let cursors = RingCursors::new();
        cursors.record_published(5);
        cursors.record_consumed(7); // simulated racy read: consumed ahead
        assert_eq!(cursors.occupancy(), 0);
        // Initial state half-race: consumer stored, published load
        // still sees the -1 sentinel.
        let cursors = RingCursors::new();
        cursors.record_consumed(0);
        assert_eq!(cursors.occupancy(), 0);
    }

    /// Saturating arithmetic on the extreme cursor values: the
    /// difference must not overflow `i64` when published is at the
    /// type's ceiling while consumed still holds the `-1` sentinel.
    #[test]
    fn ring_cursors_saturate_at_extremes() {
        let cursors = RingCursors::new();
        cursors.record_published(i64::MAX);
        assert_eq!(cursors.occupancy(), usize::try_from(i64::MAX).unwrap());
    }
}
