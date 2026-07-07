//! Unified `ThetaData` client -- single entry point, one auth, lazy FPSS.
//!
//! Connect once. Use historical data immediately. Streaming connects
//! on-demand when you first subscribe -- not at startup.
//!
//! ```rust,no_run
//! use thetadatadx::{Client, Credentials, DirectConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), thetadatadx::Error> {
//!     // One connect, one auth. FPSS is NOT connected yet.
//!     // Or inline: Credentials::new("user@example.com", "your-password")
//!     let client = Client::connect(
//!         &Credentials::from_file("creds.txt")?,
//!         DirectConfig::production(),
//!     ).await?;
//!
//!     // Market-data -- works immediately, via the `market-data` surface
//!     let eod = client.market_data().stock_history_eod("AAPL", "20240101", "20240301").await?;
//!
//!     // Streaming -- connects lazily on first subscribe, via `stream`
//!     use thetadatadx::streaming::{Contract, StreamData, StreamEvent};
//!     client.stream().start_streaming(|event| {
//!         if let StreamEvent::Data(StreamData::Trade { price, size, .. }) = event {
//!             println!("trade {price} x {size}");
//!         }
//!     })?;
//!     client.stream().subscribe(Contract::stock("AAPL").quote())?;
//!
//!     Ok(())
//! }
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;

use crate::auth::Credentials;
use crate::config::DirectConfig;
use crate::error::Error;
use crate::fpss::protocol::{Contract, FullSubscriptionKind, Subscription, SubscriptionKind};
use crate::fpss::{StreamEvent, StreamingClient};
use crate::mdds::MarketDataClient;
use crate::tdbe::types::enums::SecType;

/// Snapshot of the streaming side of the unified client.
///
/// One [`ArcSwap`] cell so every read path collapses to a single
/// atomic load. The user callback runs directly on the event-dispatch
/// consumer thread inside [`StreamingClient`], so the slot only needs to
/// track the live client.
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
    /// inside the [`StreamingClient`]'s event-dispatch consumer thread (panic
    /// isolated via `catch_unwind`); ring-buffer overflow is reported
    /// through [`StreamingClient::dropped_count`].
    Live { client: Arc<StreamingClient> },
    /// `stop_streaming()` ran (or `Drop` did). Distinguishes "was
    /// started, then stopped" from "never started" for
    /// [`ConnectionStatus::Disconnected`] vs
    /// [`ConnectionStatus::NotStarted`].
    Stopped,
}

/// Render a [`crate::SubscriptionTier`] (or `None`) as the
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
/// [`Client::subscription_info`] entry point — fields are
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

use crate::lifecycle::DispatcherSession;

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
/// Authenticates once at connect time. Historical data is
/// available immediately via the [`market_data`](Self::market_data)
/// surface. Streaming connects lazily on the first
/// [`start_streaming`](StreamSurface::start_streaming) on the
/// [`stream`](Self::stream) surface.
///
/// The data surfaces are sub-namespace accessors:
/// `client.market_data().<query>(..)` exposes the same method set as the
/// standalone market-data client; `client.stream().<op>(..)` exposes the
/// streaming lifecycle. The FLATFILES bulk-download methods stay on the
/// client directly.
pub struct Client {
    market_data: MarketDataClient,
    creds: Credentials,
    /// FLATFILES retry tuning. Snapshot of
    /// [`crate::config::DirectConfig::flatfiles`] taken at connect time
    /// so subsequent `DirectConfig` mutations cannot retroactively change
    /// retry behavior for already-issued requests.
    flatfiles_config: crate::config::FlatFilesConfig,
    /// Streaming lifecycle state, held behind an `Arc` so a teardown that
    /// outlives the borrowed [`StreamSurface`] — notably the pull-based
    /// [`crate::streaming::RecordBatchStream`], whose `close` / drop
    /// fire after the `&Client` borrow has ended — can quiesce the same state
    /// cells [`Self::stop_streaming`] does, through a [`std::sync::Weak`]. The
    /// callback path reaches it through `&self`; both routes funnel through
    /// [`StreamingState::quiesce`], so the two delivery modes leave the client
    /// in the same truthful, reusable state.
    streaming: Arc<StreamingState>,
}

/// The streaming lifecycle state of a [`Client`].
///
/// Factored out of [`Client`] so it can be shared (via `Arc`) with a
/// pull-based reader whose teardown outlives the `&Client` borrow that opened
/// it. Both [`Client::stop_streaming`] and the batch reader's close drive
/// [`Self::quiesce`], the single transition that swaps the slot to `Stopped`,
/// retires the dispatcher session, and records the drain flag — so streaming
/// status stays truthful and the client stays reusable regardless of which
/// delivery mode (callback or columnar pull) ran.
pub(crate) struct StreamingState {
    /// Streaming-side state machine. See [`StreamingSlot`] for the
    /// `Idle → Live → Stopped` lifecycle. The
    /// [`ArcSwap`] makes `is_streaming` / `connection_status` /
    /// `with_streaming` single-atomic-load reads — the previous design
    /// took two `Mutex` locks plus an `AtomicBool` for the same answer.
    state: ArcSwap<StreamingSlot>,
    /// Quiescence flags of every superseded streaming session that has
    /// not yet drained, captured during [`Client::stop_streaming`] /
    /// [`Client::reconnect_streaming`] before the `Live → Stopped` swap
    /// drops the previous `Arc<StreamingClient>`. [`Client::await_drain`]
    /// waits for **every** entry to flip to `true` before reporting
    /// quiescence; completed flags are GC'd lazily on each poll.
    ///
    /// A `Vec` (rather than a single slot) is required because stacked
    /// `start → stop → start → stop` cycles can layer multiple in-flight
    /// generations on top of each other before any one of them drains.
    /// If only the most recently retired flag were tracked, an earlier
    /// session whose event-dispatch consumer is still firing the callback
    /// would be silently lost when a later stop overwrites the slot —
    /// `await_drain()` would then return `true` based on the latest
    /// generation while the earlier callback is still firing on the
    /// FFI `ctx`. The Vec preserves every retired generation until its
    /// own flag is observed `true`.
    prev_drained: Mutex<Vec<Arc<AtomicBool>>>,
    /// Monotonic counter incremented by every [`Client::stop_streaming`].
    ///
    /// Each `start_streaming*()` snapshots this value at entry and
    /// re-checks it after the FPSS connect completes. If the snapshot
    /// no longer matches, an interleaving `stop_streaming` raised the
    /// generation, the freshly built [`StreamingClient`] is dropped, and
    /// the install is rejected. Closes the `Stopped → Live` resurrection
    /// race where an in-flight start could come up AFTER stop returned.
    stop_generation: AtomicU64,
    /// Dispatcher lifecycle — single mutex covering the single-flight
    /// serialisation, the `JoinHandle`, and the failure payload.
    ///
    /// Collapsed from three separate primitives:
    /// `start_lock: Mutex<()>`, `dispatcher_handle: Mutex<Option<JoinHandle<()>>>`,
    /// and `dispatcher_failed: Arc<AtomicBool>`.  Every `start_streaming` /
    /// `stop_streaming` / `reconnect_streaming` / `Drop` path acquires
    /// this one lock, transitions the variant, and releases.  Dispatcher
    /// panic state is carried by the `Failed` variant's payload — derived
    /// from `JoinHandle::join()` returning `Err(_)` rather than a
    /// separate atomic flag.
    dispatcher: Mutex<DispatcherSession>,
}

/// The deferred, lock-free part of a teardown, extracted under the dispatcher
/// lock by [`StreamingState::extract_for_teardown_locked`] and finished by
/// [`StreamingState::run_teardown`] with no lock held.
struct TeardownWork {
    /// The retired session's live client, to shut down (drains its threads and
    /// ring). `None` when the slot was not `Live`.
    client: Option<Arc<StreamingClient>>,
    /// The retired dispatcher session, to wake (columnar) and join.
    session: DispatcherSession,
}

/// Whether a retiring dispatcher session should register its drain flag for
/// [`StreamingState::await_drain`].
///
/// `await_drain` waits for an in-flight per-event user CALLBACK to finish
/// firing. Only the callback session has such a callback, so only it registers
/// a flag. The columnar (batches) pull session has no callback — and its
/// `drained` flag stays unset until the reader's `Arc<StreamingClient>` is
/// dropped rather than merely closed — so registering it would make a manual
/// `await_drain` time out spuriously while a closed-but-not-dropped columnar
/// reader is alive. The two are told apart by the explicit
/// [`DispatcherSession::Running::registers_drain_flag`] set at start, NOT by
/// whether a teardown wake hook is present: a callback session can now also
/// carry a hook (the TypeScript `ThreadsafeFunction` abort), so the hook is no
/// longer a proxy for "columnar". A non-`Running` session has nothing to wait
/// on.
fn session_registers_drain_flag(session: &DispatcherSession) -> bool {
    match session {
        // Read the explicit per-session flag: the per-event callback sessions
        // set it `true` (a user callback `await_drain` must wait for); the
        // columnar pull session sets it `false`. This must NOT be inferred from
        // whether `on_teardown` is present — a callback session can now also
        // carry a wake hook (the TypeScript `ThreadsafeFunction` abort), so the
        // hook is no longer a proxy for "columnar".
        DispatcherSession::Running {
            registers_drain_flag,
            ..
        } => *registers_drain_flag,
        DispatcherSession::Idle | DispatcherSession::Failed { .. } => false,
    }
}

impl StreamingState {
    /// A fresh, never-started streaming state.
    fn new() -> Self {
        Self {
            state: ArcSwap::from_pointee(StreamingSlot::Idle),
            prev_drained: Mutex::new(Vec::new()),
            stop_generation: AtomicU64::new(0),
            dispatcher: Mutex::new(DispatcherSession::Idle),
        }
    }

    /// Quiesce the streaming session: swap the slot to `Stopped`, retire the
    /// dispatcher session, and record the previous generation's drain flag.
    ///
    /// This is the single teardown both delivery modes share —
    /// [`Client::stop_streaming`] (callback path) and the pull reader's close
    /// (columnar path) both call it — so after either one the slot is
    /// `Stopped`, the dispatcher is `Idle` (or `Failed` if it panicked), and
    /// `is_streaming` / `connection_status` report the truth. Idempotent:
    /// calling it on an already-`Stopped` state bumps the generation and
    /// no-ops the rest.
    pub(crate) fn quiesce(&self) {
        // Direct teardown (stop_streaming / reconnect / Client drop): retire
        // whatever session is currently live, unconditionally.
        if let Some(work) = self.extract_for_teardown_locked(None) {
            self.run_teardown(work);
        }
    }

    /// Quiesce off the calling thread: extract the teardown work under the
    /// lock here (cheap, non-blocking), then finish the blocking part — the
    /// FPSS client shutdown and the dispatcher join — on a detached helper
    /// thread.
    ///
    /// This is the [`Client::drop`] path. A plain [`Self::quiesce`] would run
    /// [`Self::run_teardown`], whose cross-thread dispatcher join blocks the
    /// calling thread. When the last `Arc<Client>` is dropped while a binding
    /// runtime lock is held — the Python GIL, most importantly, since the
    /// dispatcher re-acquires it via `Python::attach` to finish an in-flight
    /// callback — that inline join deadlocks: the drop thread waits on the
    /// dispatcher, the dispatcher waits on the GIL the drop thread still holds.
    /// Detaching the join breaks the cycle: the drop returns at once (releasing
    /// the GIL / event loop), and the helper completes the join once the
    /// dispatcher drains. Callers who want a synchronous barrier use the
    /// explicit [`Client::close`] / `stop_streaming` + `await_drain` path,
    /// which already releases the binding lock around the wait.
    ///
    /// Mirrors the self-join detach `StreamingClient::drop` uses (see
    /// `fpss/mod.rs`): both spawn a named helper so cleanup still completes
    /// instead of blocking a thread on work it cannot finish inline.
    fn quiesce_detached(self: &Arc<Self>) {
        let state = Arc::clone(self);
        let detached = std::thread::Builder::new()
            .name("thetadatadx-client-drop-detach".to_owned())
            .spawn(move || state.quiesce());
        if let Err(e) = detached {
            // Spawning failed (thread limit / OOM). Fall back to an inline
            // teardown: on the common no-streaming drop there is no dispatcher
            // to join, so `quiesce` returns immediately and no deadlock is
            // possible; only a forgetful streaming drop under a held runtime
            // lock could still block, and that is strictly better than leaking
            // the dispatcher thread and the FPSS connection.
            tracing::warn!(
                target: "thetadatadx::client",
                error = %e,
                "failed to spawn thetadatadx-client-drop-detach; running teardown inline",
            );
            self.quiesce();
        }
    }

    /// Retire the live session ONLY if the live stop-generation still equals
    /// `expected_gen`, atomically with the generation re-check.
    ///
    /// This is the columnar reader's close gate. The generation re-check, the
    /// generation bump, the slot swap, and the dispatcher-session extraction
    /// all happen under the one dispatcher lock that also serialises
    /// `start_dispatcher`'s install and the bump in
    /// [`Self::extract_for_teardown_locked`], so a racing `stop_streaming` +
    /// `start_streaming` either fully precedes this locked section (the
    /// generation has advanced, so the re-check fails and this is a no-op,
    /// leaving the newer session to its owner) or fully follows it (this
    /// reader's session is already retired). A bare compare on the generation
    /// atomic alone would not close the window between the reader's earlier
    /// read and the teardown; guarding the check and the extraction together
    /// does. The slow part (client shutdown + dispatcher join) runs AFTER the
    /// lock is released, so it never holds the lock a callback might need.
    #[cfg(any(feature = "arrow", test))]
    pub(crate) fn quiesce_if_owned(&self, expected_gen: u64) {
        if let Some(work) = self.extract_for_teardown_locked(Some(expected_gen)) {
            self.run_teardown(work);
        }
    }

    /// The atomic part of teardown, run under the dispatcher lock: optionally
    /// gate on `expected_gen`, bump the generation, swap the slot to `Stopped`,
    /// and extract the previous live client and dispatcher session. Returns the
    /// extracted work for [`Self::run_teardown`] to finish OUTSIDE the lock, or
    /// `None` when the generation gate did not match.
    ///
    /// Only the bump + swap + extraction are under the lock — exactly the part
    /// that must be atomic with the generation gate and with
    /// `start_dispatcher`. The client shutdown and the dispatcher join, which
    /// can block while the dispatcher drains already-buffered events through
    /// user callbacks, are deliberately NOT done here: a callback may call
    /// `is_streaming` / `connection_status` / `stop_streaming`, which take this
    /// same lock, so holding it across the join would deadlock.
    fn extract_for_teardown_locked(&self, expected_gen: Option<u64>) -> Option<TeardownWork> {
        let mut guard = self.dispatcher.lock().unwrap_or_else(|e| e.into_inner());
        // The columnar reader-close gate: only retire while this reader's
        // session is still the live one. Checked under the lock so it is atomic
        // with the bump + swap + extraction below.
        if let Some(expected) = expected_gen {
            if self.stop_generation.load(Ordering::Acquire) != expected {
                return None;
            }
        }
        // Bump the stop generation BEFORE the slot swap so any in-flight
        // `start_streaming*()` that snapshotted the previous value will fail
        // its install check and not resurrect the slot to `Live` after this
        // returns. AcqRel because the ordering relative to the `state.swap`
        // below is what closes the resurrection race. The bump is under the
        // dispatcher lock, so it is also mutually exclusive with
        // `start_dispatcher`'s generation snapshot and the reader-close gate.
        self.stop_generation.fetch_add(1, Ordering::AcqRel);
        // Whether the session being retired should register its drain flag for
        // `await_drain`. `await_drain` exists to wait for a user CALLBACK to
        // finish firing; the columnar (batches) pull session has no callback,
        // so its flag must NOT be registered (see the push below).
        let register_drain_flag = session_registers_drain_flag(&guard);
        // Atomically swap to `Stopped`; whichever caller wins the swap owns the
        // previous `Arc<StreamingSlot>` and is the one that runs the shutdown
        // sequence.
        let prev = self.state.swap(Arc::new(StreamingSlot::Stopped));
        let client = match &*prev {
            StreamingSlot::Live { client } => {
                // Register the drain flag for `await_drain` ONLY for a callback
                // session. `await_drain` waits for in-flight user callbacks to
                // finish; the columnar session has no callback, and its
                // `drained` flag stays unset until the reader's
                // `Arc<StreamingClient>` is dropped (not merely closed), so
                // registering it would make a manual `await_drain` while a
                // closed-but-not-dropped columnar reader is alive time out
                // spuriously. Skipping it leaves the callback path's
                // `await_drain` unchanged.
                //
                // For a callback session: capture the drain flag BEFORE the
                // shutdown signal and PUSH it onto the retired-generations list
                // (rather than overwriting a single slot). Stacked
                // stop/start/stop cycles layer multiple in-flight generations
                // on top of each other; an earlier still-firing session's flag
                // must NOT be lost when a later session retires before the
                // earlier one has drained. `await_drain()` waits for ALL
                // entries, and lazily GCs flags that have flipped to `true`, so
                // a long-lived handle does not accumulate `Arc<AtomicBool>`
                // entries past their useful lifetime.
                if register_drain_flag {
                    self.prev_drained
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(client.drained_flag());
                }
                Some(Arc::clone(client))
            }
            StreamingSlot::Idle | StreamingSlot::Stopped => None,
        };
        // Extract the dispatcher session so the join runs outside the lock.
        let session = std::mem::replace(&mut *guard, DispatcherSession::Idle);
        Some(TeardownWork { client, session })
    }

