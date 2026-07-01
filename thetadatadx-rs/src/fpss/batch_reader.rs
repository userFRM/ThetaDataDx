//! Pull-based Arrow `RecordBatch` reader over the FPSS stream.
//!
//! [`RecordBatchStream`] is a sibling delivery mode to the per-event
//! callback: instead of pushing one [`StreamEvent`] at a time into a user
//! closure, it pulls the same market-data events out in columnar Arrow
//! batches under the fixed [`super::batch_schema::stream_batch_schema`]. The
//! callback path is unchanged; a caller chooses one or the other per
//! subscription.
//!
//! # Pipeline
//!
//! The reader reuses the exact streaming machinery the callback path uses:
//! `client.stream().batches(..).build()` starts FPSS through the same
//! connect / install / dispatcher path as `start_streaming`, but the
//! dispatcher's per-event handler appends each market-data event to a reused
//! [`super::batch_schema::StreamBatchBuilder`] instead of calling user code.
//! The dispatcher flushes a [`RecordBatch`] onto a bounded internal queue
//! whenever the first of these fires:
//!
//! * the builder reaches `batch_size` rows,
//! * `linger` elapses since the first row of the current batch (so a quiet
//!   stream still delivers a partial batch rather than stalling), or
//! * the stream shuts down (a final partial batch is flushed before the
//!   terminal end-of-stream marker).
//!
//! The reader side ([`futures_core::Stream`], the `next_blocking` pull, or the
//! FFI) pulls finished batches off that queue. The queue is always bounded; it
//! never grows without limit.
//!
//! # Backpressure
//!
//! [`Backpressure`] selects what happens when the reader falls behind and
//! the queue fills:
//!
//! * [`Backpressure::Block`] (the default, lossless): the dispatcher blocks
//!   on a full queue. Because the dispatcher is the ring drainer, blocking
//!   it stops draining the ring; a full ring then applies backpressure to
//!   the wire. No event is dropped.
//! * [`Backpressure::DropOldest`]: the queue is a bounded ring of at most
//!   `capacity` batches; pushing onto a full queue drops the oldest batch
//!   and increments the observable [`RecordBatchStream::dropped`] counter.
//!   The dispatcher never blocks. Drops are counted, never silent.
//!
//! # Lifecycle
//!
//! Dropping the [`RecordBatchStream`] (or its blocking iterator, or calling
//! [`RecordBatchStream::close`]) stops the FPSS session through the same
//! `stop_streaming` path the callback surface uses, which tears the
//! subscription down and ends the dispatcher. No thread, socket, or
//! subscription leaks.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once, Weak};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use arrow_array::RecordBatch;
use arrow_schema::Schema;

use super::batch_schema::{stream_batch_schema, StreamBatchBuilder};
use super::{StreamEvent, StreamingClient};
use crate::client::{Client, StreamingState};
use crate::streaming::StreamError;

/// Default rows per batch when [`BatchReaderBuilder::batch_size`] is not set.
pub const DEFAULT_BATCH_SIZE: usize = 65_536;

/// Default linger before a partial batch is flushed when
/// [`BatchReaderBuilder::linger`] is not set.
pub const DEFAULT_LINGER: Duration = Duration::from_millis(50);

/// Default bounded-queue depth, in batches, for both backpressure modes.
///
/// The queue holds finished batches waiting for the reader. A small bound
/// keeps memory flat (each slot is one full `batch_size` batch) while
/// leaving enough slack that a reader briefly off-CPU does not immediately
/// stall the dispatcher.
pub const DEFAULT_QUEUE_DEPTH: usize = 4;

/// Upper bound on [`BatchReaderBuilder::batch_size`].
///
/// The batch builder preallocates every column of the fixed schema to the
/// batch size up front, so the prealloc scales linearly with this value
/// (roughly a few hundred bytes per row across the schema's columns). The
/// bound caps a single batch's prealloc to a few hundred megabytes — recoverable
/// rather than an out-of-memory abort — while sitting far above any realistic
/// streaming batch (it is 16x the default and over a million rows). A larger
/// request is clamped here rather than honored, which matters because a binding
/// can hand the core a value an unsigned conversion has wrapped to near the
/// integer maximum; without this clamp that wrap would drive a multi-hundred-
/// gigabyte prealloc.
pub const MAX_BATCH_SIZE: usize = 1_048_576;

/// Upper bound on the bounded-queue depth (the `capacity` of
/// [`Backpressure::DropOldest`], and the Block-mode queue depth).
///
/// The queue preallocates its backing to this many batch slots, and under
/// `DropOldest` it can hold this many finished batches at once, so the buffered
/// memory is `capacity * batch_size`. Capping the depth keeps that product
/// bounded; the default is 4 and any realistic reader needs only a handful of
/// slots of slack, so this bound is generous while still defending against a
/// wrapped or fat-fingered value that would otherwise preallocate a deque of
/// billions of slots.
pub const MAX_QUEUE_DEPTH: usize = 4_096;

/// What happens when the reader falls behind and the bounded batch queue
/// fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backpressure {
    /// Lossless: block the dispatcher on a full queue, which stops the ring
    /// drain and applies backpressure to the wire. The default.
    #[default]
    Block,
    /// Bounded buffer: keep at most `capacity` finished batches; on overflow
    /// drop the oldest and increment [`RecordBatchStream::dropped`]. Never
    /// blocks the dispatcher.
    DropOldest {
        /// Maximum number of finished batches buffered before the oldest is
        /// dropped. Must be at least 1; a `0` is treated as 1.
        capacity: usize,
    },
}

impl Backpressure {
    /// The bounded-queue depth this policy resolves to, clamped to
    /// `[1, MAX_QUEUE_DEPTH]`. `Block` uses [`DEFAULT_QUEUE_DEPTH`];
    /// `DropOldest` uses its `capacity`. Clamping here is the single chokepoint
    /// every caller (the live reader and the offline test harness) shares, so a
    /// wrapped or oversized `capacity` from any binding cannot preallocate a
    /// runaway deque.
    fn resolved_capacity(self) -> usize {
        let requested = match self {
            Backpressure::Block => DEFAULT_QUEUE_DEPTH,
            Backpressure::DropOldest { capacity } => capacity,
        };
        requested.clamp(1, MAX_QUEUE_DEPTH)
    }
}

