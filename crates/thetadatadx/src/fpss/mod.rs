//! FPSS (Feed Processing Streaming Server) real-time streaming client.
//!
//! # Architecture (from decompiled Java -- `FPSSClient.java`)
//!
//! The FPSS protocol provides real-time market data over a custom TLS/TCP
//! binary protocol. The Java terminal's `FPSSClient` runs:
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
//! `.await` anywhere. This matches the Java terminal exactly:
//!
//! ```text
//! Java:  std::thread (blocking DataInputStream.read) -> LMAX Disruptor ring -> event handler callback
//! Rust:  std::thread (blocking TLS read)             -> LMAX Disruptor ring -> user's FnMut(&FpssEvent) callback
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! # use thetadatadx::fpss::{FpssClient, FpssData, FpssEvent};
//! # use thetadatadx::auth::Credentials;
//! # fn example() -> Result<(), thetadatadx::error::Error> {
//! let creds = Credentials::new("user@example.com", "pw");
//! let hosts = thetadatadx::config::DirectConfig::production().fpss_hosts;
//! let client = FpssClient::connect(&creds, &hosts, 4096, Default::default(), Default::default(), true, |event: &FpssEvent| {
//!     // Runs on the Disruptor consumer thread -- keep it fast.
//!     // Push to your own queue for heavy processing.
//!     match event {
//!         FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, .. }) => { /* f64 prices */ }
//!         FpssEvent::Data(FpssData::Trade { contract_id, price, size, .. }) => { /* f64 price */ }
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
//!                                   +--------------------+             | (zero-alloc)     |
//!                                                                      +------------------+
//! ```
//!
//! The I/O thread owns the TLS stream exclusively. Write requests (subscribe,
//! unsubscribe, ping) arrive via a `std::sync::mpsc` command channel. Between
//! blocking reads (during read timeouts), the I/O thread drains the command
//! queue and sends frames. This eliminates all lock contention on the TLS stream.
//!
//! # Sub-modules
//!
//! - [`connection`] -- TLS TCP connection establishment (blocking)
//! - [`framing`] -- Wire frame reader/writer (sync `Read`/`Write`)
//! - [`protocol`] -- Message types, contract serialization, subscription payloads
//! - [`ring`] -- LMAX Disruptor ring buffer and adaptive wait strategy

pub mod connection;
pub mod framing;
pub mod protocol;
pub mod ring;

use std::collections::HashMap;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Empty symbol placeholder used when the contract_id has not been resolved yet.
static EMPTY_SYMBOL: std::sync::LazyLock<Arc<str>> = std::sync::LazyLock::new(|| Arc::from(""));

use disruptor::{build_single_producer, Producer, Sequence};

use self::ring::{AdaptiveWaitStrategy, RingEvent};

use crate::auth::Credentials;
use crate::config::{FpssFlushMode, ReconnectPolicy};
use crate::error::Error;
use tdbe::codec::fit::{apply_deltas, FitReader};
use tdbe::types::enums::{RemoveReason, StreamMsgType, StreamResponseType};
use tdbe::types::price::Price;

use self::framing::{
    read_frame, read_frame_into, write_frame, write_raw_frame, write_raw_frame_no_flush, Frame,
};
use self::protocol::{
    build_credentials_payload, build_ping_payload, build_subscribe_payload, parse_contract_message,
    parse_disconnect_reason, parse_req_response, Contract, SubscriptionKind, PING_INTERVAL_MS,
    RECONNECT_DELAY_MS, TOO_MANY_REQUESTS_DELAY_MS,
};

