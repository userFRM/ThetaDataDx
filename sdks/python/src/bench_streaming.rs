//! Offline saturation bench hook for the Python streaming callback path.
//!
//! `__bench_flood_events(n, callback)` drives the same LMAX Disruptor
//! pipeline the live FPSS consumer uses (single producer, single
//! consumer thread, `RING_SIZE` slots) and, for every delivered event,
//! runs the identical per-event handover the generated `start_streaming`
//! dispatcher runs in production:
//!
//! 1. `Python::attach` to acquire the GIL on the consumer thread (the
//!    production granularity is per-event, not per-batch — see
//!    `_generated/streaming_methods.rs`, where the FPSS callback closure
//!    re-attaches for each delivered event);
//! 2. `fpss_event_to_typed(py, event)` — the exact borrowed-`&StreamEvent`
//!    → typed `#[pyclass]` marshal the production path calls, reused here
//!    rather than reimplemented;
//! 3. `callback.call1(py, (typed,))` — the same `call1` 1-tuple vectorcall
//!    the dispatcher uses.
//!
//! The publish loop retries on `RingBufferFull` so every attempted event
//! is delivered (no silent drops); the delivered count is returned to the
//! caller, which asserts `delivered == n`. The returned wall-clock is the
//! publish-loop + drain duration measured on the consumer thread side via
//! the shared delivery counter — interpreter spin-up, ring allocation,
//! and thread spawn are excluded (they run before the timed region opens),
//! mirroring the `iter_custom` setup-exclusion in the core Rust bench
//! `crates/thetadatadx/benches/streaming_throughput.rs`.
//!
//! This is a bench-only `#[pyfunction]`, NOT part of the public utility
//! surface. It is enrolled in `PY_NON_UTILITY_PYFUNCTIONS` in
//! `scripts/ci/check_binding_parity.py` alongside the other offline
//! hooks (`decode_response_bytes`, `blocked_fpss_methods`) so the parity
//! gate does not mistake it for an untracked cross-binding utility.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use disruptor::{build_single_producer, BusySpin, Producer, Sequence};
use pyo3::prelude::*;

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{StreamData, StreamEvent};
use thetadatadx::Price;

use crate::fpss_event_to_typed;

/// Disruptor ring size. Matches the production default
/// `FpssConfig::ring_size = 131_072` (see `crates/thetadatadx/src/config/fpss.rs`)
/// and the core Rust bench, so the Python ceiling is measured against the
/// out-of-the-box live SDK ring configuration.
const RING_SIZE: usize = 131_072;

/// One ring slot. Shape-compatible with the engine's `RingEvent`
/// (`crates/thetadatadx/src/fpss/ring.rs`).
#[derive(Default)]
struct RingSlot {
    event: Option<StreamEvent>,
}

// SAFETY: `StreamEvent: Clone + Send`; the Disruptor's sequencing
// guarantees exclusive write / shared read of each slot, so the
// `!Sync` `Option<StreamEvent>` is only ever touched by one thread at a
// time. Matches the `unsafe impl Sync for RingEvent` rationale in
// `crates/thetadatadx/src/fpss/ring.rs`.
unsafe impl Sync for RingSlot {}

/// Build a `Trade` event carrying a shared `Arc<Contract>`. The contract
/// is interned by the caller (allocated once, `Arc::clone` per event) so
/// the publish closure pays only a refcount bump — identical to the live
/// FPSS decode path and to the core Rust bench's `make_event`.
#[inline]
fn make_event(contract: &Arc<Contract>, idx: u64) -> StreamEvent {
    StreamEvent::Data(StreamData::Trade {
        contract: Arc::clone(contract),
        ms_of_day: (idx % 86_400_000) as i32,
        sequence: idx as i32,
        ext_condition1: 0,
        ext_condition2: 0,
        ext_condition3: 0,
        ext_condition4: 0,
        condition: 0,
        size: 100,
        exchange: 0,
        price: Price::new(15025, 8).to_f64(),
        condition_flags: 0,
        price_flags: 0,
        volume_type: 0,
        records_back: 0,
        date: 20240315,
        received_at_ns: idx,
    })
}

/// Flood `n` synthetic `Trade` events through the real Disruptor pipeline,
/// firing the production Python callback path (`Python::attach` →
/// `fpss_event_to_typed` → `call1`) for each delivered event.
///
/// Returns `(delivered, elapsed_ns)`:
/// - `delivered` is the number of events the consumer actually fired the
///   callback for; the caller asserts `delivered == n` (zero-drop).
/// - `elapsed_ns` is the publish-loop + drain wall clock in nanoseconds,
///   measured on the consumer side. Setup (ring build, thread spawn) is
///   excluded — it runs before the timed region opens.
///
/// The GIL is released across the whole flood via `py.detach` so the
/// consumer thread can re-acquire it per event exactly as the live
/// dispatcher does; without this the producer would deadlock against the
/// consumer's `Python::attach`.
#[pyfunction]
pub(crate) fn __bench_flood_events(
    py: Python<'_>,
    n: u64,
    callback: Py<PyAny>,
) -> PyResult<(u64, u64)> {
    let callback = Arc::new(callback);
    let dispatch_cb = Arc::clone(&callback);

    let delivered = Arc::new(AtomicU64::new(0));
    let delivered_consumer = Arc::clone(&delivered);

    // Allocate the contract once; the publish closure clones the Arc per
    // event (refcount bump only). Same shape as the FPSS contract cache.
    let contract = Arc::new(Contract::stock("SPY"));

    // Build the Disruptor + consumer thread BEFORE the timed region. The
    // consumer body is the production handover: per-event GIL acquire,
    // typed-pyclass marshal, `call1`. Wrapped in `catch_unwind` to match
    // the live SSOT pipeline (a panicking callback must not abort the
    // consumer). `record_panic` accounting is the only production detail
    // intentionally omitted — it is a relaxed atomic add off the timed
    // path and would not change the ceiling.
    let factory = || RingSlot { event: None };
    let mut producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
                delivered_consumer.fetch_add(1, Ordering::Relaxed);
                Python::attach(|py| {
                    let typed = match fpss_event_to_typed(py, evt) {
                        Ok(obj) => obj,
                        Err(err) => {
                            err.write_unraisable(py, None);
                            return;
                        }
                    };
                    if let Err(err) = dispatch_cb.call1(py, (typed,)) {
                        err.write_unraisable(py, None);
                    }
                });
            }
        })
        .build();

    // Release the GIL for the whole flood: the consumer thread re-acquires
    // it per event via `Python::attach`. Holding it here would deadlock the
    // consumer. The timed region is the publish loop + producer drop
    // (which blocks until the consumer has drained every slot), so the
    // elapsed wall clock covers exactly the per-event marshal+callback
    // cost across all `n` events.
    let elapsed: Duration = py.detach(|| {
        let start = Instant::now();
        for i in 0..n {
            loop {
                let evt = make_event(&contract, i);
                if producer
                    .try_publish(|slot| {
                        slot.event = Some(evt);
                    })
                    .is_ok()
                {
                    break;
                }
                std::hint::spin_loop();
            }
        }
        // Dropping the producer joins the consumer thread, so by the time
        // this returns every published event has been delivered and the
        // callback has fired for each.
        drop(producer);
        start.elapsed()
    });

    Ok((delivered.load(Ordering::Relaxed), elapsed.as_nanos() as u64))
}