/// Clamp a requested rows-per-batch to `[1, MAX_BATCH_SIZE]`. The single
/// chokepoint the builder setter and [`RecordBatchStream::start`] share, so a
/// value any binding produced (including one an unsigned conversion wrapped to
/// near the integer maximum) cannot drive a catastrophic per-column prealloc.
fn clamp_batch_size(rows: usize) -> usize {
    rows.clamp(1, MAX_BATCH_SIZE)
}

/// Builder for a [`RecordBatchStream`].
///
/// Returned by `client.stream().batches()`. Subscriptions are managed on the
/// same streaming surface as the callback path (`client.stream().subscribe`),
/// so a caller subscribes first and then opens the reader, exactly as they
/// would register a callback.
#[must_use = "call `.build()` to open the RecordBatch stream"]
pub struct BatchReaderBuilder<'a> {
    client: &'a Client,
    batch_size: usize,
    linger: Duration,
    backpressure: Backpressure,
}

impl<'a> BatchReaderBuilder<'a> {
    /// Construct a builder over a unified client. The public entry point is
    /// `client.stream().batches()`; this is the crate-internal constructor
    /// it calls.
    pub(crate) fn new(client: &'a Client) -> Self {
        Self {
            client,
            batch_size: DEFAULT_BATCH_SIZE,
            linger: DEFAULT_LINGER,
            backpressure: Backpressure::default(),
        }
    }

    /// Rows per batch. A batch flushes when it reaches this many rows or
    /// when [`Self::linger`] elapses, whichever comes first. Defaults to
    /// [`DEFAULT_BATCH_SIZE`]. A `0` is clamped to `1`; a value above
    /// [`MAX_BATCH_SIZE`] is clamped down to it, so a binding that wrapped an
    /// out-of-range request into a huge unsigned value cannot drive a
    /// catastrophic column prealloc.
    pub fn batch_size(mut self, rows: usize) -> Self {
        self.batch_size = clamp_batch_size(rows);
        self
    }

    /// Maximum time a partial batch waits before being flushed, so a quiet
    /// stream still delivers. Defaults to [`DEFAULT_LINGER`].
    pub fn linger(mut self, linger: Duration) -> Self {
        self.linger = linger;
        self
    }

    /// Backpressure policy for a reader that falls behind. Defaults to
    /// [`Backpressure::Block`] (lossless).
    pub fn backpressure(mut self, backpressure: Backpressure) -> Self {
        self.backpressure = backpressure;
        self
    }

    /// Open the reader: start FPSS streaming with a batching dispatcher and
    /// return the [`RecordBatchStream`] handle.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure
    /// while establishing the FPSS connection, or if a stream is already
    /// active on this client.
    pub fn build(self) -> Result<RecordBatchStream, crate::error::Error> {
        RecordBatchStream::start(self.client, self.batch_size, self.linger, self.backpressure)
    }
}

/// The bounded queue shared between the dispatcher (producer) and the reader
/// (consumer).
///
/// A single mutex guards the deque plus the terminal flags; a `Condvar`
/// wakes a blocked producer (Block mode) or a blocked blocking-iterator
/// consumer. The async [`futures_core::Stream`] path additionally stores a
/// [`Waker`] so the dispatcher can wake a parked async task when a batch
/// lands or the stream finishes.
pub(crate) struct Shared {
    inner: Mutex<Queue>,
    /// Notifies a consumer that a batch or terminal state is available, and
    /// notifies a Block-mode producer that a slot freed up.
    cv: Condvar,
    /// Observable count of batches dropped under [`Backpressure::DropOldest`].
    dropped: AtomicU64,
    /// Set by [`RecordBatchStream::close`] / drop to tell the dispatcher to
    /// stop draining and exit.
    closed: AtomicBool,
    /// The fixed schema every batch carries. Cloned to callers via
    /// [`RecordBatchStream::schema`] without locking.
    schema: Arc<Schema>,
}

impl Shared {
    /// Signal teardown to the dispatcher and any parked waiter: set `closed`
    /// (under `inner`, per the lost-wakeup discipline) and wake the queue
    /// condvar plus any registered async waker.
    ///
    /// This is the one place the `closed` predicate is published, so every
    /// teardown route shares the same wakeup guarantee:
    ///
    /// * the reader's own [`RecordBatchStream::close_shared`] calls it, and
    /// * a teardown that bypasses the reader (a `Client` drop /
    ///   `stop_streaming` / `reconnect_streaming`, which run
    ///   `StreamingState::quiesce` directly) calls it through the wake hook the
    ///   columnar dispatcher installs, BEFORE quiesce joins the dispatcher
    ///   thread.
    ///
    /// Without this hook the latter routes would shut only the FPSS side and
    /// never the batch queue, so a Block-mode dispatcher parked in `flush` on a
    /// full queue would wait on `closed` forever and quiesce's join would hang.
    ///
    /// Idempotent: storing `closed` again and notifying an empty condvar are
    /// both no-ops, so it is safe to call from several teardown paths.
    pub(crate) fn close_and_wake(&self) {
        // Store under `inner` so it serialises with a waiter's under-lock
        // check-then-park in `flush` / `next_blocking` (see those sites). A
        // store outside the lock opens a lost-wakeup window.
        let waker = {
            let mut guard = lock(&self.inner);
            self.closed.store(true, Ordering::Release);
            guard.waker.take()
        };
        self.cv.notify_all();
        if let Some(w) = waker {
            w.wake();
        }
    }
}

struct Queue {
    /// Finished batches awaiting the reader.
    batches: VecDeque<RecordBatch>,
    /// Bound on `batches`; the meaning differs per mode but the deque is
    /// bounded in both.
    capacity: usize,
    /// `true` once the dispatcher has flushed its final batch and will
    /// produce no more (clean shutdown or close).
    finished: bool,
    /// A terminal error from the dispatcher, surfaced once to the reader
    /// after any already-queued batches are consumed.
    error: Option<StreamError>,
    /// Waker for a parked async [`futures_core::Stream`] consumer.
    waker: Option<Waker>,
}

