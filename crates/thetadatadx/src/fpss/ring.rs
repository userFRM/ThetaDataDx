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
//! [`AdaptiveWaitStrategy`] implements a three-phase wait tuned for FPSS tick
//! intervals (~100us during active trading).

use std::hint;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use disruptor::{Producer, RingBufferFull, Sequence};

use super::events::FpssEventInternal;
use crate::util::cache_padded::CachePadded;

/// Tuning mode for the FPSS event-ring consumer wait, branched on in
/// [`AdaptiveWaitStrategy::wait_for`].
///
/// Each preset is a different point on the latency-vs-CPU curve. The
/// mode is selected once at ring-build time and never changes for the
/// life of the consumer, so the single match in `wait_for` is
/// negligible against the spin / sleep body it guards.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum WaitMode {
    /// Spin then yield then a `spin_loop` hint; never sleeps. Lowest
    /// latency, highest idle CPU. The crate default.
    LowLatency,
    /// Short spin, brief yield, then a timed park. Low idle CPU at the
    /// cost of up to one park interval of added tail latency.
    Balanced,
    /// Minimal spin then a longer timed park. Lowest idle CPU.
    Efficient,
    /// Pure `spin_loop` hint, no yield or sleep. Absolute minimum
    /// latency; pins a core at 100% while the ring is idle.
    BusySpin,
}

/// Configurable wait strategy for the FPSS event ring consumer.
///
/// One `Copy` type that carries a [`WaitMode`] plus the spin / yield /
/// park tuning, branching in [`Self::wait_for`] — the disruptor's
/// `WaitStrategy` trait is `Copy + Send`, so the whole strategy lives
/// in registers and the per-poll cost is the chosen mode's body, not
/// indirection.
///
/// The phases the modes draw on:
/// 1. **Spin** -- busy-wait `spin_iters` times (lowest latency, highest CPU).
/// 2. **Yield** -- `thread::yield_now()` `yield_iters` times (moderate).
/// 3. **Park** -- `thread::sleep(park_us)` (low CPU, adds park-interval latency).
///
/// `wait_for` fires on every ring-empty poll, so the choice of mode is
/// a direct latency-vs-CPU knob. For FPSS real-time market data the
/// [`WaitMode::LowLatency`] default sizes the spin phase to cover the
/// typical inter-tick interval (~100us during active trading): at ~3ns
/// per spin iteration, 100 spins covers ~300ns, the yield phase handles
/// brief pauses between bursts, and the trailing `spin_loop` hint covers
/// idle periods (pre-market, post-market) without parking.
#[derive(Copy, Clone, Debug)]
pub struct AdaptiveWaitStrategy {
    mode: WaitMode,
    spin_iters: u32,
    yield_iters: u32,
    park_us: u64,
}

impl AdaptiveWaitStrategy {
    /// Inclusive upper bound on the spin count; caps a misconfiguration
    /// from turning the spin phase into a multi-millisecond busy-wait.
    pub(crate) const MAX_SPIN_ITERS: u32 = 1_000_000;
    /// Inclusive upper bound on the yield count.
    pub(crate) const MAX_YIELD_ITERS: u32 = 100_000;
    /// Inclusive upper bound on the park interval (microseconds); caps a
    /// misconfiguration from parking the consumer for seconds.
    pub(crate) const MAX_PARK_US: u64 = 1_000_000;

    /// [`WaitMode::LowLatency`] preset: 100 spins + 10 yields before a
    /// trailing `spin_loop` hint; never sleeps.
    ///
    /// At ~3ns per spin iteration, 100 spins = ~300ns — well within the
    /// typical FPSS tick interval (~100us during active trading). This
    /// is the crate default and reproduces the historical fixed
    /// behaviour byte-for-byte.
    #[must_use]
    pub fn low_latency() -> Self {
        Self {
            mode: WaitMode::LowLatency,
            spin_iters: 100,
            yield_iters: 10,
            park_us: 50,
        }
    }

    /// [`WaitMode::Balanced`] preset: short spin, brief yield, then a
    /// 50us park. Low idle CPU, ~park-interval added tail latency.
    #[must_use]
    pub fn balanced() -> Self {
        Self {
            mode: WaitMode::Balanced,
            spin_iters: 32,
            yield_iters: 4,
            park_us: 50,
        }
    }

