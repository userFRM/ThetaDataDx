//! FPSS real-time streaming client.
//!
//! Synchronous blocking I/O on `std::thread` (no tokio). A TLS reader
//! publishes events to a lock-free event ring; the consumer
//! thread invokes the user callback inside `std::panic::catch_unwind`
//! so panics are counted on [`StreamingClient::panic_count`] rather than
//! tearing down the pipeline. See `docs-site/docs/streaming/index.md`
//! for the architectural overview.
//!
//! # Examples
//!
//! ```rust,no_run
//! # use thetadatadx::fpss::{StreamingClient, StreamEvent};
//! # use thetadatadx::auth::Credentials;
//! # use thetadatadx::fpss::protocol::Contract;
//! # fn example() -> Result<(), thetadatadx::fpss::FpssError> {
//! let creds = Credentials::new("user@example.com", "pw");
//! let hosts = thetadatadx::config::DirectConfig::production().streaming.hosts;
//!
//! let client = StreamingClient::builder(&creds, &hosts).build()?;
//! client.subscribe(Contract::stock("AAPL").quote())?;
//!
//! for event in &client {
//!     let _event: StreamEvent = event?;
//!     // ...
//! }
//! # Ok(())
//! # }
//! ```

mod accumulator;
pub(crate) mod affinity;
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

pub use self::decode::UNRESOLVED_CONTRACT_SYMBOL_PREFIX;
use self::events::IoCommand;
pub use self::events::{StreamControl, StreamData, StreamEvent};
pub use self::framing::{read_frame, write_frame, Frame};
use self::io_loop::{io_loop, ping_loop, wait_for_login, LoginResult};
pub use self::session::{reconnect_delay, reconnect_delay_for};

// Lock-free event ring buffer plumbing. The third-party ring-buffer
// crate's `EventPoller` / `Polling` / `SingleProducerBarrier` types are
// stored inside `StreamingClient::poller_state` and never reach the public
// signature surface; these imports are crate-internal only.
use self::ring::{RingCursors, RingEvent};
use disruptor::wait_strategies::WaitStrategy; // VOCAB-OK: internal crate name, not user-facing
use disruptor::{EventPoller, Polling, SingleProducerBarrier}; // VOCAB-OK: internal crate name, not user-facing

/// Hidden test-internals surface for vendor-failure-mode resilience tests
/// in `crates/thetadatadx/tests/`.
///
/// Re-exports the otherwise crate-private `decode_frame` dispatcher and
/// `DeltaState` so integration tests can drive the full
/// `read_frame_into → decode_frame → StreamEvent` pipeline against
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

    // Production ring-constructor surface, re-exported so the streaming
    // channel bench can time the exact pipeline the live client builds:
    // `build_poller_producer` wires the sequence-recording producer
    // adapter (`SequencedProducer`) over the ring and pairs it with the
    // poller the consumer drains. Timing a raw `build_single_producer`
    // ring instead would pin the shared ring machinery while leaving the
    // instrumented publish path — one relaxed occupancy store per
    // publish, one per drained batch — unmeasured. `RingCursors` is the
    // shared occupancy cursor pair the adapter writes into; `RingEvent`
    // is the ring slot; `Polling` discriminates the poller drain result.
    pub use super::io_loop::build_poller_producer;
    pub use super::ring::{AdaptiveWaitStrategy, RingCursors, RingEvent, RingProducer};
    pub use disruptor::Polling; // VOCAB-OK: internal crate name, not user-facing
}

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Duration;

use crate::auth::Credentials;
use crate::backoff::JitterMode;
use crate::config::{
    HostSelectionPolicy, ReconnectPolicy, StreamingFlushMode, StreamingWaitStrategy,
};
use crate::error::Error;
use crate::tdbe::types::enums::{RemoveReason, SecType, StreamMsgType};

use self::protocol::{
    build_credentials_payload, build_subscribe_payload, Contract, SubscriptionKind,
};

/// Capacity of the bounded command channel from the public surface and the
/// ping thread to the I/O thread.
///
/// The channel carries control-plane frames only (subscribe / unsubscribe /
/// ping / shutdown), never market-data ticks. A subscribe burst across a full
/// 10k-15k option watchlist enqueues one command per contract, so the bound is
/// sized comfortably above a single large watchlist while still capping
/// unbounded growth under a pathological control-plane storm. The I/O thread
/// drains this queue every read-timeout cycle, so steady-state occupancy is
/// near zero.
///
/// The ping thread uses a blocking `send` for natural backpressure; the public
/// subscribe / unsubscribe methods use a non-blocking `try_send` and surface a
/// typed [`FpssErrorKind::Disconnected`] backpressure error rather than ever
/// silently dropping a command.
pub(in crate::fpss) const CMD_CHANNEL_CAPACITY: usize = 16_384;

// ---------------------------------------------------------------------------
// FpssError — typed error enum returned by the FPSS public surface
// ---------------------------------------------------------------------------

/// Typed errors returned by the FPSS client public surface.
///
/// Each variant pairs a concrete failure category with a human-readable
/// detail string. Mark on a `match` arm to dispatch retry / fail-fast
/// behaviour without parsing strings. The enum is `#[non_exhaustive]`
/// so new variants can be added without breaking SemVer.
///
/// # Conversions
///
/// `From<FpssError> for Error` maps each `FpssError` variant into an
/// umbrella [`crate::Error`] variant according to the table below.
/// `DispatcherFailed` does NOT have a dedicated umbrella variant; it
/// is encoded as `Error::Fpss { kind: Disconnected }` with a
/// `"dispatcher failed: "` prefix on the message, which the reverse
/// direction recognises:
///
/// | `FpssError`               | `Error`                                                         |
/// |---------------------------|-----------------------------------------------------------------|
/// | `ConnectionRefused(m)`    | `Error::Fpss { kind: ConnectionRefused, message: m }`           |
/// | `Timeout(m)`              | `Error::Fpss { kind: Timeout, message: m }`                     |
/// | `Protocol(m)`             | `Error::Fpss { kind: ProtocolError, message: m }`               |
/// | `Disconnected(m)`         | `Error::Fpss { kind: Disconnected, message: m }`                |
/// | `RateLimited(m)`          | `Error::Fpss { kind: TooManyRequests, message: m }`             |
/// | `AuthenticationFailed(m)` | `Error::Auth { kind: InvalidCredentials, message: m }`          |
/// | `Config(m)`               | `Error::Config { kind: InvalidValue { field: "fpss", message }}`|
/// | `Io(m)`                   | `Error::Io(io::Error::other(m))`                                |
/// | `DispatcherFailed(m)`     | `Error::Fpss { kind: Disconnected, message: "dispatcher failed: {m}" }` |
///
/// Round-tripping `FpssError → Error → FpssError` preserves the
/// variant for every row above (the prefixed message lets the
/// `Disconnected → DispatcherFailed` decoder run) and preserves the
/// message string verbatim. Two caveats:
///
/// - `Io(m) → Error::Io(io::Error::other(m)) → Io(io.to_string())`
///   preserves the message text but the synthesised inner
///   `io::Error` reports `ErrorKind::Other`. A caller that
///   round-trips through `Error` and then inspects the recovered
///   `io::ErrorKind` will see `Other`, not whatever kind the original
///   `FpssError::Io` was carrying in its string form.
/// - `Config(m) → Error::Config { kind: InvalidValue { field: "fpss",
///   message } }` regenerates `field = "fpss"` unconditionally. A
///   caller that converted an `Error::Config { kind: InvalidValue {
///   field: "<custom>", .. } }` into `FpssError::Config` loses the
///   original field name on the round trip.
/// - `Disconnected(m)` with a user-supplied `m` that happens to start
///   with the literal `"dispatcher failed: "` prefix re-emerges as
///   `DispatcherFailed` — do not author messages with that prefix
///   manually.
///
/// `From<Error> for FpssError` is **best-effort categorisation**. The
/// FPSS-shaped umbrella variants (`Error::Fpss`, `Error::Auth`,
/// `Error::Config`, `Error::Io`, `Error::Tls`, `Error::Timeout`)
/// preserve their human-readable message and route to the closest
/// `FpssError` variant; everything else (gRPC, decode, transport)
/// collapses to `FpssError::Protocol` with the `Display` string of the
/// source error. Use this direction at SDK boundaries where the
/// caller already knows the error originated on the FPSS surface.
#[derive(thiserror::Error, Debug, Clone)]
#[non_exhaustive]
pub enum FpssError {
    /// Could not connect to any FPSS server (TLS handshake failed, no
    /// route, DNS failure, etc.).
    #[error("connection refused: {0}")]
    ConnectionRefused(String),

    /// Operation timed out (initial connect, login, or read deadline).
    #[error("timeout: {0}")]
    Timeout(String),

    /// Wire-protocol violation (unexpected frame, malformed payload, or
    /// decoder failure).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Server closed the connection.
    #[error("disconnected: {0}")]
    Disconnected(String),

    /// Server replied `TOO_MANY_REQUESTS`; back off before retrying.
    #[error("rate limited: {0}")]
    RateLimited(String),

    /// Authentication failed (invalid credentials, expired session,
    /// server-side rejection).
    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Builder validation failed (missing field, out-of-range value,
    /// invalid host:port).
    #[error("configuration error: {0}")]
    Config(String),

    /// Internal supervisor thread terminated unexpectedly. The next
    /// `next_event()` / iterator pull surfaces this variant; the
    /// client has transitioned to a failed state.
    #[error("dispatcher failed: {0}")]
    DispatcherFailed(String),

    /// I/O error on the FPSS socket.
    #[error("io: {0}")]
    Io(String),
}

impl From<FpssError> for Error {
    fn from(e: FpssError) -> Self {
        use crate::error::{AuthErrorKind, ConfigErrorKind, FpssErrorKind};
        match e {
            FpssError::ConnectionRefused(message) => Error::Fpss {
                kind: FpssErrorKind::ConnectionRefused,
                message,
            },
            FpssError::Timeout(message) => Error::Fpss {
                kind: FpssErrorKind::Timeout,
                message,
            },
            FpssError::Protocol(message) => Error::Fpss {
                kind: FpssErrorKind::ProtocolError,
                message,
            },
            FpssError::Disconnected(message) => Error::Fpss {
                kind: FpssErrorKind::Disconnected,
                message,
            },
            FpssError::RateLimited(message) => Error::Fpss {
                kind: FpssErrorKind::TooManyRequests,
                message,
            },
            FpssError::AuthenticationFailed(message) => Error::Auth {
                kind: AuthErrorKind::InvalidCredentials,
                message,
            },
            FpssError::DispatcherFailed(message) => Error::Fpss {
                kind: FpssErrorKind::Disconnected,
                message: format!("dispatcher failed: {message}"),
            },
            FpssError::Config(message) => Error::Config {
                kind: ConfigErrorKind::InvalidValue {
                    field: "fpss".to_string(),
                    message: message.clone(),
                },
                message,
                source: None,
            },
            FpssError::Io(message) => Error::Io(std::io::Error::other(message)),
        }
    }
}

impl From<Error> for FpssError {
    fn from(e: Error) -> Self {
        use crate::error::FpssErrorKind;
        match e {
            Error::Fpss { kind, message } => match kind {
                FpssErrorKind::ConnectionRefused => FpssError::ConnectionRefused(message),
                FpssErrorKind::Timeout => FpssError::Timeout(message),
                FpssErrorKind::ProtocolError => FpssError::Protocol(message),
                FpssErrorKind::Disconnected => {
                    if let Some(payload) = message.strip_prefix("dispatcher failed: ") {
                        FpssError::DispatcherFailed(payload.to_string())
                    } else {
                        FpssError::Disconnected(message)
                    }
                }
                FpssErrorKind::TooManyRequests => FpssError::RateLimited(message),
            },
            Error::Auth { message, .. } => FpssError::AuthenticationFailed(message),
            Error::Io(io) => FpssError::Io(io.to_string()),
            Error::Tls(t) => FpssError::ConnectionRefused(t.to_string()),
            Error::Timeout { duration_ms } => {
                FpssError::Timeout(format!("deadline exceeded after {duration_ms}ms"))
            }
            Error::Config { message, .. } => FpssError::Config(message),
            other => FpssError::Protocol(other.to_string()),
        }
    }
}

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

/// Whether a security type has an upstream full-stream broadcast.
///
/// Full-stream subscriptions are only broadcast for [`SecType::Stock`] and
/// [`SecType::Option`]. The server accepts a full-stream subscribe frame for
/// other security types and answers with a `Subscribed` response, but never
/// streams a tick, so the subscribe boundary rejects them up front rather than
/// leaving the caller waiting on a feed that will never arrive. Indices and
/// rates are addressed per-contract instead
/// (for example `Contract::index("VIX").trade()`).
#[must_use]
pub(crate) fn full_stream_sec_type_supported(sec_type: SecType) -> bool {
    matches!(sec_type, SecType::Stock | SecType::Option)
}

// ---------------------------------------------------------------------------
// StreamingClientBuilder — fluent constructor for `StreamingClient`
// ---------------------------------------------------------------------------

/// Fluent builder for an [`StreamingClient`].
///
/// Holds the connection-side knobs (credentials, hosts, ring size,
/// flush mode, reconnect policy, OHLCVC derivation, timeouts) and
/// returns a connected client from [`Self::build`]. Optional setters
/// consume `self` so calls chain.
///
/// # Example
///
/// ```rust,no_run
/// # use thetadatadx::fpss::{StreamingClient, StreamEvent};
/// # use thetadatadx::auth::Credentials;
/// # fn example() -> Result<(), thetadatadx::fpss::FpssError> {
/// let creds = Credentials::new("user@example.com", "pw");
/// let hosts = thetadatadx::config::DirectConfig::production().streaming.hosts;
///
/// let client = StreamingClient::builder(&creds, &hosts)
///     .ring_size(8192)
///     .read_timeout_ms(15_000)
///     .build()?;
/// # let _ = client;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct StreamingClientBuilder<'a> {
    creds: &'a Credentials,
    hosts: &'a [(String, u16)],
    ring_size: usize,
    flush_mode: StreamingFlushMode,
    policy: ReconnectPolicy,
    wait_ms: u64,
    wait_max_ms: u64,
    wait_rate_limited_ms: u64,
    wait_server_restart_ms: u64,
    jitter: JitterMode,
    replay_burst_size: u32,
    replay_pace_ms: u64,
    derive_ohlcvc: bool,
    connect_timeout_ms: u64,
    read_timeout_ms: u64,
    ping_interval_ms: u64,
    io_read_slice_ms: u64,
    data_watchdog_ms: u64,
    keepalive_idle_secs: u64,
    keepalive_interval_secs: u64,
    keepalive_retries: u32,
    host_selection: HostSelectionPolicy,
    host_shuffle_seed: Option<u64>,
    wait_strategy: ring::AdaptiveWaitStrategy,
    consumer_cpu: Option<usize>,
}