    /// The slow part of teardown, run with NO lock held: shut the live client,
    /// wake a parked columnar dispatcher, and join the dispatcher thread.
    ///
    /// Holding no lock here is what keeps the teardown deadlock-free: while the
    /// client is shut down it keeps draining already-ring-buffered events
    /// through user callbacks until it observes the shutdown, and such a
    /// callback may call `is_streaming` / `connection_status` /
    /// `stop_streaming`, all of which take the dispatcher lock. With the lock
    /// released, those calls proceed, the dispatcher reaches its shutdown exit,
    /// and the join below returns.
    fn run_teardown(&self, work: TeardownWork) {
        let TeardownWork { client, session } = work;
        // Shut the FPSS client signal so its reader thread + event ring
        // consumer drain and exit.
        if let Some(client) = client {
            client.shutdown();
        }
        if let DispatcherSession::Running {
            handle,
            on_teardown,
            ..
        } = session
        {
            // Avoid blocking the consumer thread joining itself when a callback
            // (or, for the pull reader, a dispatcher-thread drop) called this.
            // Detach in that case; `await_drain` still observes quiescence via
            // the `prev_drained` flags. A self-call also means the dispatcher is
            // executing this teardown rather than parked on a backpressure
            // primitive, so the wake hook is neither needed nor run on that path
            // (it is dropped below with `session`).
            //
            // Fallback failed-state record: `JoinHandle::join()` returns `Err`
            // only if the dispatcher thread panicked OUTSIDE its inner
            // `catch_unwind` scaffolding (the gate wait or the post-body log) —
            // a panic INSIDE the consumer body is caught and logged there, so
            // the thread exits `Ok`. The common faulted-session transition (an
            // I/O-thread unwind, or a body machinery panic) is published by the
            // dispatcher thread itself at the fault point
            // (`record_dispatcher_failed_if_current` / the binding equivalents),
            // not here. When this fallback does fire it records
            // `DispatcherSession::Failed` so `is_streaming` / `connection_status`
            // see the failure even though the slot is now `Stopped`, by
            // RE-ACQUIRING the lock, which is safe now that the join has
            // completed and the lock is free.
            if handle.thread().id() != std::thread::current().id() {
                // Signal-grace-wake-join. `client.shutdown()` above signalled
                // the event ring, so a dispatcher parked there exits on its own
                // and is joined without ever firing the hook. The hook fires
                // only as a fallback, when the dispatcher is still blocked off
                // the ring after the grace window — the columnar pull dispatcher
                // parked in its bounded-queue `flush` wait (which the FPSS
                // shutdown does not touch), or a binding whose per-event handler
                // is parked in a full bounded callback queue (the TypeScript
                // `ThreadsafeFunction` path). Gating the wake behind the grace
                // (rather than firing it unconditionally) matters for hooks
                // whose wake is destructive: the TypeScript abort hook makes the
                // function permanently reject calls, and a `reconnect` re-uses
                // that same function, so aborting it on every stop would leave a
                // reconnected session unable to deliver events. A dispatcher
                // that exits cleanly is joined before the grace elapses, so the
                // destructive wake runs only when it is the sole way to break a
                // real deadlock.
                if let Err(payload) = join_dispatcher_with_wake(handle, on_teardown) {
                    let reason = downcast_panic_payload(payload);
                    tracing::error!(
                        target: "thetadatadx::client",
                        reason = %reason,
                        "thetadatadx-fpss-dispatcher panicked; session marked as failed",
                    );
                    // Record `Failed` ONLY if no newer session was installed
                    // since the extract left the slot `Idle`. The lock was
                    // released across the join, so a `start_dispatcher` may have
                    // installed a fresh `Running` session in that window; the
                    // panic belongs to the now-superseded OLD session, so
                    // overwriting the slot unconditionally would clobber the new
                    // session's `JoinHandle` (orphaning its thread) and falsely
                    // report a healthy live session as failed. Writing `Failed`
                    // only while the slot is still `Idle` records the panic for
                    // the common no-race case and leaves any newer session
                    // untouched.
                    let mut guard = self.dispatcher.lock().unwrap_or_else(|e| e.into_inner());
                    if matches!(*guard, DispatcherSession::Idle) {
                        *guard = DispatcherSession::Failed { reason };
                    }
                }
            }
        }
    }

    /// The current stop generation.
    ///
    /// Bumped by every teardown. The columnar reader captures the value its
    /// session was installed at (returned from the start path) and passes it
    /// to [`Self::quiesce_if_owned`] on close, so it retires only its own
    /// session. This read accessor exists for tests; the live close path
    /// compares the generation internally inside `quiesce_if_owned`.
    #[cfg(test)]
    fn stop_generation(&self) -> u64 {
        self.stop_generation.load(Ordering::Acquire)
    }

    /// Test-only: the current streaming-slot variant as a static label, so a
    /// test can assert the `Idle → Live → Stopped` transition `quiesce` drives
    /// without naming the private `Arc<StreamingClient>` payload.
    #[cfg(test)]
    fn slot_label(&self) -> &'static str {
        match &**self.state.load() {
            StreamingSlot::Idle => "Idle",
            StreamingSlot::Live { .. } => "Live",
            StreamingSlot::Stopped => "Stopped",
        }
    }

    /// Test-only: whether the dispatcher session is currently `Idle`.
    #[cfg(test)]
    fn dispatcher_is_idle(&self) -> bool {
        matches!(
            *self.dispatcher.lock().unwrap_or_else(|e| e.into_inner()),
            DispatcherSession::Idle
        )
    }
}

impl Client {
    /// Start a fluent [`ClientBuilder`](crate::ClientBuilder), the headline ergonomic for
    /// constructing a client with the API key (or email + password) and
    /// the target environment selected inline.
    ///
    /// The API key is a first-class, directly-passed argument
    /// ([`ClientBuilder::api_key`](crate::ClientBuilder::api_key) and its
    /// env / `.env` siblings), distinct from the email + password pair.
    /// The lower-level typed path [`Client::connect`] (a pre-built
    /// [`Credentials`] + [`DirectConfig`]) stays available for power
    /// users; the builder composes those two values internally and calls
    /// it.
    ///
    /// ```rust,no_run
    /// use thetadatadx::Client;
    ///
    /// # async fn doc() -> Result<(), thetadatadx::Error> {
    /// let client = Client::builder()
    ///     .api_key("td1_example_key")
    ///     .stage()
    ///     .connect()
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn builder() -> crate::client_builder::ClientBuilder {
        crate::client_builder::ClientBuilder::new()
    }

