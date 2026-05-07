//! FPSS (Feed Processing Streaming Server) real-time streaming client.
//!
//! See ADR-001 (`docs/architecture/ADR-001-java-terminal-parity.md`) for the
//! Java terminal parity reverse-engineering source.
//!
//! # Architecture
//!
//! The FPSS protocol provides real-time market data over a custom TLS/TCP
//! binary protocol. The client runs:
//!
//! 1. A TLS connection to one of 4 FPSS servers (NJ-A/NJ-B, ports 20000/20001)
//! 2. An authentication handshake (email + password over the wire)
//! 3. A heartbeat thread sending PING every 100ms
//! 4. A reader thread dispatching incoming frames to callbacks
//! 5. Automatic reconnection on disconnect (except for permanent errors)
//!
//! # Fully synchronous -- no tokio in the FPSS path
//!
//! This module is 100% blocking I/O on `std::thread`. No tokio, no async, no
//! `.await` anywhere. The pipeline is:
//!
//! ```text
//! std::thread (blocking TLS read) -> LMAX Disruptor ring -> user's FnMut(&FpssEvent) callback
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! # use thetadatadx::fpss::{FpssClient, FpssConnectArgs, FpssData, FpssEvent};
//! # use thetadatadx::auth::Credentials;
//! # fn example() -> Result<(), thetadatadx::error::Error> {
//! let creds = Credentials::new("user@example.com", "pw");
//! let hosts = thetadatadx::config::DirectConfig::production().fpss.hosts;
//! let args = FpssConnectArgs::new(&creds, &hosts);
//! let client = FpssClient::connect(args, |event: &FpssEvent| {
//!     // Runs on the Disruptor consumer thread -- keep it fast.
//!     // Push to your own queue for heavy processing.
//!     match event {
//!         FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. }) => {
//!             let _root = &contract.symbol; // symbol / option root
//!             let _ = (bid, ask); // f64 prices
//!         }
//!         FpssEvent::Data(FpssData::Trade { contract, price, size, .. }) => {
//!             let _root = &contract.symbol;
//!             let _ = (price, size);
//!         }
//!         FpssEvent::Control(_) => { /* lifecycle */ }
//!         _ => {}
//!     }
//! })?;
//!
//! // Subscribe (blocking write to TLS stream via internal command channel).
//! client.subscribe_quotes(
//!     &thetadatadx::fpss::protocol::Contract::stock("AAPL"),
//! )?;
//!
//! // ... later
//! client.shutdown();
//! # Ok(())
//! # }
//! ```
//!
//! # Internal architecture
//!
//! ```text
//!  +---------------+  cmd channel   +--------------------+  publish()  +------------------+
//!  | FpssClient    |--------------->| I/O thread         |------------>| Disruptor Ring   |
//!  |               |                | (std::thread)      |             | (SPSC, lock-     |
//!  | .subscribe()  |                | blocking TLS read  |             |  free, pre-      |
//!  | .unsubscribe  |                | + write drain      |             |  allocated)      |
//!  | .shutdown()   |                +--------------------+             +--------+---------+
//!  +---------------+                +--------------------+                      | consumer
//!                                   | Ping thread        |                      v
//!                                   | (std::thread,      |             +------------------+
//!                                   |  sleep loop)       |             | User handler(F)  |
//!                                   +--------------------+             | (catch_unwind,   |
//!                                                                      |  panic-isolated) |
//!                                                                      +------------------+
//! ```
//!
//! The I/O thread owns the TLS stream exclusively. Write requests (subscribe,
//! unsubscribe, ping) arrive via a `std::sync::mpsc` command channel. Between
//! blocking reads (during read timeouts), the I/O thread drains the command
//! queue and sends frames. This eliminates all lock contention on the TLS stream.
//!
//! There is exactly ONE queue between the TLS reader and the user callback —
//! the LMAX Disruptor ring. The reader publishes events; the Disruptor's
//! consumer thread invokes the user callback wrapped in
//! [`std::panic::catch_unwind`] so a panic from user code (or binding glue
//! such as PyO3 / napi) is counted on [`FpssClient::panic_count`] and
//! reported via `tracing::error!` rather than tearing down the consumer.
//! Ring-buffer overflow (consumer falling behind) is counted on
//! [`FpssClient::dropped_count`] via `Producer::try_publish` failures.
//!
//! # Sub-modules
//!
//! - [`connection`] -- TLS TCP connection establishment (blocking)
//! - [`framing`] -- Wire frame reader/writer (sync `Read`/`Write`)
//! - [`protocol`] -- Message types, contract serialization, subscription payloads
//! - [`ring`] -- LMAX Disruptor ring buffer and adaptive wait strategy

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