/// Tick data events from the FPSS stream.
///
/// These are the hot-path events decoded from FIT wire format and
/// delta-decompressed. All price fields are decoded to `f64` at parse time.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FpssData {
    /// Decoded quote tick (code 21).
    Quote {
        contract_id: i32,
        /// Resolved symbol string (e.g. "AAPL"). Empty if contract not yet assigned.
        symbol: Arc<str>,
        ms_of_day: i32,
        bid_size: i32,
        bid_exchange: i32,
        bid: f64,
        bid_condition: i32,
        ask_size: i32,
        ask_exchange: i32,
        ask: f64,
        ask_condition: i32,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
    /// Decoded trade tick (code 22).
    Trade {
        contract_id: i32,
        /// Resolved symbol string (e.g. "AAPL"). Empty if contract not yet assigned.
        symbol: Arc<str>,
        ms_of_day: i32,
        sequence: i32,
        ext_condition1: i32,
        ext_condition2: i32,
        ext_condition3: i32,
        ext_condition4: i32,
        condition: i32,
        size: i32,
        exchange: i32,
        price: f64,
        condition_flags: i32,
        price_flags: i32,
        volume_type: i32,
        records_back: i32,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
    /// Decoded open interest tick (code 23).
    OpenInterest {
        contract_id: i32,
        /// Resolved symbol string (e.g. "AAPL"). Empty if contract not yet assigned.
        symbol: Arc<str>,
        ms_of_day: i32,
        open_interest: i32,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
    /// Decoded OHLCVC bar (code 24 or trade-derived).
    ///
    /// `volume` and `count` are `i64` to avoid overflow on high-volume symbols.
    Ohlcvc {
        contract_id: i32,
        /// Resolved symbol string (e.g. "AAPL"). Empty if contract not yet assigned.
        symbol: Arc<str>,
        ms_of_day: i32,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: i64,
        count: i64,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
}

/// Control/lifecycle events from the FPSS stream.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FpssControl {
    /// Login succeeded (METADATA code 3).
    LoginSuccess { permissions: String },
    /// Server sent a CONTRACT assignment (code 20).
    ContractAssigned { id: i32, contract: Contract },
    /// Subscription response (code 40).
    ReqResponse {
        req_id: i32,
        result: StreamResponseType,
    },
    /// Market open signal (code 30).
    MarketOpen,
    /// Market close / stop signal (code 32).
    MarketClose,
    /// Server error message (code 11).
    ServerError { message: String },
    /// Server disconnected us (code 12).
    Disconnected { reason: RemoveReason },
    /// Auto-reconnect is about to attempt reconnection.
    ///
    /// Emitted before sleeping for the delay. `attempt` is 1-based.
    Reconnecting {
        reason: RemoveReason,
        attempt: u32,
        delay_ms: u64,
    },
    /// Auto-reconnect succeeded -- connection is live again.
    Reconnected,
    /// Protocol-level parse error.
    Error { message: String },
}

/// All FPSS events -- either data or control.
///
/// Subscribers receive these through the Disruptor callback. The enum is
/// non-exhaustive to allow adding new event types without breaking downstream.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum FpssEvent {
    /// Tick data event (quote, trade, open interest, OHLCVC).
    Data(FpssData),
    /// Control/lifecycle event (login, contract assignment, market open/close, etc.).
    Control(FpssControl),
    /// Raw undecoded data (fallback for payloads too short or corrupt to decode).
    ///
    /// Filtered from user callbacks -- only visible to internal code.
    #[doc(hidden)]
    RawData { code: u8, payload: Vec<u8> },
    /// Placeholder default for ring buffer pre-allocation.
    ///
    /// Filtered from user callbacks -- only visible to internal code.
    #[doc(hidden)]
    #[default]
    Empty,
}

// ---------------------------------------------------------------------------
// Command channel -- FpssClient -> I/O thread
// ---------------------------------------------------------------------------

/// Commands sent from the `FpssClient` handle to the I/O thread.
enum IoCommand {
    /// Write a raw frame (code + payload) to the TLS stream.
    WriteFrame {
        code: StreamMsgType,
        payload: Vec<u8>,
    },
    /// Graceful shutdown: send STOP, then exit the I/O loop.
    Shutdown,
}

// ---------------------------------------------------------------------------
// FpssClient
// ---------------------------------------------------------------------------

/// Real-time streaming client for `ThetaData`'s FPSS servers.
///
/// # Lifecycle (from `FPSSClient.java`)
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
///
/// Source: `FPSSClient.java` -- main connection/reconnection state machine.
pub struct FpssClient {
    /// Channel to send write commands to the I/O thread.
    cmd_tx: std_mpsc::Sender<IoCommand>,
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
    active_subs: Mutex<Vec<(SubscriptionKind, Contract)>>,
    /// Active full-type (firehose) subscriptions for reconnection.
    active_full_subs: Mutex<Vec<(SubscriptionKind, tdbe::types::enums::SecType)>>,
    /// Server-assigned contract ID mapping.
    contract_map: Arc<Mutex<HashMap<i32, Contract>>>,
    /// The server address we connected to.
    server_addr: String,
}

// SAFETY: All fields are either Send+Sync or behind Mutex/Atomic.
unsafe impl Sync for FpssClient {}

impl FpssClient {
    /// Connect to a `ThetaData` FPSS server, authenticate, and start processing
    /// events via the provided callback.
    ///
    /// The callback runs on the Disruptor's consumer thread -- keep it fast.
    /// For heavy processing, push events to your own queue from the callback.
    ///
    /// # Sequence (from `FPSSClient.java`)
    ///
    /// 1. Try each server in `hosts` until one connects (blocking TLS over TCP)
    /// 2. Send CREDENTIALS (code 0) with email + password
    /// 3. Wait for METADATA (code 3) = login success, or DISCONNECTED (code 12) = failure
    /// 4. Start ping heartbeat (100ms interval, `std::thread` with sleep loop)
    /// 5. Start I/O thread (blocking TLS read -> Disruptor ring -> callback)
    ///
    /// Source: `FPSSClient.connect()` and `FPSSClient.sendCredentials()`.
    /// Connect to FPSS streaming servers.
    ///
    /// `hosts` is the FPSS server list from [`DirectConfig::fpss_hosts`].
    /// Servers are tried in order until one connects.
    ///
    /// `policy` controls auto-reconnect behavior after involuntary disconnect.
    ///
    /// When `derive_ohlcvc` is `false`, the client will NOT emit derived
    /// `FpssData::Ohlcvc` events after each trade. You still receive
    /// server-sent OHLCVC frames (wire code 24). This reduces throughput
    /// overhead by eliminating one extra event per trade.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the TLS handshake or FPSS authentication fails.
    pub fn connect<F>(
        creds: &Credentials,
        hosts: &[(String, u16)],
        ring_size: usize,
        flush_mode: FpssFlushMode,
        policy: ReconnectPolicy,
        derive_ohlcvc: bool,
        handler: F,
    ) -> Result<Self, Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
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
        // Send CREDENTIALS (code 0)
        // Source: FPSSClient.sendCredentials()
        let cred_payload = build_credentials_payload(&creds.email, &creds.password);
        let frame = Frame::new(StreamMsgType::Credentials, cred_payload);
        write_frame(&mut stream, &frame)?;
        tracing::debug!("sent CREDENTIALS to {server_addr}");

        // Wait for METADATA (success) or DISCONNECTED (failure)
        // Source: FPSSClient.connect() -- blocks until login response arrives
        let login_result = wait_for_login(&mut stream)?;

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
        let contract_map = Arc::new(Mutex::new(HashMap::new()));
        // Pre-rendered symbol strings keyed by contract ID. Populated on
        // ContractAssigned so resolve_symbol() is zero-alloc on the hot path.
        let symbol_cache: Arc<Mutex<HashMap<i32, Arc<str>>>> = Arc::new(Mutex::new(HashMap::new()));

        // Command channel: FpssClient -> I/O thread
        let (cmd_tx, cmd_rx) = std_mpsc::channel::<IoCommand>();

        // Ping command channel: ping thread -> I/O thread
        let ping_cmd_tx = cmd_tx.clone();

        // Spawn the I/O thread: blocking TLS read + Disruptor publish + command drain.
        let io_shutdown = Arc::clone(&shutdown);
        let io_authenticated = Arc::clone(&authenticated);
        let io_contract_map = Arc::clone(&contract_map);
        let io_symbol_cache = Arc::clone(&symbol_cache);
        let io_server_addr = server_addr.clone();
        let io_creds = creds.clone();
        let io_hosts = hosts.to_vec();

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
                    io_contract_map,
                    io_symbol_cache,
                    permissions,
                    io_server_addr,
                    derive_ohlcvc,
                    flush_mode,
                    policy,
                    io_creds,
                    io_hosts,
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
            cmd_tx,
            io_handle: Some(io_handle),
            ping_handle: Some(ping_handle),
            shutdown,
            authenticated,
            next_req_id: AtomicI32::new(1),
            active_subs: Mutex::new(Vec::new()),
            active_full_subs: Mutex::new(Vec::new()),
            contract_map,
            server_addr,
        })
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
    /// `derive_ohlcvc: false` to [`connect`] to disable this and reduce throughput overhead.
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

        self.cmd_tx
            .send(IoCommand::WriteFrame {
                code: StreamMsgType::Trade,
                payload,
            })
            .map_err(|_| Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "I/O thread has exited".to_string(),
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
    /// Same pattern as [`subscribe_full_trades`] but for open interest.
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

        self.cmd_tx
            .send(IoCommand::WriteFrame {
                code: StreamMsgType::OpenInterest,
                payload,
            })
            .map_err(|_| Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "I/O thread has exited".to_string(),
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
    /// Same format as [`subscribe_full_trades`] but with the REMOVE code.
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

        self.cmd_tx
            .send(IoCommand::WriteFrame {
                code: StreamMsgType::RemoveTrade,
                payload,
            })
            .map_err(|_| Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "I/O thread has exited".to_string(),
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
    /// Same format as [`subscribe_full_open_interest`] but with the REMOVE code.
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

        self.cmd_tx
            .send(IoCommand::WriteFrame {
                code: StreamMsgType::RemoveOpenInterest,
                payload,
            })
            .map_err(|_| Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "I/O thread has exited".to_string(),
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
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = build_subscribe_payload(req_id, contract);
        let code = kind.subscribe_code();

        self.cmd_tx
            .send(IoCommand::WriteFrame { code, payload })
            .map_err(|_| Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "I/O thread has exited".to_string(),
            })?;

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
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = build_subscribe_payload(req_id, contract);
        let code = kind.unsubscribe_code();

        self.cmd_tx
            .send(IoCommand::WriteFrame { code, payload })
            .map_err(|_| Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "I/O thread has exited".to_string(),
            })?;

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
    /// Source: `FPSSClient.disconnect()` -- sends STOP (code 32), then closes socket.
    pub fn shutdown(&self) {
        if self.shutdown.swap(true, Ordering::AcqRel) {
            return; // already shut down
        }

        tracing::info!(server = %self.server_addr, "shutting down FPSS client");

        // Send shutdown command to I/O thread (which will send STOP to server).
        let _ = self.cmd_tx.send(IoCommand::Shutdown);

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

    /// Get the current contract map (server-assigned IDs -> contracts).
    ///
    /// Useful for decoding data messages that reference contracts by ID.
    pub fn contract_map(&self) -> HashMap<i32, Contract> {
        self.contract_map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Look up a single contract by its server-assigned ID.
    ///
    /// Much cheaper than [`contract_map()`](Self::contract_map) for the hot path
    /// where callers decode FIT ticks and need to resolve individual contract IDs.
    pub fn contract_lookup(&self, id: i32) -> Option<Contract> {
        self.contract_map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .cloned()
    }

    /// Get a snapshot of currently active per-contract subscriptions.
    pub fn active_subscriptions(&self) -> Vec<(SubscriptionKind, Contract)> {
        self.active_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Get a snapshot of currently active full-type (firehose) subscriptions.
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
}

impl Drop for FpssClient {
    fn drop(&mut self) {
        // Signal shutdown if not already done.
        self.shutdown.store(true, Ordering::Release);
        // Send shutdown command so I/O thread exits its loop.
        let _ = self.cmd_tx.send(IoCommand::Shutdown);

        // Join background threads.
        if let Some(h) = self.ping_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.io_handle.take() {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Login result (internal)
// ---------------------------------------------------------------------------

enum LoginResult {
    Success(String),
    Disconnected(RemoveReason),
}

/// Wait for the server's login response (blocking).
///
/// Source: `FPSSClient.connect()` -- reads frames until METADATA or DISCONNECTED.
fn wait_for_login(stream: &mut connection::FpssStream) -> Result<LoginResult, Error> {
    loop {
        let frame = read_frame(stream)?.ok_or_else(|| Error::Fpss {
            kind: crate::error::FpssErrorKind::Disconnected,
            message: "connection closed during login handshake".to_string(),
        })?;

        match frame.code {
            StreamMsgType::Metadata => {
                let permissions = String::from_utf8_lossy(&frame.payload).to_string();
                return Ok(LoginResult::Success(permissions));
            }
            StreamMsgType::Disconnected => {
                let reason = parse_disconnect_reason(&frame.payload);
                return Ok(LoginResult::Disconnected(reason));
            }
            StreamMsgType::Error => {
                let msg = String::from_utf8_lossy(&frame.payload);
                tracing::warn!(message = %msg, "server error during login");
                return Err(Error::Fpss {
                    kind: crate::error::FpssErrorKind::ConnectionRefused,
                    message: format!("server error during login: {msg}"),
                });
            }
            other => {
                tracing::trace!(code = ?other, "ignoring frame during login handshake");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// I/O thread: blocking read + Disruptor publish + command drain
// ---------------------------------------------------------------------------

/// Maximum number of consecutive reconnection attempts before giving up.
const MAX_RECONNECT_ATTEMPTS: u32 = 5;

/// The I/O thread owns the TLS stream. It does three things in a loop:
///
/// 1. Attempt a blocking read (with short timeout) for incoming frames
/// 2. Drain the command channel for outgoing writes (subscribe, ping, etc.)
/// 3. Publish decoded events into the Disruptor ring
///
/// On involuntary disconnect, the reconnection policy determines whether
/// to automatically re-establish the connection within this same thread
/// (no new threads spawned).
///
/// This thread IS the Disruptor producer. Events flow directly from the TLS
/// socket into the ring buffer with zero intermediate channels.
// Reason: all parameters are moved into this function from a spawned thread closure.
#[allow(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    clippy::too_many_lines
)]
fn io_loop<F>(
    stream: connection::FpssStream,
    cmd_rx: std_mpsc::Receiver<IoCommand>,
    mut handler: F,
    ring_size: usize,
    shutdown: Arc<AtomicBool>,
    authenticated: Arc<AtomicBool>,
    contract_map: Arc<Mutex<HashMap<i32, Contract>>>,
    symbol_cache: Arc<Mutex<HashMap<i32, Arc<str>>>>,
    permissions: String,
    _server_addr: String,
    derive_ohlcvc: bool,
    flush_mode: FpssFlushMode,
    policy: ReconnectPolicy,
    creds: Credentials,
    hosts: Vec<(String, u16)>,
) where
    F: FnMut(&FpssEvent) + Send + 'static,
{
    let ring_size = ring::next_power_of_two(ring_size.max(ring::MIN_RING_SIZE));

    let factory = || RingEvent { event: None };
    let wait_strategy = AdaptiveWaitStrategy::fpss_default();

    let mut producer = build_single_producer(ring_size, factory, wait_strategy)
        .handle_events_with(
            move |ring_event: &RingEvent, _sequence: Sequence, _eob: bool| {
                if let Some(ref evt) = ring_event.event {
                    // Filter out internal-only events (Issue #185).
                    match evt {
                        FpssEvent::Empty | FpssEvent::RawData { .. } => {}
                        _ => handler(evt),
                    }
                }
            },
        )
        .build();

    // Publish login success event.
    producer.publish(|slot| {
        slot.event = Some(FpssEvent::Control(FpssControl::LoginSuccess {
            permissions,
        }));
    });

    // Split the stream into buffered read + buffered write.
    let mut reader = BufReader::new(stream);

    // Per-contract delta state for FIT decompression.
    let mut delta_state: DeltaState = DeltaState::new();

    // Reusable frame payload buffer.
    let mut frame_buf: Vec<u8> = Vec::with_capacity(framing::MAX_PAYLOAD_LEN);

    // Outer reconnection loop: each iteration runs one connection session.
    // On involuntary disconnect, the policy decides whether to reconnect.
    let mut reconnect_attempt: u32 = 0;

    'session: loop {
        // Track consecutive read timeouts to detect the 10s overall timeout.
        let max_consecutive_timeouts = (protocol::READ_TIMEOUT_MS / 50).max(1);
        let mut consecutive_timeouts: u64 = 0;

        // --- Inner read/write loop for one connection session ---
        // When the inner loop breaks, `disconnect_reason` holds the reason.
        let disconnect_reason: RemoveReason = 'inner: loop {
            if shutdown.load(Ordering::Relaxed) {
                break 'session;
            }

            // --- Phase 1: Try to read a frame (short blocking read) ---
            match read_frame_into(&mut reader, &mut frame_buf) {
                Ok(Some((code, payload_len))) => {
                    consecutive_timeouts = 0;
                    // Reset reconnect counter on successful data reception.
                    reconnect_attempt = 0;

                    let (primary, secondary) = decode_frame(
                        code,
                        &frame_buf[..payload_len],
                        &authenticated,
                        &contract_map,
                        &symbol_cache,
                        &shutdown,
                        &mut delta_state,
                        derive_ohlcvc,
                    );

                    if let Some(evt) = primary {
                        producer.publish(|slot| {
                            slot.event = Some(evt);
                        });
                    }
                    if let Some(evt) = secondary {
                        producer.publish(|slot| {
                            slot.event = Some(evt);
                        });
                    }
                }
                Ok(None) => {
                    // Clean EOF
                    tracing::warn!("FPSS connection closed by server");
                    producer.publish(|slot| {
                        slot.event = Some(FpssEvent::Control(FpssControl::Disconnected {
                            reason: RemoveReason::Unspecified,
                        }));
                    });
                    authenticated.store(false, Ordering::Release);
                    break 'inner RemoveReason::Unspecified;
                }
                Err(ref e) if is_read_timeout(e) => {
                    consecutive_timeouts += 1;
                    if consecutive_timeouts >= max_consecutive_timeouts {
                        tracing::warn!(
                            timeout_ms = protocol::READ_TIMEOUT_MS,
                            "FPSS read timed out (no data for {}ms)",
                            consecutive_timeouts * 50
                        );
                        producer.publish(|slot| {
                            slot.event = Some(FpssEvent::Control(FpssControl::Disconnected {
                                reason: RemoveReason::TimedOut,
                            }));
                        });
                        authenticated.store(false, Ordering::Release);
                        break 'inner RemoveReason::TimedOut;
                    }
                    // Otherwise, fall through to drain commands.
                }
                Err(e) => {
                    tracing::error!(error = %e, "FPSS read error");
                    producer.publish(|slot| {
                        slot.event = Some(FpssEvent::Control(FpssControl::Disconnected {
                            reason: RemoveReason::Unspecified,
                        }));
                    });
                    authenticated.store(false, Ordering::Release);
                    break 'inner RemoveReason::Unspecified;
                }
            }

            // --- Phase 2: Drain command channel (non-blocking) ---
            loop {
                match cmd_rx.try_recv() {
                    Ok(IoCommand::WriteFrame { code, payload }) => {
                        let writer = reader.get_mut();
                        let result = if code == StreamMsgType::Ping
                            || flush_mode == FpssFlushMode::Immediate
                        {
                            write_raw_frame(writer, code, &payload)
                        } else {
                            write_raw_frame_no_flush(writer, code, &payload)
                        };
                        if let Err(e) = result {
                            tracing::warn!(error = %e, "failed to write frame");
                        }
                    }
                    Ok(IoCommand::Shutdown) => {
                        let stop_payload = protocol::build_stop_payload();
                        let writer = reader.get_mut();
                        let _ = write_raw_frame(writer, StreamMsgType::Stop, &stop_payload);
                        tracing::debug!("sent STOP, I/O thread exiting");
                        shutdown.store(true, Ordering::Release);
                        break;
                    }
                    Err(std_mpsc::TryRecvError::Empty) => break,
                    Err(std_mpsc::TryRecvError::Disconnected) => {
                        tracing::debug!("command channel disconnected, I/O thread exiting");
                        shutdown.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        }; // end 'inner loop (yields RemoveReason)

        // If shutdown was requested (explicit or channel disconnect), exit entirely.
        if shutdown.load(Ordering::Relaxed) {
            break 'session;
        }

        // --- Reconnection decision ---
        let reason = disconnect_reason;
        reconnect_attempt += 1;

        let delay = match &policy {
            ReconnectPolicy::Manual => {
                tracing::info!(reason = ?reason, "manual reconnect policy -- not reconnecting");
                break 'session;
            }
            ReconnectPolicy::Auto => {
                if reconnect_attempt > MAX_RECONNECT_ATTEMPTS {
                    tracing::error!(
                        attempts = reconnect_attempt - 1,
                        "max reconnect attempts reached, giving up"
                    );
                    break 'session;
                }
                if let Some(ms) = reconnect_delay(reason) {
                    Duration::from_millis(ms)
                } else {
                    tracing::error!(reason = ?reason, "permanent disconnect -- not reconnecting");
                    break 'session;
                }
            }
            ReconnectPolicy::Custom(f) => {
                if let Some(d) = f(reason, reconnect_attempt) {
                    d
                } else {
                    tracing::info!(reason = ?reason, "custom policy returned None -- not reconnecting");
                    break 'session;
                }
            }
        };

        // Emit Reconnecting event before sleeping.
        let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        tracing::info!(
            reason = ?reason,
            attempt = reconnect_attempt,
            delay_ms,
            "auto-reconnecting FPSS"
        );
        metrics::counter!("thetadatadx.fpss.reconnects").increment(1);
        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(FpssControl::Reconnecting {
                reason,
                attempt: reconnect_attempt,
                delay_ms,
            }));
        });

        thread::sleep(delay);

        if shutdown.load(Ordering::Relaxed) {
            break 'session;
        }

        // --- Attempt new TLS connection and re-authenticate ---
        let new_stream = {
            let borrowed: Vec<(&str, u16)> = hosts.iter().map(|(h, p)| (h.as_str(), *p)).collect();
            connection::connect_to_servers(&borrowed)
        };

        let mut new_stream = match new_stream {
            Ok((s, addr)) => {
                tracing::info!(server = %addr, "reconnected to FPSS server");
                s
            }
            Err(e) => {
                tracing::warn!(error = %e, "reconnection failed, will retry");
                // Loop around to try again (reconnect_attempt is already incremented).
                continue 'session;
            }
        };

        // Re-authenticate on the new stream.
        let cred_payload = build_credentials_payload(&creds.email, &creds.password);
        let frame = Frame::new(StreamMsgType::Credentials, cred_payload);
        if let Err(e) = write_frame(&mut new_stream, &frame) {
            tracing::warn!(error = %e, "failed to send credentials on reconnect");
            continue 'session;
        }

        let login_result = match wait_for_login(&mut new_stream) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "login failed on reconnect");
                continue 'session;
            }
        };

        let new_permissions = match login_result {
            LoginResult::Success(p) => {
                tracing::info!(permissions = %p, "re-authenticated on reconnect");
                p
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
                tracing::warn!(reason = ?reason, "server rejected login on reconnect");
                continue 'session;
            }
        };

        // Set the short I/O read timeout on the new stream.
        let io_read_timeout = Duration::from_millis(50);
        if let Err(e) = new_stream.sock.set_read_timeout(Some(io_read_timeout)) {
            tracing::warn!(error = %e, "failed to set read timeout on reconnect");
            continue 'session;
        }

        // Clear delta state -- fresh connection means fresh deltas.
        delta_state.clear();
        contract_map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();

        authenticated.store(true, Ordering::Release);

        // Publish reconnection events.
        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(FpssControl::LoginSuccess {
                permissions: new_permissions,
            }));
        });
        producer.publish(|slot| {
            slot.event = Some(FpssEvent::Control(FpssControl::Reconnected));
        });

        // Replace the reader with the new stream.
        reader = BufReader::new(new_stream);

        // Drain any commands that queued up during reconnection (subscribe, ping, etc.)
        // and send them over the new connection to re-establish subscriptions.
        loop {
            match cmd_rx.try_recv() {
                Ok(IoCommand::WriteFrame { code, payload }) => {
                    let writer = reader.get_mut();
                    let result =
                        if code == StreamMsgType::Ping || flush_mode == FpssFlushMode::Immediate {
                            write_raw_frame(writer, code, &payload)
                        } else {
                            write_raw_frame_no_flush(writer, code, &payload)
                        };
                    if let Err(e) = result {
                        tracing::warn!(error = %e, "failed to write queued frame on reconnect");
                    }
                }
                Ok(IoCommand::Shutdown) => {
                    let stop_payload = protocol::build_stop_payload();
                    let writer = reader.get_mut();
                    let _ = write_raw_frame(writer, StreamMsgType::Stop, &stop_payload);
                    shutdown.store(true, Ordering::Release);
                    break;
                }
                Err(std_mpsc::TryRecvError::Empty) => break,
                Err(std_mpsc::TryRecvError::Disconnected) => {
                    shutdown.store(true, Ordering::Release);
                    break;
                }
            }
        }

        if shutdown.load(Ordering::Relaxed) {
            break 'session;
        }

        // Continue 'session loop: the inner read/write loop will run on the new stream.
    } // end 'session loop

    // Producer drop joins the Disruptor consumer thread and drains remaining events.
    tracing::debug!("fpss-io thread exiting");
}

/// Check if an error is a read timeout (`WouldBlock` or `TimedOut`).
fn is_read_timeout(e: &Error) -> bool {
    match e {
        Error::Io(io_err) => matches!(
            io_err.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// FIT delta state for tick decompression
// ---------------------------------------------------------------------------

/// Number of FIT fields per tick type (excluding the `contract_id` which is the
/// first FIT field). The FIT decoder returns `n_fields` total, where field [0]
/// is the `contract_id` and fields [1..] are the tick data.
const QUOTE_FIELDS: usize = 11;
const TRADE_FIELDS: usize = 16;
const OI_FIELDS: usize = 3;
const OHLCVC_FIELDS: usize = 9;

/// Per-contract OHLCVC accumulator, updated on every Trade event.
///
/// `volume` and `count` use `i64` because they accumulate across many trades
/// and can exceed `i32::MAX` for high-volume symbols (e.g. SPY). The Java
/// terminal uses `int` (32-bit) but silently wraps on overflow; we use `i64`
/// to avoid overflow entirely.
struct OhlcvcAccumulator {
    open: i32,
    high: i32,
    low: i32,
    close: i32,
    volume: i64,
    count: i64,
    price_type: i32,
    date: i32,
    ms_of_day: i32,
    initialized: bool,
}

impl OhlcvcAccumulator {
    fn new() -> Self {
        Self {
            open: 0,
            high: 0,
            low: 0,
            close: 0,
            volume: 0,
            count: 0,
            price_type: 0,
            date: 0,
            ms_of_day: 0,
            initialized: false,
        }
    }

    #[allow(clippy::too_many_arguments)] // Reason: OHLCVC bar has many fields from server init message
    fn init_from_server(
        &mut self,
        ms_of_day: i32,
        open: i32,
        high: i32,
        low: i32,
        close: i32,
        volume: i32,
        count: i32,
        price_type: i32,
        date: i32,
    ) {
        self.ms_of_day = ms_of_day;
        self.open = open;
        self.high = high;
        self.low = low;
        self.close = close;
        self.volume = i64::from(volume);
        self.count = i64::from(count);
        self.price_type = price_type;
        self.date = date;
        self.initialized = true;
    }

    fn process_trade(&mut self, ms_of_day: i32, price: i32, size: i32, price_type: i32, date: i32) {
        if self.initialized {
            self.ms_of_day = ms_of_day;
            let adjusted_price = change_price_type(price, price_type, self.price_type);
            self.volume += i64::from(size);
            self.count += 1;
            if adjusted_price > self.high {
                self.high = adjusted_price;
            }
            if adjusted_price < self.low {
                self.low = adjusted_price;
            }
            self.close = adjusted_price;
        } else {
            self.open = price;
            self.high = price;
            self.low = price;
            self.close = price;
            self.volume = i64::from(size);
            self.count = 1;
            self.price_type = price_type;
            self.date = date;
            self.ms_of_day = ms_of_day;
            self.initialized = true;
        }
    }
}

/// Convert a price from one `price_type` to another (mirrors Java PriceCalcUtils.changePriceType).
// Reason: protocol-defined integer widths from Java FPSS specification.
#[allow(clippy::cast_possible_truncation)]
fn change_price_type(price: i32, price_type: i32, new_price_type: i32) -> i32 {
    const POW10: [i32; 10] = [
        1,
        10,
        100,
        1_000,
        10_000,
        100_000,
        1_000_000,
        10_000_000,
        100_000_000,
        1_000_000_000,
    ];
    if price == 0 || price_type == new_price_type {
        return price;
    }
    let exp = new_price_type - price_type;
    if exp <= 0 {
        let idx = usize::try_from(-exp).unwrap_or(0);
        if idx < POW10.len() {
            price * POW10[idx]
        } else {
            price
        }
    } else {
        let idx = usize::try_from(exp).unwrap_or(0);
        if idx < POW10.len() {
            price / POW10[idx]
        } else {
            0
        }
    }
}

/// Per-contract, per-message-type delta decompression state.
///
/// FIT uses delta compression: the first tick for a contract is absolute,
/// subsequent ticks carry only the difference from the previous tick.
/// We maintain the last absolute values per `(msg_type, contract_id)`.
struct DeltaState {
    /// Key: `(StreamMsgType as u8, contract_id)`, Value: last absolute field values.
    prev: HashMap<(u8, i32), Vec<i32>>,
    /// Per-contract OHLCVC accumulators.
    ohlcvc: HashMap<i32, OhlcvcAccumulator>,
    /// Reusable scratch buffer for FIT decoding, avoiding per-tick allocation.
    /// Resized (never shrunk) to fit the largest tick type seen.
    alloc_buf: Vec<i32>,
    /// Set after `decode_tick` to indicate the last row was a DATE marker.
    /// Callers use this to distinguish normal DATE skips from corrupt payloads.
    last_was_date: bool,
    /// Actual data field count from the first absolute tick for each
    /// `(msg_type, contract_id)`. The dev server sends 8-field trades (simple
    /// format) while production sends 16-field trades (extended format).
    field_counts: HashMap<(u8, i32), usize>,
    /// Timestamp of last STOP (market close) signal. Used to suppress
    /// "unknown `contract_id`" warnings for 5 seconds after STOP, matching
    /// Java terminal behavior where stale ticks are expected during teardown.
    last_stop: Option<Instant>,
}

/// Duration after a STOP signal during which "unknown `contract_id`" warnings
/// are suppressed. The Java terminal silences these for 5 seconds.
const STOP_SUPPRESS_DURATION: Duration = Duration::from_secs(5);

impl DeltaState {
    fn new() -> Self {
        // Pre-allocate for the largest tick type (Trade = 16 fields + 1 contract_id).
        Self {
            prev: HashMap::new(),
            ohlcvc: HashMap::new(),
            alloc_buf: vec![0i32; TRADE_FIELDS + 1],
            last_was_date: false,
            field_counts: HashMap::new(),
            last_stop: None,
        }
    }

    /// Clear all accumulated delta state.
    ///
    /// Called on START/STOP (market open/close) signals to reset delta
    /// decompression, matching Java's behavior where `Tick.readID()` starts
    /// fresh after a session boundary.
    ///
    /// Note: `last_stop` is intentionally NOT cleared here because STOP
    /// itself calls `clear()`, and the timestamp must survive to suppress
    /// stale-tick warnings for 5 seconds after the STOP signal.
    fn clear(&mut self) {
        self.prev.clear();
        self.ohlcvc.clear();
        self.last_was_date = false;
        self.field_counts.clear();
    }

    /// Whether we are within the post-STOP suppression window.
    fn is_in_stop_suppression_window(&self) -> bool {
        self.last_stop
            .is_some_and(|t| t.elapsed() < STOP_SUPPRESS_DURATION)
    }

    /// Decode FIT payload and apply delta decompression.
    ///
    /// The ENTIRE payload is FIT-encoded. The first FIT field (alloc[0]) is the
    /// `contract_id`. Tick data fields start at alloc[1..].
    ///
    /// This matches the Java terminal's `FPSSClient` which calls:
    /// ```java
    /// fitReader.open(p.data(), 0, p.len());  // FIT starts at offset 0
    /// int size = fitReader.readChanges(alloc); // alloc[0] = contract_id
    /// Contract c = idToContract.get(alloc[0]); // first field IS the contract_id
    /// ```
    ///
    /// Returns `(contract_id, tick_fields, data_field_count)` or `None` if
    /// payload is too short or the FIT row is a DATE marker. Sets
    /// `self.last_was_date` so callers can distinguish DATE markers from
    /// corrupt payloads.
    fn decode_tick(
        &mut self,
        msg_code: u8,
        payload: &[u8],
        expected_fields: usize,
    ) -> Option<(i32, Vec<i32>, usize)> {
        self.last_was_date = false;

        if payload.is_empty() {
            return None;
        }

        // Reuse the scratch buffer: resize if needed (retains capacity),
        // then zero-fill the portion we need.
        let total_fields = expected_fields + 1;
        if self.alloc_buf.len() < total_fields {
            self.alloc_buf.resize(total_fields, 0);
        }
        self.alloc_buf[..total_fields].fill(0);

        let mut reader = FitReader::new(payload);
        let n = reader.read_changes(&mut self.alloc_buf[..total_fields]);

        if reader.is_date {
            // DATE marker row -- skip (no user-visible data).
            self.last_was_date = true;
            return None;
        }

        if n == 0 {
            return None;
        }

        // First FIT field is the contract_id.
        let contract_id = self.alloc_buf[0];

        // Tick data is alloc[1..]. Extract into its own vec.
        // This clone is unavoidable: we need to store a copy in the delta HashMap.
        let mut fields: Vec<i32> = self.alloc_buf[1..total_fields].to_vec();

        // Delta decompression applies only to the tick portion (excluding
        // contract_id), matching Java's `Tick.readID()`:
        //   for (int i = 1; i < len; ++i) {
        //       this.data[i - 1] = firstData[i] + this.data[i - 1];
        //   }
        // It skips firstData[0] (contract_id) and applies deltas from
        // firstData[1..] onto tick data[0..].
        let tick_n = n.saturating_sub(1);

        let key = (msg_code, contract_id);
        if let Some(prev) = self.prev.get(&key) {
            // Delta row: accumulate onto previous absolute values.
            apply_deltas(&mut fields, prev, tick_n);
        } else {
            // First absolute tick: record the actual field count.
            self.field_counts.insert(key, tick_n);
        }

        // Store as the new previous state (tick fields only, not contract_id).
        self.prev.insert(key, fields.clone());

        let data_fields = *self.field_counts.get(&key).unwrap_or(&expected_fields);
        Some((contract_id, fields, data_fields))
    }
}

/// Decode a frame into zero, one, or two `FpssEvent`s.
///
/// Returns `(primary, secondary)` where `secondary` is only `Some` for Trade
/// frames that also produce a derived OHLCVC event. This eliminates the
/// per-frame `Vec<FpssEvent>` allocation that was on the hot path.
///
/// This is the frame dispatch logic from `FPSSClient.java`'s reader thread.
/// Tick data frames (Quote, Trade, `OpenInterest`, Ohlcvc) are FIT-decoded and
/// delta-decompressed before being emitted as typed events.
// Reason: FPSS protocol uses Java-defined integer widths; frame decode is inherently large.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::needless_pass_by_value
)]
fn decode_frame(
    code: StreamMsgType,
    payload: &[u8],
    authenticated: &AtomicBool,
    contract_map: &Mutex<HashMap<i32, Contract>>,
    symbol_cache: &Mutex<HashMap<i32, Arc<str>>>,
    shutdown: &AtomicBool,
    delta_state: &mut DeltaState,
    derive_ohlcvc: bool,
) -> (Option<FpssEvent>, Option<FpssEvent>) {
    // Capture wall-clock timestamp once per frame for all data variants.
    let received_at_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    // Resolve contract_id to a symbol string from the pre-rendered cache.
    // Returns Arc::clone of the cached symbol, or the empty-string sentinel.
    // Zero allocation on the hot path -- the Arc<str> was built once in
    // the ContractAssigned handler below.
    let resolve_symbol = |contract_id: i32| -> Arc<str> {
        symbol_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&contract_id)
            .map(Arc::clone)
            .unwrap_or_else(|| Arc::clone(&EMPTY_SYMBOL))
    };

    // Log a warning when ticks arrive for contract IDs not in the map,
    // but suppress for 5 seconds after STOP (market close) since stale
    // ticks are expected during teardown. Matches Java terminal behavior.
    let warn_unknown_contract = |contract_id: i32, kind: &str, delta_state: &DeltaState| {
        let known = contract_map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&contract_id);
        if !known && !delta_state.is_in_stop_suppression_window() {
            tracing::warn!(contract_id, kind, "no contract for ID");
        }
    };

    match code {
        StreamMsgType::Metadata => {
            // Can arrive again after reconnection
            let permissions = String::from_utf8_lossy(payload).to_string();
            tracing::debug!(permissions = %permissions, "received METADATA");
            authenticated.store(true, Ordering::Release);
            (
                Some(FpssEvent::Control(FpssControl::LoginSuccess {
                    permissions,
                })),
                None,
            )
        }

        StreamMsgType::Contract => match parse_contract_message(payload) {
            Ok((id, contract)) => {
                tracing::debug!(id, contract = %contract, "contract assigned");
                // Pre-render the symbol string once and cache as Arc<str>
                // so resolve_symbol() on the hot path is just Arc::clone.
                let symbol_str: Arc<str> = Arc::from(contract.to_string().as_str());
                symbol_cache
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(id, symbol_str);
                contract_map
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(id, contract.clone());
                (
                    Some(FpssEvent::Control(FpssControl::ContractAssigned {
                        id,
                        contract,
                    })),
                    None,
                )
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse CONTRACT message");
                (
                    Some(FpssEvent::Control(FpssControl::Error {
                        message: format!("failed to parse CONTRACT message: {e}"),
                    })),
                    None,
                )
            }
        },

        StreamMsgType::Quote => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, QUOTE_FIELDS) {
                Some((contract_id, f, _n)) => {
                    warn_unknown_contract(contract_id, "quote", delta_state);
                    metrics::counter!("thetadatadx.fpss.events", "kind" => "quote").increment(1);
                    let pt = f[9];
                    (
                        Some(FpssEvent::Data(FpssData::Quote {
                            contract_id,
                            symbol: resolve_symbol(contract_id),
                            ms_of_day: f[0],
                            bid_size: f[1],
                            bid_exchange: f[2],
                            bid: Price::new(f[3], pt).to_f64(),
                            bid_condition: f[4],
                            ask_size: f[5],
                            ask_exchange: f[6],
                            ask: Price::new(f[7], pt).to_f64(),
                            ask_condition: f[8],
                            date: f[10],
                            received_at_ns,
                        })),
                        None,
                    )
                }
                // DATE markers return None from decode_tick -- this is normal
                // protocol flow (session date boundary), not corruption.
                None if delta_state.last_was_date => (None, None),
                None => (
                    Some(FpssEvent::RawData {
                        code: code as u8,
                        payload: payload.to_vec(),
                    }),
                    None,
                ),
            }
        }

        StreamMsgType::Trade => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, TRADE_FIELDS) {
                Some((contract_id, f, n_data)) => {
                    warn_unknown_contract(contract_id, "trade", delta_state);
                    metrics::counter!("thetadatadx.fpss.events", "kind" => "trade").increment(1);

                    if n_data != 8 && n_data != TRADE_FIELDS {
                        tracing::warn!(
                            contract_id,
                            n_data,
                            "unexpected trade field count (expected 8 or 16)"
                        );
                    }

                    // 8-field: [ms_of_day, sequence, size, condition, price, exchange, price_type, date]
                    // 16-field: [ms_of_day, sequence, ext1..ext4, condition, size, exchange, price, cond_flags, price_flags, vol_type, records_back, price_type, date]
                    let sym = resolve_symbol(contract_id);
                    let trade_event = if n_data <= 8 {
                        let pt = f[6];
                        FpssEvent::Data(FpssData::Trade {
                            contract_id,
                            symbol: Arc::clone(&sym),
                            ms_of_day: f[0],
                            sequence: f[1],
                            ext_condition1: 0,
                            ext_condition2: 0,
                            ext_condition3: 0,
                            ext_condition4: 0,
                            condition: f[3],
                            size: f[2],
                            exchange: f[5],
                            price: Price::new(f[4], pt).to_f64(),
                            condition_flags: 0,
                            price_flags: 0,
                            volume_type: 0,
                            records_back: 0,
                            date: f[7],
                            received_at_ns,
                        })
                    } else {
                        let pt = f[14];
                        FpssEvent::Data(FpssData::Trade {
                            contract_id,
                            symbol: Arc::clone(&sym),
                            ms_of_day: f[0],
                            sequence: f[1],
                            ext_condition1: f[2],
                            ext_condition2: f[3],
                            ext_condition3: f[4],
                            ext_condition4: f[5],
                            condition: f[6],
                            size: f[7],
                            exchange: f[8],
                            price: Price::new(f[9], pt).to_f64(),
                            condition_flags: f[10],
                            price_flags: f[11],
                            volume_type: f[12],
                            records_back: f[13],
                            date: f[15],
                            received_at_ns,
                        })
                    };

                    // Extract for OHLCVC derivation (format-aware)
                    let (ms_of_day, size, price, price_type, date) = if n_data <= 8 {
                        (f[0], f[2], f[4], f[6], f[7])
                    } else {
                        (f[0], f[7], f[9], f[14], f[15])
                    };

                    // Derive OHLCVC from trade (Java: OHLCVC.processTrade).
                    // Only if enabled AND the server has already seeded a bar.
                    // When derive_ohlcvc is false, skip entirely — zero overhead.
                    let ohlcvc_event = if derive_ohlcvc {
                        if let Some(acc) = delta_state.ohlcvc.get_mut(&contract_id) {
                            if acc.initialized {
                                acc.process_trade(ms_of_day, price, size, price_type, date);
                                let apt = acc.price_type;
                                Some(FpssEvent::Data(FpssData::Ohlcvc {
                                    contract_id,
                                    symbol: Arc::clone(&sym),
                                    ms_of_day: acc.ms_of_day,
                                    open: Price::new(acc.open, apt).to_f64(),
                                    high: Price::new(acc.high, apt).to_f64(),
                                    low: Price::new(acc.low, apt).to_f64(),
                                    close: Price::new(acc.close, apt).to_f64(),
                                    volume: acc.volume,
                                    count: acc.count,
                                    date: acc.date,
                                    received_at_ns,
                                }))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    (Some(trade_event), ohlcvc_event)
                }
                // DATE markers return None from decode_tick -- normal protocol flow.
                None if delta_state.last_was_date => (None, None),
                None => (
                    Some(FpssEvent::RawData {
                        code: code as u8,
                        payload: payload.to_vec(),
                    }),
                    None,
                ),
            }
        }

        StreamMsgType::OpenInterest => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, OI_FIELDS) {
                Some((contract_id, f, _n)) => {
                    warn_unknown_contract(contract_id, "open_interest", delta_state);
                    metrics::counter!("thetadatadx.fpss.events", "kind" => "open_interest")
                        .increment(1);
                    (
                        Some(FpssEvent::Data(FpssData::OpenInterest {
                            contract_id,
                            symbol: resolve_symbol(contract_id),
                            ms_of_day: f[0],
                            open_interest: f[1],
                            date: f[2],
                            received_at_ns,
                        })),
                        None,
                    )
                }
                None if delta_state.last_was_date => (None, None),
                None => (
                    Some(FpssEvent::RawData {
                        code: code as u8,
                        payload: payload.to_vec(),
                    }),
                    None,
                ),
            }
        }

        StreamMsgType::Ohlcvc => {
            let msg_code = code as u8;
            match delta_state.decode_tick(msg_code, payload, OHLCVC_FIELDS) {
                Some((contract_id, f, _n)) => {
                    warn_unknown_contract(contract_id, "ohlcvc", delta_state);
                    metrics::counter!("thetadatadx.fpss.events", "kind" => "ohlcvc").increment(1);
                    let acc = delta_state
                        .ohlcvc
                        .entry(contract_id)
                        .or_insert_with(OhlcvcAccumulator::new);
                    acc.init_from_server(f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], f[8]);
                    let pt = f[7];
                    (
                        Some(FpssEvent::Data(FpssData::Ohlcvc {
                            contract_id,
                            symbol: resolve_symbol(contract_id),
                            ms_of_day: f[0],
                            open: Price::new(f[1], pt).to_f64(),
                            high: Price::new(f[2], pt).to_f64(),
                            low: Price::new(f[3], pt).to_f64(),
                            close: Price::new(f[4], pt).to_f64(),
                            volume: i64::from(f[5]),
                            count: i64::from(f[6]),
                            date: f[8],
                            received_at_ns,
                        })),
                        None,
                    )
                }
                None if delta_state.last_was_date => (None, None),
                None => (
                    Some(FpssEvent::RawData {
                        code: code as u8,
                        payload: payload.to_vec(),
                    }),
                    None,
                ),
            }
        }

        StreamMsgType::ReqResponse => match parse_req_response(payload) {
            Ok((req_id, result)) => {
                tracing::debug!(req_id, result = ?result, "subscription response");
                (
                    Some(FpssEvent::Control(FpssControl::ReqResponse {
                        req_id,
                        result,
                    })),
                    None,
                )
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse REQ_RESPONSE");
                (
                    Some(FpssEvent::Control(FpssControl::Error {
                        message: format!("failed to parse REQ_RESPONSE: {e}"),
                    })),
                    None,
                )
            }
        },

        StreamMsgType::Start => {
            tracing::info!("market open signal received");
            delta_state.clear();
            contract_map
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear(); // Java: idToContract.clear()
            (Some(FpssEvent::Control(FpssControl::MarketOpen)), None)
        }

        StreamMsgType::Stop => {
            tracing::info!("market close signal received");
            delta_state.last_stop = Some(Instant::now());
            delta_state.clear();
            contract_map
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear(); // Java: idToContract.clear()
            (Some(FpssEvent::Control(FpssControl::MarketClose)), None)
        }

        StreamMsgType::Error => {
            // The dev server's replay loop boundary leaks FIT tick data into
            // Error frames. Detect binary content and skip silently -- these
            // are not real errors, just replay artifacts. Matches Java terminal
            // behavior (logs and ignores).
            let is_binary = framing::is_binary_payload(payload);
            if is_binary {
                tracing::debug!(
                    len = payload.len(),
                    "skipping binary Error frame (replay boundary artifact)"
                );
                (None, None)
            } else {
                let message = String::from_utf8_lossy(payload).to_string();
                tracing::warn!(message = %message, "server error");
                (
                    Some(FpssEvent::Control(FpssControl::ServerError { message })),
                    None,
                )
            }
        }

        StreamMsgType::Disconnected => {
            let reason = parse_disconnect_reason(payload);
            tracing::warn!(reason = ?reason, "server disconnected us");
            metrics::counter!("thetadatadx.fpss.disconnects", "reason" => format!("{:?}", reason))
                .increment(1);
            authenticated.store(false, Ordering::Release);

            // Permanent errors -- no reconnect will fix these.
            if reconnect_delay(reason).is_none() {
                tracing::error!(reason = ?reason, "permanent disconnect -- stopping");
                shutdown.store(true, Ordering::Release);
            }

            (
                Some(FpssEvent::Control(FpssControl::Disconnected { reason })),
                None,
            )
        }

        // Ignore frame types we don't handle (e.g., server sending PING)
        other => {
            tracing::trace!(code = ?other, "ignoring unhandled frame type");
            (None, None)
        }
    }
}