    /// [`WaitMode::Efficient`] preset: minimal spin then a longer 250us
    /// park. Lowest idle CPU.
    #[must_use]
    pub fn efficient() -> Self {
        Self {
            mode: WaitMode::Efficient,
            spin_iters: 8,
            yield_iters: 0,
            park_us: 250,
        }
    }

    /// [`WaitMode::BusySpin`] preset: pure `spin_loop` hint, no yield or
    /// sleep. Absolute minimum latency; pins a core while idle.
    #[must_use]
    pub fn busy_spin() -> Self {
        Self {
            mode: WaitMode::BusySpin,
            spin_iters: 0,
            yield_iters: 0,
            park_us: 0,
        }
    }

    /// Alias for [`Self::low_latency`] naming the historical fixed FPSS
    /// strategy, kept for the in-crate tests that assert the default
    /// preset reproduces it byte-for-byte. Production code constructs the
    /// strategy from config via [`Self::from_mode`].
    #[cfg(any(test, feature = "__test-helpers"))]
    #[must_use]
    pub fn fpss_default() -> Self {
        Self::low_latency()
    }

    /// Return a copy of this strategy with the spin / yield / park
    /// counts replaced, clamping each to its sane upper bound and
    /// preserving the [`WaitMode`].
    #[must_use]
    pub(crate) fn with_tuning(self, spin_iters: u32, yield_iters: u32, park_us: u64) -> Self {
        Self::from_mode(self.mode, spin_iters, yield_iters, park_us)
    }

    /// Build a strategy for a [`WaitMode`] with explicit spin / yield /
    /// park tuning, clamping each parameter to its sane upper bound.
    ///
    /// The mode selects the phase shape; the three integers tune the
    /// phases that mode actually uses (e.g. `park_us` is inert under
    /// [`WaitMode::LowLatency`] / [`WaitMode::BusySpin`], which never
    /// sleep). Clamping here means an out-of-range config can never turn
    /// the hot-path wait into a multi-second stall.
    pub(crate) fn from_mode(
        mode: WaitMode,
        spin_iters: u32,
        yield_iters: u32,
        park_us: u64,
    ) -> Self {
        Self {
            mode,
            spin_iters: spin_iters.min(Self::MAX_SPIN_ITERS),
            yield_iters: yield_iters.min(Self::MAX_YIELD_ITERS),
            park_us: park_us.min(Self::MAX_PARK_US),
        }
    }
}