    /// Connect with an inline API key and a target config.
    ///
    /// A lightweight direct convenience over [`Client::builder`] for the
    /// single most common shape — an API key plus a [`DirectConfig`].
    /// Equivalent to
    /// `Client::builder().api_key(key).config(config).connect().await`
    /// and to `Client::connect(&Credentials::api_key(key), config).await`.
    /// Reach for the [`builder`](Self::builder) when sourcing the key from
    /// the environment or a file, or for email + password auth.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn connect_with_api_key(
        key: impl Into<String>,
        config: DirectConfig,
    ) -> Result<Self, Error> {
        Self::connect(&Credentials::api_key(key), config).await
    }

    /// Connect to `ThetaData`. Authenticates once, opens gRPC channel.
    ///
    /// FPSS streaming is NOT connected yet -- call
    /// [`start_streaming`](StreamSurface::start_streaming) on the
    /// [`stream`](Client::stream) surface when you need real-time data.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error> {
        // Validate BEFORE the exporter install: `try_install_exporter` binds
        // the metrics port and installs the process-global recorder (which has
        // no uninstall), so running it against a config that the downstream
        // funnel would reject leaves a port bound with no live client. Idempotent
        // with the re-check inside `MarketDataClient::connect`.
        let config = config.validate()?;
        // Start the Prometheus exporter BEFORE opening the gRPC channel
        // so the first `thetadatadx.grpc.requests` counter hit is already
        // covered. No-op when the feature is disabled or `metrics_port`
        // is `None` (the default).
        crate::observability::try_install_exporter(&config)?;
        let flatfiles_config = config.flatfiles.clone();
        let market_data = MarketDataClient::connect(creds, config).await?;
        Ok(Self {
            market_data,
            creds: creds.clone(),
            flatfiles_config,
            streaming: Arc::new(StreamingState::new()),
        })
    }

    /// Helper: error returned when `start_streaming*` is called while
    /// the slot is already [`StreamingSlot::Live`].
    fn already_streaming() -> Error {
        Error::Stream {
            kind: crate::error::StreamErrorKind::ConnectionRefused,
            message: "streaming already started".into(),
        }
    }

    /// Helper: error returned when an in-flight `start_streaming*()`
    /// raced behind a [`Self::stop_streaming`] and would have resurrected
    /// streaming after the caller observed it stopped. The freshly built
    /// [`StreamingClient`] is dropped before this returns.
    fn stopped_during_start() -> Error {
        Error::Stream {
            kind: crate::error::StreamErrorKind::Disconnected,
            message: "stop_streaming() raced ahead of start_streaming(); start refused".into(),
        }
    }

    /// Start the FPSS streaming connection with a callback handler.
    ///
    /// Open the streaming channel, authenticate, and start the reader
    /// and consumer threads. The user callback fires on the bounded-ring
    /// consumer thread, one event at a time.
    ///
    /// # Contracts
    ///
    /// 1. **Reader never blocks on user code.** A full ring drops the
    ///    event and bumps [`Self::dropped_event_count`]. Poll the
    ///    counter on a periodic timer to detect a slow consumer.
    /// 2. **Per-callback panic isolation.** Each callback invocation is
    ///    individually wrapped in [`std::panic::catch_unwind`]. A panic
    ///    on event N is caught, recorded via [`Self::panic_count`], and
    ///    does not stop event delivery — event N+1 continues normally.
    /// 3. **Lifecycle restriction.** Do NOT call
    ///    [`Self::stop_streaming`], [`Self::reconnect_streaming`], or
    ///    anything that drops the underlying client from inside the
    ///    callback. The calls do not deadlock (cleanup detaches), but
    ///    they return BEFORE the old consumer has finished draining
    ///    the in-flight ring contents — FFI callers freeing `ctx`
    ///    after `thetadatadx_*_stop_streaming` returns will observe
    ///    use-after-free. Instead, set a flag from the callback, call
    ///    [`Self::stop_streaming`] from another thread, then
    ///    [`Self::await_drain`] before reusing captured resources.
    ///
    /// The callback MUST return within microseconds; hand slow work
    /// off to a bounded queue inside the callback body.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub(crate) fn start_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
    {
        // Identity batch scope — each batch drain runs directly. The
        // GIL-amortising form lives in [`Self::start_streaming_scoped`].
        // No teardown wake hook: a handler that delivers events through a Rust
        // call, the CPython interpreter lock, or the C ABI parks only on the
        // event ring, which `client.shutdown()` already signals at teardown.
        self.start_streaming_scoped(handler, |drain| drain(), None)
    }

    /// Start FPSS streaming with each consumer batch drain wrapped in a
    /// caller-supplied `scope`.
    ///
    /// Identical to [`Self::start_streaming`] in every observable respect
    /// — the same single-flight gate, install/rollback, panic isolation,
    /// and one-call-per-event delivery — except the dispatcher drives
    /// [`crate::fpss::StreamingClient::for_each_scoped`] instead of
    /// `for_each`, so `scope` brackets each batch drain. The inter-batch
    /// wait on an idle ring runs outside `scope`.
    ///
    /// A language binding uses this to acquire its interpreter lock once
    /// per batch rather than once per event: the lock is held across
    /// every `handler` call in a batch and released across the idle wait,
    /// which both raises sustained drain throughput and keeps a blocking
    /// wait off the lock. `handler` still fires exactly once per event.
    ///
    /// `on_teardown` is the optional dispatcher wakeup hook (see
    /// [`DispatcherSession::Running`]). It is required when `handler` can park
    /// off the event ring waiting on a primitive the FPSS shutdown does not
    /// touch — notably a binding whose `handler` hands each event to a bounded
    /// queue and blocks once that queue is full. The TypeScript binding's
    /// per-event callback path does exactly this: it routes events through a
    /// napi `ThreadsafeFunction` with a bounded call queue and a `Blocking`
    /// call mode, so a full queue parks the dispatcher inside `call` waiting
    /// for the Node main thread to drain it. During teardown the main thread is
    /// itself inside the dispatcher join, so it can never drain the queue, and
    /// the hook is what aborts the threadsafe function (making the in-flight
    /// `Blocking` call return) so the dispatcher resumes, observes the
    /// shutdown, exits `for_each_scoped`, and the join completes. Pass `None`
    /// when `handler` parks only on the event ring (the Rust / Python / C ABI
    /// callback paths), which `client.shutdown()` already signals.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub(crate) fn start_streaming_scoped<F, S>(
        &self,
        mut handler: F,
        scope: S,
        on_teardown: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<(), Error>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
        S: FnMut(&mut dyn FnMut() -> crate::PollOutcome) -> crate::PollOutcome + Send + 'static,
    {
        // The callback path's dispatcher body drives the scoped drain with
        // the user handler. The connect / spawn / install / rollback
        // sequence is shared with the columnar `batches()` path via
        // `start_dispatcher`; only the per-thread consumer body differs.
        let mut scope = scope;
        // Captured so the dispatcher body can flip the session to `Failed` on
        // an I/O-thread fault (a clean shutdown returns `PollOutcome::Shutdown`
        // and leaves the session `Running`).
        let streaming = Arc::clone(&self.streaming);
        self.start_dispatcher(
            move |client| {
                // Mirror the columnar body's `catch_unwind`: a panic in the
                // drain machinery or in the binding-supplied scope closure (the
                // Python client's `Python::attach`, which can panic during
                // interpreter finalization) escapes `for_each_scoped`. Without
                // this catch the panic reaches the spawn closure's
                // `catch_unwind`, which only logs, so the thread exits `Ok`, the
                // teardown join never records, and the session stays `Running`
                // behind a dead dispatcher — the exact appears-healthy stall
                // this fault path exists to eliminate.
                let drained = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.for_each_scoped(|event| handler(event), &mut scope)
                }));
                match drained {
                    // The FPSS I/O thread unwound: surface it to callback and
                    // binding users as a failed session so `is_streaming` flips
                    // false, matching the pull path's `DispatcherFailed`.
                    Ok(crate::PollOutcome::Failed) => {
                        record_dispatcher_failed_if_current(
                            &streaming.dispatcher,
                            "fpss io thread terminated abnormally".to_string(),
                        );
                    }
                    // Clean shutdown (`for_each_scoped` only ever returns a
                    // terminal outcome): the callback path has no sink to flush.
                    Ok(_) => {}
                    // A panic escaped the drain: flip the session to `Failed`
                    // here (the spawn closure's `catch_unwind` swallows the
                    // re-raised panic and only logs, so the thread exits `Ok`
                    // and the teardown join never records it), then re-raise so
                    // that closure still logs the machinery panic.
                    Err(payload) => {
                        record_dispatcher_failed_if_current(
                            &streaming.dispatcher,
                            panic_reason(payload.as_ref()),
                        );
                        std::panic::resume_unwind(payload);
                    }
                }
            },
            on_teardown,
            // Callback session: `await_drain` waits for this user handler.
            true,
        )
        .map(|(_client, _generation)| ())
    }

    /// Start FPSS streaming with a custom dispatcher consumer body.
    ///
    /// Factors the connect / spawn / install / rollback sequence shared by
    /// the per-event callback path ([`Self::start_streaming_scoped`]) and
    /// the columnar pull path ([`Self::start_streaming_batches`]). The
    /// `dispatcher_body` closure runs on the dispatcher thread once the
    /// streaming slot is installed; it owns the consumer loop (typically
    /// [`crate::fpss::StreamingClient::for_each_scoped`]) and returns when
    /// the ring shuts down. The single-flight gate, startup gate, install,
    /// and failure rollback are identical across both consumers, so they
    /// live here once.
    ///
    /// `on_teardown` is the optional dispatcher wakeup hook (see
    /// [`DispatcherSession::Running`]). The callback path passes `None` when its
    /// handler parks only on the event ring, or a hook that releases a handler
    /// blocked off the ring (the TypeScript `ThreadsafeFunction` callback queue);
    /// the columnar path passes a hook that releases a dispatcher parked on the
    /// batch queue so a direct `stop_streaming` / drop can join it. It is
    /// installed atomically with the `Running` transition under the dispatcher
    /// lock, so a teardown racing the start never sees a `Running` session
    /// without its hook.
    ///
    /// `registers_drain_flag` records whether the session has a user callback
    /// that [`StreamSurface::await_drain`] must wait for (see
    /// [`DispatcherSession::Running`]). The per-event callback path passes
    /// `true`; the columnar pull path passes `false`.
    ///
    /// On success returns the live client and the stop-generation the session
    /// was installed at. The columnar reader stamps that generation so its
    /// close tears down only the session it started, never a later session
    /// that replaced it (see [`crate::streaming::RecordBatchStream`]).
    /// The value is the generation the install was validated against, so it
    /// identifies this session even if a teardown bumps the generation right
    /// after the install commits.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure, or
    /// when a stream is already active on this client.
    pub(crate) fn start_dispatcher<B>(
        &self,
        dispatcher_body: B,
        on_teardown: Option<Box<dyn FnOnce() + Send>>,
        registers_drain_flag: bool,
    ) -> Result<(Arc<StreamingClient>, u64), Error>
    where
        B: FnOnce(Arc<StreamingClient>) + Send + 'static,
    {
        // Single-flight gate: `dispatcher` mutex serialises the entire
        // connect-spawn-install sequence so two concurrent starts cannot
        // each spawn a dispatcher and race to overwrite the Running
        // variant.  The lock is held across the FPSS connect call
        // (typically tens of milliseconds); a second concurrent start
        // is rejected upfront by the `is_streaming` fast path or by
        // `install_live` once it observes a `Live` slot.
        let mut dispatcher_guard = self
            .streaming
            .dispatcher
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        // Reject a second concurrent start before paying the connect cost.
        if matches!(&**self.streaming.state.load(), StreamingSlot::Live { .. }) {
            return Err(Self::already_streaming());
        }

        // Snapshot the stop generation BEFORE connecting. If another
        // thread calls `stop_streaming()` between this load and the
        // post-connect `install_live`, the install path observes the
        // mismatch and refuses to resurrect the slot to `Live`.
        let gen_at_entry = self.streaming.stop_generation.load(Ordering::Acquire);

        let config = self.market_data.config();
        let client = StreamingClient::builder(&self.creds, &config.streaming.hosts)
            .ring_size(config.streaming.ring_size)
            .flush_mode(config.streaming.flush_mode)
            .consumer_cpu(config.streaming.consumer_cpu)
            .reconnect_policy(config.reconnect.policy.clone())
            .reconnect_wait_ms(config.reconnect.wait_ms)
            .reconnect_wait_max_ms(config.reconnect.wait_max_ms)
            .reconnect_wait_rate_limited_ms(config.reconnect.wait_rate_limited_ms)
            .reconnect_wait_server_restart_ms(config.reconnect.wait_server_restart_ms)
            .reconnect_jitter(config.reconnect.jitter)
            .reconnect_replay_burst_size(config.reconnect.replay_burst_size)
            .reconnect_replay_pace_ms(config.reconnect.replay_pace_ms)
            .connect_timeout_ms(config.streaming.connect_timeout_ms)
            .read_timeout_ms(config.streaming.timeout_ms)
            .ping_interval_ms(config.streaming.ping_interval_ms)
            .io_read_slice_ms(config.streaming.io_read_slice_ms)
            .keepalive_idle_secs(config.streaming.keepalive_idle_secs)
            .keepalive_interval_secs(config.streaming.keepalive_interval_secs)
            .keepalive_retries(config.streaming.keepalive_retries)
            .host_selection(config.streaming.host_selection)
            .host_shuffle_seed(config.streaming.host_shuffle_seed)
            .build()
            .map_err(crate::error::Error::from)?;
        let client_arc = Arc::new(client);

        // Spawn the dispatcher behind a startup gate so the consumer body
        // does not run until the streaming slot is installed. This
        // closes two windows simultaneously:
        //
        //   1. `is_streaming()` / `connection_status()` cannot observe
        //      `Live` without a live dispatcher thread behind it (the
        //      install happens before the gate opens, and the gate is
        //      what releases the dispatcher into its iterator loop).
        //   2. A re-entrant call from the first delivered callback
        //      cannot observe `Idle` / `Stopped` because the slot is
        //      already `Live` by the time the dispatcher pulls its
        //      first event.
        //
        // The gate also lets the install-failure rollback signal the
        // dispatcher to fall through without ever running the consumer
        // body.
        //
        // `OnceLock::wait()` (stable since Rust 1.87, below our 1.88 MSRV) blocks the
        // dispatcher until the spawn site calls `.set(true)` (go) or
        // `.set(false)` (abort).
        let gate: Arc<OnceLock<bool>> = Arc::new(OnceLock::new());
        let gate_for_dispatcher = Arc::clone(&gate);
        let dispatcher_client = Arc::clone(&client_arc);
        let dispatcher_handle = std::thread::Builder::new()
            .name("thetadatadx-fpss-dispatcher".into())
            .spawn(move || {
                if !*gate_for_dispatcher.wait() {
                    return;
                }
                // The consumer body drives `StreamingClient::for_each_scoped`,
                // which drives `poll_batch`, which wraps each callback
                // invocation in its own `catch_unwind`.  A panic in the
                // handler is caught, recorded via `panic_count()`, and does
                // not stop event delivery for subsequent events.  The outer
                // `catch_unwind` below guards only the event-iteration
                // machinery itself (ring mutex poison, OOM in the polling
                // path, etc.) — not user-callback panics.
                //
                // This `catch_unwind` SWALLOWS a body panic and only logs it:
                // the thread then exits normally, so the teardown join returns
                // `Ok`. The faulted-session transition is published by the body
                // itself at the fault point (an I/O-thread unwind flips the
                // session via `record_dispatcher_failed_if_current`; the
                // columnar body does the same before it re-raises a machinery
                // panic into here), so this arm does not itself record `Failed`.
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    dispatcher_body(dispatcher_client);
                }));
                if outcome.is_err() {
                    tracing::error!(
                        target: "thetadatadx::client",
                        "thetadatadx-fpss-dispatcher panicked in event iteration machinery; failed state recorded by the dispatcher body",
                    );
                }
            })
            .map_err(|e| Error::Stream {
                kind: crate::error::StreamErrorKind::ConnectionRefused,
                message: format!("failed to spawn streaming dispatcher thread: {e}"),
            })?;

        // Run the install under `catch_unwind` so a panic (e.g. an
        // `ArcSwap` allocator OOM) cannot leave the dispatcher blocked
        // on `gate.wait()` forever.
        let install_attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.install_live(
                StreamingSlot::Live {
                    client: Arc::clone(&client_arc),
                },
                gen_at_entry,
            )
        }));

        let install_result = match install_attempt {
            Ok(r) => r,
            Err(panic_payload) => {
                // Signal abort so the dispatcher exits without invoking
                // the user callback, then roll the freshly-built client
                // back before re-raising the panic.
                let _ = gate.set(false);
                client_arc.shutdown();
                drop(client_arc);
                if dispatcher_handle.thread().id() != std::thread::current().id() {
                    let _ = dispatcher_handle.join();
                }
                *dispatcher_guard = DispatcherSession::Idle;
                std::panic::resume_unwind(panic_payload);
            }
        };

        match install_result {
            Ok(()) => {
                // Publish the Running variant before opening the gate so
                // `stop_streaming` finds the JoinHandle regardless of how
                // quickly the dispatcher thread starts executing. The teardown
                // wake hook is installed in the same transition, so a quiesce
                // racing this start never observes a `Running` session lacking
                // its hook.
                *dispatcher_guard = DispatcherSession::Running {
                    handle: dispatcher_handle,
                    on_teardown,
                    registers_drain_flag,
                };
                let _ = gate.set(true);
                Ok((client_arc, gen_at_entry))
            }
            Err(install_err) => {
                // Shut down the client so the dispatcher sees a clean
                // ring shutdown when it wakes; abort the gate so the
                // dispatcher skips its iterator loop entirely.
                client_arc.shutdown();
                drop(client_arc);
                let _ = gate.set(false);
                if dispatcher_handle.thread().id() != std::thread::current().id() {
                    let _ = dispatcher_handle.join();
                }
                *dispatcher_guard = DispatcherSession::Idle;
                Err(install_err)
            }
        }
    }

    /// Start FPSS streaming in columnar pull mode, routing decoded
    /// market-data events into the Arrow batch sink behind `shared`.
    ///
    /// Reuses the connect / install / dispatcher machinery via
    /// [`Self::start_dispatcher`]; the dispatcher thread owns a
    /// `BatchSink` and drives the scoped drain
    /// with it instead of a user callback. The sink appends one row per data
    /// event, flushes on `batch_size` / `linger` / shutdown, and publishes a
    /// terminal marker when the drain loop returns. The returned
    /// [`crate::streaming::RecordBatchStream`] reads finished
    /// batches off the same `shared` queue.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure, or
    /// when a stream is already active on this client.
    /// On success returns the live client and the stop-generation the columnar
    /// session was installed at, so the reader can stamp the session it owns
    /// and refuse to tear down a later session that replaced it.
    #[cfg(feature = "arrow")]
    pub(crate) fn start_streaming_batches(
        &self,
        shared: Arc<crate::fpss::batch_reader::Shared>,
        batch_size: usize,
        linger: std::time::Duration,
        backpressure: crate::fpss::batch_reader::Backpressure,
    ) -> Result<(Arc<StreamingClient>, u64), Error> {
        // Teardown wake hook: a `Block` dispatcher can be parked in the batch
        // queue's `flush` wait, which the FPSS shutdown does not touch. Any
        // teardown that runs `quiesce` directly (a `Client` drop /
        // `stop_streaming` / `reconnect_streaming`, not just the reader's own
        // close) invokes this just before joining the dispatcher, so the parked
        // flush is released and the join completes. It runs the SAME wakeup the
        // reader's `close_shared` runs, so the two routes converge; it is
        // idempotent, so a close that already woke is harmless.
        let wake_shared = Arc::clone(&shared);
        let on_teardown: Box<dyn FnOnce() + Send> = Box::new(move || wake_shared.close_and_wake());
        // Captured so the dispatcher body can flip the session to `Failed` on a
        // fault, so the unified `Client::is_streaming` reports the dead loop for
        // the columnar pull path too (not only the reader's terminal error).
        let streaming = Arc::clone(&self.streaming);
        self.start_dispatcher(
            move |client| {
                let sink = crate::fpss::batch_reader::BatchSink::new(
                    shared,
                    batch_size,
                    linger,
                    backpressure,
                );
                // The handler and the scope each take a short, non-overlapping
                // lock on the sink's accumulator (see `BatchSink`), so cloning
                // the sink (an `Arc` bundle) into both closures plus the
                // post-loop finish is sound and the drain path stays lock-free
                // between events.
                let handler_sink = sink.clone();
                let scope_sink = sink.clone();
                // Catch a panic in the event-iteration machinery or in the
                // `scope_drain` linger flush (which runs OUTSIDE the per-event
                // `catch_unwind`): without this the reader would park on the
                // condvar forever waiting for a `finished` the dead dispatcher
                // never sets, while the I/O thread keeps running.
                let drained = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    client.for_each_scoped(
                        move |event| handler_sink.on_event(event),
                        move |drain| scope_sink.scope_drain(drain),
                    )
                }));
                match drained {
                    // Clean shutdown: flush the final partial batch and publish
                    // the terminal end-of-stream marker so the reader sees
                    // `finished` after consuming every queued batch.
                    Ok(crate::PollOutcome::Shutdown) => sink.finish(),
                    // The FPSS I/O thread unwound: surface it to the reader as a
                    // terminal error AND flip the session to `Failed` so
                    // `is_streaming` reports the dead loop, matching the callback
                    // path and the pull path's `DispatcherFailed`.
                    Ok(crate::PollOutcome::Failed) => {
                        sink.fail(crate::streaming::StreamError::DispatcherFailed(
                            "fpss io thread terminated abnormally".to_string(),
                        ));
                        record_dispatcher_failed_if_current(
                            &streaming.dispatcher,
                            "fpss io thread terminated abnormally".to_string(),
                        );
                    }
                    // `for_each_scoped` only ever returns a terminal outcome.
                    Ok(_) => sink.finish(),
                    // A panic escaped the drain: wake the reader with an error and
                    // flip the session to `Failed` immediately (both here, since
                    // the spawn closure's `catch_unwind` swallows the re-raised
                    // panic and only logs, so the thread exits `Ok` and the
                    // teardown join never records it). Re-raise so that closure
                    // still logs the machinery panic.
                    Err(payload) => {
                        let reason = panic_reason(payload.as_ref());
                        sink.fail(crate::streaming::StreamError::DispatcherFailed(
                            reason.clone(),
                        ));
                        record_dispatcher_failed_if_current(&streaming.dispatcher, reason);
                        std::panic::resume_unwind(payload);
                    }
                }
            },
            Some(on_teardown),
            // Columnar pull session: no user callback, so `await_drain` must
            // NOT register this session's drain flag (it stays unset until the
            // reader handle is dropped).
            false,
        )
    }

    /// A weak handle to this client's streaming lifecycle state, for the
    /// pull-based [`crate::streaming::RecordBatchStream`] to quiesce
    /// the session on close.
    ///
    /// Weak (not strong) so the reader never keeps the client's streaming
    /// state alive past the client itself: if the client is dropped first, its
    /// own `Drop` quiesces and the reader's later close finds nothing to
    /// upgrade and is a no-op; in the common order (reader closed first) the
    /// upgrade succeeds and resets the slot to `Stopped`, making
    /// `is_streaming` / `connection_status` truthful and the client reusable.
    #[cfg(feature = "arrow")]
    pub(crate) fn streaming_state_weak(&self) -> std::sync::Weak<StreamingState> {
        Arc::downgrade(&self.streaming)
    }

    /// Atomically swap the slot to a fresh `Live` state.
    ///
    /// Rejects the install when:
    ///
    /// 1. another `start_streaming*` raced in and the slot is already
    ///    `Live` (returns [`Self::already_streaming`]); or
    /// 2. an interleaving [`Self::stop_streaming`] bumped the
    ///    [`Self::stop_generation`] counter past `gen_at_entry`
    ///    (returns [`Self::stopped_during_start`]). This is the
    ///    `Stopped → Live` resurrection guard: a caller that started
    ///    connecting BEFORE `stop_streaming` was invoked must NOT see
    ///    its connection installed AFTER stop returned, even though
    ///    the FPSS connect itself succeeded.
    ///
    /// On either rejection the freshly built [`StreamingClient`] (carried
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
        let stop_gen = &self.streaming.stop_generation;
        let prev = self.streaming.state.rcu(|current| match &**current {
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
        if !Arc::ptr_eq(&self.streaming.state.load_full(), &new) {
            return Err(Self::stopped_during_start());
        }
        Ok(())
    }

    /// Snapshot of events the TLS reader could not publish into the
    /// event ring because the consumer fell behind and the ring
    /// was full. Returns `0` when streaming has not started.
    ///
    /// Operators should poll this on a periodic timer (e.g. every
    /// second) and emit a `warn` log on any non-zero delta. A
    /// per-drop log would amplify under sustained overflow.
    #[must_use]
    pub(crate) fn dropped_event_count(&self) -> u64 {
        let snap = self.streaming.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.dropped_count(),
            StreamingSlot::Idle | StreamingSlot::Stopped => 0,
        }
    }

    /// Point-in-time count of streaming events published into the
    /// event ring but not yet drained by the dispatcher — the
    /// in-flight depth between the I/O thread and your callback.
    /// Returns `0` when streaming has not started.
    ///
    /// The leading back-pressure signal:
    /// [`Self::dropped_event_count`] only moves AFTER data has been
    /// lost, while a rising occupancy that approaches
    /// [`Self::ring_capacity`] predicts those drops while there is
    /// still time to react. Sampling never blocks the feed — it is a
    /// pair of relaxed atomic loads on the calling thread; poll it
    /// from your own monitoring thread at any cadence.
    #[must_use]
    pub(crate) fn ring_occupancy(&self) -> usize {
        let snap = self.streaming.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.ring_occupancy(),
            StreamingSlot::Idle | StreamingSlot::Stopped => 0,
        }
    }

    /// Configured capacity of the streaming event ring in slots (the
    /// `streaming.ring_size` setting, a validated power of two). Returns
    /// `0` when streaming has not started.
    ///
    /// The fixed denominator for [`Self::ring_occupancy`]: when the
    /// occupancy sample approaches this value the ring is saturating
    /// and further publishes will be dropped (counted by
    /// [`Self::dropped_event_count`]).
    #[must_use]
    pub(crate) fn ring_capacity(&self) -> usize {
        let snap = self.streaming.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.ring_capacity(),
            StreamingSlot::Idle | StreamingSlot::Stopped => 0,
        }
    }

    /// Milliseconds since the most recent inbound streaming frame of
    /// any kind (data tick, heartbeat, control), or `None` when
    /// streaming has not started or no frame has been received yet.
    ///
    /// The operator-facing staleness clock: a healthy session stays in
    /// the low hundreds of milliseconds (the upstream heartbeats every
    /// ~100 ms even when no market data flows), so a steadily growing
    /// value is the earliest external signal of a dead or wedged
    /// connection.
    #[must_use]
    pub(crate) fn millis_since_last_event(&self) -> Option<u64> {
        let snap = self.streaming.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.millis_since_last_event(),
            StreamingSlot::Idle | StreamingSlot::Stopped => None,
        }
    }

    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// streaming frame of any kind. Returns `0` when streaming has not
    /// started or no frame has been received yet. Raw feed for
    /// [`Self::millis_since_last_event`], exposed for callers
    /// correlating against their own pipeline timestamps.
    #[must_use]
    pub(crate) fn last_event_received_at_unix_nanos(&self) -> i64 {
        let snap = self.streaming.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.last_event_received_at_unix_nanos(),
            StreamingSlot::Idle | StreamingSlot::Stopped => 0,
        }
    }

    /// Address (`host:port`) of the streaming server the current
    /// session is connected to, following the session across
    /// auto-reconnects. Returns `None` when streaming has not started.
    #[must_use]
    pub(crate) fn last_connected_addr(&self) -> Option<String> {
        let snap = self.streaming.state.load();
        match &**snap {
            StreamingSlot::Live { client } => Some(client.last_connected_addr()),
            StreamingSlot::Idle | StreamingSlot::Stopped => None,
        }
    }

    /// Cumulative count of user-callback faults: Rust panics caught by the
    /// per-invocation `catch_unwind` boundary, plus Python exceptions
    /// raised inside the callback and routed through
    /// `PyErr::write_unraisable` (the Python binding bumps this counter via
    /// the binding-only `record_panic` shim on the unraisable path). The
    /// TypeScript binding surfaces JS errors via Node's `uncaughtException`
    /// instead of this counter. Returns `0` when streaming has not started.
    #[must_use]
    pub(crate) fn panic_count(&self) -> u64 {
        let snap = self.streaming.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.panic_count(),
            StreamingSlot::Idle | StreamingSlot::Stopped => 0,
        }
    }

    /// Increment the panic counter on the live streaming session by one.
    ///
    /// Called by language-binding dispatchers when the user callback raises
    /// a language-level exception that bypasses the Rust `catch_unwind`
    /// boundary. No-op when streaming is not active.
    #[cfg(feature = "__internal")]
    #[doc(hidden)]
    pub(crate) fn record_panic(&self) {
        let snap = self.streaming.state.load();
        if let StreamingSlot::Live { client } = &**snap {
            client.record_panic();
        }
    }

    /// Whether streaming is currently active.
    ///
    /// This flag flips immediately on [`Self::stop_streaming`] /
    /// [`Self::reconnect_streaming`] (i.e. on the atomic swap of the
    /// `Live → Stopped` state cell), BEFORE the previous I/O thread,
    /// event-dispatch consumer, and any in-flight user callback have
    /// drained. A `false` return therefore means *no new events will
    /// be enqueued for the previous callback*, not *the previous
    /// callback has stopped firing*.
    ///
    /// Pair with [`Self::await_drain`] when the caller needs the
    /// stronger guarantee — e.g. before freeing an FFI callback
    /// context, replacing the callback closure, or asserting the old
    /// consumer thread has joined.
    pub(crate) fn is_streaming(&self) -> bool {
        match &**self.streaming.state.load() {
            StreamingSlot::Live { .. } => {
                let session = self
                    .streaming
                    .dispatcher
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                !matches!(*session, DispatcherSession::Failed { .. })
            }
            StreamingSlot::Idle | StreamingSlot::Stopped => false,
        }
    }

    /// Whether the live streaming session is currently authenticated.
    ///
    /// Distinct from [`Self::is_streaming`]: a `Live` slot can hold a
    /// streaming client whose authenticated flag has been cleared after a
    /// server disconnect, before the caller has issued a reconnect. A
    /// panicked dispatcher folds back to `false` here too, so the failed
    /// state is uniformly visible across every status reader rather than
    /// reporting "authenticated with no deliveries". Returns `false`
    /// before streaming starts and after it stops.
    pub(crate) fn is_authenticated(&self) -> bool {
        match &**self.streaming.state.load() {
            StreamingSlot::Live { client } => {
                let session = self
                    .streaming
                    .dispatcher
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                client.is_authenticated() && !matches!(*session, DispatcherSession::Failed { .. })
            }
            StreamingSlot::Idle | StreamingSlot::Stopped => false,
        }
    }

    /// Whether a prior streaming session has registered its drain flag
    /// for [`Self::await_drain`] to poll on.
    ///
    /// Returns `true` once [`Self::stop_streaming`] or
    /// [`Self::reconnect_streaming`] has captured the previous
    /// session's drain flag into the internal slot, even if the flag
    /// has not yet flipped. Returns `false` on a fresh handle that has
    /// never started streaming, or on a unified handle that has only
    /// served market-data endpoints.
    ///
    /// FFI free paths use this to disambiguate the two `false` returns
    /// from `await_drain` (timeout vs. nothing-to-wait-on); only the
    /// former is a real concern worth surfacing in operator logs.
    ///
    /// **Non-blocking.** Uses `try_lock` on the internal slot mutex.
    /// If the mutex is contended (another thread is mid-`stop_streaming`
    /// or `reconnect_streaming` swapping the slot), this returns `true`
    /// — the conservative answer, because the FFI `_free` path uses
    /// this signal to decide whether to wait on the drain barrier, and
    /// "wait" is the correct fail-safe when a stop is actively in
    /// flight. The contention window is microseconds (the lock is held
    /// only across the `Vec<Arc<AtomicBool>>` push), so the false-
    /// positive cost is negligible.
    ///
    /// Returns `true` iff there is at least one retired generation that
    /// has not yet drained. Already-drained flags are GC'd lazily here
    /// so a long-lived handle that has cycled through many sessions
    /// does not leak `Arc<AtomicBool>` entries.
    #[must_use]
    pub(crate) fn prev_drained_is_set(&self) -> bool {
        match self.streaming.prev_drained.try_lock() {
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
    /// Pertains to the per-event callback surface: it waits for in-flight user
    /// callbacks of retired sessions to finish firing. The columnar (batches)
    /// pull path has no callback and registers nothing here, so `await_drain`
    /// is a no-op (returns immediately) for a columnar-only client; the
    /// columnar reader's own close / drop is its teardown barrier.
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
    /// callback's wall-clock budget on the slowest in-flight tick. An
    /// extreme timeout whose absolute deadline cannot be represented is
    /// treated as unbounded rather than panicking.
    #[must_use]
    pub(crate) fn await_drain(&self, timeout: Duration) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        loop {
            let all_drained = {
                let mut guard = self
                    .streaming
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
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Whether the calling thread is the live dispatcher thread.
    ///
    /// A caller running inside a per-event callback is ON this thread, so it can
    /// never observe its own drain flag flip (the flag is set only after the
    /// callback returns). Callers guard a blocking [`Self::await_drain`] with
    /// this, mirroring the teardown self-join guard that detaches rather than
    /// joining the dispatcher into itself.
    fn current_thread_is_dispatcher(&self) -> bool {
        matches!(
            &*self.streaming.dispatcher.lock().unwrap_or_else(|e| e.into_inner()),
            DispatcherSession::Running { handle, .. }
                if handle.thread().id() == std::thread::current().id()
        )
    }

    // -- Streaming convenience methods --

    fn with_streaming<R>(
        &self,
        f: impl FnOnce(&StreamingClient) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let snap = self.streaming.state.load();
        match &**snap {
            StreamingSlot::Live { client } => f(client.as_ref()),
            StreamingSlot::Idle | StreamingSlot::Stopped => Err(Error::Stream {
                kind: crate::error::StreamErrorKind::Disconnected,
                message: "streaming not started -- call start_streaming() first".into(),
            }),
        }
    }

    /// Polymorphic subscribe — primary fluent entry point.
    ///
    /// Accepts the typed [`Subscription`] value returned by
    /// [`Contract::quote`] / [`Contract::trade`] /
    /// [`Contract::open_interest`] (per-contract scope) or by
    /// [`crate::streaming::SecTypeExt::full_trades`] /
    /// [`crate::streaming::SecTypeExt::full_open_interest`]
    /// (full-stream scope).
    ///
    /// ```rust,no_run
    /// # use thetadatadx::{Client, Credentials, DirectConfig};
    /// # use thetadatadx::streaming::{Contract, OptionLeg, SecTypeExt};
    /// # use thetadatadx::SecType;
    /// # async fn doc(client: &Client) -> Result<(), thetadatadx::Error> {
    /// let stock  = Contract::stock("AAPL");
    /// let option = Contract::option("SPY", OptionLeg { expiration: "20260620", strike: "550", right: "C" })?;
    /// client.stream().subscribe(stock.quote())?;
    /// client.stream().subscribe(option.trade())?;
    /// client.stream().subscribe(SecType::Option.full_trades())?;
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub(crate) fn subscribe(&self, sub: Subscription) -> Result<(), Error> {
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
    pub(crate) fn subscribe_many<I>(&self, subs: I) -> Result<(), Error>
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
    pub(crate) fn unsubscribe(&self, sub: Subscription) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe(sub.clone()))
    }

    /// Bulk-unsubscribe a batch of [`Subscription`] values. Stops at
    /// the first error.
    ///
    /// # Errors
    ///
    /// Returns an error on the first failed unsubscribe.
    pub(crate) fn unsubscribe_many<I>(&self, subs: I) -> Result<(), Error>
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
    /// Returns [`Error::Stream`] when streaming has not been started.
    pub(crate) fn active_subscriptions(&self) -> Result<Vec<(SubscriptionKind, Contract)>, Error> {
        self.with_streaming(|s| Ok(s.active_subscriptions()))
    }

    /// Get all active full-type (full-stream) subscriptions.
    /// # Errors
    ///
    /// Returns [`Error::Stream`] when streaming has not been started.
    pub(crate) fn active_full_subscriptions(
        &self,
    ) -> Result<Vec<(SubscriptionKind, SecType)>, Error> {
        self.with_streaming(|s| Ok(s.active_full_subscriptions()))
    }

    /// Shut down the streaming connection. Market-data remains available.
    ///
    /// Idempotent: calling on an `Idle` or `Stopped` slot is a no-op,
    /// repeated calls during the drain race are safe (only the first
    /// observer of the `Live` slot performs the shutdown sequence).
    ///
    /// # Asynchronous quiescence
    ///
    /// `stop_streaming` returns as soon as the slot has been swapped
    /// to `Stopped` and the FPSS shutdown signal has been raised. The
    /// I/O thread and the event-dispatch consumer continue running until
    /// they observe the signal, drain the in-flight ring contents
    /// through the user callback, and exit. [`Self::is_streaming`]
    /// flips to `false` immediately on the swap, BEFORE the old
    /// consumer has finished firing.
    ///
    /// Pair with [`Self::await_drain`] for full quiescence semantics
    /// when the caller needs to free a callback context, replace the
    /// callback closure, or otherwise rely on the old user callback
    /// having stopped firing.
    pub(crate) fn stop_streaming(&self) {
        // Single shared teardown: swap the slot to `Stopped`, retire the
        // dispatcher session, record the drain flag. The pull reader's close
        // drives the same [`StreamingState::quiesce`], so the callback and
        // columnar paths leave the client identically truthful and reusable.
        self.streaming.quiesce();
    }

    /// Reconnect the streaming connection, re-subscribing all previous subscriptions.
    ///
    /// This is the caller-driven equivalent of the JVM terminal's
    /// involuntary-disconnect recovery.
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
    /// Returns [`Error::Stream`], [`Error::Auth`], etc. when the underlying
    /// streaming session cannot be re-established (steps 1–3).
    ///
    /// Returns [`Error::PartialReconnect`] when the streaming session was
    /// re-established successfully but one or more saved subscriptions
    /// failed to restore. The variant carries the structured list of failed
    /// `(SubscriptionKind, Contract)` pairs so the caller can retry just
    /// those subscriptions or surface the partial failure to the operator.
    /// Per-subscription `tracing::warn!` lines are still emitted for
    /// operational visibility.
    pub(crate) fn reconnect_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
    {
        // Identity batch scope — see [`Self::reconnect_streaming_scoped`].
        // No teardown wake hook for the same reason as [`Self::start_streaming`].
        self.reconnect_streaming_scoped(handler, |drain| drain(), None)
    }

    /// Reconnect, re-registering `handler` with each consumer batch drain
    /// wrapped in `scope`.
    ///
    /// The reconnect counterpart of [`Self::start_streaming_scoped`]:
    /// saves the active subscriptions, tears the session down, restarts
    /// it via `start_streaming_scoped`, and replays the subscriptions.
    /// `handler` still fires exactly once per event; `scope` brackets
    /// each batch drain on the fresh connection.
    ///
    /// `on_teardown` is the optional dispatcher wakeup hook installed on the
    /// fresh session, carried straight through to
    /// [`Self::start_streaming_scoped`] — see its docs for when it is required.
    /// A binding that needs the hook (e.g. the TypeScript `ThreadsafeFunction`
    /// path) builds a fresh one per reconnect from its persistent callback
    /// handle so the new session's teardown can wake a dispatcher blocked in
    /// the bounded callback queue.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PartialReconnect`] when some subscriptions fail
    /// to restore, or a network / authentication / parsing error on the
    /// restart itself.
    pub(crate) fn reconnect_streaming_scoped<F, S>(
        &self,
        handler: F,
        scope: S,
        on_teardown: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<(), Error>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
        S: FnMut(&mut dyn FnMut() -> crate::PollOutcome) -> crate::PollOutcome + Send + 'static,
    {
        metrics::counter!("thetadatadx.fpss.reconnects").increment(1);
        // 1. Save active subscriptions before stopping
        let saved_subs = match &**self.streaming.state.load() {
            StreamingSlot::Live { client } => (
                client.active_subscriptions(),
                client.active_full_subscriptions(),
            ),
            StreamingSlot::Idle | StreamingSlot::Stopped => (Vec::new(), Vec::new()),
        };

        // 2. Stop streaming
        self.stop_streaming();

        // 3. Start a new streaming connection
        self.start_streaming_scoped(handler, scope, on_teardown)?;

        // 4. Re-subscribe all saved subscriptions (paced), accumulating
        //    failures.
        let (per_contract, full_type) = saved_subs;
        self.restore_subscriptions(&per_contract, &full_type)
    }

    /// Re-subscribe a saved subscription snapshot onto the live
    /// streaming session, paced per the configured replay knobs
    /// ([`crate::config::ReconnectConfig::replay_burst_size`] /
    /// [`crate::config::ReconnectConfig::replay_pace_ms`]).
    ///
    /// This is the single replay engine behind
    /// [`Self::reconnect_streaming`] and the embedded bindings'
    /// reconnect paths: subscriptions are submitted in bursts with a
    /// jittered pause between bursts, so a large saved set is spread
    /// over wall-clock time instead of being fired at a recovering
    /// upstream back-to-back.
    ///
    /// Use [`Self::active_subscriptions`] /
    /// [`Self::active_full_subscriptions`] to capture the snapshot
    /// before tearing the previous session down.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PartialReconnect`] carrying the structured
    /// list of subscriptions that failed to restore; everything not in
    /// the list was re-installed. Per-subscription `tracing::warn!`
    /// lines are emitted for operational visibility.
    pub(crate) fn restore_subscriptions(
        &self,
        per_contract: &[(SubscriptionKind, Contract)],
        full_type: &[(SubscriptionKind, SecType)],
    ) -> Result<(), Error> {
        let reconnect = &self.market_data.config().reconnect;
        let pacing = ReplayPacing {
            burst_size: reconnect.replay_burst_size,
            pace_ms: reconnect.replay_pace_ms,
        };
        let failed = restore_subscriptions(
            per_contract,
            full_type,
            pacing,
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
                // Quote and MarketValue are per-contract only — the
                // vendor has no full-stream broadcast for either, so a
                // full-type restore is a no-op.
                SubscriptionKind::Quote | SubscriptionKind::MarketValue => None,
            },
        );

        if failed.is_empty() {
            Ok(())
        } else {
            Err(Error::PartialReconnect { failed })
        }
    }

    /// Get the current streaming connection status.
    pub(crate) fn connection_status(&self) -> ConnectionStatus {
        match &**self.streaming.state.load() {
            StreamingSlot::Idle => ConnectionStatus::NotStarted,
            StreamingSlot::Stopped => ConnectionStatus::Disconnected,
            StreamingSlot::Live { client } => {
                // The dispatcher thread draining the FPSS iterator is
                // what delivers callbacks. If it panicked, no events
                // will ever arrive even though the I/O thread and ring
                // are still alive. Report as `Disconnected` so callers
                // see a visible failed state instead of "Connected
                // with no deliveries".
                let failed_reason: Option<String> = {
                    let session = self
                        .streaming
                        .dispatcher
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    match &*session {
                        DispatcherSession::Failed { reason } => Some(reason.clone()),
                        _ => None,
                    }
                };
                if let Some(reason) = failed_reason {
                    tracing::debug!(
                        target: "thetadatadx::client",
                        reason = %reason,
                        "connection_status: dispatcher failed",
                    );
                    ConnectionStatus::Disconnected
                } else if client.is_authenticated() {
                    ConnectionStatus::Connected
                } else {
                    // The client exists but is not authenticated — this
                    // happens during reconnection (authenticated flag is
                    // cleared on disconnect, restored on successful re-auth).
                    ConnectionStatus::Reconnecting
                }
            }
        }
    }

    /// Access the current MDDS session UUID.
    ///
    /// Returns an owned `String` rather than `&str` because the UUID
    /// lives behind a shared `crate::auth::SessionToken` that may be
    /// refreshed mid-session. Reads through the token so callers always
    /// see the current value.
    pub async fn session_uuid(&self) -> String {
        self.market_data.session_uuid().await
    }

    /// Access the config.
    pub fn config(&self) -> &DirectConfig {
        self.market_data.config()
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
            stock: tier_label(self.market_data.stock_tier()),
            options: tier_label(self.market_data.options_tier()),
            indices: tier_label(self.market_data.indices_tier()),
            interest_rate: tier_label(self.market_data.interest_rate_tier()),
        }
    }

    // ---------------------------------------------------------------------
    // FLATFILES surface (third public surface, alongside FPSS and MDDS).
    //
    // The legacy MDDS port (12000) speaks a custom binary packet-stream
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
    /// - [`crate::flatfiles::FlatFileFormat::Json`] — a single JSON array of
    ///   the same per-row objects.
    /// - [`crate::flatfiles::FlatFileFormat::Html`] — an HTML `<table>`.
    ///
    /// If `output_path` lacks a file extension, the format's canonical
    /// extension (`csv` / `jsonl` / `json` / `html`) is appended automatically.
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
        crate::flatfiles::flatfile_request_with_config(
            &self.creds,
            sec_type,
            req_type,
            date,
            output_path,
            format,
            &self.flatfiles_config,
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
        crate::flatfiles::flatfile_request_decoded_with_config(
            &self.creds,
            sec_type,
            req_type,
            date,
            &self.flatfiles_config,
        )
        .await
    }

    /// Convenience: option open-interest flat file for `date`.
    ///
    /// # Errors
    /// Same conditions as [`Self::flatfile_request`].
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
    ///
    /// # Errors
    /// Same conditions as [`Self::flatfile_request`].
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

    /// Convenience: option end-of-day flat file for `date`.
    ///
    /// # Errors
    /// Same conditions as [`Self::flatfile_request`].
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
    ///
    /// # Errors
    /// Same conditions as [`Self::flatfile_request`].
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

    /// Convenience: stock end-of-day flat file for `date`.
    ///
    /// # Errors
    /// Same conditions as [`Self::flatfile_request`].
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

impl Drop for Client {
    /// Final cleanup: idempotently stops the streaming connection, off the
    /// dropping thread.
    ///
    /// `stop_streaming` swaps the state cell to `Stopped` and only
    /// signals the FPSS client when the previous slot was `Live`.
    /// The actual TLS reader + event-dispatch consumer join happens when
    /// the last `Arc<StreamingClient>` is dropped via `StreamingClient::Drop`.
    /// Calling once from `Drop` after the user already called
    /// [`Self::close`] / `stop_streaming` is therefore a no-op — the state
    /// machine guarantees the shutdown signal runs exactly once.
    ///
    /// The teardown is **detached** onto a helper thread
    /// (`StreamingState::quiesce_detached`) rather than run inline. A binding
    /// that drops its last client handle while holding a runtime lock the
    /// dispatcher re-enters — the Python GIL is the load-bearing case, since
    /// the dispatcher re-acquires it via `Python::attach` to finish an
    /// in-flight callback — would deadlock on an inline dispatcher join: the
    /// drop thread blocks on the dispatcher, the dispatcher blocks on the lock.
    /// Detaching lets the drop return immediately (releasing that lock) while
    /// the helper finishes the join once the callback drains. Callers that need
    /// a synchronous quiescence barrier use [`Self::close`] (or `stop_streaming`
    /// + `await_drain`), which the bindings wrap in a lock-releasing region.
    fn drop(&mut self) {
        self.streaming.quiesce_detached();
    }
}

impl Client {
    /// Historical data surface — the query endpoints (EOD, history,
    /// snapshots, list, at-time).
    ///
    /// Borrowed view over the unified client's already-open MDDS
    /// channel. Exposes the exact same method set as the standalone
    /// market-data client, so `client.market_data().stock_history_eod(..)`
    /// and a standalone `MarketDataClient::stock_history_eod(..)` are one
    /// surface.
    ///
    /// ```rust,no_run
    /// # use thetadatadx::Client;
    /// # async fn doc(client: &Client) -> Result<(), thetadatadx::Error> {
    /// let eod = client
    ///     .market_data()
    ///     .stock_history_eod("AAPL", "20240101", "20240301")
    ///     .await?;
    /// # let _ = eod;
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub fn market_data(&self) -> &MarketDataClient {
        &self.market_data
    }

    /// Deterministically tear the client down.
    ///
    /// Stops streaming if it is live (idempotent; a no-op when it was never
    /// started or is already stopped). This is the deterministic teardown the
    /// language bindings expose as `close()` / context-manager exit
    /// (`__exit__`, `[Symbol.dispose]`, destructor): it runs on the calling
    /// thread and, unlike the detached [`Drop`] path, retires the streaming
    /// dispatcher synchronously so a subsequent request or reconnect sees a
    /// truthfully-stopped session.
    ///
    /// The market-data gRPC channel pool is released when the last client handle
    /// is dropped: it is an `Arc`-backed set of idle HTTP/2 connections with no
    /// worker thread to join, so its release is RAII, not an explicit signal.
    /// The bindings drop their owning handle on `close()` / context-manager
    /// exit, which is where that pool release becomes deterministic for the
    /// caller.
    ///
    /// Idempotent and safe to call more than once: the streaming state machine
    /// guarantees the shutdown signal fires exactly once.
    ///
    /// For a full callback-quiescence barrier (so a callback context can be
    /// freed) pair `stop_streaming` with [`StreamSurface::await_drain`]; `close`
    /// performs the stop but not the post-stop drain wait, matching the
    /// non-blocking `stop_streaming` contract.
    pub fn close(&self) {
        self.stop_streaming();
    }

    /// Streaming surface — subscribe / unsubscribe, the dispatcher
    /// lifecycle, reconnect, and the back-pressure / health counters.
    ///
    /// Lightweight borrowed view over the unified client's streaming
    /// state machine; constructing it is a pointer copy and performs no
    /// connection work. FPSS connects lazily on the first
    /// [`StreamSurface::start_streaming`].
    ///
    /// ```rust,no_run
    /// # use thetadatadx::Client;
    /// # use thetadatadx::streaming::{Contract, StreamData, StreamEvent};
    /// # fn doc(client: &Client) -> Result<(), thetadatadx::Error> {
    /// client.stream().start_streaming(|event| {
    ///     if let StreamEvent::Data(StreamData::Trade { price, size, .. }) = event {
    ///         println!("trade {price} x {size}");
    ///     }
    /// })?;
    /// client.stream().subscribe(Contract::stock("AAPL").quote())?;
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub fn stream(&self) -> StreamSurface<'_> {
        StreamSurface(self)
    }

    /// Flat-files surface — the bulk per-day whole-universe downloads
    /// (option trade-quote / open-interest / EOD, stock trade-quote /
    /// EOD), each returning decoded rows in memory, plus a generic
    /// dispatcher and a write-to-disk entry.
    ///
    /// Lightweight borrowed view over the unified client's credentials
    /// and flat-files retry config; constructing it is a pointer copy and
    /// performs no connection work. This is the same access shape the
    /// Python, TypeScript, and C++ bindings expose
    /// (`client.flat_files().option_trade_quote(date)`), so a flat-file
    /// pull reads the same across every binding. The lower-level
    /// standalone free functions in [`crate::flatfiles`] remain available
    /// for callers who want to pass credentials and config explicitly.
    ///
    /// ```rust,no_run
    /// # use thetadatadx::Client;
    /// # async fn doc(client: &Client) -> Result<(), thetadatadx::Error> {
    /// let rows = client.flat_files().option_trade_quote("20240115").await?;
    /// # let _ = rows;
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub fn flat_files(&self) -> FlatFiles<'_> {
        FlatFiles {
            creds: &self.creds,
            config: &self.flatfiles_config,
        }
    }
}

