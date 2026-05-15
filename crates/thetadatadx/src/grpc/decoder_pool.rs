//! Dedicated decoder pool for MDDS server-streaming responses.
//!
//! # Architecture
//!
//! ```text
//!   +-------------------+   LMAX ring   +-------------------+
//!   | h2 receive task   |-------------->| Decoder thread    |
//!   | (tokio worker)    |  work+reply   | (std::thread,     |
//!   +-------------------+               |  TLS zstd ctx +   |
//!                                       |  scratch buffer)  |
//!                                       +---------+---------+
//!                                                 |
//!                                                 | DataTable
//!                                                 v
//!                                       +-------------------+
//!                                       | oneshot::Sender   |
//!                                       | -> async caller   |
//!                                       +-------------------+
//! ```
//!
//! Decompresses zstd payloads and decodes the inner protobuf message
//! on dedicated `std::thread`s, keeping CPU-bound work off the tokio
//! reactor. Each decoder owns a thread-local zstd context and a
//! reusable scratch buffer (see [`crate::mdds::decode::transport`]);
//! communication with the tokio IO side runs through one LMAX
//! Disruptor ring per decoder (lock-free, pre-allocated slots).
//!
//! # Why off-reactor
//!
//! Bloomberg / LSEG / Refinitiv feed handlers all separate IO from
//! decode for the same reason: a single multi-millisecond decode
//! call (1 MB zstd-compressed `DataTable` payload, ~5–50 ms wall
//! time) blocks the tokio worker thread it lands on, stalling every
//! other RPC that worker is multiplexing. Moving the decode to
//! dedicated threads lets the tokio reactor keep draining h2 DATA
//! frames while N CPU cores chew through the backlog in parallel.
//!
//! # Wait strategy
//!
//! MDDS decode cadence is bursty — 64 concurrent RPCs each yielding
//! one or many chunks, separated by tens to hundreds of microseconds
//! of network IO. Pure spin burns whole cores during idle gaps so
//! the strategy ([`DecoderWaitStrategy`]) is tuned shorter than the
//! FPSS analogue: a few spin iterations, a brief yield window, then
//! a `spin_loop` hint. This trades ~50 ns of wake-up latency for
//! near-zero idle CPU between bursts.

use std::sync::Arc;
use std::thread;

use disruptor::{build_multi_producer, MultiProducer, ProcessorSettings, Producer, Sequence};
use tokio::sync::oneshot;

use crate::error::Error;
use crate::proto;
use crate::util::ring::{check_ring_size, RingSizeError};

// ─── Wait strategy ──────────────────────────────────────────────────

/// Adaptive wait strategy tuned for the MDDS decode cadence.
///
/// Decode bursts arrive concurrently from a 64-way pool, separated
/// by tens to hundreds of microseconds of network IO. A pure spin
/// would keep N cores at 100% during the idle window between
/// bursts; this strategy spins for `spin_iters` cycles, yields for
/// `yield_iters` iterations, then falls back to a `spin_loop` hint
/// that lets the OS multiplex other work onto the core.
///
/// Tuning targets:
/// - `spin_iters = 16`: ~50 ns at ~3 ns/cycle. Covers the
///   submit-to-dequeue handoff in the common case where a decoder
///   is woken from a recent prior submission.
/// - `yield_iters = 4`: ~4–40 µs depending on OS scheduler. Covers
///   inter-burst gaps without holding the core hostile to other
///   tokio workers on the same CPU.
/// - Final `spin_loop` hint: stays cooperatively idle indefinitely
///   while still reading the publisher sequence.
#[derive(Copy, Clone)]
pub struct DecoderWaitStrategy {
    spin_iters: u32,
    yield_iters: u32,
}

impl DecoderWaitStrategy {
    /// Build a wait strategy with custom iteration counts. Test-only
    /// hook for exercising boundary behaviour; the production
    /// constructor is [`Self::mdds_default`].
    #[must_use]
    pub fn new(spin_iters: u32, yield_iters: u32) -> Self {
        Self {
            spin_iters,
            yield_iters,
        }
    }

