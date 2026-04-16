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

mod accumulator;
pub mod connection;
mod decode;
mod delta;
mod events;
pub mod framing;
pub mod protocol;
pub mod ring;

use self::decode::decode_frame;
use self::delta::DeltaState;
use self::events::IoCommand;
pub use self::events::{FpssControl, FpssData, FpssEvent};

use std::collections::HashMap;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use disruptor::{build_single_producer, Producer, Sequence};

use self::ring::{AdaptiveWaitStrategy, RingEvent};

use crate::auth::Credentials;
use crate::config::{FpssFlushMode, ReconnectPolicy};
use crate::error::Error;
use tdbe::types::enums::{RemoveReason, StreamMsgType};

use self::framing::{
    read_frame, read_frame_into, write_frame, write_raw_frame, write_raw_frame_no_flush, Frame,
};
use self::protocol::{
    build_credentials_payload, build_ping_payload, build_subscribe_payload,
    parse_disconnect_reason, Contract, SubscriptionKind, PING_INTERVAL_MS, RECONNECT_DELAY_MS,
    TOO_MANY_REQUESTS_DELAY_MS,
};

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
    active_subs: Mutex<Vec<(SubscriptionKind, Contract)>>,
    /// Active full-type (firehose) subscriptions for reconnection.
    active_full_subs: Mutex<Vec<(SubscriptionKind, tdbe::types::enums::SecType)>>,
    /// Server-assigned contract ID mapping.
    contract_map: Arc<Mutex<HashMap<i32, Contract>>>,
    /// The server address we connected to.
    server_addr: String,
}

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

        // Command channel: FpssClient -> I/O thread
        let (cmd_tx, cmd_rx) = std_mpsc::channel::<IoCommand>();

        // Ping command channel: ping thread -> I/O thread
        let ping_cmd_tx = cmd_tx.clone();

        // Spawn the I/O thread: blocking TLS read + Disruptor publish + command drain.
        let io_shutdown = Arc::clone(&shutdown);
        let io_authenticated = Arc::clone(&authenticated);
        let io_contract_map = Arc::clone(&contract_map);
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
            cmd_tx: Mutex::new(cmd_tx),
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
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = build_subscribe_payload(req_id, contract);
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
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = build_subscribe_payload(req_id, contract);
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
    /// Source: `FPSSClient.disconnect()` -- sends STOP (code 32), then closes socket.
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

    /// Send a command to the I/O thread. Maps channel-send failure to a
    /// `Disconnected` FPSS error.
    fn send_cmd(&self, cmd: IoCommand) -> Result<(), Error> {
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
///
/// On `Metadata`, the payload is the server's "Bundle" string. We copy it
/// verbatim into [`LoginResult::Success`]; see
/// [`FpssControl::LoginSuccess`] for why this string is treated as opaque.
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

    // Thread-local symbol cache: contract_id -> pre-rendered symbol string.
    // Populated on ContractAssigned events, used by resolve_symbol() and
    // warn_unknown_contract() on every tick -- zero Mutex locks on the hot path.
    // The shared contract_map (Mutex-backed) is still updated for external callers
    // (contract_map(), contract_lookup() public APIs).
    let mut local_symbols: HashMap<i32, Arc<str>> = HashMap::new();

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
                        &mut local_symbols,
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
        local_symbols.clear();
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
#[allow(clippy::too_many_arguments)] // Reason: reconnection requires all FPSS state (subs, config, credentials) in one call.
#[allow(clippy::missing_errors_doc)] // Reason: internal function, doc is on the module-level reconnect docs above.
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

        client.send_cmd(IoCommand::WriteFrame { code, payload })?;

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

        client.send_cmd(IoCommand::WriteFrame { code, payload })?;

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
    use tdbe::types::price::Price;

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
}