/// Borrowed view exposing a client's flat-files surface.
///
/// Returned by [`Client::flat_files`] and
/// [`crate::MarketDataClient::flat_files`]. Holds a shared borrow of the
/// credentials and flat-files retry config the owning client carries — no
/// duplicated state, no behavior change. Each method drives the
/// `crate::flatfiles::*_with_config` engine directly, so both clients reach
/// the identical flat-file distribution over the legacy MDDS port. The view
/// is `Copy`; take it per call site rather than storing it.
///
/// This is the Rust counterpart of the `FlatFiles` / `FlatFilesNamespace`
/// handle on the Python, TypeScript, and C++ bindings: the method set
/// (`option_trade_quote`, `option_open_interest`, `option_eod`,
/// `stock_trade_quote`, `stock_eod`, the generic `request`,
/// and `to_path`) mirrors them exactly, so flat files are reached the same
/// way from every binding and from either client.
#[derive(Clone, Copy)]
pub struct FlatFiles<'a> {
    pub(crate) creds: &'a Credentials,
    pub(crate) config: &'a crate::config::FlatFilesConfig,
}

impl FlatFiles<'_> {
    /// Shared decode leg: pull and decode any served pair for `date`.
    async fn decoded(
        &self,
        sec_type: crate::flatfiles::SecType,
        req_type: crate::flatfiles::ReqType,
        date: &str,
    ) -> Result<Vec<crate::flatfiles::FlatFileRow>, Error> {
        crate::flatfiles::flatfile_request_decoded_with_config(
            self.creds, sec_type, req_type, date, self.config,
        )
        .await
    }

    /// Decoded option trade-quote flat file for `date` (`YYYYMMDD`).
    ///
    /// # Errors
    ///
    /// Returns an error on auth / server rejection, malformed wire bytes,
    /// or local I/O failure.
    pub async fn option_trade_quote(
        &self,
        date: &str,
    ) -> Result<Vec<crate::flatfiles::FlatFileRow>, Error> {
        self.decoded(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::TradeQuote,
            date,
        )
        .await
    }

    /// Decoded option open-interest flat file for `date` (`YYYYMMDD`).
    ///
    /// # Errors
    ///
    /// Same conditions as [`Self::option_trade_quote`].
    pub async fn option_open_interest(
        &self,
        date: &str,
    ) -> Result<Vec<crate::flatfiles::FlatFileRow>, Error> {
        self.decoded(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::OpenInterest,
            date,
        )
        .await
    }

    /// Decoded option end-of-day flat file for `date` (`YYYYMMDD`).
    ///
    /// # Errors
    ///
    /// Same conditions as [`Self::option_trade_quote`].
    pub async fn option_eod(
        &self,
        date: &str,
    ) -> Result<Vec<crate::flatfiles::FlatFileRow>, Error> {
        self.decoded(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::Eod,
            date,
        )
        .await
    }

    /// Decoded stock trade-quote flat file for `date` (`YYYYMMDD`).
    ///
    /// # Errors
    ///
    /// Same conditions as [`Self::option_trade_quote`].
    pub async fn stock_trade_quote(
        &self,
        date: &str,
    ) -> Result<Vec<crate::flatfiles::FlatFileRow>, Error> {
        self.decoded(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::TradeQuote,
            date,
        )
        .await
    }

    /// Decoded stock end-of-day flat file for `date` (`YYYYMMDD`).
    ///
    /// # Errors
    ///
    /// Same conditions as [`Self::option_trade_quote`].
    pub async fn stock_eod(&self, date: &str) -> Result<Vec<crate::flatfiles::FlatFileRow>, Error> {
        self.decoded(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::Eod,
            date,
        )
        .await
    }

    /// Generic dispatcher — pull and decode any served `(sec_type,
    /// req_type)` flat file for `date` (`YYYYMMDD`).
    ///
    /// Useful when the call shape comes from config rather than a static
    /// method choice. The underlying engine rejects a `(sec_type,
    /// req_type)` pair the flat-file distribution does not serve with a
    /// typed invalid-parameter error before any network round-trip.
    ///
    /// # Errors
    ///
    /// Returns an error on an unsupported `(sec_type, req_type)` pair, or
    /// the same conditions as [`Self::option_trade_quote`].
    pub async fn request(
        &self,
        sec_type: crate::flatfiles::SecType,
        req_type: crate::flatfiles::ReqType,
        date: &str,
    ) -> Result<Vec<crate::flatfiles::FlatFileRow>, Error> {
        self.decoded(sec_type, req_type, date).await
    }

    /// Pull a flat-file blob for `(sec_type, req_type, date)` and write
    /// the requested `format` directly to `output_path`, skipping the
    /// typed-row decode.
    ///
    /// Useful when the caller only wants the vendor byte-format CSV /
    /// JSONL on disk and will load it into their own pipeline later. If
    /// `output_path` lacks a file extension, the format's canonical
    /// extension (`csv` / `jsonl`) is appended automatically. Returns the
    /// final on-disk path.
    ///
    /// # Errors
    ///
    /// Returns an error on an unsupported `(sec_type, req_type)` pair,
    /// auth / server rejection, malformed wire bytes, or local I/O
    /// failure.
    pub async fn to_path(
        &self,
        sec_type: crate::flatfiles::SecType,
        req_type: crate::flatfiles::ReqType,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        crate::flatfiles::flatfile_request_with_config(
            self.creds,
            sec_type,
            req_type,
            date,
            output_path,
            format,
            self.config,
        )
        .await
    }
}

