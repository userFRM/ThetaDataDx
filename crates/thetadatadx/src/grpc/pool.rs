//! Least-loaded pool of [`Channel`] handles.
//!
//! A single HTTP/2 connection multiplexes many concurrent streams, but
//! every connection carries exactly one connection-level flow-control
//! window and `MAX_CONCURRENT_STREAMS` is finite. [`ChannelPool`] keeps
//! `N` parallel channels — one HTTP/2 connection each — and hands them
//! out via [`ChannelPool::next`], picking the channel with the fewest
//! in-flight streams. Measured against a single multiplexed connection
//! carrying the same workload, the per-worker connection fan-out
//! delivers roughly 1.8x the small-frame throughput and 2.3x the
//! large-frame throughput at the 16-concurrent account ceiling (see
//! `docs/architecture/in-house-grpc-transport.md`, "Migration"), so the
//! pool survives the transport swap unchanged.
//!
//! The pool is `Arc`-clone-cheap and `Send + Sync`; callers can clone
//! it freely across tasks. Each [`ChannelPool::next`] returns a lease
//! into the pool — the underlying [`Channel`] is owned by the pool as
//! an `Arc<Channel>` and lives as long as the pool does.
//!
//! # Reconnect, in place
//!
//! When an RPC dispatched through a pool channel observes
//! [`super::ChannelError::ConnectionClosed`], the underlying stack
//! lazily replaces the dead HTTP/2 connection on the next dispatch.
//! The pool slot does NOT get marked dead, replaced, or skipped — the
//! same `Arc<Channel>` handle the picker returned remains valid.

use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::channel::{Channel, InFlightToken};

/// Least-loaded pool of pre-opened [`Channel`]s.
#[derive(Clone)]
pub struct ChannelPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    channels: Vec<Arc<Channel>>,
    /// Round-robin cursor for tie-breaking. Wraps at `usize::MAX` —
    /// the modulo by `channels.len()` makes the wraparound transparent
    /// to callers.
    ///
    /// Seeded with a per-process random offset (see [`seeded_cursor`])
    /// rather than `0`: with a zero seed, every freshly-started
    /// process pins its first RPC to channel index 0 — i.e. the same
    /// upstream connection across an entire fleet restarting after an
    /// outage. The random offset spreads first-RPC fan-out across the
    /// channel set immediately.
    cursor: AtomicUsize,
}

impl ChannelPool {
    /// Wrap a caller-supplied set of channels in a pool.
    ///
    /// # Panics
    ///
    /// Panics if `channels` is empty. A pool must have at least one
    /// member; an empty pool has no semantics that would make
    /// `next()` succeed.
    #[must_use]
    pub fn from_channels(channels: Vec<Channel>) -> Self {
        assert!(
            !channels.is_empty(),
            "ChannelPool must hold at least one Channel"
        );
        let channels: Vec<Arc<Channel>> = channels.into_iter().map(Arc::new).collect();
        let cursor = seeded_cursor(channels.len());
        Self {
            inner: Arc::new(PoolInner { channels, cursor }),
        }
    }

    /// Number of channels in the pool.
    ///
    /// Reachable only under `__test-helpers` — production code uses the
    /// pool through [`Self::next`] and does not introspect membership.
    #[cfg(feature = "__test-helpers")]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.channels.len()
    }

    /// `true` if the pool holds no channels. The constructor's
    /// `assert!` rules this out in practice; the method exists so
    /// callers using the `len() + is_empty()` clippy-friendly idiom
    /// don't have to special-case the pool. Reachable only under
    /// `__test-helpers`.
    #[cfg(feature = "__test-helpers")]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.channels.is_empty()
    }

    /// Look up a pool member by index. Hidden from the public docs —
    /// exposed so integration tests can deterministically target a
    /// specific channel (e.g. fire a slow RPC against pool member 0
    /// to saturate it, then assert subsequent `next()` calls route
    /// around it). Reachable only under `__test-helpers`.
    #[cfg(feature = "__test-helpers")]
    #[doc(hidden)]
    #[must_use]
    pub fn member_for_test(&self, idx: usize) -> &Arc<Channel> {
        &self.inner.channels[idx]
    }

    /// Pick the next channel for an outbound RPC.
    ///
    /// Returns a [`ChannelLease`] that pre-reserves a slot on the
    /// picked channel synchronously, before the async dispatch
    /// future even runs. Under burst contention every concurrent
    /// `pool.next()` observer sees the prior reservations and
    /// routes around the loaded channels — eliminating the head-
    /// of-line blocking that occurs when a `join_all` batch of
    /// `pool.next()` calls all evaluate before any of the
    /// returned channels' async dispatch increments land. The
    /// lease derefs to `&Arc<Channel>` so existing dispatch shapes
    /// keep working unchanged.
    ///
    /// Picks the channel with the fewest in-flight streams,
    /// breaking ties via the round-robin cursor. This avoids the
    /// head-of-line blocking that strict round-robin would create
    /// when one channel is slow / saturated (e.g. holding a long
    /// server-streaming response) and the others still have h2
    /// stream credit. Round-robin tie-breaking ensures fairness
    /// when the pool is idle and prevents a heavily-loaded callsite
    /// from pinning to a single channel for sticky reasons.
    ///
    /// The in-flight counter is bumped at three points:
    ///   1. Here, when the lease is constructed — covers the
    ///      window between `pool.next()` returning and the async
    ///      dispatch future actually running.
    ///   2. Inside [`Channel::server_streaming`], when the open
    ///      path commits — covers the entire RPC lifetime.
    ///   3. Decremented when the lease drops (after the dispatch
    ///      future is constructed) and again when the resulting
    ///      `ServerStreaming` drops (after the response stream
    ///      ends). Net commitment per RPC: exactly 1 from open to
    ///      stream end.
    ///
    /// Relaxed ordering on the counter is sufficient — the pool's
    /// pick is a load-balancing hint, not a correctness barrier,
    /// so a slightly stale read is acceptable.
    pub fn next(&self) -> ChannelLease<'_> {
        let len = self.inner.channels.len();
        let cursor = self.inner.cursor.fetch_add(1, Ordering::Relaxed);
        // Single-member pool: skip the load-balancing scan; the one
        // channel is the only choice regardless of saturation.
        if len == 1 {
            let channel = &self.inner.channels[0];
            let token = channel.reserve_in_flight();
            return ChannelLease {
                channel,
                _token: token,
            };
        }

        // Bounded CAS-retry; on exhaustion, degrade to round-robin commit.
        const PICK_RETRY: usize = 4;
        for _ in 0..PICK_RETRY {
            let mut best_idx: usize = cursor % len;
            let mut best_count: usize = self.inner.channels[best_idx].in_flight_count();
            for offset in 1..len {
                let idx = (cursor.wrapping_add(offset)) % len;
                let count = self.inner.channels[idx].in_flight_count();
                if count < best_count {
                    best_idx = idx;
                    best_count = count;
                }
            }
            let channel = &self.inner.channels[best_idx];
            match channel.try_reserve_in_flight(best_count) {
                Ok(token) => {
                    return ChannelLease {
                        channel,
                        _token: token,
                    };
                }
                Err(_actual_prior) => {
                    // Lost the race — another task committed a
                    // reservation between our scan and our commit.
                    // Loop body re-scans with a fresh snapshot.
                    continue;
                }
            }
        }
        // Retry budget exhausted: degrade to round-robin pick and
        // commit unconditionally. The picker is a load-balancing
        // hint, not a correctness barrier, so accepting a sub-
        // optimal pick beats spinning.
        let idx = cursor % len;
        let channel = &self.inner.channels[idx];
        ChannelLease {
            channel,
            _token: channel.reserve_in_flight(),
        }
    }
}

