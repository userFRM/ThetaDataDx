//! FPSS real-time streaming client.
//!
//! Synchronous blocking I/O on `std::thread` (no tokio). A TLS reader
//! publishes events to a single LMAX Disruptor ring; the consumer
//! thread invokes the user callback inside `std::panic::catch_unwind`
//! so panics are counted on [`FpssClient::panic_count`] rather than
//! tearing down the pipeline. See `docs-site/docs/streaming/index.md`
//! for the architectural overview.
//!
//! # Examples
//!
//! ```rust,no_run
//! # use thetadatadx::fpss::{FpssClient, FpssConnectArgs, FpssEvent};
//! # use thetadatadx::auth::Credentials;
//! # fn example() -> Result<(), thetadatadx::error::Error> {
//! let creds = Credentials::new("user@example.com", "pw");
//! let hosts = thetadatadx::config::DirectConfig::production().fpss.hosts;
//! let args = FpssConnectArgs::new(&creds, &hosts);
//! let client = FpssClient::connect(args, |_event: &FpssEvent| {})?;
//! use thetadatadx::fpss::protocol::Contract;
//! client.subscribe(Contract::stock("AAPL").quote())?;
//! client.shutdown();
//! # Ok(())
//! # }
//! ```

mod accumulator;
pub(crate) mod connection;
mod decode;
mod delta;
mod events;
pub(crate) mod framing;
mod io_loop;
pub(crate) mod pinning;
pub mod protocol;
pub(crate) mod ring;
mod session;
pub mod wake;

#[cfg(test)]
mod streaming_soak_tests;

// Surface a thin slice of the framing codec for offline benchmarks
// (`benches/bench_framing.rs`). The full `framing` module remains
// crate-private; only the round-trip primitives are exposed.
pub use self::decode::UNRESOLVED_CONTRACT_SYMBOL_PREFIX;
use self::events::IoCommand;
pub use self::events::{BackpressurePolicy, FpssControl, FpssData, FpssEvent};
// `Delivery` stays crate-private; it is the union type the io_loop
// dispatches on. Public callers reach the two modes via
// `FpssClient::connect` (push-callback) and `FpssClient::connect_iter`
// (pull-iter), not by constructing a `Delivery` directly. Referenced
// by name from `connect_iter` below.
pub use self::framing::{read_frame, write_frame, Frame};
use self::io_loop::{io_loop, ping_loop, wait_for_login, LoginResult};
pub use self::session::{reconnect_delay, reconnect_delay_for};

/// Hidden test-internals surface for vendor-failure-mode resilience tests
/// in `crates/thetadatadx/tests/`.
///
/// Re-exports the otherwise crate-private `decode_frame` dispatcher and
/// `DeltaState` so integration tests can drive the full
/// `read_frame_into → decode_frame → FpssEvent` pipeline against
/// synthetic fixture bytes (capture+replay, mid-frame disconnect,
/// reconnect storm, schema drift, frame-decoder fuzz).
///
/// Not part of the supported public API. Subject to change without a
/// SemVer bump. Feature-gated on `__test-helpers` so the module only
/// enters the rlib when the private test feature is enabled — matches
/// the convention used by `crate::wire::test_requests` in `lib.rs`.
/// `cargo-semver-checks` runs with default features and never sees it.
#[cfg(any(test, feature = "__test-helpers"))]
#[doc(hidden)]
pub mod __test_internals {
    pub use super::decode::decode_frame;
    pub use super::delta::DeltaState;
    pub use super::events::FpssEventInternal;
    pub use super::framing::{read_frame_into, FrameReadState, MAX_PAYLOAD_LEN};
}

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Duration;

use crate::auth::Credentials;
use crate::config::{FpssFlushMode, ReconnectPolicy};
use crate::error::Error;
use tdbe::types::enums::{RemoveReason, StreamMsgType};

use self::protocol::{
    build_credentials_payload, build_subscribe_payload, Contract, SubscriptionKind,
};

/// Clamp a 64-bit counter value into a positive 31-bit wire `req_id`.
///
/// The FPSS wire protocol carries `req_id` as a 32-bit signed integer
/// and reserves the value `-1` as the "uncorrelated" sentinel emitted
/// when the server cannot resolve a `ReqResponse` back to a caller-
/// allocated id. Allocators therefore must never hand out `-1` (and,
/// defensively, must stay strictly non-negative so a future server-side
/// `id < 0` check cannot reject a legitimate frame).
///
/// `next_req_id` is widened to `AtomicI64` so a long-running session
/// cannot wrap into the sentinel after `2^31` allocations (≈ 5 days at
/// 5k subs/sec, well inside the realistic uptime envelope of a
/// production streaming consumer). This helper masks off the sign bit
/// and casts down, producing the positive `i32` the wire encoder
/// expects.
///
/// Same-value id collisions remain possible after `2^31` allocations —
/// this is a wire-protocol limitation (31-bit positive id space, since
/// `-1` is reserved as the uncorrelated sentinel and negative ids are
/// defensively excluded). The widening only eliminates the `-1`
/// sentinel collision; an honest cycle of the positive id space still
/// reuses earlier ids. Consumers correlating responses across a span
/// longer than `2^31` allocations must add their own disambiguation
/// (e.g. per-subscription state on the caller side, or a session-id
/// salt prepended to the caller-visible request handle).
#[inline]
pub(in crate::fpss) fn wire_req_id(counter_value: i64) -> i32 {
    (counter_value & 0x7FFF_FFFF) as i32
}

// ---------------------------------------------------------------------------
// FpssConnectArgs — typed parameter bundle for `FpssClient::connect`
// ---------------------------------------------------------------------------

/// Parameters for [`FpssClient::connect`].
///
/// Bundles the connection-side knobs (credentials, hosts, ring size, flush mode,
/// reconnect policy, OHLCVC derivation) into one struct so the call site reads
/// linearly rather than as a positional list of seven heterogeneous arguments.
///
/// # Example
///
/// ```rust,no_run
/// # use thetadatadx::fpss::{FpssClient, FpssConnectArgs, FpssEvent};
/// # use thetadatadx::auth::Credentials;
/// # fn example() -> Result<(), thetadatadx::error::Error> {
/// let creds = Credentials::new("user@example.com", "pw");
/// let hosts = thetadatadx::config::DirectConfig::production().fpss.hosts;
/// let args = FpssConnectArgs::new(&creds, &hosts);
/// let client = FpssClient::connect(args, |_event: &FpssEvent| {})?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct FpssConnectArgs<'a> {
    /// Authenticated user credentials.
    pub creds: &'a Credentials,
    /// FPSS server list. Servers are tried in order until one connects;
    /// the surviving list is retained for auto-reconnect.
    pub hosts: &'a [(String, u16)],
    /// Disruptor ring buffer size (events). Must be a power of two.
    ///
    /// Each ring slot stores one `FpssEventInternal` (96 bytes on the
    /// current 64-bit layout, validated by the `assert_layout_compat`
    /// test). The default
    /// `ring_size = 4096` allocates roughly `4096 × 96 ≈ 384 KiB` per
    /// `FpssClient` for the ring, plus per-event refcounted
    /// `Arc<Contract>` storage on top. Tune downward (e.g., 1024) if
    /// memory is tight; tune upward (e.g., 16_384) if you observe
    /// sustained `dropped_event_count()` under bursty load.
    pub ring_size: usize,
    /// I/O thread flush behavior. See [`FpssFlushMode`].
    pub flush_mode: FpssFlushMode,
    /// Auto-reconnect policy after involuntary disconnect.
    pub policy: ReconnectPolicy,
    /// Delay (ms) before reconnecting after a generic transient drop
    /// (TimedOut, ServerRestarting, Unspecified, …). Mirrors
    /// [`crate::config::ReconnectConfig::wait_ms`]. Default
    /// [`crate::fpss::protocol::RECONNECT_DELAY_MS`] (2_000).
    pub wait_ms: u64,
    /// Delay (ms) before reconnecting after a `TooManyRequests` drop.
    /// Mirrors [`crate::config::ReconnectConfig::wait_rate_limited_ms`].
    /// Default [`crate::fpss::protocol::TOO_MANY_REQUESTS_DELAY_MS`] (130_000).
    pub wait_rate_limited_ms: u64,
    /// When `false`, suppresses locally derived `FpssData::Ohlcvc` events.
    /// Server-sent OHLCVC frames (wire code 24) still pass through.
    pub derive_ohlcvc: bool,
    /// Per-server TCP connect timeout in milliseconds.
    ///
    /// Plumbed through to [`std::net::TcpStream::connect_timeout`] so a
    /// slow / unreachable host fails fast and the next host gets a try.
    pub connect_timeout_ms: u64,
    /// FPSS read timeout in milliseconds.
    ///
    /// Drives the framing layer's mid-frame stall budget, the initial
    /// per-socket read deadline, and the I/O loop's overall
    /// "no-data-received" deadline that emits
    /// [`tdbe::types::enums::RemoveReason::TimedOut`].
    pub read_timeout_ms: u64,
    /// FPSS heartbeat ping interval in milliseconds. Drives the
    /// background `fpss-ping` thread cadence.
    pub ping_interval_ms: u64,
}

