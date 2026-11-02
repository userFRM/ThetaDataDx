//! Offline saturation bench hook for the TypeScript streaming callback path.
//!
//! `__benchFloodEvents(n, callback)` pushes `n` synthetic streaming events
//! through the REAL delivery path: the same
//! [`crate::fpss_client::batched_delivery`] handler and drain scope that
//! `startStreaming` runs, so the same per-event marshal
//! (`fpss_event_to_buffered` then `buffered_event_to_typed`), the same arrays
//! of up to `STREAMING_CALLBACK_BATCH` events and the same bounded
//! `TsfnCallback` queue. The only thing removed is the network: the events
//! are synthesised in-process and handed over as one ring drain, then the
//! stream ends, instead of being decoded off a TLS socket.
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
        scope(&mut || {
            for i in 0..u64::from(n) {
                handler(&make_event(&contract, i));
            }
            fpss::PollOutcome::Drained(n as usize)
        });
        // End the stream so the last partial array is handed over.
        let outcome = scope(&mut || fpss::PollOutcome::Shutdown);
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
