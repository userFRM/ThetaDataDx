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

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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

// ─── Submit + decode errors ────────────────────────────────────────

/// `grpc-status` style sentinel surfaced through `DecodeResult` when
/// the decoder pool is poisoned mid-flight. Carried inside an
/// [`Error::Transport`] so consumers that already handle
/// transport-level failures (channel reset, dropped reply oneshot)
/// see this on the same matchable arm without an enum change.
pub(crate) const POOL_POISONED_REASON: &str = "decoder pool poisoned by worker panic";

/// Back-off between [`disruptor::Producer::try_publish`] retries when
/// the ring is full. Picked short enough that the producer reacts to
/// a free slot within a single tokio yield window, long enough that
/// the retry loop is not a hot busy-wait. 50 µs is roughly two times
/// the inter-burst gap the consumer-side wait strategy is tuned for —
/// matching the cadence keeps the producer aligned with the consumer
/// without spinning a whole CPU.
///
/// The producer also calls [`thread::yield_now`] before sleeping so a
/// healthy consumer that just freed a slot can be picked up by the OS
/// scheduler immediately rather than after the timer fires.
const PUBLISH_RETRY_BACKOFF: Duration = Duration::from_micros(50);

/// Failures returned by [`DecoderHandle::submit`].
///
/// `submit` previously could not fail — it published into a bounded
/// ring and returned the receiver. After Finding 1 (panic
/// containment), submit refuses to publish when the pool has been
/// poisoned by a worker panic; the caller is told fast rather than
/// being parked on a `oneshot` whose decoder thread is gone.
#[derive(Debug, thiserror::Error)]
pub enum DecoderSubmitError {
    /// The pool was poisoned by a prior worker-thread panic. No
    /// further work will be processed; callers should fail their
    /// RPC and let the upstream client decide whether to rebuild
    /// the pool.
    #[error("{POOL_POISONED_REASON}")]
    Poisoned,
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
///
/// `poisoned` is shared with the consumer thread: on a worker-thread
/// panic the consumer flips the flag and falls through to a drain
/// loop that returns `Err(Error::Transport(POOL_POISONED_REASON))`
/// for every subsequent ring slot — both for requests that landed
/// before the poison flag was set (still-in-flight in-ring) and for
/// anything a racing producer published before observing the flag.
/// Submits made after the flag is observable fail fast with
/// [`DecoderSubmitError::Poisoned`] without ever touching the ring,
/// so the producer never busy-waits on a dead consumer.
///
/// Submitters that are *already mid-publish* on a saturated ring
/// (the consumer is slow, the producer is parked waiting for a
/// slot) also observe the flag promptly: the publish loop drives
/// [`Producer::try_publish`] rather than the blocking
/// [`Producer::publish`], re-checking the poison state between
/// every attempt. A poison flip therefore propagates to every
/// blocked submitter within one back-off window
/// ([`PUBLISH_RETRY_BACKOFF`]) — they bail out with
/// [`DecoderSubmitError::Poisoned`] and drop their unsent
/// `oneshot::Sender` so the caller's `await` is never parked on a
/// ring nobody will service.
#[derive(Clone)]
pub struct DecoderHandle {
    producer: MultiProducer<RingEvent, disruptor::SingleConsumerBarrier>,
    poisoned: Arc<AtomicBool>,
}

impl DecoderHandle {
    /// `true` once the consumer thread has caught a panic. Submits
    /// after this point fail with [`DecoderSubmitError::Poisoned`]
    /// rather than parking the caller on a dead ring.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Acquire)
    }