    /// Production-tuned defaults: 16 spins, 4 yields, then hint.
    #[must_use]
    pub fn mdds_default() -> Self {
        Self::new(16, 4)
    }
}

impl disruptor::wait_strategies::WaitStrategy for DecoderWaitStrategy {
    #[inline]
    fn wait_for(&self, _sequence: Sequence) {
        for _ in 0..self.spin_iters {
            std::hint::spin_loop();
        }
        for _ in 0..self.yield_iters {
            thread::yield_now();
        }
        std::hint::spin_loop();
    }
}

// ─── Pool errors ────────────────────────────────────────────────────

/// Failures rejected at [`DecoderPool::new`] time.
#[derive(Debug, thiserror::Error)]
pub enum DecoderPoolError {
    /// `n_decoders` was `0`. A pool with no decoder threads cannot
    /// service any decode request.
    #[error("decoder pool must have at least one decoder thread")]
    EmptyPool,
    /// `ring_size_per_decoder` failed [`check_ring_size`].
    #[error("decoder ring size invalid: {0}")]
    InvalidRingSize(#[from] RingSizeError),
}

impl From<DecoderPoolError> for Error {
    fn from(err: DecoderPoolError) -> Self {
        Self::config_invalid("mdds_decoder_pool", err.to_string())
    }
}

// ─── Request shape ──────────────────────────────────────────────────

/// Outcome of a pool decode: either a fully decoded `DataTable` or
/// an [`Error`] explaining the failure (zstd, prost, or upstream IO).
pub type DecodeResult = Result<proto::DataTable, Error>;

/// One unit of decode work submitted to the pool.
///
/// The work closure runs entirely on the decoder thread; it owns
/// every cycle of the zstd decompress + protobuf decode chain and
/// reads only its captured `compressed`/`max_message_size`
/// parameters. The `reply` channel signals completion back to the
/// async caller — if the receiver was dropped (caller cancelled)
/// the decoder side checks [`oneshot::Sender::is_closed`] before
/// running the work and elides the decompress entirely so cancelled
/// RPCs do not waste CPU on results no one will read.
struct DecodeRequest {
    work: Box<dyn FnOnce() -> DecodeResult + Send + 'static>,
    reply: oneshot::Sender<DecodeResult>,
}

/// One slot in the per-decoder ring buffer.
///
/// The disruptor crate hands the consumer a `&RingEvent` (not
/// `&mut`) so the request must live behind interior mutability.
/// `UnsafeCell` is the right shape here: the Disruptor's sequence
/// barrier already enforces exclusive access — the producer holds
/// the slot until it advances the sequence, then the consumer holds
/// it until it advances the consumer barrier. We just need to
/// surface that exclusivity to Rust's type system.
///
/// The `Option` lets the consumer take ownership of the request out
/// of the slot; the next publish overwrites with a fresh `Some`.
struct RingEvent {
    request: std::cell::UnsafeCell<Option<DecodeRequest>>,
}

impl Default for RingEvent {
    fn default() -> Self {
        Self {
            request: std::cell::UnsafeCell::new(None),
        }
    }
}

impl RingEvent {
    /// Write a request into the slot. Caller (the producer closure)
    /// has exclusive access by virtue of the disruptor's producer
    /// barrier — no other thread can read or write this slot until
    /// the producer publishes the sequence.
    ///
    /// # Safety
    ///
    /// Caller must be the producer holding exclusive access to this
    /// slot's sequence number.
    unsafe fn write(&self, request: DecodeRequest) {
        // SAFETY: see method docstring — producer barrier guarantees
        // exclusivity at this sequence position.
        unsafe { *self.request.get() = Some(request) };
    }

