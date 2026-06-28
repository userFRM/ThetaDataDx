//! Offline saturation bench hook for the TypeScript streaming callback path.
//!
//! `__benchFloodEvents(n, callback)` pushes `n` synthetic streaming events
//! through the REAL `ThreadsafeFunction` dispatch path — the same
//! `TsfnCallback` type, the same bounded `STREAMING_CALLBACK_QUEUE_DEPTH`
//! call queue, and the same per-event marshal (`fpss_event_to_buffered`
//! then `buffered_event_to_typed`) that the generated `startStreaming`
//! dispatcher runs in production. The only thing removed is the network:
//! events are synthesised in-process instead of decoded off a TLS socket.
//!
//! Faithfulness to the production path:
//! - The marshal + `tsfn.call(.., Blocking)` run on a `spawn_blocking`
//!   worker (OFF the libuv main thread), exactly as the live dispatcher
//!   closure does — the main thread stays free to drain the napi
//!   `uv_async_t` queue and run the JS callback.
//! - `ThreadsafeFunctionCallMode::Blocking` is used, so a full call queue
//!   makes the worker WAIT rather than silently dropping; under correct
//!   back-pressure no event is lost at the tsfn boundary.
//! - Each synthetic event is a `Trade` carrying an `Arc<Contract>` cloned
//!   per event (refcount bump only), identical to the live streaming decode
//!   path and to the Rust / Python benches.
//!
//! Return value: the number of `tsfn.call` invocations that returned a
//! non-`Ok` status (i.e. were rejected at the tsfn boundary — a tsfn-side
//! drop). Under `Blocking` mode against a live event loop this is `0`; the
//! caller asserts it. The caller separately verifies that the JS callback
//! actually fired `n` times (the receive-side zero-drop check) and that
//! the core ring `droppedEventCount()` stayed `0`.
//!
//! This is a BENCH-ONLY export. It is a napi free function whose name
//! (`__bench_flood_events`) is enrolled in `_is_ts_internal_free_fn` in
//! `scripts/ci/check_binding_parity.py`, so the parity gate does not treat
//! it as an untracked cross-binding utility — the same carve-out the
//! Arrow-IPC serialization free functions use. It carries no `[[utility]]`
//! row and no parity-matrix obligation.

use std::sync::Arc;

use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi_derive::napi;

use thetadatadx::fpss;
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{StreamData, StreamEvent as CoreStreamEvent};
use thetadatadx::Price;

use crate::{buffered_event_to_typed, fpss_event_to_buffered};