impl<'a> FpssConnectArgs<'a> {
    /// Construct with the two required arguments and SDK defaults for the rest.
    ///
    /// `Default` is intentionally NOT implemented on this type: `creds`
    /// and `hosts` are required references with no sensible global
    /// default, so a `Default::default()` would manufacture an
    /// unusable value. Callers populate the optional fields with
    /// builder-style mutation after [`Self::new`].
    #[must_use]
    pub fn new(creds: &'a Credentials, hosts: &'a [(String, u16)]) -> Self {
        Self {
            creds,
            hosts,
            ring_size: 4096,
            flush_mode: FpssFlushMode::default(),
            policy: ReconnectPolicy::default(),
            wait_ms: protocol::RECONNECT_DELAY_MS,
            wait_rate_limited_ms: protocol::TOO_MANY_REQUESTS_DELAY_MS,
            derive_ohlcvc: true,
            connect_timeout_ms: protocol::CONNECT_TIMEOUT_MS,
            read_timeout_ms: protocol::READ_TIMEOUT_MS,
            ping_interval_ms: protocol::PING_INTERVAL_MS,
        }
    }
}

/// Selector for the test-only [`FpssClient::for_self_join_test`]
/// constructor's pre-burst path. Lets soak tests pick between
/// blocking `publish` (matches handshake-time control-frame emission)
/// and non-blocking `try_publish` (matches the live data path that
/// drives the public `dropped_count`).
#[cfg(test)]
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub enum HarnessPublishMode {
    /// Pre-publish via `Producer::publish` on the spawning thread —
    /// never overflows, suitable for the self-join repro.
    BlockingPublish,
    /// Burst on the I/O thread via `Producer::try_publish`,
    /// incrementing the shared `dropped` counter on every rejection
    /// the same way `io_loop` does on the live reader path.
    TryPublishBurst,
}

// ---------------------------------------------------------------------------
// FpssClient
// ---------------------------------------------------------------------------

/// Real-time streaming client for `ThetaData`'s FPSS servers.
///
/// # Lifecycle
///
/// 1. `FpssClient::connect()` -- TLS connect + authenticate + start background tasks
/// 2. `subscribe(...)` / `unsubscribe(...)` -- subscribe to market data
/// 3. Events delivered via the user's `FnMut(&FpssEvent)` callback on the Disruptor thread
/// 4. `shutdown()` -- clean disconnect
///
/// # Thread safety
///
/// `FpssClient` is `Send + Sync`. The polymorphic `subscribe(spec)` /
/// `unsubscribe(spec)` methods send commands through a lock-free channel to
/// the I/O thread; they never touch the TLS stream directly.
pub struct FpssClient {
    /// Channel to send write commands to the I/O thread.
    ///
    /// `std::sync::mpsc::Sender` is `Send` but explicitly not `Sync` -- concurrent
    /// `&self.send()` calls are UB. The `Mutex` makes `FpssClient: Sync` sound
    /// under stdlib's own contract.
    cmd_tx: Mutex<std_mpsc::Sender<IoCommand>>,
    /// Handle to the I/O thread (blocking TLS read + write drain).
    io_handle: Option<JoinHandle<()>>,
    /// Handle to the ping heartbeat thread.
    ping_handle: Option<JoinHandle<()>>,
    /// Shutdown flag shared with background threads.
    shutdown: Arc<AtomicBool>,
    /// Whether we are authenticated and the connection is live.
    authenticated: Arc<AtomicBool>,
    /// Monotonically increasing request ID counter, shared with the
    /// fpss-io reconnect path so re-subscribe frames carry a fresh
    /// `req_id` correlatable to the original subscribe — server-side
    /// `ReqResponse` events with `req_id = -1` are indistinguishable
    /// from manual subscribes, which breaks user-side correlation.
    ///
    /// Widened to `AtomicI64` so a long-running session at thousands of
    /// subscribes/sec cannot wrap into the wire's `-1` sentinel after
    /// `2^31` allocations (≈ 5 days at 5k subs/sec). The 31-bit clamp
    /// to a positive `i32` happens at the wire boundary in
    /// `build_subscribe_payload` / `build_full_type_subscribe_payload`
    /// callers via `(x & 0x7FFF_FFFF) as i32`.
    next_req_id: Arc<AtomicI64>,
    /// Active per-contract subscriptions for reconnection.
    pub(in crate::fpss) active_subs: Arc<Mutex<Vec<(SubscriptionKind, Contract)>>>,
    /// Active full-type (full-stream) subscriptions for reconnection.
    pub(in crate::fpss) active_full_subs:
        Arc<Mutex<Vec<(SubscriptionKind, tdbe::types::enums::SecType)>>>,
    /// The server address we connected to.
    server_addr: String,
    /// Cumulative count of `Producer::try_publish` failures: events the
    /// TLS reader could not enqueue because the Disruptor consumer fell
    /// behind and the ring buffer was full. Snapshot via
    /// [`FpssClient::dropped_count`]; this is the user-facing
    /// "ring-overflow" metric.
    dropped: Arc<AtomicU64>,
    /// Cumulative count of user-callback panics caught by the
    /// Disruptor consumer's `catch_unwind` boundary. Snapshot via
    /// [`FpssClient::panic_count`].
    panics: Arc<AtomicU64>,
    /// `ThreadId` of the Disruptor consumer thread, captured on first
    /// invocation of the consumer closure. Read by [`Drop`] to detect
    /// the **self-join** case: when the user callback (running on the
    /// consumer thread) drops the last `Arc<FpssClient>`, we cannot
    /// `JoinHandle::join` the I/O thread inline because that join
    /// transitively joins the consumer thread itself — the very thread
    /// running `Drop`. In that case [`Drop`] detaches the join onto a
    /// helper thread; cleanup still completes, callers just observe
    /// completion via [`FpssClient::drained_flag`] (or the high-level
    /// [`crate::ThetaDataDxClient::await_drain`] barrier) rather than
    /// blocking on `Drop`.
    consumer_thread_id: Arc<OnceLock<ThreadId>>,
    /// Quiescence barrier: flipped to `true` once the I/O thread and
    /// the Disruptor consumer have both joined and the user callback is
    /// guaranteed to have stopped firing. Set inside [`Drop`] for both
    /// the inline-join path and the detached-helper path. Outer holders
    /// (e.g. [`crate::ThetaDataDxClient::stop_streaming`]) may capture an
    /// [`Arc::clone`] of this flag before releasing their last
    /// `Arc<FpssClient>` so that
    /// [`crate::ThetaDataDxClient::await_drain`] can poll for full
    /// quiescence after stop / reconnect.
    drained: Arc<AtomicBool>,
    /// Slow-callback observability surface (Resilience).
    ///
    /// `slow_callback_threshold_ns` is read by the Disruptor consumer
    /// closure on every dispatch — `0` means the watchdog is disabled.
    /// `slow_callback_count` is incremented every time a user
    /// callback's measured wall-clock duration exceeds the threshold.
    /// Each over-budget event is also surfaced via `tracing::warn!`
    /// (rate-limited per 1024 events to avoid log amplification, the
    /// same cadence the broadcast drop counter uses in
    /// `tools/server/src/ws/broadcast.rs`).
    ///
    /// This is **observability only** — Rust cannot safely cancel
    /// arbitrary user code mid-callback, so we do NOT kill or unwind
    /// the consumer. Operators read the counter and decide how to
    /// respond.
    slow_callback_threshold_ns: Arc<AtomicU64>,
    slow_callback_count: Arc<AtomicU64>,
}

impl FpssClient {
    /// Connect to a `ThetaData` FPSS server, authenticate, and start processing
    /// events via the provided callback.
    ///
    /// The callback runs on the Disruptor's consumer thread -- keep it fast.
    /// For heavy processing, push events to your own queue from the callback.
    ///
    /// # Sequence
    ///
    /// 1. Try each server in `hosts` until one connects (blocking TLS over TCP)
    /// 2. Send CREDENTIALS (code 0) with email + password
    /// 3. Wait for METADATA (code 3) = login success, or DISCONNECTED (code 12) = failure
    /// 4. Start ping heartbeat (100ms interval, `std::thread` with sleep loop)
    /// 5. Start I/O thread (blocking TLS read -> Disruptor ring -> callback)
    ///
    /// Connect to FPSS streaming servers.
    ///
    /// `hosts` is the FPSS server list from [`crate::config::FpssConfig::hosts`].
    /// Servers are tried in order until one connects.
    ///
    /// `policy` controls auto-reconnect behavior after involuntary disconnect.
    ///
    /// When `args.derive_ohlcvc` is `false`, the client will NOT emit derived
    /// `FpssData::Ohlcvc` events after each trade. You still receive
    /// server-sent OHLCVC frames (wire code 24). This reduces throughput
    /// overhead by eliminating one extra event per trade.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when `ring_size` is below
    /// [`ring::MIN_RING_SIZE`] or is not a power of two — the
    /// Disruptor index-wrap requires `i & (cap - 1)`, and silent
    /// rounding would rewrite the caller's stated buffer budget.
    /// Returns [`Error`] on TLS handshake or FPSS authentication
    /// failure.
    pub fn connect<F>(args: FpssConnectArgs<'_>, handler: F) -> Result<Self, Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        Self::connect_with_delivery(args, events::Delivery::Callback(Box::new(handler)))
    }

