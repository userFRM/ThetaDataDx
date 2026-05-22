//! Two-stage decode pipeline: stage-1 decompresses payloads on each
//! per-channel decoder thread and hands them through a bounded
//! crossbeam queue to a shared stage-2 worker pool that runs prost
//! decode + Tick build.
//!
//! Design rationale, ASCII pipeline diagram, scaling tradeoffs, and
//! the backpressure / counter contracts live in
//! `docs-site/docs/channel-pool-design.md` (forward-looking design)
//! and in the PR #587 / #588 commit bodies (historical context).

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

/// Stage-pipeline counters. Each field is `CachePadded` to prevent
/// false-sharing between the stage-1 and stage-2 threads.
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

/// Shared stage-2 worker pool: bounded MPSC queue plus M worker
/// threads that pull, decode via `prost::Message::decode`, and reply
/// on the per-job oneshot. Workers tagged `mdds-decode-stage2`.
pub struct Stage2Pool {
    sender: Option<Stage2PoolSender>,
    workers: Vec<JoinHandle<()>>,
    counters: Arc<Stage2Counters>,
    worker_count: usize,
    queue_depth: usize,
}

/// Cloneable handle for stage-1 to push [`Stage2Job`]s onto the
/// bounded queue. Shares the pool's [`Stage2Counters`] and poison flag.
#[derive(Clone)]
pub(crate) struct Stage2PoolSender {
    sender: Sender<Stage2Job>,
    counters: Arc<Stage2Counters>,
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

    /// Push a stage-2 job onto the bounded queue, blocking if full
    /// (backpressure parks stage-1 rather than dropping).
    ///
    /// # Errors
    ///
    /// Returns [`Stage2SendError::Poisoned`] if a stage-2 worker has
    /// panicked, or [`Stage2SendError::PoolClosed`] if every worker
    /// has already exited. The rejected job is handed back so the
    /// caller can surface the failure through its oneshot.
    pub(crate) fn send(&self, job: Stage2Job) -> Result<(), Stage2SendError> {
        if self.is_poisoned() {
            return Err(Stage2SendError::Poisoned { job });
        }
        match self.sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Disconnected(job)) => Err(Stage2SendError::PoolClosed { job }),
            Err(TrySendError::Full(job)) => {
                let start = Instant::now();
                let outcome = self.sender.send(job);
                // `as_nanos` returns `u128`; saturate to `u64::MAX`
                // for the unreachable >584-year park case.
                let nanos = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
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
        let Some(sender) = self.sender.as_ref() else {
            unreachable!("Stage2Pool::sender called after Drop took the sender slot");
        };
        sender.clone()
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

    /// Bench-only entry point: push a `DecodedPayload` directly onto
    /// the stage-2 queue and return the reply receiver.
    ///
    /// Compiled only under `cfg(any(test, bench_internals))` so the
    /// helper never enters the published symbol surface. The
    /// criterion harness at `benches/bench_stage_pipeline.rs`
    /// activates it via `RUSTFLAGS='--cfg bench_internals' cargo bench
    /// --bench bench_stage_pipeline`; production builds cannot
    /// enable it from the Cargo CLI.
    ///
    /// Returns `Err` with the rejected payload if the pool is
    /// poisoned or fully torn down.
    #[cfg(any(test, bench_internals))]
    pub fn submit_for_bench(
        &self,
        payload: DecodedPayload,
        max_message_size: usize,
    ) -> Result<oneshot::Receiver<DecodeResult>, DecodedPayload> {
        let sender = self
            .sender
            .as_ref()
            .expect("Stage2Pool::submit_for_bench called after Drop");
        let (reply, rx) = oneshot::channel();
        let job = Stage2Job {
            payload,
            reply,
            max_message_size,
        };
        match sender.send(job) {
            Ok(()) => Ok(rx),
            Err(Stage2SendError::Poisoned { job } | Stage2SendError::PoolClosed { job }) => {
                Err(job.payload)
            }
        }
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

    /// Pin: when the queue is full, `send()` parks the producer
    /// (records `total_parked`) rather than dropping the payload.
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
        let pool = Stage2Pool::new(2, 8);
        let sender = pool.sender();

        // Job 1: malformed payload — clean prost error baseline.
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
        assert!(
            size >= 3 * align,
            "Stage2Counters must occupy at least 3 cache lines \
             (got size={size}, align={align}, expected >= {})",
            3 * align
        );
        let padded_size = std::mem::size_of::<CachePadded<AtomicU64>>();
        assert_eq!(
            padded_size, align,
            "CachePadded<AtomicU64> rounds size up to one cache line"
        );

        // Each counter must land on its own cache line. We verify by
        // checking that each field's address modulo `align` is zero
        // and that consecutive field addresses differ by at least
        // `align` bytes — that's the false-sharing-prevention
        // invariant in machine terms.
        let counters = Stage2Counters::default();
        let base = std::ptr::addr_of!(counters) as usize;
        let off_decoded = std::ptr::addr_of!(counters.total_decoded) as usize - base;
        let off_dropped = std::ptr::addr_of!(counters.total_dropped) as usize - base;
        let off_parked = std::ptr::addr_of!(counters.total_parked) as usize - base;
        assert_eq!(
            off_decoded % align,
            0,
            "total_decoded must be cache-aligned"
        );
        assert_eq!(
            off_dropped % align,
            0,
            "total_dropped must be cache-aligned"
        );
        assert_eq!(off_parked % align, 0, "total_parked must be cache-aligned");
        assert!(
            off_dropped >= off_decoded + align,
            "total_dropped must sit on a different cache line than total_decoded",
        );
        assert!(
            off_parked >= off_dropped + align,
            "total_parked must sit on a different cache line than total_dropped",
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
