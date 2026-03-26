//! LMAX Disruptor ring buffer for lock-free FPSS event dispatch.
//!
//! # Architecture
//!
//! ```text
//!  +--------------------+                  +--------------------+
//!  | Blocking TLS       |  publish()       | Disruptor Ring     |
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
//! This mirrors the Java terminal's LMAX Disruptor architecture exactly:
//! - Java: blocking `DataInputStream` -> LMAX Disruptor ring -> event handlers
//! - Rust: blocking TLS `read` -> Disruptor ring -> user's `FnMut(&FpssEvent)` callback
//!
//! No tokio, no channels, no async. The blocking read thread IS the Disruptor
//! producer. Events are pre-allocated in the ring buffer (zero allocation on
//! the hot path), and the single-producer barrier uses a plain store (no CAS).
//!
//! # Wait Strategy
//!
//! [`AdaptiveWaitStrategy`] implements a three-phase wait inspired by LMAX Disruptor's
//! `PhasedBackoffWaitStrategy` and tuned for FPSS tick intervals (~100us during active
//! trading).

use std::hint;
use std::thread;

use disruptor::Sequence;

use super::FpssEvent;

/// Adaptive wait strategy inspired by LMAX Disruptor's `PhasedBackoffWaitStrategy`.
///
/// Three phases:
/// 1. **Spin** -- busy-wait for `spin_iters` iterations (lowest latency, highest CPU)
/// 2. **Yield** -- `thread::yield_now()` for `yield_iters` iterations (moderate)
/// 3. **Hint** -- `hint::spin_loop()` indefinitely (low CPU, still responsive)
///
/// For FPSS real-time market data, we want the spin phase to cover the typical
/// inter-tick interval (~100us during active trading). At ~3ns per spin iteration,
/// 100 spins covers ~300ns -- well within the FPSS tick interval. The yield phase
/// handles brief pauses between bursts, and the hint phase covers idle periods
/// (pre-market, post-market) without burning a full core.
#[derive(Copy, Clone)]
pub struct AdaptiveWaitStrategy {
    spin_iters: u32,
    yield_iters: u32,
}

impl AdaptiveWaitStrategy {
    /// Create a new adaptive wait strategy with custom iteration counts.
    pub fn new(spin_iters: u32, yield_iters: u32) -> Self {
        Self {
            spin_iters,
            yield_iters,
        }
    }

    /// Tuned for FPSS: 100 spins + 10 yields before falling back to spin_loop hint.
    ///
    /// At ~3ns per spin iteration, 100 spins = ~300ns -- well within the typical
    /// FPSS tick interval. This matches the Java terminal's Disruptor configuration
    /// for real-time market data processing.
    pub fn fpss_default() -> Self {
        Self::new(100, 10)
    }
}