/// Borrowed view exposing the unified [`Client`]'s streaming surface.
///
/// Returned by [`Client::stream`]. Holds a shared borrow of the client
/// and forwards every call onto the same atomic streaming state machine
/// the unified client owns — no duplicated state, no behavior change.
/// The view is `Copy`; take it per call site rather than storing it.
#[derive(Clone, Copy)]
pub struct StreamSurface<'a>(&'a Client);

impl StreamSurface<'_> {
    /// Start the FPSS streaming connection with a callback handler.
    ///
    /// The callback fires on the dispatcher thread, one event at a time,
    /// each invocation panic-isolated. It MUST return within
    /// microseconds; hand slow work off to a bounded queue.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn start_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
    {
        self.0.start_streaming(handler)
    }

    /// Start FPSS streaming with a dispatcher teardown wakeup hook.
    ///
    /// Identical to [`Self::start_streaming`] except that `on_teardown` runs
    /// just before the dispatcher join at teardown. A binding whose `handler`
    /// can block off the event ring — notably one that hands each event to a
    /// bounded queue and blocks once full (the TypeScript napi
    /// `ThreadsafeFunction` path) — supplies a hook that releases that block so
    /// the dispatcher can observe the shutdown and the join can complete.
    /// `handler` that parks only on the event ring needs no hook; use the
    /// plain [`Self::start_streaming`].
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn start_streaming_with_teardown<F>(
        &self,
        handler: F,
        on_teardown: Box<dyn FnOnce() + Send>,
    ) -> Result<(), Error>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
    {
        self.0
            .start_streaming_scoped(handler, |drain| drain(), Some(on_teardown))
    }

    /// Start FPSS streaming with each consumer batch drain wrapped in a
    /// caller-supplied `scope`. See [`Self::start_streaming`]; `scope`
    /// brackets each batch drain so a binding can amortise an
    /// interpreter lock across a batch.
    ///
    /// `on_teardown` is the optional dispatcher wakeup hook — see
    /// [`Self::start_streaming_with_teardown`] for when it is required. The
    /// interpreter-lock bindings (e.g. Python) park only on the event ring and
    /// pass `None`.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn start_streaming_scoped<F, S>(
        &self,
        handler: F,
        scope: S,
        on_teardown: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<(), Error>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
        S: FnMut(&mut dyn FnMut() -> crate::PollOutcome) -> crate::PollOutcome + Send + 'static,
    {
        self.0.start_streaming_scoped(handler, scope, on_teardown)
    }

    /// Polymorphic subscribe — primary fluent entry point.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe(&self, sub: Subscription) -> Result<(), Error> {
        self.0.subscribe(sub)
    }

    /// Bulk-subscribe a batch of [`Subscription`] values. Stops at the
    /// first error; previously-installed subscriptions remain active.
    ///
    /// # Errors
    ///
    /// Returns an error on the first failed subscription.
    pub fn subscribe_many<I>(&self, subs: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = Subscription>,
    {
        self.0.subscribe_many(subs)
    }

    /// Polymorphic unsubscribe — fluent counterpart to [`Self::subscribe`].
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe(&self, sub: Subscription) -> Result<(), Error> {
        self.0.unsubscribe(sub)
    }

    /// Bulk-unsubscribe a batch of [`Subscription`] values. Stops at the
    /// first error.
    ///
    /// # Errors
    ///
    /// Returns an error on the first failed unsubscribe.
    pub fn unsubscribe_many<I>(&self, subs: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = Subscription>,
    {
        self.0.unsubscribe_many(subs)
    }

    /// Open a pull-based columnar reader over the live stream — a sibling to
    /// the per-event [`Self::start_streaming`] callback.
    ///
    /// Returns a [`BatchReaderBuilder`](crate::streaming::BatchReaderBuilder)
    /// to tune `batch_size` / `linger` / `backpressure`; call
    /// [`build`](crate::streaming::BatchReaderBuilder::build) to start FPSS
    /// and obtain a
    /// [`RecordBatchStream`](crate::streaming::RecordBatchStream) of Apache
    /// Arrow `RecordBatch` values. Subscriptions are managed on this same
    /// surface ([`Self::subscribe`] / [`Self::subscribe_many`]); building the
    /// reader starts the session, so build first, then subscribe. The batch
    /// schema is fixed for the subscription and identical across every batch.
    ///
    /// Like the callback path, FPSS connects when the reader is built, so
    /// this is an alternative to [`Self::start_streaming`], not a concurrent
    /// consumer of the same session.
    ///
    /// ```rust,no_run
    /// # use thetadatadx::Client;
    /// # use thetadatadx::streaming::Contract;
    /// # use futures::StreamExt;
    /// # async fn doc(client: &Client) -> Result<(), thetadatadx::Error> {
    /// let mut batches = client.stream().batches().batch_size(8_192).build()?;
    /// client.stream().subscribe(Contract::stock("AAPL").trade())?;
    /// while let Some(batch) = batches.next().await {
    ///     let batch = batch?;
    ///     println!("{} rows", batch.num_rows());
    /// }
    /// # Ok(()) }
    /// ```
    #[cfg(feature = "arrow")]
    #[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
    pub fn batches(&self) -> crate::streaming::BatchReaderBuilder<'_> {
        crate::streaming::BatchReaderBuilder::new(self.0)
    }

    /// Get all active per-contract subscriptions.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Stream`] when streaming has not been started.
    pub fn active_subscriptions(&self) -> Result<Vec<(SubscriptionKind, Contract)>, Error> {
        self.0.active_subscriptions()
    }

    /// Get all active full-type (full-stream) subscriptions.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Stream`] when streaming has not been started.
    pub fn active_full_subscriptions(&self) -> Result<Vec<(SubscriptionKind, SecType)>, Error> {
        self.0.active_full_subscriptions()
    }

    /// Shut down the streaming connection. Market-data remains available.
    ///
    /// Idempotent. Returns once the slot has swapped to stopped and the
    /// shutdown signal is raised; pair with [`Self::await_drain`] for
    /// full quiescence before freeing a callback context.
    pub fn stop_streaming(&self) {
        self.0.stop_streaming();
    }

    /// Reconnect the streaming connection, re-subscribing all previous
    /// subscriptions.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PartialReconnect`] when the session re-established
    /// but some subscriptions failed to restore, or a network /
    /// authentication / parsing error on the restart.
    pub fn reconnect_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
    {
        self.0.reconnect_streaming(handler)
    }

    /// Reconnect with a dispatcher teardown wakeup hook on the fresh session.
    ///
    /// The reconnect counterpart of [`Self::start_streaming_with_teardown`]:
    /// tears the old session down, restarts under `handler`, and installs
    /// `on_teardown` on the new session so a later teardown can wake a
    /// dispatcher blocked off the event ring (the TypeScript
    /// `ThreadsafeFunction` path). The hook is consumed by this one reconnect;
    /// a binding that reconnects again builds a fresh hook from its persistent
    /// callback handle.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PartialReconnect`] when some subscriptions fail to
    /// restore, or a network / authentication / parsing error.
    pub fn reconnect_streaming_with_teardown<F>(
        &self,
        handler: F,
        on_teardown: Box<dyn FnOnce() + Send>,
    ) -> Result<(), Error>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
    {
        self.0
            .reconnect_streaming_scoped(handler, |drain| drain(), Some(on_teardown))
    }

    /// Reconnect, re-registering `handler` with each consumer batch drain
    /// wrapped in `scope`.
    ///
    /// `on_teardown` is the optional dispatcher wakeup hook installed on the
    /// fresh session — see [`Self::reconnect_streaming_with_teardown`]. The
    /// interpreter-lock bindings (e.g. Python) pass `None`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PartialReconnect`] when some subscriptions fail to
    /// restore, or a network / authentication / parsing error.
    pub fn reconnect_streaming_scoped<F, S>(
        &self,
        handler: F,
        scope: S,
        on_teardown: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<(), Error>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
        S: FnMut(&mut dyn FnMut() -> crate::PollOutcome) -> crate::PollOutcome + Send + 'static,
    {
        self.0
            .reconnect_streaming_scoped(handler, scope, on_teardown)
    }

    /// Re-subscribe a saved subscription snapshot onto the live session,
    /// paced per the configured replay knobs.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PartialReconnect`] carrying the list of
    /// subscriptions that failed to restore.
    pub fn restore_subscriptions(
        &self,
        per_contract: &[(SubscriptionKind, Contract)],
        full_type: &[(SubscriptionKind, SecType)],
    ) -> Result<(), Error> {
        self.0.restore_subscriptions(per_contract, full_type)
    }

    /// Get the current streaming connection status.
    #[must_use]
    pub fn connection_status(&self) -> ConnectionStatus {
        self.0.connection_status()
    }

    /// Whether streaming is currently active. See [`Client`] notes: this
    /// flips immediately on stop, BEFORE the previous callback drains.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        self.0.is_streaming()
    }

    /// Whether the live streaming session is currently authenticated.
    ///
    /// Distinct from [`Self::is_streaming`]: the session can be live yet
    /// briefly unauthenticated mid-reconnect (the authenticated flag is
    /// cleared on disconnect and restored on a successful re-auth).
    /// Returns `false` before streaming starts and after it stops.
    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.0.is_authenticated()
    }

    /// Wait for every retired streaming session to drain. Returns `false`
    /// on timeout.
    #[must_use]
    pub fn await_drain(&self, timeout: Duration) -> bool {
        self.0.await_drain(timeout)
    }

    /// Whether the calling thread is the streaming dispatcher thread.
    ///
    /// A caller inside a per-event callback runs ON the dispatcher thread, so it
    /// can never observe its own drain flag flip (that flag is set only after the
    /// callback returns). A blocking [`Self::await_drain`] from that thread would
    /// spin to its full timeout; bindings whose `close` path can be reached from
    /// inside a callback guard the drain with this, mirroring the teardown
    /// self-join guard that detaches rather than joining the dispatcher into
    /// itself.
    #[must_use]
    pub fn current_thread_is_dispatcher(&self) -> bool {
        self.0.current_thread_is_dispatcher()
    }

    /// Whether a prior streaming session has registered a drain flag for
    /// [`Self::await_drain`] to poll on.
    #[must_use]
    pub fn prev_drained_is_set(&self) -> bool {
        self.0.prev_drained_is_set()
    }

    /// Snapshot of events dropped because the consumer fell behind and
    /// the ring was full. `0` when streaming has not started.
    #[must_use]
    pub fn dropped_event_count(&self) -> u64 {
        self.0.dropped_event_count()
    }

    /// In-flight depth between the I/O thread and the callback. `0` when
    /// streaming has not started.
    #[must_use]
    pub fn ring_occupancy(&self) -> usize {
        self.0.ring_occupancy()
    }

    /// Configured capacity of the streaming event ring in slots. `0`
    /// when streaming has not started.
    #[must_use]
    pub fn ring_capacity(&self) -> usize {
        self.0.ring_capacity()
    }

    /// Milliseconds since the most recent inbound streaming frame, or
    /// `None` when streaming has not started or no frame arrived yet.
    #[must_use]
    pub fn millis_since_last_event(&self) -> Option<u64> {
        self.0.millis_since_last_event()
    }

    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// frame. `0` when streaming has not started or no frame arrived yet.
    #[must_use]
    pub fn last_event_received_at_unix_nanos(&self) -> i64 {
        self.0.last_event_received_at_unix_nanos()
    }

    /// Address (`host:port`) of the streaming server the session is
    /// connected to. `None` when streaming has not started.
    #[must_use]
    pub fn last_connected_addr(&self) -> Option<String> {
        self.0.last_connected_addr()
    }

    /// Cumulative count of user-callback faults. `0` when streaming has
    /// not started.
    #[must_use]
    pub fn panic_count(&self) -> u64 {
        self.0.panic_count()
    }

    /// Increment the panic counter on the live session by one.
    ///
    /// Called by language-binding dispatchers when the user callback
    /// raises a language-level exception that bypasses the Rust
    /// `catch_unwind` boundary. No-op when streaming is not active.
    #[cfg(feature = "__internal")]
    #[doc(hidden)]
    pub fn record_panic(&self) {
        self.0.record_panic();
    }
}