    /// Connect with pull-iter (queue-drained) delivery.
    ///
    /// Sibling of [`Self::connect`] for the high-throughput batch path.
    /// Returns the connected client paired with an [`EventIterator`]
    /// that drains the per-client bounded queue on the user thread.
    /// Both modes share the same Disruptor ring, reader, and producer
    /// — only the consumer-side dispatch differs.
    ///
    /// The queue is sized to match the Disruptor ring (default `4096`)
    /// so backpressure semantics match the callback path: when the
    /// iterator falls behind and the queue saturates, the consumer
    /// thread drops the new event and increments
    /// [`Self::dropped_count`]. This is the same operator-facing signal
    /// callbacks surface today; pull-iter does not introduce a second
    /// drop counter to interpret.
    ///
    /// `iter()` on the returned [`EventIterator`] holds the iterator's
    /// caller thread (typically a Python `for event in iter:` loop)
    /// across a batch drain — under load, the loop pops as many events
    /// as it can without re-acquiring the GIL, which is the throughput
    /// win pull-iter delivery exists for.
    ///
    /// # Errors
    ///
    /// Returns the same set of errors as [`Self::connect`] (TLS, login,
    /// config validation).
    pub fn connect_iter(args: FpssConnectArgs<'_>) -> Result<(Self, EventIterator), Error> {
        let queue_capacity = args.ring_size;
        let queue = Arc::new(crossbeam_queue::ArrayQueue::<FpssEvent>::new(
            queue_capacity,
        ));
        let finished = Arc::new(AtomicBool::new(false));
        // Sync pull-iter retains legacy `DropNewest` behaviour (silent
        // overflow drops). The async surfaces route through
        // [`Self::connect_iter_with_wake_keep_handle_policy`] when
        // they want explicit policy control.
        let policy = events::BackpressurePolicy::DropNewest;
        // Dedicated terminal predicate for the iterator. Flipped to
        // `true` ONLY after the Disruptor consumer thread has exited
        // its consume loop and dropped the closure that owns the queue
        // push (see `Delivery::Queue::iter_closed` and the drop guard
        // in `io_loop`). Keying terminal-EOF off this — not the global
        // `client.shutdown` flag — is what guarantees `stop_streaming`
        // followed by a tail of not-yet-pushed events cannot false-EOF
        // the iterator. The race the old `client_shutdown` predicate
        // lost: `stop_streaming()` flips `shutdown` BEFORE the consumer
        // thread has finished pushing the last few events into the
        // queue, so any iterator caller polling `next_timeout` between
        // those two moments saw `Closed` and dropped tail events on
        // the floor.
        let iter_closed = Arc::new(AtomicBool::new(false));
        let delivery = events::Delivery::Queue {
            queue: Arc::clone(&queue),
            iter_closed: Arc::clone(&iter_closed),
            wake_fd: None,
            policy,
        };
        let client = Self::connect_with_delivery(args, delivery)?;
        let iterator = EventIterator {
            queue,
            finished,
            iter_closed,
        };
        Ok((client, iterator))
    }

    /// Connect with pull-iter delivery AND an asyncio-style FD wake-up
    /// channel.
    ///
    /// Sibling of [`Self::connect_iter`] for asyncio / select-loop
    /// consumers that cannot afford the 100 µs polling tick of the
    /// blocking iterator. The caller allocates a self-pipe (typically
    /// `pipe2(O_CLOEXEC | O_NONBLOCK)`) and passes the write-end FD to
    /// this method. The Disruptor consumer thread writes a single
    /// coalesced byte to the FD on every successful `queue.push` so the
    /// reader's `epoll` / `kqueue` / `select` wake fires immediately —
    /// no polling, no busy-wait.
    ///
    /// `wake_fd` ownership transfers to the returned [`wake::WakeFd`]
    /// (stored inside the pull-iter `Delivery::Queue` variant);
    /// the FD is closed on `Drop` when the [`FpssClient`] and the
    /// matching [`EventIterator`] are both released. Callers MUST NOT
    /// `close(2)` the FD themselves.
    ///
    /// Use [`Self::connect_iter`] (no wake FD) for synchronous
    /// consumers — the wake-fd path is pure overhead when the caller
    /// drains via `next_timeout` polling.
    ///
    /// # Errors
    ///
    /// Returns the same set of errors as [`Self::connect`] (TLS, login,
    /// config validation).
    pub fn connect_iter_with_wake(
        args: FpssConnectArgs<'_>,
        wake: wake::WakeFd,
    ) -> Result<(Self, EventIterator), Error> {
        let queue_capacity = args.ring_size;
        let queue = Arc::new(crossbeam_queue::ArrayQueue::<FpssEvent>::new(
            queue_capacity,
        ));
        let finished = Arc::new(AtomicBool::new(false));
        let iter_closed = Arc::new(AtomicBool::new(false));
        let wake_arc = Arc::new(wake);
        // Async pull-iter without explicit policy defaults to `Block`
        // — the safe default for new callers. Existing consumers that
        // want legacy `DropNewest` semantics call the `_policy`
        // variant below.
        let policy = events::BackpressurePolicy::Block;
        let delivery = events::Delivery::Queue {
            queue: Arc::clone(&queue),
            iter_closed: Arc::clone(&iter_closed),
            wake_fd: Some(Arc::clone(&wake_arc)),
            policy,
        };
        let client = Self::connect_with_delivery(args, delivery)?;
        let iterator = EventIterator {
            queue,
            finished,
            iter_closed,
        };
        // `wake_arc` is intentionally not surfaced to the caller — the
        // [`Delivery::Queue`] variant captured a clone, and the iterator
        // / client lifetime governs the FD close. The Python SDK keeps
        // a third `Arc<WakeFd>` clone for the `rearm()` path that runs
        // on the asyncio reader thread; that clone is acquired via the
        // explicit `connect_iter_with_wake_keep_handle` constructor in
        // `streaming_async_session.rs`, which forwards through this
        // method.
        let _ = wake_arc;
        Ok((client, iterator))
    }

    /// Variant of [`Self::connect_iter_with_wake`] that hands back the
    /// `Arc<WakeFd>` so the caller can drive [`wake::WakeFd::rearm`]
    /// from the reader thread. The async wake protocol requires:
    ///
    /// 1. Reader observes pipe-read-ready on the asyncio loop.
    /// 2. Reader calls `wake.rearm()` BEFORE draining the pipe.
    /// 3. Reader drains the pipe with one or more non-blocking `read(2)`.
    /// 4. Reader drains the event queue via `iterator.try_next` until
    ///    `NextEvent::Timeout` or `NextEvent::Closed`.
    /// 5. Reader awaits the next FD-ready signal.
    ///
    /// Step 2 is what makes the wake re-arm — without it, a producer
    /// push observed between the rearm-elsewhere and the drain would
    /// fail to re-fire the wake. Returning the shared `Arc<WakeFd>` is
    /// the only safe way to expose `rearm()` to the reader.
    ///
    /// Internally builds the same pull-iter `Delivery::Queue` variant
    /// as [`Self::connect_iter_with_wake`]; the only difference is the
    /// third return value carrying the shared handle.
    ///
    /// # Errors
    ///
    /// Returns the same set of errors as [`Self::connect`] (TLS, login,
    /// config validation).
    pub fn connect_iter_with_wake_keep_handle(
        args: FpssConnectArgs<'_>,
        wake: wake::WakeFd,
    ) -> Result<(Self, EventIterator, Arc<wake::WakeFd>), Error> {
        Self::connect_iter_with_wake_keep_handle_policy(
            args,
            wake,
            None,
            events::BackpressurePolicy::Block,
        )
    }

    /// Variant of [`Self::connect_iter_with_wake_keep_handle`] that
    /// accepts an explicit [`events::BackpressurePolicy`] and an
    /// optional `max_queue_depth` override.
    ///
    /// * `max_queue_depth` — `None` reuses `args.ring_size` (the
    ///   existing implicit sizing); `Some(n)` caps the
    ///   [`crossbeam_queue::ArrayQueue`] at `n` events. Must satisfy
    ///   the same power-of-two + [`ring::MIN_RING_SIZE`] constraints
    ///   the ring does, validated via [`ring::check_ring_size`].
    /// * `policy` — overflow strategy. See [`events::BackpressurePolicy`].
    ///
    /// Production callers reach this through the Python
    /// `client.streaming_async(max_queue_depth=..., backpressure=...)`
    /// kwargs.
    ///
    /// # Errors
    ///
    /// Returns the same set of errors as [`Self::connect`] (TLS,
    /// login, config validation), plus [`Error::Config`] when
    /// `max_queue_depth` is below [`ring::MIN_RING_SIZE`] or is not a
    /// power of two.
    pub fn connect_iter_with_wake_keep_handle_policy(
        args: FpssConnectArgs<'_>,
        wake: wake::WakeFd,
        max_queue_depth: Option<usize>,
        policy: events::BackpressurePolicy,
    ) -> Result<(Self, EventIterator, Arc<wake::WakeFd>), Error> {
        let queue_capacity = match max_queue_depth {
            Some(n) => ring::check_ring_size(n)
                .map_err(|e| Error::config_invalid("fpss.max_queue_depth", e.to_string()))?,
            None => args.ring_size,
        };
        let queue = Arc::new(crossbeam_queue::ArrayQueue::<FpssEvent>::new(
            queue_capacity,
        ));
        let finished = Arc::new(AtomicBool::new(false));
        let iter_closed = Arc::new(AtomicBool::new(false));
        let wake_arc = Arc::new(wake);
        let delivery = events::Delivery::Queue {
            queue: Arc::clone(&queue),
            iter_closed: Arc::clone(&iter_closed),
            wake_fd: Some(Arc::clone(&wake_arc)),
            policy,
        };
        let client = Self::connect_with_delivery(args, delivery)?;
        let iterator = EventIterator {
            queue,
            finished,
            iter_closed,
        };
        Ok((client, iterator, wake_arc))
    }