// Surface a thin slice of the framing codec for offline benchmarks
// (`benches/bench_framing.rs`). The full `framing` module remains
// crate-private; only the round-trip primitives are exposed.
use self::events::IoCommand;
pub use self::events::{FpssControl, FpssData, FpssEvent};
pub use self::framing::{read_frame, write_frame, Frame};
use self::io_loop::{io_loop, ping_loop, wait_for_login, LoginResult};
pub use self::session::reconnect_delay;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
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
/// let args = FpssConnectArgs {
///     creds: &creds,
///     hosts: &hosts,
///     ring_size: 4096,
///     ..Default::default()
/// };
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
    pub ring_size: usize,
    /// I/O thread flush behavior. See [`FpssFlushMode`].
    pub flush_mode: FpssFlushMode,
    /// Auto-reconnect policy after involuntary disconnect.
    pub policy: ReconnectPolicy,
    /// When `false`, suppresses locally derived `FpssData::Ohlcvc` events.
    /// Server-sent OHLCVC frames (wire code 24) still pass through.
    pub derive_ohlcvc: bool,
}

impl<'a> FpssConnectArgs<'a> {
    /// Construct with the two required arguments and SDK defaults for the rest.
    ///
    /// Equivalent to `FpssConnectArgs { creds, hosts, ..Default::default() }`,
    /// but avoids the lifetime gymnastics that the spread pattern can trip on
    /// when `hosts` is borrowed from a temporary.
    #[must_use]
    pub fn new(creds: &'a Credentials, hosts: &'a [(String, u16)]) -> Self {
        Self {
            creds,
            hosts,
            ring_size: 4096,
            flush_mode: FpssFlushMode::default(),
            policy: ReconnectPolicy::default(),
            derive_ohlcvc: true,
        }
    }
}

impl<'a> Default for FpssConnectArgs<'a> {
    fn default() -> Self {
        // Reason: `creds` and `hosts` are required references with no
        // sensible global default. `Default` is implemented so callers can
        // use `FpssConnectArgs { creds, hosts, ..Default::default() }` —
        // the placeholders are immediately overwritten in any working call.
        const EMPTY_HOSTS: &[(String, u16)] = &[];
        // Static credentials placeholder; overridden by the caller.
        // Held in a `OnceLock` so the reference outlives the function.
        static EMPTY_CREDS: std::sync::OnceLock<Credentials> = std::sync::OnceLock::new();
        let creds = EMPTY_CREDS.get_or_init(|| Credentials::new("", ""));
        Self {
            creds,
            hosts: EMPTY_HOSTS,
            ring_size: 4096,
            flush_mode: FpssFlushMode::default(),
            policy: ReconnectPolicy::default(),
            derive_ohlcvc: true,
        }
    }
}