/// Mutable batching state, guarded so the dispatcher's two closures — the
/// per-event handler and the batch scope — can each take a short, separate
/// borrow without nesting one inside the other.
struct BatchAccum {
    builder: StreamBatchBuilder,
    /// Wall-clock instant the current (non-empty) batch began; `None` while
    /// the batch is empty. The linger deadline is measured from the first
    /// row so a steady trickle still flushes on time.
    batch_started: Option<Instant>,
}

/// State carried by the batching dispatcher handler + scope. Cloned (by
/// `Arc`) into both closures; communicates with the reader only through
/// `shared`.
///
/// The handler ([`Self::on_event`]) runs inside the ring drain, which the
/// scope ([`Self::scope_drain`]) invokes. Both mutate the accumulator, so
/// the accumulator sits behind its own `Mutex` and each method takes a
/// short lock it releases before returning. Crucially, `scope_drain` never
/// holds that lock across the `drain()` call, so the handler's lock and the
/// scope's lock never overlap and the drain path is lock-free between
/// events.
#[derive(Clone)]
pub(crate) struct BatchSink {
    shared: Arc<Shared>,
    accum: Arc<Mutex<BatchAccum>>,
    batch_size: usize,
    linger: Duration,
    backpressure: Backpressure,
}

impl BatchSink {
    pub(crate) fn new(
        shared: Arc<Shared>,
        batch_size: usize,
        linger: Duration,
        backpressure: Backpressure,
    ) -> Self {
        Self {
            shared,
            accum: Arc::new(Mutex::new(BatchAccum {
                builder: StreamBatchBuilder::with_capacity(batch_size),
                batch_started: None,
            })),
            batch_size,
            linger,
            backpressure,
        }
    }

    /// Per-event handler driven by the dispatcher: append one market-data
    /// row and flush a full batch. Holds the accumulator lock only for the
    /// append; the flush takes the queue lock after the accumulator lock is
    /// released.
    pub(crate) fn on_event(&self, event: &StreamEvent) {
        let full_batch = {
            let mut accum = lock(&self.accum);
            if accum.builder.is_empty() {
                accum.batch_started = Some(Instant::now());
            }
            accum.builder.append(event) && accum.builder.len() >= self.batch_size
        };
        if full_batch {
            self.flush();
        }
    }

    /// Run by the dispatcher's batch scope once per drain attempt (including
    /// empty drains before an idle wait). Enforces the linger flush on a
    /// quiet stream and signals shutdown when the reader has closed. The
    /// accumulator lock is taken only for the linger check, never across
    /// `drain()`, so the handler invoked inside `drain()` never contends
    /// with this method.
    pub(crate) fn scope_drain(
        &self,
        drain: &mut dyn FnMut() -> crate::PollOutcome,
    ) -> crate::PollOutcome {
        if self.shared.closed.load(Ordering::Acquire) {
            return crate::PollOutcome::Shutdown;
        }
        let outcome = drain();
        let lingered = {
            let accum = lock(&self.accum);
            match accum.batch_started {
                Some(started) => !accum.builder.is_empty() && started.elapsed() >= self.linger,
                None => false,
            }
        };
        if lingered {
            self.flush();
        }
        if self.shared.closed.load(Ordering::Acquire) {
            return crate::PollOutcome::Shutdown;
        }
        outcome
    }

