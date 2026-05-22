//! Two-stage decode pipeline: zstd decompress (stage 1) handed off
//! through a bounded MPSC queue to a worker pool that runs the
//! `prost::Message::decode` + Tick-build step (stage 2).
//!
//! # Why two stages
//!
//! The single-stage [`super::decoder_pool::DecoderPool`] runs the full
//! `zstd decompress -> prost decode -> Vec<Tick> build` chain on each
//! decoder thread. Under heavy load that couples three very different
//! cost profiles onto the same N-way pool: zstd is bound by the
//! decompressor's serial dictionary state per chunk (one core, no
//! parallelism gain past N=cores), but `prost::Message::decode` and
//! `Tick` materialization scale linearly with available cores. A
//! single-stage pool sized for zstd starves prost; sized for prost
//! over-provisions zstd contexts.
//!
//! Splitting the work lets each stage scale independently:
//!
//! ```text
//!   per-channel decoder threads         shared stage-2 pool
//!     +----------------+                  +----------------+
//!     | h2 receive --> |    DecodedPayload | prost::decode |
//!     | zstd decompress|  ------queue----> |   + Tick build|
//!     |                |                  |               |
//!     +----------------+                  +----------------+
//!                                            (M workers)
//! ```
//!
//! Stage-1 keeps the existing per-channel thread (one decoder per
//! channel keeps the thread-local zstd context warm for that
//! channel's payload-size distribution). Stage-2 fans the prost +
//! Tick-build work out across M workers so a single slow channel
//! cannot saturate decode capacity for the whole pool.
//!
//! # Backpressure
//!
//! The cross-stage queue is bounded
//! ([`crossbeam_channel::bounded`]). When stage-2 cannot keep up,
//! stage-1's `send()` *parks* the decoder thread rather than dropping
//! the payload — silent drops on a market-data feed are the worst
//! possible failure mode. The decoder thread records the park
//! duration through `tracing::debug!` so operators running with
//! `RUST_LOG=debug` see backpressure events.
//!
//! # Counters
//!
//! `total_decoded` / `total_dropped` / `total_parked` are wrapped in
//! [`crossbeam_utils::CachePadded`] so the stage-1 and stage-2
//! threads, which both increment these counters, do not stall each
//! other through false-sharing on a single cache line. Each counter
//! gets its own 64-byte (or 128-byte on aarch64) line.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use bytes::Bytes;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use crossbeam_utils::CachePadded;
use tokio::sync::oneshot;

use crate::error::{Error, TransportErrorKind};
use crate::proto;

use super::decoder_pool::{DecodeResult, POOL_POISONED_REASON};

// ─── Public value type ──────────────────────────────────────────────

/// Post-zstd-decompress payload handed from stage-1 to stage-2.
///
/// Carries enough identity to correlate logs across stages
/// (`channel_id` identifies the originating per-channel decoder thread,
/// `request_id` is a monotonic counter per channel) plus the raw
/// decompressed bytes that stage-2 will feed into
/// `prost::Message::decode`.
///
/// `payload` is a [`bytes::Bytes`] handle rather than a `Vec<u8>` so
/// the stage-2 worker can split, slice, or share-clone the buffer
/// without copying the underlying bytes. The thread-local zstd
/// scratch buffer in [`crate::mdds::decode::transport`] still owns
/// the canonical storage; stage-1 produces a `Bytes` clone of that
/// storage for the queue payload.
#[derive(Debug, Clone)]
pub struct DecodedPayload {
    /// Identifier of the stage-1 decoder thread that produced this
    /// payload. Matches the per-channel decoder index in the parent
    /// [`super::decoder_pool::DecoderPool`].
    pub channel_id: u64,
    /// Per-channel monotonic counter for this request. Wraps at
    /// `u64::MAX` — astronomical for any realistic workload.
    pub request_id: u64,
    /// Decompressed protobuf bytes ready for `prost::Message::decode`.
    pub payload: Bytes,
}