/// Replay every saved subscription against the freshly reconnected
/// streaming client and return the list of subscriptions that failed to
/// restore.
///
/// The two callbacks decouple the loop from the live `Client`
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
pub(crate) fn restore_subscriptions<P, F>(
    per_contract: &[(SubscriptionKind, Contract)],
    full_type: &[(SubscriptionKind, SecType)],
    pacing: ReplayPacing,
    mut per_subscribe: P,
    mut full_subscribe: F,
) -> Vec<(SubscriptionKind, Contract)>
where
    P: FnMut(SubscriptionKind, &Contract) -> Result<(), Error>,
    F: FnMut(SubscriptionKind, SecType) -> Option<Result<(), Error>>,
{
    let mut failed: Vec<(SubscriptionKind, Contract)> = Vec::new();
    let mut submitted_in_burst: u32 = 0;
    let burst_size = pacing.burst_size.max(1);

    let pace = |submitted_in_burst: &mut u32| {
        *submitted_in_burst += 1;
        if *submitted_in_burst >= burst_size {
            *submitted_in_burst = 0;
            if pacing.pace_ms > 0 {
                // ±20% jitter on the inter-burst pause so a fleet of
                // simultaneously-reconnecting clients does not submit
                // replay bursts in phase.
                let pace = Duration::from_millis(pacing.pace_ms);
                std::thread::sleep(crate::backoff::uniform_duration(
                    pace.mul_f64(0.8),
                    pace.mul_f64(1.2),
                ));
            }
        }
    };

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
        pace(&mut submitted_in_burst);
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
        pace(&mut submitted_in_burst);
    }

    failed
}

/// Burst/pause pacing for a subscription replay. Mirrors
/// [`crate::config::ReconnectConfig::replay_burst_size`] /
/// [`crate::config::ReconnectConfig::replay_pace_ms`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReplayPacing {
    pub(crate) burst_size: u32,
    pub(crate) pace_ms: u64,
}

impl ReplayPacing {
    /// Pacing disabled — submit back-to-back. Test-only shape; the
    /// production paths read the configured knobs.
    #[cfg(test)]
    fn unpaced() -> Self {
        Self {
            burst_size: u32::MAX,
            pace_ms: 0,
        }
    }
}

/// Downcast a thread-panic payload to a human-readable string.
///
/// Tries `&str` first (the common `panic!("…")` case), then `String`
/// (the `panic!("{}", x)` case), and falls back to a fixed message for
/// exotic payload types.
fn downcast_panic_payload(payload: Box<dyn std::any::Any + Send>) -> String {
    panic_reason(payload.as_ref())
}

/// Borrow-based twin of [`downcast_panic_payload`]: extract the panic message
/// without consuming the payload, so a caller that must re-raise the panic
/// (`resume_unwind`) can still record a reason first.
fn panic_reason(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_owned();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "dispatcher panicked with non-string payload".to_owned()
}

/// Record [`DispatcherSession::Failed`] from the dispatcher thread itself when
/// its drain loop ended on an I/O-thread fault (the callback drain returned
/// [`crate::PollOutcome::Failed`]), so `is_streaming` / `connection_status`
/// flip immediately rather than only after a later teardown joins the exited
/// thread. Mirrors the pull path, where the next `next_event` surfaces
/// [`crate::streaming::StreamError::DispatcherFailed`].
///
/// Writes ONLY while the slot still holds THIS thread's `Running` session:
/// called from the dispatcher thread, so `std::thread::current()` is that
/// dispatcher. A concurrent stop/reconnect may have extracted the session
/// (slot `Idle`, its own join about to record the fault) or installed a fresh
/// one; in either case the fault belongs to the now-superseded session and
/// overwriting the slot would clobber a newer `JoinHandle` or falsely fail a
/// healthy session. Matching the stored handle's thread id to the current
/// thread pins the write to the un-superseded case.
fn record_dispatcher_failed_if_current(
    dispatcher: &std::sync::Mutex<DispatcherSession>,
    reason: String,
) {
    let mut guard = dispatcher.lock().unwrap_or_else(|e| e.into_inner());
    let is_own_running = matches!(
        &*guard,
        DispatcherSession::Running { handle, .. }
            if handle.thread().id() == std::thread::current().id()
    );
    if is_own_running {
        *guard = DispatcherSession::Failed { reason };
    }
}

/// Grace window a teardown gives the dispatcher to exit on its own — by
/// observing the ring shutdown — before the wake hook is fired.
///
/// A dispatcher whose consumer loop is not parked off the event ring returns
/// from its body within microseconds of `client.shutdown()`, so it is observed
/// finished almost immediately and the wake hook never runs. A dispatcher
/// parked off the ring — the columnar pull dispatcher in its bounded-queue
/// `flush` wait, or a binding's per-event handler blocked in a full bounded
/// callback queue — does not finish on its own, so it is still running when this
/// window elapses and the wake hook is fired to release it.
///
/// Firing the hook only as a fallback (rather than unconditionally) is required
/// because some wakes are destructive and the woken resource may be re-used: the
/// TypeScript `ThreadsafeFunction` abort permanently makes the function reject
/// calls, yet `reconnect` re-registers that same function, so aborting it on
/// every stop would leave a reconnected session unable to deliver events. A
/// dispatcher that exits cleanly is joined before the grace elapses, so the
/// destructive wake runs only when it is the sole way to break a real deadlock.
const DISPATCHER_TEARDOWN_WAKE_GRACE: Duration = Duration::from_millis(250);