    /// Take the request out of the slot. Caller (the consumer
    /// closure) has exclusive access by virtue of the disruptor's
    /// consumer barrier — the producer cannot reuse this slot until
    /// the consumer advances its sequence.
    ///
    /// # Safety
    ///
    /// Caller must be the consumer holding exclusive access to this
    /// slot's sequence number.
    unsafe fn take(&self) -> Option<DecodeRequest> {
        // SAFETY: see method docstring — consumer barrier guarantees
        // exclusivity at this sequence position.
        unsafe { (*self.request.get()).take() }
    }
}

// SAFETY: `DecodeRequest` carries a `Send + 'static` boxed closure
// and a `oneshot::Sender` (Send). The ring slot is accessed only
// through the Disruptor's sequencing guarantees (exclusive write on
// publish, exclusive read on consume), and `RingEvent::write` /
// `RingEvent::take` document that invariant on their `unsafe`
// signatures.
unsafe impl Sync for RingEvent {}

// ─── Decoder handle ─────────────────────────────────────────────────

/// Clone-cheap handle to one decoder ring.
///
/// Held by every `Channel`; cloning shares the underlying
/// [`MultiProducer`] so multiple tokio tasks can publish on the same
/// decoder ring concurrently without holding a lock. The
/// `MultiProducer` itself is internally `Arc<Mutex<...>>`-backed by
/// the Disruptor crate; cloning bumps a reference count.
#[derive(Clone)]
pub struct DecoderHandle {
    producer: MultiProducer<RingEvent, disruptor::SingleConsumerBarrier>,
}

impl DecoderHandle {
    /// Submit `compressed` for zstd decompress + `DataTable` decode.
    ///
    /// `max_message_size` is honoured at decode time so an
    /// adversarial `original_size` field cannot trigger a runaway
    /// allocation. Returns a `oneshot::Receiver` that resolves to
    /// the decoded `DataTable` (or the underlying decode error).
    ///
    /// Cancelling the receiver before the decoder reaches the slot
    /// causes the decoder to elide the decompress entirely — the
    /// captured `Bytes` is dropped and no CPU is spent on a result
    /// no one will read.
    pub(crate) fn submit(&self, response: proto::ResponseData) -> oneshot::Receiver<DecodeResult> {
        let (tx, rx) = oneshot::channel();
        let work: Box<dyn FnOnce() -> DecodeResult + Send + 'static> =
            Box::new(move || crate::mdds::decode::decode_data_table(&response));
        // `publish` busy-waits when the ring is full; the
        // multi-producer barrier serialises the sequence claim so
        // concurrent submissions from different tokio tasks stay
        // FIFO with respect to each other.
        let mut producer = self.producer.clone();
        producer.publish(|slot| {
            // SAFETY: the disruptor producer barrier guarantees the
            // claimed sequence is exclusive to this publish until we
            // return from the closure — no consumer can read it yet
            // and no other producer can claim the same sequence.
            unsafe { slot.write(DecodeRequest { work, reply: tx }) };
        });
        rx
    }
}

// ─── Pool ───────────────────────────────────────────────────────────

/// Dedicated decoder pool. Holds one Disruptor ring (and one consumer
/// thread) per decoder configured at construction time. Cloneable —
/// cloning shares the same pool across `Channel`s and is the standard
/// way to attach a pool to a `ChannelPool`.
#[derive(Clone)]
pub struct DecoderPool {
    /// One `DecoderHandle` per decoder thread. Distributed to
    /// `Channel`s round-robin so each channel pins to a single
    /// decoder, but `MultiProducer`'s `Clone` makes concurrent
    /// submission from the same `Channel` from many tasks lock-free
    /// at the application level (the disruptor crate's internal
    /// `Mutex` serialises sequence claims but is uncontended in the
    /// steady state).
    handles: Arc<[DecoderHandle]>,
    /// `Arc` over the inner so dropping the last clone joins the
    /// decoder threads cleanly. The `_inner` field is intentionally
    /// dead-after-construction — its only job is to keep the
    /// producer barriers and join handles alive until the last pool
    /// clone is dropped.
    _inner: Arc<PoolInner>,
}

/// Owns the producer barriers and decoder-thread join handles. Drop
/// order: producers drop first (signals shutdown to each ring), then
/// thread handles join. Keeping both behind one `Arc` ensures every
/// `DecoderPool` clone keeps the workers alive for its lifetime.
struct PoolInner {
    /// Held only to drive the drop-order — producers must outlive
    /// the consumers they feed. Each entry is the original
    /// `MultiProducer` we built; the per-`Channel` handles hold
    /// clones of these, so all of them go to zero only when the
    /// last `DecoderHandle` is also dropped.
    _producers: Vec<MultiProducer<RingEvent, disruptor::SingleConsumerBarrier>>,
}

impl DecoderPool {
    /// Build a pool with `n_decoders` dedicated threads, each owning
    /// a ring of `ring_size_per_decoder` slots.
    ///
    /// `n_decoders = 0` returns [`DecoderPoolError::EmptyPool`].
    /// `ring_size_per_decoder` must satisfy [`check_ring_size`]
    /// (power of two, `>= 64`).
    ///
    /// # Errors
    ///
    /// Returns [`DecoderPoolError`] when `n_decoders` is zero or the
    /// ring size fails validation.
    pub fn new(n_decoders: usize, ring_size_per_decoder: usize) -> Result<Self, DecoderPoolError> {
        if n_decoders == 0 {
            return Err(DecoderPoolError::EmptyPool);
        }
        let ring_size = check_ring_size(ring_size_per_decoder)?;

        let wait_strategy = DecoderWaitStrategy::mdds_default();
        let mut handles = Vec::with_capacity(n_decoders);
        let mut producers = Vec::with_capacity(n_decoders);

        for _idx in 0..n_decoders {
            // Each decoder thread runs the consumer side of its own
            // ring; the closure passed to `handle_events_with`
            // executes inline on the consumer thread the disruptor
            // crate spawns for us. The fixed `mdds-decoder` thread
            // name lets `top -H` / pprof / perf surface them as a
            // single group; the disruptor crate's `thread_name`
            // requires a `&'static str` so per-decoder numbering
            // would force a `Box::leak`, not worth the leak budget
            // when the group identity is the load-bearing signal.
            let producer = build_multi_producer(ring_size, RingEvent::default, wait_strategy)
                .thread_name("mdds-decoder")
                .handle_events_with(move |slot: &RingEvent, _seq: Sequence, _eob: bool| {
                    // SAFETY: the disruptor consumer barrier
                    // guarantees this thread holds exclusive access
                    // to the slot at this sequence position until
                    // the consumer barrier advances on closure
                    // return. No producer can reuse the slot, and
                    // no other consumer exists.
                    let request = unsafe { slot.take() };
                    if let Some(DecodeRequest { work, reply }) = request {
                        if reply.is_closed() {
                            // Caller cancelled before we reached
                            // this slot. Skip the decompress
                            // entirely — running it would burn CPU
                            // on a result no one will read.
                            return;
                        }
                        let result = work();
                        // Send-failure is benign: only happens if
                        // the receiver was dropped between the
                        // is_closed check and now (race with caller
                        // cancellation). The decoded DataTable is
                        // dropped along with the channel.
                        let _ = reply.send(result);
                    }
                })
                .build();
            handles.push(DecoderHandle {
                producer: producer.clone(),
            });
            producers.push(producer);
        }

        Ok(Self {
            handles: handles.into(),
            _inner: Arc::new(PoolInner {
                _producers: producers,
            }),
        })
    }