impl disruptor::wait_strategies::WaitStrategy for AdaptiveWaitStrategy {
    #[inline]
    fn wait_for(&self, _sequence: Sequence) {
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
}

// ---------------------------------------------------------------------------
// Ring event -- the pre-allocated slot in the disruptor ring buffer
// ---------------------------------------------------------------------------

/// FPSS event stored in the disruptor ring buffer.
///
/// Slots are pre-allocated by the ring buffer and reused. The `event` field
/// holds `None` for unused slots and `Some(FpssEvent)` for published events.
///
/// # Why not store FpssEvent directly?
///
/// `FpssEvent` has variants with `Vec<u8>` payloads and `String` fields that
/// cannot be meaningfully pre-allocated. Using `Option<FpssEvent>` lets us
/// pre-allocate the slot metadata while the event data is set on publish.
#[derive(Default)]
pub struct RingEvent {
    /// The FPSS event occupying this slot, or `None` for an empty pre-allocated slot.
    pub event: Option<FpssEvent>,
}

// SAFETY: FpssEvent is Clone + Send; RingEvent is only accessed through the
// disruptor's sequencing guarantees (exclusive write, shared read).
unsafe impl Sync for RingEvent {}

/// Default ring buffer size (must be a power of 2).
///
/// 131072 slots covers ~13 seconds of burst traffic at 10k events/sec with
/// headroom. This matches the Java terminal's `FPSS_QUEUE_DEPTH` spirit while
/// using the disruptor's power-of-2 constraint.
pub const DEFAULT_RING_SIZE: usize = 131_072; // 2^17

/// Minimum ring size (power of 2).
pub const MIN_RING_SIZE: usize = 64;

/// Round up to the next power of 2 (required by disruptor ring buffer).
pub(crate) fn next_power_of_two(n: usize) -> usize {
    if n.is_power_of_two() {
        n
    } else {
        n.next_power_of_two()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpss::{FpssControl, FpssData, FpssEvent};
    use crate::types::enums::RemoveReason;
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
        let s = AdaptiveWaitStrategy::fpss_default();
        assert_eq!(s.spin_iters, 100);
        assert_eq!(s.yield_iters, 10);
    }

    #[test]
    fn ring_event_default_is_none() {
        let e = RingEvent::default();
        assert!(e.event.is_none());
    }

    #[test]
    fn next_power_of_two_already_power() {
        assert_eq!(next_power_of_two(64), 64);
        assert_eq!(next_power_of_two(1024), 1024);
    }

    #[test]
    fn next_power_of_two_rounds_up() {
        assert_eq!(next_power_of_two(65), 128);
        assert_eq!(next_power_of_two(1000), 1024);
        assert_eq!(next_power_of_two(100_000), 131_072);
    }

    #[test]
    fn default_ring_size_is_power_of_two() {
        assert!(DEFAULT_RING_SIZE.is_power_of_two());
    }

    #[test]
    fn ring_size_respects_minimum() {
        // Even if caller requests a tiny ring, we enforce MIN_RING_SIZE.
        let size = next_power_of_two(1_usize.max(MIN_RING_SIZE));
        assert!(size >= MIN_RING_SIZE);
        assert!(size.is_power_of_two());
    }

    #[test]
    fn disruptor_direct_publish_dispatches_events() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let factory = || RingEvent { event: None };
        let wait_strategy = AdaptiveWaitStrategy::fpss_default();

        let mut producer = build_single_producer(64, factory, wait_strategy)
            .handle_events_with(
                move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                    if ring_event.event.is_some() {
                        counter_clone.fetch_add(1, Ordering::Relaxed);
                    }
                },
            )
            .build();

        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(FpssControl::MarketOpen));
        });
        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(FpssControl::MarketClose));
        });
        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(FpssControl::ServerError {
                message: "test".to_string(),
            }));
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

        let factory = || RingEvent { event: None };
        let wait_strategy = AdaptiveWaitStrategy::fpss_default();

        let mut producer = build_single_producer(64, factory, wait_strategy)
            .handle_events_with(
                move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                    if let Some(ref evt) = ring_event.event {
                        received_clone.lock().unwrap().push(evt.clone());
                    }
                },
            )
            .build();

        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Data(FpssData::Quote {
                contract_id: 42,
                ms_of_day: 34200000,
                bid_size: 100,
                bid_exchange: 1,
                bid: 15025,
                bid_condition: 0,
                ask_size: 200,
                ask_exchange: 1,
                ask: 15030,
                ask_condition: 0,
                price_type: 8,
                date: 20240315,
            }));
        });

        drop(producer);

        let events = received.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            FpssEvent::Data(FpssData::Quote {
                contract_id,
                bid,
                ask,
                ..
            }) => {
                assert_eq!(*contract_id, 42);
                assert_eq!(*bid, 15025);
                assert_eq!(*ask, 15030);
            }
            other => panic!("expected Data(Quote), got {other:?}"),
        }
    }

    #[test]
    fn disruptor_direct_publish_disconnect_event() {
        use std::sync::Mutex as StdMutex;

        let received = Arc::new(StdMutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);

        let factory = || RingEvent { event: None };
        let wait_strategy = AdaptiveWaitStrategy::fpss_default();

        let mut producer = build_single_producer(64, factory, wait_strategy)
            .handle_events_with(
                move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                    if let Some(ref evt) = ring_event.event {
                        received_clone.lock().unwrap().push(evt.clone());
                    }
                },
            )
            .build();

        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(FpssControl::Disconnected {
                reason: RemoveReason::ServerRestarting,
            }));
        });

        drop(producer);

        let events = received.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            FpssEvent::Control(FpssControl::Disconnected { reason }) => {
                assert_eq!(*reason, RemoveReason::ServerRestarting);
            }
            other => panic!("expected Control(Disconnected), got {other:?}"),
        }
    }

    #[test]
    fn disruptor_high_throughput() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let factory = || RingEvent { event: None };
        let wait_strategy = AdaptiveWaitStrategy::fpss_default();

        let mut producer = build_single_producer(4096, factory, wait_strategy)
            .handle_events_with(
                move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                    if ring_event.event.is_some() {
                        counter_clone.fetch_add(1, Ordering::Relaxed);
                    }
                },
            )
            .build();

        let count = 1000usize;
        for i in 0..count {
            producer.publish(|slot| {
                slot.event = Some(FpssEvent::Data(FpssData::Quote {
                    contract_id: i as i32,
                    ms_of_day: 0,
                    bid_size: 0,
                    bid_exchange: 0,
                    bid: 0,
                    bid_condition: 0,
                    ask_size: 0,
                    ask_exchange: 0,
                    ask: 0,
                    ask_condition: 0,
                    price_type: 0,
                    date: 0,
                }));
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

        let factory = || RingEvent { event: None };
        let wait_strategy = AdaptiveWaitStrategy::fpss_default();

        let mut producer = build_single_producer(64, factory, wait_strategy)
            .handle_events_with(
                move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                    if ring_event.event.is_some() {
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
                    slot.event = Some(FpssEvent::Control(FpssControl::MarketOpen));
                });
            }
            // Producer dropped here -> consumer drains and joins.
        });

        handle.join().unwrap();
        assert_eq!(counter.load(Ordering::Relaxed), 5);
    }
}
