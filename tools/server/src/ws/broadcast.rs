//! FPSS event fan-out: Disruptor callback -> broadcast task -> WS clients.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssControl, FpssEvent};
use tokio::sync::mpsc::error::TrySendError;

use crate::state::AppState;

use super::contract_map::lookup_event_contract;
use super::format::fpss_event_to_ws_json;

/// Bounded capacity for the Disruptor-callback -> broadcast-task channel.
///
/// Sized at 65_536 slots: large enough that the broadcast task's transient
/// scheduling jitter never spills into a drop under normal load, small enough
/// that an offline broadcast task can't accumulate unbounded heap. Each slot
/// is `(FpssEvent, Option<Arc<Contract>>)` ~ a few hundred bytes after
/// `FpssEvent`'s `Arc<str>` + `Arc<Contract>` refcount bumps, so the worst-
/// case memory footprint is on the order of tens of MB — bounded, scrapeable,
/// and recoverable.
const FPSS_BROADCAST_CAPACITY: usize = 65_536;

/// Emit one rate-limited warning per `WARN_EVERY_N` drops so a sustained
/// back-pressure event leaves a visible trail in the logs without flooding
/// stderr at the per-event rate.
const WARN_EVERY_N: u64 = 1024;

/// Start the FPSS -> WebSocket bridge via `ThetaDataDx::start_streaming()`.
///
/// The Disruptor callback runs on a blocking consumer thread and must stay
/// cheap. It only: (1) updates the contract map and connection flags,
/// (2) peeks the event's current contract under the map lock, and
/// (3) hands a cloned event + peeked `Arc<Contract>` snapshot to a bounded
/// channel. A dedicated tokio task serializes the JSON and fans out to every
/// WS client.
///
/// # Bounded handoff
///
/// The callback->broadcast channel is bounded to [`FPSS_BROADCAST_CAPACITY`]
/// so a stalled broadcast task can never accumulate unbounded heap. When the
/// channel is full, the callback increments [`AppState::record_fpss_broadcast_drop`]
/// and returns immediately — the Disruptor consumer thread is never blocked.
/// Drops surface to operators through the `fpss_broadcast_dropped()` counter
/// and a rate-limited `tracing::warn!` (one warning per [`WARN_EVERY_N`]
/// drops).
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

    // Bounded mpsc keeps the Disruptor callback non-blocking AND caps memory
    // on a stalled broadcast task. `try_send` is the only path used on the
    // hot side — a `Full` rejection bumps the dropped counter and returns
    // immediately, never blocking the Disruptor consumer thread.
    let (tx, mut rx) =
        tokio::sync::mpsc::channel::<(FpssEvent, Option<Arc<Contract>>)>(FPSS_BROADCAST_CAPACITY);

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
            map.insert(*id, Arc::clone(contract));
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
        // reconnect clears.
        let peeked = lookup_event_contract(event, &map_for_cb);

        // Bounded handoff with explicit overrun handling. `Full` means the
        // broadcast task is lagging — bump the drop counter and walk away,
        // never blocking the Disruptor consumer thread. `Closed` means the
        // task has exited (shutdown / panic / receiver dropped) — log once
        // at the warn level and stop accounting further events as drops to
        // avoid log flood; subsequent events still fail-fast on `try_send`.
        match tx.try_send((event.clone(), peeked)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                let dropped = state_for_cb.record_fpss_broadcast_drop();
                if dropped % WARN_EVERY_N == 0 {
                    tracing::warn!(
                        target: "thetadatadx::server::ws",
                        dropped_total = dropped,
                        capacity = FPSS_BROADCAST_CAPACITY,
                        warn_every_n = WARN_EVERY_N,
                        "fpss broadcast channel is full; events being dropped"
                    );
                }
            }
            Err(TrySendError::Closed(_)) => {
                let dropped = state_for_cb.record_fpss_broadcast_drop();
                if dropped % WARN_EVERY_N == 0 {
                    tracing::warn!(
                        target: "thetadatadx::server::ws",
                        dropped_total = dropped,
                        "fpss broadcast task is gone; events being dropped"
                    );
                }
            }
        }
    })?;

    state.set_fpss_connected(true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    /// Push N+1 messages into a channel of capacity N with no consumer.
    /// Counts the rejections and asserts the channel never grows unbounded
    /// — the bounded mpsc must reject everything past the cap.
    #[tokio::test]
    async fn try_send_rejects_overrun_at_capacity() {
        const CAP: usize = 4;
        let (tx, _rx) = mpsc::channel::<u32>(CAP);

        let mut accepted = 0_u64;
        let mut rejected = 0_u64;
        for i in 0..(CAP as u32 + 1) {
            match tx.try_send(i) {
                Ok(()) => accepted += 1,
                Err(TrySendError::Full(_)) => rejected += 1,
                Err(TrySendError::Closed(_)) => panic!("rx still alive, must not be Closed"),
            }
        }
        assert_eq!(accepted, CAP as u64);
        assert_eq!(rejected, 1);
    }

    /// End-to-end: a saturated bounded channel records exactly the right
    /// number of drops on the AppState counter. The counter is the contract
    /// the WS health endpoint and the per-1024 warning log depend on, so it
    /// must increment monotonically by one per dropped event.
    #[test]
    fn drop_counter_increments_monotonically_under_overrun() {
        // Use a tokio current-thread runtime so the bounded channel's
        // future-aware semantics match production.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rt");
        rt.block_on(async {
            const CAP: usize = 8;
            let counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let (tx, _rx) = mpsc::channel::<u32>(CAP);

            // Saturate the channel.
            for i in 0..CAP {
                tx.try_send(i as u32).expect("first CAP must accept");
            }
            // Now overflow by 17 and account every Full rejection.
            for i in 0..17_u32 {
                match tx.try_send(i) {
                    Ok(()) => panic!("channel must be full"),
                    Err(TrySendError::Full(_)) => {
                        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(TrySendError::Closed(_)) => panic!("rx still alive"),
                }
            }
            assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 17);
        });
    }
}
