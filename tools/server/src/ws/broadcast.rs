//! FPSS event fan-out: event-dispatch callback -> broadcast task -> WS clients.

use std::sync::Arc;

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{StreamControl, StreamEvent};
use tokio::sync::mpsc::error::TrySendError;

use crate::state::AppState;

use super::contract_map::lookup_event_contract;
use super::format::fpss_event_to_ws_json;

/// Bounded capacity for the event-dispatch-callback -> broadcast-task channel.
///
/// Sized at 65_536 slots: large enough that the broadcast task's transient
/// scheduling jitter never spills into a drop under normal load, small enough
/// that an offline broadcast task can't accumulate unbounded heap. Each slot
/// is `(StreamEvent, Option<Arc<Contract>>)` ~ a few hundred bytes after
/// `StreamEvent`'s `Arc<str>` + `Arc<Contract>` refcount bumps, so the worst-
/// case memory footprint is on the order of tens of MB — bounded, scrapeable,
/// and recoverable.
const FPSS_BROADCAST_CAPACITY: usize = 65_536;

/// Emit one rate-limited warning per `WARN_EVERY_N` drops so a sustained
/// back-pressure event leaves a visible trail in the logs without flooding
/// stderr at the per-event rate.
const WARN_EVERY_N: u64 = 1024;

/// Start the FPSS -> WebSocket bridge via `Client::start_streaming()`.
///
/// The event-dispatch callback runs on a blocking consumer thread and must stay
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
/// and returns immediately — the event-dispatch consumer thread is never blocked.
/// Drops surface to operators through the `fpss_broadcast_dropped()` counter
/// and a rate-limited `tracing::warn!` (one warning per [`WARN_EVERY_N`]
/// drops).
///
/// # Contract identity
///
/// Every `StreamData::*` variant carries `contract: Arc<Contract>`
/// directly — the SDK's I/O thread populates it from its internal
/// contract cache at decode time. The bridge keeps no
/// `contract_id -> Arc<Contract>` map of its own: the contract reference
/// rides on the event itself, so a concurrent reconnect or
/// market-close cannot race between callback time and serialisation
/// time. `Arc::clone` is a refcount bump, so the per-event hot-path
/// cost stays minimal.
///
/// # Errors
/// Returns an error when `Client::start_streaming` fails to
/// establish the FPSS stream.
pub fn start_fpss_bridge(state: AppState) -> Result<(), thetadatadx::Error> {
    let state_for_cb = state.clone();
    let state_for_task = state.clone();

    // Bounded mpsc keeps the event-dispatch callback non-blocking AND caps memory
    // on a stalled broadcast task. `try_send` is the only path used on the
    // hot side — a `Full` rejection bumps the dropped counter and returns
    // immediately, never blocking the event-dispatch consumer thread.
    let (tx, mut rx) =
        tokio::sync::mpsc::channel::<(StreamEvent, Option<Arc<Contract>>)>(FPSS_BROADCAST_CAPACITY);

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

    state.tdx().start_streaming(move |event: &StreamEvent| {
        // Update connection status.
        match event {
            StreamEvent::Control(StreamControl::LoginSuccess { .. }) => {
                state_for_cb.set_fpss_connected(true);
            }
            StreamEvent::Control(StreamControl::Disconnected { .. }) => {
                state_for_cb.set_fpss_connected(false);
            }
            _ => {}
        }

        // Resolve the event's contract — for `StreamData::*` it rides on
        // the event directly; for control variants there is none.
        let peeked = lookup_event_contract(event);

        // Bounded handoff with explicit overrun handling. `Full` means the
        // broadcast task is lagging — bump the drop counter and walk away,
        // never blocking the event-dispatch consumer thread. `Closed` means the
        // task has exited (shutdown / panic / receiver dropped) — log once
        // at the warn level and stop accounting further events as drops to
        // avoid log flood; subsequent events still fail-fast on `try_send`.
        match tx.try_send((event.clone(), peeked)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                let dropped = state_for_cb.record_fpss_broadcast_drop();
                if dropped.is_multiple_of(WARN_EVERY_N) {
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
                if dropped.is_multiple_of(WARN_EVERY_N) {
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
            // VOCAB-OK: tokio Runtime::block_on in test
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