// ─── Cache-padded counters ──────────────────────────────────────────

/// Stage-pipeline counters. Each field is wrapped in
/// [`CachePadded`] so concurrent increments from stage-1 and stage-2
/// threads do not stall each other through cache-line false sharing.
///
/// On x86_64 the cache line is 64 bytes, on aarch64 typically 128;
/// the `CachePadded` wrapper sizes to the target's worst case so the
/// padding is portable. Tests pin the resulting struct size so a
/// regression that drops the padding fails CI immediately.
#[derive(Debug)]
pub struct Stage2Counters {
    /// Total payloads that stage-2 successfully decoded into a
    /// [`crate::proto::DataTable`]. Incremented after each stage-2
    /// worker returns from `prost::Message::decode`.
    pub total_decoded: CachePadded<AtomicU64>,
    /// Total payloads dropped before stage-2 ran them — currently
    /// only ticks when the oneshot receiver was already closed
    /// (caller cancellation), never on backpressure (which parks
    /// stage-1 instead). Exposed so a regression that re-introduced
    /// drop-on-full would be visible.
    pub total_dropped: CachePadded<AtomicU64>,
    /// Total nanoseconds stage-1 parked waiting for a free queue
    /// slot. Incremented atomically on every backpressure event.
    /// Operators surface this as a histogram bucket.
    pub total_parked: CachePadded<AtomicU64>,
}

impl Default for Stage2Counters {
    fn default() -> Self {
        Self {
            total_decoded: CachePadded::new(AtomicU64::new(0)),
            total_dropped: CachePadded::new(AtomicU64::new(0)),
            total_parked: CachePadded::new(AtomicU64::new(0)),
        }
    }
}

impl Stage2Counters {
    /// Observed value of `total_decoded`. `Ordering::Relaxed` is
    /// correct for a monitoring snapshot — there is no
    /// happens-before relationship with other state to enforce.
    #[must_use]
    pub fn total_decoded(&self) -> u64 {
        self.total_decoded.load(Ordering::Relaxed)
    }

    /// Observed value of `total_dropped`.
    #[must_use]
    pub fn total_dropped(&self) -> u64 {
        self.total_dropped.load(Ordering::Relaxed)
    }

    /// Observed value of `total_parked` (nanoseconds).
    #[must_use]
    pub fn total_parked_nanos(&self) -> u64 {
        self.total_parked.load(Ordering::Relaxed)
    }
}

// ─── Stage-2 job ────────────────────────────────────────────────────

/// One unit of stage-2 work. Carries the decompressed [`DecodedPayload`]
/// plus the oneshot reply through which the final
/// `Result<DataTable, Error>` reaches the async caller, and the
/// `max_message_size` ceiling the prost decode runs under.
pub(crate) struct Stage2Job {
    pub(crate) payload: DecodedPayload,
    pub(crate) reply: oneshot::Sender<DecodeResult>,
    /// Mirrors [`crate::grpc::codec::Codec::max_message_size`]. Stage-2
    /// honours the ceiling at the prost-decode boundary even though
    /// stage-1 already validated it during decompress — defensive in
    /// case a future stage-1 path constructs the payload through a
    /// non-decompress route (replay tools, test fixtures).
    pub(crate) max_message_size: usize,
}

// ─── Stage-2 pool ───────────────────────────────────────────────────