    /// Finish the current batch and enqueue it under the backpressure
    /// policy. A build error becomes the stream's terminal error; a no-row
    /// builder is a no-op.
    ///
    /// Takes the accumulator lock to drain the builder, releases it, then
    /// takes the queue lock to enqueue — the two locks are never held at
    /// once, so a Block-mode wait on a full queue cannot stall the handler.
    fn flush(&self) {
        let batch = {
            let mut accum = lock(&self.accum);
            match accum.builder.finish() {
                Ok(Some(batch)) => {
                    accum.batch_started = None;
                    batch
                }
                Ok(None) => return,
                Err(arrow_err) => {
                    drop(accum);
                    // A column-length mismatch while assembling the batch.
                    // The append discipline makes this unreachable in
                    // practice; surface it as a protocol-class terminal
                    // error rather than dropping rows silently.
                    let err =
                        StreamError::Protocol(format!("arrow batch assembly failed: {arrow_err}"));
                    let mut guard = lock(&self.shared.inner);
                    if guard.error.is_none() {
                        guard.error = Some(err);
                    }
                    let waker = guard.waker.take();
                    drop(guard);
                    self.shared.cv.notify_all();
                    if let Some(w) = waker {
                        w.wake();
                    }
                    return;
                }
            }
        };

        let mut guard = lock(&self.shared.inner);
        match self.backpressure {
            Backpressure::Block => {
                while guard.batches.len() >= guard.capacity {
                    if self.shared.closed.load(Ordering::Acquire) {
                        // Reader is gone; drop rather than block forever.
                        // Teardown, not a DropOldest drop, so no counter bump.
                        return;
                    }
                    guard = self
                        .shared
                        .cv
                        .wait(guard)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                guard.batches.push_back(batch);
            }
            Backpressure::DropOldest { .. } => {
                if guard.batches.len() >= guard.capacity {
                    let _ = guard.batches.pop_front();
                    self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                }
                guard.batches.push_back(batch);
            }
        }
        let waker = guard.waker.take();
        drop(guard);
        self.shared.cv.notify_all();
        if let Some(w) = waker {
            w.wake();
        }
    }

    /// Final flush + terminal marker, run once when the dispatcher's drain
    /// loop returns (clean shutdown or close).
    pub(crate) fn finish(&self) {
        let has_rows = {
            let accum = lock(&self.accum);
            !accum.builder.is_empty()
        };
        if has_rows {
            self.flush();
        }
        let mut guard = lock(&self.shared.inner);
        guard.finished = true;
        let waker = guard.waker.take();
        drop(guard);
        self.shared.cv.notify_all();
        if let Some(w) = waker {
            w.wake();
        }
    }
}

/// A pull reader of Arrow [`RecordBatch`] values off the FPSS stream.
///
/// Implements [`futures_core::Stream`] for async consumption and exposes
/// [`Self::next_blocking`] for a synchronous pull. See the module docs for the
/// pipeline, backpressure, and lifecycle contract.
pub struct RecordBatchStream {
    shared: Arc<Shared>,
    /// The live streaming client backing this reader, shared with the
    /// dispatcher thread. Closing the reader quiesces the owning client (see
    /// `owner`), which shuts this same underlying client: it stops the I/O and
    /// ping threads, closes the socket, and shuts the ring. The dispatcher then
    /// observes ring shutdown, flushes the final batch, and exits. This `Arc`
    /// is also the shutdown fallback when the owning client is already gone.
    client: Arc<StreamingClient>,
    /// Weak handle to the owning client's streaming lifecycle state. On close
    /// the reader upgrades this and calls [`StreamingState::quiesce`] — the
    /// same teardown the callback surface's `stop_streaming` runs — so after
    /// the reader closes the client's `state` is `Stopped` and its
    /// `dispatcher` is `Idle`: `is_streaming` / `connection_status` tell the
    /// truth and a subsequent `start_streaming*` / `batches()` succeeds rather
    /// than returning `already_streaming`. Weak so the reader never keeps the
    /// client's state alive past the client; if the client is already gone the
    /// upgrade fails and the reset is a no-op (the client's own `Drop` already
    /// quiesced).
    owner: Weak<StreamingState>,
    /// The owning client's stop-generation at the instant this reader's
    /// session was installed. On close the reader quiesces only if the live
    /// generation still matches: it identifies the session THIS reader
    /// started, so a reader whose session was already superseded (by a
    /// `stop_streaming` / `reconnect_streaming` / a later session on the same
    /// client) leaves the current session to its owner rather than tearing
    /// down one it does not own.
    owned_generation: u64,
    /// Runs the owner-state reset exactly once, even though `close_shared` is
    /// `&self` and a binding may call it from several paths (explicit close,
    /// then free, then the core `Drop`) and from different threads.
    quiesce_once: Once,
    closed: bool,
}

impl RecordBatchStream {
    fn start(
        client: &Client,
        batch_size: usize,
        linger: Duration,
        backpressure: Backpressure,
    ) -> Result<Self, crate::error::Error> {
        // Clamp both memory-scaling knobs here as well as in the builder, so a
        // caller that reaches `start` by any path (including a binding that
        // constructed the values directly) cannot preallocate past the bounds.
        let batch_size = clamp_batch_size(batch_size);
        let capacity = backpressure.resolved_capacity();
        let shared = Arc::new(Shared {
            inner: Mutex::new(Queue {
                batches: VecDeque::with_capacity(capacity),
                capacity,
                finished: false,
                error: None,
                waker: None,
            }),
            cv: Condvar::new(),
            dropped: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            schema: stream_batch_schema(),
        });

        // Start FPSS through the same connect / install / dispatcher path
        // the callback surface uses; the dispatcher runs our batching sink
        // instead of user code. On a connect / install failure the session
        // is rolled back by the shared start path, so `shared` simply drops
        // here with no thread left behind. On success we hold the live
        // client `Arc` so close / drop can shut the session deterministically.
        let (live, owned_generation) = client.start_streaming_batches(
            Arc::clone(&shared),
            batch_size,
            linger,
            backpressure,
        )?;
        // Capture the owner-state handle only after a successful start, so a
        // failed build leaves no reference behind. Weak, so the reader's later
        // close never resurrects or outlives the client's state. The
        // `owned_generation` stamps the session this reader started so close
        // only tears that session down, never a later one that replaced it.
        let owner = client.streaming_state_weak();

        Ok(Self {
            shared,
            client: live,
            owner,
            owned_generation,
            quiesce_once: Once::new(),
            closed: false,
        })
    }

    /// The fixed schema every batch this stream yields will carry. Stable
    /// for the stream's lifetime and identical across all batches, so a
    /// downstream consumer can concatenate batches without reconciling
    /// schemas.
    #[must_use]
    pub fn schema(&self) -> Arc<Schema> {
        Arc::clone(&self.shared.schema)
    }

    /// Number of batches dropped so far under [`Backpressure::DropOldest`].
    /// Always `0` under [`Backpressure::Block`] (which never drops).
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// Stop the reader: tear down the FPSS session and let the dispatcher
    /// exit. Idempotent; [`Drop`] calls it.
    pub fn close(&mut self) {
        self.shutdown();
    }

    /// Signal close through a shared reference, for a caller that holds the
    /// reader behind an `Arc` and may have a blocking [`Self::next_blocking`]
    /// in flight on another thread.
    ///
    /// Sets the close flag, tears the FPSS session down, and wakes any
    /// parked producer / consumer — all through `&self`, so it can run
    /// concurrently with a blocking pull. The pull then unblocks (the
    /// dispatcher publishes `finished` once the ring shuts down) and returns
    /// `Ok(None)`. Idempotent; safe to call from any thread, any number of
    /// times. This is the teardown the language bindings call on their
    /// `close()` / context-manager exit, where the reader is shared and a
    /// pull may be blocked: it never needs exclusive ownership, so it cannot
    /// deadlock against the in-flight pull the way a `&mut` close would.
    ///
    /// Invariant: this tears down only the session this reader started. If the
    /// owning client has since moved on to a different session (a
    /// `stop_streaming` / `reconnect_streaming`, or a later session on the same
    /// client), that session is left to its current owner.
    pub fn close_shared(&self) {
        // Always signal THIS reader's own queue: set `closed` and wake a
        // Block-mode dispatcher parked on a full queue (and any parked
        // consumer), so a parked flush re-checks `closed`, stops pushing, and
        // the dispatcher reaches the ring-shutdown exit. `close_and_wake`
        // stores `closed` under `inner` (the lost-wakeup discipline) and
        // notifies. This same call is the wake hook the columnar dispatcher
        // installs, so the quiesce-direct teardown paths get the identical
        // wakeup; see `Shared::close_and_wake`. It acts on this reader's own
        // `Shared`, so it is correct regardless of which session is now live.
        self.shared.close_and_wake();
        // Authoritative teardown, run exactly once. Reset the owning client's
        // streaming state through `StreamingState::quiesce_if_owned`: swap the
        // slot to `Stopped`, shut the live client (stop the I/O + ping threads,
        // close the socket, shut the ring), and retire the dispatcher session,
        // leaving the client truthful (`is_streaming` / `connection_status`
        // report a stopped session) and reusable.
        //
        // `quiesce_if_owned` retires only when the live generation still equals
        // the one this reader's session was installed at, and does the
        // generation re-check and the teardown ATOMICALLY under the dispatcher
        // lock. So a teardown that advanced the generation (a `stop_streaming` /
        // `reconnect_streaming`, or a later session on the same client) either
        // fully precedes this call (the re-check fails, leaving the newer
        // session to its owner) or fully follows it (this reader's session is
        // already retired) — never interleaves with it, which a separate
        // check-then-quiesce would allow.
        //
        // If the client has already been dropped the weak upgrade fails (its
        // own `Drop` already quiesced); fall back to shutting our own client
        // `Arc` directly, which is all that remains and only touches this
        // reader's session.
        self.quiesce_once.call_once(|| match self.owner.upgrade() {
            Some(state) => state.quiesce_if_owned(self.owned_generation),
            None => self.client.shutdown(),
        });
    }

