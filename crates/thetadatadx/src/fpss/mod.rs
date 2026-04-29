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
//!         FpssEvent::Data(FpssData::Quote { contract, bid, ask, .. }) => {
//!             let _root = &contract.root; // symbol / option root
//!             let _ = (bid, ask); // f64 prices
//!         }
//!         FpssEvent::Data(FpssData::Trade { contract, price, size, .. }) => {
//!             let _root = &contract.root;
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
mod io_loop;
pub(crate) mod pinning;
pub mod protocol;
pub mod ring;
mod session;

use self::events::IoCommand;
pub use self::events::{FpssControl, FpssData, FpssEvent};
use self::io_loop::{io_loop, ping_loop, wait_for_login, LoginResult};
pub use self::session::reconnect_delay;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::auth::Credentials;
use crate::config::{FpssFlushMode, ReconnectPolicy};
use crate::error::Error;
use tdbe::types::enums::{RemoveReason, StreamMsgType};

use self::framing::{write_frame, Frame};
use self::protocol::{
    build_credentials_payload, build_subscribe_payload, Contract, SubscriptionKind,
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
    pub(in crate::fpss) active_subs: Arc<Mutex<Vec<(SubscriptionKind, Contract)>>>,
    /// Active full-type (firehose) subscriptions for reconnection.
    pub(in crate::fpss) active_full_subs:
        Arc<Mutex<Vec<(SubscriptionKind, tdbe::types::enums::SecType)>>>,
    /// Server-assigned contract ID mapping.
    ///
    /// Stores `Arc<Contract>` so the I/O thread, the shared map, and
    /// every decoded data event share a single heap allocation per
    /// contract_id.
    contract_map: Arc<Mutex<HashMap<i32, Arc<Contract>>>>,
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
    /// `hosts` is the FPSS server list from [`crate::config::DirectConfig::fpss_hosts`].
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
        // Source: FPSSClient.connect() -- blocks until login response arrives.
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
        let contract_map = Arc::new(Mutex::new(HashMap::new()));
        let active_subs: Arc<Mutex<Vec<(protocol::SubscriptionKind, protocol::Contract)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let active_full_subs: Arc<
            Mutex<Vec<(protocol::SubscriptionKind, tdbe::types::enums::SecType)>>,
        > = Arc::new(Mutex::new(Vec::new()));

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
        let io_active_subs = Arc::clone(&active_subs);
        let io_active_full_subs = Arc::clone(&active_full_subs);

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
                    pending_control,
                    io_server_addr,
                    derive_ohlcvc,
                    flush_mode,
                    policy,
                    io_creds,
                    io_hosts,
                    io_active_subs,
                    io_active_full_subs,
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

    /// Get the current contract map (server-assigned IDs -> `Arc<Contract>`).
    ///
    /// Each value is the SAME `Arc<Contract>` the I/O thread hands to every
    /// decoded data event for that contract_id. Cloning the map clones
    /// `Arc`s (refcount bumps), not the underlying `Contract` values.
    pub fn contract_map(&self) -> HashMap<i32, Arc<Contract>> {
        self.contract_map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Look up a single contract by its server-assigned ID.
    ///
    /// Returns `Arc<Contract>` so the caller participates in the same
    /// heap allocation used by every decoded data event. Much cheaper
    /// than [`contract_map()`](Self::contract_map) for the hot path
    /// where callers decode FIT ticks and need to resolve individual
    /// contract IDs.
    pub fn contract_lookup(&self, id: i32) -> Option<Arc<Contract>> {
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

        // Join background threads.
        if let Some(h) = self.ping_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.io_handle.take() {
            let _ = h.join();
        }
    }
}