/// Build the pool's round-robin cursor with a per-instance random
/// starting offset in `[0, len)`.
///
/// The cursor is a `usize` that only ever increments; callers reduce
/// it modulo `channels.len()`, so any starting offset is equivalent to
/// rotating the channel order. Randomising the rotation per pool
/// instance spreads each process's first RPCs across the channel set
/// instead of pinning them to index 0 — without it, a fleet restarting
/// together after an outage lands its entire first burst on the same
/// upstream connection.
fn seeded_cursor(len: usize) -> AtomicUsize {
    let seed = crate::backoff::entropy_u64();
    let offset = (seed as usize) % len.max(1);
    AtomicUsize::new(offset)
}

/// Pre-dispatch reservation on a pooled [`Channel`].
///
/// Returned from [`ChannelPool::next`]. The lease bumps the picked
/// channel's in-flight counter on construction and decrements it on
/// drop, so a synchronous batch of `pool.next()` calls — whose
/// returned dispatch futures will only run later — still sees the
/// prior reservations and routes around the loaded channels.
///
/// Derefs to `&Arc<Channel>` so the existing
/// `pool.next().server_streaming(...).await` shape continues to
/// compile unchanged. Hold the lease at least as long as the
/// dispatch future you build from it — the lease's drop is what
/// releases the pre-dispatch reservation back to the pool. Once the
/// open path returns a `ServerStreaming`, the stream's own in-flight
/// token keeps the channel marked busy for the rest of the RPC
/// lifetime, so dropping the lease after that point is the correct
/// shape.
pub struct ChannelLease<'a> {
    channel: &'a Arc<Channel>,
    _token: InFlightToken,
}

impl<'a> Deref for ChannelLease<'a> {
    type Target = Arc<Channel>;

    fn deref(&self) -> &Self::Target {
        self.channel
    }
}

impl<'a> ChannelLease<'a> {
    /// Borrow the underlying channel handle. The lease will hold
    /// the reservation alive for the rest of the temporary scope;
    /// callers that need to thread `&Arc<Channel>` into a longer-lived
    /// future should keep the lease bound to a local `let`.
    ///
    /// Production code uses the `Deref<Target = Arc<Channel>>` impl
    /// above instead; this explicit accessor is exposed under
    /// `__test-helpers` so tests can name the borrow without going
    /// through deref coercion.
    #[cfg(feature = "__test-helpers")]
    #[must_use]
    pub fn channel(&self) -> &Arc<Channel> {
        self.channel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pick-distribution semantics are covered end-to-end by the
    // `channel_pool_*` tests in `tests/grpc_mock_server.rs`, which
    // drive the real `ChannelPool::next` against multi-channel pools
    // wired to mock h2 listeners.

    #[test]
    #[should_panic(expected = "ChannelPool must hold at least one Channel")]
    fn empty_pool_panics() {
        let _ = ChannelPool::from_channels(Vec::new());
    }

    /// The cursor seed must stay inside `[0, len)` so the very first
    /// `next()` pick is a valid index, and repeated pool constructions
    /// must not all land on the same offset (the offset is the whole
    /// point of the seed).
    #[test]
    fn seeded_cursor_offsets_spread_within_bounds() {
        let len = 8;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let cursor = seeded_cursor(len);
            let offset = cursor.load(Ordering::Relaxed);
            assert!(offset < len, "seed offset must be reduced modulo len");
            seen.insert(offset);
        }
        assert!(
            seen.len() > 1,
            "64 pool constructions must not all seed the same cursor offset"
        );
        // Degenerate single-channel pool: offset must be 0.
        assert_eq!(seeded_cursor(1).load(Ordering::Relaxed), 0);
    }
}
