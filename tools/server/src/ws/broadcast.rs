//! FPSS event fan-out: Disruptor callback -> broadcast task -> WS clients.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssControl, FpssEvent};

use crate::state::AppState;

use super::contract_map::lookup_event_contract;
use super::format::fpss_event_to_ws_json;

/// Start the FPSS -> WebSocket bridge via `ThetaDataDx::start_streaming()`.
///
/// The Disruptor callback runs on a blocking consumer thread and must stay
/// cheap. It only: (1) updates the contract map and connection flags,
/// (2) peeks the event's current contract under the map lock, and
/// (3) hands a cloned event + peeked `Arc<Contract>` snapshot to an
/// unbounded channel. A dedicated tokio task serializes the JSON and fans
/// out to every WS client.
///
/// # TOCTOU safety
///
/// The `(FpssEvent, Option<Arc<Contract>>)` tuple pins the contract snapshot
/// captured **at the exact moment the callback thread saw the event**.
/// Before this change, the broadcast task re-looked up the contract just
/// before serialization — which meant a concurrent map `clear()` triggered
/// by a reconnect or market-close could race in between, erasing the
/// contract and silently producing `{"id": N}` JSON with no root / strike /
/// right. That silent degradation is unacceptable across market-close /
/// reconnect boundaries. Peeking-before-send removes the race entirely:
/// the `Arc<Contract>` refcount holds the snapshot alive even after the
/// map is cleared or rewritten.
///
/// # Hot-path cost
///
/// Values in the contract map are stored as `Arc<Contract>`. The per-event
/// snapshot `Arc::clone` is a single refcount bump — no heap allocation,
/// no `String::clone` on `Contract::root`. At 100k events/sec this is the
/// difference between zero hot-path allocations and 100k `String` allocs/sec.
pub fn start_fpss_bridge(state: AppState) -> Result<(), thetadatadx::Error> {
    let contract_map: Arc<Mutex<HashMap<i32, Arc<Contract>>>> = state.contract_map();
    let map_for_cb = Arc::clone(&contract_map);
    let state_for_cb = state.clone();
    let state_for_task = state.clone();

    // Unbounded mpsc keeps the Disruptor callback non-blocking even if the
    // broadcast task is briefly slow. Memory is bounded by channel drain
    // rate; clients get bounded per-client backpressure inside broadcast_ws.
    //
    // Per-tick clone is intentionally cheap: `FpssData::{Quote,Trade,Ohlcvc,
    // OpenInterest}` carry only primitives plus `Arc<str>` for symbol, so
    // `event.clone()` is a field copy + refcount bump — not a heap allocation
    // on the hot path. The `Option<Arc<Contract>>` tail is a refcount bump on
    // a shared `Contract` already in the map, not a fresh `Contract` clone
    // (which would allocate the `root: String` per event).
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(FpssEvent, Option<Arc<Contract>>)>();

    // Observability counter for the `tx.send` drop path below. Lives on the
    // callback closure so it survives the Disruptor consumer thread's
    // lifetime; shared with broadcast diagnostics via `tracing::debug!`.
    let dropped_broadcast: Arc<std::sync::atomic::AtomicU64> =
        Arc::new(std::sync::atomic::AtomicU64::new(0));
    let dropped_broadcast = Arc::clone(&dropped_broadcast);

    tokio::spawn(async move {
        while let Some((event, peeked)) = rx.recv().await {
            // Snapshot already taken on the callback thread — just encode
            // and broadcast. No map lock acquired in the broadcast task.
            // Borrow through the `Arc` (deref) so the serializer sees the
            // same `&Contract` it always did.
            let json = fpss_event_to_ws_json(&event, peeked.as_deref());
            if let Some(ws_json) = json {
                let msg: Arc<str> = Arc::from(ws_json);
                state_for_task.broadcast_ws(msg).await;
            }
        }
    });

    state.tdx().start_streaming(move |event: &FpssEvent| {
        // Track contract assignments. Must happen on the callback thread so
        // the broadcast task sees the mapping before it serializes the next
        // event that references it.
        if let FpssEvent::Control(FpssControl::ContractAssigned { id, contract }) = event {
            // Recover from poisoning rather than silently dropping all
            // future ContractAssigned events. If a previous lock-holder
            // panicked, the map state may be partial but that is strictly
            // less bad than losing every subsequent symbol assignment.
            let mut map = map_for_cb.lock().unwrap_or_else(|e| e.into_inner());
            // One `Contract::clone` on INSERT (rare — only on contract
            // assignment), then every subsequent per-event lookup is an
            // `Arc::clone` (refcount bump). This is the key perf win
            // vs the old `HashMap<i32, Contract>` that forced a
            // `Contract::clone` (with `String::clone` of `root`) on every
            // single event.
            map.insert(*id, Arc::new(contract.clone()));
        }

        // Update connection status.
        match event {
            FpssEvent::Control(FpssControl::LoginSuccess { .. }) => {
                state_for_cb.set_fpss_connected(true);
            }
            FpssEvent::Control(FpssControl::Disconnected { .. }) => {
                state_for_cb.set_fpss_connected(false);
            }
            _ => {}
        }

        // Peek the contract for this event NOW, while the callback thread
        // still holds the causal ordering with `ContractAssigned` /
        // reconnect clears. Cloning the `Contract` value captures a
        // snapshot that no subsequent map mutation can invalidate — even
        // if a reconnect clears the map before the broadcast task wakes,
        // the cloned `Contract` travels with the event downstream. This
        // is the documented fix for the reconnect / market-close silent-
        // degradation race; see module docs for detail.
        let peeked = lookup_event_contract(event, &map_for_cb);

        // Hand off for serialization + broadcast. Callback returns
        // immediately. A `SendError` here means the broadcast task has
        // exited (shutdown, panic, receiver dropped) — route it to
        // `tracing::debug!` with a monotonically-increasing counter so
        // soak tests can detect back-pressure / task death, matching the
        // observability pattern used by the SDK streaming callbacks
        // (see `crates/thetadatadx/build_support/sdk_surface/`).
        if tx.send((event.clone(), peeked)).is_err() {
            let dropped = dropped_broadcast
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                .wrapping_add(1);
            tracing::debug!(
                target: "thetadatadx::server::ws",
                dropped_total = dropped,
                "fpss event dropped — broadcast task is gone"
            );
        }
    })?;

    state.set_fpss_connected(true);
    Ok(())
}