    /// Blocking pull of the next batch. Shared by the FFI and bindings.
    /// Returns `Ok(None)` at clean end of stream.
    ///
    /// # Errors
    ///
    /// Surfaces a terminal [`StreamError`] from the dispatcher once all
    /// queued batches have been drained.
    pub fn next_blocking(&self) -> Result<Option<RecordBatch>, StreamError> {
        let mut guard = lock(&self.shared.inner);
        loop {
            if let Some(batch) = guard.batches.pop_front() {
                self.shared.cv.notify_all();
                return Ok(Some(batch));
            }
            if let Some(err) = guard.error.take() {
                return Err(err);
            }
            if guard.finished {
                return Ok(None);
            }
            // A close signalled from another thread ends the pull as soon as
            // the queue is drained, without waiting for the dispatcher's
            // terminal `finished` to land — `close_shared` wakes this cv, and
            // checking `closed` here lets a blocked pull return `None`
            // promptly so close never appears to hang on a quiet stream.
            // Any already-queued batches above are still delivered first.
            if self.shared.closed.load(Ordering::Acquire) {
                return Ok(None);
            }
            guard = self
                .shared
                .cv
                .wait(guard)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Non-blocking poll of the next batch for the async [`futures_core::Stream`]
    /// impl. Registers `waker` when the queue is momentarily empty.
    fn poll_next_inner(&self, cx: &Context<'_>) -> Poll<Option<Result<RecordBatch, StreamError>>> {
        let mut guard = lock(&self.shared.inner);
        if let Some(batch) = guard.batches.pop_front() {
            self.shared.cv.notify_all();
            return Poll::Ready(Some(Ok(batch)));
        }
        if let Some(err) = guard.error.take() {
            return Poll::Ready(Some(Err(err)));
        }
        if guard.finished {
            return Poll::Ready(None);
        }
        // Close from another task ends the stream once the queue is drained,
        // without waiting for the dispatcher's terminal `finished` (see
        // `next_blocking`).
        if self.shared.closed.load(Ordering::Acquire) {
            return Poll::Ready(None);
        }
        // Only re-store the waker when it would wake a different task than the
        // one already parked, avoiding a clone on every Pending poll.
        if !guard
            .waker
            .as_ref()
            .is_some_and(|w| w.will_wake(cx.waker()))
        {
            guard.waker = Some(cx.waker().clone());
        }
        Poll::Pending
    }

    /// Signal close, tear the session down, and wake any parked producer /
    /// consumer. Safe to call more than once. Delegates to
    /// [`Self::close_shared`]; the `closed` field is the owner-side
    /// idempotency guard so a `Drop` after an explicit `close()` is a no-op.
    fn shutdown(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.close_shared();
    }
}

impl Drop for RecordBatchStream {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl futures_core::Stream for RecordBatchStream {
    type Item = Result<RecordBatch, StreamError>;

    fn poll_next(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // `RecordBatchStream` holds no self-referential state; polling
        // through a shared reference is sound.
        self.get_mut().poll_next_inner(cx)
    }
}

/// Lock a mutex, recovering the guard on poison. The streaming pipeline
/// never leaves the protected state inconsistent across a panic boundary, so
/// a poisoned lock is recovered rather than propagated.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Offline test harness for the batch pipeline.
///
/// The live [`RecordBatchStream`] is fed by the FPSS dispatcher thread,
/// which needs a real connection. To exercise the batching / linger /
/// backpressure / reader machinery without a network, this harness wires the
/// exact same [`Shared`] queue, [`BatchSink`], and reader-side drain logic to
/// a synthetic producer that the test drives directly — mirroring how the
/// streaming benches drive the ring offline.
#[cfg(test)]
pub(crate) mod test_harness {
    use super::stream_batch_schema;
    use super::{Backpressure, BatchSink, Queue, Shared};
    use crate::fpss::protocol::Contract;
    use crate::fpss::{StreamData, StreamEvent};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    /// A producer half driving a [`BatchSink`] exactly as the dispatcher
    /// would: `feed` runs the per-event handler, `tick` runs the batch
    /// scope's linger pass, and `finish` flushes + marks terminal.
    pub(crate) struct Producer {
        sink: BatchSink,
    }

    impl Producer {
        /// Append one event through the sink's per-event handler.
        pub(crate) fn feed(&self, event: &StreamEvent) {
            self.sink.on_event(event);
        }

        /// Drive the linger pass once (the scope hook on an idle drain).
        pub(crate) fn tick(&self) {
            // An idle drain returns `Drained(0)`; the scope's linger check
            // runs against it.
            let mut drain = || crate::PollOutcome::Drained(0);
            let _ = self.sink.scope_drain(&mut drain);
        }

        /// Flush the final partial batch and publish the terminal marker.
        pub(crate) fn finish(self) {
            self.sink.finish();
        }
    }

    /// Build a connected `(producer, reader)` pair sharing one queue, with no
    /// FPSS connection. The reader is a real [`RecordBatchStream`] whose
    /// `client` field is left unset by using the blocking reader directly.
    pub(crate) fn harness(
        batch_size: usize,
        linger: Duration,
        backpressure: Backpressure,
    ) -> (Producer, Reader) {
        let capacity = backpressure.resolved_capacity();
        let shared = Arc::new(Shared {
            inner: Mutex::new(Queue {
                batches: VecDeque::with_capacity(capacity),
                capacity,
                finished: false,
                error: None,
                waker: None,
            }),
            cv: Condvar::new(),
            dropped: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            schema: stream_batch_schema(),
        });
        let sink = BatchSink::new(Arc::clone(&shared), batch_size, linger, backpressure);
        (Producer { sink }, Reader { shared })
    }

    /// Reader half over the shared queue, exposing the same blocking pull and
    /// observability the live [`RecordBatchStream`] exposes, without owning a
    /// live client.
    pub(crate) struct Reader {
        shared: Arc<Shared>,
    }

    impl Reader {
        /// The shared queue handle, so a test outside this module can build the
        /// teardown wake hook (`shared.close_and_wake`) the columnar dispatcher
        /// installs and assert quiesce drives it.
        pub(crate) fn shared_handle(&self) -> Arc<Shared> {
            Arc::clone(&self.shared)
        }

        /// A second reader handle over the SAME shared queue, modelling two
        /// binding handles (`close()` and `next()`) racing on one core
        /// stream.
        pub(crate) fn sharing(other: &Reader) -> Reader {
            Reader {
                shared: Arc::clone(&other.shared),
            }
        }

        /// Blocking pull of the next batch; `Ok(None)` at clean end.
        pub(crate) fn next(
            &self,
        ) -> Result<Option<arrow_array::RecordBatch>, crate::streaming::StreamError> {
            let mut guard = super::lock(&self.shared.inner);
            loop {
                if let Some(batch) = guard.batches.pop_front() {
                    self.shared.cv.notify_all();
                    return Ok(Some(batch));
                }
                if let Some(err) = guard.error.take() {
                    return Err(err);
                }
                if guard.finished {
                    return Ok(None);
                }
                // Mirror `RecordBatchStream::next_blocking`: a close ends the
                // pull once the queue is drained.
                if self.shared.closed.load(Ordering::Acquire) {
                    return Ok(None);
                }
                guard = self
                    .shared
                    .cv
                    .wait(guard)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }

        /// Batches dropped so far under `DropOldest`.
        pub(crate) fn dropped(&self) -> u64 {
            self.shared.dropped.load(Ordering::Relaxed)
        }

        /// Signal close so a Block-mode producer parked on a full queue
        /// stops, mirroring [`RecordBatchStream::close`] minus the client
        /// teardown (there is no live client in the harness). Stores `closed`
        /// under `inner` so it serialises with a waiter's under-lock
        /// check-then-park, matching `RecordBatchStream::close_shared`.
        pub(crate) fn close(&self) {
            {
                let _guard = super::lock(&self.shared.inner);
                self.shared.closed.store(true, Ordering::Release);
            }
            self.shared.cv.notify_all();
        }

        /// Reader-side queue depth, for asserting the bound holds.
        pub(crate) fn queued(&self) -> usize {
            super::lock(&self.shared.inner).batches.len()
        }

        /// Run `f` while holding the `inner` queue lock, for a test that needs
        /// to prove the close path stores `closed` under this same lock. While
        /// `f` runs, a concurrent [`Self::close`] must block on its own `inner`
        /// acquisition before it can store `closed` — the structural guarantee
        /// that closes the lost-wakeup window. Keeps the private `Queue` type
        /// out of the helper's signature.
        pub(crate) fn with_inner_held<R>(&self, f: impl FnOnce() -> R) -> R {
            let _guard = super::lock(&self.shared.inner);
            f()
        }

        /// Read the `closed` flag without taking `inner`, so a test holding the
        /// `inner` guard can observe whether a concurrent `close` has managed
        /// to store it yet.
        pub(crate) fn is_closed(&self) -> bool {
            self.shared.closed.load(Ordering::Acquire)
        }
    }

    /// Synthetic stock trade event carrying a shared contract.
    pub(crate) fn trade(contract: &Arc<Contract>, idx: u64) -> StreamEvent {
        StreamEvent::Data(StreamData::Trade {
            contract: Arc::clone(contract),
            ms_of_day: (idx % 86_400_000) as i32,
            sequence: idx as i32,
            condition: 0,
            size: 100,
            exchange: 0,
            price: 150.25,
            date: 20240315,
            received_at_ns: idx,
        })
    }

    /// Synthetic stock quote event carrying a shared contract.
    pub(crate) fn quote(contract: &Arc<Contract>, idx: u64) -> StreamEvent {
        StreamEvent::Data(StreamData::Quote {
            contract: Arc::clone(contract),
            ms_of_day: (idx % 86_400_000) as i32,
            bid_size: 10,
            bid_exchange: 1,
            bid: 150.00,
            bid_condition: 0,
            ask_size: 12,
            ask_exchange: 2,
            ask: 150.50,
            ask_condition: 0,
            date: 20240315,
            received_at_ns: idx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::test_harness::{harness, quote, trade};
    use super::Backpressure;
    use crate::fpss::batch_schema::stream_batch_schema;
    use crate::fpss::protocol::Contract;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// (a) zero-drop under Block: every fed row is delivered, summed across
    /// batches, with no drops.
    #[test]
    fn block_mode_delivers_every_row() {
        let batch_size = 1_000;
        let total: u64 = 25_000;
        let (producer, reader) =
            harness(batch_size, Duration::from_millis(50), Backpressure::Block);
        let contract = Arc::new(Contract::stock("SPY"));

        // Producer on its own thread so a full queue actually blocks it,
        // exercising the Block backpressure path against a paced reader.
        let feeder = thread::spawn(move || {
            for i in 0..total {
                producer.feed(&trade(&contract, i));
            }
            producer.finish();
        });

        let mut delivered: u64 = 0;
        while let Some(batch) = reader.next().expect("no error in block mode") {
            delivered += batch.num_rows() as u64;
        }
        feeder.join().unwrap();

        assert_eq!(delivered, total, "Block mode must deliver every row");
        assert_eq!(reader.dropped(), 0, "Block mode must never drop");
    }

    /// (b) DropOldest increments `dropped()` and never blocks the producer
    /// unboundedly: a producer that floods far past the queue bound while no
    /// reader pulls still completes, the queue stays bounded, and the drop
    /// counter is non-zero.
    #[test]
    fn drop_oldest_counts_drops_and_never_blocks() {
        let batch_size = 100;
        let capacity = 2;
        let total: u64 = 50_000;
        let (producer, reader) = harness(
            batch_size,
            Duration::from_millis(1),
            Backpressure::DropOldest { capacity },
        );
        let contract = Arc::new(Contract::stock("SPY"));

        // No reader pulls during the flood. If DropOldest ever blocked, this
        // join would hang; the test's own completion is the "never blocks"
        // assertion.
        let feeder = thread::spawn(move || {
            for i in 0..total {
                producer.feed(&trade(&contract, i));
            }
            producer.finish();
        });
        feeder.join().unwrap();

        assert!(
            reader.queued() <= capacity,
            "queue must stay within the DropOldest bound (was {})",
            reader.queued()
        );
        assert!(
            reader.dropped() > 0,
            "DropOldest must count drops when the reader never pulls"
        );

        // The freshest batches remain readable after the flood.
        let mut seen = 0u64;
        while let Some(batch) = reader.next().expect("no error in drop mode") {
            seen += batch.num_rows() as u64;
        }
        assert!(seen > 0, "the newest buffered batches must remain readable");
    }

    /// (c) linger flushes a partial batch on a quiet stream: a single row,
    /// far below `batch_size`, is delivered after the linger elapses without
    /// any further events.
    #[test]
    fn linger_flushes_partial_batch_on_quiet_stream() {
        let batch_size = 10_000;
        let linger = Duration::from_millis(20);
        let (producer, reader) = harness(batch_size, linger, Backpressure::Block);
        let contract = Arc::new(Contract::stock("SPY"));

        // One row, then quiet. Drive linger ticks as the dispatcher would on
        // an idle ring until the partial batch flushes.
        producer.feed(&trade(&contract, 0));
        let reader_handle = thread::spawn(move || reader.next().expect("no error"));

        // Simulate the idle-drain scope hook firing past the linger window.
        thread::sleep(linger * 2);
        producer.tick();

        let batch = reader_handle.join().unwrap().expect("a partial batch");
        assert_eq!(
            batch.num_rows(),
            1,
            "linger must flush the single buffered row"
        );
        producer.finish();
    }

    /// (d) schema stability across batches: every emitted batch carries the
    /// exact fixed schema, regardless of which event variants it holds, so
    /// the output is concat-safe.
    #[test]
    fn schema_is_stable_across_batches() {
        let expected = stream_batch_schema();
        let batch_size = 4;
        let (producer, reader) =
            harness(batch_size, Duration::from_millis(50), Backpressure::Block);
        let contract = Arc::new(Contract::stock("SPY"));

        let feeder = thread::spawn(move || {
            // Interleave trade and quote variants across several full
            // batches so a per-variant schema would diverge if one existed.
            for i in 0..16 {
                if i % 2 == 0 {
                    producer.feed(&trade(&contract, i));
                } else {
                    producer.feed(&quote(&contract, i));
                }
            }
            producer.finish();
        });

        let mut batches = 0;
        while let Some(batch) = reader.next().expect("no error") {
            assert_eq!(
                batch.schema(),
                expected,
                "every batch must carry the identical fixed schema"
            );
            batches += 1;
        }
        feeder.join().unwrap();
        assert!(batches >= 4, "expected several full batches, got {batches}");
    }

    /// (e) lifecycle: closing the reader unblocks a producer parked on a full
    /// Block-mode queue rather than leaking it. Without the close signal the
    /// producer thread would block forever on the full queue; the join
    /// returning is the no-leak assertion.
    #[test]
    fn close_unblocks_a_parked_block_producer() {
        let batch_size = 1;
        let (producer, reader) =
            harness(batch_size, Duration::from_millis(50), Backpressure::Block);
        let contract = Arc::new(Contract::stock("SPY"));

        // Flood with no reader: fills the bounded queue, then the producer
        // parks inside `flush` waiting for a free slot.
        let feeder = thread::spawn(move || {
            for i in 0..1_000 {
                producer.feed(&trade(&contract, i));
            }
        });

        // Give the producer time to fill the queue and park.
        thread::sleep(Duration::from_millis(50));
        // Close: the parked flush re-checks `closed` and returns.
        reader.close();
        feeder
            .join()
            .expect("closing the reader must release the parked producer");
    }

    /// The empty case is valid: a stream that ends before any event yields a
    /// clean end-of-stream with no batch and no error.
    #[test]
    fn empty_stream_ends_clean() {
        let (producer, reader) = harness(1_000, Duration::from_millis(50), Backpressure::Block);
        producer.finish();
        assert!(reader.next().expect("no error").is_none());
    }

    /// (e, consumer side) close unblocks a reader parked on a blocking pull.
    /// One handle blocks on a pull of a quiet stream; a second handle sharing
    /// the same queue signals close (the binding `close()` path racing a
    /// `next()`), and the blocked pull returns end-of-stream rather than
    /// hanging. Without the `closed` check in the pull loop the reader would
    /// wait forever for a `finished` only the dispatcher sets — the deadlock
    /// this guards against.
    #[test]
    fn close_from_another_handle_unblocks_a_parked_pull() {
        use super::test_harness::Reader;
        use std::sync::atomic::{AtomicBool, Ordering as O};

        let (producer, reader_a) = harness(1_000, Duration::from_millis(50), Backpressure::Block);
        let reader_b = Reader::sharing(&reader_a);
        // Idle producer: never feeds, never finishes, so the only way the
        // pull returns is the close signal.
        let _producer = producer;

        let done = Arc::new(AtomicBool::new(false));
        let done_t = Arc::clone(&done);
        let puller = thread::spawn(move || {
            let out = reader_a.next(); // blocks until close
            done_t.store(true, O::Release);
            out
        });

        thread::sleep(Duration::from_millis(50));
        assert!(
            !done.load(O::Acquire),
            "pull must still be parked before close"
        );

        reader_b.close(); // close from the sibling handle

        let out = puller.join().expect("puller thread");
        assert!(
            out.expect("no error").is_none(),
            "a closed stream's parked pull must return end-of-stream"
        );
    }

    /// (f) the close path stores `closed` while holding `inner`.
    ///
    /// This is the invariant that closes the lost-wakeup window. A Block-mode
    /// producer in `flush` (and a blocked consumer in `next_blocking`) checks
    /// `closed` under `inner` and then parks on `cv`, which atomically releases
    /// `inner` as it parks — so the producer holds `inner` continuously from
    /// its check through the park. If `close` stores `closed` WITHOUT `inner`,
    /// the store + `notify_all` can land in the gap between the producer's
    /// check and its park; the notify then wakes no one and the producer parks
    /// forever. Storing `closed` under `inner` serialises the store with that
    /// check-then-park: it cannot land in the gap, because the producer holds
    /// the lock across the whole gap.
    ///
    /// A timing race here is vanishingly narrow (the kernel futex handoff
    /// hides it on most runs), so rather than chase it probabilistically this
    /// asserts the structural guarantee directly: while the test holds `inner`,
    /// a concurrent `close` MUST NOT be able to store `closed` — it has to
    /// block on its own `inner` acquisition first. With the pre-fix unlocked
    /// store this assertion fails immediately and deterministically, because
    /// the store does not wait for the lock.
    #[test]
    fn close_stores_closed_under_the_inner_lock() {
        use std::sync::atomic::{AtomicBool, Ordering as O};

        let (_producer, reader_a) = harness(1_000, Duration::from_millis(50), Backpressure::Block);
        let reader_b = super::test_harness::Reader::sharing(&reader_a);

        let close_returned = Arc::new(AtomicBool::new(false));
        let close_returned_t = Arc::clone(&close_returned);

        // Hold `inner` across the whole observation window so a correct `close`
        // cannot reach its store. `with_inner_held` keeps the lock for the
        // duration of the closure.
        let closer = reader_a.with_inner_held(|| {
            let closer = thread::spawn(move || {
                reader_b.close(); // blocks on `inner` until the test releases it
                close_returned_t.store(true, O::Release);
            });

            // Give the closer ample time to run. With the store under `inner`
            // it is parked on the lock the test holds, so `closed` must still
            // be false and `close` must not have returned. Pre-fix (unlocked
            // store) the closer stores `closed` immediately and this fails.
            thread::sleep(Duration::from_millis(50));
            assert!(
                !reader_a.is_closed(),
                "close must not store `closed` while another thread holds \
                 `inner`; an unlocked store opens the lost-wakeup window this \
                 guards"
            );
            assert!(
                !close_returned.load(O::Acquire),
                "close must still be blocked on `inner` while the test holds it"
            );
            closer
        });

        // `inner` released here; the closer now acquires it, stores `closed`,
        // and returns.
        closer.join().expect("closer thread");
        assert!(
            reader_a.is_closed(),
            "after the lock is released, close stores `closed`"
        );
    }

    /// (g) batch_size is clamped to `[1, MAX_BATCH_SIZE]`. The builder
    /// preallocates every column to the batch size, so an unbounded value
    /// (e.g. a binding wrapping a negative request to a near-maximum unsigned)
    /// would otherwise drive a multi-hundred-gigabyte prealloc and abort. The
    /// clamp is the single chokepoint both the builder setter and `start` use.
    #[test]
    fn batch_size_is_clamped_to_the_bound() {
        use super::{clamp_batch_size, MAX_BATCH_SIZE};
        assert_eq!(clamp_batch_size(0), 1, "zero clamps up to 1");
        assert_eq!(clamp_batch_size(1), 1);
        assert_eq!(clamp_batch_size(4_096), 4_096, "an in-range value is kept");
        assert_eq!(
            clamp_batch_size(MAX_BATCH_SIZE),
            MAX_BATCH_SIZE,
            "the bound itself is kept"
        );
        assert_eq!(
            clamp_batch_size(MAX_BATCH_SIZE + 1),
            MAX_BATCH_SIZE,
            "just past the bound clamps down"
        );
        assert_eq!(
            clamp_batch_size(usize::MAX),
            MAX_BATCH_SIZE,
            "a wrapped near-maximum value clamps to the bound, not a ruinous alloc"
        );
    }

    /// (g) the bounded-queue depth is clamped to `[1, MAX_QUEUE_DEPTH]` for
    /// both modes. `DropOldest` can hold `capacity` finished batches at once and
    /// the deque preallocates that many slots, so an unbounded `capacity` would
    /// preallocate a runaway deque; `Block` resolves to the default depth.
    #[test]
    fn capacity_is_clamped_to_the_bound() {
        use super::{Backpressure, DEFAULT_QUEUE_DEPTH, MAX_QUEUE_DEPTH};
        assert_eq!(
            Backpressure::Block.resolved_capacity(),
            DEFAULT_QUEUE_DEPTH,
            "Block uses the default depth"
        );
        assert_eq!(
            Backpressure::DropOldest { capacity: 0 }.resolved_capacity(),
            1,
            "zero capacity clamps up to 1"
        );
        assert_eq!(
            Backpressure::DropOldest { capacity: 8 }.resolved_capacity(),
            8,
            "an in-range capacity is kept"
        );
        assert_eq!(
            Backpressure::DropOldest {
                capacity: MAX_QUEUE_DEPTH
            }
            .resolved_capacity(),
            MAX_QUEUE_DEPTH,
            "the bound itself is kept"
        );
        assert_eq!(
            Backpressure::DropOldest {
                capacity: usize::MAX
            }
            .resolved_capacity(),
            MAX_QUEUE_DEPTH,
            "a wrapped near-maximum capacity clamps to the bound"
        );
    }
}