// ---------------------------------------------------------------------------
// Ping heartbeat loop
// ---------------------------------------------------------------------------

/// Background thread that sends PING heartbeat every 100ms via the command channel.
///
/// # Behavior (from `FPSSClient.java`)
///
/// After successful login, the Java client starts a thread that sends:
/// - Code 10 (PING)
/// - 1-byte payload: `[0x00]`
/// - Every 100ms
///
/// Source: `FPSSClient.java` heartbeat thread, interval = 100ms.
// Reason: all parameters are moved into this function from a spawned thread closure.
#[allow(clippy::needless_pass_by_value)]
fn ping_loop(
    cmd_tx: std_mpsc::Sender<IoCommand>,
    shutdown: Arc<AtomicBool>,
    authenticated: Arc<AtomicBool>,
) {
    let interval = Duration::from_millis(PING_INTERVAL_MS);
    let ping_payload = build_ping_payload();

    // Java: scheduleAtFixedRate(task, 2000L, 100L) — first execution at 2000ms,
    // then every 100ms. scheduleAtFixedRate sends THEN waits, so the first ping
    // fires at exactly 2000ms.
    thread::sleep(Duration::from_millis(2000));

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        if !authenticated.load(Ordering::Relaxed) {
            // Don't send pings if not authenticated
            thread::sleep(interval);
            continue;
        }

        // Send ping FIRST, then sleep — matches Java's scheduleAtFixedRate
        // which executes the task then waits the interval.
        let cmd = IoCommand::WriteFrame {
            code: StreamMsgType::Ping,
            payload: ping_payload.clone(),
        };
        if cmd_tx.send(cmd).is_err() {
            // I/O thread has exited
            break;
        }

        thread::sleep(interval);
    }

    tracing::debug!("fpss-ping thread exiting");
}

