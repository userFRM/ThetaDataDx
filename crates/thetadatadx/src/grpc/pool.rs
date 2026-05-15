//! Round-robin pool of [`Channel`] handles.
//!
//! A single h2 connection multiplexes many concurrent streams, but
//! `MAX_CONCURRENT_STREAMS` is finite (the default upstream cap is
//! `~100`). [`ChannelPool`] keeps `N` parallel channels and hands them
//! out round-robin via [`ChannelPool::next`], so a workload that
//! exceeds the per-connection limit fans out across distinct h2
//! connections rather than blocking on stream availability.
//!
//! The pool is `Arc`-clone-cheap and `Send + Sync`; callers can clone
//! it freely across tasks. Each [`ChannelPool::next`] returns a
//! reference into the pool — the underlying [`Channel`] is owned by
//! the pool and lives as long as the pool does.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::channel::Channel;
use super::decoder_pool::DecoderPool;

/// Round-robin pool of pre-opened [`Channel`]s.
///
/// Construction opens `N` connections in parallel; if any one fails
/// the entire pool construction fails and any already-opened channels
/// are dropped (which cancels their connection-driver tasks).
#[derive(Clone)]
pub struct ChannelPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    channels: Vec<Channel>,
    /// Round-robin cursor. Wraps at `usize::MAX` — the modulo by
    /// `channels.len()` makes the wraparound transparent to callers.
    cursor: AtomicUsize,
    /// Dedicated decoder pool shared across all channels in this
    /// pool. Held here (rather than only on each `Channel` via the
    /// attached `DecoderHandle`) so the pool's threads stay alive
    /// for the full lifetime of the `ChannelPool` — if every
    /// in-flight stream finished and dropped its handle, the
    /// remaining clones on the channels would still keep the
    /// decoders running, but holding an explicit reference here
    /// makes the lifecycle contract obvious to readers.
    ///
    /// `None` means no decoder pool was wired in at construction;
    /// channels then fall back to inline decode. Production paths
    /// always wire one in via [`ChannelPool::from_channels_with_decoders`].
    _decoder_pool: Option<DecoderPool>,
}

