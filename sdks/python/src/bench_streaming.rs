//! Offline saturation bench hooks for the Python streaming callback path.
//!
//! All hooks drive the SAME LMAX Disruptor pipeline the live streaming consumer
//! uses (single producer, single consumer thread, `RING_SIZE` slots). They
//! differ only in HOW the consumer hands events across the Python boundary,
//! so the numbers isolate the boundary-crossing cost (the Disruptor itself
//! runs ~39 M events/s in the core Rust bench, far above the boundary, so it
//! is never the bottleneck here).
//!
//! - [`__bench_flood_events`] — PER-EVENT baseline. For every delivered
//!   event: `Python::attach` (per-event, the production granularity — see
//!   `_generated/streaming_methods.rs`), `fpss_event_to_typed` (the exact
//!   production marshal, reused), `call1((typed,))` (the production
//!   1-tuple vectorcall).
//!
//! - [`__bench_flood_events_batched_calls`] — LEVER 1a. Amortize only the
//!   GIL acquire: the consumer buffers `batch_size` events, then takes ONE
//!   `Python::attach` and loops `call1((typed,))` per event inside it. Same
//!   per-event marshal + per-event call as the baseline; the only thing
//!   removed is the per-event GIL reacquisition.
//!
//! - [`__bench_flood_events_batched_list`] — LEVER 1b. Amortize the GIL
//!   acquire AND the call dispatch: the consumer buffers `batch_size`
//!   events, marshals them into one Python `list`, and fires ONE
//!   `call1((list,))` per batch. One boundary crossing per batch.
//!
//! - [`__bench_flood_events_arrow`] — LEVER 3. Columnar bulk delivery: the
//!   consumer buffers `batch_size` events as `TradeTick` rows, builds ONE
//!   Arrow `RecordBatch`, and hands it to Python zero-copy over the Arrow C
//!   Stream Interface via the SDK's own `trade_tick_slice_to_arrow_table`
//!   (the same path `<TickList>.to_arrow()` uses). This is a DIFFERENT
//!   delivery model than the per-event callback — Python receives a
//!   columnar `pyarrow.Table`, not N typed event objects — so its number is
//!   the columnar bulk ceiling, not a like-for-like callback rate.
//!
//! Every hook retries the publish on `RingBufferFull` so all `n` events are
//! delivered (no silent drops); the delivered count is returned and the
//! caller asserts `delivered == n`. The returned wall clock is the publish
//! loop + drain, measured on the consumer side; interpreter spin-up, ring
//! allocation, and thread spawn run before the timed region opens and are
//! excluded (mirroring the `iter_custom` setup-exclusion in the core Rust
//! bench `crates/thetadatadx/benches/streaming_throughput.rs`).
//!
//! These are bench-only `#[pyfunction]`s, NOT part of the public utility
//! surface. They are enrolled in `PY_NON_UTILITY_PYFUNCTIONS` in
//! `scripts/ci/check_binding_parity.py` alongside the other offline hooks so
//! the parity gate does not mistake them for untracked cross-binding
//! utilities.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use disruptor::{build_single_producer, BusySpin, Producer, Sequence};
use pyo3::prelude::*;
use pyo3::types::PyList;

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{StreamData, StreamEvent};
use thetadatadx::Price;

use crate::fpss_event_to_typed;
use crate::slice_arrow::trade_tick_slice_to_arrow_table;

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
/// streaming decode path and to the core Rust bench's `make_event`.
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

/// Project a `&StreamEvent::Data(Trade)` into the historical `TradeTick`
/// row shape the Arrow builder consumes. Non-trade events are skipped
/// (the bench only floods trades). Field-for-field copy — same columns the
/// `TradeTick` Arrow schema declares.
#[inline]
fn trade_event_to_tick(event: &StreamEvent) -> Option<thetadatadx::TradeTick> {
    match event {
        StreamEvent::Data(StreamData::Trade {
            ms_of_day,
            sequence,
            ext_condition1,
            ext_condition2,
            ext_condition3,
            ext_condition4,
            condition,
            size,
            exchange,
            price,
            condition_flags,
            price_flags,
            volume_type,
            records_back,
            date,
            ..
        }) => Some(thetadatadx::TradeTick {
            ms_of_day: *ms_of_day,
            sequence: *sequence,
            ext_condition1: *ext_condition1,
            ext_condition2: *ext_condition2,
            ext_condition3: *ext_condition3,
            ext_condition4: *ext_condition4,
            condition: *condition,
            size: *size,
            exchange: *exchange,
            price: *price,
            condition_flags: *condition_flags,
            price_flags: *price_flags,
            volume_type: *volume_type,
            records_back: *records_back,
            date: *date,
            // Stock trade: no option fields. `TradeTick` uses sentinel
            // values for absent option metadata (the Arrow builder maps
            // these sentinels to nulls): `0` expiration, `0.0` strike,
            // `'\0'` right.
            expiration: 0,
            strike: 0.0,
            right: '\0',
        }),
        _ => None,
    }
}

