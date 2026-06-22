//! Offline saturation bench hook for the TypeScript streaming callback path.
//!
//! `__benchFloodEvents(n, callback)` pushes `n` synthetic FPSS events
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
//!   per event (refcount bump only), identical to the live FPSS decode
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
/// Mirrors `make_event` in `crates/thetadatadx/benches/streaming_throughput.rs`
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

/// Flood `n` synthetic FPSS `Trade` events through the real `TsfnCallback`
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
        // bump only), same as the FPSS contract cache hands out.
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
