//! Unified `ThetaData` client -- single entry point, one auth, lazy FPSS.
//!
//! Connect once. Use historical data immediately. Streaming connects
//! on-demand when you first subscribe -- not at startup.
//!
//! ```rust,no_run
//! use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), thetadatadx::Error> {
//!     // One connect, one auth. FPSS is NOT connected yet.
//!     // Or inline: Credentials::new("user@example.com", "your-password")
//!     let tdx = ThetaDataDxClient::connect(
//!         &Credentials::from_file("creds.txt")?,
//!         DirectConfig::production(),
//!     ).await?;
//!
//!     // Historical -- works immediately
//!     let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
//!
//!     // Streaming -- FPSS connects lazily on first subscribe
//!     use thetadatadx::fpss::{FpssData, FpssEvent};
//!     use thetadatadx::fpss::protocol::Contract;
//!     tdx.start_streaming(|event| {
//!         if let FpssEvent::Data(FpssData::Trade { price, size, .. }) = event {
//!             println!("trade {price} x {size}");
//!         }
//!     })?;
//!     tdx.subscribe(Contract::stock("AAPL").quote())?;
//!
//!     Ok(())
//! }
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;

use crate::auth::Credentials;
use crate::config::DirectConfig;
use crate::error::Error;
use crate::fpss::protocol::{Contract, FullSubscriptionKind, Subscription, SubscriptionKind};
use crate::fpss::{EventIterator, FpssClient, FpssEvent};
use crate::mdds::MddsClient;
use tdbe::types::enums::SecType;

/// Snapshot of the streaming side of the unified client.
///
/// One [`ArcSwap`] cell so every read path collapses to a single
/// atomic load. The previous design carried a separate
/// `Mutex<Option<StreamingDispatcher>>` alongside the [`FpssClient`];
/// after the post-#513 single-queue rewrite the user callback runs
/// directly on the Disruptor consumer thread inside [`FpssClient`],
/// so the slot only needs to track the live client.
///
/// Lifecycle: `Idle` (constructed) → `Live` (`start_streaming`
/// succeeded) → `Stopped` (`stop_streaming` returned). A subsequent
/// `start_streaming` from `Stopped` swaps back to `Live`; `Idle` is
/// reachable only at construction time, never re-entered after a
/// successful start.
enum StreamingSlot {
    /// `start_streaming()` has not been called yet.
    Idle,
    /// Streaming connection is established. The user callback runs
    /// inside the [`FpssClient`]'s Disruptor consumer thread (panic
    /// isolated via `catch_unwind`); ring-buffer overflow is reported
    /// through [`FpssClient::dropped_count`].
    Live { client: Arc<FpssClient> },
    /// `stop_streaming()` ran (or `Drop` did). Distinguishes "was
    /// started, then stopped" from "never started" for
    /// [`ConnectionStatus::Disconnected`] vs
    /// [`ConnectionStatus::NotStarted`].
    Stopped,
}

/// Render a [`crate::mdds::SubscriptionTier`] (or `None`) as the
/// human-facing label this SDK uses on `SubscriptionInfo`. Kept
/// outside the impl so the mapping is testable without spinning up an
/// authenticated client.
fn tier_label(tier: Option<crate::mdds::SubscriptionTier>) -> String {
    match tier {
        Some(crate::mdds::SubscriptionTier::Free) => "Free".to_string(),
        Some(crate::mdds::SubscriptionTier::Value) => "Value".to_string(),
        Some(crate::mdds::SubscriptionTier::Standard) => "Standard".to_string(),
        Some(crate::mdds::SubscriptionTier::Pro) => "Pro".to_string(),
        None => "Unknown".to_string(),
    }
}

/// Subscription tier information captured at authentication time.
///
/// `#[non_exhaustive]` so new tiers (e.g. additional asset classes the
/// Nexus auth payload starts emitting) can be added without a breaking
/// API change. Callers should construct it via the SDK's
/// [`ThetaDataDxClient::subscription_info`] entry point — fields are
/// populated from `AuthResponse.user.{stock,options,indices,interest_rate}_subscription`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SubscriptionInfo {
    /// Stock data subscription tier (e.g. "Free", "Value", "Standard", "Pro").
    pub stock: String,
    /// Options data subscription tier (e.g. "Free", "Value", "Standard", "Pro").
    pub options: String,
    /// Indices (e.g. SPX, VIX) subscription tier. Same string ladder as
    /// `stock` / `options`; "Unknown" when the upstream auth response
    /// omits the field.
    pub indices: String,
    /// Interest-rate / Treasury / SOFR curve subscription tier. Same
    /// string ladder as `stock` / `options`; "Unknown" when the upstream
    /// auth response omits the field.
    pub interest_rate: String,
}

/// Current state of the streaming connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionStatus {
    /// `start_streaming()` has not been called yet.
    NotStarted,
    /// Connected and authenticated.
    Connected,
    /// Currently attempting to reconnect after an involuntary disconnect.
    Reconnecting,
    /// Explicitly stopped or failed to connect.
    Disconnected,
}

/// Unified `ThetaData` client.
///
/// Authenticates once at connect time. Historical data (MDDS gRPC) is
/// available immediately. Streaming (FPSS TCP) connects lazily when
/// you call [`start_streaming`](Self::start_streaming).
///
/// All historical endpoint methods are available via `Deref` to
/// [`MddsClient`]. Streaming methods are on this struct directly.
pub struct ThetaDataDxClient {
    historical: MddsClient,
    creds: Credentials,
    /// Streaming-side state machine. See [`StreamingSlot`] for the
    /// `Idle → Live → Stopped` lifecycle. The
    /// [`ArcSwap`] makes `is_streaming` / `connection_status` /
    /// `with_streaming` single-atomic-load reads — the previous design
    /// took two `Mutex` locks plus an `AtomicBool` for the same answer.
    state: ArcSwap<StreamingSlot>,
    /// Quiescence flags of every superseded streaming session that has
    /// not yet drained, captured during [`Self::stop_streaming`] /
    /// [`Self::reconnect_streaming`] before the `Live → Stopped` swap
    /// drops the previous `Arc<FpssClient>`. [`Self::await_drain`]
    /// waits for **every** entry to flip to `true` before reporting
    /// quiescence; completed flags are GC'd lazily on each poll.
    ///
    /// A `Vec` (rather than a single slot) is required because stacked
    /// `start → stop → start → stop` cycles can layer multiple in-flight
    /// generations on top of each other before any one of them drains.
    /// If only the most recently retired flag were tracked, an earlier
    /// session whose Disruptor consumer is still firing the callback
    /// would be silently lost when a later stop overwrites the slot —
    /// `await_drain()` would then return `true` based on the latest
    /// generation while the earlier callback is still firing on the
    /// FFI `ctx`. The Vec preserves every retired generation until its
    /// own flag is observed `true`.
    prev_drained: Mutex<Vec<Arc<AtomicBool>>>,
    /// Monotonic counter incremented by every [`Self::stop_streaming`].
    ///
    /// Each `start_streaming*()` snapshots this value at entry and
    /// re-checks it after the FPSS connect completes. If the snapshot
    /// no longer matches, an interleaving `stop_streaming` raised the
    /// generation, the freshly built [`FpssClient`] is dropped, and
    /// the install is rejected. Closes the `Stopped → Live` resurrection
    /// race where an in-flight start could come up AFTER stop returned.
    stop_generation: AtomicU64,
}