/// Build a synthetic `Trade` core event carrying a shared `Arc<Contract>`.
/// Mirrors `make_event` in `thetadatadx-rs/benches/streaming_throughput.rs`
/// and the Python bench so the three bindings flood byte-identical payloads.
#[inline]
fn make_event(contract: &Arc<Contract>, idx: u64) -> CoreStreamEvent {
    fpss::StreamEvent::Data(StreamData::Trade {
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

/// Flood `n` synthetic streaming `Trade` events through the real `TsfnCallback`
/// dispatch path to `callback`, returning the count of tsfn-boundary drops
/// (non-`Ok` `call` statuses) as an `f64` (JS `number`; `n` is bounded well
/// under 2^53 in practice, and the count is `0` on the healthy path).
///
/// The marshal + dispatch run on a blocking worker so the libuv main
/// thread is free to drain the napi call queue and run the JS callback —
/// the same threading split as `startStreaming`. The returned `Promise`
/// resolves once all `n` events have been QUEUED (every `call` returned);
/// the JS callback may still be draining the last queue-depth events when
/// the promise resolves, so the caller waits for the JS-side received
/// count to reach `n` before reading timings.
///
/// Bench-only. See the module doc for the parity-gate carve-out.
//
// The `callback` param is spelled with the same INLINE `ThreadsafeFunction`
// type as the production `start_streaming` (it is exactly what `TsfnCallback`
// aliases), not the `TsfnCallback` alias itself: napi-rs renders the inline
// generic as the JS `((arg: StreamEvent) => void)` callback type in
// `index.d.ts`, whereas it emits a type-alias name verbatim (an undeclared
// `TsfnCallback` type). Matching `start_streaming`'s spelling keeps the
// generated `.d.ts` valid and in sync with the committed stub (Gate 7).
#[napi(js_name = "__benchFloodEvents")]
pub async fn __bench_flood_events(
    n: u32,
    callback: napi::threadsafe_function::ThreadsafeFunction<
        crate::StreamEvent,
        (),
        crate::StreamEvent,
        napi::Status,
        false,
        false,
        { crate::STREAMING_CALLBACK_QUEUE_DEPTH },
    >,
) -> napi::Result<f64> {
    let callback = Arc::new(callback);
    let n = n as u64;

    let drops = tokio::task::spawn_blocking(move || {
        // Allocate the contract once; clone the Arc per event (refcount
        // bump only), same as the streaming contract cache hands out.
        let contract = Arc::new(Contract::stock("SPY"));
        let mut drops: u64 = 0;
        for i in 0..n {
            let core = make_event(&contract, i);
            // The EXACT production marshal: borrowed core event ->
            // BufferedEvent -> typed napi `StreamEvent`.
            let buffered = fpss_event_to_buffered(&core);
            let typed = buffered_event_to_typed(buffered);
            // The EXACT production dispatch: real bounded TsfnCallback,
            // Blocking mode. A full queue blocks the worker (back-pressure)
            // rather than dropping; a non-Ok status means the tsfn was
            // aborted/closing, which is a real drop and is counted.
            let status = callback.call(typed, ThreadsafeFunctionCallMode::Blocking);
            if status != napi::Status::Ok {
                drops += 1;
            }
        }
        drops
    })
    .await
    .map_err(|e| napi::Error::from_reason(format!("__benchFloodEvents worker panicked: {e}")))?;

    Ok(drops as f64)
}

/// LEVER 1 (batched delivery) — flood `n` synthetic events through the real
/// `ThreadsafeFunction` path, but carrying `batch_size` events per
/// `tsfn.call` hop (one `Array<StreamEvent>` per hop) instead of one event
/// per hop. Amortizes the per-event threadsafe-function crossing + V8
/// callback invocation over a whole batch.
///
/// Same production marshal per event (the typed-event conversion path,
/// `buffered_event_to_typed`); the only change is that `batch_size` typed
/// events are collected into a `Vec<StreamEvent>` (napi renders this as
/// `Array<StreamEvent>`) and handed to the callback in one hop. Runs on a
/// `spawn_blocking` worker, `Blocking` call mode, the same bounded queue.
///
/// Returns the count of `tsfn.call` invocations (i.e. batches) that returned
/// a non-`Ok` status (tsfn-boundary drops; `0` on the healthy path). The
/// caller asserts it AND verifies the JS side received `n` events total
/// across all batches.
///
/// Bench-only. See the module doc for the parity-gate carve-out. The
/// callback param uses the same INLINE `ThreadsafeFunction` spelling as the
/// per-event export (here parameterized on `Vec<StreamEvent>`), so napi
/// renders it as `((arg: Array<StreamEvent>) => void)` and the generated
/// `.d.ts` stays valid + in sync (Gate 7).
#[napi(js_name = "__benchFloodEventsBatched")]
pub async fn __bench_flood_events_batched(
    n: u32,
    batch_size: u32,
    callback: napi::threadsafe_function::ThreadsafeFunction<
        Vec<crate::StreamEvent>,
        (),
        Vec<crate::StreamEvent>,
        napi::Status,
        false,
        false,
        { crate::STREAMING_CALLBACK_QUEUE_DEPTH },
    >,
) -> napi::Result<f64> {
    let callback = Arc::new(callback);
    let n = n as u64;
    let cap = batch_size.max(1) as usize;

    let drops = tokio::task::spawn_blocking(move || {
        let contract = Arc::new(Contract::stock("SPY"));
        let mut drops: u64 = 0;
        let mut batch: Vec<crate::StreamEvent> = Vec::with_capacity(cap);
        for i in 0..n {
            let core = make_event(&contract, i);
            let buffered = fpss_event_to_buffered(&core);
            batch.push(buffered_event_to_typed(buffered));
            if batch.len() >= cap {
                // One hop carrying the whole batch. `std::mem::take` swaps in
                // a fresh empty Vec so the next batch fills without realloc
                // churn fighting the in-flight one.
                let payload = std::mem::replace(&mut batch, Vec::with_capacity(cap));
                let status = callback.call(payload, ThreadsafeFunctionCallMode::Blocking);
                if status != napi::Status::Ok {
                    drops += 1;
                }
            }
        }
        // Flush the final partial batch.
        if !batch.is_empty() {
            let status = callback.call(batch, ThreadsafeFunctionCallMode::Blocking);
            if status != napi::Status::Ok {
                drops += 1;
            }
        }
        drops
    })
    .await
    .map_err(|e| {
        napi::Error::from_reason(format!("__benchFloodEventsBatched worker panicked: {e}"))
    })?;

    Ok(drops as f64)
}

/// Project a synthetic `Trade` core event into the historical `TradeTick`
/// row shape the Arrow builder consumes (stock trade: sentinel option
/// fields, matching the Python Arrow lever's `trade_event_to_tick`).
#[inline]
fn trade_event_to_tick(event: &CoreStreamEvent) -> Option<thetadatadx::TradeTick> {
    match event {
        fpss::StreamEvent::Data(StreamData::Trade {
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