impl<'a> StreamingClientBuilder<'a> {
    /// Construct a builder with the two required arguments and SDK
    /// defaults for the rest.
    #[must_use]
    pub fn new(creds: &'a Credentials, hosts: &'a [(String, u16)]) -> Self {
        let reconnect = crate::config::ReconnectConfig::production_defaults();
        let fpss = crate::config::StreamingConfig::production_defaults();
        Self {
            creds,
            hosts,
            // Match the production ring size so a direct-builder user gets
            // production-grade headroom by default. ThetaData streams large
            // shapes (10k-15k option contracts plus full trade streams); a
            // small default ring overflows under real market bursts and drops
            // the newest events. Callers that want a smaller footprint can set
            // `.ring_size(..)` explicitly.
            ring_size: fpss.ring_size,
            flush_mode: StreamingFlushMode::default(),
            policy: ReconnectPolicy::default(),
            wait_ms: reconnect.wait_ms,
            wait_max_ms: reconnect.wait_max_ms,
            wait_rate_limited_ms: reconnect.wait_rate_limited_ms,
            wait_server_restart_ms: reconnect.wait_server_restart_ms,
            jitter: reconnect.jitter,
            replay_burst_size: reconnect.replay_burst_size,
            replay_pace_ms: reconnect.replay_pace_ms,
            derive_ohlcvc: true,
            connect_timeout_ms: fpss.connect_timeout_ms,
            read_timeout_ms: fpss.timeout_ms,
            ping_interval_ms: fpss.ping_interval_ms,
            io_read_slice_ms: fpss.io_read_slice_ms,
            data_watchdog_ms: fpss.data_watchdog_ms,
            keepalive_idle_secs: fpss.keepalive_idle_secs,
            keepalive_interval_secs: fpss.keepalive_interval_secs,
            keepalive_retries: fpss.keepalive_retries,
            host_selection: fpss.host_selection,
            host_shuffle_seed: fpss.host_shuffle_seed,
            wait_strategy: fpss.build_wait_strategy(),
            consumer_cpu: fpss.consumer_cpu,
        }
    }

    /// Event ring buffer size (events). Must be a power of two.
    ///
    /// Default `4096`. Each slot stores one event (96 bytes on the
    /// current 64-bit layout, validated by `assert_layout_compat`), so
    /// `4096 × 96 ≈ 384 KiB` per client plus refcounted `Arc<Contract>`
    /// storage on top. Tune upward (e.g. `16_384`) if you observe
    /// sustained ring-overflow drops on bursty load.
    #[must_use]
    pub fn ring_size(mut self, n: usize) -> Self {
        self.ring_size = n;
        self
    }

    /// I/O thread flush behaviour. See [`StreamingFlushMode`].
    #[must_use]
    pub fn flush_mode(mut self, m: StreamingFlushMode) -> Self {
        self.flush_mode = m;
        self
    }

    /// Event-ring consumer wait strategy preset. See
    /// [`StreamingWaitStrategy`] for the latency-vs-CPU trade-off of each
    /// preset.
    ///
    /// Selecting a preset resets the spin / yield / park tuning to that
    /// preset's defaults; use [`Self::wait_strategy_tuning`] afterwards to
    /// override the individual counts. Rust callers that want to supply
    /// their own [`disruptor::wait_strategies::WaitStrategy`] impl use
    /// [`StreamingClient::for_each_with_wait_strategy`] instead.
    #[must_use]
    pub fn wait_strategy(mut self, strategy: StreamingWaitStrategy) -> Self {
        self.wait_strategy = match strategy {
            StreamingWaitStrategy::LowLatency => ring::AdaptiveWaitStrategy::low_latency(),
            StreamingWaitStrategy::Balanced => ring::AdaptiveWaitStrategy::balanced(),
            StreamingWaitStrategy::Efficient => ring::AdaptiveWaitStrategy::efficient(),
            StreamingWaitStrategy::BusySpin => ring::AdaptiveWaitStrategy::busy_spin(),
        };
        self
    }

    /// Override the spin / yield / park counts of the currently-selected
    /// wait strategy preset. Each value is clamped to a sane upper bound.
    ///
    /// `park_us` is inert under [`StreamingWaitStrategy::LowLatency`] /
    /// [`StreamingWaitStrategy::BusySpin`], which never sleep.
    #[must_use]
    pub fn wait_strategy_tuning(mut self, spin_iters: u32, yield_iters: u32, park_us: u64) -> Self {
        self.wait_strategy = self
            .wait_strategy
            .with_tuning(spin_iters, yield_iters, park_us);
        self
    }

    /// Pin the event-ring consumer thread to a specific CPU core.
    ///
    /// `Some(core_id)` pins the tick-consumer thread to that core for
    /// deterministic, low-jitter delivery (pair with an isolated core
    /// for best results); `None` (default) leaves the thread under the
    /// OS scheduler — the historical behaviour. An out-of-range core id
    /// is a no-op at the affinity layer rather than a hard error.
    #[must_use]
    pub fn consumer_cpu(mut self, core: Option<usize>) -> Self {
        self.consumer_cpu = core;
        self
    }

    /// Auto-reconnect policy after involuntary disconnect.
    #[must_use]
    pub fn reconnect_policy(mut self, p: ReconnectPolicy) -> Self {
        self.policy = p;
        self
    }

    /// Initial delay (ms) of the exponential reconnect ladder for
    /// generic transient drops (`TimedOut`, `Unspecified`, ...).
    /// Doubles per consecutive attempt up to
    /// [`Self::reconnect_wait_max_ms`].
    #[must_use]
    pub fn reconnect_wait_ms(mut self, ms: u64) -> Self {
        self.wait_ms = ms;
        self
    }

    /// Cap (ms) on the exponential generic-transient reconnect ladder.
    #[must_use]
    pub fn reconnect_wait_max_ms(mut self, ms: u64) -> Self {
        self.wait_max_ms = ms;
        self
    }

    /// Floor delay (ms) before reconnecting after a `TooManyRequests`
    /// drop. Jitter samples above the floor, never below it.
    #[must_use]
    pub fn reconnect_wait_rate_limited_ms(mut self, ms: u64) -> Self {
        self.wait_rate_limited_ms = ms;
        self
    }

    /// Flat reconnect cadence (ms) for `ServerRestarting` drops.
    #[must_use]
    pub fn reconnect_wait_server_restart_ms(mut self, ms: u64) -> Self {
        self.wait_server_restart_ms = ms;
        self
    }

    /// Jitter strategy applied to every reconnect delay. See
    /// [`JitterMode`].
    #[must_use]
    pub fn reconnect_jitter(mut self, mode: JitterMode) -> Self {
        self.jitter = mode;
        self
    }

    /// Subscription-replay frames per burst on the auto-reconnect
    /// path. Clamped to a minimum of `1`.
    #[must_use]
    pub fn reconnect_replay_burst_size(mut self, n: u32) -> Self {
        self.replay_burst_size = n;
        self
    }

    /// Pause (ms) between subscription-replay bursts on the
    /// auto-reconnect path. `0` removes the pause.
    #[must_use]
    pub fn reconnect_replay_pace_ms(mut self, ms: u64) -> Self {
        self.replay_pace_ms = ms;
        self
    }

    /// When `false`, suppresses locally-derived `StreamData::Ohlcvc`
    /// events. Server-sent OHLCVC frames still pass through.
    #[must_use]
    pub fn derive_ohlcvc(mut self, on: bool) -> Self {
        self.derive_ohlcvc = on;
        self
    }

    /// Per-server TCP connect timeout in milliseconds.
    #[must_use]
    pub fn connect_timeout_ms(mut self, ms: u64) -> Self {
        self.connect_timeout_ms = ms;
        self
    }

    /// FPSS read timeout in milliseconds. Drives the framing layer's
    /// mid-frame stall budget and the I/O loop's no-data deadline.
    #[must_use]
    pub fn read_timeout_ms(mut self, ms: u64) -> Self {
        self.read_timeout_ms = ms;
        self
    }

    /// FPSS heartbeat ping interval in milliseconds.
    #[must_use]
    pub fn ping_interval_ms(mut self, ms: u64) -> Self {
        self.ping_interval_ms = ms;
        self
    }

    /// Per-iteration blocking-read slice (ms) for the I/O loop.
    #[must_use]
    pub fn io_read_slice_ms(mut self, ms: u64) -> Self {
        self.io_read_slice_ms = ms;
        self
    }

    /// Last-frame watchdog (ms); `0` disables. See
    /// [`crate::config::StreamingConfig::data_watchdog_ms`].
    #[must_use]
    pub fn data_watchdog_ms(mut self, ms: u64) -> Self {
        self.data_watchdog_ms = ms;
        self
    }

    /// TCP keepalive idle time (seconds) before the first probe.
    #[must_use]
    pub fn keepalive_idle_secs(mut self, secs: u64) -> Self {
        self.keepalive_idle_secs = secs;
        self
    }

    /// TCP keepalive probe interval (seconds).
    #[must_use]
    pub fn keepalive_interval_secs(mut self, secs: u64) -> Self {
        self.keepalive_interval_secs = secs;
        self
    }

    /// TCP keepalive probe count before the kernel declares the peer
    /// dead (where the platform exposes the knob).
    #[must_use]
    pub fn keepalive_retries(mut self, retries: u32) -> Self {
        self.keepalive_retries = retries;
        self
    }

    /// Host-ordering policy for connect + failover. See
    /// [`HostSelectionPolicy`].
    #[must_use]
    pub fn host_selection(mut self, policy: HostSelectionPolicy) -> Self {
        self.host_selection = policy;
        self
    }

    /// Seed for the shuffled host order; `None` derives a fresh
    /// per-client seed. See
    /// [`crate::config::StreamingConfig::host_shuffle_seed`].
    #[must_use]
    pub fn host_shuffle_seed(mut self, seed: Option<u64>) -> Self {
        self.host_shuffle_seed = seed;
        self
    }

    /// Connect, authenticate, and start the background I/O and ping
    /// threads. Returns a ready-to-use [`StreamingClient`].
    ///
    /// # Errors
    ///
    /// Returns [`FpssError::ConnectionRefused`] if no host accepts the
    /// TLS handshake, [`FpssError::AuthenticationFailed`] on login
    /// failure, and other variants on protocol violations or
    /// configuration validation errors.
    pub fn build(self) -> Result<StreamingClient, FpssError> {
        StreamingClient::connect(self.into_args()).map_err(FpssError::from)
    }

    pub(crate) fn into_args(self) -> FpssConnectArgs<'a> {
        FpssConnectArgs {
            creds: self.creds,
            hosts: self.hosts,
            ring_size: self.ring_size,
            flush_mode: self.flush_mode,
            policy: self.policy,
            wait_ms: self.wait_ms,
            wait_max_ms: self.wait_max_ms,
            wait_rate_limited_ms: self.wait_rate_limited_ms,
            wait_server_restart_ms: self.wait_server_restart_ms,
            jitter: self.jitter,
            replay_burst_size: self.replay_burst_size,
            replay_pace_ms: self.replay_pace_ms,
            derive_ohlcvc: self.derive_ohlcvc,
            connect_timeout_ms: self.connect_timeout_ms,
            read_timeout_ms: self.read_timeout_ms,
            ping_interval_ms: self.ping_interval_ms,
            io_read_slice_ms: self.io_read_slice_ms,
            data_watchdog_ms: self.data_watchdog_ms,
            keepalive_idle_secs: self.keepalive_idle_secs,
            keepalive_interval_secs: self.keepalive_interval_secs,
            keepalive_retries: self.keepalive_retries,
            host_selection: self.host_selection,
            host_shuffle_seed: self.host_shuffle_seed,
            wait_strategy: self.wait_strategy,
            consumer_cpu: self.consumer_cpu,
        }
    }
}

// ---------------------------------------------------------------------------
// FpssConnectArgs — crate-internal parameter bundle
// ---------------------------------------------------------------------------

/// Internal parameter bundle for the crate-private connect path.
///
/// Built from [`StreamingClientBuilder::build`] and threaded into the I/O
/// loop. Not part of the public surface — callers use the builder.
#[derive(Clone, Debug)]
pub(crate) struct FpssConnectArgs<'a> {
    pub(crate) creds: &'a Credentials,
    pub(crate) hosts: &'a [(String, u16)],
    pub(crate) ring_size: usize,
    pub(crate) flush_mode: StreamingFlushMode,
    pub(crate) policy: ReconnectPolicy,
    pub(crate) wait_ms: u64,
    pub(crate) wait_max_ms: u64,
    pub(crate) wait_rate_limited_ms: u64,
    pub(crate) wait_server_restart_ms: u64,
    pub(crate) jitter: JitterMode,
    pub(crate) replay_burst_size: u32,
    pub(crate) replay_pace_ms: u64,
    pub(crate) derive_ohlcvc: bool,
    pub(crate) connect_timeout_ms: u64,
    pub(crate) read_timeout_ms: u64,
    pub(crate) ping_interval_ms: u64,
    pub(crate) io_read_slice_ms: u64,
    pub(crate) data_watchdog_ms: u64,
    pub(crate) keepalive_idle_secs: u64,
    pub(crate) keepalive_interval_secs: u64,
    pub(crate) keepalive_retries: u32,
    pub(crate) host_selection: HostSelectionPolicy,
    pub(crate) host_shuffle_seed: Option<u64>,
    /// Resolved event-ring consumer wait strategy. Built from the
    /// configured [`StreamingWaitStrategy`] preset + tuning.
    pub(crate) wait_strategy: ring::AdaptiveWaitStrategy,
    /// Optional CPU core to pin the event-ring consumer thread to;
    /// `None` (default) leaves it under the OS scheduler. Mirrors
    /// [`crate::config::StreamingConfig::consumer_cpu`].
    pub(crate) consumer_cpu: Option<usize>,
}