    fn connect_with_delivery(
        args: FpssConnectArgs<'_>,
        delivery: events::Delivery,
    ) -> Result<Self, Error> {
        let FpssConnectArgs {
            creds,
            hosts,
            ring_size,
            flush_mode,
            policy,
            wait_ms,
            wait_rate_limited_ms,
            derive_ohlcvc,
            connect_timeout_ms,
            read_timeout_ms,
            ping_interval_ms,
        } = args;
        // Validate ring_size at the public construction boundary so
        // the caller's stated buffer budget is never silently rewritten.
        // Rejecting at connect-time is cheaper than discovering a perf
        // cliff under load.
        let ring_size = ring::check_ring_size(ring_size)
            .map_err(|e| Error::config_invalid("fpss.ring_size", e.to_string()))?;
        // Validate the wired tuning knobs at the same boundary. The
        // higher-level `DirectConfig::validate` already rejects out-of-
        // range values at config-load time; this second check defends
        // against callers that bypass `DirectConfig` and construct
        // `FpssConnectArgs` directly with a hand-rolled `FpssConfig`.
        let to_i64 = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        if !crate::config::fpss_bounds::TIMEOUT_MS.contains(&read_timeout_ms) {
            return Err(Error::config_out_of_range(
                "fpss.read_timeout_ms",
                to_i64(read_timeout_ms),
                to_i64(*crate::config::fpss_bounds::TIMEOUT_MS.start()),
                to_i64(*crate::config::fpss_bounds::TIMEOUT_MS.end()),
            ));
        }
        if !crate::config::fpss_bounds::CONNECT_TIMEOUT_MS.contains(&connect_timeout_ms) {
            return Err(Error::config_out_of_range(
                "fpss.connect_timeout_ms",
                to_i64(connect_timeout_ms),
                to_i64(*crate::config::fpss_bounds::CONNECT_TIMEOUT_MS.start()),
                to_i64(*crate::config::fpss_bounds::CONNECT_TIMEOUT_MS.end()),
            ));
        }
        if !crate::config::fpss_bounds::PING_INTERVAL_MS.contains(&ping_interval_ms) {
            return Err(Error::config_out_of_range(
                "fpss.ping_interval_ms",
                to_i64(ping_interval_ms),
                to_i64(*crate::config::fpss_bounds::PING_INTERVAL_MS.start()),
                to_i64(*crate::config::fpss_bounds::PING_INTERVAL_MS.end()),
            ));
        }
        let borrowed: Vec<(&str, u16)> = hosts.iter().map(|(h, p)| (h.as_str(), *p)).collect();
        let connect_timeout = Duration::from_millis(connect_timeout_ms);
        let read_timeout = Duration::from_millis(read_timeout_ms);
        let (stream, server_addr) =
            connection::connect_to_servers(&borrowed, connect_timeout, read_timeout)?;
        Self::connect_with_stream(connection::ConnectWithStreamArgs {
            creds,
            stream,
            server_addr,
            hosts,
            ring_size,
            derive_ohlcvc,
            flush_mode,
            policy,
            wait_ms,
            wait_rate_limited_ms,
            connect_timeout,
            read_timeout,
            ping_interval: Duration::from_millis(ping_interval_ms),
            delivery,
        })
    }

    /// Connect using a pre-established stream (for testing with mock sockets).
    ///
    /// `hosts` is the full FPSS server list, needed for auto-reconnect to try
    /// all servers. Pass an empty slice to disable reconnection to other servers.
    pub(crate) fn connect_with_stream(
        args: connection::ConnectWithStreamArgs<'_>,
    ) -> Result<Self, Error> {
        let connection::ConnectWithStreamArgs {
            creds,
            mut stream,
            server_addr,
            hosts,
            ring_size,
            derive_ohlcvc,
            flush_mode,
            policy,
            wait_ms,
            wait_rate_limited_ms,
            connect_timeout,
            read_timeout,
            ping_interval,
            delivery,
        } = args;
        // Send CREDENTIALS (code 0).
        let cred_payload = build_credentials_payload(&creds.email, &creds.password);
        let frame = Frame::new(StreamMsgType::Credentials, cred_payload);
        write_frame(&mut stream, &frame)?;
        tracing::debug!("sent CREDENTIALS to {server_addr}");

        // Wait for METADATA (success) or DISCONNECTED (failure). Blocks until
        // the login response arrives.
        // `pending_control` collects every typed control frame (`Connected`,
        // `Ping`, `ReconnectedServer`, `Restart`) that arrives BEFORE
        // METADATA, preserving wire order. The io_loop drains the buffer
        // onto the event bus before `LoginSuccess` so user callbacks see
        // the same sequence the post-METADATA `decode_frame` dispatch
        // emits.
        let mut pending_control: Vec<FpssControl> = Vec::new();
        let login_result = wait_for_login(&mut stream, &mut pending_control)?;

        let permissions = match login_result {
            LoginResult::Success(permissions) => {
                tracing::info!(
                    server = %server_addr,
                    permissions = %permissions,
                    "FPSS login successful"
                );
                permissions
            }
            LoginResult::Disconnected(reason) => {
                if matches!(
                    reason,
                    RemoveReason::InvalidCredentials
                        | RemoveReason::InvalidLoginValues
                        | RemoveReason::InvalidCredentialsNullUser
                ) {
                    tracing::warn!(
                        "FPSS login failed. If your password contains special characters, \
                         try URL-encoding them."
                    );
                }
                return Err(Error::Fpss {
                    kind: crate::error::FpssErrorKind::Disconnected,
                    message: format!("server rejected login: {reason:?}"),
                });
            }
        };

        // Set a shorter read timeout for the I/O loop so it can drain commands
        // between reads. The 10s overall timeout is tracked by counting consecutive
        // read-timeout errors in the I/O loop.
        //
        // 50ms is short enough that pings (100ms interval) are serviced promptly,
        // but long enough to avoid excessive CPU spinning during quiet periods.
        let io_read_timeout = Duration::from_millis(50);
        stream
            .sock
            .set_read_timeout(Some(io_read_timeout))
            .map_err(|e| Error::Fpss {
                kind: crate::error::FpssErrorKind::ConnectionRefused,
                message: format!("failed to set read timeout: {e}"),
            })?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let authenticated = Arc::new(AtomicBool::new(true));
        let active_subs: Arc<Mutex<Vec<(protocol::SubscriptionKind, protocol::Contract)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let active_full_subs: Arc<
            Mutex<Vec<(protocol::SubscriptionKind, tdbe::types::enums::SecType)>>,
        > = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let panics = Arc::new(AtomicU64::new(0));
        // Slow-callback observability — opt-in via
        // `set_slow_callback_threshold` after `connect`. `0` disables.
        let slow_callback_threshold_ns = Arc::new(AtomicU64::new(0));
        let slow_callback_count = Arc::new(AtomicU64::new(0));
        // Captured by the Disruptor consumer closure on first dispatch
        // and read by `FpssClient::drop` to break the self-join cycle
        // (callback -> stop_streaming -> drop FpssClient -> join io
        // thread -> drop producer -> join consumer thread = self).
        let consumer_thread_id: Arc<OnceLock<ThreadId>> = Arc::new(OnceLock::new());

        // Shared `next_req_id` counter — the FpssClient public API
        // owns one handle for caller-issued subscribes; the io_loop
        // borrows another so re-subscribe frames on auto-reconnect
        // allocate fresh ids correlatable through `ReqResponse`.
        let next_req_id: Arc<AtomicI64> = Arc::new(AtomicI64::new(1));

        // Command channel: FpssClient -> I/O thread
        let (cmd_tx, cmd_rx) = std_mpsc::channel::<IoCommand>();

        // Ping command channel: ping thread -> I/O thread
        let ping_cmd_tx = cmd_tx.clone();

        // Spawn the I/O thread: blocking TLS read + Disruptor publish + command drain.
        let io_shutdown = Arc::clone(&shutdown);
        let io_authenticated = Arc::clone(&authenticated);
        let io_server_addr = server_addr.clone();
        let io_creds = creds.clone();
        let io_hosts = hosts.to_vec();
        let io_active_subs = Arc::clone(&active_subs);
        let io_active_full_subs = Arc::clone(&active_full_subs);
        let io_dropped = Arc::clone(&dropped);
        let io_panics = Arc::clone(&panics);
        let io_consumer_thread_id = Arc::clone(&consumer_thread_id);
        let io_slow_threshold_ns = Arc::clone(&slow_callback_threshold_ns);
        let io_slow_count = Arc::clone(&slow_callback_count);
        let io_next_req_id = Arc::clone(&next_req_id);

        let io_handle = thread::Builder::new()
            .name("fpss-io".to_owned())
            .spawn(move || {
                io_loop(io_loop::IoLoopArgs {
                    stream,
                    cmd_rx,
                    delivery,
                    ring_size,
                    shutdown: io_shutdown,
                    authenticated: io_authenticated,
                    permissions,
                    pending_control,
                    _server_addr: io_server_addr,
                    derive_ohlcvc,
                    flush_mode,
                    policy,
                    wait_ms,
                    wait_rate_limited_ms,
                    creds: io_creds,
                    hosts: io_hosts,
                    active_subs: io_active_subs,
                    active_full_subs: io_active_full_subs,
                    dropped: io_dropped,
                    panics: io_panics,
                    consumer_thread_id: io_consumer_thread_id,
                    slow_callback_threshold_ns: io_slow_threshold_ns,
                    slow_callback_count: io_slow_count,
                    connect_timeout,
                    read_timeout,
                    next_req_id: io_next_req_id,
                });
            })
            .map_err(|e| Error::Fpss {
                kind: crate::error::FpssErrorKind::ConnectionRefused,
                message: format!("failed to spawn fpss-io thread: {e}"),
            })?;

        // Spawn the ping thread: sends PING command at the configured cadence.
        let ping_shutdown = Arc::clone(&shutdown);
        let ping_authenticated = Arc::clone(&authenticated);

        let ping_handle = thread::Builder::new()
            .name("fpss-ping".to_owned())
            .spawn(move || {
                ping_loop(
                    ping_cmd_tx,
                    ping_shutdown,
                    ping_authenticated,
                    ping_interval,
                );
            })
            .map_err(|e| Error::Fpss {
                kind: crate::error::FpssErrorKind::ConnectionRefused,
                message: format!("failed to spawn fpss-ping thread: {e}"),
            })?;

        Ok(FpssClient {
            cmd_tx: Mutex::new(cmd_tx),
            io_handle: Some(io_handle),
            ping_handle: Some(ping_handle),
            shutdown,
            authenticated,
            next_req_id: Arc::clone(&next_req_id),
            active_subs,
            active_full_subs,
            server_addr,
            dropped,
            panics,
            consumer_thread_id,
            drained: Arc::new(AtomicBool::new(false)),
            slow_callback_threshold_ns,
            slow_callback_count,
        })
    }