    /// Submit `response` for zstd decompress + `DataTable` decode.
    ///
    /// `max_message_size` is honoured at decode time so an
    /// adversarial `original_size` field cannot trigger a runaway
    /// allocation — the decoder thread rejects the response with
    /// `Error::Decompress { kind: MessageTooLarge, .. }` before any
    /// `Vec::resize` runs. Returns a `oneshot::Receiver` that resolves
    /// to the decoded `DataTable` (or the underlying decode error).
    ///
    /// Cancelling the receiver before the decoder reaches the slot
    /// causes the decoder to elide the decompress entirely — the
    /// captured `Bytes` is dropped and no CPU is spent on a result
    /// no one will read.
    ///
    /// # Errors
    ///
    /// Returns [`DecoderSubmitError::Poisoned`] when the pool has
    /// been poisoned by a prior worker-thread panic. The caller's
    /// RPC should fail rather than retry on the same pool. This is
    /// observed both pre-publish (fast-path check before any ring
    /// interaction) and mid-publish (the producer is parked on a
    /// full ring when a peer thread poisons): on a full-ring stall
    /// the producer polls [`Self::is_poisoned`] between every
    /// [`Producer::try_publish`] retry, so a poison flip propagates
    /// to every blocked submitter within one back-off window.
    pub(crate) fn submit(
        &self,
        response: proto::ResponseData,
        max_message_size: usize,
    ) -> Result<oneshot::Receiver<DecodeResult>, DecoderSubmitError> {
        let work: Box<dyn FnOnce() -> DecodeResult + Send + 'static> =
            Box::new(move || crate::mdds::decode::decode_data_table(&response, max_message_size));
        self.submit_work(work)
    }

    /// Publish a pre-boxed `work` closure onto the ring.
    ///
    /// Used by both the public [`Self::submit`] path (which boxes a
    /// `decode_data_table` invocation) and test fixtures that need
    /// to publish synthetic work (e.g. a deterministic-panic
    /// closure, or one that parks on a barrier so a ring-full race
    /// can be reproduced).
    ///
    /// The publish loop is poison-aware: rather than calling
    /// [`Producer::publish`] (which busy-waits inside the disruptor
    /// crate until a slot frees), it calls [`Producer::try_publish`]
    /// in a back-off loop that re-checks the poison flag between
    /// every attempt. When the consumer thread flips the flag while
    /// a producer is parked on a full ring, the producer observes
    /// the flip on its next iteration and returns
    /// [`DecoderSubmitError::Poisoned`] — never landing the request
    /// on a dead ring, never publishing the boxed work it had ready
    /// to submit. The work closure is dropped along with the unsent
    /// `oneshot::Sender`, releasing every resource the request
    /// captured.
    pub(crate) fn submit_work(
        &self,
        work: Box<dyn FnOnce() -> DecodeResult + Send + 'static>,
    ) -> Result<oneshot::Receiver<DecodeResult>, DecoderSubmitError> {
        // Fast-path poison check: refuse to publish into a ring
        // whose consumer thread has poisoned. The slow-path
        // re-check below covers the case where the flag flips
        // *during* a ring-full stall.
        if self.is_poisoned() {
            return Err(DecoderSubmitError::Poisoned);
        }
        // Detect the runtime flavor once per submit and pass the
        // resolved value into the retry loop. `Handle::try_current`
        // is thread-local but still ~10 ns per call; under sustained
        // ring saturation the loop would otherwise re-detect on
        // every iteration even though the result is invariant for
        // the lifetime of this call.
        let backoff_mode = BackoffMode::detect();
        let (tx, rx) = oneshot::channel();
        // Both the work closure and the reply sender ride together
        // in a single `Option` so the bounded retry loop can take
        // them back out on the publish-success path without
        // disturbing the move-into-FnOnce machinery used by the
        // failure branches. On every retry that hits a full ring
        // the loop re-checks the poison flag, dropping the request
        // (and its `oneshot::Sender`) without ever publishing if
        // the consumer has died meanwhile.
        let mut pending: Option<DecodeRequest> = Some(DecodeRequest { work, reply: tx });
        let mut producer = self.producer.clone();
        loop {
            // Re-check the poison flag on every iteration so a
            // mid-publish flip (consumer dies while we are parked
            // on a full ring) bails out promptly. The
            // `Ordering::Acquire` here pairs with the consumer's
            // `Ordering::Release` store on the panic path.
            if self.is_poisoned() {
                return Err(DecoderSubmitError::Poisoned);
            }
            // `try_publish` returns the unconsumed closure to us
            // via the `RingBufferFull` error. We don't want to
            // re-construct the request on every iteration (each
            // construction allocates a fresh oneshot), so the
            // closure passed to `try_publish` reaches into our
            // `Option<DecodeRequest>` and takes it only when the
            // sequence claim succeeded.
            let mut taken = pending.take();
            let outcome = producer.try_publish(|slot| {
                let request = taken
                    .take()
                    .expect("try_publish closure runs exactly once per accepted claim");
                // SAFETY: the disruptor producer barrier
                // guarantees the claimed sequence is exclusive to
                // this publish until we return from the closure —
                // no consumer can read it yet and no other
                // producer can claim the same sequence.
                unsafe { slot.write(request) };
            });
            match outcome {
                Ok(_seq) => return Ok(rx),
                Err(disruptor::RingBufferFull) => {
                    // The closure did not run — `taken` still holds
                    // the request. Restore it for the next attempt.
                    pending = taken;
                    // Brief back-off to avoid pinning the producer at
                    // 100% CPU when the ring stays full for several
                    // cycles. When called from a multi-thread tokio
                    // worker, wrap the wait in `block_in_place` so the
                    // runtime can migrate queued tasks to a sibling
                    // worker for the duration of the sleep — without
                    // it the calling task would stall its worker.
                    backoff_ring_full(backoff_mode, PUBLISH_RETRY_BACKOFF);
                }
            }
        }
    }
}

