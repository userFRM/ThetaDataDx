//! CPU core affinity for the streaming event-ring consumer thread.
//!
//! The streaming ring runs in polling mode: no processor thread is
//! spawned, so the consumer is whichever thread drives the drain loop
//! ([`crate::fpss::StreamingClient::for_each`] /
//! [`crate::fpss::StreamingClient::next_event`] and the binding
//! dispatchers built on them). Pinning that thread to an isolated core
//! gives deterministic, low-jitter tick delivery.
//!
//! The disruptor builder's `pin_at_core` only affects handler-mode
//! processor threads, which this crate does not spawn, so pinning is
//! applied here on the real drain thread instead.

use std::sync::atomic::{AtomicI64, Ordering};

/// Records the most recent core id the consumer drain loop attempted to
/// pin to, as a test seam. `-1` means "no pin attempted".
///
/// The drain loop calls [`pin_consumer_thread`] at most once per drive;
/// this counter lets a unit test assert a `Some(core)` config reaches
/// the pin path without depending on the host actually having that core
/// online (the real `set_for_current` no-ops / returns `false` on an
/// absent core).
#[cfg(any(test, feature = "__test-helpers"))]
pub(crate) static LAST_PINNED_CORE: AtomicI64 = AtomicI64::new(-1);

#[cfg(not(any(test, feature = "__test-helpers")))]
#[allow(dead_code)]
static LAST_PINNED_CORE: AtomicI64 = AtomicI64::new(-1);

/// Pin the calling thread to `core`, if requested.
///
/// `None` leaves the thread under the OS scheduler (the default). For
/// `Some(core)` this asks the OS to pin the current thread to that core;
/// an out-of-range or offline core is a best-effort no-op (a `warn` is
/// logged) rather than a hard failure, matching the lenient
/// config-validation posture for tuning knobs.
///
/// Idempotent and cheap to call once at drain-loop entry — it performs
/// at most one affinity syscall and never touches the per-event hot
/// path.
pub(crate) fn pin_consumer_thread(core: Option<usize>) {
    let Some(core_id) = core else {
        return;
    };

    // Record the attempt for the test seam regardless of outcome.
    if let Ok(recorded) = i64::try_from(core_id) {
        LAST_PINNED_CORE.store(recorded, Ordering::Relaxed);
    }

    let available = core_affinity::get_core_ids().unwrap_or_default();
    match available.into_iter().find(|c| c.id == core_id) {
        Some(target) => {
            if core_affinity::set_for_current(target) {
                tracing::debug!(core = core_id, "pinned streaming consumer thread to core");
            } else {
                tracing::warn!(
                    core = core_id,
                    "failed to pin streaming consumer thread; continuing unpinned"
                );
            }
        }
        None => {
            tracing::warn!(
                core = core_id,
                "requested consumer core is not available; continuing unpinned"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_does_not_record_a_pin() {
        LAST_PINNED_CORE.store(-1, Ordering::Relaxed);
        pin_consumer_thread(None);
        assert_eq!(LAST_PINNED_CORE.load(Ordering::Relaxed), -1);
    }

    #[test]
    fn some_records_the_requested_core() {
        // Use a deliberately high core id: it is almost certainly absent
        // on CI, so this exercises the "record the attempt, then no-op
        // gracefully" path without depending on real affinity support.
        LAST_PINNED_CORE.store(-1, Ordering::Relaxed);
        pin_consumer_thread(Some(4096));
        assert_eq!(LAST_PINNED_CORE.load(Ordering::Relaxed), 4096);
    }
}
