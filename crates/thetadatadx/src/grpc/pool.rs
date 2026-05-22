//! Round-robin pool of [`Channel`] handles.
//!
//! A single h2 connection multiplexes many concurrent streams, but
//! `MAX_CONCURRENT_STREAMS` is finite (the default upstream cap is
//! `~100`). [`ChannelPool`] keeps `N` parallel channels and hands them
//! out via [`ChannelPool::next`], picking the channel with the fewest
//! in-flight streams so a workload that exceeds the per-connection
//! limit fans out across distinct h2 connections rather than blocking
//! on stream availability.
//!
//! The pool is `Arc`-clone-cheap and `Send + Sync`; callers can clone
//! it freely across tasks. Each [`ChannelPool::next`] returns a
//! reference into the pool — the underlying [`Channel`] is owned by
//! the pool as an `Arc<Channel>` and lives as long as the pool does.
//!
//! # Reconnect, in place
//!
//! When an RPC dispatched through a pool channel observes
//! [`super::ChannelError::ConnectionClosed`], the channel itself
//! triggers a single-flight in-place reconnect (see
//! [`super::Channel::trigger_reconnect`]). The pool slot does NOT
//! get marked dead, replaced, or skipped — the same `Arc<Channel>`
//! handle the picker returned remains valid; only the inner
//! `SendRequest<Bytes>` swaps to a fresh h2 session in the background.
//! The next RPC dispatched through the channel picks up the new
//! sender transparently.

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
    channels: Vec<Arc<Channel>>,
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
        let channels: Vec<Arc<Channel>> = channels
            .into_iter()
            .map(|c| {
                let arc = Arc::new(c);
                // Install the weak self-reference so the channel's
                // reconnect path can upgrade to an `Arc<Channel>` and
                // spawn a `'static` reconnect future.
                arc.install_self_weak(Arc::downgrade(&arc));
                arc
            })
            .collect();
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
        let channels: Vec<Arc<Channel>> = channels
            .into_iter()
            .enumerate()
            .map(|(idx, ch)| {
                let arc = Arc::new(ch.with_decoder(decoder_pool.handle(idx).clone()));
                arc.install_self_weak(Arc::downgrade(&arc));
                arc
            })
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

    /// `true` if the pool holds no channels. The constructor's
    /// `assert!` rules this out in practice; the method exists so
    /// callers using the `len() + is_empty()` clippy-friendly idiom
    /// don't have to special-case the pool.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.channels.is_empty()
    }

    /// Look up a pool member by index. Hidden from the public docs —
    /// exposed so integration tests can deterministically target a
    /// specific channel (e.g. fire a slow RPC against pool member 0
    /// to saturate it, then assert subsequent `next()` calls route
    /// around it).
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
/// open path returns a `ServerStreaming`, the stream's own
/// `InFlightToken` keeps the channel marked busy for the rest of
/// the RPC lifetime, so dropping the lease after that point is the
/// correct shape.
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
    #[must_use]
    pub fn channel(&self) -> &Arc<Channel> {
        self.channel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Round-robin distribution semantics are covered end-to-end by
    // the `channel_pool_concurrent_dispatch_spreads_across_members`
    // test in `tests/grpc_mock_server.rs`, which drives the real
    // `ChannelPool::next` against a 4-channel pool wired to mock h2
    // listeners. The standalone unit tests previously here drove an
    // `IndexedPool` shim that re-implemented the modulo cursor —
    // asserting that two copies of the same algorithm agree.

    #[test]
    #[should_panic(expected = "ChannelPool must hold at least one Channel")]
    fn empty_pool_panics() {
        let _ = ChannelPool::from_channels(Vec::new());
    }
}