/// Outcome of a single non-blocking poll inside
/// [`StreamingClient::try_next_event_internal`]. The internal blocking
/// loop (`next_event`) distinguishes "empty right now, retry" from
/// "ring shut down, stop" before mapping back to the public
/// `Option<StreamEvent>` shape.
enum TryNext {
    Event(StreamEvent),
    Empty,
    Shutdown,
}

/// Internal state for [`StreamingClient::poller_state`].
///
/// Pairs the event ring's `EventPoller` with a small staging queue so the
/// event-at-a-time API (`next_event`, the `Iterator` impl) can buffer a
/// drained batch and yield events one by one without rerunning the
/// poller per yield. `pending` is drained before each new `poll()` call;
/// when the producer drops and the ring shuts down with `pending`
/// empty, the entire [`PollerState`] is dropped so subsequent polls
/// short-circuit to `Ok(None)`.
struct PollerState {
    poller: EventPoller<RingEvent, SingleProducerBarrier>,
    pending: VecDeque<StreamEvent>,
    /// Ring sequence of the last slot a drained batch released
    /// (`-1` = nothing consumed yet). Plain `i64` — only the consumer
    /// thread (serialised by the `poller_state` mutex) advances it,
    /// by the drained batch's length per `poll()`. Mirrored into the
    /// shared [`RingCursors`] with one `Relaxed` store per batch so
    /// [`StreamingClient::ring_occupancy`] can sample in-flight depth.
    /// Deliveries from `pending` do not advance it: those events left
    /// the ring on the `poll()` that staged them.
    consumed_seq: i64,
}

/// Selector for the test-only [`StreamingClient::for_self_join_test`]
/// constructor's pre-burst path. Lets soak tests pick between
/// blocking `publish` (matches handshake-time control-frame emission)
/// and non-blocking `try_publish` (matches the live data path that
/// drives the public `dropped_count`).
#[cfg(any(test, feature = "__test-helpers"))]
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

/// Argument bundle for [`StreamingClient::spawn_io_and_assemble`].
///
/// Carries the already-built ring `producer` plus every shared `Arc`,
/// channel end, and tuning value the I/O + ping threads and the
/// assembled [`StreamingClient`] need. Bundled into one struct so the shared
/// spawn helper reads linearly rather than as a long positional list,
/// matching the [`FpssConnectArgs`] convention.
struct SpawnArgs<'a, P> {
    producer: P,
    poller: EventPoller<RingEvent, SingleProducerBarrier>,
    stream: connection::FpssStream,
    cmd_rx: std_mpsc::Receiver<IoCommand>,
    cmd_tx: std_mpsc::SyncSender<IoCommand>,
    ping_cmd_tx: std_mpsc::SyncSender<IoCommand>,
    ring_size: usize,
    permissions: String,
    pending_control: Vec<StreamControl>,
    server_addr: String,
    creds: &'a Credentials,
    hosts: &'a [(String, u16)],
    host_selection: HostSelectionPolicy,
    host_shuffle_seed: u64,
    derive_ohlcvc: bool,
    flush_mode: StreamingFlushMode,
    wait_strategy: ring::AdaptiveWaitStrategy,
    consumer_cpu: Option<usize>,
    policy: ReconnectPolicy,
    wait_ms: u64,
    wait_max_ms: u64,
    wait_rate_limited_ms: u64,
    wait_server_restart_ms: u64,
    jitter: JitterMode,
    replay_burst_size: u32,
    replay_pace_ms: u64,
    connect_timeout: Duration,
    read_timeout: Duration,
    io_read_slice: Duration,
    data_watchdog: Duration,
    keepalive: connection::TcpKeepaliveSpec,
    last_event_at_ns: Arc<AtomicI64>,
    connected_addr: Arc<Mutex<String>>,
    ping_interval: Duration,
    shutdown: Arc<AtomicBool>,
    authenticated: Arc<AtomicBool>,
    active_subs: Arc<Mutex<Vec<(SubscriptionKind, Contract)>>>,
    active_full_subs: Arc<Mutex<Vec<(SubscriptionKind, crate::tdbe::types::enums::SecType)>>>,
    dropped: Arc<AtomicU64>,
    panics: Arc<AtomicU64>,
    ring_cursors: Arc<RingCursors>,
    consumer_thread_id: Arc<OnceLock<ThreadId>>,
    slow_callback_threshold_ns: Arc<AtomicU64>,
    slow_callback_count: Arc<AtomicU64>,
    next_req_id: Arc<AtomicI64>,
}

// ---------------------------------------------------------------------------
// StreamingClient
// ---------------------------------------------------------------------------