/// Selector for the test-only [`FpssClient::for_self_join_test`]
/// constructor's pre-burst path. Lets soak tests pick between
/// blocking `publish` (matches handshake-time control-frame emission)
/// and non-blocking `try_publish` (matches the live data path that
/// drives the public `dropped_count`).
#[cfg(any(test, feature = "test-harness"))]
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
/// 2. `subscribe_quotes()` / `subscribe_trades()` -- subscribe to market data
/// 3. Events delivered via the user's `FnMut(&FpssEvent)` callback on the Disruptor thread
/// 4. `shutdown()` -- clean disconnect
///
/// # Thread safety
///
/// `FpssClient` is `Send + Sync`. The `subscribe_*` and `unsubscribe_*` methods
/// send commands through a lock-free channel to the I/O thread; they never touch
/// the TLS stream directly.
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
    /// Monotonically increasing request ID counter.
    next_req_id: AtomicI32,
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
    /// completion via [`FpssClient::is_streaming`] or
    /// [`ThetaDataDx::is_streaming`] returning `false` rather than
    /// blocking on `Drop`.
    consumer_thread_id: Arc<OnceLock<ThreadId>>,
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
    /// Returns [`Error`] if the TLS handshake or FPSS authentication fails.
    pub fn connect<F>(args: FpssConnectArgs<'_>, handler: F) -> Result<Self, Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        let FpssConnectArgs {
            creds,
            hosts,
            ring_size,
            flush_mode,
            policy,
            derive_ohlcvc,
        } = args;
        let borrowed: Vec<(&str, u16)> = hosts.iter().map(|(h, p)| (h.as_str(), *p)).collect();
        let (stream, server_addr) = connection::connect_to_servers(&borrowed)?;
        Self::connect_with_stream(
            creds,
            stream,
            server_addr,
            hosts,
            ring_size,
            derive_ohlcvc,
            flush_mode,
            policy,
            handler,
        )
    }

    /// Connect using a pre-established stream (for testing with mock sockets).
    ///
    /// `hosts` is the full FPSS server list, needed for auto-reconnect to try
    /// all servers. Pass an empty slice to disable reconnection to other servers.
    #[allow(clippy::too_many_arguments)] // Reason: FFI boundary requires all connection params in one call
    pub(crate) fn connect_with_stream<F>(
        creds: &Credentials,
        mut stream: connection::FpssStream,
        server_addr: String,
        hosts: &[(String, u16)],
        ring_size: usize,
        derive_ohlcvc: bool,
        flush_mode: FpssFlushMode,
        policy: ReconnectPolicy,
        handler: F,
    ) -> Result<Self, Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
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
        // Captured by the Disruptor consumer closure on first dispatch
        // and read by `FpssClient::drop` to break the self-join cycle
        // (callback -> stop_streaming -> drop FpssClient -> join io
        // thread -> drop producer -> join consumer thread = self).
        let consumer_thread_id: Arc<OnceLock<ThreadId>> = Arc::new(OnceLock::new());

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

        let io_handle = thread::Builder::new()
            .name("fpss-io".to_owned())
            .spawn(move || {
                io_loop(
                    stream,
                    cmd_rx,
                    handler,
                    ring_size,
                    io_shutdown,
                    io_authenticated,
                    permissions,
                    pending_control,
                    io_server_addr,
                    derive_ohlcvc,
                    flush_mode,
                    policy,
                    io_creds,
                    io_hosts,
                    io_active_subs,
                    io_active_full_subs,
                    io_dropped,
                    io_panics,
                    io_consumer_thread_id,
                );
            })
            .map_err(|e| Error::Fpss {
                kind: crate::error::FpssErrorKind::ConnectionRefused,
                message: format!("failed to spawn fpss-io thread: {e}"),
            })?;

        // Spawn the ping thread: sends PING command every 100ms.
        let ping_shutdown = Arc::clone(&shutdown);
        let ping_authenticated = Arc::clone(&authenticated);

        let ping_handle = thread::Builder::new()
            .name("fpss-ping".to_owned())
            .spawn(move || {
                ping_loop(ping_cmd_tx, ping_shutdown, ping_authenticated);
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
            next_req_id: AtomicI32::new(1),
            active_subs,
            active_full_subs,
            server_addr,
            dropped,
            panics,
            consumer_thread_id,
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

    /// Subscribe to quote data for a contract.
    ///
    /// # Wire protocol (from `PacketStream.addQuote()`)
    ///
    /// Sends code 21 (QUOTE) with payload `[req_id: i32 BE] [contract bytes]`.
    /// Server responds with code 40 (`REQ_RESPONSE`).
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_quotes(&self, contract: &Contract) -> Result<(), Error> {
        self.subscribe(SubscriptionKind::Quote, contract)
    }

    /// Subscribe to trade data for a contract.
    ///
    /// Source: `PacketStream.addTrade()` -- sends code 22 (TRADE).
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_trades(&self, contract: &Contract) -> Result<(), Error> {
        self.subscribe(SubscriptionKind::Trade, contract)
    }

    /// Subscribe to open interest data for a contract.
    ///
    /// Source: `PacketStream.addOpenInterest()` -- sends code 23 (`OPEN_INTEREST`).
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_open_interest(&self, contract: &Contract) -> Result<(), Error> {
        self.subscribe(SubscriptionKind::OpenInterest, contract)
    }

    /// Subscribe to quotes + trades for a contract (convenience batch).
    ///
    /// **Note:** if the second subscription (trades) fails, the first (quotes)
    /// remains active. The FPSS protocol does not support batched subscriptions,
    /// so rollback would require an `unsubscribe` call that could itself fail.
    /// Use individual `subscribe_quotes` / `subscribe_trades` and their
    /// corresponding `unsubscribe` methods when you need atomic control.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_all(&self, contract: &Contract) -> Result<(), Error> {
        self.subscribe_quotes(contract)?;
        self.subscribe_trades(contract)?;
        Ok(())
    }

    /// Subscribe to all trades for a security type (full trade stream).
    ///
    /// # Behavior (from `ThetaData` server)
    ///
    /// The server sends a **bundle** per trade event (not just trades):
    /// 1. Pre-trade NBBO quote (last quote before the trade)
    /// 2. OHLC bar for the traded contract
    /// 3. The trade itself
    /// 4. Post-trade NBBO quote 1
    /// 5. Post-trade NBBO quote 2
    ///
    /// Your callback will receive [`FpssData::Quote`], [`FpssData::Trade`], and
    /// [`FpssData::Ohlcvc`] events interleaved. This is normal behavior from
    /// the `ThetaData` FPSS server.
    ///
    /// If OHLCVC derivation is enabled (default), you will also
    /// receive locally-derived [`FpssData::Ohlcvc`] after each trade. Pass
    /// `derive_ohlcvc: false` to [`FpssClient::connect`] to disable this and reduce throughput overhead.
    ///
    /// # Wire protocol (from `PacketStream.java`)
    ///
    /// Sends code 22 (TRADE) with 5-byte payload `[req_id: i32 BE] [sec_type: u8]`.
    /// The server distinguishes this from per-contract subscriptions by payload length.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_full_trades(
        &self,
        sec_type: tdbe::types::enums::SecType,
    ) -> Result<(), Error> {
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = protocol::build_full_type_subscribe_payload(req_id, sec_type);

        self.send_cmd(IoCommand::WriteFrame {
            code: StreamMsgType::Trade,
            payload,
        })?;

        tracing::debug!(req_id, sec_type = ?sec_type, "sent full trade subscription");

        // Track for reconnection
        {
            let mut subs = self
                .active_full_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            subs.push((SubscriptionKind::Trade, sec_type));
        }

        Ok(())
    }

    /// Subscribe to all open interest data for a security type (full OI stream).
    ///
    /// Same pattern as [`FpssClient::subscribe_full_trades`] but for open interest.
    ///
    /// # Wire protocol
    ///
    /// Sends code 23 (`OPEN_INTEREST`) with 5-byte payload `[req_id: i32 BE] [sec_type: u8]`.
    /// The server distinguishes this from per-contract subscriptions by payload length.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_full_open_interest(
        &self,
        sec_type: tdbe::types::enums::SecType,
    ) -> Result<(), Error> {
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = protocol::build_full_type_subscribe_payload(req_id, sec_type);

        self.send_cmd(IoCommand::WriteFrame {
            code: StreamMsgType::OpenInterest,
            payload,
        })?;

        tracing::debug!(req_id, sec_type = ?sec_type, "sent full open interest subscription");

        // Track for reconnection
        {
            let mut subs = self
                .active_full_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            subs.push((SubscriptionKind::OpenInterest, sec_type));
        }

        Ok(())
    }

    /// Unsubscribe from all trades for a security type (full trade stream).
    ///
    /// # Wire protocol
    ///
    /// Sends code 52 (`REMOVE_TRADE`) with 5-byte payload `[req_id: i32 BE] [sec_type: u8]`.
    /// Same format as [`FpssClient::subscribe_full_trades`] but with the REMOVE code.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe_full_trades(
        &self,
        sec_type: tdbe::types::enums::SecType,
    ) -> Result<(), Error> {
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = protocol::build_full_type_subscribe_payload(req_id, sec_type);

        self.send_cmd(IoCommand::WriteFrame {
            code: StreamMsgType::RemoveTrade,
            payload,
        })?;

        // Remove from tracked subscriptions
        {
            let mut subs = self
                .active_full_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            subs.retain(|(k, s)| !(k == &SubscriptionKind::Trade && s == &sec_type));
        }

        tracing::debug!(req_id, sec_type = ?sec_type, "sent full trade unsubscribe");
        Ok(())
    }

    /// Unsubscribe from all open interest for a security type (full OI stream).
    ///
    /// # Wire protocol
    ///
    /// Sends code 53 (`REMOVE_OPEN_INTEREST`) with 5-byte payload `[req_id: i32 BE] [sec_type: u8]`.
    /// Same format as [`FpssClient::subscribe_full_open_interest`] but with the REMOVE code.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe_full_open_interest(
        &self,
        sec_type: tdbe::types::enums::SecType,
    ) -> Result<(), Error> {
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = protocol::build_full_type_subscribe_payload(req_id, sec_type);

        self.send_cmd(IoCommand::WriteFrame {
            code: StreamMsgType::RemoveOpenInterest,
            payload,
        })?;

        // Remove from tracked subscriptions
        {
            let mut subs = self
                .active_full_subs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            subs.retain(|(k, s)| !(k == &SubscriptionKind::OpenInterest && s == &sec_type));
        }

        tracing::debug!(req_id, sec_type = ?sec_type, "sent full open interest unsubscribe");
        Ok(())
    }

    /// Unsubscribe from quote data for a contract.
    ///
    /// Source: `PacketStream.removeQuote()` -- sends code 51 (`REMOVE_QUOTE`).
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe_quotes(&self, contract: &Contract) -> Result<(), Error> {
        self.unsubscribe(SubscriptionKind::Quote, contract)
    }

    /// Unsubscribe from trade data for a contract.
    ///
    /// Source: `PacketStream.removeTrade()` -- sends code 52 (`REMOVE_TRADE`).
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe_trades(&self, contract: &Contract) -> Result<(), Error> {
        self.unsubscribe(SubscriptionKind::Trade, contract)
    }

    /// Unsubscribe from open interest data for a contract.
    ///
    /// Source: `PacketStream.removeOpenInterest()` -- sends code 53.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe_open_interest(&self, contract: &Contract) -> Result<(), Error> {
        self.unsubscribe(SubscriptionKind::OpenInterest, contract)
    }

    /// Internal subscribe implementation.
    fn subscribe(&self, kind: SubscriptionKind, contract: &Contract) -> Result<(), Error> {
        contract.validate()?;
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
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

    /// Internal unsubscribe implementation.
    fn unsubscribe(&self, kind: SubscriptionKind, contract: &Contract) -> Result<(), Error> {
        contract.validate()?;
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
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
    #[cfg(any(test, feature = "test-harness"))]
    #[doc(hidden)]
    pub fn for_self_join_test<F>(
        n_burst_events: usize,
        ring_size: usize,
        mode: HarnessPublishMode,
        handler: F,
    ) -> Arc<Self>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        use disruptor::{build_single_producer, BusySpin, Producer, Sequence};

        use self::ring::RingEvent;

        let ring_size = ring::next_power_of_two(ring_size.max(ring::MIN_RING_SIZE));

        let shutdown = Arc::new(AtomicBool::new(false));
        let authenticated = Arc::new(AtomicBool::new(true));
        let active_subs: Arc<Mutex<Vec<(SubscriptionKind, Contract)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let active_full_subs: Arc<Mutex<Vec<(SubscriptionKind, tdbe::types::enums::SecType)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicU64::new(0));
        let panics = Arc::new(AtomicU64::new(0));
        let consumer_thread_id: Arc<OnceLock<ThreadId>> = Arc::new(OnceLock::new());

        let (cmd_tx, _cmd_rx) = std_mpsc::channel::<IoCommand>();

        let handler_cell = Mutex::new(handler);
        let panics_consumer = Arc::clone(&panics);
        let consumer_thread_id_cell = Arc::clone(&consumer_thread_id);

        let factory = || RingEvent { event: None };
        let mut producer = build_single_producer(ring_size, factory, BusySpin)
            .handle_events_with(move |slot: &RingEvent, _seq: Sequence, _eob: bool| {
                consumer_thread_id_cell.get_or_init(|| thread::current().id());
                if let Some(ref evt) = slot.event {
                    match evt {
                        FpssEvent::Empty | FpssEvent::RawData { .. } => {}
                        _ => {
                            let mut h = handler_cell
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h(evt)))
                                .is_err()
                            {
                                panics_consumer.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
            })
            .build();

        // Pre-burst on the spawning thread: blocking publishes never
        // overflow, so this path is for the self-join repro that just
        // needs the callback to fire. Try-publish bursts run on the
        // io thread below so the test can race the producer against a
        // gated consumer the same way the real reader thread does.
        if matches!(mode, HarnessPublishMode::BlockingPublish) {
            for _ in 0..n_burst_events {
                producer.publish(|slot| {
                    slot.event = Some(FpssEvent::Control(FpssControl::MarketOpen));
                });
            }
        }

        // Fake I/O thread: in `TryPublishBurst` mode, push the burst
        // via `try_publish` exactly like `io_loop` does on the real
        // TLS reader path, incrementing the shared `dropped` counter
        // on every overflow rejection. Then park until shutdown and
        // drop the producer (producer-drop joins the consumer, the
        // exact transitive dependency that creates the self-join
        // hazard in the production exit path).
        let io_shutdown = Arc::clone(&shutdown);
        let io_dropped = Arc::clone(&dropped);
        let io_burst = n_burst_events;
        let io_handle = thread::Builder::new()
            .name("fpss-io-test".to_owned())
            .spawn(move || {
                if matches!(mode, HarnessPublishMode::TryPublishBurst) {
                    for _ in 0..io_burst {
                        if producer
                            .try_publish(|slot| {
                                slot.event = Some(FpssEvent::Control(FpssControl::MarketOpen));
                            })
                            .is_err()
                        {
                            io_dropped.fetch_add(1, Ordering::Relaxed);
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
            next_req_id: AtomicI32::new(1),
            active_subs,
            active_full_subs,
            server_addr: "test://self-join".to_owned(),
            dropped,
            panics,
            consumer_thread_id,
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
        // `ThetaDataDx::stop_streaming()` swaps the live slot to
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
            // termination.
            let detached = thread::Builder::new()
                .name("fpss-shutdown-detach".to_owned())
                .spawn(move || {
                    if let Some(h) = ping_handle {
                        let _ = h.join();
                    }
                    if let Some(h) = io_handle {
                        let _ = h.join();
                    }
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
                // running on.
            }
            return;
        }

        if let Some(h) = ping_handle {
            let _ = h.join();
        }
        if let Some(h) = io_handle {
            let _ = h.join();
        }
    }
}