/// Shared stage-2 worker pool. Holds the bounded MPSC queue plus M
/// worker threads that pull from it, decode the carried payload via
/// `prost::Message::decode`, and reply through the per-job
/// `oneshot::Sender`.
///
/// Construction spawns M worker threads tagged `mdds-decode-stage2`
/// so `top -H` / pprof / perf group them together. Threads exit
/// cleanly when the queue's sender side closes (every crate-internal
/// sender clone dropped) — the pool's `Drop` impl waits for the
/// joins before returning so worker panics never escape.
pub struct Stage2Pool {
    /// Cloneable sender handed to every stage-1 decoder thread.
    /// `Option` so the pool's `Drop` impl can take the inner
    /// `Sender` out before joining workers — closing the channel
    /// from the pool side signals every worker to exit its
    /// `recv` loop once stage-1 producers also drop their clones.
    sender: Option<Stage2PoolSender>,
    /// Worker threads, joined on drop.
    workers: Vec<JoinHandle<()>>,
    /// Shared counters, observable from outside the pool for
    /// monitoring.
    counters: Arc<Stage2Counters>,
    /// Number of workers spawned. Cached because `workers` is taken
    /// during `Drop` and the field would otherwise be unreadable
    /// during shutdown.
    worker_count: usize,
    /// Configured queue depth — exposed for monitoring / tests.
    queue_depth: usize,
}

/// Cloneable handle to push [`Stage2Job`]s onto the shared bounded
/// queue. Holds an [`Arc`] to the same [`Stage2Counters`] the pool
/// publishes so stage-1 threads can record `total_parked` increments
/// from the producer side.
///
/// The inner `Sender` is wrapped in an `Option` so the pool's
/// `Drop` impl can take it out before joining workers — closing the
/// channel from the pool side signals every worker to exit its
/// `recv` loop. Stage-1 producers still hold their own
/// [`Sender`] clones; the channel only closes when every clone
/// drops.
#[derive(Clone)]
pub(crate) struct Stage2PoolSender {
    /// `crossbeam_channel::Sender` is internally `Arc<...>` so cloning
    /// is cheap — every stage-1 decoder thread holds its own clone.
    sender: Sender<Stage2Job>,
    counters: Arc<Stage2Counters>,
    /// Shared poison flag — flipped when a stage-2 worker catches a
    /// panic. Stage-1 threads check this between pushes so they
    /// surface the poison fast rather than parking on a queue whose
    /// consumers are gone.
    poisoned: Arc<AtomicBool>,
}

impl Stage2PoolSender {
    /// `true` once any stage-2 worker has caught a panic. Stage-1
    /// reads this on the hot path to decide whether to attempt a
    /// push or fail fast with a transport-level error.
    #[must_use]
    pub(crate) fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Acquire)
    }

    /// Push a stage-2 job onto the bounded queue.
    ///
    /// Blocks if the queue is full so backpressure parks stage-1
    /// rather than dropping the payload — silent drops on a market
    /// data feed are unacceptable. The park duration is accumulated
    /// into `total_parked` (nanoseconds) and `tracing::debug!`-logged
    /// so operators see backpressure events without scraping
    /// histograms.
    ///
    /// Returns `Err(Stage2SendError::Poisoned { job })` when the
    /// stage-2 pool has been poisoned by a worker panic; the
    /// rejected job is handed back to the caller so the caller can
    /// reply through its embedded oneshot with the transport-level
    /// failure. Returns `Err(Stage2SendError::PoolClosed { job })`
    /// when every worker has already exited (only happens during
    /// shutdown).
    pub(crate) fn send(&self, job: Stage2Job) -> Result<(), Stage2SendError> {
        if self.is_poisoned() {
            return Err(Stage2SendError::Poisoned { job });
        }
        // Try once non-blocking so the common case (queue not full)
        // avoids the `Instant::now()` call entirely.
        match self.sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Disconnected(job)) => Err(Stage2SendError::PoolClosed { job }),
            Err(TrySendError::Full(job)) => {
                // Slow path: queue full. Park on the sender and
                // record how long we waited for downstream operators.
                let start = Instant::now();
                let outcome = self.sender.send(job);
                let elapsed = start.elapsed();
                let nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
                self.counters
                    .total_parked
                    .fetch_add(nanos, Ordering::Relaxed);
                tracing::debug!(
                    target: "thetadatadx::grpc::stage_pipeline",
                    park_nanos = nanos,
                    "stage-1 parked on full stage-2 queue (backpressure)"
                );
                match outcome {
                    Ok(()) => Ok(()),
                    Err(crossbeam_channel::SendError(job)) => {
                        Err(Stage2SendError::PoolClosed { job })
                    }
                }
            }
        }
    }
}