/// Resolved back-off strategy for [`backoff_ring_full`].
///
/// Detected once at [`DecoderHandle::submit_work`] entry rather than
/// on every retry: under sustained ring saturation the publish loop
/// would otherwise call [`tokio::runtime::Handle::try_current`] (and
/// the subsequent `runtime_flavor` query) on every iteration even
/// though the runtime flavor is invariant for the lifetime of the
/// `submit_work` call. Hoisting the detection collapses that cost to
/// one resolution per submit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackoffMode {
    /// Calling thread is on a multi-thread tokio runtime worker —
    /// the back-off sleep must be wrapped in
    /// [`tokio::task::block_in_place`] so the runtime can steal
    /// queued work onto a sibling worker rather than stalling the
    /// current worker.
    TokioMultiThread,
    /// Calling thread is on a current-thread tokio runtime or
    /// outside tokio entirely — a plain sync sleep is correct.
    /// `block_in_place` would panic on a current-thread runtime
    /// and is meaningless off-runtime.
    SyncSleep,
}

impl BackoffMode {
    /// Resolve once for the lifetime of a `submit_work` call. Thread-
    /// local on a tokio worker; the detection cost is small but the
    /// caller-side hoist still pays for itself under sustained ring
    /// saturation where the retry loop would otherwise re-detect on
    /// every iteration.
    fn detect() -> Self {
        use tokio::runtime::{Handle, RuntimeFlavor};
        match Handle::try_current().map(|h| h.runtime_flavor()) {
            Ok(RuntimeFlavor::MultiThread) => Self::TokioMultiThread,
            _ => Self::SyncSleep,
        }
    }
}