/// Build the Disruptor + consumer thread with a user-supplied consumer
/// `body`, drive `n` synthetic trades through the publish loop (retry on
/// overflow so every event is delivered), and return `(delivered,
/// elapsed_ns)`. `body` runs on the consumer thread for every delivered
/// event; `flush` runs once after the publish loop drains so a batching
/// body can emit its final partial batch. Setup (ring build, thread spawn)
/// is excluded from the timed region.
fn run_pipeline<B, F>(py: Python<'_>, n: u64, body: B, flush: F) -> (u64, u64)
where
    B: FnMut(&StreamEvent) + Send + 'static,
    F: FnOnce() + Send,
{
    let delivered = Arc::new(AtomicU64::new(0));
    let delivered_consumer = Arc::clone(&delivered);
    let contract = Arc::new(Contract::stock("SPY"));

    let mut body = body;
    let factory = || RingSlot { event: None };
    let mut producer = build_single_producer(RING_SIZE, factory, BusySpin)
        .handle_events_with(move |slot: &RingSlot, _seq: Sequence, _eob: bool| {
            if let Some(ref evt) = slot.event {
                delivered_consumer.fetch_add(1, Ordering::Relaxed);
                body(evt);
            }
        })
        .build();

    // Release the GIL for the whole flood: the consumer thread re-acquires
    // it (per event or per batch, depending on `body`). Holding it here
    // would deadlock the consumer against its own `Python::attach`.
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
        // returns. `flush` then emits any buffered partial batch.
        drop(producer);
        flush();
        start.elapsed()
    });

    (delivered.load(Ordering::Relaxed), elapsed.as_nanos() as u64)
}

/// PER-EVENT baseline. One `Python::attach` + one `fpss_event_to_typed` +
/// one `call1` per delivered event. Returns `(delivered, elapsed_ns)`; the
/// caller asserts `delivered == n` (zero-drop).
#[pyfunction]
pub(crate) fn __bench_flood_events(
    py: Python<'_>,
    n: u64,
    callback: Py<PyAny>,
) -> PyResult<(u64, u64)> {
    let dispatch_cb = Arc::new(callback);
    let body = move |evt: &StreamEvent| {
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
    };
    Ok(run_pipeline(py, n, body, || {}))
}

/// Marshal + dispatch one buffered batch under ONE GIL acquisition, one
/// `call1((typed,))` per event (Lever 1a emit). Clears `events`.
fn emit_batch_calls(events: &mut Vec<StreamEvent>, cb: &Py<PyAny>) {
    if events.is_empty() {
        return;
    }
    Python::attach(|py| {
        for evt in events.iter() {
            match fpss_event_to_typed(py, evt) {
                Ok(typed) => {
                    if let Err(err) = cb.call1(py, (typed,)) {
                        err.write_unraisable(py, None);
                    }
                }
                Err(err) => err.write_unraisable(py, None),
            }
        }
    });
    events.clear();
}

/// LEVER 1a — amortize the GIL acquire only. The consumer buffers
/// `batch_size` borrowed events (cloned into an owned buffer so they
/// outlive the ring slot), then takes ONE `Python::attach` and fires one
/// `call1((typed,))` per event inside it. Per-event marshal + per-event
/// call are unchanged from the baseline; only the per-event GIL
/// reacquisition is removed.
#[pyfunction]
pub(crate) fn __bench_flood_events_batched_calls(
    py: Python<'_>,
    n: u64,
    batch_size: u64,
    callback: Py<PyAny>,
) -> PyResult<(u64, u64)> {
    let dispatch_cb = Arc::new(callback);
    let cap = batch_size.max(1) as usize;
    let buf: Arc<std::sync::Mutex<Vec<StreamEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::with_capacity(cap)));

    let flush_buf = Arc::clone(&buf);
    let flush_cb = Arc::clone(&dispatch_cb);
    let body = move |evt: &StreamEvent| {
        let mut v = buf.lock().unwrap_or_else(|e| e.into_inner());
        v.push(evt.clone());
        if v.len() >= cap {
            emit_batch_calls(&mut v, &dispatch_cb);
        }
    };
    let flush = move || {
        let mut v = flush_buf.lock().unwrap_or_else(|e| e.into_inner());
        emit_batch_calls(&mut v, &flush_cb);
    };
    Ok(run_pipeline(py, n, body, flush))
}

