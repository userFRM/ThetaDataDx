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

use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::channel::{Channel, InFlightToken};
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
    /// Returns a [`ChannelLease`] that pre-reserves a slot on the
    /// picked channel synchronously, before the async dispatch
    /// future even runs. Under burst contention every concurrent
    /// `pool.next()` observer sees the prior reservations and
    /// routes around the loaded channels — eliminating the head-
    /// of-line blocking that occurs when a `join_all` batch of
    /// `pool.next()` calls all evaluate before any of the
    /// returned channels' async dispatch increments land. The
    /// lease derefs to `&Channel` so the existing dispatch shape
    /// (`pool.next().server_streaming(...).await`) keeps working
    /// unchanged.
    ///
    /// Picks the channel with the fewest in-flight streams,
    /// breaking ties via the round-robin cursor. This avoids the
    /// head-of-line blocking that strict round-robin would create
    /// when one channel is slow / saturated (e.g. holding a long
    /// server-streaming response) and the others still have h2
    /// stream credit. Round-robin tie-breaking ensures fairness
    /// when the pool is idle (all members at the same in-flight
    /// count) and prevents a heavily-loaded callsite from pinning
    /// to a single channel for sticky reasons.
    ///
    /// The in-flight counter is bumped at three points:
    ///   1. Here, when the lease is constructed — covers the
    ///      window between `pool.next()` returning and the async
    ///      dispatch future actually running.
    ///   2. Inside [`Channel::server_streaming_frame`], when the
    ///      open path commits — covers the entire RPC lifetime.
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
            return ChannelLease {
                channel,
                _token: channel.reserve_in_flight(),
            };
        }

        // CAS-retry pick: scan for the least-loaded LIVE channel,
        // then commit the reservation only if the channel is still
        // at the observed count. Under true concurrency two tasks
        // may both scan and both pick the same least-loaded
        // channel; the commit guard ensures the loser rolls back
        // its speculative increment and re-scans rather than
        // pinning to a now-saturated channel.
        //
        // Dead-channel routing (issue #577 #3): channels that have
        // observed `ChannelError::ConnectionClosed` are skipped
        // during the live scan. If ALL channels are dead the picker
        // falls through to the dead pool so the RPC can still
        // surface its terminal error -- the alternative (block on
        // pool death) would mask the underlying problem.
        //
        // Bounded retries: PICK_RETRY caps live-lock under heavy
        // contention. On exhaustion we fall back to a round-robin
        // pick that always commits -- the load-balancing hint is
        // degraded but the dispatch is never lost.
        const PICK_RETRY: usize = 4;
        for _ in 0..PICK_RETRY {
            // First pass: scan only LIVE channels for the
            // least-loaded one.
            let mut best_idx: Option<usize> = None;
            let mut best_count: usize = usize::MAX;
            for offset in 0..len {
                let idx = (cursor.wrapping_add(offset)) % len;
                let ch = &self.inner.channels[idx];
                if ch.is_dead() {
                    continue;
                }
                let count = ch.in_flight_count();
                if best_idx.is_none() || count < best_count {
                    best_idx = Some(idx);
                    best_count = count;
                }
            }
            // No live channel left -- last-resort: route to a dead
            // channel so the caller observes the terminal error
            // and can recycle the pool. The alternative (block)
            // would hide the root cause behind a hang.
            let pick_idx = best_idx.unwrap_or(cursor % len);
            let channel = &self.inner.channels[pick_idx];
            // `best_count` is `usize::MAX` when we degraded to
            // dead-channel routing -- ignore the load barrier in
            // that case so the CAS commit cannot bounce.
            let load_barrier = if best_idx.is_some() {
                best_count
            } else {
                usize::MAX
            };
            match channel.try_reserve_in_flight(load_barrier) {
                Ok(token) => {
                    return ChannelLease {
                        channel,
                        _token: token,
                    };
                }
                Err(_actual_prior) => {
                    // Lost the race -- another task committed a
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

    /// Whether every channel in the pool has been marked dead. Used
    /// by the channel-recycling path on `MddsClient` to decide
    /// whether to rebuild the pool atomically -- a partial pool
    /// (one or two dead channels in a pool of four) is left to the
    /// picker, which routes around the dead members. A fully-dead
    /// pool needs explicit reconstruction.
    #[must_use]
    pub fn all_dead(&self) -> bool {
        self.inner.channels.iter().all(crate::grpc::Channel::is_dead)
    }

    /// Number of channels currently marked dead in the pool. Exposed
    /// so the diagnostic surface on `MddsClient` can report the
    /// current health without scanning each member individually.
    #[doc(hidden)]
    #[must_use]
    pub fn dead_count(&self) -> usize {
        self.inner
            .channels
            .iter()
            .filter(|c| c.is_dead())
            .count()
    }
}

/// Pre-dispatch reservation on a pooled [`Channel`].
///
/// Returned from [`ChannelPool::next`]. The lease bumps the picked
/// channel's in-flight counter on construction and decrements it on
/// drop, so a synchronous batch of `pool.next()` calls — whose
/// returned dispatch futures will only run later — still sees the
/// prior reservations and routes around the loaded channels.
///
/// Derefs to `&Channel` so the existing
/// `pool.next().server_streaming(...).await` shape continues to
/// compile unchanged. Hold the lease at least as long as the
/// dispatch future you build from it — the lease's drop is what
/// releases the pre-dispatch reservation back to the pool. Once the
/// open path returns a `ServerStreaming`, the stream's own
/// `InFlightToken` keeps the channel marked busy for the rest of
/// the RPC lifetime, so dropping the lease after that point is the
/// correct shape.
pub struct ChannelLease<'a> {
    channel: &'a Channel,
    _token: InFlightToken,
}

impl<'a> Deref for ChannelLease<'a> {
    type Target = Channel;

    fn deref(&self) -> &Self::Target {
        self.channel
    }
}

impl<'a> ChannelLease<'a> {
    /// Borrow the underlying channel reference. The lease will hold
    /// the reservation alive for the rest of the temporary scope;
    /// callers that need to thread `&Channel` into a longer-lived
    /// future should keep the lease bound to a local `let`.
    #[must_use]
    pub fn channel(&self) -> &Channel {
        self.channel
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

    // ── Dead-channel routing (issue #577 #3) ──────────────────────────
    //
    // Drive a 3-channel pool with one or more members marked dead and
    // assert `next()` skips them while at least one live member
    // remains. Real `Channel`s are built over `tokio::io::duplex`
    // pairs so the pool sees real `Channel` values without needing a
    // network. The server-side of each duplex is parked on
    // `accept()` — we never actually open a stream, so the test only
    // exercises the picker logic.

    use crate::grpc::Channel;
    use crate::grpc::codec::DEFAULT_MAX_MESSAGE_SIZE;
    use http::uri::Scheme;

    /// Build a pool of `n` channels over `tokio::io::duplex` pairs.
    /// Server tasks are returned so the test can keep them alive
    /// (dropping them would tear down the duplex and the channel
    /// would immediately observe `ConnectionClosed`, defeating the
    /// dead-channel test). Each `Channel`'s `dead` flag starts
    /// `false`; callers flip it via `mark_dead()` to simulate the
    /// cascade.
    async fn build_pool_with_duplex_channels(
        n: usize,
    ) -> (ChannelPool, Vec<tokio::task::JoinHandle<()>>) {
        let mut channels = Vec::with_capacity(n);
        let mut server_tasks = Vec::with_capacity(n);
        for _ in 0..n {
            let (client_io, server_io) = tokio::io::duplex(64 * 1024);
            let server_task = tokio::spawn(async move {
                let mut conn = h2::server::handshake(server_io)
                    .await
                    .expect("server handshake");
                // Park forever -- the test does not open streams.
                let _ = conn.accept().await;
            });
            let channel = Channel::handshake_for_test(
                client_io,
                "127.0.0.1",
                0,
                DEFAULT_MAX_MESSAGE_SIZE,
                Scheme::HTTP,
            )
            .await
            .expect("client handshake");
            channels.push(channel);
            server_tasks.push(server_task);
        }
        (ChannelPool::from_channels(channels), server_tasks)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn next_skips_dead_channels_while_live_members_remain() {
        let (pool, _server_tasks) = build_pool_with_duplex_channels(3).await;
        // Mark channel 0 dead -- the picker must route every
        // subsequent `next()` to channels 1 or 2.
        pool.inner.channels[0].mark_dead();

        let mut dead_pick_count = 0;
        let mut live_pick_count = 0;
        for _ in 0..30 {
            let lease = pool.next();
            // Identity test: compare pointers, not field values --
            // each `Channel` is its own allocation in the
            // `Vec<Channel>`, so address comparison is sound.
            if std::ptr::eq(lease.channel(), &pool.inner.channels[0]) {
                dead_pick_count += 1;
            } else {
                live_pick_count += 1;
            }
            // Drop the lease so the next pick sees a fresh
            // in-flight snapshot.
            drop(lease);
        }
        assert_eq!(
            dead_pick_count, 0,
            "dead channel must be skipped while live members remain"
        );
        assert_eq!(live_pick_count, 30, "all 30 picks should hit a live member");
        assert_eq!(pool.dead_count(), 1);
        assert!(!pool.all_dead());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn next_falls_through_to_dead_channels_when_pool_is_fully_dead() {
        // Last-resort routing: when EVERY member is dead, the
        // picker still returns a (dead) channel rather than blocking.
        // The caller's RPC then observes the terminal error and the
        // retry shell can surface it -- the alternative (block on
        // dead pool) would mask the root cause behind a hang.
        let (pool, _server_tasks) = build_pool_with_duplex_channels(3).await;
        for ch in pool.inner.channels.iter() {
            ch.mark_dead();
        }
        // Every member is dead -- `next()` returns a dead channel
        // without panicking or hanging. The test would hang
        // indefinitely if the picker blocked.
        for _ in 0..10 {
            let lease = pool.next();
            assert!(
                lease.channel().is_dead(),
                "fully-dead pool routes to dead channel as last resort"
            );
            drop(lease);
        }
        assert_eq!(pool.dead_count(), 3);
        assert!(pool.all_dead());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn single_member_pool_returns_dead_channel_even_if_marked_dead() {
        // The single-member fast path in `next()` skips the
        // load-balancing scan and returns the only channel
        // unconditionally. Pin that behaviour -- the alternative
        // (return None / block) would force every caller to handle
        // an Option even in the common case where the pool has one
        // live channel.
        let (pool, _server_tasks) = build_pool_with_duplex_channels(1).await;
        pool.inner.channels[0].mark_dead();
        let lease = pool.next();
        assert!(lease.channel().is_dead());
        assert_eq!(pool.len(), 1);
    }
}
