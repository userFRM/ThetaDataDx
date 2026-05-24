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
//! Pipeline: blocking TLS `read` -> Disruptor ring -> user's
//! `FnMut(&FpssEvent)` callback.
//!
//! No tokio, no channels, no async. The blocking read thread IS the Disruptor
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
//! [`AdaptiveWaitStrategy`] implements a three-phase wait inspired by LMAX Disruptor's
//! `PhasedBackoffWaitStrategy` and tuned for FPSS tick intervals (~100us during active
//! trading).

use std::hint;
use std::thread;

use disruptor::Sequence;

use super::events::FpssEventInternal;

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
    #[must_use]
    pub fn new(spin_iters: u32, yield_iters: u32) -> Self {
        Self {
            spin_iters,
            yield_iters,
        }
    }

    /// Tuned for FPSS: 100 spins + 10 yields before falling back to `spin_loop` hint.
    ///
    /// At ~3ns per spin iteration, 100 spins = ~300ns -- well within the typical
    /// FPSS tick interval. This matches the Java terminal's Disruptor configuration
    /// for real-time market data processing.
    #[must_use]
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
/// Slots are pre-allocated by the ring buffer and reused. The `event`
/// field is an [`FpssEventInternal`] — its `Empty` variant marks an
/// unwritten / drained slot, while `Data`, `Control`, and `Unparseable`
/// carry decoder output. The Disruptor consumer reborrows `Data` /
/// `Control` slots to a public `&FpssEvent` via
/// [`FpssEventInternal::as_public`] and skips the internal-only
/// (`Empty`, `Unparseable`) discriminants.
///
/// # Why not store `FpssEvent` directly?
///
/// The public `FpssEvent` enum hides the ring-buffer pre-allocation
/// placeholder and the decode-failure fallback by design;
/// only `FpssEventInternal` carries those slots. `FpssEventInternal`
/// also dispenses with the `Option<FpssEvent>` discriminant by folding
/// the `None` case into its `Empty` variant, so the consumer pays one
/// branch instead of two.
#[derive(Default)]
pub(crate) struct RingEvent {
    /// The FPSS event occupying this slot. Defaults to
    /// [`FpssEventInternal::Empty`] for unwritten / drained slots.
    pub(crate) event: FpssEventInternal,
}

// SAFETY: FpssEventInternal is Clone + Send; RingEvent is only accessed
// through the disruptor's sequencing guarantees (exclusive write, shared read).
unsafe impl Sync for RingEvent {}

// Ring-size validation lives in [`crate::util::ring`] so the gRPC
// decoder pool can share the same contract. Re-export the items here
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
    use crate::fpss::{FpssControl, FpssData, FpssEvent};
    use disruptor::{build_single_producer, Producer};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use tdbe::types::enums::RemoveReason;

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
            slot.event = FpssEventInternal::Control(FpssControl::MarketOpen);
        });
        producer.publish(|slot| {
            slot.event = FpssEventInternal::Control(FpssControl::MarketClose);
        });
        producer.publish(|slot| {
            slot.event = FpssEventInternal::Control(FpssControl::ServerError {
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
            slot.event = FpssEventInternal::Data(FpssData::Quote {
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
            FpssEvent::Data(FpssData::Quote {
                contract, bid, ask, ..
            }) => {
                assert_eq!(contract.symbol, "AAPL");
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
            slot.event = FpssEventInternal::Control(FpssControl::Disconnected {
                reason: RemoveReason::ServerRestarting,
            });
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
        // throughput test doesn't allocate per event (matches real
        // hot-path behaviour after v8).
        let throughput_contract = std::sync::Arc::new(crate::fpss::protocol::Contract::stock(""));
        for _ in 0..count {
            let contract_clone = std::sync::Arc::clone(&throughput_contract);
            producer.publish(|slot| {
                slot.event = FpssEventInternal::Data(FpssData::Quote {
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
                    slot.event = FpssEventInternal::Control(FpssControl::MarketOpen);
                });
            }
            // Producer dropped here -> consumer drains and joins.
        });

        handle.join().unwrap();
        assert_eq!(counter.load(Ordering::Relaxed), 5);
    }
}