/// Poll cadence for [`DISPATCHER_TEARDOWN_WAKE_GRACE`].
const DISPATCHER_TEARDOWN_POLL: Duration = Duration::from_millis(1);

/// Join a dispatcher thread, firing its teardown wake hook only if it does not
/// exit on its own within [`DISPATCHER_TEARDOWN_WAKE_GRACE`].
///
/// The caller must have already signalled the client shutdown (so a dispatcher
/// parked on the event ring is on its way out) and must not be the dispatcher
/// thread itself. Returns the [`std::thread::JoinHandle::join`] result so the
/// caller can record a dispatcher panic.
fn join_dispatcher_with_wake(
    handle: std::thread::JoinHandle<()>,
    on_teardown: Option<Box<dyn FnOnce() + Send>>,
) -> std::thread::Result<()> {
    let deadline = Instant::now() + DISPATCHER_TEARDOWN_WAKE_GRACE;
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            // Still running after the grace window: the dispatcher is parked off
            // the event ring. Fire the wake hook to release it, then fall
            // through to the blocking join, which now completes.
            if let Some(wake) = on_teardown {
                wake();
            }
            break;
        }
        std::thread::sleep(DISPATCHER_TEARDOWN_POLL);
    }
    handle.join()
}

#[cfg(test)]
mod tests {
    use crate::fpss::protocol::OptionLeg;

    use super::*;

    /// Lightweight stand-in for `StreamingSlot` carrying just enough
    /// shape to walk the state machine transitions without spinning up
    /// a real FPSS connection. The transitions and the `ArcSwap`
    /// install/swap mechanics are what we are validating; the live
    /// payload (`StreamingClient`, `StreamingDispatcher`) is exercised by
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

    /// Compile-level proof that [`Client::flat_files`] returns a
    /// [`FlatFiles`] view and that each method resolves against the right
    /// argument and return types — mirroring the flat-files surface the
    /// Python, TypeScript, and C++ bindings expose. No network call is
    /// made: each method's returned future is fed into a type assertion
    /// that pins its `Output`, so the signatures are checked at compile
    /// time without ever polling the future.
    #[allow(unused)]
    fn flat_files_view_surface_compiles(client: &Client) {
        use std::future::Future;

        // Pin a future's `Output` to the decoded-rows result without
        // awaiting it: constructing the call site is enough to type-check
        // the method signature.
        fn assert_rows<F>(_: F)
        where
            F: Future<Output = Result<Vec<crate::flatfiles::FlatFileRow>, Error>>,
        {
        }
        fn assert_path<F>(_: F)
        where
            F: Future<Output = Result<std::path::PathBuf, Error>>,
        {
        }

        let view: FlatFiles<'_> = client.flat_files();
        // The view is `Copy` — taking it again is a pointer copy.
        let copy: FlatFiles<'_> = view;

        // Decoded terminals: `&str` date in, `Vec<FlatFileRow>` out.
        assert_rows(view.option_trade_quote("20240115"));
        assert_rows(view.option_open_interest("20240115"));
        assert_rows(view.option_eod("20240115"));
        assert_rows(view.stock_trade_quote("20240115"));
        assert_rows(copy.stock_eod("20240115"));

        // Generic decoded dispatcher: typed enums in.
        assert_rows(view.request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::TradeQuote,
            "20240115",
        ));

        // Write-to-disk: typed enums + path + format in, `PathBuf` out.
        assert_path(view.to_path(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::Eod,
            "20240115",
            std::path::Path::new("/tmp/out.csv"),
            crate::flatfiles::FlatFileFormat::Csv,
        ));
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

    /// [`StreamingState::quiesce`] is the single teardown both delivery modes
    /// share. From a never-started state it leaves the slot `Stopped`, the
    /// dispatcher `Idle`, and bumps the stop generation (the resurrection
    /// guard). This is the state the pull reader's close now lands the client
    /// in — truthful and reusable — instead of leaving a stale `Live` slot and
    /// a `Running` dispatcher with an exited thread. The `Live`-slot shutdown
    /// branch shares its body with the callback path's `stop_streaming`, which
    /// the FPSS integration tests exercise against a real connection.
    #[test]
    fn quiesce_from_idle_lands_stopped_and_idle() {
        let state = StreamingState::new();
        assert_eq!(state.slot_label(), "Idle");
        assert!(state.dispatcher_is_idle());
        let gen0 = state.stop_generation();

        state.quiesce();

        assert_eq!(
            state.slot_label(),
            "Stopped",
            "quiesce must swap the slot to Stopped"
        );
        assert!(
            state.dispatcher_is_idle(),
            "quiesce must leave the dispatcher Idle"
        );
        assert!(
            state.stop_generation() > gen0,
            "quiesce must bump the stop generation (resurrection guard)"
        );

        // Idempotent: a second close (e.g. binding close() then the core Drop)
        // is a harmless no-op beyond bumping the generation again.
        let gen1 = state.stop_generation();
        state.quiesce();
        assert_eq!(state.slot_label(), "Stopped");
        assert!(state.dispatcher_is_idle());
        assert!(state.stop_generation() > gen1);
    }

    /// `quiesce` retires a `Running` dispatcher session — joining its thread
    /// and resetting it to `Idle` — which is what makes the client reusable
    /// after a pull reader closes: a later `start_streaming*` / `batches()`
    /// installs a fresh dispatcher instead of finding a stale `Running` one
    /// behind an exited thread. Driven with a real (immediately-exiting)
    /// thread and the slot left out of `Live` so the join branch is what is
    /// under test, deterministically and without a network.
    #[test]
    fn quiesce_retires_a_running_dispatcher() {
        let state = StreamingState::new();
        let handle = std::thread::spawn(|| { /* exits immediately */ });
        *state.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = DispatcherSession::Running {
            handle,
            on_teardown: None,
            registers_drain_flag: true,
        };
        assert!(
            !state.dispatcher_is_idle(),
            "precondition: Running installed"
        );

        state.quiesce();

        assert!(
            state.dispatcher_is_idle(),
            "quiesce must join the dispatcher thread and reset the session to \
             Idle so the client is reusable"
        );
    }