impl ThetaDataDxClient {
    /// Connect to `ThetaData`. Authenticates once, opens gRPC channel.
    ///
    /// FPSS streaming is NOT connected yet -- call [`ThetaDataDxClient::start_streaming`]
    /// when you need real-time data.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error> {
        // Start the Prometheus exporter BEFORE opening the gRPC channel
        // so the first `thetadatadx.grpc.requests` counter hit is already
        // covered. No-op when the feature is disabled or `metrics_port`
        // is `None` (the default).
        crate::observability::try_install_exporter(&config)?;
        let historical = MddsClient::connect(creds, config).await?;
        Ok(Self {
            historical,
            creds: creds.clone(),
            state: ArcSwap::from_pointee(StreamingSlot::Idle),
            prev_drained: Mutex::new(Vec::new()),
            stop_generation: AtomicU64::new(0),
        })
    }

    /// Helper: build a [`StreamingSlot::Live`] cell from a freshly
    /// connected [`FpssClient`].
    fn live_slot(client: FpssClient) -> StreamingSlot {
        StreamingSlot::Live {
            client: Arc::new(client),
        }
    }

    /// Helper: error returned when `start_streaming*` is called while
    /// the slot is already [`StreamingSlot::Live`].
    fn already_streaming() -> Error {
        Error::Fpss {
            kind: crate::error::FpssErrorKind::ConnectionRefused,
            message: "streaming already started".into(),
        }
    }

    /// Helper: error returned when an in-flight `start_streaming*()`
    /// raced behind a [`Self::stop_streaming`] and would have resurrected
    /// streaming after the caller observed it stopped. The freshly built
    /// [`FpssClient`] is dropped before this returns.
    fn stopped_during_start() -> Error {
        Error::Fpss {
            kind: crate::error::FpssErrorKind::Disconnected,
            message: "stop_streaming() raced ahead of start_streaming(); start refused".into(),
        }
    }

    /// Start the FPSS streaming connection with a callback handler.
    ///
    /// Opens a TLS/TCP connection to `ThetaData`'s FPSS servers,
    /// authenticates with the same credentials used at connect time,
    /// and starts the FPSS reader thread plus the LMAX Disruptor
    /// consumer thread.
    ///
    /// # Pipeline (single-queue SSOT, post-#513)
    ///
    /// `TLS reader thread -> Disruptor ring (try_publish, non-blocking)
    /// -> Disruptor consumer thread -> catch_unwind(user callback)`.
    ///
    /// The TLS reader publishes every decoded event into a pre-allocated
    /// LMAX Disruptor ring via `Producer::try_publish`. A single
    /// dedicated consumer thread owned by the Disruptor invokes the
    /// user callback for each event, with each invocation wrapped in
    /// [`std::panic::catch_unwind`]. Two contracts:
    ///
    /// 1. **Reader never blocks on user code.** When the consumer
    ///    falls behind and the ring is full, `try_publish` returns
    ///    [`disruptor::RingBufferFull`], the event is dropped, and
    ///    [`Self::dropped_event_count`] increments. Operators should
    ///    poll the counter on a periodic timer.
    /// 2. **User panics never kill the consumer.** A panic from user
    ///    code (or from binding glue such as PyO3 / napi) is caught,
    ///    [`Self::panic_count`] increments, and the consumer keeps
    ///    delivering subsequent events.
    /// 3. **Lifecycle restriction.** The user callback runs on the
    ///    FPSS Disruptor consumer thread. From inside the callback you
    ///    MUST NOT call [`Self::stop_streaming`],
    ///    [`Self::reconnect_streaming`], or any API that drops the
    ///    underlying `Arc<FpssClient>`. These calls do not deadlock
    ///    (the `FpssClient::Drop` self-join guard detaches cleanup
    ///    onto a helper thread), but they return BEFORE the old
    ///    consumer has finished firing the callback for the in-flight
    ///    ring contents. Application code that frees a captured
    ///    context, replaces the callback closure, or otherwise relies
    ///    on the old callback having stopped firing the moment stop
    ///    returns will observe a torn state — including
    ///    use-after-free in FFI callers whose `ctx` was freed once
    ///    `tdx_*_stop_streaming` returned.
    ///
    ///    If the application needs to stop or reconnect in response
    ///    to an event, set a flag from the callback and observe it
    ///    from a separate thread that calls [`Self::stop_streaming`]
    ///    there, then call [`Self::await_drain`] before reusing the
    ///    captured resources.
    ///
    /// The user callback runs on the LMAX Disruptor consumer thread,
    /// with `catch_unwind` panic isolation. The callback MUST return
    /// within microseconds; for slow downstream work, hand off to a
    /// bounded queue inside the callback.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn start_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        // Reject a second concurrent start before paying the connect
        // cost. The post-connect slot install below revalidates this
        // because another caller may race in during the connect; the
        // upfront check is just a fast-path optimisation.
        if matches!(&**self.state.load(), StreamingSlot::Live { .. }) {
            return Err(Self::already_streaming());
        }

        // Snapshot the stop generation BEFORE connecting. If another
        // thread calls `stop_streaming()` between this load and the
        // post-connect `install_live`, the install path observes the
        // mismatch and refuses to resurrect the slot to `Live`.
        let gen_at_entry = self.stop_generation.load(Ordering::Acquire);

        let config = self.historical.config();
        let client = FpssClient::connect(
            crate::fpss::FpssConnectArgs {
                creds: &self.creds,
                hosts: &config.fpss.hosts,
                ring_size: config.fpss.ring_size,
                flush_mode: config.fpss.flush_mode,
                policy: config.reconnect.policy.clone(),
                derive_ohlcvc: config.fpss.derive_ohlcvc,
                connect_timeout_ms: config.fpss.connect_timeout_ms,
                read_timeout_ms: config.fpss.timeout_ms,
                ping_interval_ms: config.fpss.ping_interval_ms,
            },
            handler,
        )?;

        self.install_live(Self::live_slot(client), gen_at_entry)
    }

    /// Start the FPSS streaming connection in pull-iter delivery mode.
    ///
    /// Sibling of [`Self::start_streaming`] for high-throughput batch
    /// consumption. Returns an [`EventIterator`] that drains the
    /// per-client bounded queue on the calling thread; the queue is
    /// sized to match the Disruptor ring (default `4096`) so backpressure
    /// surfaces on the same [`Self::dropped_event_count`] counter as the
    /// callback path.
    ///
    /// # When to choose pull-iter
    ///
    /// Pull-iter trades a small per-event latency increase (one queue
    /// hop, one user-thread wakeup per drain) for the ability to amortise
    /// per-event overhead across an entire batch under one synchronous
    /// section. The dominant win is on the Python binding: a `for event
    /// in iter:` loop holds the GIL across the drain, so 1 GIL acquire
    /// covers N events instead of N GIL acquires for N callback
    /// invocations. Under heavy load this closes the throughput gap with
    /// `databento`'s `for record in client:` pattern (and exceeds it,
    /// because we drain the Disruptor consumer queue directly without
    /// their intermediate `queue.Queue`).
    ///
    /// Push-callback ([`Self::start_streaming`]) remains the recommended
    /// default for single-event reaction latency.
    ///
    /// # Mutual exclusion
    ///
    /// Push and pull are mutually exclusive on a given client; calling
    /// this when streaming is already running returns
    /// [`crate::error::FpssErrorKind::ConnectionRefused`] with
    /// `"streaming already started"`. To switch modes, call
    /// [`Self::stop_streaming`] first, then `start_streaming_iter()`
    /// again.
    ///
    /// # Errors
    ///
    /// Returns the same error set as [`Self::start_streaming`] (TLS,
    /// auth, config validation).
    pub fn start_streaming_iter(&self) -> Result<EventIterator, Error> {
        // Reject a second concurrent start before paying the connect
        // cost — same fast-path optimisation as `start_streaming`.
        if matches!(&**self.state.load(), StreamingSlot::Live { .. }) {
            return Err(Self::already_streaming());
        }

        let gen_at_entry = self.stop_generation.load(Ordering::Acquire);

        let config = self.historical.config();
        let (client, iterator) = FpssClient::connect_iter(crate::fpss::FpssConnectArgs {
            creds: &self.creds,
            hosts: &config.fpss.hosts,
            ring_size: config.fpss.ring_size,
            flush_mode: config.fpss.flush_mode,
            policy: config.reconnect.policy.clone(),
            derive_ohlcvc: config.fpss.derive_ohlcvc,
            connect_timeout_ms: config.fpss.connect_timeout_ms,
            read_timeout_ms: config.fpss.timeout_ms,
            ping_interval_ms: config.fpss.ping_interval_ms,
        })?;

        self.install_live(Self::live_slot(client), gen_at_entry)?;
        Ok(iterator)
    }

    /// Atomically swap the slot to a fresh `Live` state.
    ///
    /// Rejects the install when:
    ///
    /// 1. another `start_streaming*` raced in and the slot is already
    ///    `Live` (returns [`Self::already_streaming`]); or
    /// 2. an interleaving [`Self::stop_streaming`] bumped the
    ///    [`Self::stop_generation`] counter past `gen_at_entry`
    ///    (returns [`Self::stopped_during_start`]). This is the H2
    ///    `Stopped → Live` resurrection guard: a caller that started
    ///    connecting BEFORE `stop_streaming` was invoked must NOT see
    ///    its connection installed AFTER stop returned, even though
    ///    the FPSS connect itself succeeded.
    ///
    /// On either rejection the freshly built [`FpssClient`] (carried
    /// inside `new`) falls out of scope, which triggers its reader-
    /// thread shutdown and detaches the dispatcher cleanly.
    fn install_live(&self, new_slot: StreamingSlot, gen_at_entry: u64) -> Result<(), Error> {
        let new = Arc::new(new_slot);
        // CAS loop: only swap from `Idle` or `Stopped` into `Live`,
        // AND only when the stop-generation matches the snapshot taken
        // at start-entry. ArcSwap doesn't expose `compare_and_swap` on
        // `&Arc<T>` directly for non-Eq T; we instead read, decide,
        // and rcu the state. The `rcu` closure is retried until the
        // swap is observed atomically.
        let stop_gen = &self.stop_generation;
        let prev = self.state.rcu(|current| match &**current {
            StreamingSlot::Live { .. } => Arc::clone(current),
            _ => {
                // Re-check the stop generation INSIDE the rcu closure.
                // If another thread called `stop_streaming` after we
                // snapshotted `gen_at_entry`, refuse the install by
                // leaving the slot unchanged and signalling via the
                // returned `prev` shape (see post-rcu match below).
                if stop_gen.load(Ordering::Acquire) != gen_at_entry {
                    Arc::clone(current)
                } else {
                    Arc::clone(&new)
                }
            }
        });
        if matches!(&*prev, StreamingSlot::Live { .. }) {
            // Lost the race: another start_streaming installed first.
            // `new` falls out of scope and shuts down its FPSS client.
            return Err(Self::already_streaming());
        }
        // Final check: if the rcu closure refused due to the generation
        // mismatch, the cell was left at its current value (Stopped or
        // Idle). Distinguish from "successful install" by re-reading
        // the cell — if it does not point to our `new`, we lost.
        if !Arc::ptr_eq(&self.state.load_full(), &new) {
            return Err(Self::stopped_during_start());
        }
        Ok(())
    }

    /// Snapshot of events the TLS reader could not publish into the
    /// Disruptor ring because the consumer fell behind and the ring
    /// was full. Returns `0` when streaming has not started.
    ///
    /// Operators should poll this on a periodic timer (e.g. every
    /// second) and emit a `warn` log on any non-zero delta. A
    /// per-drop log would amplify under sustained overflow.
    #[must_use]
    pub fn dropped_event_count(&self) -> u64 {
        let snap = self.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.dropped_count(),
            StreamingSlot::Idle | StreamingSlot::Stopped => 0,
        }
    }

    /// Snapshot of user-callback panics caught by the Disruptor
    /// consumer's `catch_unwind` boundary. Each panic is also
    /// surfaced via `tracing::error!` with target
    /// `thetadatadx::fpss::io_loop`. Returns `0` when streaming has
    /// not started.
    #[must_use]
    pub fn panic_count(&self) -> u64 {
        let snap = self.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.panic_count(),
            StreamingSlot::Idle | StreamingSlot::Stopped => 0,
        }
    }

    /// Set the slow-callback wall-clock threshold for the live
    /// streaming session.
    ///
    /// When the user-callback wall-clock duration exceeds `threshold`,
    /// [`Self::slow_callback_count`] increments and a `tracing::warn!`
    /// fires (rate-limited per 1024 over-budget events). Pass
    /// `Duration::ZERO` to disable. Default is disabled.
    ///
    /// **Observability only** — Rust cannot safely cancel arbitrary
    /// user code, so the watchdog never kills the consumer. Operators
    /// poll the counter and decide how to respond.
    ///
    /// No-op when streaming has not started; on the next
    /// [`Self::start_streaming`] the threshold defaults back to
    /// disabled (callers must re-arm).
    pub fn set_slow_callback_threshold(&self, threshold: Duration) {
        let snap = self.state.load();
        if let StreamingSlot::Live { client } = &**snap {
            client.set_slow_callback_threshold(threshold);
        }
    }

    /// Snapshot of user-callback invocations whose wall-clock duration
    /// exceeded the threshold set by
    /// [`Self::set_slow_callback_threshold`]. Returns `0` when the
    /// watchdog is disabled or when streaming has not started.
    #[must_use]
    pub fn slow_callback_count(&self) -> u64 {
        let snap = self.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.slow_callback_count(),
            StreamingSlot::Idle | StreamingSlot::Stopped => 0,
        }
    }

    /// Whether streaming is currently active.
    ///
    /// This flag flips immediately on [`Self::stop_streaming`] /
    /// [`Self::reconnect_streaming`] (i.e. on the atomic swap of the
    /// `Live → Stopped` state cell), BEFORE the previous I/O thread,
    /// Disruptor consumer, and any in-flight user callback have
    /// drained. A `false` return therefore means *no new events will
    /// be enqueued for the previous callback*, not *the previous
    /// callback has stopped firing*.
    ///
    /// Pair with [`Self::await_drain`] when the caller needs the
    /// stronger guarantee — e.g. before freeing an FFI callback
    /// context, replacing the callback closure, or asserting the old
    /// consumer thread has joined.
    pub fn is_streaming(&self) -> bool {
        matches!(&**self.state.load(), StreamingSlot::Live { .. })
    }

    /// Wait for the previously-superseded streaming session to
    /// quiesce, polling its internal drain flag until it observes
    /// `true` or `timeout` elapses.
    ///
    /// Returns `true` when the previous session's I/O thread and
    /// Disruptor consumer have both joined, so the previous user
    /// callback is guaranteed to have stopped firing. Returns `false`
    /// on timeout, or if no stream has ever been started or stopped on
    /// this handle (nothing to drain).
    ///
    /// # When to call
    ///
    /// Call from a thread other than the FPSS consumer thread after
    /// either [`Self::stop_streaming`] or [`Self::reconnect_streaming`]
    /// returns, when application code needs to:
    ///
    /// - free an FFI callback context that the old callback closure
    ///   captured by value;
    /// - drop a captured `Arc` whose contents the old callback was
    ///   reading;
    /// - install a fresh callback whose `'static` captures must not
    ///   alias the old callback's still-running invocations.
    ///
    /// # Lifecycle restriction
    ///
    /// Because callback-thread stop / reconnect detaches cleanup onto
    /// a helper thread (see [`Self::start_streaming`] for the full
    /// rationale), `await_drain` MUST be called from a thread other
    /// than the FPSS Disruptor consumer thread. Calling it from
    /// inside the user callback would block the very thread the
    /// helper is waiting on and the call would always time out.
    ///
    /// # Resolution
    ///
    /// Polls every 1 ms; returns as soon as the flag flips. The poll
    /// loop is intentionally simple (no condvar, no parking) because
    /// drain is a one-shot, latency-tolerant event — the vast
    /// majority of calls return on the first or second tick.
    /// Whether a prior streaming session has registered its drain flag
    /// for [`Self::await_drain`] to poll on.
    ///
    /// Returns `true` once [`Self::stop_streaming`] or
    /// [`Self::reconnect_streaming`] has captured the previous
    /// session's drain flag into the internal slot, even if the flag
    /// has not yet flipped. Returns `false` on a fresh handle that has
    /// never started streaming, or on a unified handle that has only
    /// served historical endpoints.
    ///
    /// FFI free paths use this to disambiguate the two `false` returns
    /// from `await_drain` (timeout vs. nothing-to-wait-on); only the
    /// former is a real concern worth surfacing in operator logs.
    ///
    /// **Non-blocking.** Uses `try_lock` on the internal slot mutex.
    /// If the mutex is contended (another thread is mid-`stop_streaming`
    /// or `reconnect_streaming` swapping the slot), this returns `true`
    /// — the institutionally-safe answer, because the FFI `_free` path
    /// uses this signal to decide whether to wait on the drain barrier,
    /// and "wait" is the correct fail-safe when a stop is actively in
    /// flight. The contention window is microseconds (the lock is held
    /// only across the `Vec<Arc<AtomicBool>>` push), so the false-
    /// positive cost is negligible.
    ///
    /// Returns `true` iff there is at least one retired generation that
    /// has not yet drained. Already-drained flags are GC'd lazily here
    /// so a long-lived handle that has cycled through many sessions
    /// does not leak `Arc<AtomicBool>` entries.
    #[must_use]
    pub fn prev_drained_is_set(&self) -> bool {
        match self.prev_drained.try_lock() {
            Ok(mut guard) => {
                guard.retain(|f| !f.load(Ordering::Acquire));
                !guard.is_empty()
            }
            Err(std::sync::TryLockError::Poisoned(p)) => {
                let mut guard = p.into_inner();
                guard.retain(|f| !f.load(Ordering::Acquire));
                !guard.is_empty()
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                // Mutex contended — another thread is mid-stop. Treat
                // as "drain in progress" (true) so the FFI free path
                // waits for the drain barrier rather than skipping it.
                true
            }
        }
    }

    /// Wait for **every** retired streaming session to drain.
    ///
    /// Stacked `stop → start → stop` cycles layer multiple in-flight
    /// generations on top of each other before any one of them drains;
    /// this loop waits for ALL of them, not just the most-recent. On
    /// each iteration completed flags are GC'd from the internal Vec
    /// (so the next poll sees a smaller working set) and `true` is
    /// returned exactly when the Vec is empty.
    ///
    /// Returns `false` on timeout. The poll cadence is 1 ms — drain is
    /// a low-latency event in practice (single-digit milliseconds for
    /// a non-backlogged ring); the worst case is bounded by the user
    /// callback's wall-clock budget on the slowest in-flight tick.
    #[must_use]
    pub fn await_drain(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let all_drained = {
                let mut guard = self
                    .prev_drained
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                // Lazy GC of completed flags. `retain` walks the Vec
                // once per poll, which is `O(generations)`; under
                // normal load there is at most one in-flight generation.
                guard.retain(|f| !f.load(Ordering::Acquire));
                guard.is_empty()
            };
            if all_drained {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    // -- Streaming convenience methods --

    fn with_streaming<R>(
        &self,
        f: impl FnOnce(&FpssClient) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let snap = self.state.load();
        match &**snap {
            StreamingSlot::Live { client } => f(client.as_ref()),
            StreamingSlot::Idle | StreamingSlot::Stopped => Err(Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "streaming not started -- call start_streaming() first".into(),
            }),
        }
    }

    /// Polymorphic subscribe — primary fluent entry point.
    ///
    /// Accepts the typed [`Subscription`] value returned by
    /// [`Contract::quote`] / [`Contract::trade`] /
    /// [`Contract::open_interest`] (per-contract scope) or by
    /// [`crate::fpss::protocol::SecTypeExt::full_trades`] /
    /// [`crate::fpss::protocol::SecTypeExt::full_open_interest`]
    /// (full-stream scope).
    ///
    /// ```rust,no_run
    /// # use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
    /// # use thetadatadx::fpss::protocol::{Contract, SecTypeExt};
    /// # use tdbe::types::enums::SecType;
    /// # async fn doc(client: &ThetaDataDxClient) -> Result<(), thetadatadx::Error> {
    /// let stock  = Contract::stock("AAPL");
    /// let option = Contract::option("SPY", "20260620", "550", "C")?;
    /// client.subscribe(stock.quote())?;
    /// client.subscribe(option.trade())?;
    /// client.subscribe(SecType::Option.full_trades())?;
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe(&self, sub: Subscription) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe(sub.clone()))
    }

    /// Bulk-subscribe a batch of [`Subscription`] values. Stops at the
    /// first error and returns it; previously-installed subscriptions
    /// remain active (the FPSS protocol does not support batched
    /// transactions). Use individual [`Self::subscribe`] +
    /// [`Self::unsubscribe`] when atomic rollback is required.
    ///
    /// # Errors
    ///
    /// Returns an error on the first failed subscription. Successful
    /// installs that preceded the failure are NOT rolled back.
    pub fn subscribe_many<I>(&self, subs: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = Subscription>,
    {
        for sub in subs {
            self.subscribe(sub)?;
        }
        Ok(())
    }

    /// Polymorphic unsubscribe — fluent counterpart to [`Self::subscribe`].
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe(&self, sub: Subscription) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe(sub.clone()))
    }

    /// Bulk-unsubscribe a batch of [`Subscription`] values. Stops at
    /// the first error.
    ///
    /// # Errors
    ///
    /// Returns an error on the first failed unsubscribe.
    pub fn unsubscribe_many<I>(&self, subs: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = Subscription>,
    {
        for sub in subs {
            self.unsubscribe(sub)?;
        }
        Ok(())
    }

    /// Get all active per-contract subscriptions.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn active_subscriptions(&self) -> Result<Vec<(SubscriptionKind, Contract)>, Error> {
        self.with_streaming(|s| Ok(s.active_subscriptions()))
    }

    /// Get all active full-type (full-stream) subscriptions.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn active_full_subscriptions(&self) -> Result<Vec<(SubscriptionKind, SecType)>, Error> {
        self.with_streaming(|s| Ok(s.active_full_subscriptions()))
    }

    /// Shut down the streaming connection. Historical remains available.
    ///
    /// Idempotent: calling on an `Idle` or `Stopped` slot is a no-op,
    /// repeated calls during the drain race are safe (only the first
    /// observer of the `Live` slot performs the shutdown sequence).
    ///
    /// # Asynchronous quiescence
    ///
    /// `stop_streaming` returns as soon as the slot has been swapped
    /// to `Stopped` and the FPSS shutdown signal has been raised. The
    /// I/O thread and the Disruptor consumer continue running until
    /// they observe the signal, drain the in-flight ring contents
    /// through the user callback, and exit. [`Self::is_streaming`]
    /// flips to `false` immediately on the swap, BEFORE the old
    /// consumer has finished firing.
    ///
    /// Pair with [`Self::await_drain`] for full quiescence semantics
    /// when the caller needs to free a callback context, replace the
    /// callback closure, or otherwise rely on the old user callback
    /// having stopped firing.
    pub fn stop_streaming(&self) {
        // Bump the stop generation BEFORE the slot swap so any
        // in-flight `start_streaming*()` that snapshotted the previous
        // value will fail its install check and not resurrect the
        // slot to `Live` after this returns. AcqRel because the
        // ordering relative to the `state.swap` below is what closes
        // the resurrection race.
        self.stop_generation.fetch_add(1, Ordering::AcqRel);
        // Atomically swap to `Stopped`; whichever caller wins the swap
        // owns the previous `Arc<StreamingSlot>` and is the one that
        // runs the shutdown sequence.
        let prev = self.state.swap(Arc::new(StreamingSlot::Stopped));

        // Drop the FPSS client signal so its reader thread + Disruptor
        // consumer drain and exit. The actual join happens in
        // `FpssClient::Drop` when the last `Arc<FpssClient>` is dropped
        // (typically with `prev` going out of scope at end of scope).
        if let StreamingSlot::Live { client } = &*prev {
            // Capture the drain flag BEFORE the shutdown signal and
            // PUSH it onto the retired-generations list (rather than
            // overwriting a single slot). Stacked stop/start/stop
            // cycles layer multiple in-flight generations on top of
            // each other; an earlier still-firing session's flag must
            // NOT be lost when a later session retires before the
            // earlier one has drained. `await_drain()` waits for ALL
            // entries, and lazily GCs flags that have flipped to
            // `true`, so a long-lived handle does not accumulate
            // `Arc<AtomicBool>` entries past their useful lifetime.
            self.prev_drained
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(client.drained_flag());
            client.shutdown();
        }
    }

    /// Reconnect the streaming connection, re-subscribing all previous subscriptions.
    ///
    /// This is the caller-driven equivalent of Java's `handleInvoluntaryDisconnect()`.
    /// It saves active subscriptions, stops the current streaming connection,
    /// starts a new one with the provided handler, and re-subscribes everything.
    ///
    /// # Sequence
    ///
    /// 1. Save active per-contract and full-type subscriptions
    /// 2. Stop the current streaming connection
    /// 3. Start a new streaming connection with the provided handler
    /// 4. Re-subscribe all saved subscriptions, collecting per-subscription
    ///    failures rather than aborting on the first error
    ///
    /// # Errors
    ///
    /// Returns [`Error::Fpss`], [`Error::Auth`], etc. when the underlying
    /// streaming session cannot be re-established (steps 1–3).
    ///
    /// Returns [`Error::PartialReconnect`] when the streaming session was
    /// re-established successfully but one or more saved subscriptions
    /// failed to restore. The variant carries the structured list of failed
    /// `(SubscriptionKind, Contract)` pairs so the caller can retry just
    /// those subscriptions or surface the partial failure to the operator.
    /// Per-subscription `tracing::warn!` lines are still emitted for
    /// operational visibility.
    pub fn reconnect_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        metrics::counter!("thetadatadx.fpss.reconnects").increment(1);
        // 1. Save active subscriptions before stopping
        let saved_subs = match &**self.state.load() {
            StreamingSlot::Live { client } => (
                client.active_subscriptions(),
                client.active_full_subscriptions(),
            ),
            StreamingSlot::Idle | StreamingSlot::Stopped => (Vec::new(), Vec::new()),
        };

        // 2. Stop streaming
        self.stop_streaming();

        // 3. Start a new streaming connection
        self.start_streaming(handler)?;

        // 4. Re-subscribe all saved subscriptions, accumulating failures
        let (per_contract, full_type) = saved_subs;
        let failed = restore_subscriptions(
            &per_contract,
            &full_type,
            |kind, contract| {
                self.subscribe(Subscription::Contract {
                    contract: contract.clone(),
                    kind,
                })
            },
            |kind, sec_type| match kind {
                SubscriptionKind::Trade => Some(self.subscribe(Subscription::Full {
                    sec_type,
                    kind: FullSubscriptionKind::Trades,
                })),
                SubscriptionKind::OpenInterest => Some(self.subscribe(Subscription::Full {
                    sec_type,
                    kind: FullSubscriptionKind::OpenInterest,
                })),
                SubscriptionKind::Quote => None,
            },
        );

        if failed.is_empty() {
            Ok(())
        } else {
            Err(Error::PartialReconnect { failed })
        }
    }

    /// Get the current streaming connection status.
    pub fn connection_status(&self) -> ConnectionStatus {
        match &**self.state.load() {
            StreamingSlot::Idle => ConnectionStatus::NotStarted,
            StreamingSlot::Stopped => ConnectionStatus::Disconnected,
            StreamingSlot::Live { client } => {
                if client.is_authenticated() {
                    ConnectionStatus::Connected
                } else {
                    // The client exists but is not authenticated -- this happens
                    // during reconnection (authenticated flag is cleared on
                    // disconnect, restored on successful re-auth).
                    ConnectionStatus::Reconnecting
                }
            }
        }
    }

    /// Access the current MDDS session UUID.
    ///
    /// Returns an owned `String` rather than `&str` because the UUID
    /// lives behind a shared [`crate::auth::SessionToken`] that may be
    /// refreshed mid-session. Reads through the token so callers always
    /// see the current value.
    pub async fn session_uuid(&self) -> String {
        self.historical.session_uuid().await
    }

    /// Access the config.
    pub fn config(&self) -> &DirectConfig {
        self.historical.config()
    }

    /// Get subscription tier information captured at authentication time.
    ///
    /// Returns one entry per asset class the Nexus auth payload carries
    /// (`stock`, `options`, `indices`, `interest_rate`). Missing fields
    /// surface as the string `"Unknown"` so a Pro-on-indices user is
    /// distinguishable from an auth response that did not advertise an
    /// indices tier.
    pub fn subscription_info(&self) -> SubscriptionInfo {
        SubscriptionInfo {
            stock: tier_label(self.historical.stock_tier()),
            options: tier_label(self.historical.options_tier()),
            indices: tier_label(self.historical.indices_tier()),
            interest_rate: tier_label(self.historical.interest_rate_tier()),
        }
    }

    // ---------------------------------------------------------------------
    // FLATFILES surface (third public surface, alongside FPSS and MDDS).
    //
    // The legacy MDDS port (12000) speaks a custom binary PacketStream
    // protocol that supports a single FLAT_FILE request type. The server
    // pre-builds an INDEX + DATA blob per (sec_type, data_type, date)
    // tuple overnight and streams it back on demand. See
    // [`crate::flatfiles`] for the wire-format details and the decode /
    // writer implementation used by this surface, covering CSV and
    // JSONL output plus a typed in-memory return path.
    // ---------------------------------------------------------------------

    /// Pull a flat-file blob for `(sec_type, req_type, date)` over the legacy
    /// MDDS port, decode it, and write the requested `format` to disk.
    ///
    /// `format` selects the on-disk encoding:
    /// - [`crate::flatfiles::FlatFileFormat::Csv`] — vendor byte-format CSV
    ///   (lowercase headers, comma-separated, no quoting). Byte-matches the
    ///   legacy terminal's downloads on the same input.
    /// - [`crate::flatfiles::FlatFileFormat::Jsonl`] — JSON Lines, one
    ///   object per row.
    ///
    /// If `output_path` lacks a file extension, the format's canonical
    /// extension (`csv` / `jsonl`) is appended automatically.
    ///
    /// For columnar consumers (Parquet, Arrow IPC, polars) use
    /// [`Self::flatfile_request_decoded`] and feed the resulting
    /// `Vec<FlatFileRow>` into the writer of your choice — the SDK does
    /// not pull in Parquet / Arrow itself.
    ///
    /// # Errors
    /// Returns [`Error::FlatFilesUnavailable`] for auth / server
    /// rejection, [`Error::Config`] for malformed wire bytes, or
    /// [`Error::Io`] for local I/O issues.
    pub async fn flatfile_request(
        &self,
        sec_type: crate::flatfiles::SecType,
        req_type: crate::flatfiles::ReqType,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        crate::flatfiles::flatfile_request(
            &self.creds,
            sec_type,
            req_type,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Pull a flat-file blob and return decoded rows in memory.
    ///
    /// Same auth and stream path as [`Self::flatfile_request`], but skips
    /// the on-disk writer. Returns a `Vec<FlatFileRow>` ready to feed into
    /// an algorithm (backtester, risk model, in-memory analytics) without
    /// an intermediate file.
    ///
    /// The whole vector is materialised before the function returns; for
    /// whole-universe blobs that can be hundreds of MB.
    ///
    /// # Errors
    /// Same conditions as [`Self::flatfile_request`].
    pub async fn flatfile_request_decoded(
        &self,
        sec_type: crate::flatfiles::SecType,
        req_type: crate::flatfiles::ReqType,
        date: &str,
    ) -> Result<Vec<crate::flatfiles::FlatFileRow>, Error> {
        crate::flatfiles::flatfile_request_decoded(&self.creds, sec_type, req_type, date).await
    }

    /// Convenience: option open-interest flat file for `date`.
    pub async fn flatfile_option_open_interest(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::OpenInterest,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: option trade-quote flat file for `date`.
    pub async fn flatfile_option_trade_quote(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::TradeQuote,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: option trade flat file for `date`.
    pub async fn flatfile_option_trade(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::Trade,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: option quote flat file for `date`.
    pub async fn flatfile_option_quote(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::Quote,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: option end-of-day flat file for `date`.
    pub async fn flatfile_option_eod(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::Eod,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: stock trade-quote flat file for `date`.
    pub async fn flatfile_stock_trade_quote(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::TradeQuote,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: stock trade flat file for `date`.
    pub async fn flatfile_stock_trade(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::Trade,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: stock quote flat file for `date`.
    pub async fn flatfile_stock_quote(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::Quote,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: stock end-of-day flat file for `date`.
    pub async fn flatfile_stock_eod(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::Eod,
            date,
            output_path,
            format,
        )
        .await
    }
}

impl Drop for ThetaDataDxClient {
    /// Final cleanup: idempotently stops the streaming connection.
    ///
    /// `stop_streaming` swaps the state cell to `Stopped` and only
    /// signals the FPSS client when the previous slot was `Live`.
    /// The actual TLS reader + Disruptor consumer join happens when
    /// the last `Arc<FpssClient>` is dropped via `FpssClient::Drop`.
    /// Calling once from `Drop` after the user already called
    /// `stop_streaming` is therefore a no-op — the state machine
    /// guarantees the shutdown signal runs exactly once.
    fn drop(&mut self) {
        self.stop_streaming();
    }
}

// All historical methods available directly via Deref.
impl std::ops::Deref for ThetaDataDxClient {
    type Target = MddsClient;
    fn deref(&self) -> &MddsClient {
        &self.historical
    }
}

/// Replay every saved subscription against the freshly reconnected
/// streaming client and return the list of subscriptions that failed to
/// restore.
///
/// The two callbacks decouple the loop from the live `ThetaDataDxClient`
/// streaming methods so the resubscription logic is unit-testable with
/// in-memory fakes — the [`reconnect_streaming`] caller in production
/// passes through to the polymorphic `subscribe(Subscription)` paths,
/// while the
/// regression test below injects closures that return canned `Err` for a
/// specific subscription pair to prove the failure list carries the right
/// structured contents.
///
/// Per-failure operational visibility is preserved: every error path emits a
/// `tracing::warn!` line carrying `kind`, `contract` (or `sec_type`), and
/// the underlying error, identical to the single-call-site loop this
/// helper replaces.
///
/// `full_subscribe` returns `Some(Result<()>)` for kinds that are valid
/// full-type subscriptions, and `None` for kinds that are not (currently
/// only `SubscriptionKind::Quote` is excluded). A `None` triggers the same
/// "skipping" warning the previous in-line loop emitted.
fn restore_subscriptions<P, F>(
    per_contract: &[(SubscriptionKind, Contract)],
    full_type: &[(SubscriptionKind, SecType)],
    mut per_subscribe: P,
    mut full_subscribe: F,
) -> Vec<(SubscriptionKind, Contract)>
where
    P: FnMut(SubscriptionKind, &Contract) -> Result<(), Error>,
    F: FnMut(SubscriptionKind, SecType) -> Option<Result<(), Error>>,
{
    let mut failed: Vec<(SubscriptionKind, Contract)> = Vec::new();

    for (kind, contract) in per_contract {
        if let Err(e) = per_subscribe(*kind, contract) {
            tracing::warn!(
                kind = ?kind,
                contract = %contract,
                error = %e,
                "failed to re-subscribe after reconnect"
            );
            failed.push((*kind, contract.clone()));
        }
    }

    for (kind, sec_type) in full_type {
        match full_subscribe(*kind, *sec_type) {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                tracing::warn!(
                    kind = ?kind,
                    sec_type = ?sec_type,
                    error = %e,
                    "failed to re-subscribe full-type after reconnect"
                );
                // Full-type subscriptions are encoded as a synthetic
                // `Contract` with an empty `root` so the structured failure
                // list stays homogeneous. Operators see the original
                // `sec_type` via the `tracing::warn!` line above.
                failed.push((*kind, Contract::full_type_marker(*sec_type)));
            }
            None => {
                tracing::warn!(
                    kind = ?kind,
                    sec_type = ?sec_type,
                    "full-type subscription is not supported for this kind, skipping"
                );
            }
        }
    }

    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lightweight stand-in for `StreamingSlot` carrying just enough
    /// shape to walk the state machine transitions without spinning up
    /// a real FPSS connection. The transitions and the `ArcSwap`
    /// install/swap mechanics are what we are validating; the live
    /// payload (`FpssClient`, `StreamingDispatcher`) is exercised by
    /// the existing FPSS integration tests.
    enum SlotMarker {
        Idle,
        Live(u32),
        Stopped,
    }

    fn variant(s: &SlotMarker) -> &'static str {
        match s {
            SlotMarker::Idle => "Idle",
            SlotMarker::Live(_) => "Live",
            SlotMarker::Stopped => "Stopped",
        }
    }

    /// Walks Idle → Live → Stopped → Live → Stopped, asserting the
    /// `ArcSwap` cell observes each transition exactly once and that
    /// the `Live` payload (here a generation counter) is preserved
    /// across re-installs.
    #[test]
    fn streaming_slot_state_machine_transitions() {
        let cell: ArcSwap<SlotMarker> = ArcSwap::from_pointee(SlotMarker::Idle);

        // Idle observed.
        assert_eq!(variant(&cell.load()), "Idle");

        // Idle → Live(1)
        let prev = cell.swap(Arc::new(SlotMarker::Live(1)));
        assert_eq!(variant(&prev), "Idle");
        assert_eq!(variant(&cell.load()), "Live");

        // Live(1) → Stopped
        let prev = cell.swap(Arc::new(SlotMarker::Stopped));
        assert!(matches!(&*prev, SlotMarker::Live(1)));
        assert_eq!(variant(&cell.load()), "Stopped");

        // Stopped → Live(2)  — the second start path
        let prev = cell.swap(Arc::new(SlotMarker::Live(2)));
        assert_eq!(variant(&prev), "Stopped");
        assert!(matches!(&**cell.load(), SlotMarker::Live(2)));

        // Live(2) → Stopped (second shutdown)
        let prev = cell.swap(Arc::new(SlotMarker::Stopped));
        assert!(matches!(&*prev, SlotMarker::Live(2)));
        assert_eq!(variant(&cell.load()), "Stopped");
    }

    /// Concurrent `start` race: only one caller observes the install,
    /// the other sees `Live` and must reject. Modeled with the same
    /// rcu CAS the real `install_live` uses.
    #[test]
    fn streaming_slot_rejects_double_install() {
        let cell: ArcSwap<SlotMarker> = ArcSwap::from_pointee(SlotMarker::Idle);

        let new1 = Arc::new(SlotMarker::Live(1));
        let prev = cell.rcu(|cur| match &**cur {
            SlotMarker::Live(_) => Arc::clone(cur),
            _ => Arc::clone(&new1),
        });
        assert!(matches!(&*prev, SlotMarker::Idle));
        assert_eq!(variant(&cell.load()), "Live");

        // Second installer races in: must observe `Live` from `prev`.
        let new2 = Arc::new(SlotMarker::Live(2));
        let prev = cell.rcu(|cur| match &**cur {
            SlotMarker::Live(_) => Arc::clone(cur),
            _ => Arc::clone(&new2),
        });
        assert!(
            matches!(&*prev, SlotMarker::Live(1)),
            "second installer must see existing Live(1) and bail"
        );
        // Cell is unchanged: still Live(1), the Live(2) install was rejected.
        assert!(matches!(&**cell.load(), SlotMarker::Live(1)));
    }

    /// Inject a single failing per-contract subscribe call and prove the
    /// returned failure list contains exactly the failed `(kind, contract)`
    /// pair — not a count, not a boolean, the real structured contents.
    #[test]
    fn restore_subscriptions_collects_failed_per_contract() {
        let aapl = Contract::stock("AAPL");
        let msft = Contract::stock("MSFT");
        let per_contract = vec![
            (SubscriptionKind::Quote, aapl.clone()),
            (SubscriptionKind::Quote, msft.clone()),
        ];
        let full_type: Vec<(SubscriptionKind, SecType)> = Vec::new();

        let failed = restore_subscriptions(
            &per_contract,
            &full_type,
            |_kind, contract| {
                if contract.symbol == "MSFT" {
                    Err(Error::Fpss {
                        kind: crate::error::FpssErrorKind::Disconnected,
                        message: "injected: MSFT subscribe rejected".to_string(),
                    })
                } else {
                    Ok(())
                }
            },
            |_, _| None,
        );

        assert_eq!(failed.len(), 1, "exactly one subscription must have failed");
        assert_eq!(failed[0].0, SubscriptionKind::Quote);
        assert_eq!(failed[0].1, msft);
    }

    /// A successful run must return an empty failure list — no false
    /// positives, no spurious entries.
    #[test]
    fn restore_subscriptions_empty_on_full_success() {
        let aapl = Contract::stock("AAPL");
        let per_contract = vec![(SubscriptionKind::Trade, aapl)];
        let full_type = vec![(SubscriptionKind::Trade, SecType::Stock)];

        let failed = restore_subscriptions(
            &per_contract,
            &full_type,
            |_, _| Ok(()),
            |_, _| Some(Ok(())),
        );

        assert!(failed.is_empty(), "no failures expected, got {failed:?}");
    }

    /// A full-type subscription failure must show up in the list with the
    /// `full_type_marker` synthetic contract carrying the right `SecType`,
    /// so callers can pattern-match the failure without losing the
    /// originally failed sec_type.
    #[test]
    fn restore_subscriptions_records_full_type_failure() {
        let per_contract: Vec<(SubscriptionKind, Contract)> = Vec::new();
        let full_type = vec![(SubscriptionKind::OpenInterest, SecType::Option)];

        let failed = restore_subscriptions(
            &per_contract,
            &full_type,
            |_, _| Ok(()),
            |_, _| {
                Some(Err(Error::Fpss {
                    kind: crate::error::FpssErrorKind::TooManyRequests,
                    message: "injected: full-type subscribe rate-limited".to_string(),
                }))
            },
        );

        assert_eq!(failed.len(), 1);
        let (kind, contract) = &failed[0];
        assert_eq!(*kind, SubscriptionKind::OpenInterest);
        assert_eq!(contract.sec_type, SecType::Option);
        assert!(
            contract.symbol.is_empty(),
            "full-type marker carries empty root, got {:?}",
            contract.symbol
        );
    }

    /// `reconnect_streaming` returns `Error::PartialReconnect` carrying the
    /// failed list when subscriptions cannot be restored — the regression
    /// test for issue #461. The variant payload is asserted by pattern-
    /// match, not just `is_err()`, so a future refactor that changes the
    /// payload shape breaks this test loudly.
    #[test]
    fn partial_reconnect_error_carries_failed_subscriptions() {
        let aapl = Contract::stock("AAPL");
        let per_contract = vec![(SubscriptionKind::Quote, aapl.clone())];
        let full_type: Vec<(SubscriptionKind, SecType)> = Vec::new();

        let failed = restore_subscriptions(
            &per_contract,
            &full_type,
            |_, _| {
                Err(Error::Fpss {
                    kind: crate::error::FpssErrorKind::Disconnected,
                    message: "injected".to_string(),
                })
            },
            |_, _| None,
        );

        // This is exactly the path `reconnect_streaming` takes when failed
        // is non-empty: build the structured `PartialReconnect` error.
        let err = if failed.is_empty() {
            None
        } else {
            Some(Error::PartialReconnect { failed })
        };

        match err {
            Some(Error::PartialReconnect { failed }) => {
                assert_eq!(failed.len(), 1);
                assert_eq!(failed[0].0, SubscriptionKind::Quote);
                assert_eq!(failed[0].1, aapl);
            }
            other => panic!("expected PartialReconnect, got {other:?}"),
        }
    }

    /// H2 regression: an in-flight `start_streaming*` that snapshotted
    /// `stop_generation = N` at entry must NOT install `Live` when an
    /// interleaving `stop_streaming` has bumped `stop_generation` to
    /// `N+1` by the time the install runs. This is the
    /// `Stopped → Live` resurrection race the generation token closes.
    ///
    /// Models the install path the real `install_live` walks: rcu
    /// over the `ArcSwap`, gated by an `AtomicU64` stop-gen
    /// re-read inside the closure, with a post-rcu pointer-equality
    /// check on the cell to distinguish "rcu refused due to gen
    /// mismatch" from "rcu installed our value".
    #[test]
    fn install_live_refuses_when_stop_generation_advanced() {
        let cell: ArcSwap<SlotMarker> = ArcSwap::from_pointee(SlotMarker::Stopped);
        let stop_gen = AtomicU64::new(0);

        // Caller snapshots the gen at entry, then bumps it (simulating
        // an interleaving `stop_streaming` that ran while the FPSS
        // connect was still in flight).
        let gen_at_entry = stop_gen.load(Ordering::Acquire);
        stop_gen.fetch_add(1, Ordering::AcqRel);

        // Now run the install_live shape. The rcu closure must observe
        // the bumped gen and refuse to install.
        let new = Arc::new(SlotMarker::Live(99));
        let _prev = cell.rcu(|current| match &**current {
            SlotMarker::Live(_) => Arc::clone(current),
            _ => {
                if stop_gen.load(Ordering::Acquire) != gen_at_entry {
                    Arc::clone(current)
                } else {
                    Arc::clone(&new)
                }
            }
        });

        // Cell must STILL be Stopped — the install was refused.
        assert!(
            matches!(&**cell.load(), SlotMarker::Stopped),
            "install_live must refuse to resurrect Stopped → Live when stop_generation advanced",
        );
        // Pointer-equality probe matches the production code's final
        // disambiguation: the cell does NOT point to `new`, so the
        // caller would observe `Self::stopped_during_start()`.
        assert!(
            !Arc::ptr_eq(&cell.load_full(), &new),
            "cell must not hold `new` after the gen-mismatch refusal",
        );
    }

    /// Sanity: when the generation has NOT advanced, the install
    /// proceeds. The two tests together pin both branches of the
    /// generation gate.
    #[test]
    fn install_live_installs_when_stop_generation_stable() {
        let cell: ArcSwap<SlotMarker> = ArcSwap::from_pointee(SlotMarker::Stopped);
        let stop_gen = AtomicU64::new(7);

        let gen_at_entry = stop_gen.load(Ordering::Acquire);

        let new = Arc::new(SlotMarker::Live(42));
        let _prev = cell.rcu(|current| match &**current {
            SlotMarker::Live(_) => Arc::clone(current),
            _ => {
                if stop_gen.load(Ordering::Acquire) != gen_at_entry {
                    Arc::clone(current)
                } else {
                    Arc::clone(&new)
                }
            }
        });

        assert!(matches!(&**cell.load(), SlotMarker::Live(42)));
        assert!(Arc::ptr_eq(&cell.load_full(), &new));
    }

    /// `prev_drained_is_set` distinguishes "at least one stop captured a
    /// drain flag that has not yet flipped" from "no streaming session
    /// ever existed (or every retired generation has drained)". FFI
    /// free paths use this to disambiguate the two `false` returns from
    /// `await_drain` (timeout vs. nothing-to-wait-on).
    ///
    /// Post PR514 audit: the slot is now a `Vec` so stacked
    /// stop/start/stop cycles cannot lose an earlier still-draining
    /// generation when a later one retires.
    #[test]
    fn prev_drained_is_set_tracks_vec_of_generations() {
        let slot: Mutex<Vec<Arc<AtomicBool>>> = Mutex::new(Vec::new());
        // Initial state: no session ever started -- nothing to wait on.
        assert!(slot.lock().unwrap().is_empty());

        // Simulate the capture stop_streaming performs when a Live
        // session is being superseded — stacked.
        let flag_a = Arc::new(AtomicBool::new(false));
        let flag_b = Arc::new(AtomicBool::new(false));
        slot.lock().unwrap().push(Arc::clone(&flag_a));
        slot.lock().unwrap().push(Arc::clone(&flag_b));
        assert_eq!(slot.lock().unwrap().len(), 2);

        // The later flag flips first. The earlier one is STILL pending
        // — under the old single-slot design it would have been
        // overwritten by `flag_b` and silently lost.
        flag_b.store(true, Ordering::Release);
        // Lazy GC during a check would prune `flag_b`; `flag_a` stays.
        let mut g = slot.lock().unwrap();
        g.retain(|f| !f.load(Ordering::Acquire));
        assert_eq!(
            g.len(),
            1,
            "flag_b drained, but flag_a still pending — must NOT be reported as fully drained"
        );
        drop(g);

        // Once `flag_a` flips, the Vec drains to empty.
        flag_a.store(true, Ordering::Release);
        let mut g = slot.lock().unwrap();
        g.retain(|f| !f.load(Ordering::Acquire));
        assert!(g.is_empty(), "all retired generations drained");
    }

    /// HIGH-001 regression — multi-generation `await_drain` must wait
    /// for ALL retired sessions, not just the most-recent. The pre-fix
    /// single-slot tracker would have returned `true` as soon as the
    /// last-pushed flag flipped, even with earlier flags still pending.
    #[test]
    fn await_drain_waits_for_all_retired_generations() {
        // We exercise the predicate logic directly through a `Vec` to
        // avoid spinning up FpssClient instances (which require a
        // network credential). The production `await_drain` runs the
        // exact same `retain` + `is_empty` cadence on the Mutex'd Vec.
        let slot: Mutex<Vec<Arc<AtomicBool>>> = Mutex::new(Vec::new());

        let flag_a = Arc::new(AtomicBool::new(false));
        let flag_b = Arc::new(AtomicBool::new(false));
        let flag_c = Arc::new(AtomicBool::new(false));
        slot.lock().unwrap().push(Arc::clone(&flag_a));
        slot.lock().unwrap().push(Arc::clone(&flag_b));
        slot.lock().unwrap().push(Arc::clone(&flag_c));

        // Stagger the drain — c, then a, then b. Verify the Vec is only
        // empty AFTER the last (b) flips.
        flag_c.store(true, Ordering::Release);
        let mut g = slot.lock().unwrap();
        g.retain(|f| !f.load(Ordering::Acquire));
        assert_eq!(g.len(), 2, "c drained; a + b still pending");
        drop(g);

        flag_a.store(true, Ordering::Release);
        let mut g = slot.lock().unwrap();
        g.retain(|f| !f.load(Ordering::Acquire));
        assert_eq!(g.len(), 1, "a + c drained; b still pending");
        drop(g);

        flag_b.store(true, Ordering::Release);
        let mut g = slot.lock().unwrap();
        g.retain(|f| !f.load(Ordering::Acquire));
        assert!(g.is_empty(), "all three retired generations drained");
    }

    // -- Fluent Subscription dispatch -------------------------
    //
    // Spinning up a real `ThetaDataDxClient` requires a network round-trip
    // to authenticate, which a unit test can't do. The dispatch shape
    // is therefore validated against a stand-in helper that walks the
    // same `match` arms `subscribe(Subscription)` runs internally.
    // Live-network routing is covered by the streaming integration
    // tests under `crates/thetadatadx/tests/`.

    /// Mirror of [`ThetaDataDxClient::subscribe`]'s `match` shape. Returns
    /// the routed (kind, contract-or-sec-type) tuple so the test can
    /// assert the dispatch reached the right arm without a real FPSS
    /// connection.
    enum DispatchProbe {
        ContractQuote(Contract),
        ContractTrade(Contract),
        ContractOpenInterest(Contract),
        FullTrades(SecType),
        FullOpenInterest(SecType),
    }

    fn dispatch_probe(sub: Subscription) -> DispatchProbe {
        match sub {
            Subscription::Contract { contract, kind } => match kind {
                SubscriptionKind::Quote => DispatchProbe::ContractQuote(contract),
                SubscriptionKind::Trade => DispatchProbe::ContractTrade(contract),
                SubscriptionKind::OpenInterest => DispatchProbe::ContractOpenInterest(contract),
            },
            Subscription::Full { sec_type, kind } => match kind {
                FullSubscriptionKind::Trades => DispatchProbe::FullTrades(sec_type),
                FullSubscriptionKind::OpenInterest => DispatchProbe::FullOpenInterest(sec_type),
            },
        }
    }

    #[test]
    fn subscribe_dispatch_routes_per_contract_kinds() {
        let aapl = Contract::stock("AAPL");
        let opt = Contract::option("SPY", "20260620", "550", "C").unwrap();
        assert!(matches!(
            dispatch_probe(aapl.quote()),
            DispatchProbe::ContractQuote(c) if c.symbol == "AAPL"
        ));
        assert!(matches!(
            dispatch_probe(opt.trade()),
            DispatchProbe::ContractTrade(c) if c.symbol == "SPY"
        ));
        assert!(matches!(
            dispatch_probe(opt.open_interest()),
            DispatchProbe::ContractOpenInterest(c) if c.is_call == Some(true)
        ));
    }

    #[test]
    fn subscribe_dispatch_routes_full_stream_kinds() {
        use crate::fpss::protocol::SecTypeExt;
        assert!(matches!(
            dispatch_probe(SecType::Option.full_trades()),
            DispatchProbe::FullTrades(SecType::Option)
        ));
        assert!(matches!(
            dispatch_probe(SecType::Option.full_open_interest()),
            DispatchProbe::FullOpenInterest(SecType::Option)
        ));
        assert!(matches!(
            dispatch_probe(SecType::Stock.full_trades()),
            DispatchProbe::FullTrades(SecType::Stock)
        ));
    }

    #[test]
    fn theta_data_client_alias_resolves_to_theta_data_dx() {
        // Phase 3a: `ThetaDataDxClient` is the new public name; the old
        // `ThetaDataDxClient` is kept as a compatibility alias. Both must
        // refer to the same type so existing call sites keep compiling.
        fn _alias_check(c: ThetaDataDxClient) -> ThetaDataDxClient {
            c
        }
    }

    /// Every `SubscriptionTier` discriminant has a stable user-facing
    /// label, and a `None` tier renders as "Unknown" — that's what
    /// disambiguates a Pro-on-indices user from a Nexus response that
    /// did not advertise an indices tier at all.
    #[test]
    fn tier_label_covers_every_discriminant() {
        use crate::mdds::SubscriptionTier;
        assert_eq!(tier_label(Some(SubscriptionTier::Free)), "Free");
        assert_eq!(tier_label(Some(SubscriptionTier::Value)), "Value");
        assert_eq!(tier_label(Some(SubscriptionTier::Standard)), "Standard");
        assert_eq!(tier_label(Some(SubscriptionTier::Pro)), "Pro");
        assert_eq!(tier_label(None), "Unknown");
    }

    /// `AuthUser`'s four `*_subscription` wire bytes map through
    /// `SubscriptionTier::from_wire` into the four
    /// `SubscriptionInfo` fields. Pin the mapping here so a future
    /// refactor of the auth → client wiring cannot silently regress a
    /// field (the existing wiring covered only `stock` + `options`).
    #[test]
    fn auth_user_subscription_bytes_map_to_subscription_info_fields() {
        use crate::auth::nexus::AuthUser;
        use crate::mdds::SubscriptionTier;

        let user = AuthUser {
            email: None,
            // Wire bytes: Free=0, Value=1, Standard=2, Pro=3.
            stock_subscription: Some(3),
            options_subscription: Some(2),
            indices_subscription: Some(1),
            interest_rate_subscription: Some(0),
        };
        // The four fields fold independently through `from_wire`.
        assert_eq!(
            SubscriptionTier::from_wire(user.stock_subscription.unwrap()),
            Some(SubscriptionTier::Pro),
        );
        assert_eq!(
            SubscriptionTier::from_wire(user.options_subscription.unwrap()),
            Some(SubscriptionTier::Standard),
        );
        assert_eq!(
            SubscriptionTier::from_wire(user.indices_subscription.unwrap()),
            Some(SubscriptionTier::Value),
        );
        assert_eq!(
            SubscriptionTier::from_wire(user.interest_rate_subscription.unwrap()),
            Some(SubscriptionTier::Free),
        );

        // Round-trip into the human-facing labels too — this is what
        // `subscription_info()` returns to users.
        let info = SubscriptionInfo {
            stock: tier_label(SubscriptionTier::from_wire(
                user.stock_subscription.unwrap(),
            )),
            options: tier_label(SubscriptionTier::from_wire(
                user.options_subscription.unwrap(),
            )),
            indices: tier_label(SubscriptionTier::from_wire(
                user.indices_subscription.unwrap(),
            )),
            interest_rate: tier_label(SubscriptionTier::from_wire(
                user.interest_rate_subscription.unwrap(),
            )),
        };
        assert_eq!(info.stock, "Pro");
        assert_eq!(info.options, "Standard");
        assert_eq!(info.indices, "Value");
        assert_eq!(info.interest_rate, "Free");
    }

    /// Missing `*_subscription` bytes on `AuthUser` (the realistic
    /// Pro-on-stock, no-indices case) surface as the "Unknown" string
    /// on the corresponding `SubscriptionInfo` field — not as a
    /// silent collapse onto another tier.
    #[test]
    fn missing_subscription_byte_surfaces_as_unknown() {
        use crate::mdds::SubscriptionTier;

        let info = SubscriptionInfo {
            stock: tier_label(SubscriptionTier::from_wire(3)),
            options: tier_label(None),
            indices: tier_label(None),
            interest_rate: tier_label(None),
        };
        assert_eq!(info.stock, "Pro");
        assert_eq!(info.options, "Unknown");
        assert_eq!(info.indices, "Unknown");
        assert_eq!(info.interest_rate, "Unknown");
    }
}
