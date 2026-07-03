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
//! # fn example() -> Result<(), thetadatadx::streaming::StreamError> {
//! let creds = Credentials::new("user@example.com", "pw");
//! let config = thetadatadx::config::DirectConfig::production();
//! let hosts = config.streaming_hosts();
//!
//! let client = StreamingClient::builder(&creds, hosts).build()?;
//! client.subscribe(Contract::stock("AAPL").quote())?;
//!
//! for event in &client {
//!     let _event: StreamEvent = event?;
//!     // ...
//! }
//! # Ok(())
//! # }
//! ```

pub(crate) mod affinity;
#[cfg(feature = "arrow")]
pub mod batch_reader;
#[cfg(feature = "arrow")]
pub mod batch_schema;
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

pub use self::decode::UNRESOLVED_CONTRACT_SYMBOL_PREFIX;
use self::events::IoCommand;
pub use self::events::{StreamControl, StreamData, StreamEvent};
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
/// in `thetadatadx-rs/tests/`.
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
    pub use super::framing::{read_frame_into, MAX_PAYLOAD_LEN};

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

/// Crate-private FPSS decode / wire / framing / login / connection surface,
/// exposed behind `__internal` for a downstream consumer that drives the
/// decode into its own ingest ring (no Disruptor, no second consumer thread).
/// NOT part of the supported public API: subject to change without a SemVer
/// bump, and absent unless `__internal` is enabled, so `cargo-semver-checks`
/// (default features) never sees it.
#[cfg(feature = "__internal")]
#[doc(hidden)]
pub mod internals {
    pub use super::connection::{connect_to_servers, FpssStream, TcpKeepaliveSpec};
    pub use super::decode::decode_frame;
    pub use super::delta::DeltaState;
    pub use super::events::FpssEventInternal;
    pub use super::framing::{
        is_transient_read, read_frame_into_with_stall_timeout, write_raw_frame,
        write_raw_frame_no_flush, FrameRead, MAX_PAYLOAD_LEN,
    };
    pub use super::io_loop::{wait_for_login, LoginResult};
    pub use super::protocol::wire::{
        build_credentials_payload, build_full_type_subscribe_payload, build_ping_payload,
        build_stop_payload, build_subscribe_payload,
    };
}

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Duration;

use crate::auth::Credentials;
use crate::backoff::JitterMode;
use crate::config::{HostSelectionPolicy, ReconnectPolicy, StreamingFlushMode};
use crate::error::Error;
use crate::tdbe::types::enums::{RemoveReason, SecType, StreamMsgType};

use self::protocol::{build_login_payload, build_subscribe_payload, Contract, SubscriptionKind};

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
/// The ping thread and the public subscribe / unsubscribe methods all enqueue
/// with a non-blocking `try_send`. Subscribe / unsubscribe surface a typed
/// [`StreamErrorKind::Disconnected`] backpressure error on a full channel
/// rather than silently dropping a command; the idempotent ping heartbeat
/// simply skips a beat when the channel is momentarily full (the I/O thread is
/// draining a backlog, so the connection is demonstrably alive and the next
/// beat follows one interval later) instead of blocking the ping thread.
pub(in crate::fpss) const CMD_CHANNEL_CAPACITY: usize = 16_384;

// ---------------------------------------------------------------------------
// StreamError — typed error enum returned by the FPSS public surface
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
/// `From<StreamError> for Error` maps each `StreamError` variant into an
/// umbrella [`crate::Error`] variant according to the table below.
/// `DispatcherFailed` does NOT have a dedicated umbrella variant; it
/// is encoded as `Error::Stream { kind: Disconnected }` with a
/// `"dispatcher failed: "` prefix on the message, which the reverse
/// direction recognises:
///
/// | `StreamError`               | `Error`                                                         |
/// |---------------------------|-----------------------------------------------------------------|
/// | `ConnectionRefused(m)`    | `Error::Stream { kind: ConnectionRefused, message: m }`           |
/// | `Timeout(m)`              | `Error::Stream { kind: Timeout, message: m }`                     |
/// | `Protocol(m)`             | `Error::Stream { kind: ProtocolError, message: m }`               |
/// | `Disconnected(m)`         | `Error::Stream { kind: Disconnected, message: m }`                |
/// | `RateLimited(m)`          | `Error::Stream { kind: TooManyRequests, message: m }`             |
/// | `AuthenticationFailed(m)` | `Error::Auth { kind: InvalidCredentials, message: m }`          |
/// | `Config(m)`               | `Error::Config { kind: InvalidValue { field: "fpss", message }}`|
/// | `Io(m)`                   | `Error::Io(io::Error::other(m))`                                |
/// | `DispatcherFailed(m)`     | `Error::Stream { kind: Disconnected, message: "dispatcher failed: {m}" }` |
///
/// Round-tripping `StreamError → Error → StreamError` preserves the
/// variant for every row above (the prefixed message lets the
/// `Disconnected → DispatcherFailed` decoder run) and preserves the
/// message string verbatim. Two caveats:
///
/// - `Io(m) → Error::Io(io::Error::other(m)) → Io(io.to_string())`
///   preserves the message text but the synthesised inner
///   `io::Error` reports `ErrorKind::Other`. A caller that
///   round-trips through `Error` and then inspects the recovered
///   `io::ErrorKind` will see `Other`, not whatever kind the original
///   `StreamError::Io` was carrying in its string form.
/// - `Config(m) → Error::Config { kind: InvalidValue { field: "fpss",
///   message } }` regenerates `field = "fpss"` unconditionally. A
///   caller that converted an `Error::Config { kind: InvalidValue {
///   field: "<custom>", .. } }` into `StreamError::Config` loses the
///   original field name on the round trip.
/// - `Disconnected(m)` with a user-supplied `m` that happens to start
///   with the literal `"dispatcher failed: "` prefix re-emerges as
///   `DispatcherFailed` — do not author messages with that prefix
///   manually.
///
/// `From<Error> for StreamError` is **best-effort categorisation**. The
/// FPSS-shaped umbrella variants (`Error::Stream`, `Error::Auth`,
/// `Error::Config`, `Error::Io`, `Error::Tls`, `Error::Timeout`)
/// preserve their human-readable message and route to the closest
/// `StreamError` variant; everything else (gRPC, decode, transport)
/// collapses to `StreamError::Protocol` with the `Display` string of the
/// source error. Use this direction at SDK boundaries where the
/// caller already knows the error originated on the FPSS surface.
#[derive(thiserror::Error, Debug, Clone)]
#[non_exhaustive]
pub enum StreamError {
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

    /// A drain method was entered while the staging state was already
    /// held by another in-flight drain on the same client. The drain is
    /// single-consumer by contract: a callback that re-enters a drain
    /// method, or a second thread that drains concurrently, hits this
    /// instead of blocking on the non-reentrant staging mutex. The call
    /// performed no work; retry once the in-flight drain returns.
    #[error("reentrant or concurrent drain rejected: {0}")]
    ReentrantDrain(String),

    /// I/O error on the FPSS socket.
    #[error("io: {0}")]
    Io(String),
}

impl From<StreamError> for Error {
    fn from(e: StreamError) -> Self {
        use crate::error::{AuthErrorKind, ConfigErrorKind, StreamErrorKind};
        match e {
            StreamError::ConnectionRefused(message) => Error::Stream {
                kind: StreamErrorKind::ConnectionRefused,
                message,
            },
            StreamError::Timeout(message) => Error::Stream {
                kind: StreamErrorKind::Timeout,
                message,
            },
            StreamError::Protocol(message) => Error::Stream {
                kind: StreamErrorKind::ProtocolError,
                message,
            },
            StreamError::Disconnected(message) => Error::Stream {
                kind: StreamErrorKind::Disconnected,
                message,
            },
            StreamError::RateLimited(message) => Error::Stream {
                kind: StreamErrorKind::TooManyRequests,
                message,
            },
            StreamError::AuthenticationFailed(message) => Error::Auth {
                kind: AuthErrorKind::InvalidCredentials,
                message,
            },
            StreamError::DispatcherFailed(message) => Error::Stream {
                kind: StreamErrorKind::Disconnected,
                message: format!("dispatcher failed: {message}"),
            },
            StreamError::ReentrantDrain(message) => Error::Stream {
                kind: StreamErrorKind::ReentrantDrain,
                message,
            },
            StreamError::Config(message) => Error::Config {
                kind: ConfigErrorKind::InvalidValue {
                    field: "fpss".to_string(),
                    message: message.clone(),
                },
                message,
                source: None,
            },
            StreamError::Io(message) => Error::Io(std::io::Error::other(message)),
        }
    }
}

impl From<Error> for StreamError {
    fn from(e: Error) -> Self {
        use crate::error::StreamErrorKind;
        match e {
            Error::Stream { kind, message } => match kind {
                StreamErrorKind::ConnectionRefused => StreamError::ConnectionRefused(message),
                StreamErrorKind::Timeout => StreamError::Timeout(message),
                StreamErrorKind::ProtocolError => StreamError::Protocol(message),
                StreamErrorKind::Disconnected => {
                    if let Some(payload) = message.strip_prefix("dispatcher failed: ") {
                        StreamError::DispatcherFailed(payload.to_string())
                    } else {
                        StreamError::Disconnected(message)
                    }
                }
                StreamErrorKind::TooManyRequests => StreamError::RateLimited(message),
                StreamErrorKind::ReentrantDrain => StreamError::ReentrantDrain(message),
            },
            Error::Auth { message, .. } => StreamError::AuthenticationFailed(message),
            Error::Io(io) => StreamError::Io(io.to_string()),
            Error::Tls(t) => StreamError::ConnectionRefused(t.to_string()),
            Error::Timeout { duration_ms } => {
                StreamError::Timeout(format!("deadline exceeded after {duration_ms}ms"))
            }
            Error::Config { message, .. } => StreamError::Config(message),
            other => StreamError::Protocol(other.to_string()),
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
/// flush mode, reconnect policy, timeouts) and returns a connected
/// client from [`Self::build`]. Optional setters consume `self` so
/// calls chain.
///
/// # Example
///
/// ```rust,no_run
/// # use thetadatadx::fpss::{StreamingClient, StreamEvent};
/// # use thetadatadx::auth::Credentials;
/// # fn example() -> Result<(), thetadatadx::streaming::StreamError> {
/// let creds = Credentials::new("user@example.com", "pw");
/// let config = thetadatadx::config::DirectConfig::production();
/// let hosts = config.streaming_hosts();
///
/// let client = StreamingClient::builder(&creds, hosts)
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
    connect_timeout_ms: u64,
    read_timeout_ms: u64,
    ping_interval_ms: u64,
    io_read_slice_ms: u64,
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
            connect_timeout_ms: fpss.connect_timeout_ms,
            read_timeout_ms: fpss.timeout_ms,
            ping_interval_ms: fpss.ping_interval_ms,
            io_read_slice_ms: fpss.io_read_slice_ms,
            keepalive_idle_secs: fpss.keepalive_idle_secs,
            keepalive_interval_secs: fpss.keepalive_interval_secs,
            keepalive_retries: fpss.keepalive_retries,
            host_selection: fpss.host_selection,
            host_shuffle_seed: fpss.host_shuffle_seed,
            wait_strategy: ring::AdaptiveWaitStrategy::low_latency(),
            consumer_cpu: fpss.consumer_cpu,
        }
    }