    /// Cumulative count of events the TLS reader could not publish into
    /// the Disruptor ring because the consumer fell behind and the ring
    /// was full (`Producer::try_publish` returned [`disruptor::RingBufferFull`]).
    ///
    /// This is the user-facing "events dropped due to slow callback"
    /// metric on the post-SSOT pipeline. Operators should poll on a
    /// periodic timer (e.g. every second) and emit a `warn` log on any
    /// non-zero delta — a per-drop log would amplify under sustained
    /// overflow.
    #[must_use]
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Cumulative count of user-callback panics caught by the
    /// Disruptor consumer's `catch_unwind` boundary. Each panic is
    /// also surfaced via `tracing::error!` with target
    /// `thetadatadx::fpss::io_loop`. The consumer thread NEVER dies
    /// from a user-code panic.
    #[must_use]
    pub fn panic_count(&self) -> u64 {
        self.panics.load(Ordering::Relaxed)
    }

    /// Set the slow-callback wall-clock threshold.
    ///
    /// When the user-callback wall-clock duration exceeds `threshold`,
    /// [`Self::slow_callback_count`] increments and a `tracing::warn!`
    /// fires (rate-limited per 1024 over-budget events to avoid log
    /// amplification under sustained pressure).
    ///
    /// Pass `Duration::ZERO` to disable the watchdog. The default is
    /// disabled — operators opt in once the application's expected
    /// callback budget is known.
    ///
    /// **Observability only.** Rust cannot safely cancel arbitrary
    /// user code mid-callback, so the watchdog never kills the
    /// consumer. The counter and log surface let operators detect
    /// regressions; the application decides how to respond.
    pub fn set_slow_callback_threshold(&self, threshold: Duration) {
        let ns = u64::try_from(threshold.as_nanos()).unwrap_or(u64::MAX);
        self.slow_callback_threshold_ns.store(ns, Ordering::Relaxed);
    }

    /// Cumulative count of user-callback invocations whose wall-clock
    /// duration exceeded the threshold set by
    /// [`Self::set_slow_callback_threshold`]. Returns `0` when the
    /// watchdog is disabled (threshold = 0).
    #[must_use]
    pub fn slow_callback_count(&self) -> u64 {
        self.slow_callback_count.load(Ordering::Relaxed)
    }