    /// The `Client` drop path (`quiesce_detached`) must NOT block the calling
    /// thread on the dispatcher join — that is what makes a forgetful drop
    /// deadlock-safe when the dropping thread holds a runtime lock the
    /// dispatcher re-enters (the Python GIL). Modelled without Python: a
    /// dispatcher thread parks on a gate the caller controls, is installed as
    /// the `Running` session, and `quiesce_detached` is driven on the "GIL"
    /// thread. The assertion is that the call RETURNS while the dispatcher is
    /// still parked — an inline join would block here until the gate opens, so a
    /// prompt return proves the join ran on the detached helper instead. Only
    /// after the caller opens the gate does the detached helper's join complete.
    #[test]
    fn drop_detaches_the_dispatcher_join_off_the_calling_thread() {
        use std::sync::atomic::{AtomicBool, Ordering as O};

        // Gate the dispatcher thread the way an in-flight callback awaiting the
        // GIL would: it will not exit (so the join cannot complete) until the
        // caller releases it.
        let gate = Arc::new(AtomicBool::new(false));
        let gate_t = Arc::clone(&gate);
        let dispatcher = std::thread::spawn(move || {
            while !gate_t.load(O::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        let state = Arc::new(StreamingState::new());
        *state.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = DispatcherSession::Running {
            handle: dispatcher,
            on_teardown: None,
            registers_drain_flag: false,
        };

        // The "GIL" thread's drop. If the join were inline this call would block
        // until the gate opens; it must return promptly instead.
        let start = std::time::Instant::now();
        state.quiesce_detached();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "quiesce_detached blocked on the dispatcher join instead of \
             detaching it; a forgetful drop under a held GIL would deadlock",
        );
        assert!(
            !gate.load(O::Acquire),
            "precondition: dispatcher is still parked, so the prompt return \
             above proves the join was detached rather than already complete",
        );

        // Release the parked dispatcher; the detached helper's join now
        // completes and the session retires to Idle.
        gate.store(true, O::Release);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !state.dispatcher_is_idle() {
            assert!(
                std::time::Instant::now() < deadline,
                "detached teardown helper never retired the dispatcher session \
                 after the gate opened",
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// `quiesce` / `close` are idempotent and safe to call repeatedly on a
    /// never-started (and already-stopped) streaming state: the slot walks to
    /// `Stopped`, stays there, and each call bumps the stop generation without
    /// panicking or blocking. This is the base-client `close()` contract —
    /// calling it twice, or on a client that only ran market-data queries, is a
    /// no-op teardown.
    #[test]
    fn quiesce_is_idempotent_on_a_never_started_state() {
        let state = StreamingState::new();
        assert_eq!(state.slot_label(), "Idle");

        state.quiesce();
        let gen1 = state.stop_generation();
        assert_eq!(state.slot_label(), "Stopped");
        assert!(state.dispatcher_is_idle());

        // Second close: still safe, still Stopped, generation advances.
        state.quiesce();
        assert_eq!(state.slot_label(), "Stopped");
        assert!(state.dispatcher_is_idle());
        assert!(state.stop_generation() > gen1);
    }

    /// quiesce must not deadlock joining a columnar dispatcher parked in its
    /// bounded-queue `Block` flush wait on a teardown that bypasses the
    /// reader's own close (a `Client` drop / `stop_streaming` /
    /// `reconnect_streaming`, which call quiesce DIRECTLY). The FPSS shutdown
    /// signals the event ring but not the batch queue, so the only thing that
    /// releases the parked flush is the wake hook quiesce runs before the join.
    ///
    /// This builds the exact deadlock geometry without a network: a real
    /// producer thread fills a depth-1 `Block` queue and parks in `flush`, that
    /// thread is installed as the `Running` dispatcher with the columnar wake
    /// hook (`Shared::close_and_wake`), and quiesce is driven on a side thread
    /// under a watchdog. With the hook the parked flush is woken, the producer
    /// thread exits, and the join completes well within the deadline; without
    /// it (the pre-fix `on_teardown: None`) the producer stays parked and the
    /// join hangs past the watchdog, failing the test instead of wedging the
    /// suite.
    #[cfg(feature = "arrow")]
    #[test]
    fn quiesce_does_not_deadlock_on_a_parked_block_dispatcher() {
        use crate::fpss::batch_reader::test_harness::{harness, trade};
        use crate::fpss::batch_reader::Backpressure;
        use crate::fpss::protocol::Contract;
        use std::sync::atomic::{AtomicBool, Ordering as O};

        // Depth-1 Block queue, no reader: the producer fills it then parks.
        let (producer, reader) =
            harness(1, std::time::Duration::from_millis(50), Backpressure::Block);
        let shared = reader.shared_handle();
        let contract = Arc::new(Contract::stock("SPY"));

        // Producer thread: floods the queue and parks in `flush`. After the
        // wake hook fires it returns promptly (post-close flushes drop and
        // return), so the join can complete.
        let feeder = std::thread::spawn(move || {
            for i in 0..256 {
                producer.feed(&trade(&contract, i));
            }
        });
        // Let the producer fill the depth-1 queue and park in the Block wait.
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Install that real thread as the columnar dispatcher session, WITH the
        // wake hook the live columnar path installs.
        let state = Arc::new(StreamingState::new());
        let wake_shared = Arc::clone(&shared);
        *state.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = DispatcherSession::Running {
            handle: feeder,
            on_teardown: Some(Box::new(move || wake_shared.close_and_wake())),
            // Columnar-shaped session in this test, but the assertion is on the
            // join completing, not on drain-flag registration.
            registers_drain_flag: false,
        };

        // Drive quiesce on a side thread so a hung join is caught by the
        // watchdog rather than wedging the whole test binary.
        let done = Arc::new(AtomicBool::new(false));
        let done_t = Arc::clone(&done);
        let state_t = Arc::clone(&state);
        let quiescer = std::thread::spawn(move || {
            state_t.quiesce();
            done_t.store(true, O::Release);
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done.load(O::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "quiesce deadlocked joining a parked Block dispatcher: the \
                 teardown wake hook did not release the parked flush"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        quiescer.join().expect("quiescer thread");
        assert!(
            state.dispatcher_is_idle(),
            "after quiesce the dispatcher session is retired to Idle"
        );
    }

    /// Teardown must not deadlock when a user callback firing during the
    /// post-shutdown drain calls into the client (e.g. `is_streaming()`), which
    /// takes the dispatcher lock. The client shutdown keeps draining
    /// already-buffered events through callbacks until the shutdown is observed;
    /// if the teardown held the dispatcher lock across the dispatcher join, such
    /// a callback would block on the lock forever, the dispatcher would never
    /// exit, and the join would never return.
    ///
    /// This models that exactly without a network: the dispatcher thread, once
    /// the teardown signals it (via the wake hook, standing in for the shutdown
    /// being observed), calls `dispatcher_is_idle()` — which acquires and
    /// releases the dispatcher lock, exactly as `is_streaming()` does — and only
    /// then exits. The teardown runs on a side thread under a watchdog and must
    /// COMPLETE: with shutdown + join OUTSIDE the lock the callback acquires the
    /// free lock, the thread exits, and the join returns. If the join held the
    /// lock (the pre-fix regression) the callback would block and the watchdog
    /// would fire.
    #[test]
    fn teardown_does_not_deadlock_when_a_callback_takes_the_dispatcher_lock() {
        use std::sync::atomic::{AtomicBool, Ordering as O};

        let state = Arc::new(StreamingState::new());

        // The wake hook stands in for `client.shutdown()` being observed by the
        // dispatcher: it releases the dispatcher thread to run its "callback".
        let released = Arc::new(AtomicBool::new(false));

        // Dispatcher thread: wait until teardown releases it, then do exactly
        // what an `is_streaming()` call in a user callback does — take and drop
        // the dispatcher lock — before exiting. If teardown still holds the lock
        // at that point, this blocks forever.
        let dispatcher = {
            let state = Arc::clone(&state);
            let released = Arc::clone(&released);
            std::thread::spawn(move || {
                while !released.load(O::Acquire) {
                    std::hint::spin_loop();
                }
                // Acquires + releases the dispatcher lock, like is_streaming().
                let _ = state.dispatcher_is_idle();
            })
        };

        let wake_released = Arc::clone(&released);
        *state.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = DispatcherSession::Running {
            handle: dispatcher,
            on_teardown: Some(Box::new(move || wake_released.store(true, O::Release))),
            registers_drain_flag: true,
        };

        // Drive teardown on a side thread so a hung join is caught by the
        // watchdog rather than wedging the suite.
        let done = Arc::new(AtomicBool::new(false));
        let done_t = Arc::clone(&done);
        let state_t = Arc::clone(&state);
        let teardown = std::thread::spawn(move || {
            state_t.quiesce();
            done_t.store(true, O::Release);
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done.load(O::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "teardown deadlocked: it held the dispatcher lock across the \
                 dispatcher join while a callback was blocked acquiring that lock"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        teardown.join().expect("teardown thread");
        assert!(state.dispatcher_is_idle());
    }

    /// A join panic on the OLD dispatcher must not clobber a NEWER session
    /// installed during the lock-free join window. `run_teardown` releases the
    /// dispatcher lock across the join, so a `start_dispatcher` can install a
    /// fresh `Running` session before the join returns. If the old dispatcher
    /// then panics, the `Failed` write must NOT overwrite that newer session —
    /// it would orphan the new session's `JoinHandle` and report a healthy live
    /// session as failed. The write is now conditional on the slot still being
    /// `Idle` (the state extract left it), so a newer session is left alone.
    ///
    /// Modeled deterministically: build a `TeardownWork` whose `Running`
    /// session's handle panics only after a NEW session has been installed,
    /// then run the teardown; the new session must still be `Running`
    /// afterward.
    #[test]
    fn join_panic_does_not_clobber_a_newer_session() {
        use std::sync::atomic::{AtomicBool, Ordering as O};

        let state = Arc::new(StreamingState::new());

        // The teardown wake hook fires just before the join; the test uses it
        // as the signal that the join window has opened.
        let join_window_open = Arc::new(AtomicBool::new(false));
        // Set by the test once the NEW session is installed; the old handle
        // waits for it, then panics, so the new session is guaranteed in place
        // before the join returns and the conditional `Failed` write runs.
        let new_installed = Arc::new(AtomicBool::new(false));

        // OLD dispatcher handle: wait until the new session is installed, then
        // panic so `handle.join()` returns Err.
        let old_handle = {
            let new_installed = Arc::clone(&new_installed);
            std::thread::spawn(move || {
                while !new_installed.load(O::Acquire) {
                    std::hint::spin_loop();
                }
                panic!("old dispatcher panicked in event-iteration machinery");
            })
        };
        let work = TeardownWork {
            client: None,
            session: DispatcherSession::Running {
                handle: old_handle,
                on_teardown: Some({
                    let join_window_open = Arc::clone(&join_window_open);
                    Box::new(move || join_window_open.store(true, O::Release))
                }),
                registers_drain_flag: true,
            },
        };

        // Run the teardown on a side thread (its join blocks until the old
        // handle panics, which the test gates below).
        let done = Arc::new(AtomicBool::new(false));
        let teardown = {
            let state = Arc::clone(&state);
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                state.run_teardown(work);
                done.store(true, O::Release);
            })
        };

        // Wait until the teardown opens the join window (wake hook fired), then
        // install a NEW Running session into the slot the extract left Idle,
        // exactly as a racing `start_dispatcher` would.
        while !join_window_open.load(O::Acquire) {
            std::hint::spin_loop();
        }
        let new_handle = std::thread::spawn(|| {});
        *state.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = DispatcherSession::Running {
            handle: new_handle,
            on_teardown: None,
            registers_drain_flag: true,
        };
        // Release the old handle to panic; the join now returns Err and the
        // conditional `Failed` write runs against the slot holding the NEW
        // Running session.
        new_installed.store(true, O::Release);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done.load(O::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "teardown did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        teardown.join().expect("teardown thread");

        // The newer session must survive: a join panic on the superseded old
        // session must not flip the live session to Failed.
        let guard = state.dispatcher.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            matches!(*guard, DispatcherSession::Running { .. }),
            "a newer session installed during the join window must not be \
             clobbered by the old dispatcher's join-panic Failed write"
        );
        drop(guard);
        // Tidy: retire the surviving session so its thread is joined.
        state.quiesce();
    }

    /// `await_drain` must not be made to time out spuriously by the columnar
    /// path: a retiring columnar session has no user callback to wait for, so
    /// it must NOT register a drain flag, whereas a callback session MUST. This
    /// pins `session_registers_drain_flag` (the single decision the teardown
    /// uses) for both session shapes, and models the `await_drain` consequence
    /// on the retired-generations Vec (the production `await_drain` runs the
    /// same `retain` + `is_empty` cadence over it; building a real Live session
    /// needs a network credential, so the Vec is exercised directly, as the
    /// other `await_drain` tests do).
    #[test]
    fn columnar_session_does_not_register_a_drain_flag() {
        // The decision is the explicit `registers_drain_flag` field, NOT the
        // presence of a wake hook: a columnar session sets it `false`, a
        // callback session `true`. A callback session may now ALSO carry a hook
        // (the TypeScript abort), so this case pins that a hook-bearing callback
        // session still registers its drain flag.
        let columnar = DispatcherSession::Running {
            handle: std::thread::spawn(|| {}),
            on_teardown: Some(Box::new(|| {})),
            registers_drain_flag: false,
        };
        let callback = DispatcherSession::Running {
            handle: std::thread::spawn(|| {}),
            on_teardown: Some(Box::new(|| {})),
            registers_drain_flag: true,
        };
        assert!(
            !session_registers_drain_flag(&columnar),
            "a columnar session has no callback to drain-wait for"
        );
        assert!(
            session_registers_drain_flag(&callback),
            "a callback session must register its drain flag for await_drain"
        );
        // A non-Running session has nothing to wait on.
        assert!(!session_registers_drain_flag(&DispatcherSession::Idle));

        // The await_drain consequence, modeled on the retired-generations Vec.
        // Columnar teardown registers nothing, so await_drain (retain +
        // is_empty) reports drained immediately — no spurious timeout while a
        // closed-but-not-dropped columnar reader is alive.
        let prev_drained: Mutex<Vec<Arc<AtomicBool>>> = Mutex::new(Vec::new());
        if session_registers_drain_flag(&columnar) {
            prev_drained
                .lock()
                .unwrap()
                .push(Arc::new(AtomicBool::new(false)));
        }
        {
            let mut g = prev_drained.lock().unwrap();
            g.retain(|f| !f.load(Ordering::Acquire));
            assert!(
                g.is_empty(),
                "columnar teardown must leave nothing for await_drain to block on"
            );
        }

        // Callback teardown registers a still-unflipped flag, so await_drain
        // correctly does NOT report drained until the callback finishes (no
        // regression to the callback path).
        if session_registers_drain_flag(&callback) {
            prev_drained
                .lock()
                .unwrap()
                .push(Arc::new(AtomicBool::new(false)));
        }
        {
            let mut g = prev_drained.lock().unwrap();
            g.retain(|f| !f.load(Ordering::Acquire));
            assert_eq!(
                g.len(),
                1,
                "callback teardown registers a flag await_drain must wait on"
            );
        }

        // Detach the trivial threads.
        for s in [columnar, callback] {
            if let DispatcherSession::Running { handle, .. } = s {
                let _ = handle.join();
            }
        }
    }

    /// A columnar reader's close must tear down ONLY the session it started.
    /// `RecordBatchStream::close_shared` calls
    /// `state.quiesce_if_owned(self.owned_generation)`; this exercises that
    /// method directly against the mixed-mode supersession sequence.
    ///
    /// Sequence modeled: reader R starts its session and stamps the generation
    /// it owns; R's session is then retired (a `stop_streaming`, which advances
    /// the generation); a NEW session is installed on the same state (the
    /// callback session a caller can legitimately start once the slot is no
    /// longer `Live`). When R is finally dropped, its stamp no longer matches
    /// the live generation, so `quiesce_if_owned` must NOT retire the newer
    /// session.
    #[test]
    fn stale_generation_reader_does_not_tear_down_a_newer_session() {
        let state = StreamingState::new();

        // R installs its session and stamps the generation it owns.
        let r_handle = std::thread::spawn(|| {});
        *state.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = DispatcherSession::Running {
            handle: r_handle,
            on_teardown: None,
            registers_drain_flag: true,
        };
        let r_owned_generation = state.stop_generation();

        // R's session is retired by a stop (advances the generation).
        state.quiesce();
        assert!(state.dispatcher_is_idle());
        assert_ne!(
            state.stop_generation(),
            r_owned_generation,
            "the stop must advance the generation past R's stamp"
        );

        // A NEW session is installed on the same client (e.g. a callback
        // session, valid now that the slot is no longer Live).
        let newer_handle = std::thread::spawn(|| {});
        *state.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = DispatcherSession::Running {
            handle: newer_handle,
            on_teardown: None,
            registers_drain_flag: true,
        };

        // R's drop runs its close gate against its stale stamp: it must leave
        // the newer session alone.
        state.quiesce_if_owned(r_owned_generation);
        assert!(
            !state.dispatcher_is_idle(),
            "R's stale close must leave the newer session Running, not retire it"
        );

        // Tidy up the still-Running newer session so its thread is joined.
        state.quiesce();
    }

    /// The matching-generation reader (the normal single-reader close) DOES
    /// retire: when no teardown has advanced the generation, the live session
    /// is still the one this reader started, so `quiesce_if_owned` retires it.
    /// Guards against the stamp over-suppressing the common path.
    #[test]
    fn matching_generation_reader_quiesces_its_own_session() {
        let state = StreamingState::new();
        let handle = std::thread::spawn(|| {});
        *state.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = DispatcherSession::Running {
            handle,
            on_teardown: None,
            registers_drain_flag: true,
        };
        let owned_generation = state.stop_generation();

        // Nothing advanced the generation, so the reader's stamp still matches
        // and its close retires its own session.
        state.quiesce_if_owned(owned_generation);
        assert!(
            state.dispatcher_is_idle(),
            "the matching-generation close retires its own session"
        );
    }

    /// The reader-close gate does its generation re-check and its teardown
    /// under the SAME dispatcher-lock acquisition, so they are atomic against a
    /// racing stop + start. A non-atomic check-then-act (read the generation,
    /// release, then separately retire) could read a still-matching generation,
    /// have a `stop_streaming` + `start_streaming` install a newer session in
    /// the gap, then retire that newer session it does not own.
    ///
    /// A cross-thread interleaving in that ~2-op gap is too narrow to hit
    /// reliably, so this asserts the structural guarantee directly (the same
    /// approach the lost-wakeup test uses): while the test holds the dispatcher
    /// lock, a concurrent `quiesce_if_owned` cannot even reach its generation
    /// check (it blocks on the lock); and when the test advances the generation
    /// under that held lock and then releases, the gate observes the advanced
    /// generation and no-ops. The re-check and the retire therefore cannot
    /// straddle a teardown: any teardown is either fully before (gate sees the
    /// new generation, no-op) or fully after (gate already ran) the gate's
    /// single locked section.
    #[test]
    fn quiesce_if_owned_rechecks_generation_under_the_dispatcher_lock() {
        use std::sync::atomic::{AtomicBool, Ordering as O};

        let state = Arc::new(StreamingState::new());

        // R's session at generation G, stamped by R.
        let r_handle = std::thread::spawn(|| {});
        *state.dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = DispatcherSession::Running {
            handle: r_handle,
            on_teardown: None,
            registers_drain_flag: true,
        };
        let r_owned_generation = state.stop_generation();

        let gate_returned = Arc::new(AtomicBool::new(false));

        // Hold the dispatcher lock for the whole observation window. A correct
        // `quiesce_if_owned` cannot reach its generation check or its retire
        // while this is held.
        let mut guard = state.dispatcher.lock().unwrap_or_else(|e| e.into_inner());

        let gate = {
            let state = Arc::clone(&state);
            let gate_returned = Arc::clone(&gate_returned);
            std::thread::spawn(move || {
                // Blocks on the dispatcher lock until the test releases it.
                state.quiesce_if_owned(r_owned_generation);
                gate_returned.store(true, O::Release);
            })
        };

        // Give the gate thread time to park on the lock. With the check under
        // the lock it cannot have returned.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            !gate_returned.load(O::Acquire),
            "quiesce_if_owned must block on the dispatcher lock before its \
             generation check; it cannot check-then-act outside the lock"
        );

        // Under the held lock, model a stop + start that advanced past R's
        // stamp and installed a NEWER session. Advancing the generation here is
        // exactly what a `stop_streaming` does; doing it under the lock the gate
        // is waiting on is what makes the gate's later re-check see the new
        // value atomically.
        state
            .stop_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let newer = std::thread::spawn(|| {});
        *guard = DispatcherSession::Running {
            handle: newer,
            on_teardown: None,
            registers_drain_flag: true,
        };
        assert_ne!(
            state
                .stop_generation
                .load(std::sync::atomic::Ordering::Acquire),
            r_owned_generation,
            "the modeled stop advanced the generation past R's stamp"
        );

        // Release the lock: the gate now acquires it, re-checks the generation
        // (advanced -> mismatch), and must NOT retire the newer session.
        drop(guard);
        gate.join().expect("gate thread");
        assert!(
            !state.dispatcher_is_idle(),
            "after R's stale gate ran, the newer session must still be Running"
        );

        state.quiesce(); // tidy the surviving session's thread
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
            ReplayPacing::unpaced(),
            |_kind, contract| {
                if &*contract.symbol == "MSFT" {
                    Err(Error::Stream {
                        kind: crate::error::StreamErrorKind::Disconnected,
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
            ReplayPacing::unpaced(),
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
            ReplayPacing::unpaced(),
            |_, _| Ok(()),
            |_, _| {
                Some(Err(Error::Stream {
                    kind: crate::error::StreamErrorKind::TooManyRequests,
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
    /// failed list when subscriptions cannot be restored. The variant payload is asserted by pattern-
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
            ReplayPacing::unpaced(),
            |_, _| {
                Err(Error::Stream {
                    kind: crate::error::StreamErrorKind::Disconnected,
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

    /// Resurrection-race regression: an in-flight `start_streaming*` that
    /// snapshotted `stop_generation = N` at entry must NOT install `Live` when an
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
    /// The slot is a `Vec` so stacked stop/start/stop cycles cannot lose
    /// an earlier still-draining generation when a later one retires.
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

    /// Multi-generation `await_drain` must wait for ALL retired sessions,
    /// not just the most-recent. A single-slot tracker would return `true`
    /// as soon as the last-pushed flag flipped, even with earlier flags
    /// still pending.
    #[test]
    fn await_drain_waits_for_all_retired_generations() {
        // We exercise the predicate logic directly through a `Vec` to
        // avoid spinning up StreamingClient instances (which require a
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
    // Spinning up a real `Client` requires a network round-trip
    // to authenticate, which a unit test can't do. The dispatch shape
    // is therefore validated against a stand-in helper that walks the
    // same `match` arms `subscribe(Subscription)` runs internally.
    // Live-network routing is covered by the streaming integration
    // tests under `thetadatadx-rs/tests/`.

    /// Mirror of [`Client::subscribe`]'s `match` shape. Returns
    /// the routed (kind, contract-or-sec-type) tuple so the test can
    /// assert the dispatch reached the right arm without a real FPSS
    /// connection.
    enum DispatchProbe {
        ContractQuote(Contract),
        ContractTrade(Contract),
        ContractOpenInterest(Contract),
        ContractMarketValue(Contract),
        FullTrades(SecType),
        FullOpenInterest(SecType),
    }

    fn dispatch_probe(sub: Subscription) -> DispatchProbe {
        match sub {
            Subscription::Contract { contract, kind } => match kind {
                SubscriptionKind::Quote => DispatchProbe::ContractQuote(contract),
                SubscriptionKind::Trade => DispatchProbe::ContractTrade(contract),
                SubscriptionKind::OpenInterest => DispatchProbe::ContractOpenInterest(contract),
                SubscriptionKind::MarketValue => DispatchProbe::ContractMarketValue(contract),
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
        let opt = Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20260620",
                strike: "550",
                right: "C",
            },
        )
        .unwrap();
        assert!(matches!(
            dispatch_probe(aapl.quote()),
            DispatchProbe::ContractQuote(c) if &*c.symbol == "AAPL"
        ));
        assert!(matches!(
            dispatch_probe(opt.trade()),
            DispatchProbe::ContractTrade(c) if &*c.symbol == "SPY"
        ));
        assert!(matches!(
            dispatch_probe(opt.open_interest()),
            DispatchProbe::ContractOpenInterest(c) if c.is_call == Some(true)
        ));
        assert!(matches!(
            dispatch_probe(aapl.market_value()),
            DispatchProbe::ContractMarketValue(c) if &*c.symbol == "AAPL"
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
        // `Client` is the canonical public client type; this
        // guards that the name continues to resolve to the same type so
        // existing call sites keep compiling.
        fn _alias_check(c: Client) -> Client {
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
    #[cfg(feature = "__internal")]
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

    /// The shared fault-record helper the callback dispatcher and the columnar
    /// pull dispatcher both call on a `PollOutcome::Failed` drain: run from the
    /// dispatcher thread itself, it flips its OWN `Running` session to `Failed`
    /// so `is_streaming` / `connection_status` report the dead loop immediately.
    /// This is what makes the columnar path's `is_streaming` truthful (finding
    /// that `sink.fail` alone left the unified session `Running`).
    #[test]
    fn record_dispatcher_failed_if_current_flips_own_running_session() {
        use std::sync::atomic::{AtomicBool, Ordering as O};

        let dispatcher = Arc::new(std::sync::Mutex::new(DispatcherSession::Idle));
        // The dispatcher thread waits until the parent installs its `Running`
        // session (mirroring the spawn+install ordering), then records `Failed`
        // from its own thread so the stored handle's id matches.
        let installed = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let handle = {
            let dispatcher = Arc::clone(&dispatcher);
            let installed = Arc::clone(&installed);
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                while !installed.load(O::Acquire) {
                    std::hint::spin_loop();
                }
                record_dispatcher_failed_if_current(
                    &dispatcher,
                    "fpss io thread terminated abnormally".to_string(),
                );
                done.store(true, O::Release);
            })
        };
        *dispatcher.lock().unwrap_or_else(|e| e.into_inner()) = DispatcherSession::Running {
            handle,
            on_teardown: None,
            registers_drain_flag: true,
        };
        installed.store(true, O::Release);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done.load(O::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "the dispatcher thread did not record Failed within 5 s",
            );
            std::hint::spin_loop();
        }
        assert!(
            matches!(
                &*dispatcher.lock().unwrap_or_else(|e| e.into_inner()),
                DispatcherSession::Failed { reason } if reason.contains("terminated abnormally")
            ),
            "the dispatcher's own fault record must flip its Running session to Failed",
        );
    }
}