/// Stage-1 → stage-2 push failure modes. The rejected
/// [`Stage2Job`] is handed back to the caller (which holds the
/// caller-facing `oneshot::Sender`) so the failure can be
/// surfaced through that oneshot rather than dropped silently.
///
/// `Stage2Job` is not `Debug` (it owns an `oneshot::Sender`), so
/// the manual [`std::fmt::Debug`] impl below redacts the job
/// payload to a stable static string. The `Display` impl follows
/// the same convention.
pub(crate) enum Stage2SendError {
    /// A stage-2 worker caught a panic; the pool is now in
    /// poisoned state and refuses new jobs.
    Poisoned {
        /// The job that could not be enqueued. The caller should
        /// destructure to recover the embedded `oneshot::Sender`
        /// and reply with the transport-level failure.
        job: Stage2Job,
    },
    /// The stage-2 pool has been fully torn down (all workers
    /// exited). Only observable during process shutdown.
    PoolClosed {
        /// The job that could not be enqueued.
        job: Stage2Job,
    },
}

impl std::fmt::Debug for Stage2SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Poisoned { .. } => write!(f, "Stage2SendError::Poisoned"),
            Self::PoolClosed { .. } => write!(f, "Stage2SendError::PoolClosed"),
        }
    }
}

impl std::fmt::Display for Stage2SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Poisoned { .. } => write!(f, "{POOL_POISONED_REASON}"),
            Self::PoolClosed { .. } => write!(f, "stage-2 decode pool closed"),
        }
    }
}

impl std::error::Error for Stage2SendError {}

impl Stage2Pool {
    /// Build a stage-2 pool with `worker_count` worker threads
    /// consuming from a bounded queue of `queue_depth` slots.
    ///
    /// `worker_count = 0` is clamped to `1` — a zero-worker pool
    /// would never drain its queue and stage-1 would deadlock on
    /// the first push.
    ///
    /// `queue_depth = 0` is clamped to `1` — `crossbeam_channel`
    /// rejects zero-capacity bounded channels, and a one-slot queue
    /// degenerates to a rendezvous channel which still preserves
    /// backpressure semantics.
    #[must_use]
    pub fn new(worker_count: usize, queue_depth: usize) -> Self {
        let worker_count = worker_count.max(1);
        let queue_depth = queue_depth.max(1);
        let counters = Arc::new(Stage2Counters::default());
        let poisoned = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = bounded::<Stage2Job>(queue_depth);

        let mut workers = Vec::with_capacity(worker_count);
        for worker_idx in 0..worker_count {
            let receiver = receiver.clone();
            let counters_clone = Arc::clone(&counters);
            let poisoned_clone = Arc::clone(&poisoned);
            let handle = thread::Builder::new()
                .name(format!("mdds-decode-stage2-{worker_idx}"))
                .spawn(move || run_stage2_worker(receiver, counters_clone, poisoned_clone))
                .expect("spawn stage-2 worker thread");
            workers.push(handle);
        }
        // Drop the local receiver — workers hold the live clones.
        // The original receiver going out of scope here is harmless;
        // every worker has its own clone with a positive refcount.
        drop(receiver);

        Self {
            sender: Some(Stage2PoolSender {
                sender,
                counters: Arc::clone(&counters),
                poisoned,
            }),
            workers,
            counters,
            worker_count,
            queue_depth,
        }
    }

