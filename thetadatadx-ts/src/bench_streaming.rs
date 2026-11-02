//! Offline saturation bench hook for the TypeScript streaming callback path.
//!
//! `__benchFloodEvents(n, callback)` pushes `n` synthetic streaming events
//! through the REAL delivery path: the same
//! [`crate::fpss_client::batched_delivery`] handler and drain scope that
//! `startStreaming` runs, so the same per-event marshal
//! (`fpss_event_to_buffered` then `buffered_event_to_typed`), the same arrays
//! of up to `STREAMING_CALLBACK_BATCH` events and the same bounded
//! `TsfnCallback` queue. The only thing removed is the network: the events
//! are synthesised in-process and handed over as one ring drain instead of
//! being decoded off a TLS socket.
//!
//! The hand-over runs on a `spawn_blocking` worker (OFF the libuv main
//! thread), exactly as the live dispatcher does, so the main thread stays free
//! to drain the napi `uv_async_t` queue and run the JS callback. The calls are
//! `Blocking`, so a full call queue makes the worker wait rather than drop.
//! The promise rejects if a hand-over failed; the caller separately verifies
//! that the JS side received all `n` events.
//!
//! This is a BENCH-ONLY export, not part of the cross-binding surface.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi_derive::napi;

use thetadatadx::Price;
use thetadatadx::fpss;
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{StreamData, StreamEvent as CoreStreamEvent};

use crate::TsfnCallback;
use crate::fpss_client::batched_delivery;

/// Build a synthetic `Trade` core event carrying a shared `Arc<Contract>`.
/// Mirrors `make_event` in `thetadatadx-rs/benches/streaming_throughput.rs`
/// and the Python bench so the three bindings flood byte-identical payloads.
#[inline]
fn make_event(contract: &Arc<Contract>, idx: u64) -> CoreStreamEvent {
    fpss::StreamEvent::Data(StreamData::Trade {
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

/// Flood `n` synthetic streaming `Trade` events through the production
/// delivery path to `callback` as one ring drain.
///
/// The returned `Promise` resolves once every array has been QUEUED; the JS
/// callback may still be draining the last arrays when it resolves, so the
/// caller waits for the JS-side received count to reach `n` before reading
/// timings.
///
/// Bench-only. See the module doc for the parity-gate carve-out.
#[napi(
    js_name = "__benchFloodEvents",
    ts_args_type = "n: number, callback: ((arg: Array<StreamEvent>) => void)"
)]
pub async fn __bench_flood_events(n: u32, callback: TsfnCallback) -> napi::Result<()> {
    let callback = Arc::new(callback);
    let failed = tokio::task::spawn_blocking(move || {
        let closing_expected = Arc::new(AtomicBool::new(false));
        let delivery_failed = Arc::new(AtomicBool::new(false));
        let (mut handler, mut scope) =
            batched_delivery(&callback, &closing_expected, &delivery_failed);
        // Allocate the contract once; clone the Arc per event (refcount
        // bump only), same as the streaming contract cache hands out.
        let contract = Arc::new(Contract::stock("SPY"));
        let outcome = scope(&mut || {
            for i in 0..u64::from(n) {
                handler(&make_event(&contract, i));
            }
            fpss::PollOutcome::Drained(n as usize)
        });
        matches!(outcome, fpss::PollOutcome::Failed)
    })
    .await
    .map_err(|e| napi::Error::from_reason(format!("__benchFloodEvents worker panicked: {e}")))?;
    if failed {
        return Err(napi::Error::from_reason(
            "__benchFloodEvents: delivery failed",
        ));
    }
    Ok(())
}

/// Project a synthetic `Trade` core event into the market-data `TradeTick`
/// row shape the Arrow builder consumes (stock trade: sentinel option
/// fields, matching the Python Arrow lever's `trade_event_to_tick`).
#[inline]
fn trade_event_to_tick(event: &CoreStreamEvent) -> Option<thetadatadx::TradeTick> {
    match event {
        fpss::StreamEvent::Data(StreamData::Trade {
            ms_of_day,
            sequence,
            condition,
            size,
            exchange,
            price,
            date,
            ..
        }) => Some(thetadatadx::TradeTick {
            ms_of_day: *ms_of_day,
            sequence: *sequence,
            condition: *condition,
            size: *size,
            exchange: *exchange,
            price: *price,
            date: *date,
            // The stream sends `condition` and `exchange`; it sends no
            // extended-condition column at all, so those carry a cleared
            // presence flag and reach Arrow as nulls rather than as the
            // condition code zero.
            has_condition: true,
            has_exchange: true,
            has_ext_condition1: false,
            has_ext_condition2: false,
            has_ext_condition3: false,
            has_ext_condition4: false,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            expiration: 0,
            strike: 0.0,
            right: '\0',
        }),
        _ => None,
    }
}

/// Serialize a `TradeTick` slice to an Arrow IPC stream byte buffer via the
/// SAME machinery the SDK's `tradeTickToArrowIpc` export uses
/// (`TicksArrowExt::to_arrow` -> `arrow_ipc::writer::StreamWriter`).
fn trade_ticks_to_arrow_ipc(rows: &[thetadatadx::TradeTick]) -> napi::Result<Vec<u8>> {
    let batch = thetadatadx::frames::TicksArrowExt::to_arrow(rows)
        .map_err(|e| napi::Error::from_reason(format!("arrow conversion failed: {e}")))?;
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = arrow_ipc::writer::StreamWriter::try_new(
            std::io::Cursor::new(&mut buf),
            &batch.schema(),
        )
        .map_err(|e| napi::Error::from_reason(format!("arrow ipc writer init failed: {e}")))?;
        writer
            .write(&batch)
            .map_err(|e| napi::Error::from_reason(format!("arrow ipc write failed: {e}")))?;
        writer
            .finish()
            .map_err(|e| napi::Error::from_reason(format!("arrow ipc finish failed: {e}")))?;
    }
    Ok(buf)
}