// ---------------------------------------------------------------------------
// Reconnection helper
// ---------------------------------------------------------------------------

/// Reconnect an FPSS client after a disconnect.
///
/// # Behavior (from `FPSSClient.java`)
///
/// 1. Wait `delay_ms` before attempting reconnection
/// 2. Establish a new TLS connection
/// 3. Re-authenticate
/// 4. Re-subscribe all previously active subscriptions with `req_id = -1`
///
/// On `TOO_MANY_REQUESTS`: wait 130 seconds before reconnecting.
/// On `ACCOUNT_ALREADY_CONNECTED`: do NOT reconnect (permanent error).
///
/// Source: `FPSSClient.java` reconnection logic in the main loop.
#[allow(clippy::too_many_arguments)]
// Reason: missing errors doc for internal function.
#[allow(clippy::missing_errors_doc)]
pub fn reconnect<F>(
    creds: &Credentials,
    hosts: &[(String, u16)],
    previous_subs: Vec<(SubscriptionKind, Contract)>,
    previous_full_subs: Vec<(SubscriptionKind, tdbe::types::enums::SecType)>,
    delay_ms: u64,
    ring_size: usize,
    flush_mode: FpssFlushMode,
    policy: ReconnectPolicy,
    derive_ohlcvc: bool,
    handler: F,
) -> Result<FpssClient, Error>
where
    F: FnMut(&FpssEvent) + Send + 'static,
{
    tracing::info!(delay_ms, "waiting before FPSS reconnection");
    thread::sleep(Duration::from_millis(delay_ms));

    let client = FpssClient::connect(
        creds,
        hosts,
        ring_size,
        flush_mode,
        policy,
        derive_ohlcvc,
        handler,
    )?;

    // Re-subscribe all previous per-contract subscriptions with req_id = -1
    // Source: FPSSClient.java -- reconnect logic uses req_id = -1 for re-subscriptions
    for (kind, contract) in &previous_subs {
        let payload = build_subscribe_payload(-1, contract);
        let code = kind.subscribe_code();

        client
            .cmd_tx
            .send(IoCommand::WriteFrame { code, payload })
            .map_err(|_| Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "I/O thread exited during reconnect".to_string(),
            })?;

        tracing::debug!(
            kind = ?kind,
            contract = %contract,
            "re-subscribed after reconnect (req_id=-1)"
        );
    }

    // Re-subscribe all previous full-type (firehose) subscriptions with req_id = -1
    for (kind, sec_type) in &previous_full_subs {
        let payload = protocol::build_full_type_subscribe_payload(-1, *sec_type);
        let code = kind.subscribe_code();

        client
            .cmd_tx
            .send(IoCommand::WriteFrame { code, payload })
            .map_err(|_| Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "I/O thread exited during reconnect".to_string(),
            })?;

        tracing::debug!(
            kind = ?kind,
            sec_type = ?sec_type,
            "re-subscribed full-type after reconnect (req_id=-1)"
        );
    }

    // Store the re-subscribed lists
    {
        let mut subs = client
            .active_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *subs = previous_subs;
    }
    {
        let mut subs = client
            .active_full_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *subs = previous_full_subs;
    }

    Ok(client)
}