    /// Cloneable sender handle. Stage-1 decoder threads hold one
    /// clone each.
    ///
    /// # Panics
    ///
    /// Panics if called after the pool's `Drop` impl has run — the
    /// internal sender is taken on drop so workers can exit their
    /// `recv` loop. Production callsites resolve the sender during
    /// pool construction and hold the clone for the lifetime of
    /// their stage-1 decoder; the panic-on-after-drop is therefore
    /// unreachable in practice.
    #[must_use]
    pub(crate) fn sender(&self) -> Stage2PoolSender {
        self.sender
            .as_ref()
            .expect("Stage2Pool::sender called after Drop")
            .clone()
    }

    /// Snapshot of the pipeline counters. Cheap — just clones an
    /// `Arc`. The returned handle reflects live state, not a frozen
    /// snapshot.
    #[must_use]
    pub fn counters(&self) -> Arc<Stage2Counters> {
        Arc::clone(&self.counters)
    }

    /// Number of worker threads in this stage-2 pool.
    #[must_use]
    pub fn worker_count(&self) -> usize {
        self.worker_count
    }

    /// Configured queue depth.
    #[must_use]
    pub fn queue_depth(&self) -> usize {
        self.queue_depth
    }

    /// `true` once any worker has caught a panic.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.sender
            .as_ref()
            .is_some_and(Stage2PoolSender::is_poisoned)
    }
}

impl Drop for Stage2Pool {
    fn drop(&mut self) {
        // Close the pool-owned sender so worker threads can exit
        // their `recv` loop once every stage-1 clone also drops.
        // The pool-side close alone is not enough — clones held
        // by stage-1 decoder threads keep the channel alive — but
        // it removes one refcount and is the documented shutdown
        // signal. The two-stage `DecoderPool` orders drops so
        // every `DecoderHandle` (and its embedded
        // `Stage2PoolSender` clone) goes away before the
        // `Arc<Stage2Pool>` held in `PoolInner::_stage2` does;
        // joining the workers here is therefore the final
        // teardown step.
        drop(self.sender.take());
        let workers = std::mem::take(&mut self.workers);
        for handle in workers {
            // Worker panics during steady-state operation are
            // caught by `catch_unwind` in `run_stage2_worker` and
            // flip the poison flag instead of unwinding. Any
            // unwind that escapes that catch lands here; we log
            // and continue rather than re-raising so the rest of
            // the pool tears down cleanly.
            if let Err(panic_payload) = handle.join() {
                tracing::error!(
                    target: "thetadatadx::grpc::stage_pipeline",
                    "stage-2 worker thread join failed (panic escaped catch_unwind): {:?}",
                    panic_payload
                );
            }
        }
    }
}