impl ChannelPool {
    /// Wrap a caller-supplied set of channels in a pool. No decoder
    /// pool is attached; channels fall back to inline zstd +
    /// protobuf decode on the caller's tokio task. Production paths
    /// should use [`ChannelPool::from_channels_with_decoders`] so
    /// the heavy decode work runs off-reactor.
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
        Self {
            inner: Arc::new(PoolInner {
                channels,
                cursor: AtomicUsize::new(0),
                _decoder_pool: None,
            }),
        }
    }

    /// Wrap a caller-supplied set of channels in a pool, attaching
    /// a [`DecoderPool`] so every RPC dispatched through the
    /// pool routes its decode work to dedicated threads.
    ///
    /// Each channel is bound to one decoder handle, distributed
    /// round-robin across the pool's decoder ring set; channels and
    /// decoders need not be equal in count (the modulo wraps).
    /// Cloning the handles is cheap (multi-producer reference
    /// count) so the channel-to-decoder mapping does not constrain
    /// pool sizing.
    ///
    /// # Panics
    ///
    /// Panics if `channels` is empty for the same reason as
    /// [`Self::from_channels`].
    #[must_use]
    pub fn from_channels_with_decoders(channels: Vec<Channel>, decoder_pool: DecoderPool) -> Self {
        assert!(
            !channels.is_empty(),
            "ChannelPool must hold at least one Channel"
        );
        let channels = channels
            .into_iter()
            .enumerate()
            .map(|(idx, ch)| ch.with_decoder(decoder_pool.handle(idx).clone()))
            .collect();
        Self {
            inner: Arc::new(PoolInner {
                channels,
                cursor: AtomicUsize::new(0),
                _decoder_pool: Some(decoder_pool),
            }),
        }
    }

    /// Number of channels in the pool.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.channels.len()
    }

    /// Always `false` — the pool is non-empty by construction.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Look up a pool member by index. Hidden from the public docs —
    /// exposed so integration tests can deterministically target a
    /// specific channel (e.g. fire a slow RPC against pool member 0
    /// to saturate it, then assert subsequent `next()` calls route
    /// around it).
    #[doc(hidden)]
    #[must_use]
    pub fn member_for_test(&self, idx: usize) -> &Channel {
        &self.inner.channels[idx]
    }

    /// Pick the next channel for an outbound RPC.
    ///
    /// Picks the channel with the fewest in-flight streams, breaking
    /// ties via the round-robin cursor. This avoids the head-of-line
    /// blocking that strict round-robin would create when one
    /// channel is slow / saturated (e.g. holding a long server-
    /// streaming response) and the others still have h2 stream
    /// credit.
    ///
    /// The in-flight count is maintained by the [`Channel`] itself:
    /// `server_streaming_frame` increments at dispatch, the
    /// [`super::ServerStreaming`] drop guard decrements at stream
    /// end. Relaxed ordering on the counter is sufficient — the
    /// pool's pick is a load-balancing hint, not a correctness
    /// barrier, so a slightly stale read is acceptable.
    ///
    /// Round-robin tie-breaking ensures fairness when the pool is
    /// idle (all members at `0` in-flight) and prevents a heavily-
    /// loaded callsite from pinning to a single channel for sticky
    /// reasons.
    #[must_use]
    pub fn next(&self) -> &Channel {
        let len = self.inner.channels.len();
        let cursor = self.inner.cursor.fetch_add(1, Ordering::Relaxed);
        // Single-member pool: skip the scan; the one channel is the
        // only choice regardless of saturation.
        if len == 1 {
            return &self.inner.channels[0];
        }
        // Scan all members and track the index with the fewest
        // in-flight streams. Ties are broken in favour of the
        // channel closest to the round-robin cursor — so when the
        // pool is idle every member sees its share of traffic
        // rather than pinning the first one.
        let mut best_idx = cursor % len;
        let mut best_count = self.inner.channels[best_idx].in_flight_count();
        for offset in 1..len {
            let idx = (cursor.wrapping_add(offset)) % len;
            let count = self.inner.channels[idx].in_flight_count();
            if count < best_count {
                best_idx = idx;
                best_count = count;
            }
        }
        &self.inner.channels[best_idx]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Counts how many times each pool index is picked.
    #[derive(Default)]
    struct PickCounter {
        per_index: Vec<AtomicUsize>,
    }

    impl PickCounter {
        fn new(n: usize) -> Self {
            Self {
                per_index: (0..n).map(|_| AtomicUsize::new(0)).collect(),
            }
        }

        fn increment(&self, idx: usize) {
            self.per_index[idx].fetch_add(1, Ordering::Relaxed);
        }

        fn snapshot(&self) -> Vec<usize> {
            self.per_index
                .iter()
                .map(|c| c.load(Ordering::Relaxed))
                .collect()
        }
    }

    // ── Pool semantics without real h2 channels ──────────────────────
    //
    // The pool's correctness is mechanical (atomic cursor + indexed
    // access). Testing it via real `Channel::connect_h2c` calls adds
    // network noise to a logic test, so the property tests below use
    // an `IndexedPool` shim that exposes the same round-robin
    // semantics over `usize` indices. `ChannelPool::next` is then
    // covered end-to-end by the bench, which builds a real
    // 4-connection pool against the mock h2 server.

    struct IndexedPool {
        cursor: AtomicUsize,
        size: usize,
    }

    impl IndexedPool {
        fn new(size: usize) -> Self {
            assert!(size > 0);
            Self {
                cursor: AtomicUsize::new(0),
                size,
            }
        }

        fn next(&self) -> usize {
            self.cursor.fetch_add(1, Ordering::Relaxed) % self.size
        }
    }

    #[test]
    fn round_robin_distributes_evenly_over_full_cycles() {
        let pool = IndexedPool::new(4);
        let counter = PickCounter::new(4);
        for _ in 0..4_000 {
            counter.increment(pool.next());
        }
        let counts = counter.snapshot();
        // 4000 picks across 4 indices — each index gets exactly 1000.
        for (i, c) in counts.iter().enumerate() {
            assert_eq!(*c, 1000, "index {i} got {c} picks, expected 1000");
        }
    }

    #[test]
    fn round_robin_distributes_within_one_under_partial_cycle() {
        let pool = IndexedPool::new(4);
        let counter = PickCounter::new(4);
        for _ in 0..7 {
            counter.increment(pool.next());
        }
        let counts = counter.snapshot();
        // 7 picks across 4 indices — three indices get 2 picks, one
        // gets 1. No index can be > max+1 above any other.
        let min = *counts.iter().min().unwrap();
        let max = *counts.iter().max().unwrap();
        assert!(
            max - min <= 1,
            "imbalance > 1: counts={counts:?} min={min} max={max}"
        );
    }

    #[test]
    fn round_robin_is_thread_safe() {
        let pool = Arc::new(IndexedPool::new(8));
        let counter = Arc::new(PickCounter::new(8));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let pool = Arc::clone(&pool);
                let counter = Arc::clone(&counter);
                std::thread::spawn(move || {
                    for _ in 0..1_000 {
                        counter.increment(pool.next());
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("worker thread joined");
        }
        let counts = counter.snapshot();
        let total: usize = counts.iter().sum();
        assert_eq!(total, 16_000, "every pick was counted exactly once");
        // 16000 picks across 8 indices = 2000 each on average; with
        // `Relaxed` atomics under contention some imbalance is normal
        // but should stay within a small fraction.
        let min = *counts.iter().min().unwrap();
        let max = *counts.iter().max().unwrap();
        assert!(
            max - min <= 8,
            "imbalance > 8: counts={counts:?} min={min} max={max}"
        );
    }

    #[test]
    #[should_panic(expected = "ChannelPool must hold at least one Channel")]
    fn empty_pool_panics() {
        let _ = ChannelPool::from_channels(Vec::new());
    }
}