    /// Event ring buffer size (events). Must be a power of two.
    ///
    /// Default `131_072`. Each slot stores one event (96 bytes on the
    /// current 64-bit layout, validated by `assert_layout_compat`), so
    /// `131_072 × 96 ≈ 12 MiB` per client plus refcounted `Arc<Contract>`
    /// storage on top. Tune down (e.g. `16_384`) for a smaller per-client
    /// footprint, or up for more overflow headroom on bursty load — a
    /// larger ring absorbs longer consumer stalls before events drop.
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
    /// Returns [`StreamError::ConnectionRefused`] if no host accepts the
    /// TLS handshake, [`StreamError::AuthenticationFailed`] on login
    /// failure, and other variants on protocol violations or
    /// configuration validation errors.
    pub fn build(self) -> Result<StreamingClient, StreamError> {
        StreamingClient::connect(self.into_args()).map_err(StreamError::from)
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
            connect_timeout_ms: self.connect_timeout_ms,
            read_timeout_ms: self.read_timeout_ms,
            ping_interval_ms: self.ping_interval_ms,
            io_read_slice_ms: self.io_read_slice_ms,
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
    pub(crate) connect_timeout_ms: u64,
    pub(crate) read_timeout_ms: u64,
    pub(crate) ping_interval_ms: u64,
    pub(crate) io_read_slice_ms: u64,
    pub(crate) keepalive_idle_secs: u64,
    pub(crate) keepalive_interval_secs: u64,
    pub(crate) keepalive_retries: u32,
    pub(crate) host_selection: HostSelectionPolicy,
    pub(crate) host_shuffle_seed: Option<u64>,
    /// Fixed low-latency event-ring consumer wait strategy
    /// ([`ring::AdaptiveWaitStrategy`]).
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
    keepalive: connection::TcpKeepaliveSpec,
    last_event_at_ns: Arc<AtomicI64>,
    connected_addr: Arc<Mutex<String>>,
    ping_interval: Duration,
    shutdown: Arc<AtomicBool>,
    authenticated: Arc<AtomicBool>,
    active_subs: Arc<Mutex<Vec<(SubscriptionKind, Contract)>>>,
    active_full_subs: Arc<Mutex<Vec<(SubscriptionKind, crate::tdbe::types::enums::SecType)>>>,
    pending_subs: Arc<Mutex<std::collections::HashMap<i32, io_loop::PendingSubEntry>>>,
    dropped: Arc<AtomicU64>,
    panics: Arc<AtomicU64>,
    io_faulted: Arc<AtomicBool>,
    ring_cursors: Arc<RingCursors>,
    consumer_thread_id: Arc<Mutex<Option<ThreadId>>>,
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
    /// In-flight subscribes keyed by `req_id`, awaiting a server
    /// `REQ_RESPONSE`. A subscribe records its tracked identity here when the
    /// frame is sent; the I/O thread removes the entry when the response
    /// lands, and on a rejection also drops the matching entry from
    /// `active_subs` / `active_full_subs` so a server-rejected subscription is
    /// neither replayed on reconnect nor over-reported by
    /// `active_subscriptions()`.
    pub(in crate::fpss) pending_subs:
        Arc<Mutex<std::collections::HashMap<i32, io_loop::PendingSubEntry>>>,
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
    /// Fixed low-latency event-ring consumer wait strategy applied by
    /// the blocking poll loops ([`Self::next_event`] /
    /// [`Self::for_each_scoped`]) when the ring is momentarily empty, so
    /// the consumer-side wait matches the ring builder's strategy.
    wait_strategy: ring::AdaptiveWaitStrategy,
    /// Optional CPU core to pin the consumer drain thread to; `None`
    /// (default) leaves it under the OS scheduler. Applied by the drain
    /// primitives once drain ownership is proven (inside
    /// [`Self::poll_batch`] / [`Self::try_next_event_internal`]) via
    /// [`affinity::pin_consumer_thread`].
    consumer_cpu: Option<usize>,
    /// `ThreadId` the consumer-core pin currently targets, or `None`
    /// before the first pin. [`Self::record_drainer_and_pin`] re-applies
    /// the pin whenever the proven drainer differs from this value, so a
    /// handoff of drain ownership to a new thread re-pins the *new*
    /// drainer instead of leaving the affinity stuck on a stale thread.
    /// The recorded thread is the steady-state on the single-consumer
    /// path, so the affinity syscall still fires at most once there and
    /// never on the per-event path.
    consumer_pinned_to: Mutex<Option<ThreadId>>,
    /// Cumulative count of user-callback panics caught by the
    /// event-dispatch consumer's `catch_unwind` boundary. Snapshot via
    /// [`StreamingClient::panic_count`].
    panics: Arc<AtomicU64>,
    /// Set by the I/O thread's fault guard if `io_loop` unwinds. A panic
    /// there drops the ring producer, which the consumer would otherwise
    /// read as a clean end-of-stream while `is_authenticated()` stayed
    /// `true`. The guard flips this (plus `authenticated` false and
    /// `shutdown` true) so the blocking drain paths surface
    /// [`StreamError::DispatcherFailed`] instead of `Ok(None)`.
    io_faulted: Arc<AtomicBool>,
    /// `ThreadId` of the thread that actually owns the drain, recorded by
    /// the drain primitives **after** they acquire the `poller_state` lock.
    ///
    /// Held in a `Mutex<Option<_>>` rather than a `OnceLock` on purpose:
    /// the identity must be recordable only once drain ownership is proven
    /// and **correctable** afterwards. A `OnceLock` armed at drain entry
    /// (before the `try_lock`) could be claimed permanently by a thread
    /// whose drain attempt then failed with `Busy` / `ReentrantDrain` and
    /// never drained a single event; `Drop` would then read that phantom
    /// identity and miss the real drainer's detach path. Recording inside
    /// the lock guarantees the value is always the most recent thread that
    /// held drain ownership, so the `Drop` self-join detector and the
    /// consumer-CPU pin both observe the true drainer.
    ///
    /// `None` until the first drain runs. The harness in
    /// [`Self::for_self_join_test`] records through the same cell from its
    /// dispatcher-thread consumer; production bindings own their own
    /// dispatcher join handles and additionally detect self-join at their
    /// level.
    consumer_thread_id: Arc<Mutex<Option<ThreadId>>>,
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
            connect_timeout_ms,
            read_timeout_ms,
            ping_interval_ms,
            io_read_slice_ms,
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
        // The write deadline bounds the credentials write that drives the
        // lazy TLS handshake and every steady-state ping/subscribe write.
        // It shares the read timeout's budget: both bound a single
        // unacknowledged transport operation during the connect window.
        let write_timeout = read_timeout;
        // The initial dial runs synchronously inside the caller's `connect`
        // before any client (and thus any Drop) exists, so there is no
        // shutdown flag to observe yet; pass a never-set one. The
        // reconnect path in the io_loop passes its live shutdown flag.
        let (stream, server_addr) = connection::connect_to_servers(
            &borrowed,
            connect_timeout,
            read_timeout,
            write_timeout,
            keepalive,
            &AtomicBool::new(false),
        )?;
        Self::connect_with_stream(connection::ConnectWithStreamArgs {
            creds,
            stream,
            server_addr,
            hosts,
            host_selection,
            host_shuffle_seed: seed,
            ring_size,
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
            keepalive,
            ping_interval,
        } = args;
        // Send CREDENTIALS (code 0). Write straight from the `Zeroizing` buffer
        // rather than moving the cleartext into a `Frame`, so the secret bytes
        // are wiped on drop instead of lingering in a frame-owned `Vec`.
        let cred_payload = build_login_payload(creds)?;
        framing::write_raw_frame(&mut stream, StreamMsgType::Credentials, &cred_payload)?;
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
        // Initial synchronous dial on the caller's thread: no live shutdown
        // flag exists yet (the I/O thread is not spawned), so the handshake is
        // bounded by the socket timeouts alone. The reconnect path, which runs
        // on the I/O thread, passes its shutdown flag.
        let login_result = wait_for_login(&mut stream, &mut pending_control, read_timeout, None)?;

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
                return Err(Error::Stream {
                    kind: crate::error::StreamErrorKind::Disconnected,
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
            .map_err(|e| Error::Stream {
                kind: crate::error::StreamErrorKind::ConnectionRefused,
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
        let pending_subs: Arc<Mutex<std::collections::HashMap<i32, io_loop::PendingSubEntry>>> =
            Arc::new(Mutex::new(std::collections::HashMap::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let panics = Arc::new(AtomicU64::new(0));
        let io_faulted = Arc::new(AtomicBool::new(false));
        // Captured by the event-dispatch consumer closure on first dispatch
        // and read by `StreamingClient::drop` to break the self-join cycle
        // (callback -> stop_streaming -> drop StreamingClient -> join io
        // thread -> drop producer -> join consumer thread = self).
        let consumer_thread_id: Arc<Mutex<Option<ThreadId>>> = Arc::new(Mutex::new(None));

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
            keepalive,
            last_event_at_ns,
            connected_addr,
            ping_interval,
            shutdown,
            authenticated,
            active_subs,
            active_full_subs,
            pending_subs,
            dropped,
            panics,
            io_faulted,
            ring_cursors,
            consumer_thread_id,
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
            keepalive,
            last_event_at_ns,
            connected_addr,
            ping_interval,
            shutdown,
            authenticated,
            active_subs,
            active_full_subs,
            pending_subs,
            dropped,
            panics,
            io_faulted,
            ring_cursors,
            consumer_thread_id,
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
        let io_pending_subs = Arc::clone(&pending_subs);
        let io_dropped = Arc::clone(&dropped);
        let io_next_req_id = Arc::clone(&next_req_id);
        let io_last_event_at_ns = Arc::clone(&last_event_at_ns);
        let io_connected_addr = Arc::clone(&connected_addr);
        // The fault flag is handed to `io_loop`, which arms its own drop
        // guard AFTER binding the ring producer so an unwind sets the flag
        // BEFORE the producer publishes the ring's shutdown sequence. Arming
        // the guard out here (before `producer` moves into `io_loop`) would
        // drop it after the producer on the panic path, leaving a window
        // where the consumer reads shutdown before the flag is set.
        let io_faulted_loop = Arc::clone(&io_faulted);

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
                    pending_subs: io_pending_subs,
                    dropped: io_dropped,
                    connect_timeout,
                    read_timeout,
                    io_read_slice,
                    keepalive,
                    last_event_at_ns: io_last_event_at_ns,
                    connected_addr: io_connected_addr,
                    next_req_id: io_next_req_id,
                    io_faulted: io_faulted_loop,
                });
            })
            .map_err(|e| Error::Stream {
                kind: crate::error::StreamErrorKind::ConnectionRefused,
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
            .map_err(|e| Error::Stream {
                kind: crate::error::StreamErrorKind::ConnectionRefused,
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
            pending_subs,
            server_addr,
            last_event_at_ns,
            connected_addr,
            replay_burst_size: client_replay_burst_size,
            replay_pace_ms: client_replay_pace_ms,
            dropped,
            panics,
            io_faulted,
            ring_cursors,
            ring_size,
            wait_strategy,
            consumer_cpu,
            consumer_pinned_to: Mutex::new(None),
            consumer_thread_id,
            drained: Arc::new(AtomicBool::new(false)),
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

    /// Shared quiescence flag for this client. Flipped to `true` after
    /// the network reader and the event-delivery thread have both stopped,
    /// so the user callback is guaranteed to have stopped firing.
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

    /// Record the current thread as the drain owner and apply the
    /// configured consumer-core pin.
    ///
    /// Called by the drain primitives ([`Self::try_next_event_internal`]
    /// and [`Self::poll_batch`]) **after** they acquire the `poller_state`
    /// lock — never before. Acquiring drain ownership first is the
    /// invariant that keeps the recorded identity honest: a thread whose
    /// drain attempt loses the `try_lock` race (a `Busy` / `ReentrantDrain`
    /// that never drains) must not be able to claim the consumer-thread
    /// role. Only the thread holding the staging lock has proven it is the
    /// real drainer, so only it records here.
    ///
    /// The identity is stored in a `Mutex<Option<ThreadId>>` and refreshed
    /// to the current thread on every call, so the value always reflects
    /// the most recent proven drainer (correctable, unlike a `OnceLock`).
    /// Recording the consumer thread id arms the `Drop` self-join guard:
    /// without it `Drop` could join the I/O handle inline on the
    /// dispatcher thread a user callback called back into — that thread is
    /// the one cleanup must complete on, so the inline join would
    /// self-deadlock.
    ///
    /// The CPU affinity pin targets whichever thread is the proven
    /// drainer and is re-applied when that thread changes (tracked by
    /// `consumer_pinned_to`), only when a `consumer_cpu` is configured.
    /// On the single-consumer steady state the drainer never changes, so
    /// the affinity syscall still fires at most once; a genuine handoff
    /// re-pins the new drainer rather than leaving the pin on the stale
    /// thread.
    fn record_drainer_and_pin(&self) {
        // Record under the lock: the value is small and the lock is
        // uncontended on the single-consumer happy path. Overwrite rather
        // than set-once so a (legitimate) change of drain owner corrects
        // the recorded identity instead of pinning a stale one.
        let cur = thread::current().id();
        if let Ok(mut id) = self.consumer_thread_id.lock() {
            if *id != Some(cur) {
                *id = Some(cur);
            }
        }
        if self.consumer_cpu.is_none() {
            return;
        }
        // Re-pin only when the proven drainer differs from the thread the
        // pin currently targets. The one-shot flag this replaces could
        // not re-pin after a handoff, so a new drainer inherited the old
        // thread's affinity. Holding the lock across the syscall keeps
        // the check-and-pin atomic against a concurrent handoff; on the
        // single-consumer path the branch is taken exactly once.
        if let Ok(mut pinned) = self.consumer_pinned_to.lock() {
            if *pinned != Some(cur) {
                affinity::pin_consumer_thread(self.consumer_cpu);
                *pinned = Some(cur);
            }
        }
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
    /// # Single-consumer contract
    ///
    /// The drain is single-consumer: do not re-enter `next_event` (or any
    /// other drain method) from inside a callback driven by this client, and
    /// do not drive two drains on the same client concurrently. A reentrant
    /// or concurrent drain returns [`StreamError::ReentrantDrain`] rather than
    /// blocking on the non-reentrant staging mutex, so misuse fails fast
    /// instead of hard-hanging. The normal single-threaded drain always
    /// acquires uncontended and is unaffected.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::DispatcherFailed`] if the internal staging
    /// queue's mutex was poisoned by a panicking caller on a previous
    /// invocation, or [`StreamError::ReentrantDrain`] if the drain was
    /// re-entered or driven concurrently.
    pub fn next_event(&self) -> Result<Option<StreamEvent>, StreamError> {
        // The drain owner is recorded inside `try_next_event_internal`,
        // after the staging lock is acquired — not here at entry — so a
        // failed `ReentrantDrain` attempt never claims the consumer-thread
        // identity. See `record_drainer_and_pin`.
        let waiter = self.wait_strategy;
        loop {
            match self.try_next_event_internal()? {
                TryNext::Event(event) => return Ok(Some(event)),
                TryNext::Empty => waiter.wait_for(0),
                TryNext::Shutdown => return self.shutdown_outcome(),
            }
        }
    }

    /// End-of-stream disposition for the drain paths: a clean `Ok(None)`
    /// normally, or [`StreamError::DispatcherFailed`] if the I/O thread
    /// unwound. Without this a panicked `io_loop` would read as a graceful
    /// shutdown (`Ok(None)`); the fault guard sets `io_faulted` on unwind so
    /// the failure surfaces here instead.
    fn shutdown_outcome(&self) -> Result<Option<StreamEvent>, StreamError> {
        // We only reach here after the drain read the ring's shutdown sequence
        // (a RELAXED load in the disruptor poll). This Acquire fence pairs with
        // the Release fence the fault guard runs after setting `io_faulted`
        // (see `io_loop::IoLoopFaultGuard`): fence-to-fence through the relaxed
        // shutdown store/load establishes that a reader which saw ring shutdown
        // also sees `io_faulted`. A plain Acquire load would synchronise only
        // with a Release store to `io_faulted` itself, leaving a weak-memory
        // window where shutdown is visible but `io_faulted` still reads false.
        std::sync::atomic::fence(Ordering::Acquire);
        if self.io_faulted.load(Ordering::Relaxed) {
            Err(StreamError::DispatcherFailed(
                "fpss io thread terminated abnormally".to_string(),
            ))
        } else {
            Ok(None)
        }
    }

    /// Terminal disposition for the callback drain path: [`PollOutcome::Failed`]
    /// if the I/O thread unwound, else [`PollOutcome::Shutdown`]. The
    /// callback-drain analogue of [`Self::shutdown_outcome`] (which serves the
    /// pull path): without it a panicked `io_loop` reads as a graceful
    /// shutdown to `poll_batch` / `for_each` consumers, so a callback or
    /// binding dispatcher would see a clean stream end on an I/O-thread fault.
    fn terminal_poll_outcome(&self) -> PollOutcome {
        // Same Acquire-fence discipline as `shutdown_outcome`: we reach here
        // only after the drain observed the ring's shutdown sequence (a RELAXED
        // load in the disruptor poll). This fence pairs with the Release fence
        // the fault guard runs after setting `io_faulted`; fence-to-fence
        // through the relaxed shutdown store/load establishes that a reader
        // which saw ring shutdown also sees `io_faulted`. A plain Acquire load
        // would synchronise only with a Release store to `io_faulted` itself,
        // leaving a weak-memory window where shutdown is visible but
        // `io_faulted` still reads false.
        std::sync::atomic::fence(Ordering::Acquire);
        if self.io_faulted.load(Ordering::Relaxed) {
            PollOutcome::Failed
        } else {
            PollOutcome::Shutdown
        }
    }

    /// Non-blocking single-event pull from the ring. Returns `Ok(None)`
    /// when the ring is momentarily empty OR terminally shut down — use
    /// [`Self::next_event`] when you need to distinguish the two.
    ///
    /// # Single-consumer contract
    ///
    /// Single-consumer like [`Self::next_event`]: a reentrant (from inside a
    /// callback) or concurrent drain returns [`StreamError::ReentrantDrain`]
    /// rather than blocking on the non-reentrant staging mutex.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::DispatcherFailed`] if the staging mutex was
    /// poisoned, or [`StreamError::ReentrantDrain`] if the drain was re-entered
    /// or driven concurrently.
    pub fn try_next_event(&self) -> Result<Option<StreamEvent>, StreamError> {
        match self.try_next_event_internal()? {
            TryNext::Event(event) => Ok(Some(event)),
            TryNext::Empty => Ok(None),
            TryNext::Shutdown => self.shutdown_outcome(),
        }
    }

    fn try_next_event_internal(&self) -> Result<TryNext, StreamError> {
        // `try_lock`, not `lock`: the staging mutex is non-reentrant, so a
        // user callback that re-enters a drain method on this client (or a
        // second thread draining concurrently) would hard-hang on a blocking
        // acquire. The drain is single-consumer by contract; the happy path
        // is always uncontended and acquires immediately. Contention can
        // only mean a reentrant or concurrent drain, so fail fast with a
        // typed outcome instead of deadlocking.
        let mut guard = match self.poller_state.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(StreamError::ReentrantDrain(
                    "next_event/try_next_event must not be re-entered from a callback or driven \
                     concurrently"
                        .to_string(),
                ));
            }
            Err(std::sync::TryLockError::Poisoned(e)) => {
                return Err(StreamError::DispatcherFailed(format!(
                    "poller mutex poisoned: {e}"
                )));
            }
        };
        // Drain ownership is now proven (the staging lock is held), so this
        // is the real drainer: record its identity and apply the CPU pin.
        // A reentrant/concurrent caller that failed the `try_lock` above
        // returned before reaching here, so it can never claim the role.
        self.record_drainer_and_pin();
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

    /// Invoke one user-callback dispatch under the panic boundary.
    ///
    /// Returns the `catch_unwind` result so the caller keeps ownership of
    /// the panic-counting and delivery-counting decisions. Rust cannot
    /// safely cancel arbitrary user code, so a panicking callback is caught
    /// here and counted; the drain then continues with the next event.
    #[inline]
    fn dispatch_caught(&self, call: impl FnOnce()) -> std::thread::Result<()> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(call))
    }

    /// Non-blocking single-batch drain through `on_event`.
    ///
    /// Returns a [`PollOutcome`] so a caller integrating the polling
    /// loop into its own scheduler can tell a drained batch (and how
    /// many events it carried) apart from terminal shutdown. Each
    /// `&StreamEvent` handed to `on_event` is a zero-copy borrow into the
    /// ring slot, valid only for that call.
    ///
    /// # Single-consumer contract
    ///
    /// The drain is single-consumer: do not re-enter `poll_batch`,
    /// [`Self::next_event`], [`Self::try_next_event`], [`Self::for_each`],
    /// or [`Self::for_each_scoped`] from inside `on_event`, and do not drive
    /// two drains on the same client concurrently. A reentrant or concurrent
    /// drain is rejected with [`PollOutcome::Busy`] (no events delivered)
    /// rather than blocking on the non-reentrant staging mutex, so a
    /// misuse fails fast instead of hard-hanging. The normal single-threaded
    /// drain always acquires uncontended and is unaffected.
    pub fn poll_batch(&self, mut on_event: impl FnMut(&StreamEvent)) -> PollOutcome {
        // `try_lock`, not `lock`: the staging mutex is non-reentrant, so a
        // user `on_event` callback that re-enters a drain method on this
        // client (or a second thread draining concurrently) would hard-hang
        // on a blocking acquire. The drain is single-consumer by contract,
        // so the happy path is always uncontended and acquires immediately;
        // contention can only mean a reentrant or concurrent drain, which is
        // reported as `Busy` rather than deadlocking. A poisoned mutex means
        // the producer side faulted (a machinery panic unwound while holding
        // the staging lock), so report the terminal disposition — `Failed`
        // when the I/O thread faulted — matching what `next_event` /
        // `try_next_event` surface, rather than decaying to a clean `Shutdown`.
        let mut guard = match self.poller_state.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return PollOutcome::Busy,
            Err(std::sync::TryLockError::Poisoned(_)) => return self.terminal_poll_outcome(),
        };
        // Arm the self-join guard only now that drain ownership is proven:
        // a caller driving `poll_batch` in its own loop and holding the
        // staging lock is the real consumer thread, so a `Drop` reached
        // from inside `on_event` detaches its join rather than blocking
        // this thread on its own termination. A reentrant/concurrent caller
        // that hit `Busy` above returned before this point and never claims
        // the role.
        self.record_drainer_and_pin();
        let Some(mut state) = guard.take() else {
            // The staging state was already dropped by a prior terminal drain.
            // Re-report the same terminal disposition (faulted vs clean) so a
            // repeated poll after shutdown keeps surfacing a fault rather than
            // decaying to a clean `Shutdown`.
            //
            // ponytail: on this state==None path `terminal_poll_outcome`'s
            // Acquire fence may pair with nothing if a DIFFERENT thread observed
            // the ring shutdown first (it dropped the staging state before its
            // own fence). This is the same abstract-machine-only window
            // `shutdown_outcome` already accepts for its state==None case:
            // benign on real hardware, and it needs a post-termination
            // cross-thread drain handoff no shipped consumer performs (the drain
            // is single-consumer). Left as-is to match that baseline.
            return self.terminal_poll_outcome();
        };

        // Drain anything buffered from a previous `next_event` call so
        // batch consumers see those events first.
        let mut delivered = 0usize;
        while let Some(event) = state.pending.pop_front() {
            if self.dispatch_caught(|| on_event(&event)).is_err() {
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
                        if self.dispatch_caught(|| on_event(event)).is_err() {
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
            // Terminal shutdown with nothing left to deliver: distinguish a
            // clean stop from an I/O-thread fault so a callback/binding drain
            // surfaces the failure instead of a graceful end of stream.
            None => self.terminal_poll_outcome(),
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
    /// been delivered. The returned [`PollOutcome`] is [`PollOutcome::Shutdown`]
    /// on a clean stop or [`PollOutcome::Failed`] when the I/O thread unwound,
    /// so a caller can distinguish a graceful end of stream from a dispatcher
    /// failure (the callback-drain analogue of the pull path's
    /// [`crate::streaming::StreamError::DispatcherFailed`]).
    ///
    /// Single-consumer: do not re-enter any drain method from inside
    /// `on_event`, and do not run a second drain on the same client
    /// concurrently. Such a reentrant or concurrent drain is rejected (see
    /// [`PollOutcome::Busy`]) rather than blocking the consumer thread.
    pub fn for_each(&self, on_event: impl FnMut(&StreamEvent)) -> PollOutcome {
        // Identity batch scope: each batch drain runs directly, with no
        // wrapping. The inter-batch wait is the same three-phase strategy
        // `for_each_scoped` applies.
        self.for_each_scoped(on_event, |drain| drain())
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
    /// [`PollOutcome::Shutdown`] (clean stop) or [`PollOutcome::Failed`]
    /// (the I/O thread unwound), identical to [`Self::for_each`], and
    /// returns the terminal outcome so a caller can distinguish a graceful
    /// end of stream from a dispatcher failure.
    ///
    /// Single-consumer: do not re-enter any drain method from inside
    /// `on_event`, and do not run a second drain on the same client
    /// concurrently. A reentrant or concurrent drain is rejected (see
    /// [`PollOutcome::Busy`]) rather than blocking the consumer thread.
    pub fn for_each_scoped<S>(
        &self,
        mut on_event: impl FnMut(&StreamEvent),
        mut scope: S,
    ) -> PollOutcome
    where
        S: FnMut(&mut dyn FnMut() -> PollOutcome) -> PollOutcome,
    {
        // The drain owner + CPU pin are recorded inside `poll_batch`, after
        // the staging lock is acquired, so the identity reflects the proven
        // drainer rather than whichever thread merely entered this loop.
        let waiter = self.wait_strategy;
        loop {
            // Drain one batch inside the caller's scope. `on_event` fires
            // once per event, exactly as in `for_each`; the scope only
            // brackets the batch, it does not change delivery cardinality.
            let outcome = scope(&mut || self.poll_batch(&mut on_event));
            match outcome {
                // Return the terminal outcome so the caller can tell a clean
                // shutdown from an I/O-thread fault (`Failed`).
                terminal @ (PollOutcome::Shutdown | PollOutcome::Failed) => return terminal,
                // Empty ring or a transient busy (another drain held the
                // staging state) — wait OUTSIDE the scope so a held resource
                // (e.g. the GIL) is released across the idle wait, then retry.
                // `Busy` cannot arise from this loop's own poll (it holds no
                // lock when it polls); it is handled defensively in case an
                // `on_event` callback re-entered a drain.
                PollOutcome::Drained(0) | PollOutcome::Busy => {
                    waiter.wait_for(0);
                }
                PollOutcome::Drained(_) => {}
            }
        }
    }

    /// Drain events through `on_event`, applying a caller-supplied
    /// [`crate::streaming::wait::WaitStrategy`] on each momentarily
    /// empty ring instead of the default low-latency wait.
    ///
    /// This is the Rust-native bring-your-own-strategy escape hatch: the
    /// default wait ([`crate::streaming::wait::BusySpinWithSpinLoopHint`]-style
    /// spin) suits real-time market data, but a Rust caller with an
    /// exotic backoff (e.g. an adaptive PID-controlled park, or a
    /// strategy that coordinates with another subsystem) can supply any
    /// `W: WaitStrategy` here. Use a strategy from
    /// [`crate::streaming::wait`] (e.g.
    /// [`crate::streaming::wait::BusySpin`]) or implement the trait on
    /// your own type.
    ///
    /// `W` is monomorphised into the loop, so the per-poll cost is the
    /// caller's `wait_for` body with no indirection. Delivery semantics
    /// match [`Self::for_each`]: `on_event` fires exactly once per event
    /// and the loop returns on terminal shutdown after the ring drains.
    ///
    /// # Why Rust-only
    ///
    /// `wait_for` fires on every ring-empty poll on the hot path. Routing
    /// that per-poll callback across the C ABI, the CPython interpreter
    /// lock, or the JavaScript event loop would add call-boundary
    /// overhead to the single tightest loop in the consumer — a latency
    /// regression, not a tuning knob. The bindings therefore run the
    /// fixed low-latency wait with no override.
    pub fn for_each_with_wait_strategy<W, F>(&self, mut on_event: F, strategy: W) -> PollOutcome
    where
        W: crate::streaming::wait::WaitStrategy,
        F: FnMut(&StreamEvent),
    {
        // The drain owner + CPU pin are recorded inside `poll_batch`, after
        // the staging lock is acquired, so the identity reflects the proven
        // drainer rather than whichever thread merely entered this loop.
        loop {
            match self.poll_batch(&mut on_event) {
                // Return the terminal outcome so the caller can tell a clean
                // shutdown from an I/O-thread fault (`Failed`).
                terminal @ (PollOutcome::Shutdown | PollOutcome::Failed) => return terminal,
                // `Busy` cannot arise from this loop's own poll (it holds no
                // lock when it polls); back off and retry defensively in case
                // an `on_event` callback re-entered a drain.
                PollOutcome::Drained(0) | PollOutcome::Busy => strategy.wait_for(0),
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
    /// # Overlapping subscriptions
    ///
    /// Do not hold both a full-stream subscription and a per-contract
    /// subscription of the same kind for the same security type at once —
    /// for example a full-stream option-trade subscription
    /// ([`protocol::SecTypeExt::full_trades`] on [`SecType::Option`]) together
    /// with a per-contract trade subscription
    /// ([`Contract::trade`]) on an individual option. The server broadcasts a
    /// contract that matches both on each feed independently, and this client
    /// does not de-duplicate across the two scopes, so every event for a
    /// matching contract is delivered twice. Pick one scope per kind and
    /// security type.
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
        // Untrack-on-unsubscribe is terminal: send, then remove the tracked
        // entry and evict the in-flight correlation for this identity for the
        // same reason as the per-contract unsubscribe — the removed entry is
        // no longer live, so a late rejection of its subscribe must not
        // survive to untrack a future re-subscribe of the same
        // `(kind, sec_type)` by value.
        if unsubscribe {
            self.send_cmd(IoCommand::WriteFrame { code, payload })?;
            tracing::debug!(
                req_id,
                sec_type = ?sec_type,
                unsubscribe,
                "sent full-stream unsubscribe frame"
            );
            self.active_full_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|(k, s)| !(*k == kind_for_track && *s == sec_type));
            io_loop::evict_pending_for_identity(
                &mut self
                    .pending_subs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                &protocol::PendingSub::Full(kind_for_track, sec_type),
            );
            return Ok(());
        }

        // Track for reconnection BEFORE the wire send, same REQ_RESPONSE
        // ordering hazard as the per-contract path: a rejection processed
        // before the pending entry exists would leave the rejected
        // subscription tracked and replayed forever. Register first, roll
        // both registrations back on a send failure.
        let newly_tracked = {
            let mut subs = self
                .active_full_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if subs
                .iter()
                .any(|(k, s)| *k == kind_for_track && *s == sec_type)
            {
                false
            } else {
                // Idempotent by `(kind, sec_type)`: a repeated full-stream
                // subscribe must not accumulate duplicate tracked entries that
                // would replay the same subscribe frame multiple times on
                // reconnect.
                subs.push((kind_for_track, sec_type));
                true
            }
        };

        // Record the pending full-stream subscribe by `req_id` so a server
        // rejection drops exactly this entry from the replay set. Only the
        // subscribe that actually added the tracked entry may carry an
        // untrack-capable correlation: an unsubscribe is handled terminally
        // above and a duplicate subscribe shares the one live entry, so letting
        // its rejection untrack by value would drop the live subscription.
        if newly_tracked {
            let mut pending = self
                .pending_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            io_loop::evict_stale_pending(&mut pending);
            pending.insert(
                req_id,
                io_loop::PendingSubEntry {
                    sub: protocol::PendingSub::Full(kind_for_track, sec_type),
                    recorded_at: std::time::Instant::now(),
                },
            );
        }

        if let Err(e) = self.send_cmd(IoCommand::WriteFrame { code, payload }) {
            // The frame never reached the I/O thread, so no `REQ_RESPONSE`
            // will reconcile the optimistic registration above; undo it.
            if newly_tracked {
                self.pending_subs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&req_id);
                self.active_full_subs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .retain(|(k, s)| !(*k == kind_for_track && *s == sec_type));
            }
            return Err(e);
        }
        tracing::debug!(
            req_id,
            sec_type = ?sec_type,
            unsubscribe,
            "sent full-stream subscription frame"
        );
        Ok(())
    }

    /// Per-contract subscribe wire emission.
    fn send_sub_contract(&self, kind: SubscriptionKind, contract: &Contract) -> Result<(), Error> {
        contract.validate()?;
        self.check_connected()?;

        let req_id = wire_req_id(self.next_req_id.fetch_add(1, Ordering::Relaxed));
        let payload = build_subscribe_payload(req_id, contract)?;
        let code = kind.subscribe_code();

        // Track for reconnection BEFORE the wire send. The server's
        // `REQ_RESPONSE` is correlated by `req_id` on the I/O thread; if a
        // rejection is processed before the pending entry and tracked sub
        // exist, it finds nothing to untrack and the rejected subscription
        // stays in `active_subs` to be replayed forever on reconnect.
        // Registering first closes that window; a send failure rolls both
        // registrations back, since no response will arrive for a frame that
        // never left.
        let newly_tracked = {
            let mut subs = self
                .active_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Idempotent by `(kind, contract)`: a repeated per-contract
            // subscribe must not accumulate duplicate tracked entries that
            // would replay the same subscribe frame multiple times on
            // reconnect.
            if subs.iter().any(|(k, c)| *k == kind && c == contract) {
                false
            } else {
                subs.push((kind, contract.clone()));
                true
            }
        };

        // Record the in-flight subscribe keyed by its `req_id` so the I/O
        // thread can correlate the asynchronous server `REQ_RESPONSE` back to
        // this contract. A rejection then untracks exactly this entry rather
        // than leaving a permanently over-reported, forever-replayed sub.
        //
        // Only the subscribe that actually added the tracked entry may carry
        // an untrack-capable pending correlation. A repeated subscribe for an
        // already-tracked `(kind, contract)` shares one live entry; if a
        // duplicate's `REQ_RESPONSE` (for example `MaxStreamsReached`) could
        // untrack by value, it would drop the original live subscription and
        // silence its stream on the next reconnect. Skipping the pending
        // insert for a duplicate keeps the rejection from touching the live
        // entry.
        if newly_tracked {
            let mut pending = self
                .pending_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            io_loop::evict_stale_pending(&mut pending);
            pending.insert(
                req_id,
                io_loop::PendingSubEntry {
                    sub: protocol::PendingSub::Contract(kind, contract.clone()),
                    recorded_at: std::time::Instant::now(),
                },
            );
        }

        if let Err(e) = self.send_cmd(IoCommand::WriteFrame { code, payload }) {
            // The frame never reached the I/O thread, so no `REQ_RESPONSE`
            // will reconcile the optimistic registration above; undo it.
            if newly_tracked {
                self.pending_subs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&req_id);
                self.active_subs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .retain(|(k, c)| !(*k == kind && c == contract));
            }
            return Err(e);
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

        // Evict any in-flight correlation for this identity. The removed entry
        // is no longer live, so a late rejection of the subscribe that created
        // it must not survive to untrack a future re-subscribe of the same
        // `(kind, contract)` by value.
        {
            let mut pending = self
                .pending_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            io_loop::evict_pending_for_identity(
                &mut pending,
                &protocol::PendingSub::Contract(kind, contract.clone()),
            );
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
            return Err(Error::Stream {
                kind: crate::error::StreamErrorKind::Disconnected,
                message: "client is shut down".to_string(),
            });
        }
        if !self.authenticated.load(Ordering::Acquire) {
            return Err(Error::Stream {
                kind: crate::error::StreamErrorKind::Disconnected,
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
    /// - The user `handler` runs on the event-delivery thread,
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
        let pending_subs: Arc<Mutex<std::collections::HashMap<i32, io_loop::PendingSubEntry>>> =
            Arc::new(Mutex::new(std::collections::HashMap::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let panics = Arc::new(AtomicU64::new(0));
        let ring_cursors = Arc::new(RingCursors::new());
        let consumer_thread_id: Arc<Mutex<Option<ThreadId>>> = Arc::new(Mutex::new(None));
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
                    // The disruptor's consumer thread is the genuine drain
                    // owner in this harness; record its identity (idempotent
                    // refresh) the same way the production primitives do once
                    // they hold the staging lock.
                    if let Ok(mut id) = consumer_thread_id_cell.lock() {
                        let cur = thread::current().id();
                        if *id != Some(cur) {
                            *id = Some(cur);
                        }
                    }
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
            pending_subs,
            server_addr: "test://self-join".to_owned(),
            last_event_at_ns: Arc::new(AtomicI64::new(0)),
            connected_addr: Arc::new(Mutex::new("test://self-join".to_owned())),
            replay_burst_size: 50,
            replay_pace_ms: 0,
            dropped,
            panics,
            io_faulted: Arc::new(AtomicBool::new(false)),
            ring_cursors,
            ring_size,
            wait_strategy: ring::AdaptiveWaitStrategy::low_latency(),
            consumer_cpu: None,
            consumer_pinned_to: Mutex::new(None),
            consumer_thread_id,
            drained: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Test-only constructor for a [`StreamingClient`] already in the faulted
    /// terminal state: no I/O thread, an empty (shut-down) poller, and
    /// `io_faulted` set. A callback drain ([`Self::for_each`] /
    /// [`Self::poll_batch`]) therefore returns [`PollOutcome::Failed`] at once
    /// and a pull ([`Self::next_event`]) returns `DispatcherFailed`. The
    /// `authenticated` flag is `true` so that, ABSENT the fault, a binding's
    /// `is_streaming` would report healthy — the test proves the fault is what
    /// flips it. Exposed (not `#[cfg(test)]`) so the binding crates' tests can
    /// drive their real dispatcher loop over a faulted client with no network.
    #[doc(hidden)]
    pub fn for_io_fault_test() -> Arc<Self> {
        let (cmd_tx, _cmd_rx) = std_mpsc::sync_channel::<IoCommand>(CMD_CHANNEL_CAPACITY);
        let ring_size = ring::check_ring_size(64).expect("64 is a valid ring size");
        let io_faulted = Arc::new(AtomicBool::new(true));
        // Publish the fault with a Release fence, exactly as `IoLoopFaultGuard`
        // orders the store before the ring shutdown, so a drain's Acquire fence
        // observes it (matching the production memory-ordering discipline).
        std::sync::atomic::fence(Ordering::Release);
        Arc::new(StreamingClient {
            cmd_tx: Mutex::new(cmd_tx),
            io_handle: None,
            ping_handle: None,
            poller_state: Mutex::new(None),
            shutdown: Arc::new(AtomicBool::new(true)),
            authenticated: Arc::new(AtomicBool::new(true)),
            next_req_id: Arc::new(AtomicI64::new(1)),
            active_subs: Arc::new(Mutex::new(Vec::new())),
            active_full_subs: Arc::new(Mutex::new(Vec::new())),
            pending_subs: Arc::new(Mutex::new(std::collections::HashMap::new())),
            server_addr: "test://io-fault".to_owned(),
            last_event_at_ns: Arc::new(AtomicI64::new(0)),
            connected_addr: Arc::new(Mutex::new("test://io-fault".to_owned())),
            replay_burst_size: 50,
            replay_pace_ms: 0,
            dropped: Arc::new(AtomicU64::new(0)),
            panics: Arc::new(AtomicU64::new(0)),
            io_faulted,
            ring_cursors: Arc::new(RingCursors::new()),
            ring_size,
            wait_strategy: ring::AdaptiveWaitStrategy::low_latency(),
            consumer_cpu: None,
            consumer_pinned_to: Mutex::new(None),
            consumer_thread_id: Arc::new(Mutex::new(None)),
            drained: Arc::new(AtomicBool::new(false)),
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
            pending_subs: Arc::new(Mutex::new(std::collections::HashMap::new())),
            server_addr: "test://ring-occupancy".to_owned(),
            last_event_at_ns: Arc::new(AtomicI64::new(0)),
            connected_addr: Arc::new(Mutex::new("test://ring-occupancy".to_owned())),
            replay_burst_size: 50,
            replay_pace_ms: 0,
            dropped: Arc::new(AtomicU64::new(0)),
            panics: Arc::new(AtomicU64::new(0)),
            io_faulted: Arc::new(AtomicBool::new(false)),
            ring_cursors,
            ring_size,
            wait_strategy: ring::AdaptiveWaitStrategy::low_latency(),
            consumer_cpu: None,
            consumer_pinned_to: Mutex::new(None),
            consumer_thread_id: Arc::new(Mutex::new(None)),
            drained: Arc::new(AtomicBool::new(false)),
        };
        (client, producer)
    }

    /// The consumer thread id recorded by the drain paths, if any drain
    /// has run. `None` before the first `next_event` / `for_each*` /
    /// `poll_batch` call. Exists so a test can assert the `Drop`
    /// self-join guard is armed by a real drain path rather than only by
    /// the test-only `for_self_join_test` setter.
    #[cfg(test)]
    pub(in crate::fpss) fn recorded_consumer_thread_id(&self) -> Option<ThreadId> {
        self.consumer_thread_id
            .lock()
            .map(|id| *id)
            .unwrap_or_else(|poisoned| *poisoned.into_inner())
    }

    /// Send a command to the I/O thread over the bounded control channel.
    ///
    /// Uses a non-blocking `try_send` so a public `&self` caller is never
    /// parked behind a saturated channel while holding the command lock. A
    /// full channel and a hung-up I/O thread both map to a typed
    /// [`StreamErrorKind::Disconnected`] error: the command is reported to the
    /// caller, never silently dropped. A full channel means the application is
    /// issuing control-plane commands faster than the I/O thread can drain
    /// them; the caller can retry after the queue clears.
    pub(in crate::fpss) fn send_cmd(&self, cmd: IoCommand) -> Result<(), Error> {
        self.cmd_tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_send(cmd)
            .map_err(|e| match e {
                std_mpsc::TrySendError::Full(_) => Error::Stream {
                    kind: crate::error::StreamErrorKind::Disconnected,
                    message: format!(
                        "command queue full ({CMD_CHANNEL_CAPACITY} pending); \
                         the I/O thread is draining slower than commands arrive — retry shortly"
                    ),
                },
                std_mpsc::TrySendError::Disconnected(_) => Error::Stream {
                    kind: crate::error::StreamErrorKind::Disconnected,
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
/// * [`PollOutcome::Busy`] — the staging state was already held by another
///   in-flight drain (a callback re-entered the drain, or a second thread
///   drained concurrently). The call did no work; the in-flight drain owns
///   the events. Retry on a subsequent poll. The blocking
///   [`StreamingClient::for_each`] family never surfaces this from their own
///   loop — they hold no lock when they poll — so it only appears to a
///   caller driving [`StreamingClient::poll_batch`] in a way that re-enters
///   or races the drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PollOutcome {
    /// The available batch was drained into the closure. Carries the
    /// number of events delivered on this call (may be `0`).
    Drained(usize),
    /// Terminal: the session has shut down cleanly and the ring is fully
    /// drained. No further events will arrive.
    Shutdown,
    /// Terminal: the session ended because the FPSS I/O thread terminated
    /// abnormally (it panicked/unwound), not on a clean stop. The ring is
    /// fully drained and no further events will arrive. This is the
    /// callback-drain analogue of the pull path's
    /// [`crate::streaming::StreamError::DispatcherFailed`]: a consumer must
    /// treat it as a dispatcher failure rather than a graceful end of
    /// stream (e.g. flip a session to failed / surface an error) so an I/O
    /// fault is not silently seen as a normal shutdown.
    Failed,
    /// A concurrent or reentrant drain already holds the staging state.
    /// No events were delivered on this call; retry later. Distinct from
    /// `Drained(0)` (live but momentarily empty) so a reentrant-drain
    /// caller can tell "another drain owns this" from "nothing right now".
    Busy,
}

/// Owning iterator for an [`StreamingClient`] reference.
///
/// Yields one [`StreamEvent`] per call to [`Iterator::next`] by repeatedly
/// invoking [`StreamingClient::next_event`]; surfaces typed errors as
/// `Some(Err(_))` and terminates with `None` on clean shutdown.
impl Iterator for &StreamingClient {
    type Item = Result<StreamEvent, StreamError>;

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
        let consumer_id = self
            .consumer_thread_id
            .lock()
            .map(|id| *id)
            .unwrap_or_else(|poisoned| *poisoned.into_inner());

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

    /// The default builder threads the fixed low-latency wait strategy
    /// into the connect args, and the ring poller builds with it.
    #[test]
    fn builder_threads_low_latency_wait_strategy() {
        use crate::fpss::ring::RingCursors;
        use std::sync::Arc;

        let creds = Credentials::new("user", "pw");
        let hosts: Vec<(String, u16)> = vec![("nj-a.thetadata.us".to_owned(), 20000)];

        let args = StreamingClientBuilder::new(&creds, &hosts).into_args();
        // The poller builds with the connect args' wait strategy; a
        // successful build (matching `RingEvent` / `SingleProducerBarrier`
        // type) confirms the strategy is wired through.
        let (_p, poller) =
            io_loop::build_poller_producer(64, Arc::new(RingCursors::new()), args.wait_strategy);
        let _poller: EventPoller<ring::RingEvent, SingleProducerBarrier> = poller;
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

    /// A duplicate subscribe followed by a server rejection of the duplicate
    /// must not drop the live original.
    ///
    /// One tracked `(kind, contract)` entry is shared by every subscribe for
    /// it; only the subscribe that created the entry carries an
    /// untrack-capable pending correlation. So when the server rejects the
    /// duplicate (here `MaxStreamsReached`, the realistic trigger: the first
    /// subscribe already used the contract's stream slot), the rejection finds
    /// no pending entry to act on and the still-live original stays in
    /// `active_subs` — and therefore in the reconnect-replay set, which is a
    /// clone of `active_subs`.
    #[test]
    fn duplicate_subscribe_then_reject_keeps_live_sub() {
        use super::io_loop::apply_req_response_for_test;
        use crate::tdbe::types::enums::StreamResponseType;

        let client = StreamingClient::for_self_join_test(
            0,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            |_event| {},
        );

        // First subscribe is accepted and owns the tracked entry (and the only
        // untrack-capable pending correlation). req_id counter starts at 1, so
        // this allocates req_id 1.
        let contract = Contract::stock("AAPL");
        client
            .subscribe(contract.clone().trade())
            .expect("first subscribe");
        // Duplicate subscribe: shares the live tracked entry, registers no
        // untrack-capable pending correlation. This allocates req_id 2.
        client
            .subscribe(contract.clone().trade())
            .expect("duplicate subscribe");

        // Exactly one untrack-capable correlation exists despite two subscribes.
        assert_eq!(
            client
                .pending_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            1,
            "a duplicate subscribe must not register a second untrack-capable correlation"
        );

        // The server rejects the duplicate's req_id (2). The original (req_id
        // 1) is live and must survive.
        apply_req_response_for_test(
            &client.pending_subs,
            &client.active_subs,
            &client.active_full_subs,
            2,
            StreamResponseType::MaxStreamsReached,
        );

        let tracked = client.active_subscriptions();
        assert_eq!(
            tracked.len(),
            1,
            "rejecting a duplicate must leave the live original tracked, got {tracked:?}"
        );
        assert!(
            tracked.iter().any(|(_, c)| *c == contract),
            "the live original must remain in active_subscriptions(), got {tracked:?}"
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

    /// Subscribe, unsubscribe, then re-subscribe the same contract, all before
    /// the first subscribe's `REQ_RESPONSE` lands; a late rejection of that
    /// superseded first request must not drop the re-subscribed live entry.
    ///
    /// The first subscribe owns the correlation for its `req_id`; the
    /// unsubscribe both removes the tracked entry and evicts that correlation,
    /// so when the re-subscribe re-adds the entry it owns a fresh correlation
    /// under a new `req_id`. A server rejection of the obsolete first `req_id`
    /// then finds no resident correlation and is a no-op, leaving the live
    /// re-subscribed entry tracked and in the reconnect-replay set (which is a
    /// clone of `active_subs`).
    #[test]
    fn unsub_resub_then_reject_superseded_keeps_live_sub() {
        use super::io_loop::apply_req_response_for_test;
        use crate::tdbe::types::enums::StreamResponseType;

        let client = StreamingClient::for_self_join_test(
            0,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            |_event| {},
        );

        // req_id counter starts at 1: subscribe -> 1, unsubscribe -> 2,
        // re-subscribe -> 3.
        let contract = Contract::stock("AAPL");
        client
            .subscribe(contract.clone().trade())
            .expect("first subscribe");
        client
            .unsubscribe(contract.clone().trade())
            .expect("unsubscribe");
        client
            .subscribe(contract.clone().trade())
            .expect("re-subscribe");

        // Exactly one resident correlation: the unsubscribe evicted the first
        // subscribe's (req_id 1) correlation, and the re-subscribe registered a
        // fresh one (req_id 3).
        assert_eq!(
            client
                .pending_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            1,
            "unsubscribe must evict the superseded correlation; only the \
             re-subscribe's correlation may remain"
        );

        // The server rejects the obsolete first req_id (1). With its
        // correlation already evicted this is a no-op, not a value match that
        // would drop the live re-subscribed entry.
        apply_req_response_for_test(
            &client.pending_subs,
            &client.active_subs,
            &client.active_full_subs,
            1,
            StreamResponseType::MaxStreamsReached,
        );

        let tracked = client.active_subscriptions();
        assert_eq!(
            tracked.len(),
            1,
            "rejecting a superseded subscribe must leave the re-subscribed entry \
             tracked, got {tracked:?}"
        );
        assert!(
            tracked.iter().any(|(_, c)| *c == contract),
            "the re-subscribed contract must remain in active_subscriptions() so \
             it is replayed on reconnect, got {tracked:?}"
        );

        client.shutdown();
    }

    /// The full-stream analogue: subscribe, unsubscribe, re-subscribe a
    /// full-stream `(kind, sec_type)`, then reject the superseded first
    /// request — the re-subscribed full-stream entry must stay tracked.
    #[test]
    fn full_unsub_resub_then_reject_superseded_keeps_live_sub() {
        use super::io_loop::apply_req_response_for_test;
        use crate::tdbe::types::enums::StreamResponseType;

        let client = StreamingClient::for_self_join_test(
            0,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            |_event| {},
        );

        // subscribe -> 1, unsubscribe -> 2, re-subscribe -> 3.
        client
            .subscribe(SecType::Stock.full_trades())
            .expect("first full subscribe");
        client
            .unsubscribe(SecType::Stock.full_trades())
            .expect("full unsubscribe");
        client
            .subscribe(SecType::Stock.full_trades())
            .expect("re-subscribe full");

        assert_eq!(
            client
                .pending_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            1,
            "full-stream unsubscribe must evict the superseded correlation"
        );

        apply_req_response_for_test(
            &client.pending_subs,
            &client.active_subs,
            &client.active_full_subs,
            1,
            StreamResponseType::MaxStreamsReached,
        );

        let tracked = client.active_full_subscriptions();
        assert_eq!(
            tracked.len(),
            1,
            "rejecting a superseded full-stream subscribe must leave the \
             re-subscribed entry tracked, got {tracked:?}"
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
    use super::{HarnessPublishMode, PollOutcome, StreamControl, StreamError, StreamingClient};

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

    /// A real drain path arms the `Drop` self-join guard.
    ///
    /// In production the only thing that records the consumer thread id
    /// is `record_drainer_and_pin`, called by the drain primitives
    /// (`poll_batch` / `try_next_event_internal`) once they hold the
    /// staging lock. If that recording regresses, `Drop` reads a `None`
    /// thread id, never
    /// detects a `Drop` running on the consumer thread, and joins the
    /// I/O handle inline — a self-join deadlock when a user callback
    /// drops the last client handle. This pins the recording to a real
    /// drain call rather than the test-only setter: before any drain the
    /// id is unset; after `poll_batch` on this thread it equals this
    /// thread, so the guard's `consumer_id == Some(cur)` branch fires.
    #[test]
    fn drain_path_records_consumer_thread_id_for_self_join_guard() {
        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);

        assert_eq!(
            client.recorded_consumer_thread_id(),
            None,
            "no drain has run yet; the guard must be unarmed"
        );

        assert!(
            publish_one(&mut producer),
            "fresh ring must accept a publish"
        );
        let mut delivered = 0usize;
        let _ = client.poll_batch(|_event| delivered += 1);

        assert_eq!(
            client.recorded_consumer_thread_id(),
            Some(std::thread::current().id()),
            "poll_batch must record the draining thread so Drop detects a self-join"
        );
    }

    /// The bring-your-own-strategy drain is reachable through the
    /// crate-owned wait-strategy re-export, so a Rust caller never names
    /// the underlying ring crate. Dropping the producer terminates the
    /// drain; `BusySpin`'s `wait_for` is a no-op, so the loop spins only
    /// until the ring reports shutdown.
    #[test]
    fn for_each_with_wait_strategy_accepts_crate_owned_preset() {
        use crate::streaming::wait::BusySpin;

        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);
        for _ in 0..3 {
            assert!(
                publish_one(&mut producer),
                "fresh ring must accept a publish"
            );
        }
        // Drop the producer so the poller observes terminal shutdown once
        // the three published events drain, and the loop returns.
        drop(producer);

        let mut delivered = 0usize;
        client.for_each_with_wait_strategy(|_event| delivered += 1, BusySpin);

        assert_eq!(
            delivered, 3,
            "every published event must be delivered before shutdown returns"
        );
    }

    /// A faulted I/O thread — the `io_loop` unwound and its fault guard set
    /// `io_faulted` before the ring published its shutdown sequence — must
    /// surface through the CALLBACK drain path, not read as a clean shutdown.
    /// `for_each` returns [`PollOutcome::Failed`] and the pull `next_event`
    /// returns [`StreamError::DispatcherFailed`], so both paths agree on the
    /// fault. Mirrors the pull-path disposition in `shutdown_outcome`; without
    /// it a callback/binding dispatcher would see a graceful end of stream on
    /// an I/O-thread panic and stay "streaming".
    #[test]
    fn faulted_io_thread_surfaces_through_callback_drain_not_clean_shutdown() {
        let (client, producer) = StreamingClient::for_ring_occupancy_test(64);
        // Simulate the io_loop fault guard: the thread unwound and flagged it.
        // Release pairs with the drain's Acquire fence, exactly as the real
        // `IoLoopFaultGuard` orders the store before the ring shutdown.
        client
            .io_faulted
            .store(true, std::sync::atomic::Ordering::Release);
        // Drop the producer so the poller observes terminal ring shutdown.
        drop(producer);

        // Blocking callback drain: faulted, not a clean `Shutdown`.
        assert_eq!(
            client.for_each(|_event| {}),
            PollOutcome::Failed,
            "for_each must return PollOutcome::Failed on a faulted io thread"
        );

        // Pull drain surfaces DispatcherFailed, proving both paths agree.
        assert!(
            matches!(client.next_event(), Err(StreamError::DispatcherFailed(_))),
            "next_event must surface DispatcherFailed on a faulted io thread"
        );
    }

    /// The clean-shutdown path is unchanged: with NO fault flag, `for_each`
    /// returns [`PollOutcome::Shutdown`] and `next_event` returns `Ok(None)` —
    /// no false `Failed` / `DispatcherFailed`.
    #[test]
    fn clean_shutdown_stays_clean_through_callback_drain() {
        let (client, producer) = StreamingClient::for_ring_occupancy_test(64);
        drop(producer);

        assert_eq!(
            client.for_each(|_event| {}),
            PollOutcome::Shutdown,
            "a clean shutdown must not read as Failed"
        );
        assert!(
            matches!(client.next_event(), Ok(None)),
            "a clean shutdown must surface as Ok(None), not DispatcherFailed"
        );
    }

    /// The blocking `next_event` drain also arms the guard, covering the
    /// post-lock recording shared by every blocking drain.
    #[test]
    fn next_event_records_consumer_thread_id_for_self_join_guard() {
        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);

        assert_eq!(client.recorded_consumer_thread_id(), None);

        assert!(
            publish_one(&mut producer),
            "fresh ring must accept a publish"
        );
        let event = client
            .next_event()
            .expect("staging mutex must not be poisoned");
        assert!(event.is_some(), "one event must be delivered");

        assert_eq!(
            client.recorded_consumer_thread_id(),
            Some(std::thread::current().id()),
            "next_event must record the draining thread so Drop detects a self-join"
        );
    }

    /// A callback that re-enters `poll_batch` on the same client must get a
    /// typed `Busy` outcome rather than hard-hanging on the non-reentrant
    /// staging mutex. The drain is single-consumer by contract; the test is
    /// timeout-bounded by the suite wrapper, so a regression that restored
    /// the blocking acquire would hang the process and be killed, not pass.
    #[test]
    fn reentrant_poll_batch_returns_busy_not_deadlock() {
        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);
        assert!(
            publish_one(&mut producer),
            "fresh ring must accept a publish"
        );

        let mut reentrant_outcome: Option<PollOutcome> = None;
        let outer = client.poll_batch(|_event| {
            // Re-enter the drain from inside the callback. The outer drain
            // already holds the staging state, so this must fail fast.
            if reentrant_outcome.is_none() {
                reentrant_outcome = Some(client.poll_batch(|_| {}));
            }
        });

        assert_eq!(
            outer,
            PollOutcome::Drained(1),
            "the outer drain still delivers its event normally"
        );
        assert_eq!(
            reentrant_outcome,
            Some(PollOutcome::Busy),
            "a reentrant poll_batch must report Busy instead of deadlocking"
        );
    }

    /// A concurrent drain attempt that FAILS to acquire the staging lock
    /// (`Busy`) must NOT become the recorded consumer-thread identity; the
    /// thread that actually holds the lock and drains is the one recorded,
    /// so `Drop`'s self-join detector tracks the real drainer.
    ///
    /// This is the regression guard for the identity-claim defect: the
    /// recording used to be armed at drain entry, before the `try_lock`, so
    /// the first thread to enter could permanently claim the role even if
    /// its drain then failed. Here a losing thread (`B`) attempts the drain
    /// while the winning thread (`A`) holds the lock inside its callback; A
    /// must be the recorded identity, never B.
    #[test]
    fn failed_concurrent_drain_does_not_claim_consumer_identity() {
        use std::sync::Barrier;

        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);
        // Two events: one drained by A inside the callback window, one left
        // so A's outer drain reports a delivery.
        assert!(publish_one(&mut producer), "fresh ring must accept publish");
        assert!(publish_one(&mut producer), "fresh ring must accept publish");

        let client = Arc::new(client);
        let a_id = std::thread::current().id();

        // Gate B's attempt to occur strictly while A holds the staging lock,
        // and gate A's release until B has finished its failed attempt.
        let b_may_try = Arc::new(Barrier::new(2));
        let b_done = Arc::new(Barrier::new(2));

        let b_outcome = {
            let client_b = Arc::clone(&client);
            let b_may_try = Arc::clone(&b_may_try);
            let b_done = Arc::clone(&b_done);
            std::thread::Builder::new()
                .name("drain-loser-B".to_owned())
                .spawn(move || {
                    // Wait until A is inside its callback holding the lock.
                    b_may_try.wait();
                    // This must fail fast with Busy (A holds the staging lock).
                    let outcome = client_b.poll_batch(|_| {});
                    // Snapshot the recorded identity right after B's failed
                    // attempt: with the bug, B would have claimed it here.
                    let recorded_after_b = client_b.recorded_consumer_thread_id();
                    b_done.wait();
                    (outcome, recorded_after_b, std::thread::current().id())
                })
                .expect("spawn B")
        };

        // A is the real drainer. Inside the callback the staging lock is held,
        // so this is the deterministic window for B's losing attempt.
        let mut released_b = false;
        let outer = client.poll_batch(|_event| {
            if !released_b {
                released_b = true;
                b_may_try.wait(); // let B attempt now
                b_done.wait(); // wait for B's failed attempt to complete
            }
        });

        let (b_poll, recorded_after_b, b_id) = b_outcome.join().expect("join B");

        assert_eq!(
            b_poll,
            PollOutcome::Busy,
            "B's concurrent drain must report Busy (A holds the staging lock)"
        );
        assert_ne!(a_id, b_id, "A and B must be distinct threads for this test");
        assert_eq!(
            recorded_after_b,
            Some(a_id),
            "the failed concurrent attempt (B) must NOT claim the consumer identity; \
             the real drainer (A) must be recorded"
        );
        assert_ne!(
            recorded_after_b,
            Some(b_id),
            "B failed to drain and must never be recorded as the consumer thread"
        );
        // A delivered at least its first event; the recorded identity stays A.
        assert!(
            matches!(outer, PollOutcome::Drained(_)),
            "A's drain delivers normally, got {outer:?}"
        );
        assert_eq!(
            client.recorded_consumer_thread_id(),
            Some(a_id),
            "after both drains, the recorded consumer is the real drainer A"
        );
    }

    /// Finding #3: when drain ownership moves to a different thread, the
    /// consumer-CPU pin must follow the NEW drainer rather than staying
    /// stuck on the original thread. The one-shot pin guard this replaces
    /// pinned exactly once for the life of the client, so a handoff left
    /// the new drainer inheriting the old thread's core binding.
    ///
    /// Uses the `affinity` test seams (a high, almost-certainly-absent
    /// core id so the real `set_for_current` no-ops, with the attempt
    /// counted regardless) to assert the pin path runs once per distinct
    /// drainer: once on this thread, NOT again on a repeat call from the
    /// same thread, then AGAIN once ownership moves to a spawned thread.
    #[test]
    fn pin_follows_drain_owner_across_handoff() {
        use std::sync::atomic::Ordering;

        // `PIN_ATTEMPTS` is a process-global seam; serialise this test
        // against any other test that resets/reads it.
        let _guard = pin_seam_guard();

        let (mut client, _producer) = StreamingClient::for_ring_occupancy_test(64);
        // Configure a consumer core so the pin path is live. The id is
        // deliberately out of range so the OS call is a graceful no-op on
        // CI; the attempt is still counted by the seam.
        client.consumer_cpu = Some(4096);

        super::affinity::PIN_ATTEMPTS.store(0, Ordering::Relaxed);

        // First drive on THIS thread pins once and records this thread as
        // the pinned target.
        client.record_drainer_and_pin();
        let this_thread = std::thread::current().id();
        assert_eq!(
            super::affinity::PIN_ATTEMPTS.load(Ordering::Relaxed),
            1,
            "the first drive must pin the consumer thread exactly once"
        );
        assert_eq!(
            *client.consumer_pinned_to.lock().unwrap(),
            Some(this_thread),
            "the pin must target the first drainer"
        );

        // A repeat drive on the SAME thread must NOT re-pin: steady state
        // on the single-consumer path keeps the affinity syscall off the
        // per-event path.
        client.record_drainer_and_pin();
        assert_eq!(
            super::affinity::PIN_ATTEMPTS.load(Ordering::Relaxed),
            1,
            "an unchanged drainer must not re-pin"
        );

        // Ownership moves to a different thread: the pin must follow it.
        let client = Arc::new(client);
        let handoff_id = {
            let client = Arc::clone(&client);
            std::thread::Builder::new()
                .name("drain-handoff".to_owned())
                .spawn(move || {
                    client.record_drainer_and_pin();
                    std::thread::current().id()
                })
                .expect("spawn handoff drainer")
                .join()
                .expect("join handoff drainer")
        };

        assert_ne!(
            handoff_id, this_thread,
            "the handoff thread must be distinct for this test"
        );
        assert_eq!(
            super::affinity::PIN_ATTEMPTS.load(Ordering::Relaxed),
            2,
            "a drain-owner handoff must re-pin the new drainer (not stay stuck \
             on the stale thread)"
        );
        assert_eq!(
            *client.consumer_pinned_to.lock().unwrap(),
            Some(handoff_id),
            "the pin must now target the new drainer"
        );
        assert_eq!(
            client.recorded_consumer_thread_id(),
            Some(handoff_id),
            "the recorded drain owner must also be the new thread"
        );
    }

    /// Serialises tests that touch the process-global `PIN_ATTEMPTS`
    /// affinity seam so concurrent runs never observe each other's writes.
    fn pin_seam_guard() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The real drainer's `Drop` detaches the join: when `Drop` runs on the
    /// recorded consumer thread, the I/O / ping handles are joined on a
    /// detached helper rather than inline, and `drained` flips once that
    /// helper completes. Pairs with the identity guard above — `Drop` must
    /// observe the TRUE drainer so a callback-driven drop never self-joins.
    #[test]
    fn drop_on_recorded_drainer_detaches_join() {
        let client = StreamingClient::for_self_join_test(
            4,
            64,
            HarnessPublishMode::BlockingPublish,
            None,
            |_event| {},
        );
        // The harness's dispatcher-thread consumer records itself as the
        // drain owner as it processes the burst. Wait until that identity is
        // present so the Drop below exercises the detach path deterministically.
        let drained = client.drained_flag();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while client.recorded_consumer_thread_id().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "consumer identity must be recorded by the dispatcher consumer"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let recorded = client
            .recorded_consumer_thread_id()
            .expect("identity recorded");
        // Sanity: the recorded drainer is the dispatcher thread, not this
        // test thread (this thread never drove a drain).
        assert_ne!(
            recorded,
            std::thread::current().id(),
            "the recorded drainer is the dispatcher consumer, not the test thread"
        );

        // Drop the only handle. The detach helper joins the I/O thread and
        // flips `drained`; poll for that quiescence (bounded).
        drop(client);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !drained.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "Drop's detach helper must flip drained for the real drainer"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// A callback that re-enters the event-at-a-time drain must get a typed
    /// `ReentrantDrain` error rather than hard-hanging. Driving the reentry
    /// through `poll_batch`'s callback exercises the shared staging-mutex
    /// guard that both drain families hold.
    #[test]
    fn reentrant_next_event_returns_typed_error_not_deadlock() {
        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);
        assert!(
            publish_one(&mut producer),
            "fresh ring must accept a publish"
        );

        let mut next_event_err: Option<StreamError> = None;
        let mut try_next_err: Option<StreamError> = None;
        client.poll_batch(|_event| {
            if next_event_err.is_none() {
                next_event_err = client.next_event().err();
                try_next_err = client.try_next_event().err();
            }
        });

        assert!(
            matches!(next_event_err, Some(StreamError::ReentrantDrain(_))),
            "a reentrant next_event must return ReentrantDrain, got {next_event_err:?}"
        );
        assert!(
            matches!(try_next_err, Some(StreamError::ReentrantDrain(_))),
            "a reentrant try_next_event must return ReentrantDrain, got {try_next_err:?}"
        );
    }

    /// The single-consumer happy path is unchanged: an uncontended
    /// `poll_batch` still acquires the staging mutex and delivers every
    /// published event, exactly as before the reentrancy guard.
    #[test]
    fn uncontended_poll_batch_happy_path_unchanged() {
        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);
        for _ in 0..5 {
            assert!(publish_one(&mut producer), "ring must accept the publish");
        }

        let mut delivered = 0usize;
        let outcome = client.poll_batch(|_event| delivered += 1);

        assert_eq!(outcome, PollOutcome::Drained(5));
        assert_eq!(delivered, 5);
    }

    /// A panic raised inside the caller-supplied `scope` closure propagates
    /// OUT of `for_each_scoped` rather than being swallowed. This is the
    /// load-bearing precondition for the callback dispatcher body's
    /// `catch_unwind` (see `Client::start_streaming_scoped`): the Python
    /// client wraps each batch drain in `Python::attach`, which can panic
    /// during interpreter finalization. If `for_each_scoped` swallowed that
    /// panic the dispatcher would appear to end cleanly and leave the session
    /// `Running` behind a dead thread; because it propagates, the body's
    /// `catch_unwind` observes it and flips the session to `Failed`.
    #[test]
    fn scope_closure_panic_propagates_out_of_for_each_scoped() {
        let (client, mut producer) = StreamingClient::for_ring_occupancy_test(64);
        // One event so the first batch drain has work; the scope panics on its
        // first (and only) invocation, before the loop could spin on an empty
        // ring, so this returns promptly via the unwind rather than hanging.
        assert!(publish_one(&mut producer), "fresh ring must accept publish");

        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.for_each_scoped(
                |_event| {},
                |_drain: &mut dyn FnMut() -> PollOutcome| -> PollOutcome {
                    panic!("scope boom");
                },
            )
        }));
        std::panic::set_hook(prev_hook);

        let payload = result.expect_err(
            "a panic in the scope closure must unwind out of for_each_scoped, not be swallowed",
        );
        let msg = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
        assert_eq!(
            msg,
            Some("scope boom"),
            "the propagated payload must be the scope closure's own panic",
        );
    }
}