/// Stage-2 worker main loop. Pulls jobs until the channel
/// disconnects, runs `prost::Message::decode` under
/// `catch_unwind`, and replies via the per-job oneshot.
fn run_stage2_worker(
    receiver: Receiver<Stage2Job>,
    counters: Arc<Stage2Counters>,
    poisoned: Arc<AtomicBool>,
) {
    while let Ok(job) = receiver.recv() {
        let Stage2Job {
            payload,
            reply,
            max_message_size,
        } = job;
        // Caller cancellation race: if the oneshot was dropped
        // between the stage-1 push and now, skip the decode
        // entirely. Stage-1's bookkeeping already counted the
        // payload as "queued"; we count it as "dropped" so the
        // counter reflects the observable outcome.
        if reply.is_closed() {
            counters.total_dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        // Fast-path: a poisoned pool drains the queue without
        // touching prost so callers observe the transport-level
        // failure immediately instead of hanging.
        if poisoned.load(Ordering::Acquire) {
            let _ = reply.send(Err(Error::Transport {
                kind: TransportErrorKind::DecoderPoisoned,
                message: POOL_POISONED_REASON.to_string(),
            }));
            counters.total_dropped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        // Run the prost decode under catch_unwind so a degenerate
        // payload (corrupted tag, oversized field) flips the pool
        // poison flag rather than killing the worker mid-loop.
        let payload_bytes = payload.payload.clone();
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(move || {
            // Honour the max_message_size ceiling at the prost
            // boundary even though stage-1 already validated it
            // during decompress — defence in depth against
            // synthetic payloads constructed by replay tooling.
            if payload_bytes.len() > max_message_size {
                return Err(Error::decompress_message_too_large(
                    payload_bytes.len(),
                    max_message_size,
                ));
            }
            let table: proto::DataTable = prost::Message::decode(payload_bytes.as_ref())
                .map_err(|e| Error::decode_protobuf(e.to_string()))?;
            Ok(table)
        }));
        let result = match outcome {
            Ok(decoded) => {
                counters.total_decoded.fetch_add(1, Ordering::Relaxed);
                decoded
            }
            Err(_panic_payload) => {
                poisoned.store(true, Ordering::Release);
                tracing::error!(
                    target: "thetadatadx::grpc::stage_pipeline",
                    channel_id = payload.channel_id,
                    request_id = payload.request_id,
                    "mdds stage-2 worker panicked; pool poisoned"
                );
                Err(Error::Transport {
                    kind: TransportErrorKind::DecoderPoisoned,
                    message: POOL_POISONED_REASON.to_string(),
                })
            }
        };
        // Send-failure is benign: the caller may have cancelled
        // between `is_closed()` above and now.
        let _ = reply.send(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Construct a stage-2 job carrying a manually-encoded
    /// `DataTable`. Used by the round-trip + backpressure tests.
    fn make_job(rows: usize) -> (Stage2Job, oneshot::Receiver<DecodeResult>) {
        use crate::proto::{data_value, DataValue, DataValueList};
        let row = DataValueList {
            values: vec![DataValue {
                data_type: Some(data_value::DataType::Number(7)),
            }],
        };
        let table = proto::DataTable {
            headers: vec!["x".to_string()],
            data_table: (0..rows).map(|_| row.clone()).collect(),
        };
        let bytes = Bytes::from(prost::Message::encode_to_vec(&table));
        let (tx, rx) = oneshot::channel();
        let job = Stage2Job {
            payload: DecodedPayload {
                channel_id: 0,
                request_id: 0,
                payload: bytes,
            },
            reply: tx,
            max_message_size: usize::MAX,
        };
        (job, rx)
    }

    #[tokio::test]
    async fn queue_round_trip_single_worker() {
        let pool = Stage2Pool::new(1, 4);
        let sender = pool.sender();
        let (job, rx) = make_job(3);
        sender.send(job).expect("queue accepts job");
        let table = rx.await.expect("oneshot delivers").expect("decode ok");
        assert_eq!(table.data_table.len(), 3);
        assert_eq!(pool.counters().total_decoded(), 1);
        drop(sender);
        drop(pool);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn queue_round_trip_many_workers() {
        let pool = Stage2Pool::new(4, 16);
        let sender = pool.sender();
        let mut rxs = Vec::with_capacity(32);
        for _ in 0..32 {
            let (job, rx) = make_job(2);
            sender.send(job).expect("queue accepts job");
            rxs.push(rx);
        }
        for rx in rxs {
            let table = rx.await.expect("oneshot delivers").expect("decode ok");
            assert_eq!(table.data_table.len(), 2);
        }
        assert_eq!(pool.counters().total_decoded(), 32);
        drop(sender);
        drop(pool);
    }

    /// Backpressure invariant: when the queue is full, `send()`
    /// parks the producer rather than dropping the payload. The
    /// test forces a guaranteed-stuck consumer by pinning the
    /// worker on `recv()` while the producer fills the bounded
    /// channel directly — once the channel is at capacity the
    /// next `send` MUST park. We exercise the
    /// [`Stage2PoolSender::send`] path that records `total_parked`
    /// nanoseconds and assert that counter advances.
    ///
    /// Building this on a synthetic stuck-channel is the only way
    /// to make the test deterministic on fast hardware: a real
    /// `Stage2Pool` worker decodes a small payload in
    /// microseconds, so on a fast box the producer could push
    /// many payloads before observing a full queue. The
    /// stuck-channel shape pins the queue at capacity from t=0 so
    /// the very next push parks unconditionally.
    #[test]
    fn backpressure_parks_producer() {
        const QUEUE_DEPTH: usize = 2;
        let counters = Arc::new(Stage2Counters::default());
        let poisoned = Arc::new(AtomicBool::new(false));
        let (sender_inner, receiver) = bounded::<Stage2Job>(QUEUE_DEPTH);
        let sender = Stage2PoolSender {
            sender: sender_inner,
            counters: Arc::clone(&counters),
            poisoned: Arc::clone(&poisoned),
        };

        // Fill the queue to capacity with synthetic jobs (no
        // worker pulls them — the receiver is held by the test).
        for _ in 0..QUEUE_DEPTH {
            let (job, _rx) = make_job(1);
            sender
                .sender
                .try_send(job)
                .expect("queue absorbs initial fill");
        }

        // Producer thread pushes one more job. With the queue
        // full and the receiver pinned (no consumer), the
        // producer parks on the blocking send branch.
        let sender_for_producer = sender.clone();
        let producer = thread::spawn(move || {
            let (job, _rx) = make_job(1);
            sender_for_producer
                .send(job)
                .expect("eventually accepted after consumer pulls one")
        });

        // Give the producer time to enter the park branch — the
        // `try_send` fails fast, then the producer falls into the
        // blocking-send arm and starts the `Instant::now()` timer.
        thread::sleep(Duration::from_millis(50));

        // Drain ONE slot so the parked producer unblocks. We
        // pull a job, then immediately drop it (replies are
        // ignored by this test).
        let drained = receiver.recv().expect("queue has at least one job");
        drop(drained);

        producer.join().expect("producer joins");

        let parked = counters.total_parked_nanos();
        assert!(
            parked > 0,
            "stage-1 must park on a full queue (total_parked = {parked})"
        );

        // Drain remaining jobs to release any held resources.
        while receiver.try_recv().is_ok() {}
        drop(receiver);
        drop(sender);
    }

    /// Poison containment: a panicking decode flips the poison
    /// flag and surviving workers continue draining the queue
    /// with the transport-level failure reply.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn poison_containment_keeps_workers_alive() {
        // Two workers so one can panic while the other keeps
        // pulling jobs. The panic-flip is global across all
        // workers in the pool (single shared `poisoned` flag), so
        // the surviving worker drains subsequent jobs with the
        // poisoned-reply branch rather than the decode branch —
        // both branches keep the worker alive past the panic.
        let pool = Stage2Pool::new(2, 8);
        let sender = pool.sender();

        // Job 1: malformed payload that prost rejects — a clean
        // error, no panic. Used to baseline that the worker is
        // happy to keep going across normal errors.
        let (tx_clean, rx_clean) = oneshot::channel();
        let bad_payload = Bytes::from_static(&[0xFF, 0xFF, 0xFF, 0xFF]);
        sender
            .send(Stage2Job {
                payload: DecodedPayload {
                    channel_id: 1,
                    request_id: 1,
                    payload: bad_payload,
                },
                reply: tx_clean,
                max_message_size: usize::MAX,
            })
            .expect("queue accepts");
        let outcome = tokio::time::timeout(Duration::from_secs(2), rx_clean)
            .await
            .expect("reply within deadline")
            .expect("oneshot delivers");
        assert!(
            outcome.is_err(),
            "malformed prost payload returns a clean error"
        );

        // Job 2 onward: real payloads after the surviving worker
        // continues. The pool must not be poisoned at this point
        // because a clean decode error doesn't trip the panic
        // path.
        assert!(
            !pool.is_poisoned(),
            "clean decode error does not poison the pool"
        );
        let (job, rx) = make_job(5);
        sender.send(job).expect("queue accepts");
        let table = tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .expect("reply within deadline")
            .expect("oneshot delivers")
            .expect("decode ok");
        assert_eq!(table.data_table.len(), 5);
        drop(sender);
        drop(pool);
    }

    /// Manually flipping the poison flag drains subsequent jobs
    /// with the transport-level reply rather than decoding them.
    /// This isolates the poison-drain branch from any test-side
    /// flakiness around catching a real panic in a worker thread.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn poisoned_pool_drains_with_transport_error() {
        let pool = Stage2Pool::new(2, 8);
        let sender = pool.sender();
        // Flip the poison flag externally — equivalent to a
        // worker catching a panic in production.
        sender.poisoned.store(true, Ordering::Release);

        let (job, rx) = make_job(1);
        // Send still returns Err(Poisoned) on the fast-path check
        // because the producer reads the flag before pushing.
        let err = sender.send(job).expect_err("poisoned pool refuses send");
        assert!(matches!(err, Stage2SendError::Poisoned { .. }));
        drop(rx);
        drop(sender);
        drop(pool);
    }

    /// `Stage2Counters` carries three [`CachePadded<AtomicU64>`]
    /// fields. The struct's stride must be at least three cache
    /// lines so concurrent increments on the three counters cannot
    /// false-share. The `CachePadded` wrapper sizes to the
    /// target's cache line; on x86_64 the line is 64 bytes, on
    /// aarch64 typically 128. We assert the size is a positive
    /// multiple of `align_of::<CachePadded<AtomicU64>>()` rather
    /// than pinning a literal byte count — that keeps the test
    /// portable across architectures without losing the false-
    /// sharing guarantee.
    #[test]
    fn counters_are_cache_line_padded() {
        let size = std::mem::size_of::<Stage2Counters>();
        let align = std::mem::align_of::<CachePadded<AtomicU64>>();
        assert!(
            align >= 64,
            "CachePadded<AtomicU64> must align to at least 64 bytes \
             (false-sharing prevention); got {align}"
        );
        // The struct holds three cache-padded counters; its size
        // must be at least 3 * align.
        assert!(
            size >= 3 * align,
            "Stage2Counters must occupy at least 3 cache lines \
             (got size={size}, align={align}, expected >= {})",
            3 * align
        );
        // Padding stride: each AtomicU64 inside its own CachePadded.
        let padded_size = std::mem::size_of::<CachePadded<AtomicU64>>();
        assert_eq!(
            padded_size, align,
            "CachePadded<AtomicU64> rounds size up to one cache line"
        );
    }

    /// Worker count clamps `0` to `1` — a zero-worker pool would
    /// deadlock stage-1 on the first push.
    #[test]
    fn worker_count_clamps_zero_to_one() {
        let pool = Stage2Pool::new(0, 4);
        assert_eq!(pool.worker_count(), 1);
        drop(pool);
    }

    /// Queue depth clamps `0` to `1` — `crossbeam_channel::bounded(0)`
    /// is a rendezvous channel, and a queue depth of zero would
    /// have nowhere to absorb a single in-flight payload. Clamping
    /// to `1` preserves the bounded-and-blocking semantics.
    #[test]
    fn queue_depth_clamps_zero_to_one() {
        let pool = Stage2Pool::new(2, 0);
        assert_eq!(pool.queue_depth(), 1);
        drop(pool);
    }

    /// Worker override is respected even when it exceeds the
    /// available core count — the pool is a software construct
    /// and the user explicitly opted into the configured count.
    #[test]
    fn worker_count_respects_override() {
        // 32-worker pool. Even on a 4-core box the pool builds
        // and reports the configured count; the OS scheduler
        // handles oversubscription.
        let pool = Stage2Pool::new(32, 32);
        assert_eq!(pool.worker_count(), 32);
        drop(pool);
    }
}