/// Brief back-off invoked when the Disruptor ring is full.
///
/// The caller is expected to have resolved [`BackoffMode`] *outside*
/// the retry loop and pass it in here, so a saturated ring does not
/// re-query [`tokio::runtime::Handle::try_current`] on every spin —
/// the resolution is invariant for the lifetime of one publish
/// attempt and the caller-side hoist keeps the cost to one
/// detection per submit rather than O(retries).
fn backoff_ring_full(mode: BackoffMode, duration: Duration) {
    let wait = || {
        thread::yield_now();
        thread::sleep(duration);
    };
    match mode {
        BackoffMode::TokioMultiThread => tokio::task::block_in_place(wait),
        BackoffMode::SyncSleep => wait(),
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
        let pool_poisoned = Arc::new(AtomicBool::new(false));

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
            //
            // The consumer thread's invariant: `work()` runs under
            // `catch_unwind` so a single bad decode (zstd corruption
            // tripping an assertion, prost panicking on a malformed
            // field) cannot kill the decoder. On caught panic the
            // pool-wide `pool_poisoned` flag flips, every future
            // ring slot drains with [`POOL_POISONED_REASON`], and
            // `DecoderHandle::submit` rejects new work fast.
            let poisoned = Arc::clone(&pool_poisoned);
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
                    let Some(DecodeRequest { work, reply }) = request else {
                        return;
                    };
                    if reply.is_closed() {
                        // Caller cancelled before we reached this
                        // slot. Skip the decompress entirely.
                        return;
                    }
                    // Fast-path: already-poisoned pool drains the
                    // request without running `work()` so the
                    // caller sees an immediate transport error
                    // instead of hanging on a never-completed
                    // oneshot.
                    if poisoned.load(Ordering::Acquire) {
                        let _ = reply.send(Err(Error::Transport {
                            kind: crate::error::TransportErrorKind::DecoderPoisoned,
                            message: POOL_POISONED_REASON.to_string(),
                        }));
                        return;
                    }
                    // Run the decode under catch_unwind so a panic
                    // (zstd assert, prost decode trap, allocator
                    // failure on a degenerate payload) flips the
                    // pool's poison flag instead of killing the
                    // consumer thread mid-loop. The Disruptor
                    // crate's consumer loop only exits when its
                    // shutdown signal arrives; catching the panic
                    // keeps it alive long enough to drain still-
                    // queued requests with `Err(PoolPoisoned)`.
                    //
                    // `AssertUnwindSafe` is sound here because
                    // `work` is a freshly-constructed FnOnce closure
                    // we own outright — there is no shared
                    // interior-mutable state to leave inconsistent
                    // on a partial unwind.
                    let outcome = std::panic::catch_unwind(AssertUnwindSafe(work));
                    let result = match outcome {
                        Ok(decoded) => decoded,
                        Err(_panic_payload) => {
                            // Poison the pool atomically and reply
                            // to the caller whose work triggered
                            // the panic with the transport-level
                            // failure. The drop of `panic_payload`
                            // releases its boxed `Any`; we do not
                            // re-raise — surviving panics is the
                            // whole point of this branch.
                            poisoned.store(true, Ordering::Release);
                            tracing::error!(
                                target: "thetadatadx::grpc::decoder_pool",
                                "mdds decoder worker panicked; pool poisoned"
                            );
                            Err(Error::Transport {
                                kind: crate::error::TransportErrorKind::DecoderPoisoned,
                                message: POOL_POISONED_REASON.to_string(),
                            })
                        }
                    };
                    // Send-failure is benign: receiver may have
                    // been dropped between the `is_closed` check
                    // and now (caller cancellation race).
                    let _ = reply.send(result);
                })
                .build();
            handles.push(DecoderHandle {
                producer: producer.clone(),
                poisoned: Arc::clone(&pool_poisoned),
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
        let rx = handle
            .submit(response, usize::MAX)
            .expect("submit succeeds");
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
            rxs.push(
                handle
                    .submit(make_response(8), usize::MAX)
                    .expect("submit succeeds"),
            );
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
        let rx = handle
            .submit(make_response(1), usize::MAX)
            .expect("submit succeeds");
        // Drop the receiver before the decoder reaches it. The
        // decoder elides the work; observable side effect is the
        // pool drains cleanly without panic.
        drop(rx);
        // Subsequent submission still succeeds.
        let rx = handle
            .submit(make_response(2), usize::MAX)
            .expect("submit succeeds");
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
        let rx = clone
            .handle(0)
            .submit(make_response(1), usize::MAX)
            .expect("submit succeeds");
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

    // ─── Finding 1: panic containment + poison drain ─────────────────

    /// Mint a `DecoderHandle` from a [`DecoderPool`] together with a
    /// custom work closure. The production `submit_work` path
    /// re-checks the poison flag between every `try_publish`
    /// attempt and refuses to publish on a poisoned pool — exactly
    /// the property the new test
    /// (`poison_flag_unblocks_publishers_on_full_ring`) verifies.
    ///
    /// The drain-side tests (`pending_in_flight_drains_*`) need to
    /// publish *before* the consumer races ahead and flips the
    /// poison flag, so they bypass the poison-aware path and call
    /// `try_publish` directly. The fixture intentionally does not
    /// observe the poison state — its only job is to land a request
    /// in the ring so the consumer-side drain branch can be
    /// exercised.
    fn submit_custom_work(
        handle: &DecoderHandle,
        work: Box<dyn FnOnce() -> DecodeResult + Send + 'static>,
    ) -> oneshot::Receiver<DecodeResult> {
        let (tx, rx) = oneshot::channel();
        let mut producer = handle.producer.clone();
        let mut pending = Some(DecodeRequest { work, reply: tx });
        loop {
            let mut taken = pending.take();
            let outcome = producer.try_publish(|slot| {
                let request = taken
                    .take()
                    .expect("try_publish closure runs exactly once per accepted claim");
                // SAFETY: the disruptor producer barrier guarantees
                // the claimed sequence is exclusive to this publish
                // until we return from the closure.
                unsafe { slot.write(request) };
            });
            match outcome {
                Ok(_seq) => return rx,
                Err(disruptor::RingBufferFull) => {
                    pending = taken;
                    thread::yield_now();
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn panicking_work_poisons_pool() {
        let pool = DecoderPool::new(1, 64).expect("pool");
        let handle = pool.handle(0).clone();
        // Publish a request whose work closure panics deterministically.
        let rx = submit_custom_work(
            &handle,
            Box::new(|| panic!("synthetic panic in decoder work closure")),
        );
        // The caller observes the transport-level poison reply rather
        // than a hang.
        let outcome = tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .expect("oneshot resolves before deadline")
            .expect("oneshot delivered");
        match outcome {
            Err(Error::Transport { message: msg, .. }) => {
                assert!(
                    msg.contains(POOL_POISONED_REASON),
                    "transport error must carry the pool-poisoned reason, got {msg:?}"
                );
            }
            other => panic!("expected Transport(POOL_POISONED_REASON) reply, got {other:?}"),
        }
        // Pool reports poisoned. Drop the pool to clean up worker threads.
        assert!(handle.is_poisoned(), "panic must flip the pool poison flag");
        drop(pool);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn submit_after_poison_fails_fast() {
        let pool = DecoderPool::new(1, 64).expect("pool");
        let handle = pool.handle(0).clone();
        // Panic the consumer first so the pool transitions to poisoned.
        let rx = submit_custom_work(
            &handle,
            Box::new(|| panic!("synthetic panic to poison pool")),
        );
        let _ = tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .expect("first reply lands")
            .expect("oneshot delivered");
        // Wait briefly until the poison flag is observable to the
        // producer; the consumer may set it on a different thread.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !handle.is_poisoned() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(handle.is_poisoned(), "pool poisoned after first panic");
        // Subsequent submits must refuse without parking on the ring.
        match handle.submit(make_response(1), usize::MAX) {
            Err(DecoderSubmitError::Poisoned) => { /* expected */ }
            other => panic!("expected DecoderSubmitError::Poisoned, got {other:?}"),
        }
        drop(pool);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pending_in_flight_drains_with_poisoned_after_panic() {
        // Race shape: a producer publishes several requests; the
        // consumer panics on the first one, then drains the rest of
        // the ring with `Err(Transport(POOL_POISONED_REASON))`. The
        // caller's oneshot resolves with the poison error rather
        // than hanging.
        //
        // The ring is FIFO so the panicking request is at the head;
        // the subsequent requests are queued behind it.
        let pool = DecoderPool::new(1, 64).expect("pool");
        let handle = pool.handle(0).clone();

        // First request panics.
        let rx_panic =
            submit_custom_work(&handle, Box::new(|| panic!("synthetic panic at ring head")));
        // Two follow-up requests with normal work that should never
        // run because the pool poisons first.
        let rx_follow_1 = submit_custom_work(
            &handle,
            Box::new(|| {
                Ok(proto::DataTable {
                    headers: vec!["x".into()],
                    data_table: Vec::new(),
                })
            }),
        );
        let rx_follow_2 = submit_custom_work(
            &handle,
            Box::new(|| {
                Ok(proto::DataTable {
                    headers: vec!["x".into()],
                    data_table: Vec::new(),
                })
            }),
        );

        // The panicking request returns the poison reason.
        let head_outcome = tokio::time::timeout(Duration::from_secs(2), rx_panic)
            .await
            .expect("head reply lands")
            .expect("oneshot delivered");
        match head_outcome {
            Err(Error::Transport { message: msg, .. }) => assert!(
                msg.contains(POOL_POISONED_REASON),
                "head reply carries poison reason, got {msg:?}"
            ),
            other => panic!("expected poisoned transport reply at head, got {other:?}"),
        }

        // Each queued follow-up also drains with the poison reply
        // rather than hanging on a dead ring.
        for (idx, rx) in [rx_follow_1, rx_follow_2].into_iter().enumerate() {
            let outcome = tokio::time::timeout(Duration::from_secs(2), rx)
                .await
                .unwrap_or_else(|_| panic!("queued reply {idx} resolves before deadline"))
                .expect("oneshot delivered");
            match outcome {
                Err(Error::Transport { message: msg, .. }) => assert!(
                    msg.contains(POOL_POISONED_REASON),
                    "queued reply {idx} carries poison reason, got {msg:?}"
                ),
                other => {
                    panic!("expected poisoned transport reply for queued {idx}, got {other:?}")
                }
            }
        }
        drop(pool);
    }

    /// Poison flag must interrupt submitters that are already parked
    /// in the publish retry loop on a full ring. The old
    /// `producer.publish()` call site busy-waited until a slot
    /// freed — a poison flip from a peer thread did not propagate
    /// to those parked publishers, so they would keep spinning
    /// until the consumer drained the ring. The current submit
    /// path re-checks the poison flag between every `try_publish`
    /// attempt, so all parked submitters return
    /// `Err(DecoderSubmitError::Poisoned)` within one back-off
    /// window once the flag flips.
    ///
    /// Test shape:
    ///   1. Build a tiny ring (capacity = 64, the minimum).
    ///   2. Publish a "barrier" work item that blocks the consumer
    ///      on a parking primitive — every subsequent work item
    ///      sits in the ring un-drained.
    ///   3. Fill the rest of the ring so the next submit must
    ///      block in `try_publish` retry.
    ///   4. Spawn `overflow_submitters` extra threads that all call
    ///      `submit_work`; they immediately observe a full ring and
    ///      park on the back-off loop.
    ///   5. From the main thread, flip the poison flag.
    ///   6. Assert every parked submitter returns
    ///      `DecoderSubmitError::Poisoned` within 250 ms — strictly
    ///      bounded, not "eventually".
    ///   7. Cleanup: release the barrier so the consumer thread can
    ///      drain the ring and the pool can shut down cleanly.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn poison_flag_unblocks_publishers_on_full_ring() {
        use std::sync::{Arc, Barrier};
        use std::time::Instant;

        const RING_SIZE: usize = 64;
        const OVERFLOW_SUBMITTERS: usize = 8;
        const POISON_PROPAGATION_BUDGET: Duration = Duration::from_millis(250);

        let pool = DecoderPool::new(1, RING_SIZE).expect("pool");
        let handle = pool.handle(0).clone();

        // Synchronisation primitive the head work-item parks on.
        // `Barrier::new(2)` rendezvous between the consumer thread
        // (running the head work) and the test driver which only
        // releases the barrier in the cleanup step.
        let consumer_barrier = Arc::new(Barrier::new(2));

        // Step 2: head work-item blocks the consumer thread.
        let head_barrier = Arc::clone(&consumer_barrier);
        let _head_rx = handle
            .submit_work(Box::new(move || {
                // Park the consumer thread until the test driver
                // releases the barrier. Once released, return a
                // synthetic OK so the consumer can advance.
                head_barrier.wait();
                Ok(proto::DataTable {
                    headers: vec!["x".into()],
                    data_table: Vec::new(),
                })
            }))
            .expect("head publish before poison");

        // Wait for the consumer to actually pick up the head item.
        // Without this, steps 3–4 might fill the ring with the head
        // item still unconsumed, in which case the consumer never
        // parks on the barrier and the test cannot make the ring
        // "full from the producer's POV with a stuck consumer".
        // A short sleep is enough: the disruptor consumer wakes
        // within tens of microseconds of the publish.
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Step 3: fill the ring with work items that will never be
        // reached (consumer is stuck on the barrier). The ring has
        // `RING_SIZE` slots; the head item was already taken so we
        // need RING_SIZE - 1 more to fill to capacity. Each holds a
        // benign `Ok` work body that never runs.
        let mut filler_rxs = Vec::with_capacity(RING_SIZE - 1);
        for _ in 0..(RING_SIZE - 1) {
            filler_rxs.push(
                handle
                    .submit_work(Box::new(move || {
                        Ok(proto::DataTable {
                            headers: vec!["x".into()],
                            data_table: Vec::new(),
                        })
                    }))
                    .expect("filler publish before ring saturates"),
            );
        }

        // Step 4: spawn overflow submitters. Each will park in the
        // publish retry loop because the ring is now saturated. Each
        // thread records the instant `submit_work` returns so the
        // main thread can measure the *post-poison* reaction window
        // (finish - poison_at) instead of the wall-clock distance
        // from thread spawn — which would otherwise include the 50ms
        // settle sleep below and inflate the measurement.
        let mut overflow_handles = Vec::with_capacity(OVERFLOW_SUBMITTERS);
        for _ in 0..OVERFLOW_SUBMITTERS {
            let producer_handle = handle.clone();
            overflow_handles.push(thread::spawn(move || {
                let outcome = producer_handle.submit_work(Box::new(move || {
                    Ok(proto::DataTable {
                        headers: vec!["x".into()],
                        data_table: Vec::new(),
                    })
                }));
                let finished_at = Instant::now();
                (finished_at, outcome)
            }));
        }

        // Give the overflow submitters a moment to enter the retry
        // loop. Without this, the poison flip below could race the
        // submitter's fast-path check and return `Poisoned` before
        // the submitter ever reached the retry loop — defeating
        // the test's purpose.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Step 5: flip the poison flag. This simulates a consumer-
        // side panic without actually killing the consumer thread
        // (which is parked on the barrier).
        let poison_at = Instant::now();
        handle.poisoned.store(true, Ordering::Release);

        // Step 6: every overflow submitter returns Poisoned within
        // the budget *measured from the poison flip*. The budget is
        // strict — a regression that restored busy-wait `publish()`
        // would block here forever (consumer never advances, ring
        // never frees, no Poisoned).
        for (idx, join) in overflow_handles.into_iter().enumerate() {
            let (finished_at, outcome) = join.join().expect("overflow submitter joins");
            match outcome {
                Err(DecoderSubmitError::Poisoned) => { /* expected */ }
                other => panic!("overflow submitter {idx} did not observe poison: {other:?}"),
            }
            // `saturating_duration_since` returns 0 if the submitter
            // somehow finished before the poison flip — impossible
            // on the happy path (poison flip is the only event that
            // unblocks the parked retry loop) but defensive against
            // future restructuring.
            let reaction = finished_at.saturating_duration_since(poison_at);
            assert!(
                reaction < POISON_PROPAGATION_BUDGET,
                "overflow submitter {idx} took {reaction:?} to observe poison \
                 after the flag flipped (budget {POISON_PROPAGATION_BUDGET:?})"
            );
        }

        // Step 7: cleanup. Release the consumer barrier so the
        // consumer thread advances past the head item and the
        // disruptor's drop-time join can complete. The remaining
        // ring slots drain with the poison reply (the consumer's
        // drain branch sees `poisoned == true` and short-circuits
        // every slot).
        consumer_barrier.wait();
        // Drop the receivers without awaiting; we only care that
        // the ring drains. Awaiting them would couple this test to
        // the ring drain order which is incidental.
        drop(filler_rxs);
        drop(pool);
    }
}