/// LEVER 3 (TypeScript columnar bulk) — flood `n` synthetic trade events,
/// accumulate `batch_size` of them as `TradeTick` rows, serialize ONE Arrow
/// `RecordBatch` to an Arrow IPC byte buffer per batch (the same
/// `TicksArrowExt::to_arrow` -> `StreamWriter` path the SDK's
/// `tradeTickToArrowIpc` export uses), and cross the `ThreadsafeFunction`
/// boundary ONCE per batch carrying that `Buffer` — NOT N JS objects.
///
/// This bypasses the per-event `buffered_event_to_typed` JS-object
/// construction entirely: the Node callback receives an Arrow IPC `Buffer`
/// it decodes columnar via `apache-arrow` (`tableFromIPC`). DIFFERENT
/// delivery model than the per-event / array-batch callbacks — Node gets a
/// columnar Table, not typed event objects — so its number is the columnar
/// bulk ceiling, the TypeScript analogue of the Python Arrow lever.
///
/// Returns the count of `tsfn.call` invocations (batches) that returned a
/// non-`Ok` status (tsfn-boundary drops; `0` on the healthy path). The
/// caller asserts it AND verifies the Arrow row count summed across batches
/// equals `n`.
///
/// Bench-only. See the module doc for the parity-gate carve-out. `T` is a
/// napi `Buffer`, which napi renders as `((arg: Buffer) => void)` in the
/// generated `.d.ts`.
#[napi(js_name = "__benchFloodEventsArrowIpc")]
pub async fn __bench_flood_events_arrow_ipc(
    n: u32,
    batch_size: u32,
    callback: napi::threadsafe_function::ThreadsafeFunction<
        napi::bindgen_prelude::Buffer,
        (),
        napi::bindgen_prelude::Buffer,
        napi::Status,
        false,
        false,
        { crate::STREAMING_CALLBACK_QUEUE_DEPTH },
    >,
) -> napi::Result<f64> {
    let callback = Arc::new(callback);
    let n = n as u64;
    let cap = batch_size.max(1) as usize;

    let drops = tokio::task::spawn_blocking(move || -> napi::Result<u64> {
        let contract = Arc::new(Contract::stock("SPY"));
        let mut drops: u64 = 0;
        let mut rows: Vec<thetadatadx::TradeTick> = Vec::with_capacity(cap);
        let flush = |rows: &mut Vec<thetadatadx::TradeTick>, drops: &mut u64| -> napi::Result<()> {
            if rows.is_empty() {
                return Ok(());
            }
            // Build + serialize one Arrow IPC buffer for the batch, hand it
            // across the boundary in one hop.
            let ipc = trade_ticks_to_arrow_ipc(rows)?;
            let status = callback.call(
                napi::bindgen_prelude::Buffer::from(ipc),
                ThreadsafeFunctionCallMode::Blocking,
            );
            if status != napi::Status::Ok {
                *drops += 1;
            }
            rows.clear();
            Ok(())
        };
        for i in 0..n {
            let core = make_event(&contract, i);
            if let Some(tick) = trade_event_to_tick(&core) {
                rows.push(tick);
                if rows.len() >= cap {
                    flush(&mut rows, &mut drops)?;
                }
            }
        }
        flush(&mut rows, &mut drops)?;
        Ok(drops)
    })
    .await
    .map_err(|e| {
        napi::Error::from_reason(format!("__benchFloodEventsArrowIpc worker panicked: {e}"))
    })??;

    Ok(drops as f64)
}
