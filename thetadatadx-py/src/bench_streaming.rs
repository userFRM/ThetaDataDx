//! Offline saturation bench hook for the Python streaming callback path.
//!
//! [`__bench_flood_events`] drives the SAME LMAX Disruptor pipeline the live
//! streaming consumer uses (single producer, single consumer thread,
//! `RING_SIZE` slots) and hands the events to Python at the production
//! granularity: one `Python::attach` per batch the consumer drains from the
//! ring, then for every event the production marshal (`fpss_event_to_typed`)
//! and the production 1-tuple `call1((typed,))`. Production holds the GIL
//! across the drain in `StreamView::start_streaming`; the bench copies each
//! drained event into a buffer and attaches at the batch end, which the
//! Disruptor marks, so it pays one `Arc` clone per event that production does
//! not. The Disruptor itself runs ~39 M events/s in the core Rust bench, far
//! above the boundary, so the number isolates the boundary-crossing cost.
//!
//! The hook retries the publish on `RingBufferFull` so all `n` events are
//! delivered (no silent drops); the delivered count is returned and the
//! caller asserts `delivered == n`. The returned wall clock is the publish
//! loop + drain, measured on the consumer side; interpreter spin-up, ring
//! allocation, and thread spawn run before the timed region opens and are
//! excluded (mirroring the `iter_custom` setup-exclusion in the core Rust
//! bench `thetadatadx-rs/benches/streaming_throughput.rs`).
//!
//! This is a bench-only `#[pyfunction]`, NOT part of the public utility
//! surface.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use disruptor::{BusySpin, Producer, Sequence, build_single_producer};
use pyo3::prelude::*;

use thetadatadx::Price;
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{StreamData, StreamEvent};

use crate::fpss_event_to_typed;

/// Disruptor ring size. Matches the production default
/// `FpssConfig::ring_size = 131_072` (see `thetadatadx-rs/src/config/fpss.rs`)
/// and the core Rust bench, so the Python ceiling is measured against the
/// out-of-the-box live SDK ring configuration.
const RING_SIZE: usize = 131_072;

/// One ring slot. Shape-compatible with the engine's `RingEvent`
/// (`thetadatadx-rs/src/fpss/ring.rs`).
#[derive(Default)]
struct RingSlot {
    event: Option<StreamEvent>,
}

// SAFETY: `StreamEvent: Clone + Send`; the Disruptor's sequencing
// guarantees exclusive write / shared read of each slot, so the
// `!Sync` `Option<StreamEvent>` is only ever touched by one thread at a
// time. Matches the `unsafe impl Sync for RingEvent` rationale in
// `thetadatadx-rs/src/fpss/ring.rs`.
unsafe impl Sync for RingSlot {}

/// Build a `Trade` event carrying a shared `Arc<Contract>`. The contract
/// is interned by the caller (allocated once, `Arc::clone` per event) so
/// the publish closure pays only a refcount bump — identical to the live
/// streaming decode path and to the core Rust bench's `make_event`.
#[inline]
fn make_event(contract: &Arc<Contract>, idx: u64) -> StreamEvent {
    StreamEvent::Data(StreamData::Trade {
        contract: Arc::clone(contract),
        ms_of_day: (idx % 86_400_000) as i32,
        sequence: idx as i32,
        condition: 0,
        size: 100,
        exchange: 0,
        price: Price::new(15025, 8).to_f64(),
        date: 20240315,
        received_at_ns: idx,
    })
}

/// Build the Disruptor + consumer thread with a user-supplied consumer
/// `body`, drive `n` synthetic trades through the publish loop (retry on
/// overflow so every event is delivered), and return `(delivered,
/// elapsed_ns)`. `body` runs on the consumer thread for every delivered
/// event, with the Disruptor's end-of-batch flag. Setup (ring build, thread
/// spawn) is excluded from the timed region.
fn run_pipeline<B>(py: Python<'_>, n: u64, body: B) -> (u64, u64)
where
    B: FnMut(&StreamEvent, bool) + Send + 'static,
{
    let delivered = Arc::new(AtomicU64::new(0));
    let delivered_consumer = Arc::clone(&delivered);
    let contract = Arc::new(Contract::stock("SPY"));

    let mut body = body;
    let factory = || RingSlot { event: None };
    let mut producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, end_of_batch: bool| {
            if let Some(ref evt) = slot.event {
                delivered_consumer.fetch_add(1, Ordering::Relaxed);
                body(evt, end_of_batch);
            }
        })
        .build();

    // Release the GIL for the whole flood: the consumer thread re-acquires
    // it once per drained batch. Holding it here would deadlock the consumer
    // against its own `Python::attach`.
    let elapsed: Duration = py.detach(|| {
        let start = Instant::now();
        for i in 0..n {
            loop {
                let evt = make_event(&contract, i);
                if producer.try_publish(|slot| slot.event = Some(evt)).is_ok() {
                    break;
                }
                std::hint::spin_loop();
            }
        }
        // Dropping the producer joins the consumer thread, so every
        // published event has been handed to `body` by the time this
        // returns; the last one carries the end-of-batch flag.
        drop(producer);
        start.elapsed()
    });

    (delivered.load(Ordering::Relaxed), elapsed.as_nanos() as u64)
}

/// Flood `n` synthetic trades through the pipeline and hand them to
/// `callback` at the production granularity: one GIL acquisition per
/// drained batch, one `fpss_event_to_typed` + `call1((typed,))` per event.
/// Returns `(delivered, elapsed_ns)`; the caller asserts `delivered == n`
/// (zero-drop).
#[pyfunction]
pub(crate) fn __bench_flood_events(
    py: Python<'_>,
    n: u64,
    callback: Py<PyAny>,
) -> PyResult<(u64, u64)> {
    let mut drained: Vec<StreamEvent> = Vec::new();
    let body = move |evt: &StreamEvent, end_of_batch: bool| {
        drained.push(evt.clone());
        if end_of_batch {
            Python::attach(|py| {
                for evt in &drained {
                    match fpss_event_to_typed(py, evt) {
                        Ok(typed) => {
                            if let Err(err) = callback.call1(py, (typed,)) {
                                err.write_unraisable(py, None);
                            }
                        }
                        Err(err) => err.write_unraisable(py, None),
                    }
                }
            });
            drained.clear();
        }
    };
    Ok(run_pipeline(py, n, body))
}