    /// Shared quiescence flag for this client. Flipped to `true` after
    /// the I/O thread and the Disruptor consumer have both joined, so
    /// the user callback is guaranteed to have stopped firing.
    ///
    /// Returned as an `Arc<AtomicBool>` so a higher-level holder
    /// (e.g. [`crate::ThetaDataDxClient::stop_streaming`]) can capture a
    /// clone before releasing its last `Arc<FpssClient>` and use it to
    /// implement an asynchronous drain barrier.
    ///
    /// Stays `false` if the detached shutdown helper could not spawn
    /// (extreme OOM / FD exhaustion); a poller observing that state
    /// will time out, which matches the unreachable cleanup it
    /// describes.
    #[must_use]
    pub fn drained_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.drained)
    }

    /// Polymorphic subscribe — wire-level entry point.
    ///
    /// Accepts a typed [`protocol::Subscription`] value built via
    /// [`Contract::quote`] / [`Contract::trade`] /
    /// [`Contract::open_interest`] (per-contract scope) or
    /// [`protocol::SecTypeExt::full_trades`] /
    /// [`protocol::SecTypeExt::full_open_interest`] (full-stream
    /// scope). Dispatches to the per-contract or full-stream
    /// payload builder by enum variant.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe(&self, sub: protocol::Subscription) -> Result<(), Error> {
        match sub {
            protocol::Subscription::Contract { contract, kind } => {
                self.send_per_contract(kind, &contract, /* unsubscribe */ false)
            }
            protocol::Subscription::Full { sec_type, kind } => {
                self.send_full_stream(kind, sec_type, /* unsubscribe */ false)
            }
        }
    }

    /// Polymorphic unsubscribe — wire-level counterpart to
    /// [`Self::subscribe`].
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe(&self, sub: protocol::Subscription) -> Result<(), Error> {
        match sub {
            protocol::Subscription::Contract { contract, kind } => {
                self.send_per_contract(kind, &contract, /* unsubscribe */ true)
            }
            protocol::Subscription::Full { sec_type, kind } => {
                self.send_full_stream(kind, sec_type, /* unsubscribe */ true)
            }
        }
    }

    /// Per-contract subscribe / unsubscribe wire emission.
    fn send_per_contract(
        &self,
        kind: SubscriptionKind,
        contract: &Contract,
        unsubscribe: bool,
    ) -> Result<(), Error> {
        if unsubscribe {
            self.send_unsub_contract(kind, contract)
        } else {
            self.send_sub_contract(kind, contract)
        }
    }

    /// Full-stream subscribe / unsubscribe wire emission.
    fn send_full_stream(
        &self,
        kind: protocol::FullSubscriptionKind,
        sec_type: tdbe::types::enums::SecType,
        unsubscribe: bool,
    ) -> Result<(), Error> {
        self.check_connected()?;
        let req_id = wire_req_id(self.next_req_id.fetch_add(1, Ordering::Relaxed));
        let payload = protocol::build_full_type_subscribe_payload(req_id, sec_type);
        // Wire codes for full-stream subscribe / unsubscribe: code 22
        // (TRADE) / 52 (REMOVE_TRADE) for Trades, code 23
        // (OPEN_INTEREST) / 53 (REMOVE_OPEN_INTEREST) for OI.
        let (code, kind_for_track) = match (kind, unsubscribe) {
            (protocol::FullSubscriptionKind::Trades, false) => {
                (StreamMsgType::Trade, SubscriptionKind::Trade)
            }
            (protocol::FullSubscriptionKind::Trades, true) => {
                (StreamMsgType::RemoveTrade, SubscriptionKind::Trade)
            }
            (protocol::FullSubscriptionKind::OpenInterest, false) => {
                (StreamMsgType::OpenInterest, SubscriptionKind::OpenInterest)
            }
            (protocol::FullSubscriptionKind::OpenInterest, true) => (
                StreamMsgType::RemoveOpenInterest,
                SubscriptionKind::OpenInterest,
            ),
        };
        self.send_cmd(IoCommand::WriteFrame { code, payload })?;
        tracing::debug!(
            req_id,
            sec_type = ?sec_type,
            unsubscribe,
            "sent full-stream subscription frame"
        );
        // Track / untrack for reconnection.
        let mut subs = self
            .active_full_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if unsubscribe {
            subs.retain(|(k, s)| !(*k == kind_for_track && *s == sec_type));
        } else {
            subs.push((kind_for_track, sec_type));
        }
        Ok(())
    }

    /// Per-contract subscribe wire emission.
    fn send_sub_contract(&self, kind: SubscriptionKind, contract: &Contract) -> Result<(), Error> {
        contract.validate()?;
        self.check_connected()?;

        let req_id = wire_req_id(self.next_req_id.fetch_add(1, Ordering::Relaxed));
        let payload = build_subscribe_payload(req_id, contract)?;
        let code = kind.subscribe_code();

        self.send_cmd(IoCommand::WriteFrame { code, payload })?;

        // Track for reconnection
        {
            let mut subs = self
                .active_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            subs.push((kind, contract.clone()));
        }

        tracing::debug!(
            req_id,
            kind = ?kind,
            contract = %contract,
            "sent subscription"
        );
        Ok(())
    }

    /// Per-contract unsubscribe wire emission.
    fn send_unsub_contract(
        &self,
        kind: SubscriptionKind,
        contract: &Contract,
    ) -> Result<(), Error> {
        contract.validate()?;
        self.check_connected()?;

        let req_id = wire_req_id(self.next_req_id.fetch_add(1, Ordering::Relaxed));
        let payload = build_subscribe_payload(req_id, contract)?;
        let code = kind.unsubscribe_code();

        self.send_cmd(IoCommand::WriteFrame { code, payload })?;

        // Remove from tracked subscriptions
        {
            let mut subs = self
                .active_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            subs.retain(|(k, c)| !(k == &kind && c == contract));
        }

        tracing::debug!(
            req_id,
            kind = ?kind,
            contract = %contract,
            "sent unsubscribe"
        );
        Ok(())
    }

    /// Send the STOP message and shut down background threads.
    ///
    /// Sends STOP (code 32), then closes the socket.
    pub fn shutdown(&self) {
        if self.shutdown.swap(true, Ordering::AcqRel) {
            return; // already shut down
        }

        tracing::info!(server = %self.server_addr, "shutting down FPSS client");

        // Send shutdown command to I/O thread (which will send STOP to server).
        let _ = self.send_cmd(IoCommand::Shutdown);

        // Clear active subscriptions on explicit shutdown. Involuntary disconnects
        // preserve the lists so `reconnect()` can re-subscribe automatically.
        {
            let mut subs = self
                .active_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            subs.clear();
        }
        {
            let mut subs = self
                .active_full_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            subs.clear();
        }

        self.authenticated.store(false, Ordering::Release);
        tracing::debug!("FPSS shutdown signal sent");
    }

    /// Check if the client is currently authenticated.
    pub fn is_authenticated(&self) -> bool {
        self.authenticated.load(Ordering::Acquire)
    }

    /// Get the server address we are connected to.
    pub fn server_addr(&self) -> &str {
        &self.server_addr
    }

    /// Get a snapshot of currently active per-contract subscriptions.
    pub fn active_subscriptions(&self) -> Vec<(SubscriptionKind, Contract)> {
        self.active_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Get a snapshot of currently active full-type (full-stream) subscriptions.
    pub fn active_full_subscriptions(
        &self,
    ) -> Vec<(SubscriptionKind, tdbe::types::enums::SecType)> {
        self.active_full_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Verify connection is live before sending.
    fn check_connected(&self) -> Result<(), Error> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "client is shut down".to_string(),
            });
        }
        if !self.authenticated.load(Ordering::Acquire) {
            return Err(Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "not authenticated".to_string(),
            });
        }
        Ok(())
    }

    /// Test-only constructor that wires up the same Disruptor +
    /// I/O-thread topology as [`Self::connect_with_stream`] **without**
    /// touching the network. It exists to drive the `Drop` self-join
    /// guard from `tests/streaming_soak.rs` against the real
    /// `FpssClient` instance and the real `consumer_thread_id`
    /// plumbing, not a mock of either.
    ///
    /// Topology:
    /// - The user `handler` runs on the Disruptor consumer thread,
    ///   under `catch_unwind`, exactly like `io_loop`.
    /// - The fake "I/O thread" runs a [`mode`]-dependent burst loop
    ///   (`publish` blocking or `try_publish` non-blocking, mirroring
    ///   `io_loop`'s data path) and idles on the shutdown signal.
    /// - `try_publish` failures increment the same `dropped`
    ///   `Arc<AtomicU64>` the public [`Self::dropped_count`] reads,
    ///   so soak tests can assert on the public surface.
    /// - `Drop` reads `consumer_thread_id` and runs the same
    ///   self-join guard as the production path.
    ///
    /// `n_burst_events` synthetic `FpssEvent::Control(MarketOpen)`
    /// frames are pushed via [`HarnessPublishMode`].
    ///
    /// The optional `start_signal` lets the test defer the io thread's
    /// burst until after it has finished any setup that must happen
    /// before the consumer can dispatch. When `Some`, the io thread
    /// busy-waits on the flag flipping to `true` before publishing the
    /// burst. When `None`, the burst runs as soon as the io thread
    /// scheduler is given a chance to start.
    #[cfg(test)]
    #[doc(hidden)]
    pub fn for_self_join_test<F>(
        n_burst_events: usize,
        ring_size: usize,
        mode: HarnessPublishMode,
        start_signal: Option<Arc<AtomicBool>>,
        handler: F,
    ) -> Arc<Self>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        use disruptor::{build_single_producer, BusySpin, Producer, Sequence};

        use self::events::FpssEventInternal;
        use self::ring::RingEvent;

        let ring_size = ring::check_ring_size(ring_size).expect(
            "for_self_join_test: ring_size must be validated; tests must pass a power of two \
             >= MIN_RING_SIZE (e.g. 64, 128, 256)",
        );

        let shutdown = Arc::new(AtomicBool::new(false));
        let authenticated = Arc::new(AtomicBool::new(true));
        let active_subs: Arc<Mutex<Vec<(SubscriptionKind, Contract)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let active_full_subs: Arc<Mutex<Vec<(SubscriptionKind, tdbe::types::enums::SecType)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let panics = Arc::new(AtomicU64::new(0));
        let consumer_thread_id: Arc<OnceLock<ThreadId>> = Arc::new(OnceLock::new());
        let next_req_id: Arc<AtomicI64> = Arc::new(AtomicI64::new(1));

        let (cmd_tx, _cmd_rx) = std_mpsc::channel::<IoCommand>();

        let handler_cell = Mutex::new(handler);
        let panics_consumer = Arc::clone(&panics);
        let consumer_thread_id_cell = Arc::clone(&consumer_thread_id);

        let factory = RingEvent::default;
        let mut producer = build_single_producer(ring_size, factory, BusySpin)
            .handle_events_with(move |slot: &RingEvent, _seq: Sequence, _eob: bool| {
                consumer_thread_id_cell.get_or_init(|| thread::current().id());
                if let Some(evt) = slot.event.as_public() {
                    let mut h = handler_cell
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h(evt))).is_err() {
                        panics_consumer.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
            .build();

        // Fake I/O thread: in `TryPublishBurst` mode, push the burst
        // via `try_publish` exactly like `io_loop` does on the real
        // TLS reader path, incrementing the shared `dropped` counter
        // on every overflow rejection. In `BlockingPublish` mode,
        // push the same burst via `publish` (no overflow, fixed
        // count) so the self-join repro has a steady stream of
        // events for the user callback to fire on. Both modes run
        // the burst on the io thread so the calling test thread has
        // a stable handoff point: by the time `for_self_join_test`
        // returns, NO events are in the ring yet, and the test can
        // stash its `Arc<FpssClient>` reference into the callback's
        // shared cell before any consumer dispatch races it. After
        // the burst, park until shutdown and drop the producer
        // (producer-drop joins the consumer, the exact transitive
        // dependency that creates the self-join hazard in the
        // production exit path).
        let io_shutdown = Arc::clone(&shutdown);
        let io_dropped = Arc::clone(&dropped);
        let io_burst = n_burst_events;
        let io_handle = thread::Builder::new()
            .name("fpss-io-test".to_owned())
            .spawn(move || {
                if let Some(signal) = start_signal {
                    while !signal.load(Ordering::Acquire) {
                        if io_shutdown.load(Ordering::Acquire) {
                            drop(producer);
                            return;
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                }
                match mode {
                    HarnessPublishMode::BlockingPublish => {
                        for _ in 0..io_burst {
                            producer.publish(|slot| {
                                slot.event = FpssEventInternal::Control(FpssControl::MarketOpen);
                            });
                        }
                    }
                    HarnessPublishMode::TryPublishBurst => {
                        for _ in 0..io_burst {
                            if producer
                                .try_publish(|slot| {
                                    slot.event =
                                        FpssEventInternal::Control(FpssControl::MarketOpen);
                                })
                                .is_err()
                            {
                                io_dropped.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
                while !io_shutdown.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(5));
                }
                drop(producer);
            })
            .expect("failed to spawn fpss-io-test thread");

        Arc::new(FpssClient {
            cmd_tx: Mutex::new(cmd_tx),
            io_handle: Some(io_handle),
            ping_handle: None,
            shutdown,
            authenticated,
            next_req_id: Arc::clone(&next_req_id),
            active_subs,
            active_full_subs,
            server_addr: "test://self-join".to_owned(),
            dropped,
            panics,
            consumer_thread_id,
            drained: Arc::new(AtomicBool::new(false)),
            slow_callback_threshold_ns: Arc::new(AtomicU64::new(0)),
            slow_callback_count: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Send a command to the I/O thread. Maps channel-send failure to a
    /// `Disconnected` FPSS error.
    pub(in crate::fpss) fn send_cmd(&self, cmd: IoCommand) -> Result<(), Error> {
        self.cmd_tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(cmd)
            .map_err(|_| Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "I/O thread has exited".to_string(),
            })
    }
}

// ---------------------------------------------------------------------------
// EventIterator -- pull-mode (queue-drain) delivery handle
// ---------------------------------------------------------------------------

/// Outcome of a single deadline-bounded pop on an [`EventIterator`].
///
/// Returned by [`EventIterator::next_timeout`]. Disambiguates the three
/// distinct states a bounded-wait pop can land in:
///
/// * [`NextEvent::Ready`] — an event was popped within the deadline.
/// * [`NextEvent::Timeout`] — the deadline expired with no event, but the
///   upstream stream is still live; the caller should re-poll.
/// * [`NextEvent::Closed`] — the iterator (`close()`) or the upstream
///   [`FpssClient`] has shut down AND the queue is fully drained; no
///   further events will ever arrive. The caller MUST stop iterating.
///
/// Pre-`9.1.0` `next_timeout` returned `Option<FpssEvent>` and overloaded
/// `None` to mean both "timeout" and "closed", which forced every
/// language binding to guess. The typed variants make the contract
/// explicit at every layer (C ABI return code, C++ wrapper `ended_`,
/// Python `StopIteration`, TypeScript `done: true`).
#[derive(Debug)]
pub enum NextEvent {
    /// An event was popped from the queue within the wait window.
    Ready(FpssEvent),
    /// The wait window expired with no event, and the upstream stream
    /// is still live. The caller should loop and re-poll.
    Timeout,
    /// The iterator has been closed (or the upstream client shut down)
    /// AND the queue is fully drained. The caller MUST stop iterating;
    /// further `next_timeout` calls will keep returning `Closed`.
    Closed,
}

/// Drain handle for a pull-iter (`Delivery::Queue`) streaming session.
///
/// Returned by [`FpssClient::connect_iter`] and ultimately surfaced
/// through `start_streaming_iter()` on the language SDKs (Python
/// `with client.streaming_iter() as it: for event in it:` or
/// `client.start_streaming_iter()`, TypeScript
/// `client.startStreamingIter()` (async iterable), C
/// `tdx_*_event_iter_next`, C++
/// `client.start_streaming_iter().next(timeout)`).
///
/// `next()` pops the head of the bounded queue or, on empty, sleeps a
/// configurable short interval and retries — until either an event
/// arrives or the underlying [`FpssClient`] has been shut down. After
/// shutdown the iterator drains every queued event and then returns
/// `None`, matching the `StopIteration` contract on the Python binding.
///
/// Cloning is intentionally not implemented: the queue is single-consumer
/// by design, and an SDK that handed multiple iterator instances to user
/// code would silently fan out events across them. Callers that need a
/// secondary observation path should pull events from a single iterator
/// and re-broadcast.
pub struct EventIterator {
    /// Same `Arc<ArrayQueue>` the [`Delivery::Queue`] consumer pushes
    /// into. The Disruptor consumer thread owns the producer side; this
    /// iterator owns the consumer side.
    queue: Arc<crossbeam_queue::ArrayQueue<FpssEvent>>,
    /// Set to `true` by [`Self::close`] (or, on the language bindings,
    /// when the user explicitly stops streaming) to break the wait loop
    /// even if the underlying [`FpssClient`] is still draining late
    /// events. Independent of [`Self::iter_closed`] so the iterator
    /// can be retired without forcing a global shutdown.
    finished: Arc<AtomicBool>,
    /// Terminal-EOF predicate. Flipped to `true` by the Disruptor
    /// consumer thread's drop guard AFTER its consume loop has exited
    /// and every in-flight event has been pushed onto [`Self::queue`].
    /// This is the only point in the system where "no more pushes will
    /// ever happen" is observable, so the iterator keys terminal EOF
    /// off this flag — NOT off [`FpssClient::shutdown`]. Pre-`9.1.0`
    /// the iterator predicate read the raw `shutdown` flag; that
    /// flipped to `true` inside `stop_streaming()` BEFORE the consumer
    /// had finished draining the ring buffer into the queue, so an
    /// iterator caller polling `next_timeout` between those two
    /// moments observed `Closed` and dropped the tail of events on
    /// the floor.
    iter_closed: Arc<AtomicBool>,
}

impl EventIterator {
    /// Pop the next event, blocking until one is available or the
    /// underlying streaming session is shut down.
    ///
    /// Returns `None` when [`Self::close`] has been called, or when the
    /// owning [`FpssClient`] has shut down and the queue is fully
    /// drained. Otherwise spins in a 100 µs sleep loop on an empty
    /// queue — matching the databento `for record in client:` polling
    /// shape but without their intermediate `queue.Queue` middleman.
    ///
    /// On the Python binding the surrounding PyO3 wrapper releases the
    /// GIL across this call (`py.allow_threads`) so the Disruptor
    /// consumer can acquire the GIL to push the next event without
    /// fighting the iterator thread for it.
    #[must_use]
    pub fn next(&self) -> Option<FpssEvent> {
        loop {
            if let Some(evt) = self.queue.pop() {
                return Some(evt);
            }
            // Empty queue. If either local close or the consumer-side
            // drain guard has fired, no further pushes can land —
            // re-check the queue once more to absorb the final-push
            // window between the last `queue.push` and the guard's
            // store, then return.
            if self.finished.load(Ordering::Acquire) || self.iter_closed.load(Ordering::Acquire) {
                return self.queue.pop();
            }
            // 100 µs ≈ 100k iterations/s upper bound on the polling
            // cost during quiet periods; under load the queue is rarely
            // observed empty so the sleep path is cold.
            std::thread::sleep(Duration::from_micros(100));
        }
    }

    /// Try to pop without sleeping, returning the same typed three-state
    /// outcome as [`Self::next_timeout`].
    ///
    /// Returns:
    ///
    /// * [`NextEvent::Ready`] — an event was popped from the queue.
    /// * [`NextEvent::Timeout`] — the queue was empty but the upstream
    ///   stream is still live; the caller should re-poll later.
    /// * [`NextEvent::Closed`] — the iterator (`close()`) or the
    ///   upstream [`FpssClient`] has shut down AND the queue is fully
    ///   drained; no further events will ever arrive.
    ///
    /// Pre-`9.1.0` this returned `Option<FpssEvent>` and overloaded
    /// `None` to mean both "queue empty right now" AND "queue closed
    /// forever". The C ABI's non-blocking poll path inherited that
    /// ambiguity and reported every `None` as `Timeout`, so a C client
    /// calling `tdx_fpss_event_iter_next(.., 0)` after
    /// `stop_streaming()` would see `1` (timeout) forever instead of
    /// `-1` (terminal). The typed return makes the contract symmetric
    /// with `next_timeout` and lets every binding map `Closed` to its
    /// terminal sentinel (C ABI `-1`, C++ `ended_ = true`, Python
    /// `StopIteration`, TypeScript `done: true`).
    #[must_use]
    pub fn try_next(&self) -> NextEvent {
        if let Some(evt) = self.queue.pop() {
            return NextEvent::Ready(evt);
        }
        // Empty queue. If either local close or the consumer-side
        // drain guard has fired, the producer side has run dry; re-
        // check the queue once more to close the final-push race
        // (mirror of [`Self::next_timeout`]'s logic), then signal
        // terminal Closed. Otherwise the upstream is still live —
        // signal Timeout so a polling caller re-polls instead of
        // false-EOFing.
        if self.finished.load(Ordering::Acquire) || self.iter_closed.load(Ordering::Acquire) {
            return match self.queue.pop() {
                Some(evt) => NextEvent::Ready(evt),
                None => NextEvent::Closed,
            };
        }
        NextEvent::Timeout
    }

    /// Pop the next event with a deadline, returning a typed three-state
    /// outcome that disambiguates timeout from end-of-stream.
    ///
    /// Returns:
    ///
    /// * [`NextEvent::Ready`] — an event was popped within the wait
    ///   window.
    /// * [`NextEvent::Timeout`] — the wait window expired with no event,
    ///   and the upstream stream is still live. Loop and re-poll.
    /// * [`NextEvent::Closed`] — the iterator was closed or the
    ///   upstream [`FpssClient`] has shut down AND the queue is fully
    ///   drained. Stop iterating.
    ///
    /// The blocking [`Iterator::next`] impl on `EventIterator` (no
    /// timeout) keeps returning `Option<FpssEvent>` — there `None`
    /// unambiguously means terminal because that path blocks until
    /// either an event arrives or the queue closes.
    #[must_use]
    pub fn next_timeout(&self, timeout: Duration) -> NextEvent {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(evt) = self.queue.pop() {
                return NextEvent::Ready(evt);
            }
            // Empty queue. If either local close or the consumer-side
            // drain guard has fired, drain any final residual then
            // signal terminal. The double-pop closes the final-push
            // race (mirror of [`Self::next`]'s logic).
            if self.finished.load(Ordering::Acquire) || self.iter_closed.load(Ordering::Acquire) {
                return match self.queue.pop() {
                    Some(evt) => NextEvent::Ready(evt),
                    None => NextEvent::Closed,
                };
            }
            if std::time::Instant::now() >= deadline {
                return NextEvent::Timeout;
            }
            std::thread::sleep(Duration::from_micros(100));
        }
    }

    /// Mark the iterator as finished so [`Self::next`] returns once the
    /// queue is drained, without requiring the underlying
    /// [`FpssClient`] to shut down. Idempotent.
    pub fn close(&self) {
        self.finished.store(true, Ordering::Release);
    }

    /// Snapshot of the number of events currently queued (between the
    /// Disruptor consumer thread and the iterator caller). Useful for
    /// tests and operator dashboards.
    #[must_use]
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Test-only constructor that wires an [`EventIterator`] onto an
    /// externally-supplied queue + drain-guard flag, bypassing the
    /// Disruptor + TLS plumbing of [`FpssClient::connect_iter`].
    ///
    /// Soak tests use this to drive the iterator's `next` loop
    /// against synthetic events without standing up a live FPSS
    /// session. `iter_closed` corresponds to the same flag the
    /// production drop guard flips after the consumer thread exits;
    /// tests assert no false-EOF by writing into the queue, then
    /// flipping the flag, then draining.
    #[cfg(test)]
    #[doc(hidden)]
    pub(crate) fn for_test(
        queue: Arc<crossbeam_queue::ArrayQueue<FpssEvent>>,
        iter_closed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            queue,
            finished: Arc::new(AtomicBool::new(false)),
            iter_closed,
        }
    }
}

impl Iterator for EventIterator {
    type Item = FpssEvent;
    fn next(&mut self) -> Option<FpssEvent> {
        EventIterator::next(self)
    }
}

impl Drop for FpssClient {
    fn drop(&mut self) {
        // Signal shutdown if not already done.
        self.shutdown.store(true, Ordering::Release);
        // Send shutdown command so I/O thread exits its loop.
        let _ = self.send_cmd(IoCommand::Shutdown);

        // Self-join guard.
        //
        // The exit path of the I/O thread drops the Disruptor producer
        // (`crates/thetadatadx/src/fpss/io_loop/mod.rs:640`), and
        // `disruptor::Producer::drop` joins the consumer thread
        // (`disruptor` 4.x `single.rs`). So `self.io_handle.join()`
        // transitively joins the consumer thread.
        //
        // If `Drop` is running on either of those threads — the I/O
        // thread itself, or the Disruptor consumer thread (the thread
        // running the user callback) — joining the I/O handle inline
        // would block the very thread cleanup needs to complete on,
        // producing a self-join deadlock. The consumer-thread case is
        // load-bearing: a user callback that calls
        // `ThetaDataDxClient::stop_streaming()` swaps the live slot to
        // `Stopped` and drops the last `Arc<FpssClient>` while running
        // on the consumer thread.
        //
        // Detach the join onto a helper thread in those cases. Cleanup
        // still completes; observers see `is_streaming()` flip to
        // `false` once the helper finishes, instead of `Drop` blocking
        // forever.
        let cur = thread::current().id();
        let consumer_id = self.consumer_thread_id.get().copied();

        // Take both handles up-front so the helper-thread path can move
        // them into the detached closure.
        let ping_handle = self.ping_handle.take();
        let io_handle = self.io_handle.take();

        let io_handle_thread_id = io_handle.as_ref().map(|h| h.thread().id());

        let self_join = io_handle_thread_id == Some(cur) || consumer_id == Some(cur);

        if self_join {
            // Detach on a fresh thread so the consumer thread (or the
            // I/O thread itself) is not blocked waiting on its own
            // termination. The detached helper signals `drained` once
            // both joins have returned so callers polling
            // `await_drain` see exact quiescence.
            let drained_flag = Arc::clone(&self.drained);
            let detached = thread::Builder::new()
                .name("fpss-shutdown-detach".to_owned())
                .spawn(move || {
                    if let Some(h) = ping_handle {
                        let _ = h.join();
                    }
                    if let Some(h) = io_handle {
                        let _ = h.join();
                    }
                    drained_flag.store(true, Ordering::Release);
                });
            if let Err(e) = detached {
                tracing::warn!(
                    error = %e,
                    "failed to spawn fpss-shutdown-detach; handles will be leaked rather than \
                     attempting an inline join that would deadlock the current thread"
                );
                // Best-effort path: spawning a thread realistically only
                // fails on catastrophic OOM / FD exhaustion. We choose
                // to leak both handles (they were already moved out of
                // `self`) rather than risk an inline join that would
                // self-deadlock the consumer or I/O thread we are
                // running on. The `drained` flag stays `false`; an
                // `await_drain` caller will time out, which is the
                // honest answer for an unreachable cleanup state.
            }
            return;
        }

        if let Some(h) = ping_handle {
            let _ = h.join();
        }
        if let Some(h) = io_handle {
            let _ = h.join();
        }
        self.drained.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod connect_args_tests {
    use super::*;
    use crate::config::DirectConfig;

    /// `FpssConnectArgs::new` seeds the timing knobs from the
    /// protocol-level Java-parity constants so a caller that does
    /// not override them reproduces the legacy behaviour exactly.
    /// Regression guard against the fields silently going to `0`.
    #[test]
    fn new_seeds_timing_defaults_from_java_parity_constants() {
        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("nj-a.thetadata.us".to_owned(), 20000)];
        let args = FpssConnectArgs::new(&creds, &hosts);
        assert_eq!(args.connect_timeout_ms, protocol::CONNECT_TIMEOUT_MS);
        assert_eq!(args.read_timeout_ms, protocol::READ_TIMEOUT_MS);
        assert_eq!(args.ping_interval_ms, protocol::PING_INTERVAL_MS);
    }

    /// `FpssClient::connect` rejects a `read_timeout_ms` outside the
    /// validated range. This is the second-line defence for callers
    /// that bypass `DirectConfig::validate` and hand-roll an args
    /// bundle with a stale field.
    #[test]
    fn connect_rejects_out_of_range_read_timeout_ms() {
        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("127.0.0.1".to_owned(), 1)];
        let mut args = FpssConnectArgs::new(&creds, &hosts);
        args.read_timeout_ms = 50; // below 100 ms minimum
        let res = FpssClient::connect(args, |_| {});
        let err = match res {
            Ok(_) => panic!("must reject"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(msg.contains("read_timeout_ms"), "{msg}");
    }

    /// Same defence for `connect_timeout_ms`.
    #[test]
    fn connect_rejects_out_of_range_connect_timeout_ms() {
        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("127.0.0.1".to_owned(), 1)];
        let mut args = FpssConnectArgs::new(&creds, &hosts);
        args.connect_timeout_ms = 50; // below 1 s minimum
        let res = FpssClient::connect(args, |_| {});
        let err = match res {
            Ok(_) => panic!("must reject"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("connect_timeout_ms"));
    }

    /// Same defence for `ping_interval_ms`.
    #[test]
    fn connect_rejects_out_of_range_ping_interval_ms() {
        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("127.0.0.1".to_owned(), 1)];
        let mut args = FpssConnectArgs::new(&creds, &hosts);
        args.ping_interval_ms = 50; // below 100 ms minimum
        let res = FpssClient::connect(args, |_| {});
        let err = match res {
            Ok(_) => panic!("must reject"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("ping_interval_ms"));
    }

    /// `start_streaming` reads the wired knobs from `DirectConfig::fpss`
    /// — not from a stale local — so a user override on the config
    /// surfaces at connect time. Verified by inspecting that
    /// `DirectConfig::production().fpss.timeout_ms` matches the
    /// `FpssConnectArgs` field path used by `client.rs::start_streaming`.
    #[test]
    fn production_config_threads_timing_knobs_into_connect_args_shape() {
        let cfg = DirectConfig::production();
        let creds = Credentials::new("user", "pw");
        let args = FpssConnectArgs {
            creds: &creds,
            hosts: &cfg.fpss.hosts,
            ring_size: cfg.fpss.ring_size,
            flush_mode: cfg.fpss.flush_mode,
            policy: cfg.reconnect.policy.clone(),
            wait_ms: cfg.reconnect.wait_ms,
            wait_rate_limited_ms: cfg.reconnect.wait_rate_limited_ms,
            derive_ohlcvc: cfg.fpss.derive_ohlcvc,
            connect_timeout_ms: cfg.fpss.connect_timeout_ms,
            read_timeout_ms: cfg.fpss.timeout_ms,
            ping_interval_ms: cfg.fpss.ping_interval_ms,
        };
        // The args struct is the single channel through which
        // tuning reaches the runtime. If a future refactor drops
        // any of these fields, this test fails to compile, which is
        // the desired regression guard.
        assert_eq!(args.connect_timeout_ms, cfg.fpss.connect_timeout_ms);
        assert_eq!(args.read_timeout_ms, cfg.fpss.timeout_ms);
        assert_eq!(args.ping_interval_ms, cfg.fpss.ping_interval_ms);
        assert_eq!(args.ring_size, cfg.fpss.ring_size);
        assert_eq!(args.wait_ms, cfg.reconnect.wait_ms);
        assert_eq!(
            args.wait_rate_limited_ms,
            cfg.reconnect.wait_rate_limited_ms
        );
    }
}