/// Determine the reconnect delay based on the disconnect reason.
///
/// Source: `FPSSClient.java` -- reconnect logic checks `RemoveReason` to decide delay.
///
/// # Intentional divergence from Java (see jvm-deviations.md)
///
/// Java only treats `AccountAlreadyConnected` (code 6) as a permanent error,
/// retrying forever on invalid credentials — which burns rate limits and never
/// succeeds. We treat all 7 credential/account error codes as permanent because
/// no amount of retrying will fix bad credentials. This is a deliberate
/// improvement over the Java behavior.
#[must_use]
pub fn reconnect_delay(reason: RemoveReason) -> Option<u64> {
    match reason {
        // Permanent errors -- no amount of reconnection will fix bad credentials.
        // Java only checks AccountAlreadyConnected here; we extend this to all
        // credential errors. See jvm-deviations.md "Permanent Disconnect".
        RemoveReason::AccountAlreadyConnected
        | RemoveReason::InvalidCredentials
        | RemoveReason::InvalidLoginValues
        | RemoveReason::InvalidLoginSize
        | RemoveReason::FreeAccount
        | RemoveReason::ServerUserDoesNotExist
        | RemoveReason::InvalidCredentialsNullUser => None,
        RemoveReason::TooManyRequests => Some(TOO_MANY_REQUESTS_DELAY_MS),
        _ => Some(RECONNECT_DELAY_MS),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_delay_permanent() {
        // All credential / account errors are permanent -- no reconnect.
        assert_eq!(reconnect_delay(RemoveReason::AccountAlreadyConnected), None);
        assert_eq!(reconnect_delay(RemoveReason::InvalidCredentials), None);
        assert_eq!(reconnect_delay(RemoveReason::InvalidLoginValues), None);
        assert_eq!(reconnect_delay(RemoveReason::InvalidLoginSize), None);
        assert_eq!(reconnect_delay(RemoveReason::FreeAccount), None);
        assert_eq!(reconnect_delay(RemoveReason::ServerUserDoesNotExist), None);
        assert_eq!(
            reconnect_delay(RemoveReason::InvalidCredentialsNullUser),
            None
        );
    }

    #[test]
    fn reconnect_delay_too_many_requests() {
        assert_eq!(
            reconnect_delay(RemoveReason::TooManyRequests),
            Some(130_000)
        );
    }

    #[test]
    fn reconnect_delay_normal() {
        assert_eq!(reconnect_delay(RemoveReason::ServerRestarting), Some(2_000));
        assert_eq!(reconnect_delay(RemoveReason::Unspecified), Some(2_000));
        assert_eq!(reconnect_delay(RemoveReason::TimedOut), Some(2_000));
    }

    #[test]
    fn fpss_event_default_exists() {
        let _evt: FpssEvent = Default::default();
    }

    #[test]
    fn reconnect_policy_default_is_auto() {
        let policy: ReconnectPolicy = Default::default();
        assert!(matches!(policy, ReconnectPolicy::Auto));
    }

    #[test]
    fn reconnect_policy_custom_works() {
        let policy = ReconnectPolicy::Custom(std::sync::Arc::new(|reason, attempt| {
            if attempt > 3 {
                return None;
            }
            match reason {
                RemoveReason::TooManyRequests => Some(Duration::from_secs(60)),
                _ => Some(Duration::from_secs(1)),
            }
        }));
        if let ReconnectPolicy::Custom(f) = &policy {
            assert_eq!(f(RemoveReason::TimedOut, 1), Some(Duration::from_secs(1)));
            assert_eq!(
                f(RemoveReason::TooManyRequests, 2),
                Some(Duration::from_secs(60))
            );
            assert_eq!(f(RemoveReason::TimedOut, 4), None);
        } else {
            panic!("expected Custom");
        }
    }

    #[test]
    fn fpss_control_reconnecting_variant() {
        let evt = FpssEvent::Control(FpssControl::Reconnecting {
            reason: RemoveReason::ServerRestarting,
            attempt: 1,
            delay_ms: 2000,
        });
        if let FpssEvent::Control(FpssControl::Reconnecting {
            reason,
            attempt,
            delay_ms,
        }) = &evt
        {
            assert_eq!(*reason, RemoveReason::ServerRestarting);
            assert_eq!(*attempt, 1);
            assert_eq!(*delay_ms, 2000);
        } else {
            panic!("expected Reconnecting");
        }
    }

    #[test]
    fn fpss_control_reconnected_variant() {
        let evt = FpssEvent::Control(FpssControl::Reconnected);
        assert!(matches!(&evt, FpssEvent::Control(FpssControl::Reconnected)));
    }

    #[test]
    fn max_reconnect_attempts_is_5() {
        assert_eq!(MAX_RECONNECT_ATTEMPTS, 5);
    }

    #[test]
    fn ohlcvc_accumulator_first_trade_initializes() {
        let mut acc = OhlcvcAccumulator::new();
        assert!(!acc.initialized);
        acc.process_trade(34200000, 15025, 100, 8, 20240315);
        assert!(acc.initialized);
        assert_eq!(acc.open, 15025);
        assert_eq!(acc.high, 15025);
        assert_eq!(acc.low, 15025);
        assert_eq!(acc.close, 15025);
        assert_eq!(acc.volume, 100);
        assert_eq!(acc.count, 1);
    }

    #[test]
    fn ohlcvc_accumulator_updates() {
        let mut acc = OhlcvcAccumulator::new();
        acc.process_trade(34200000, 15025, 100, 8, 20240315);
        acc.process_trade(34200100, 15100, 200, 8, 20240315);
        acc.process_trade(34200200, 14950, 50, 8, 20240315);
        assert_eq!(acc.open, 15025);
        assert_eq!(acc.high, 15100);
        assert_eq!(acc.low, 14950);
        assert_eq!(acc.close, 14950);
        assert_eq!(acc.volume, 350);
        assert_eq!(acc.count, 3);
    }

    #[test]
    fn ohlcvc_accumulator_server_init_then_trade() {
        let mut acc = OhlcvcAccumulator::new();
        acc.init_from_server(34200000, 15000, 15100, 14900, 15050, 1000, 10, 8, 20240315);
        acc.process_trade(34200300, 15200, 50, 8, 20240315);
        assert_eq!(acc.high, 15200);
        assert_eq!(acc.low, 14900);
        assert_eq!(acc.volume, 1050);
        assert_eq!(acc.count, 11);
    }

    #[test]
    fn ohlcvc_accumulator_no_overflow_on_high_volume() {
        let mut acc = OhlcvcAccumulator::new();
        acc.process_trade(34200000, 15025, i32::MAX, 8, 20240315);
        acc.process_trade(34200100, 15100, i32::MAX, 8, 20240315);
        // Would overflow i32 (2 * 2_147_483_647 = 4_294_967_294), fine in i64
        assert_eq!(acc.volume, 2 * i64::from(i32::MAX));
        assert_eq!(acc.count, 2);
    }

    #[test]
    fn change_price_type_tests() {
        assert_eq!(change_price_type(15025, 8, 8), 15025);
        assert_eq!(change_price_type(15025, 8, 7), 150250);
        assert_eq!(change_price_type(150250, 7, 8), 15025);
        assert_eq!(change_price_type(0, 8, 7), 0);
    }

    #[test]
    fn fpss_event_split_data_control() {
        let data_evt = FpssEvent::Data(FpssData::Trade {
            contract_id: 42,
            symbol: Arc::from(""),
            ms_of_day: 0,
            sequence: 0,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: 100,
            exchange: 0,
            price: Price::new(15025, 8).to_f64(),
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            date: 20240315,
            received_at_ns: 0,
        });
        match &data_evt {
            FpssEvent::Data(FpssData::Trade {
                contract_id, price, ..
            }) => {
                assert_eq!(*contract_id, 42);
                assert!((*price - 150.25).abs() < f64::EPSILON);
            }
            other => panic!("expected Data(Trade), got {other:?}"),
        }
        let ctrl = FpssEvent::Control(FpssControl::MarketOpen);
        assert!(matches!(&ctrl, FpssEvent::Control(FpssControl::MarketOpen)));
    }

    // -----------------------------------------------------------------------
    // FIT encoding helpers for trade mapping tests
    // -----------------------------------------------------------------------

    const FIELD_SEP: u8 = 0xB;
    const END_NIB: u8 = 0xD;
    const NEG_NIB: u8 = 0xE;

    /// Collect the decimal digits of an absolute i32 value as nibbles.
    /// Pushes a NEGATIVE nibble first if the value is negative.
    fn int_to_nibbles(val: i32) -> Vec<u8> {
        let mut nibbles = Vec::new();
        if val < 0 {
            nibbles.push(NEG_NIB);
        }
        let abs = (val as i64).unsigned_abs();
        if abs == 0 {
            nibbles.push(0);
            return nibbles;
        }
        let s = abs.to_string();
        for ch in s.chars() {
            nibbles.push(ch.to_digit(10).unwrap() as u8);
        }
        nibbles
    }

    /// Encode a slice of i32 values into a FIT byte buffer.
    /// Fields are separated by FIELD_SEP, terminated by END.
    fn encode_fit_row(fields: &[i32]) -> Vec<u8> {
        let mut nibbles: Vec<u8> = Vec::new();
        for (i, &val) in fields.iter().enumerate() {
            if i > 0 {
                nibbles.push(FIELD_SEP);
            }
            nibbles.extend(int_to_nibbles(val));
        }
        nibbles.push(END_NIB);

        // Pack nibbles into bytes (two per byte). Pad with 0 nibble if odd.
        let mut bytes = Vec::new();
        let mut i = 0;
        while i < nibbles.len() {
            let high = nibbles[i];
            let low = if i + 1 < nibbles.len() {
                nibbles[i + 1]
            } else {
                0
            };
            bytes.push((high << 4) | (low & 0x0F));
            i += 2;
        }
        bytes
    }

    // -----------------------------------------------------------------------
    // 8-field trade mapping
    // -----------------------------------------------------------------------

    #[test]
    fn decode_tick_8field_trade_returns_correct_n_data_and_fields() {
        // 8-field trade layout (dev server format):
        //   FIT fields: [contract_id, ms_of_day, sequence, size, condition,
        //                price, exchange, price_type, date]
        //   = 1 contract_id + 8 data fields = 9 FIT fields total
        let fit_payload = encode_fit_row(&[
            100,      // contract_id
            34200000, // ms_of_day
            12345,    // sequence
            50,       // size
            6,        // condition
            5500000,  // price
            57,       // exchange
            6,        // price_type
            20250428, // date
        ]);

        let mut ds = DeltaState::new();
        let msg_code = StreamMsgType::Trade as u8;
        let result = ds.decode_tick(msg_code, &fit_payload, TRADE_FIELDS);

        let (contract_id, f, n_data) = result.expect("decode_tick should succeed");

        // Verify contract_id extraction.
        assert_eq!(contract_id, 100);

        // The first absolute tick records the actual field count.
        // 9 FIT fields total - 1 contract_id = 8 data fields.
        assert_eq!(n_data, 8, "n_data must be 8 for an 8-field trade");

        // Verify 8-field mapping produces correct Trade event fields.
        // 8-field layout: [ms_of_day, sequence, size, condition, price, exchange, price_type, date]
        assert_eq!(f[0], 34200000, "ms_of_day");
        assert_eq!(f[1], 12345, "sequence");
        assert_eq!(f[2], 50, "size");
        assert_eq!(f[3], 6, "condition");
        assert_eq!(f[4], 5500000, "price");
        assert_eq!(f[5], 57, "exchange");
        assert_eq!(f[6], 6, "price_type");
        assert_eq!(f[7], 20250428, "date");

        // Verify the n_data <= 8 mapping path produces the correct Trade variant.
        assert!(n_data <= 8);
        // Simulate the mapping from decode_frame's Trade arm:
        let trade = FpssData::Trade {
            contract_id,
            symbol: Arc::from(""),
            ms_of_day: f[0],
            sequence: f[1],
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: f[3],
            size: f[2],
            exchange: f[5],
            price: Price::new(f[4], f[6]).to_f64(),
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            date: f[7],
            received_at_ns: 0,
        };

        match trade {
            FpssData::Trade {
                contract_id: cid,
                ms_of_day,
                sequence,
                size,
                condition,
                price,
                exchange,
                date,
                ext_condition1,
                ext_condition2,
                ext_condition3,
                ext_condition4,
                condition_flags,
                price_flags,
                volume_type,
                records_back,
                ..
            } => {
                assert_eq!(cid, 100);
                assert_eq!(ms_of_day, 34200000);
                assert_eq!(sequence, 12345);
                assert_eq!(size, 50);
                assert_eq!(condition, 6);
                assert_eq!(price, Price::new(5500000, 6).to_f64());
                assert_eq!(exchange, 57);
                assert_eq!(date, 20250428);
                // 8-field trades zero out extended fields.
                assert_eq!(ext_condition1, 0);
                assert_eq!(ext_condition2, 0);
                assert_eq!(ext_condition3, 0);
                assert_eq!(ext_condition4, 0);
                assert_eq!(condition_flags, 0);
                assert_eq!(price_flags, 0);
                assert_eq!(volume_type, 0);
                assert_eq!(records_back, 0);
            }
            other => panic!("expected Trade, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // 16-field trade mapping
    // -----------------------------------------------------------------------

    #[test]
    fn decode_tick_16field_trade_returns_correct_n_data_and_fields() {
        // 16-field trade layout (production format):
        //   FIT fields: [contract_id, ms_of_day, sequence, ext1, ext2, ext3, ext4,
        //                condition, size, exchange, price, cond_flags, price_flags,
        //                vol_type, records_back, price_type, date]
        //   = 1 contract_id + 16 data fields = 17 FIT fields total
        let fit_payload = encode_fit_row(&[
            200,      // contract_id
            34200000, // ms_of_day (f[0])
            99999,    // sequence  (f[1])
            1,        // ext_condition1 (f[2])
            2,        // ext_condition2 (f[3])
            3,        // ext_condition3 (f[4])
            4,        // ext_condition4 (f[5])
            15,       // condition (f[6])
            500,      // size (f[7])
            57,       // exchange (f[8])
            18750000, // price (f[9])
            7,        // condition_flags (f[10])
            3,        // price_flags (f[11])
            1,        // volume_type (f[12])
            0,        // records_back (f[13])
            8,        // price_type (f[14])
            20250428, // date (f[15])
        ]);

        let mut ds = DeltaState::new();
        let msg_code = StreamMsgType::Trade as u8;
        let result = ds.decode_tick(msg_code, &fit_payload, TRADE_FIELDS);

        let (contract_id, f, n_data) = result.expect("decode_tick should succeed");

        // Verify contract_id extraction.
        assert_eq!(contract_id, 200);

        // 17 FIT fields total - 1 contract_id = 16 data fields.
        assert_eq!(n_data, 16, "n_data must be 16 for a 16-field trade");
        assert_eq!(n_data, TRADE_FIELDS);

        // Verify all 16 data fields.
        assert_eq!(f[0], 34200000, "ms_of_day");
        assert_eq!(f[1], 99999, "sequence");
        assert_eq!(f[2], 1, "ext_condition1");
        assert_eq!(f[3], 2, "ext_condition2");
        assert_eq!(f[4], 3, "ext_condition3");
        assert_eq!(f[5], 4, "ext_condition4");
        assert_eq!(f[6], 15, "condition");
        assert_eq!(f[7], 500, "size");
        assert_eq!(f[8], 57, "exchange");
        assert_eq!(f[9], 18750000, "price");
        assert_eq!(f[10], 7, "condition_flags");
        assert_eq!(f[11], 3, "price_flags");
        assert_eq!(f[12], 1, "volume_type");
        assert_eq!(f[13], 0, "records_back");
        assert_eq!(f[14], 8, "price_type");
        assert_eq!(f[15], 20250428, "date");

        // Verify the n_data > 8 mapping path produces the correct Trade variant.
        assert!(n_data > 8);
        let trade = FpssData::Trade {
            contract_id,
            symbol: Arc::from(""),
            ms_of_day: f[0],
            sequence: f[1],
            ext_condition1: f[2],
            ext_condition2: f[3],
            ext_condition3: f[4],
            ext_condition4: f[5],
            condition: f[6],
            size: f[7],
            exchange: f[8],
            price: Price::new(f[9], f[14]).to_f64(),
            condition_flags: f[10],
            price_flags: f[11],
            volume_type: f[12],
            records_back: f[13],
            date: f[15],
            received_at_ns: 0,
        };

        match trade {
            FpssData::Trade {
                contract_id: cid,
                ms_of_day,
                sequence,
                ext_condition1,
                ext_condition2,
                ext_condition3,
                ext_condition4,
                condition,
                size,
                exchange,
                price,
                condition_flags,
                price_flags,
                volume_type,
                records_back,
                date,
                ..
            } => {
                assert_eq!(cid, 200);
                assert_eq!(ms_of_day, 34200000);
                assert_eq!(sequence, 99999);
                assert_eq!(ext_condition1, 1);
                assert_eq!(ext_condition2, 2);
                assert_eq!(ext_condition3, 3);
                assert_eq!(ext_condition4, 4);
                assert_eq!(condition, 15);
                assert_eq!(size, 500);
                assert_eq!(exchange, 57);
                assert_eq!(price, Price::new(18750000, 8).to_f64());
                assert_eq!(condition_flags, 7);
                assert_eq!(price_flags, 3);
                assert_eq!(volume_type, 1);
                assert_eq!(records_back, 0);
                assert_eq!(date, 20250428);
            }
            other => panic!("expected Trade, got {other:?}"),
        }
    }
}