/// Marshal one buffered batch into a single Python `list` and fire ONE
/// `call1((list,))` (Lever 1b emit). Clears `events`.
fn emit_batch_list(events: &mut Vec<StreamEvent>, cb: &Py<PyAny>) {
    if events.is_empty() {
        return;
    }
    Python::attach(|py| {
        let items: Vec<Py<PyAny>> = events
            .iter()
            .filter_map(|evt| match fpss_event_to_typed(py, evt) {
                Ok(typed) => Some(typed),
                Err(err) => {
                    err.write_unraisable(py, None);
                    None
                }
            })
            .collect();
        match PyList::new(py, items) {
            Ok(list) => {
                if let Err(err) = cb.call1(py, (list,)) {
                    err.write_unraisable(py, None);
                }
            }
            Err(err) => err.write_unraisable(py, None),
        }
    });
    events.clear();
}

/// LEVER 1b — amortize the GIL acquire AND the call dispatch. The consumer
/// buffers `batch_size` events, marshals them into one Python `list` of
/// typed event objects, and fires ONE `call1((list,))` per batch. One
/// boundary crossing per batch instead of per event.
#[pyfunction]
pub(crate) fn __bench_flood_events_batched_list(
    py: Python<'_>,
    n: u64,
    batch_size: u64,
    callback: Py<PyAny>,
) -> PyResult<(u64, u64)> {
    let dispatch_cb = Arc::new(callback);
    let cap = batch_size.max(1) as usize;
    let buf: Arc<std::sync::Mutex<Vec<StreamEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::with_capacity(cap)));

    let flush_buf = Arc::clone(&buf);
    let flush_cb = Arc::clone(&dispatch_cb);
    let body = move |evt: &StreamEvent| {
        let mut v = buf.lock().unwrap_or_else(|e| e.into_inner());
        v.push(evt.clone());
        if v.len() >= cap {
            emit_batch_list(&mut v, &dispatch_cb);
        }
    };
    let flush = move || {
        let mut v = flush_buf.lock().unwrap_or_else(|e| e.into_inner());
        emit_batch_list(&mut v, &flush_cb);
    };
    Ok(run_pipeline(py, n, body, flush))
}

/// LEVER 3 — columnar bulk delivery over the Arrow C Stream Interface. The
/// consumer buffers `batch_size` events as `TradeTick` rows, builds ONE
/// Arrow `RecordBatch` via the SDK's `trade_tick_slice_to_arrow_table`
/// (zero-copy export to a `pyarrow.Table` over `FFI_ArrowArrayStream`), and
/// fires ONE `call1((table,))` per batch.
///
/// DIFFERENT delivery model than the per-event callback: Python receives a
/// columnar `pyarrow.Table`, not typed event objects. The number is the
/// columnar bulk ceiling, not a like-for-like callback rate.
#[pyfunction]
pub(crate) fn __bench_flood_events_arrow(
    py: Python<'_>,
    n: u64,
    batch_size: u64,
    callback: Py<PyAny>,
) -> PyResult<(u64, u64)> {
    let dispatch_cb = Arc::new(callback);
    let cap = batch_size.max(1) as usize;
    let buf: Arc<std::sync::Mutex<Vec<thetadatadx::TradeTick>>> =
        Arc::new(std::sync::Mutex::new(Vec::with_capacity(cap)));

    let flush_buf = Arc::clone(&buf);
    let flush_cb = Arc::clone(&dispatch_cb);
    let body = move |evt: &StreamEvent| {
        if let Some(tick) = trade_event_to_tick(evt) {
            let mut v = buf.lock().unwrap_or_else(|e| e.into_inner());
            v.push(tick);
            if v.len() >= cap {
                emit_batch_arrow(&mut v, &dispatch_cb);
            }
        }
    };
    let flush = move || {
        let mut v = flush_buf.lock().unwrap_or_else(|e| e.into_inner());
        emit_batch_arrow(&mut v, &flush_cb);
    };
    Ok(run_pipeline(py, n, body, flush))
}

/// Build one Arrow `RecordBatch` from a buffered `TradeTick` slice and hand
/// it to Python zero-copy via the SDK's `trade_tick_slice_to_arrow_table`,
/// then fire ONE `call1((table,))` (Lever 3 emit). Clears `rows`.
fn emit_batch_arrow(rows: &mut Vec<thetadatadx::TradeTick>, cb: &Py<PyAny>) {
    if rows.is_empty() {
        return;
    }
    Python::attach(|py| match trade_tick_slice_to_arrow_table(py, rows) {
        Ok(table) => {
            if let Err(err) = cb.call1(py, (table,)) {
                err.write_unraisable(py, None);
            }
        }
        Err(err) => err.write_unraisable(py, None),
    });
    rows.clear();
}