/// Real-time streaming client for `ThetaData`'s FPSS servers.
///
/// # Lifecycle
///
/// 1. [`StreamingClient::builder`] -- configure (`ring_size`, `flush_mode`,
///    timeouts, reconnect policy), then `build()?` to TLS-connect,
///    authenticate, and start the background I/O thread
/// 2. `subscribe(...)` / `unsubscribe(...)` -- subscribe to market data
/// 3. Drain events on the caller's thread via [`Self::next_event`]
///    (blocking), [`Self::try_next_event`] (non-blocking),
///    [`Self::poll_batch`] / [`Self::for_each`] (callback adapters), or
///    the `Iterator` impl on `&StreamingClient`
/// 4. `shutdown()` -- clean disconnect
///
/// # Thread safety
///
/// `StreamingClient` is `Send + Sync`. The polymorphic `subscribe(spec)` /
/// `unsubscribe(spec)` methods send commands through a lock-free channel to
/// the I/O thread; they never touch the TLS stream directly.
pub struct StreamingClient {
    /// Channel to send write commands to the I/O thread.
    ///
    /// `std::sync::mpsc::SyncSender` is `Send + Sync`, but the `Mutex` is
    /// retained so the command-send path stays a single serialized critical
    /// section: it preserves command ordering under concurrent `&self`
    /// subscribe / unsubscribe calls and keeps the bounded `try_send`
    /// backpressure decision race-free.
    cmd_tx: Mutex<std_mpsc::SyncSender<IoCommand>>,
    /// Handle to the I/O thread (blocking TLS read + write drain).
    io_handle: Option<JoinHandle<()>>,
    /// Handle to the ping heartbeat thread.
    ping_handle: Option<JoinHandle<()>>,
    /// Ring poller drained by [`Self::next_event`],
    /// [`Self::poll_batch`], [`Self::for_each`], and the
    /// `Iterator for &StreamingClient` impl. Wrapped in a `Mutex` so the
    /// client is `Sync` and can be cloned into an embedding thread for
    /// control operations while another thread drives the polling loop;
    /// in practice only one thread polls so the lock is uncontended.
    /// Becomes `None` after a clean shutdown where the ring has been
    /// fully drained.
    poller_state: Mutex<Option<PollerState>>,
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
        Arc<Mutex<Vec<(SubscriptionKind, crate::tdbe::types::enums::SecType)>>>,
    /// The server address the initial connect landed on. Snapshot;
    /// see `last_connected_addr()` for the live session address.
    server_addr: String,
    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// frame of any kind (`0` = never). Written by the I/O thread,
    /// read by [`StreamingClient::millis_since_last_event`] /
    /// [`StreamingClient::last_event_received_at_unix_nanos`].
    last_event_at_ns: Arc<AtomicI64>,
    /// Address of the live session's server. Updated by the I/O
    /// thread after every successful reconnect.
    connected_addr: Arc<Mutex<String>>,
    /// Replay pacing snapshot for [`Self::restore_subscriptions`].
    /// Mirrors the builder's `replay_burst_size` / `replay_pace_ms`.
    replay_burst_size: u32,
    replay_pace_ms: u64,
    /// Cumulative count of publish failures: events the TLS reader could
    /// not enqueue because the consumer fell behind and the ring buffer
    /// was full. Snapshot via [`StreamingClient::dropped_count`]; this is the
    /// user-facing "ring-overflow" metric.
    dropped: Arc<AtomicU64>,
    /// Producer / consumer progress cursors for the event ring,
    /// sampled by [`StreamingClient::ring_occupancy`]. The I/O thread
    /// stores the published sequence on every successful publish; the
    /// drain paths store the consumed sequence once per drained
    /// batch. Cache-padded so the two write streams never share a
    /// line.
    ring_cursors: Arc<RingCursors>,
    /// Configured event-ring capacity in slots (validated power of
    /// two). Snapshot via [`StreamingClient::ring_capacity`] so operators
    /// can scale [`StreamingClient::ring_occupancy`] samples without
    /// re-reading their own configuration.
    ring_size: usize,
    /// Event-ring consumer wait strategy applied by the blocking poll
    /// loops ([`Self::next_event`] / [`Self::for_each_scoped`]) when the
    /// ring is momentarily empty. Resolved once at connect from the
    /// configured [`StreamingWaitStrategy`] preset + tuning so the
    /// consumer-side wait matches the ring builder's strategy.
    wait_strategy: ring::AdaptiveWaitStrategy,
    /// Optional CPU core to pin the consumer drain thread to; `None`
    /// (default) leaves it under the OS scheduler. Applied once at
    /// drain-loop entry in [`Self::for_each_scoped`] / [`Self::next_event`]
    /// via [`affinity::pin_consumer_thread`].
    consumer_cpu: Option<usize>,
    /// One-shot guard so [`Self::pin_consumer_once`] applies the CPU pin
    /// exactly once across repeated `next_event` / `for_each` drives,
    /// keeping the affinity syscall off the per-event path.
    consumer_pinned: std::sync::atomic::AtomicBool,
    /// Cumulative count of user-callback panics caught by the
    /// event-dispatch consumer's `catch_unwind` boundary. Snapshot via
    /// [`StreamingClient::panic_count`].
    panics: Arc<AtomicU64>,
    /// Captured `ThreadId` of a per-binding dispatcher / consumer
    /// thread that the binding wants the core's `Drop` self-join
    /// detector to skip. The harness in
    /// [`Self::for_self_join_test`] is the only path that actually
    /// initialises this cell — production bindings own their own
    /// dispatcher join handles and detect self-join at their level.
    /// Kept here so the offline self-join soak harness has a real
    /// fixture without paying for a separate type.
    consumer_thread_id: Arc<OnceLock<ThreadId>>,
    /// Quiescence barrier: flipped to `true` once the I/O thread and
    /// the event-dispatch consumer have both joined and the user callback
    /// is guaranteed to have stopped firing. Set inside [`Drop`] for both
    /// the inline-join path and the detached-helper path. Outer holders
    /// (e.g. [`crate::StreamSurface::stop_streaming`]) may capture an
    /// [`Arc::clone`] of this flag before releasing their last
    /// `Arc<StreamingClient>` so that
    /// [`crate::Client::await_drain`] can poll for full
    /// quiescence after stop / reconnect.
    drained: Arc<AtomicBool>,
    /// Slow-callback observability surface (Resilience).
    ///
    /// `slow_callback_threshold_ns` is read by the event-dispatch consumer
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

impl StreamingClient {
    /// Start a new [`StreamingClientBuilder`] with the two required arguments
    /// and SDK defaults for the rest. Optional setters chain.
    #[must_use]
    pub fn builder<'a>(
        creds: &'a Credentials,
        hosts: &'a [(String, u16)],
    ) -> StreamingClientBuilder<'a> {
        StreamingClientBuilder::new(creds, hosts)
    }

    /// Connect, authenticate, and start the background I/O and ping
    /// threads. Returns the assembled client; the ring poller is held
    /// internally and drained via [`Self::next_event`],
    /// [`Self::poll_batch`], [`Self::for_each`], or the `Iterator for
    /// &StreamingClient` impl.
    ///
    /// Crate-internal — public callers use [`Self::builder`].
    pub(crate) fn connect(args: FpssConnectArgs<'_>) -> Result<Self, Error> {
        let FpssConnectArgs {
            creds,
            hosts,
            ring_size,
            flush_mode,
            policy,
            wait_ms,
            wait_max_ms,
            wait_rate_limited_ms,
            wait_server_restart_ms,
            jitter,
            replay_burst_size,
            replay_pace_ms,
            derive_ohlcvc,
            connect_timeout_ms,
            read_timeout_ms,
            ping_interval_ms,
            io_read_slice_ms,
            data_watchdog_ms,
            keepalive_idle_secs,
            keepalive_interval_secs,
            keepalive_retries,
            host_selection,
            host_shuffle_seed,
            wait_strategy,
            consumer_cpu,
        } = args;
        let ring_size = ring::check_ring_size(ring_size)
            .map_err(|e| Error::config_invalid("fpss.ring_size", e.to_string()))?;
        let to_i64 = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        if !crate::config::streaming_bounds::TIMEOUT_MS.contains(&read_timeout_ms) {
            return Err(Error::config_out_of_range(
                "fpss.read_timeout_ms",
                to_i64(read_timeout_ms),
                to_i64(*crate::config::streaming_bounds::TIMEOUT_MS.start()),
                to_i64(*crate::config::streaming_bounds::TIMEOUT_MS.end()),
            ));
        }
        if !crate::config::streaming_bounds::CONNECT_TIMEOUT_MS.contains(&connect_timeout_ms) {
            return Err(Error::config_out_of_range(
                "fpss.connect_timeout_ms",
                to_i64(connect_timeout_ms),
                to_i64(*crate::config::streaming_bounds::CONNECT_TIMEOUT_MS.start()),
                to_i64(*crate::config::streaming_bounds::CONNECT_TIMEOUT_MS.end()),
            ));
        }
        if !crate::config::streaming_bounds::PING_INTERVAL_MS.contains(&ping_interval_ms) {
            return Err(Error::config_out_of_range(
                "fpss.ping_interval_ms",
                to_i64(ping_interval_ms),
                to_i64(*crate::config::streaming_bounds::PING_INTERVAL_MS.start()),
                to_i64(*crate::config::streaming_bounds::PING_INTERVAL_MS.end()),
            ));
        }
        if !crate::config::streaming_bounds::IO_READ_SLICE_MS.contains(&io_read_slice_ms) {
            return Err(Error::config_out_of_range(
                "fpss.io_read_slice_ms",
                to_i64(io_read_slice_ms),
                to_i64(*crate::config::streaming_bounds::IO_READ_SLICE_MS.start()),
                to_i64(*crate::config::streaming_bounds::IO_READ_SLICE_MS.end()),
            ));
        }
        if !crate::config::streaming_bounds::KEEPALIVE_IDLE_SECS.contains(&keepalive_idle_secs) {
            return Err(Error::config_out_of_range(
                "fpss.keepalive_idle_secs",
                to_i64(keepalive_idle_secs),
                to_i64(*crate::config::streaming_bounds::KEEPALIVE_IDLE_SECS.start()),
                to_i64(*crate::config::streaming_bounds::KEEPALIVE_IDLE_SECS.end()),
            ));
        }
        if !crate::config::streaming_bounds::KEEPALIVE_INTERVAL_SECS
            .contains(&keepalive_interval_secs)
        {
            return Err(Error::config_out_of_range(
                "fpss.keepalive_interval_secs",
                to_i64(keepalive_interval_secs),
                to_i64(*crate::config::streaming_bounds::KEEPALIVE_INTERVAL_SECS.start()),
                to_i64(*crate::config::streaming_bounds::KEEPALIVE_INTERVAL_SECS.end()),
            ));
        }
        if !crate::config::streaming_bounds::KEEPALIVE_RETRIES.contains(&keepalive_retries) {
            return Err(Error::config_out_of_range(
                "fpss.keepalive_retries",
                i64::from(keepalive_retries),
                i64::from(*crate::config::streaming_bounds::KEEPALIVE_RETRIES.start()),
                i64::from(*crate::config::streaming_bounds::KEEPALIVE_RETRIES.end()),
            ));
        }
        // Apply the host-selection policy once for the cold connect.
        // Reconnects reuse the same seed, optionally pinning the last
        // stable host ahead of a policy-ordered tail.
        let seed = host_shuffle_seed.unwrap_or_else(crate::backoff::entropy_u64);
        let ordered_hosts = connection::order_hosts(hosts, host_selection, seed, None);
        let keepalive = connection::TcpKeepaliveSpec {
            idle: Duration::from_secs(keepalive_idle_secs),
            interval: Duration::from_secs(keepalive_interval_secs),
            retries: keepalive_retries,
        };
        let borrowed: Vec<(&str, u16)> = ordered_hosts
            .iter()
            .map(|(h, p)| (h.as_str(), *p))
            .collect();
        let connect_timeout = Duration::from_millis(connect_timeout_ms);
        let read_timeout = Duration::from_millis(read_timeout_ms);
        let (stream, server_addr) =
            connection::connect_to_servers(&borrowed, connect_timeout, read_timeout, keepalive)?;
        Self::connect_with_stream(connection::ConnectWithStreamArgs {
            creds,
            stream,
            server_addr,
            hosts,
            host_selection,
            host_shuffle_seed: seed,
            ring_size,
            derive_ohlcvc,
            flush_mode,
            wait_strategy,
            consumer_cpu,
            policy,
            wait_ms,
            wait_max_ms,
            wait_rate_limited_ms,
            wait_server_restart_ms,
            jitter,
            replay_burst_size,
            replay_pace_ms,
            connect_timeout,
            read_timeout,
            io_read_slice: Duration::from_millis(io_read_slice_ms),
            data_watchdog: Duration::from_millis(data_watchdog_ms),
            keepalive,
            ping_interval: Duration::from_millis(ping_interval_ms),
        })
    }

    /// Connect using a pre-established stream (for testing with mock sockets).
    ///
    /// `hosts` is the declared FPSS server list, needed for auto-reconnect to
    /// re-apply the host-selection policy. Pass an empty slice to disable
    /// reconnection to other servers.
    ///
    /// Returns the connected client with its internal poller bundled
    /// in; drain via [`Self::next_event`] / [`Self::poll_batch`] /
    /// [`Self::for_each`] or the `Iterator` impl.
    pub(crate) fn connect_with_stream(
        args: connection::ConnectWithStreamArgs<'_>,
    ) -> Result<Self, Error> {
        let connection::ConnectWithStreamArgs {
            creds,
            mut stream,
            server_addr,
            hosts,
            host_selection,
            host_shuffle_seed,
            ring_size,
            derive_ohlcvc,
            flush_mode,
            wait_strategy,
            consumer_cpu,
            policy,
            wait_ms,
            wait_max_ms,
            wait_rate_limited_ms,
            wait_server_restart_ms,
            jitter,
            replay_burst_size,
            replay_pace_ms,
            connect_timeout,
            read_timeout,
            io_read_slice,
            data_watchdog,
            keepalive,
            ping_interval,
        } = args;
        // Send CREDENTIALS (code 0).
        let cred_payload = build_credentials_payload(&creds.email, &creds.password)?;
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
        let mut pending_control: Vec<StreamControl> = Vec::new();
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
                let is_auth_failure = matches!(
                    reason,
                    RemoveReason::InvalidCredentials
                        | RemoveReason::InvalidLoginValues
                        | RemoveReason::InvalidCredentialsNullUser
                );
                if is_auth_failure {
                    tracing::warn!(
                        "FPSS login failed. If your password contains special characters, \
                         try URL-encoding them."
                    );
                    return Err(Error::Auth {
                        kind: crate::error::AuthErrorKind::InvalidCredentials,
                        message: format!("FPSS server rejected login: {reason:?}"),
                    });
                }
                return Err(Error::Fpss {
                    kind: crate::error::FpssErrorKind::Disconnected,
                    message: format!("server rejected login: {reason:?}"),
                });
            }
        };

        // Set a shorter read timeout for the I/O loop so it can drain
        // commands between reads. The overall no-frames deadline is
        // enforced on a wall clock inside the I/O loop; this slice only
        // sets how often the loop wakes to service outbound commands.
        stream
            .sock
            .set_read_timeout(Some(io_read_slice))
            .map_err(|e| Error::Fpss {
                kind: crate::error::FpssErrorKind::ConnectionRefused,
                message: format!("failed to set read timeout: {e}"),
            })?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let authenticated = Arc::new(AtomicBool::new(true));
        let active_subs: Arc<Mutex<Vec<(protocol::SubscriptionKind, protocol::Contract)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let active_full_subs: Arc<
            Mutex<
                Vec<(
                    protocol::SubscriptionKind,
                    crate::tdbe::types::enums::SecType,
                )>,
            >,
        > = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let panics = Arc::new(AtomicU64::new(0));
        // Slow-callback observability — opt-in via
        // `set_slow_callback_threshold` after `connect`. `0` disables.
        let slow_callback_threshold_ns = Arc::new(AtomicU64::new(0));
        let slow_callback_count = Arc::new(AtomicU64::new(0));
        // Captured by the event-dispatch consumer closure on first dispatch
        // and read by `StreamingClient::drop` to break the self-join cycle
        // (callback -> stop_streaming -> drop StreamingClient -> join io
        // thread -> drop producer -> join consumer thread = self).
        let consumer_thread_id: Arc<OnceLock<ThreadId>> = Arc::new(OnceLock::new());

        // Shared `next_req_id` counter — the StreamingClient public API
        // owns one handle for caller-issued subscribes; the io_loop
        // borrows another so re-subscribe frames on auto-reconnect
        // allocate fresh ids correlatable through `ReqResponse`.
        let next_req_id: Arc<AtomicI64> = Arc::new(AtomicI64::new(1));

        // Staleness clock shared with the I/O thread: UNIX nanoseconds
        // of the most recent inbound frame (login handshake counts —
        // frames were just exchanged).
        let last_event_at_ns: Arc<AtomicI64> = Arc::new(AtomicI64::new(
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            )
            .unwrap_or(i64::MAX),
        ));

        // Live connected-address cell: seeded with the initial server,
        // updated by the I/O thread after every successful reconnect.
        let connected_addr: Arc<Mutex<String>> = Arc::new(Mutex::new(server_addr.clone()));

        // Command channel: StreamingClient -> I/O thread. Bounded so a
        // control-plane burst cannot grow the queue without limit; see
        // `CMD_CHANNEL_CAPACITY`.
        let (cmd_tx, cmd_rx) = std_mpsc::sync_channel::<IoCommand>(CMD_CHANNEL_CAPACITY);

        // Ping command channel: ping thread -> I/O thread
        let ping_cmd_tx = cmd_tx.clone();

        // Build the ring producer + poller. The producer is moved into
        // the I/O thread; the poller is bundled into the assembled
        // `StreamingClient` and drained via `next_event` / `poll_batch` /
        // `for_each` / the `Iterator for &StreamingClient` impl. The shared
        // cursor pair feeds `ring_occupancy()`: the producer records
        // every published sequence, the drain paths record batch
        // completions.
        let ring_cursors = Arc::new(RingCursors::new());
        let (producer, poller) =
            io_loop::build_poller_producer(ring_size, Arc::clone(&ring_cursors), wait_strategy);

        // Build the client with the producer + poller bundled in. The
        // poller lives inside `StreamingClient::poller_state` and is drained
        // via `next_event`, `poll_batch`, `for_each`, or the
        // `Iterator for &StreamingClient` impl.
        Self::spawn_io_and_assemble(SpawnArgs {
            producer,
            poller,
            stream,
            cmd_rx,
            cmd_tx,
            ping_cmd_tx,
            ring_size,
            permissions,
            pending_control,
            server_addr,
            creds,
            hosts,
            host_selection,
            host_shuffle_seed,
            derive_ohlcvc,
            flush_mode,
            wait_strategy,
            consumer_cpu,
            policy,
            wait_ms,
            wait_max_ms,
            wait_rate_limited_ms,
            wait_server_restart_ms,
            jitter,
            replay_burst_size,
            replay_pace_ms,
            connect_timeout,
            read_timeout,
            io_read_slice,
            data_watchdog,
            keepalive,
            last_event_at_ns,
            connected_addr,
            ping_interval,
            shutdown,
            authenticated,
            active_subs,
            active_full_subs,
            dropped,
            panics,
            ring_cursors,
            consumer_thread_id,
            slow_callback_threshold_ns,
            slow_callback_count,
            next_req_id,
        })
    }

    /// Spawn the I/O + ping threads for an already-built ring producer
    /// and assemble the [`StreamingClient`] handle.
    fn spawn_io_and_assemble<P>(args: SpawnArgs<'_, P>) -> Result<Self, Error>
    where
        P: ring::RingProducer,
    {
        let SpawnArgs {
            producer,
            poller,
            stream,
            cmd_rx,
            cmd_tx,
            ping_cmd_tx,
            ring_size,
            permissions,
            pending_control,
            server_addr,
            creds,
            hosts,
            host_selection,
            host_shuffle_seed,
            derive_ohlcvc,
            flush_mode,
            wait_strategy,
            consumer_cpu,
            policy,
            wait_ms,
            wait_max_ms,
            wait_rate_limited_ms,
            wait_server_restart_ms,
            jitter,
            replay_burst_size,
            replay_pace_ms,
            connect_timeout,
            read_timeout,
            io_read_slice,
            data_watchdog,
            keepalive,
            last_event_at_ns,
            connected_addr,
            ping_interval,
            shutdown,
            authenticated,
            active_subs,
            active_full_subs,
            dropped,
            panics,
            ring_cursors,
            consumer_thread_id,
            slow_callback_threshold_ns,
            slow_callback_count,
            next_req_id,
        } = args;

        // Replay knobs are `Copy`; snapshot for the client handle
        // before the originals move into the I/O thread args.
        let client_replay_burst_size = replay_burst_size;
        let client_replay_pace_ms = replay_pace_ms;

        // Spawn the I/O thread: blocking TLS read + ring publish + command drain.
        let io_shutdown = Arc::clone(&shutdown);
        let io_authenticated = Arc::clone(&authenticated);
        let io_creds = creds.clone();
        let io_hosts = hosts.to_vec();
        let io_active_subs = Arc::clone(&active_subs);
        let io_active_full_subs = Arc::clone(&active_full_subs);
        let io_dropped = Arc::clone(&dropped);
        let io_next_req_id = Arc::clone(&next_req_id);
        let io_last_event_at_ns = Arc::clone(&last_event_at_ns);
        let io_connected_addr = Arc::clone(&connected_addr);

        let io_handle = thread::Builder::new()
            .name("fpss-io".to_owned())
            .spawn(move || {
                io_loop(io_loop::IoLoopArgs {
                    stream,
                    cmd_rx,
                    producer,
                    ring_size,
                    shutdown: io_shutdown,
                    authenticated: io_authenticated,
                    permissions,
                    pending_control,
                    derive_ohlcvc,
                    flush_mode,
                    policy,
                    wait_ms,
                    wait_max_ms,
                    wait_rate_limited_ms,
                    wait_server_restart_ms,
                    jitter,
                    replay_burst_size,
                    replay_pace_ms,
                    creds: io_creds,
                    hosts: io_hosts,
                    host_selection,
                    host_shuffle_seed,
                    active_subs: io_active_subs,
                    active_full_subs: io_active_full_subs,
                    dropped: io_dropped,
                    connect_timeout,
                    read_timeout,
                    io_read_slice,
                    data_watchdog,
                    keepalive,
                    last_event_at_ns: io_last_event_at_ns,
                    connected_addr: io_connected_addr,
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

        Ok(StreamingClient {
            cmd_tx: Mutex::new(cmd_tx),
            io_handle: Some(io_handle),
            ping_handle: Some(ping_handle),
            poller_state: Mutex::new(Some(PollerState {
                poller,
                pending: VecDeque::new(),
                consumed_seq: -1,
            })),
            shutdown,
            authenticated,
            next_req_id: Arc::clone(&next_req_id),
            active_subs,
            active_full_subs,
            server_addr,
            last_event_at_ns,
            connected_addr,
            replay_burst_size: client_replay_burst_size,
            replay_pace_ms: client_replay_pace_ms,
            dropped,
            panics,
            ring_cursors,
            ring_size,
            wait_strategy,
            consumer_cpu,
            consumer_pinned: std::sync::atomic::AtomicBool::new(false),
            consumer_thread_id,
            drained: Arc::new(AtomicBool::new(false)),
            slow_callback_threshold_ns,
            slow_callback_count,
        })
    }

    /// Cumulative count of events the TLS reader could not publish into
    /// the event ring because the consumer fell behind and the ring
    /// was full.
    ///
    /// This is the user-facing "events dropped due to slow callback"
    /// metric. Operators should poll on a
    /// periodic timer (e.g. every second) and emit a `warn` log on any
    /// non-zero delta — a per-drop log would amplify under sustained
    /// overflow.
    #[must_use]
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Point-in-time count of events published into the event ring but
    /// not yet drained by the consumer — the in-flight depth between
    /// the I/O thread and your callback.
    ///
    /// This is the leading back-pressure signal:
    /// [`Self::dropped_count`] only moves AFTER data has been lost,
    /// while a rising occupancy that approaches
    /// [`Self::ring_capacity`] predicts those drops while there is
    /// still time to react (shed callback work, widen the ring, scale
    /// the consumer).
    ///
    /// Reading is a pair of `Relaxed` atomic loads on the calling
    /// thread — it never blocks the feed, takes no lock, and is safe
    /// to poll from any thread at any cadence. Because the two
    /// cursors are sampled independently, a read racing a concurrent
    /// drain is clamped at `0` rather than underflowing; treat the
    /// value as a monitoring sample, not an exact queue length.
    ///
    /// Consumed progress is recorded once per drained batch, so a
    /// sample taken mid-batch can briefly include events the consumer
    /// is already iterating. Conversely, the event-at-a-time APIs
    /// ([`Self::next_event`] / [`Self::try_next_event`]) stage whole
    /// drained batches internally, so events staged but not yet
    /// returned to the caller read as already consumed here. The value
    /// never exceeds [`Self::ring_capacity`].
    #[must_use]
    pub fn ring_occupancy(&self) -> usize {
        self.ring_cursors.occupancy()
    }

    /// Configured capacity of the event ring in slots (the builder's
    /// `ring_size`, a validated power of two).
    ///
    /// The fixed denominator for [`Self::ring_occupancy`]: when the
    /// occupancy sample approaches this value the ring is saturating
    /// and further publishes will be dropped (counted by
    /// [`Self::dropped_count`]).
    #[must_use]
    pub fn ring_capacity(&self) -> usize {
        self.ring_size
    }

    /// Cumulative count of user-callback faults: Rust panics caught by the
    /// per-invocation `catch_unwind` boundary, and Python exceptions raised
    /// inside the callback (surfaced via `PyErr::write_unraisable` on the
    /// language-binding layer). Both kinds are counted atomically here so
    /// callers observe a single unified fault counter regardless of whether
    /// the fault originated in Rust or Python. The consumer thread never
    /// dies from either kind of fault.
    #[must_use]
    pub fn panic_count(&self) -> u64 {
        self.panics.load(Ordering::Relaxed)
    }

    /// Increment the panic counter by one.
    ///
    /// Called by the Python binding's dispatcher when the user callback
    /// raises a `PyErr` that is not a Rust panic and therefore bypasses
    /// the `catch_unwind` boundary. Keeps `panic_count()` as the single
    /// unified fault counter for both Rust panics and Python exceptions.
    /// The TypeScript binding does not wire this entry point today — JS
    /// errors surface through Node's `uncaughtException`.
    #[cfg(feature = "__internal")]
    #[doc(hidden)]
    pub fn record_panic(&self) {
        self.panics.fetch_add(1, Ordering::Relaxed);
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
    /// the I/O thread and the event-dispatch consumer have both joined, so
    /// the user callback is guaranteed to have stopped firing.
    ///
    /// Returned as an `Arc<AtomicBool>` so a higher-level holder
    /// (e.g. [`crate::StreamSurface::stop_streaming`]) can capture a
    /// clone before releasing its last `Arc<StreamingClient>` and use it to
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

    /// Apply the configured consumer-core pin to the current thread at
    /// most once for the life of this client.
    ///
    /// The blocking drain paths ([`Self::next_event`] /
    /// [`Self::for_each_scoped`] / [`Self::for_each_with_wait_strategy`])
    /// call this at entry; the one-shot
    /// `AtomicBool` keeps the affinity syscall off the per-event path
    /// when `next_event` is driven one event at a time. A `None`
    /// `consumer_cpu` short-circuits to nothing.
    fn pin_consumer_once(&self) {
        if self.consumer_cpu.is_none() {
            return;
        }
        if self.consumer_pinned.swap(true, Ordering::Relaxed) {
            return;
        }
        affinity::pin_consumer_thread(self.consumer_cpu);
    }

    /// Block the calling thread until the next event is available or
    /// the ring shuts down.
    ///
    /// Returns:
    /// - `Ok(Some(event))` when an event is delivered (or was buffered
    ///   from the previous batch poll).
    /// - `Ok(None)` ONLY when the ring is fully drained AND the
    ///   producer has dropped (terminal shutdown). Iterator-style
    ///   consumers can treat this as the end of the stream.
    /// - `Err(_)` on a typed FPSS failure.
    ///
    /// On a momentarily empty live ring the call applies a three-phase
    /// wait (spin, yield, spin-loop hint) and re-polls rather than
    /// returning. A consumer that wants non-blocking semantics with
    /// explicit "empty right now" handling should use
    /// [`Self::try_next_event`] instead.
    ///
    /// # Errors
    ///
    /// Returns [`FpssError::DispatcherFailed`] if the internal staging
    /// queue's mutex was poisoned by a panicking caller on a previous
    /// invocation.
    pub fn next_event(&self) -> Result<Option<StreamEvent>, FpssError> {
        self.pin_consumer_once();
        let waiter = self.wait_strategy;
        loop {
            match self.try_next_event_internal()? {
                TryNext::Event(event) => return Ok(Some(event)),
                TryNext::Empty => waiter.wait_for(0),
                TryNext::Shutdown => return Ok(None),
            }
        }
    }

    /// Non-blocking single-event pull from the ring. Returns `Ok(None)`
    /// when the ring is momentarily empty OR terminally shut down — use
    /// [`Self::next_event`] when you need to distinguish the two.
    ///
    /// # Errors
    ///
    /// Returns [`FpssError::DispatcherFailed`] if the staging mutex was
    /// poisoned.
    pub fn try_next_event(&self) -> Result<Option<StreamEvent>, FpssError> {
        match self.try_next_event_internal()? {
            TryNext::Event(event) => Ok(Some(event)),
            TryNext::Empty | TryNext::Shutdown => Ok(None),
        }
    }

    fn try_next_event_internal(&self) -> Result<TryNext, FpssError> {
        let mut guard = self
            .poller_state
            .lock()
            .map_err(|e| FpssError::DispatcherFailed(format!("poller mutex poisoned: {e}")))?;
        let Some(mut state) = guard.take() else {
            return Ok(TryNext::Shutdown);
        };

        if let Some(event) = state.pending.pop_front() {
            *guard = Some(state);
            return Ok(TryNext::Event(event));
        }

        let outcome = match state.poller.poll() {
            Ok(mut batch) => {
                // Count every slot this drain releases, including
                // internal-only events that `as_public` filters out —
                // the ring frees every slot the batch guard covers. A
                // register increment inside the existing iteration; no
                // atomic, no extra pass.
                let mut batch_len: i64 = 0;
                for ring_event in &mut batch {
                    batch_len += 1;
                    if let Some(public) = ring_event.event.as_public() {
                        state.pending.push_back(public.clone());
                    }
                }
                // One store per drained batch (never per event): mirror
                // the local cursor into the shared occupancy sample.
                state.consumed_seq += batch_len;
                self.ring_cursors.record_consumed(state.consumed_seq);
                match state.pending.pop_front() {
                    Some(event) => TryNext::Event(event),
                    None => TryNext::Empty,
                }
            }
            Err(Polling::NoEvents) => TryNext::Empty,
            Err(Polling::Shutdown) => match state.pending.pop_front() {
                Some(event) => TryNext::Event(event),
                None => TryNext::Shutdown,
            },
        };

        match outcome {
            TryNext::Shutdown => {
                // Producer is gone and the queue is drained; release
                // the staging state so subsequent polls short-circuit.
                Ok(TryNext::Shutdown)
            }
            other => {
                *guard = Some(state);
                Ok(other)
            }
        }
    }

    /// Non-blocking single-batch drain through `on_event`.
    ///
    /// Returns a [`PollOutcome`] so a caller integrating the polling
    /// loop into its own scheduler can tell a drained batch (and how
    /// many events it carried) apart from terminal shutdown. Each
    /// `&StreamEvent` handed to `on_event` is a zero-copy borrow into the
    /// ring slot, valid only for that call.
    pub fn poll_batch(&self, mut on_event: impl FnMut(&StreamEvent)) -> PollOutcome {
        let Ok(mut guard) = self.poller_state.lock() else {
            return PollOutcome::Shutdown;
        };
        let Some(mut state) = guard.take() else {
            return PollOutcome::Shutdown;
        };

        // Drain anything buffered from a previous `next_event` call so
        // batch consumers see those events first.
        let mut delivered = 0usize;
        while let Some(event) = state.pending.pop_front() {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| on_event(&event))).is_err()
            {
                self.panics.fetch_add(1, Ordering::Relaxed);
            } else {
                delivered += 1;
            }
        }

        let outcome = match state.poller.poll() {
            Ok(mut batch) => {
                // Count every slot this drain releases, including
                // internal-only events that `as_public` filters out —
                // the ring frees every slot the batch guard covers. A
                // register increment inside the existing iteration; no
                // atomic, no extra pass.
                let mut batch_len: i64 = 0;
                for ring_event in &mut batch {
                    batch_len += 1;
                    if let Some(event) = ring_event.event.as_public() {
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            on_event(event);
                        }))
                        .is_err()
                        {
                            let prev = self.panics.fetch_add(1, Ordering::Relaxed);
                            let count = prev + 1;
                            if count == 1 || count.is_multiple_of(1024) {
                                tracing::warn!(
                                    target: "thetadatadx::fpss::poller",
                                    panic_count = count,
                                    "user poller handler panicked; event skipped, drain continuing"
                                );
                            }
                        } else {
                            delivered += 1;
                        }
                    }
                }
                // One store per drained batch (never per event): mirror
                // the local cursor into the shared occupancy sample.
                state.consumed_seq += batch_len;
                self.ring_cursors.record_consumed(state.consumed_seq);
                Some(PollOutcome::Drained(delivered))
            }
            Err(Polling::NoEvents) => Some(PollOutcome::Drained(delivered)),
            Err(Polling::Shutdown) => {
                if delivered == 0 {
                    None
                } else {
                    Some(PollOutcome::Drained(delivered))
                }
            }
        };

        match outcome {
            Some(o) => {
                *guard = Some(state);
                o
            }
            None => PollOutcome::Shutdown,
        }
    }

    /// Block the calling thread, draining events through `on_event`
    /// until the ring shuts down.
    ///
    /// Each available batch is drained in order; on a momentarily empty
    /// ring the loop applies a three-phase wait (spin, then `yield_now`,
    /// then a `spin_loop` hint). Keeps an active stream low-latency and
    /// yields to other runnable threads on a quiet stream, but does
    /// NOT park — the loop stays runnable rather than idling at zero
    /// CPU. A consumer that wants to release the core while idle should
    /// drive [`Self::poll_batch`] behind its own parking strategy.
    ///
    /// Returns once [`Self::shutdown`] (or dropping the [`StreamingClient`])
    /// has fired AND every event already published into the ring has
    /// been delivered.
    pub fn for_each(&self, on_event: impl FnMut(&StreamEvent)) {
        // Identity batch scope: each batch drain runs directly, with no
        // wrapping. The inter-batch wait is the same three-phase strategy
        // `for_each_scoped` applies.
        self.for_each_scoped(on_event, |drain| drain());
    }

    /// Block the calling thread draining events through `on_event`, with
    /// each batch drain wrapped in a caller-supplied `scope`.
    ///
    /// Identical to [`Self::for_each`] except the work of draining one
    /// batch — every per-event `on_event` call for the events available
    /// at that instant — is executed inside `scope`, while the inter-batch
    /// wait on a momentarily empty ring runs OUTSIDE it. This lets a
    /// consumer acquire a resource once per batch rather than once per
    /// event without changing the one-call-per-event delivery contract:
    /// `on_event` still fires exactly once per event.
    ///
    /// The canonical use is a language binding that must hold an
    /// interpreter lock (e.g. the CPython GIL) to call into user code.
    /// Wrapping each batch drain in the lock amortises its acquisition
    /// across every event in the batch; keeping the wait outside the
    /// scope means the lock is released whenever the ring is idle, so a
    /// blocking wait never holds it.
    ///
    /// `scope` receives a `FnMut() -> PollOutcome` that drains one batch
    /// and returns its outcome; `scope` must call it exactly once and
    /// return its result. The loop terminates on
    /// [`PollOutcome::Shutdown`], identical to [`Self::for_each`].
    pub fn for_each_scoped<S>(&self, mut on_event: impl FnMut(&StreamEvent), mut scope: S)
    where
        S: FnMut(&mut dyn FnMut() -> PollOutcome) -> PollOutcome,
    {
        self.pin_consumer_once();
        let waiter = self.wait_strategy;
        loop {
            // Drain one batch inside the caller's scope. `on_event` fires
            // once per event, exactly as in `for_each`; the scope only
            // brackets the batch, it does not change delivery cardinality.
            let outcome = scope(&mut || self.poll_batch(&mut on_event));
            match outcome {
                PollOutcome::Shutdown => return,
                // Empty ring — wait OUTSIDE the scope so a held resource
                // (e.g. the GIL) is released across the idle wait.
                PollOutcome::Drained(0) => {
                    waiter.wait_for(0);
                }
                PollOutcome::Drained(_) => {}
            }
        }
    }

    /// Drain events through `on_event`, applying a caller-supplied
    /// [`disruptor::wait_strategies::WaitStrategy`] on each momentarily
    /// empty ring instead of the configured
    /// [`crate::StreamingWaitStrategy`] preset.
    ///
    /// This is the Rust-native bring-your-own-strategy escape hatch: the
    /// preset enum plus the `wait_spin_iters` / `wait_yield_iters` /
    /// `wait_park_us` knobs cover the common latency-vs-CPU points across
    /// every binding, but a Rust caller with an exotic backoff (e.g. an
    /// adaptive PID-controlled park, or a strategy that coordinates with
    /// another subsystem) can supply any `W: WaitStrategy` here.
    ///
    /// `W` is monomorphised into the loop, so the per-poll cost is the
    /// caller's `wait_for` body with no indirection — identical
    /// codegen to the preset path. Delivery semantics match
    /// [`Self::for_each`]: `on_event` fires exactly once per event and
    /// the loop returns on terminal shutdown after the ring drains.
    ///
    /// # Why Rust-only
    ///
    /// `wait_for` fires on every ring-empty poll on the hot path. Routing
    /// that per-poll callback across the C ABI, the CPython interpreter
    /// lock, or the JavaScript event loop would add call-boundary
    /// overhead to the single tightest loop in the consumer — a latency
    /// regression, not a tuning knob. The FFI-safe form of this override
    /// is the [`crate::StreamingWaitStrategy`] preset enum plus the
    /// numeric spin / yield / park tuning, which every binding exposes.
    pub fn for_each_with_wait_strategy<W, F>(&self, mut on_event: F, strategy: W)
    where
        W: disruptor::wait_strategies::WaitStrategy,
        F: FnMut(&StreamEvent),
    {
        self.pin_consumer_once();
        loop {
            match self.poll_batch(&mut on_event) {
                PollOutcome::Shutdown => return,
                PollOutcome::Drained(0) => strategy.wait_for(0),
                PollOutcome::Drained(_) => {}
            }
        }
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
        sec_type: crate::tdbe::types::enums::SecType,
        unsubscribe: bool,
    ) -> Result<(), Error> {
        self.check_connected()?;
        // Reject security types with no upstream full-stream broadcast before
        // allocating a req_id, emitting a frame, or tracking the subscription
        // for reconnect replay. Stock and Option are the only security types
        // with a full-stream broadcast; an index or rate full-stream subscribe
        // is accepted on the wire and answered `Subscribed`, then never streams
        // a tick — so it is rejected here at the subscribe boundary instead.
        if !full_stream_sec_type_supported(sec_type) {
            return Err(Error::Config {
                kind: crate::error::ConfigErrorKind::InvalidValue {
                    field: "Subscription::full".to_string(),
                    message: format!(
                        "full-stream subscriptions are supported only for Stock and Option; \
                         {sec_type:?} has no full broadcast upstream — subscribe per-contract \
                         instead (for example Contract::index(\"VIX\").trade())"
                    ),
                },
                message: "unsupported full-stream security type".to_string(),
                source: None,
            });
        }
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
        } else if !subs
            .iter()
            .any(|(k, s)| *k == kind_for_track && *s == sec_type)
        {
            // Idempotent by `(kind, sec_type)`: a repeated full-stream subscribe
            // must not accumulate duplicate tracked entries that would replay
            // the same subscribe frame multiple times on reconnect.
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
            // Idempotent by `(kind, contract)`: a repeated per-contract
            // subscribe must not accumulate duplicate tracked entries that
            // would replay the same subscribe frame multiple times on
            // reconnect.
            if !subs.iter().any(|(k, c)| *k == kind && c == contract) {
                subs.push((kind, contract.clone()));
            }
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

    /// Get the server address the initial connect landed on.
    ///
    /// Snapshot from connect time; auto-reconnect may move the session
    /// to a different host. [`Self::last_connected_addr`] tracks the
    /// live session.
    pub fn server_addr(&self) -> &str {
        &self.server_addr
    }

    /// Address (`host:port`) of the server the current session is
    /// connected to. Unlike [`Self::server_addr`], this follows the
    /// session across auto-reconnects, so operators can observe which
    /// host is actually serving the stream.
    #[must_use]
    pub fn last_connected_addr(&self) -> String {
        self.connected_addr
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// UNIX-nanosecond receive timestamp of the most recent inbound
    /// frame of any kind (data tick, heartbeat, control). `0` means no
    /// frame has been received yet.
    ///
    /// This is the raw feed for [`Self::millis_since_last_event`];
    /// exposed so callers correlating against their own wall-clock
    /// pipeline timestamps do not lose precision.
    #[must_use]
    pub fn last_event_received_at_unix_nanos(&self) -> i64 {
        self.last_event_at_ns.load(Ordering::Relaxed)
    }

    /// Milliseconds since the most recent inbound frame of any kind,
    /// or `None` when no frame has been received yet.
    ///
    /// The operator-facing staleness clock: a healthy session stays in
    /// the low hundreds of milliseconds (the server heartbeats every
    /// ~100 ms even when no market data flows), so a steadily growing
    /// value is the earliest external signal of a dead or wedged
    /// connection. Sampled from a wall clock; values can be perturbed
    /// by host clock adjustments.
    #[must_use]
    pub fn millis_since_last_event(&self) -> Option<u64> {
        let at = self.last_event_at_ns.load(Ordering::Relaxed);
        if at <= 0 {
            return None;
        }
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        )
        .unwrap_or(i64::MAX);
        Some(u64::try_from((now - at).max(0)).unwrap_or(u64::MAX) / 1_000_000)
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
    ) -> Vec<(SubscriptionKind, crate::tdbe::types::enums::SecType)> {
        self.active_full_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Re-subscribe a saved subscription snapshot onto this session,
    /// paced per the builder's replay knobs
    /// ([`StreamingClientBuilder::reconnect_replay_burst_size`] /
    /// [`StreamingClientBuilder::reconnect_replay_pace_ms`]).
    ///
    /// The single replay engine for caller-driven reconnect flows
    /// (including the embedded bindings): subscriptions are submitted
    /// in bursts with a jittered pause between bursts so a large saved
    /// set is spread over wall-clock time instead of being fired at a
    /// recovering upstream back-to-back. Capture the snapshot from
    /// [`Self::active_subscriptions`] /
    /// [`Self::active_full_subscriptions`] before tearing the previous
    /// session down.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PartialReconnect`] carrying the structured
    /// list of subscriptions that failed to restore; everything not in
    /// the list was re-installed.
    pub fn restore_subscriptions(
        &self,
        per_contract: &[(SubscriptionKind, Contract)],
        full_type: &[(SubscriptionKind, crate::tdbe::types::enums::SecType)],
    ) -> Result<(), Error> {
        let pacing = crate::client::ReplayPacing {
            burst_size: self.replay_burst_size,
            pace_ms: self.replay_pace_ms,
        };
        let failed = crate::client::restore_subscriptions(
            per_contract,
            full_type,
            pacing,
            |kind, contract| {
                self.subscribe(protocol::Subscription::Contract {
                    contract: contract.clone(),
                    kind,
                })
            },
            |kind, sec_type| match kind {
                SubscriptionKind::Trade => Some(self.subscribe(protocol::Subscription::Full {
                    sec_type,
                    kind: protocol::FullSubscriptionKind::Trades,
                })),
                SubscriptionKind::OpenInterest => {
                    Some(self.subscribe(protocol::Subscription::Full {
                        sec_type,
                        kind: protocol::FullSubscriptionKind::OpenInterest,
                    }))
                }
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

    /// Test-only constructor that wires up the same event ring +
    /// I/O-thread topology as [`Self::connect_with_stream`] **without**
    /// touching the network. It exists to drive the `Drop` self-join
    /// guard against the real `StreamingClient` instance and the real
    /// `consumer_thread_id` plumbing, not a mock of either.
    ///
    /// Topology:
    /// - The user `handler` runs on the event-dispatch consumer thread,
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
    /// `n_burst_events` synthetic `StreamEvent::Control(MarketOpen)`
    /// frames are pushed via [`HarnessPublishMode`].
    ///
    /// The optional `start_signal` lets the test defer the io thread's
    /// burst until after it has finished any setup that must happen
    /// before the consumer can dispatch. When `Some`, the io thread
    /// busy-waits on the flag flipping to `true` before publishing the
    /// burst. When `None`, the burst runs as soon as the io thread
    /// scheduler is given a chance to start.
    #[cfg(any(test, feature = "__test-helpers"))]
    #[doc(hidden)]
    pub fn for_self_join_test<F>(
        n_burst_events: usize,
        ring_size: usize,
        mode: HarnessPublishMode,
        start_signal: Option<Arc<AtomicBool>>,
        handler: F,
    ) -> Arc<Self>
    where
        F: FnMut(&StreamEvent) + Send + 'static,
    {
        use disruptor::{build_single_producer, BusySpin, Sequence}; // VOCAB-OK: internal crate name

        use self::events::FpssEventInternal;
        use self::ring::RingEvent;
        use self::ring::{RingProducer, SequencedProducer};

        let ring_size = ring::check_ring_size(ring_size).expect(
            "for_self_join_test: ring_size must be validated; tests must pass a power of two \
             >= MIN_RING_SIZE (e.g. 64, 128, 256)",
        );

        let shutdown = Arc::new(AtomicBool::new(false));
        let authenticated = Arc::new(AtomicBool::new(true));
        let active_subs: Arc<Mutex<Vec<(SubscriptionKind, Contract)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let active_full_subs: Arc<
            Mutex<Vec<(SubscriptionKind, crate::tdbe::types::enums::SecType)>>,
        > = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let panics = Arc::new(AtomicU64::new(0));
        let ring_cursors = Arc::new(RingCursors::new());
        let consumer_thread_id: Arc<OnceLock<ThreadId>> = Arc::new(OnceLock::new());
        let next_req_id: Arc<AtomicI64> = Arc::new(AtomicI64::new(1));

        let (cmd_tx, cmd_rx) = std_mpsc::sync_channel::<IoCommand>(CMD_CHANNEL_CAPACITY);

        let handler_cell = Mutex::new(handler);
        let panics_consumer = Arc::clone(&panics);
        let consumer_thread_id_cell = Arc::clone(&consumer_thread_id);
        let consumer_cursors = Arc::clone(&ring_cursors);

        let factory = RingEvent::default;
        let mut producer = SequencedProducer::new(
            build_single_producer(ring_size, factory, BusySpin)
                .handle_events_with(move |slot: &RingEvent, seq: Sequence, eob: bool| {
                    consumer_thread_id_cell.get_or_init(|| thread::current().id());
                    if let Some(evt) = slot.event.as_public() {
                        let mut h = handler_cell
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h(evt)))
                            .is_err()
                        {
                            panics_consumer.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    // Batch-granularity consumed cursor, mirroring the
                    // production drain: one store on the last event of
                    // each dispatched batch.
                    if eob {
                        consumer_cursors.record_consumed(seq);
                    }
                })
                .build(),
            Arc::clone(&ring_cursors),
        );

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
        // stash its `Arc<StreamingClient>` reference into the callback's
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
                // Keep the command receiver alive for the client's lifetime so
                // `send_cmd` observes a live (bounded) channel rather than an
                // immediate hang-up. The harness does not act on queued
                // commands; they accumulate up to `CMD_CHANNEL_CAPACITY`, which
                // is exactly the backpressure surface under test.
                let _cmd_rx = cmd_rx;
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
                                slot.event = FpssEventInternal::Control(StreamControl::MarketOpen);
                            });
                        }
                    }
                    HarnessPublishMode::TryPublishBurst => {
                        for _ in 0..io_burst {
                            if producer
                                .try_publish(|slot| {
                                    slot.event =
                                        FpssEventInternal::Control(StreamControl::MarketOpen);
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

        Arc::new(StreamingClient {
            cmd_tx: Mutex::new(cmd_tx),
            io_handle: Some(io_handle),
            ping_handle: None,
            poller_state: Mutex::new(None),
            shutdown,
            authenticated,
            next_req_id: Arc::clone(&next_req_id),
            active_subs,
            active_full_subs,
            server_addr: "test://self-join".to_owned(),
            last_event_at_ns: Arc::new(AtomicI64::new(0)),
            connected_addr: Arc::new(Mutex::new("test://self-join".to_owned())),
            replay_burst_size: 50,
            replay_pace_ms: 0,
            dropped,
            panics,
            ring_cursors,
            ring_size,
            wait_strategy: ring::AdaptiveWaitStrategy::low_latency(),
            consumer_cpu: None,
            consumer_pinned: std::sync::atomic::AtomicBool::new(false),
            consumer_thread_id,
            drained: Arc::new(AtomicBool::new(false)),
            slow_callback_threshold_ns: Arc::new(AtomicU64::new(0)),
            slow_callback_count: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Test-only constructor wiring the production polling topology —
    /// [`io_loop::build_poller_producer`] plus a live [`PollerState`]
    /// — **without** touching the network or spawning any thread.
    ///
    /// Returns the assembled client together with the ring producer so
    /// a test drives publishes and `poll_batch` / `next_event` drains
    /// deterministically from a single thread. This is the fixture for
    /// the ring-occupancy contract: the producer records published
    /// sequences and the drain paths record consumed batches into the
    /// same shared cursors the public [`Self::ring_occupancy`] reads.
    ///
    /// [`Self::for_self_join_test`] cannot serve here: it owns a
    /// dispatcher-thread consumer, so the moment between "published"
    /// and "drained" is not observable from the test thread.
    #[cfg(test)]
    pub(in crate::fpss) fn for_ring_occupancy_test(
        ring_size: usize,
    ) -> (Self, impl ring::RingProducer) {
        let ring_size = ring::check_ring_size(ring_size).expect(
            "for_ring_occupancy_test: ring_size must be validated; tests must pass a power of \
             two >= MIN_RING_SIZE (e.g. 64, 128, 256)",
        );

        let ring_cursors = Arc::new(RingCursors::new());
        let (producer, poller) = io_loop::build_poller_producer(
            ring_size,
            Arc::clone(&ring_cursors),
            ring::AdaptiveWaitStrategy::low_latency(),
        );

        let (cmd_tx, _cmd_rx) = std_mpsc::sync_channel::<IoCommand>(CMD_CHANNEL_CAPACITY);

        let client = StreamingClient {
            cmd_tx: Mutex::new(cmd_tx),
            io_handle: None,
            ping_handle: None,
            poller_state: Mutex::new(Some(PollerState {
                poller,
                pending: VecDeque::new(),
                consumed_seq: -1,
            })),
            shutdown: Arc::new(AtomicBool::new(false)),
            authenticated: Arc::new(AtomicBool::new(true)),
            next_req_id: Arc::new(AtomicI64::new(1)),
            active_subs: Arc::new(Mutex::new(Vec::new())),
            active_full_subs: Arc::new(Mutex::new(Vec::new())),
            server_addr: "test://ring-occupancy".to_owned(),
            last_event_at_ns: Arc::new(AtomicI64::new(0)),
            connected_addr: Arc::new(Mutex::new("test://ring-occupancy".to_owned())),
            replay_burst_size: 50,
            replay_pace_ms: 0,
            dropped: Arc::new(AtomicU64::new(0)),
            panics: Arc::new(AtomicU64::new(0)),
            ring_cursors,
            ring_size,
            wait_strategy: ring::AdaptiveWaitStrategy::low_latency(),
            consumer_cpu: None,
            consumer_pinned: std::sync::atomic::AtomicBool::new(false),
            consumer_thread_id: Arc::new(OnceLock::new()),
            drained: Arc::new(AtomicBool::new(false)),
            slow_callback_threshold_ns: Arc::new(AtomicU64::new(0)),
            slow_callback_count: Arc::new(AtomicU64::new(0)),
        };
        (client, producer)
    }

    /// Send a command to the I/O thread over the bounded control channel.
    ///
    /// Uses a non-blocking `try_send` so a public `&self` caller is never
    /// parked behind a saturated channel while holding the command lock. A
    /// full channel and a hung-up I/O thread both map to a typed
    /// [`FpssErrorKind::Disconnected`] error: the command is reported to the
    /// caller, never silently dropped. A full channel means the application is
    /// issuing control-plane commands faster than the I/O thread can drain
    /// them; the caller can retry after the queue clears.
    pub(in crate::fpss) fn send_cmd(&self, cmd: IoCommand) -> Result<(), Error> {
        self.cmd_tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_send(cmd)
            .map_err(|e| match e {
                std_mpsc::TrySendError::Full(_) => Error::Fpss {
                    kind: crate::error::FpssErrorKind::Disconnected,
                    message: format!(
                        "command queue full ({CMD_CHANNEL_CAPACITY} pending); \
                         the I/O thread is draining slower than commands arrive — retry shortly"
                    ),
                },
                std_mpsc::TrySendError::Disconnected(_) => Error::Fpss {
                    kind: crate::error::FpssErrorKind::Disconnected,
                    message: "I/O thread has exited".to_string(),
                },
            })
    }
}

/// Outcome of a single non-blocking [`StreamingClient::poll_batch`] drain.
///
/// Lets a caller integrating the ring drive into its own loop tell
/// "drained `n` events, more may come" apart from "the session has
/// terminated and the ring is empty":
///
/// * [`PollOutcome::Drained`] — the currently-available batch was
///   handed to the closure; the wrapped count is how many events were
///   delivered this call (`0` when the ring was momentarily empty but
///   the session is still live — re-poll later).
/// * [`PollOutcome::Shutdown`] — the [`StreamingClient`] has shut down AND
///   every published event has been drained. No further events will
///   ever arrive; stop polling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PollOutcome {
    /// The available batch was drained into the closure. Carries the
    /// number of events delivered on this call (may be `0`).
    Drained(usize),
    /// Terminal: the session has shut down and the ring is fully
    /// drained. No further events will arrive.
    Shutdown,
}

/// Owning iterator for an [`StreamingClient`] reference.
///
/// Yields one [`StreamEvent`] per call to [`Iterator::next`] by repeatedly
/// invoking [`StreamingClient::next_event`]; surfaces typed errors as
/// `Some(Err(_))` and terminates with `None` on clean shutdown.
impl Iterator for &StreamingClient {
    type Item = Result<StreamEvent, FpssError>;

    fn next(&mut self) -> Option<Self::Item> {
        match StreamingClient::next_event(self) {
            Ok(Some(event)) => Some(Ok(event)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

impl Drop for StreamingClient {
    fn drop(&mut self) {
        // Signal shutdown if not already done.
        self.shutdown.store(true, Ordering::Release);
        // Send shutdown command so I/O thread exits its loop.
        let _ = self.send_cmd(IoCommand::Shutdown);

        // Self-join guard.
        //
        // The exit path of the I/O thread drops the ring producer
        // (`drop(producer)` at the end of `io_loop`), which stores the
        // shutdown sequence on the ring. The caller's consumer loop —
        // typically a binding-owned dispatcher thread running
        // `next_event` / `for_each` — observes that and exits.
        //
        // If `Drop` is running on the I/O thread itself, or on the
        // binding's dispatcher thread that the user callback called
        // back into, joining the I/O handle inline would block the
        // very thread cleanup needs to complete on — a self-join
        // deadlock. The dispatcher-thread case is load-bearing: a
        // user callback that calls
        // `Client::stop_streaming()` swaps the live slot to
        // `Stopped` and drops the last `Arc<StreamingClient>` while running
        // on the consumer thread.
        //
        // Detach the join onto a helper thread in those cases. Cleanup
        // still completes; observers see `is_streaming()` flip to
        // `false` once the helper finishes, instead of `Drop` blocking
        // forever.
        let cur = thread::current().id();
        let consumer_id = self.consumer_thread_id.get().copied();

        // Drop the poller state proactively so any caller still holding
        // a borrow observes the ring shutdown immediately rather than
        // racing the producer drop.
        if let Ok(mut guard) = self.poller_state.lock() {
            *guard = None;
        }

        let ping_handle = self.ping_handle.take();
        let io_handle = self.io_handle.take();
        let io_handle_thread_id = io_handle.as_ref().map(|h| h.thread().id());
        let self_join = io_handle_thread_id == Some(cur) || consumer_id == Some(cur);

        if self_join {
            // Detach onto a fresh thread so the consumer / I/O thread is
            // not blocked waiting on its own termination. The detached
            // helper flips `drained` once both joins return so callers
            // polling `await_drain` see exact quiescence.
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
mod builder_tests {
    use super::*;
    use crate::config::DirectConfig;

    /// Default builder seeds every timing knob from the production
    /// sub-config defaults so a bare `StreamingClient::builder(..)` behaves
    /// identically to a `DirectConfig::production()` connect.
    /// Regression guard against the fields silently going to `0` or
    /// drifting from the config crate.
    #[test]
    fn builder_seeds_timing_defaults_from_production_config() {
        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("nj-a.thetadata.us".to_owned(), 20000)];
        let args = StreamingClientBuilder::new(&creds, &hosts).into_args();
        let fpss = crate::config::StreamingConfig::production_defaults();
        let reconnect = crate::config::ReconnectConfig::production_defaults();
        assert_eq!(args.connect_timeout_ms, fpss.connect_timeout_ms);
        assert_eq!(args.read_timeout_ms, fpss.timeout_ms);
        assert_eq!(args.ping_interval_ms, fpss.ping_interval_ms);
        assert_eq!(args.io_read_slice_ms, fpss.io_read_slice_ms);
        assert_eq!(args.data_watchdog_ms, fpss.data_watchdog_ms);
        assert_eq!(args.keepalive_idle_secs, fpss.keepalive_idle_secs);
        assert_eq!(args.keepalive_interval_secs, fpss.keepalive_interval_secs);
        assert_eq!(args.keepalive_retries, fpss.keepalive_retries);
        assert_eq!(args.host_selection, fpss.host_selection);
        assert_eq!(args.host_shuffle_seed, fpss.host_shuffle_seed);
        assert_eq!(args.wait_ms, reconnect.wait_ms);
        assert_eq!(args.wait_max_ms, reconnect.wait_max_ms);
        assert_eq!(args.wait_rate_limited_ms, reconnect.wait_rate_limited_ms);
        assert_eq!(
            args.wait_server_restart_ms,
            reconnect.wait_server_restart_ms
        );
        assert_eq!(args.jitter, reconnect.jitter);
        assert_eq!(args.replay_burst_size, reconnect.replay_burst_size);
        assert_eq!(args.replay_pace_ms, reconnect.replay_pace_ms);
    }

    /// The direct builder default ring size must match the production
    /// streaming config default so a direct-builder user gets the same
    /// overflow headroom as a config-driven one. This guards against the
    /// two defaults drifting apart again (the builder previously hardcoded a
    /// much smaller ring that overflowed under real market bursts).
    #[test]
    fn builder_ring_size_default_matches_production_config() {
        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("nj-a.thetadata.us".to_owned(), 20000)];
        let args = StreamingClientBuilder::new(&creds, &hosts).into_args();
        let fpss = crate::config::StreamingConfig::production_defaults();
        assert_eq!(
            args.ring_size, fpss.ring_size,
            "builder default ring size must track production config default"
        );
    }

    /// The wait-strategy preset selected on the builder reaches the
    /// connect args, and the default is the low-latency strategy that
    /// preserves the historical fixed behaviour.
    #[test]
    fn builder_threads_wait_strategy_preset() {
        use crate::config::StreamingWaitStrategy;
        use disruptor::wait_strategies::WaitStrategy;
        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("nj-a.thetadata.us".to_owned(), 20000)];

        // Default preset is the low-latency strategy that never sleeps; a
        // selected Balanced preset with a measurable park actually parks.
        // Compare the two RELATIVELY: an absolute upper bound on the
        // never-sleeping path flakes when a loaded host stretches the
        // yield phase, whereas a real `thread::sleep` has a reliable lower
        // bound and is always slower than the spin-only path.
        let default_args = StreamingClientBuilder::new(&creds, &hosts).into_args();
        let t0 = std::time::Instant::now();
        default_args.wait_strategy.wait_for(0);
        let low_latency_elapsed = t0.elapsed();

        let balanced_args = StreamingClientBuilder::new(&creds, &hosts)
            .wait_strategy(StreamingWaitStrategy::Balanced)
            .wait_strategy_tuning(0, 0, 2_000)
            .into_args();
        let t1 = std::time::Instant::now();
        balanced_args.wait_strategy.wait_for(0);
        let balanced_elapsed = t1.elapsed();

        // Balanced parks ~its configured 2 ms (a sleep never returns much
        // early — reliable lower bound).
        assert!(
            balanced_elapsed >= std::time::Duration::from_micros(1_800),
            "Balanced should park ~2ms, took {balanced_elapsed:?}"
        );
        // The never-sleeping low-latency path is faster than the 2 ms park —
        // a relative bound that holds even on a contended CI host.
        assert!(
            low_latency_elapsed < balanced_elapsed,
            "LowLatency ({low_latency_elapsed:?}) should be faster than Balanced ({balanced_elapsed:?})"
        );
    }

    /// Selecting any preset wires into a built ring without changing the
    /// poller type: `build_poller_producer` returns the same
    /// `EventPoller<RingEvent, SingleProducerBarrier>` regardless of the
    /// strategy preset passed in.
    #[test]
    fn wait_strategy_preset_does_not_change_poller_type() {
        use crate::fpss::ring::{AdaptiveWaitStrategy, RingCursors};
        use std::sync::Arc;

        // Each call returns the identical poller type; if any preset
        // leaked `W` into the return type this would not compile because
        // the two bindings could not share a `let` type below.
        let (_p1, poller1) = io_loop::build_poller_producer(
            64,
            Arc::new(RingCursors::new()),
            AdaptiveWaitStrategy::busy_spin(),
        );
        let (_p2, poller2) = io_loop::build_poller_producer(
            64,
            Arc::new(RingCursors::new()),
            AdaptiveWaitStrategy::balanced(),
        );
        // Same concrete type — assign through a shared binding type.
        let pollers: [EventPoller<ring::RingEvent, SingleProducerBarrier>; 2] = [poller1, poller2];
        assert_eq!(pollers.len(), 2);
    }

    /// `build()` rejects a `read_timeout_ms` outside the validated
    /// range. Second-line defence for callers that bypass
    /// `DirectConfig::validate`.
    #[test]
    fn build_rejects_out_of_range_read_timeout_ms() {
        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("127.0.0.1".to_owned(), 1)];
        let res = StreamingClientBuilder::new(&creds, &hosts)
            .read_timeout_ms(50) // below 100 ms minimum
            .build();
        let err = match res {
            Ok(_) => panic!("must reject"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(msg.contains("read_timeout_ms"), "{msg}");
    }

    /// Same defence for `connect_timeout_ms`.
    #[test]
    fn build_rejects_out_of_range_connect_timeout_ms() {
        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("127.0.0.1".to_owned(), 1)];
        let res = StreamingClientBuilder::new(&creds, &hosts)
            .connect_timeout_ms(50) // below 1 s minimum
            .build();
        let err = match res {
            Ok(_) => panic!("must reject"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("connect_timeout_ms"));
    }

    /// Same defence for `ping_interval_ms`.
    #[test]
    fn build_rejects_out_of_range_ping_interval_ms() {
        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("127.0.0.1".to_owned(), 1)];
        let res = StreamingClientBuilder::new(&creds, &hosts)
            .ping_interval_ms(50) // below 100 ms minimum
            .build();
        let err = match res {
            Ok(_) => panic!("must reject"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("ping_interval_ms"));
    }

    /// The fluent builder is the only channel through which tuning
    /// reaches the runtime. If a future refactor drops any setter, this
    /// test fails to compile — the desired regression guard.
    #[test]
    fn production_config_threads_timing_knobs_through_builder() {
        let cfg = DirectConfig::production();
        let creds = Credentials::new("user", "pw");
        let args = StreamingClientBuilder::new(&creds, &cfg.streaming.hosts)
            .ring_size(cfg.streaming.ring_size)
            .flush_mode(cfg.streaming.flush_mode)
            .reconnect_policy(cfg.reconnect.policy.clone())
            .reconnect_wait_ms(cfg.reconnect.wait_ms)
            .reconnect_wait_rate_limited_ms(cfg.reconnect.wait_rate_limited_ms)
            .derive_ohlcvc(cfg.streaming.derive_ohlcvc)
            .connect_timeout_ms(cfg.streaming.connect_timeout_ms)
            .read_timeout_ms(cfg.streaming.timeout_ms)
            .ping_interval_ms(cfg.streaming.ping_interval_ms)
            .into_args();
        assert_eq!(args.connect_timeout_ms, cfg.streaming.connect_timeout_ms);
        assert_eq!(args.read_timeout_ms, cfg.streaming.timeout_ms);
        assert_eq!(args.ping_interval_ms, cfg.streaming.ping_interval_ms);
        assert_eq!(args.ring_size, cfg.streaming.ring_size);
        assert_eq!(args.wait_ms, cfg.reconnect.wait_ms);
        assert_eq!(
            args.wait_rate_limited_ms,
            cfg.reconnect.wait_rate_limited_ms
        );
    }
}

#[cfg(test)]
mod full_stream_guard_tests {
    use super::{full_stream_sec_type_supported, HarnessPublishMode, StreamingClient};
    use crate::error::{ConfigErrorKind, Error};
    use crate::fpss::protocol::{Contract, SecTypeExt};
    use crate::tdbe::types::enums::SecType;

    /// The full-stream broadcast is only delivered upstream for Stock and
    /// Option. The subscribe boundary uses this predicate to reject any
    /// other security type before emitting a frame or tracking the
    /// subscription for reconnect replay, so a caller is told up front
    /// rather than waiting on a feed that will never arrive.
    #[test]
    fn full_stream_supported_only_for_stock_and_option() {
        assert!(full_stream_sec_type_supported(SecType::Stock));
        assert!(full_stream_sec_type_supported(SecType::Option));
        assert!(!full_stream_sec_type_supported(SecType::Index));
        assert!(!full_stream_sec_type_supported(SecType::Rate));
        assert!(!full_stream_sec_type_supported(SecType::Unknown));
    }

    /// End-to-end: a full-stream subscription on an index is rejected at the
    /// `subscribe` boundary with a configuration error, and the rejected
    /// subscription is never tracked — so the reconnect path has nothing to
    /// replay. Uses the test-only authenticated harness so the guard (which
    /// runs after `check_connected`) is exercised on the real subscribe path.
    #[test]
    fn subscribe_rejects_full_index_and_does_not_track_it() {
        let client = StreamingClient::for_self_join_test(
            0,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            |_event| {},
        );

        let err = match client.subscribe(SecType::Index.full_trades()) {
            Ok(()) => panic!("full-stream Index subscribe must be rejected"),
            Err(e) => e,
        };
        match err {
            Error::Config {
                kind: ConfigErrorKind::InvalidValue { ref field, .. },
                ..
            } => assert_eq!(field, "Subscription::full"),
            other => panic!("expected Error::Config InvalidValue, got {other:?}"),
        }

        // Rejected before the tracking push, so the reconnect-replay list is
        // still empty — the io_loop reconnect path will never re-send it.
        assert!(
            client.active_full_subscriptions().is_empty(),
            "rejected full-stream subscription must not be tracked"
        );

        client.shutdown();
    }

    /// A repeated per-contract subscribe must track the contract exactly
    /// once. Without de-dup the tracked list grows on every duplicate call
    /// and replays the same subscribe frame multiple times on reconnect.
    #[test]
    fn duplicate_contract_subscribe_tracks_once() {
        let client = StreamingClient::for_self_join_test(
            0,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            |_event| {},
        );

        let sub = Contract::stock("AAPL").trade();
        client.subscribe(sub.clone()).expect("first subscribe");
        client.subscribe(sub.clone()).expect("duplicate subscribe");
        client.subscribe(sub).expect("third subscribe");

        let tracked = client.active_subscriptions();
        assert_eq!(
            tracked.len(),
            1,
            "duplicate per-contract subscribes must collapse to one tracked entry, got {tracked:?}"
        );

        client.shutdown();
    }

    /// A repeated full-stream subscribe must track the `(kind, sec_type)`
    /// pair exactly once, for the same reconnect-replay reason.
    #[test]
    fn duplicate_full_subscribe_tracks_once() {
        let client = StreamingClient::for_self_join_test(
            0,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            |_event| {},
        );

        client
            .subscribe(SecType::Stock.full_trades())
            .expect("first full subscribe");
        client
            .subscribe(SecType::Stock.full_trades())
            .expect("duplicate full subscribe");

        let tracked = client.active_full_subscriptions();
        assert_eq!(
            tracked.len(),
            1,
            "duplicate full-stream subscribes must collapse to one tracked entry, got {tracked:?}"
        );

        client.shutdown();
    }

    /// After an unsubscribe the tracked entry is gone, and a later
    /// subscribe re-adds exactly one — proving de-dup does not break the
    /// existing remove-once semantics.
    #[test]
    fn unsubscribe_then_resubscribe_tracks_once() {
        let client = StreamingClient::for_self_join_test(
            0,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            |_event| {},
        );

        let contract = Contract::stock("MSFT");
        client
            .subscribe(contract.clone().trade())
            .expect("subscribe");
        assert_eq!(client.active_subscriptions().len(), 1);

        client
            .unsubscribe(contract.clone().trade())
            .expect("unsubscribe");
        assert!(
            client.active_subscriptions().is_empty(),
            "unsubscribe must remove the tracked entry"
        );

        client.subscribe(contract.trade()).expect("re-subscribe");
        assert_eq!(
            client.active_subscriptions().len(),
            1,
            "re-subscribe after unsubscribe must track exactly once"
        );

        client.shutdown();
    }

    /// The command channel is bounded: once it is saturated, a further
    /// `try_send` reports `Full` rather than growing without limit. This
    /// pins the backpressure contract `send_cmd` relies on to surface a
    /// typed queue-full error instead of silently dropping a command or
    /// accumulating unbounded memory. A held receiver keeps the channel
    /// alive so saturation (not hang-up) is the observed condition.
    #[test]
    fn command_channel_is_bounded() {
        use std::sync::mpsc as std_mpsc;
        let cap = super::CMD_CHANNEL_CAPACITY;
        let (tx, _rx) = std_mpsc::sync_channel::<super::events::IoCommand>(cap);
        // Fill to capacity: every send up to the bound must succeed.
        for _ in 0..cap {
            tx.try_send(super::events::IoCommand::Shutdown)
                .expect("sends up to capacity must succeed");
        }
        // The next send must report a full channel — the bound holds.
        match tx.try_send(super::events::IoCommand::Shutdown) {
            Err(std_mpsc::TrySendError::Full(_)) => {}
            other => panic!("expected TrySendError::Full once saturated, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod panic_isolation_tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::{HarnessPublishMode, StreamingClient};

    /// Poll the drained flag until it flips or the deadline passes.
    ///
    /// The drained flag is set by [`StreamingClient::drop`] after it has joined
    /// the I/O handle. Call this AFTER dropping the last `Arc<StreamingClient>`
    /// reference; until then the flag stays false.
    fn wait_for_drain(drained: &std::sync::atomic::AtomicBool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !drained.load(Ordering::Acquire) {
            if std::time::Instant::now() > deadline {
                panic!("StreamingClient did not drain within 5 seconds");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Poll the delivery counter until it reaches `expected` or the deadline
    /// passes.  The consumer thread updates it after each non-panic event, so
    /// when `expected` is reached the consumer has processed all events
    /// (including the earlier panicking ones) and `panic_count()` is stable.
    fn wait_for_deliveries(delivered: &AtomicU64, expected: u64) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while delivered.load(Ordering::Relaxed) < expected {
            if std::time::Instant::now() > deadline {
                panic!(
                    "consumer did not deliver {expected} events within 5 s; \
                     got {}",
                    delivered.load(Ordering::Relaxed)
                );
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// A panic in the callback on event 0 is caught and counted; events 1+
    /// continue to be delivered normally.
    ///
    /// The `for_self_join_test` harness drives events through the ring
    /// consumer closure which has per-invocation `catch_unwind` incrementing
    /// the shared `panics` counter (the same Arc that `StreamingClient::panic_count()`
    /// reads).  Once all non-panic events are delivered the consumer has
    /// processed every event, so `panic_count()` is stable and can be read on
    /// the still-live `Arc<StreamingClient>` before dropping.
    ///
    /// Contract: `client.panic_count() == 1` AND `delivered == N_EVENTS - 1`.
    #[test]
    fn panic_on_event_zero_is_isolated_delivery_continues() {
        const N_EVENTS: usize = 10;

        let delivered = Arc::new(AtomicU64::new(0));
        let delivered_c = Arc::clone(&delivered);
        let mut call_index: u64 = 0;

        let client = StreamingClient::for_self_join_test(
            N_EVENTS,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            move |_event| {
                let idx = call_index;
                call_index += 1;
                if idx == 0 {
                    panic!("intentional test panic on event 0");
                }
                delivered_c.fetch_add(1, Ordering::Relaxed);
            },
        );

        // Wait until the consumer has processed all N_EVENTS - 1 deliveries.
        // At that point it has also processed event 0 (the panic), so the
        // `panics` counter is stable on the live client.
        wait_for_deliveries(&delivered, (N_EVENTS - 1) as u64);

        // Read `panic_count()` on the live client before triggering Drop.
        // The `panics` Arc is shared between the `StreamingClient` struct and the
        // consumer closure; the consumer is done, the value is stable.
        let observed_panics = client.panic_count();
        let delivered_count = delivered.load(Ordering::Relaxed);

        // Drain: signal shutdown, drop the last Arc (triggers Drop → io_handle
        // join), then confirm the drained flag flips.
        let drained = client.drained_flag();
        client.shutdown();
        drop(client);
        wait_for_drain(&drained);

        assert_eq!(
            observed_panics, 1,
            "StreamingClient::panic_count() must equal 1 after one caught panic; \
             got {observed_panics}"
        );
        assert_eq!(
            delivered_count,
            (N_EVENTS - 1) as u64,
            "events 1..N_EVENTS must have been delivered after the panic on event 0; \
             got {delivered_count}"
        );
    }

    /// Two consecutive panics each increment the counter independently.
    ///
    /// Asserts against `client.panic_count()` — the shared counter on the
    /// public API — so a regression in the dispatcher's `catch_unwind`
    /// increment path is caught here, not just by the local delivery counter.
    #[test]
    fn two_consecutive_panics_count_independently() {
        const N_EVENTS: usize = 5;

        let delivered = Arc::new(AtomicU64::new(0));
        let delivered_c = Arc::clone(&delivered);
        let mut call_index: u64 = 0;

        let client = StreamingClient::for_self_join_test(
            N_EVENTS,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            move |_event| {
                let idx = call_index;
                call_index += 1;
                if idx < 2 {
                    panic!("intentional test panic on events 0 and 1");
                }
                delivered_c.fetch_add(1, Ordering::Relaxed);
            },
        );

        // Wait until the consumer has processed all N_EVENTS - 2 deliveries.
        // Events 0 and 1 panicked, so the delivery counter saturates at 3.
        wait_for_deliveries(&delivered, (N_EVENTS - 2) as u64);

        let observed_panics = client.panic_count();
        let delivered_count = delivered.load(Ordering::Relaxed);

        let drained = client.drained_flag();
        client.shutdown();
        drop(client);
        wait_for_drain(&drained);

        assert_eq!(
            observed_panics, 2,
            "StreamingClient::panic_count() must equal 2 after two caught panics; \
             got {observed_panics}"
        );
        assert_eq!(
            delivered_count,
            (N_EVENTS - 2) as u64,
            "events 2..N_EVENTS must have been delivered after two panics; \
             got {delivered_count}"
        );
    }
}

#[cfg(test)]
mod ring_occupancy_tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::events::FpssEventInternal;
    use super::ring::RingProducer;
    use super::{HarnessPublishMode, PollOutcome, StreamControl, StreamingClient};

    /// Publish one synthetic control event through the test producer.
    fn publish_one(producer: &mut impl RingProducer) -> bool {
        producer
            .try_publish(|slot| {
                slot.event = FpssEventInternal::Control(StreamControl::MarketOpen);
            })
            .is_ok()
    }

    /// Occupancy rises by one per published event and returns to zero
    /// after a single `poll_batch` drain; capacity reports the
    /// configured ring size.
    #[test]
    fn occupancy_tracks_publish_then_drain() {
        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);

        assert_eq!(client.ring_capacity(), 64);
        assert_eq!(client.ring_occupancy(), 0, "fresh ring must read empty");

        for published in 1..=5usize {
            assert!(publish_one(&mut producer), "ring must not be full yet");
            assert_eq!(
                client.ring_occupancy(),
                published,
                "occupancy must count each undrained publish"
            );
        }

        let mut delivered = 0usize;
        let outcome = client.poll_batch(|_event| delivered += 1);
        assert_eq!(outcome, PollOutcome::Drained(5));
        assert_eq!(delivered, 5);
        assert_eq!(client.ring_occupancy(), 0, "a drained ring must read empty");
    }

    /// A publish burst against a full ring saturates at the configured
    /// capacity: overflow publishes fail without advancing the
    /// published cursor, so occupancy never exceeds capacity.
    #[test]
    fn occupancy_never_exceeds_capacity_under_full_ring_burst() {
        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);

        let mut accepted = 0usize;
        for _ in 0..(64 + 16) {
            if publish_one(&mut producer) {
                accepted += 1;
            }
            assert!(
                client.ring_occupancy() <= client.ring_capacity(),
                "occupancy must never exceed capacity"
            );
        }
        assert_eq!(accepted, 64, "exactly ring_size publishes must land");
        assert_eq!(client.ring_occupancy(), client.ring_capacity());

        let outcome = client.poll_batch(|_event| {});
        assert_eq!(outcome, PollOutcome::Drained(64));
        assert_eq!(client.ring_occupancy(), 0);
    }

    /// Consumed progress is recorded at batch granularity: a single
    /// `try_next_event` pull stages the whole available batch out of
    /// the ring, so occupancy drops to zero even though undelivered
    /// events remain in the client-side staging queue.
    #[test]
    fn occupancy_counts_drained_batches_not_delivered_events() {
        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);

        for _ in 0..3 {
            assert!(publish_one(&mut producer));
        }
        assert_eq!(client.ring_occupancy(), 3);

        let first = client
            .try_next_event()
            .expect("staging mutex must not be poisoned");
        assert!(first.is_some(), "one event must be delivered");
        assert_eq!(
            client.ring_occupancy(),
            0,
            "the drain staged the whole batch out of the ring; occupancy \
             tracks ring slots, not the staging queue"
        );

        // The two staged events still arrive.
        assert!(client
            .try_next_event()
            .expect("staging mutex must not be poisoned")
            .is_some());
        assert!(client
            .try_next_event()
            .expect("staging mutex must not be poisoned")
            .is_some());
    }

    /// The dispatcher-thread harness keeps the same contract end to
    /// end: once every event is delivered, the consumer's end-of-batch
    /// cursor store has caught up with the producer and occupancy
    /// reads zero.
    #[test]
    fn harness_occupancy_drains_to_zero_after_delivery() {
        const N_EVENTS: usize = 10;

        let delivered = Arc::new(AtomicU64::new(0));
        let delivered_c = Arc::clone(&delivered);

        let client = StreamingClient::for_self_join_test(
            N_EVENTS,
            64,
            HarnessPublishMode::TryPublishBurst,
            None,
            move |_event| {
                delivered_c.fetch_add(1, Ordering::Relaxed);
            },
        );

        assert_eq!(client.ring_capacity(), 64);

        // Bounded wait, no fixed sleep: occupancy must reach zero once
        // the dispatcher has drained the burst.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let all_delivered = delivered.load(Ordering::Relaxed) == N_EVENTS as u64;
            if all_delivered && client.ring_occupancy() == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "occupancy did not drain to zero within 5 s; delivered={}, occupancy={}",
                delivered.load(Ordering::Relaxed),
                client.ring_occupancy()
            );
            std::thread::sleep(Duration::from_millis(1));
        }

        client.shutdown();
    }
}