impl disruptor::wait_strategies::WaitStrategy for AdaptiveWaitStrategy {
    #[inline]
    fn wait_for(&self, _sequence: Sequence) {
        match self.mode {
            WaitMode::LowLatency => {
                // Phase 1: Spin (lowest latency)
                for _ in 0..self.spin_iters {
                    hint::spin_loop();
                }
                // Phase 2: Yield (moderate)
                for _ in 0..self.yield_iters {
                    thread::yield_now();
                }
                // Phase 3: Spin-loop hint (low CPU, still responsive)
                hint::spin_loop();
            }
            WaitMode::Balanced | WaitMode::Efficient => {
                // Short spin then yield to absorb sub-microsecond gaps,
                // then a timed park for longer idle. The disruptor
                // re-checks the cursor after `wait_for` returns, so a
                // timed sleep is the correct parking form — this crate
                // has no condvar/notify path.
                for _ in 0..self.spin_iters {
                    hint::spin_loop();
                }
                for _ in 0..self.yield_iters {
                    thread::yield_now();
                }
                thread::sleep(Duration::from_micros(self.park_us));
            }
            WaitMode::BusySpin => {
                // Pure hint: absolute minimum latency, pins a core.
                hint::spin_loop();
            }
        }
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
/// ([`CachePadded`]). Reads are cold: only operator polling touches
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
#[cfg(test)]
use crate::util::ring::RingSizeError;
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
    fn fpss_default_strategy() {
        // `low_latency` (== the historical `fpss_default`) MUST keep the
        // 100-spin / 10-yield tuning and the LowLatency phase shape.
        let s = AdaptiveWaitStrategy::fpss_default();
        assert_eq!(s.mode, WaitMode::LowLatency);
        assert_eq!(s.spin_iters, 100);
        assert_eq!(s.yield_iters, 10);
        let direct = AdaptiveWaitStrategy::low_latency();
        assert_eq!(direct.mode, WaitMode::LowLatency);
        assert_eq!(direct.spin_iters, 100);
        assert_eq!(direct.yield_iters, 10);
    }

    #[test]
    fn preset_modes_carry_expected_shape() {
        let balanced = AdaptiveWaitStrategy::balanced();
        assert_eq!(balanced.mode, WaitMode::Balanced);
        assert!(balanced.park_us > 0);

        let efficient = AdaptiveWaitStrategy::efficient();
        assert_eq!(efficient.mode, WaitMode::Efficient);
        // Efficient parks longer than Balanced for lower idle CPU.
        assert!(efficient.park_us >= balanced.park_us);

        let busy = AdaptiveWaitStrategy::busy_spin();
        assert_eq!(busy.mode, WaitMode::BusySpin);
        assert_eq!(busy.yield_iters, 0);
        assert_eq!(busy.park_us, 0);
    }

    #[test]
    fn from_mode_clamps_out_of_range_params() {
        let s = AdaptiveWaitStrategy::from_mode(WaitMode::Balanced, u32::MAX, u32::MAX, u64::MAX);
        assert_eq!(s.mode, WaitMode::Balanced);
        assert_eq!(s.spin_iters, AdaptiveWaitStrategy::MAX_SPIN_ITERS);
        assert_eq!(s.yield_iters, AdaptiveWaitStrategy::MAX_YIELD_ITERS);
        assert_eq!(s.park_us, AdaptiveWaitStrategy::MAX_PARK_US);
    }

    #[test]
    fn each_preset_wait_for_returns() {
        use disruptor::wait_strategies::WaitStrategy;
        // Smoke: every preset's `wait_for` completes without hanging.
        // Balanced / Efficient sleep (short park), BusySpin / LowLatency
        // spin; all must return promptly.
        for s in [
            AdaptiveWaitStrategy::low_latency(),
            AdaptiveWaitStrategy::busy_spin(),
            // Keep the parked presets short for the test.
            AdaptiveWaitStrategy::from_mode(WaitMode::Balanced, 1, 0, 1),
            AdaptiveWaitStrategy::from_mode(WaitMode::Efficient, 1, 0, 1),
        ] {
            s.wait_for(0);
        }
    }

    #[test]
    fn parked_modes_actually_sleep() {
        use disruptor::wait_strategies::WaitStrategy;
        // A parked mode with a measurable park must elapse at least the
        // park interval; LowLatency must not sleep at all.
        let parked = AdaptiveWaitStrategy::from_mode(WaitMode::Balanced, 0, 0, 2_000);
        let start = std::time::Instant::now();
        parked.wait_for(0);
        assert!(start.elapsed() >= std::time::Duration::from_micros(2_000));
    }

    #[test]
    fn ring_event_default_is_empty() {
        let e = RingEvent::default();
        assert!(matches!(e.event, FpssEventInternal::Empty));
    }

    #[test]
    fn check_ring_size_accepts_powers_of_two() {
        assert_eq!(check_ring_size(64), Ok(64));
        assert_eq!(check_ring_size(1024), Ok(1024));
        assert_eq!(check_ring_size(131_072), Ok(131_072));
    }

    #[test]
    fn check_ring_size_rejects_non_power_of_two() {
        let err = check_ring_size(65).unwrap_err();
        assert_eq!(
            err,
            RingSizeError::NotPowerOfTwo {
                provided: 65,
                suggested: 128,
            }
        );
    }

    #[test]
    fn check_ring_size_rejects_below_minimum() {
        let err = check_ring_size(32).unwrap_err();
        assert_eq!(
            err,
            RingSizeError::TooSmall {
                provided: 32,
                minimum: MIN_RING_SIZE,
            }
        );
    }

    #[test]
    fn check_ring_size_error_messages_name_offender() {
        let msg = check_ring_size(1000).unwrap_err().to_string();
        assert!(msg.contains("1000"));
        assert!(msg.contains("1024"));
    }

    #[test]
    fn disruptor_direct_publish_dispatches_events() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let factory = RingEvent::default;
        let wait_strategy = AdaptiveWaitStrategy::fpss_default();

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
        let wait_strategy = AdaptiveWaitStrategy::fpss_default();

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
        let wait_strategy = AdaptiveWaitStrategy::fpss_default();

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
        let wait_strategy = AdaptiveWaitStrategy::fpss_default();

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
        let wait_strategy = AdaptiveWaitStrategy::fpss_default();

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