    /// Number of decoder threads in this pool.
    #[must_use]
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    /// Always `false` — `new` rejects empty pools at construction.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Borrow the `idx`-th decoder handle. Used by `ChannelPool` to
    /// fan channels out across decoders.
    #[must_use]
    pub fn handle(&self, idx: usize) -> &DecoderHandle {
        &self.handles[idx % self.handles.len()]
    }
}

/// Default decoder thread count. Uses
/// [`std::thread::available_parallelism`] divided by two so the pool
/// leaves headroom for the tokio reactor and the application's own
/// CPU work. Falls back to `2` when `available_parallelism` fails
/// (containers without `/proc/cpuinfo`, etc.). Capped to `channels`
/// because more decoders than concurrent channels is pure overhead.
#[must_use]
pub fn default_decoder_thread_count(channels: usize) -> usize {
    let logical = thread::available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(2);
    let half = (logical / 2).max(1);
    half.min(channels.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{CompressionAlgo, CompressionDescription, DataTable, ResponseData};
    use prost::Message;
    use std::io::Write;
    use std::time::Duration;

    /// Build a `ResponseData` carrying a zstd-compressed `DataTable`
    /// with a single row. Matches the production wire shape so the
    /// pool tests exercise the same decode path consumers will.
    fn make_response(rows: usize) -> ResponseData {
        use crate::proto::{data_value, DataValue, DataValueList};
        let row_template = DataValueList {
            values: vec![DataValue {
                data_type: Some(data_value::DataType::Number(42)),
            }],
        };
        let table = DataTable {
            headers: vec!["x".to_string()],
            data_table: (0..rows).map(|_| row_template.clone()).collect(),
        };
        let inner = table.encode_to_vec();
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).expect("zstd encoder");
        encoder.write_all(&inner).expect("zstd write");
        let compressed = encoder.finish().expect("zstd finalize");
        ResponseData {
            compressed_data: compressed,
            compression_description: Some(CompressionDescription {
                algo: i32::from(CompressionAlgo::Zstd),
                ..CompressionDescription::default()
            }),
            original_size: i32::try_from(inner.len()).unwrap_or(0),
        }
    }

    #[test]
    fn rejects_empty_pool() {
        let result = DecoderPool::new(0, 64);
        assert!(matches!(
            result.as_ref().err(),
            Some(DecoderPoolError::EmptyPool)
        ));
        // Drop the Ok branch's pool defensively — `assert!` only
        // inspects the error and the empty-pool branch is the only
        // one this test should ever see.
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_ring_size() {
        let result = DecoderPool::new(1, 65);
        assert!(matches!(
            result.as_ref().err(),
            Some(DecoderPoolError::InvalidRingSize(_))
        ));
        assert!(result.is_err());
    }

    #[test]
    fn default_decoder_count_caps_to_channels() {
        // Logical cores >> 4: capped to channel count.
        assert!(default_decoder_thread_count(4) <= 4);
        // Pathological channel = 0: lower-bound to 1.
        assert!(default_decoder_thread_count(0) >= 1);
    }

    #[tokio::test]
    async fn decodes_single_response() {
        let pool = DecoderPool::new(1, 64).expect("pool");
        let handle = pool.handle(0).clone();
        let response = make_response(3);
        let rx = handle.submit(response);
        let table = rx
            .await
            .expect("oneshot delivered")
            .expect("decode succeeds");
        assert_eq!(table.data_table.len(), 3);
        assert_eq!(table.headers, vec!["x".to_string()]);
        drop(pool);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn decodes_concurrent_responses() {
        let pool = DecoderPool::new(2, 128).expect("pool");
        let handle = pool.handle(0).clone();
        let mut rxs = Vec::with_capacity(16);
        for _ in 0..16 {
            rxs.push(handle.submit(make_response(8)));
        }
        for rx in rxs {
            let table = rx
                .await
                .expect("oneshot delivered")
                .expect("decode succeeds");
            assert_eq!(table.data_table.len(), 8);
        }
        drop(pool);
    }

    #[tokio::test]
    async fn cancelled_request_is_skipped() {
        let pool = DecoderPool::new(1, 64).expect("pool");
        let handle = pool.handle(0).clone();
        let rx = handle.submit(make_response(1));
        // Drop the receiver before the decoder reaches it. The
        // decoder elides the work; observable side effect is the
        // pool drains cleanly without panic.
        drop(rx);
        // Subsequent submission still succeeds.
        let rx = handle.submit(make_response(2));
        let table = tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .expect("decode within deadline")
            .expect("oneshot delivered")
            .expect("decode succeeds");
        assert_eq!(table.data_table.len(), 2);
        drop(pool);
    }

    #[tokio::test]
    async fn pool_clone_shares_decoders() {
        let pool = DecoderPool::new(1, 64).expect("pool");
        let clone = pool.clone();
        assert_eq!(pool.len(), clone.len());
        let rx = clone.handle(0).submit(make_response(1));
        let table = rx
            .await
            .expect("oneshot delivered")
            .expect("decode succeeds");
        assert_eq!(table.data_table.len(), 1);
        drop(pool);
        drop(clone);
    }

    #[test]
    fn decoder_wait_strategy_is_copy_send() {
        fn assert_copy_send<T: Copy + Send>() {}
        assert_copy_send::<DecoderWaitStrategy>();
    }
}
