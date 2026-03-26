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
//! let creds = Credentials { email: "user@example.com".into(), password: "pw".into() };
//! let client = FpssClient::connect(&creds, 4096, |event: &FpssEvent| {
//!     // Runs on the Disruptor consumer thread -- keep it fast.
//!     // Push to your own queue for heavy processing.
//!     match event {
//!         FpssEvent::Data(FpssData::Quote { contract_id, bid, ask, .. }) => { /* decoded fields */ }
//!         FpssEvent::Data(FpssData::Trade { contract_id, price, size, .. }) => { /* decoded fields */ }
//!         FpssEvent::Control(_) => { /* lifecycle */ }
//!         _ => {}
//!     }
//! })?;
//!
//! // Subscribe (blocking write to TLS stream via internal command channel).
//! let req_id = client.subscribe_quotes(
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
use std::time::Duration;

use disruptor::{build_single_producer, Producer, Sequence};

use self::ring::{AdaptiveWaitStrategy, RingEvent};

use crate::auth::Credentials;
use crate::codec::fit::{apply_deltas, FitReader};
use crate::error::Error;
use crate::types::enums::{RemoveReason, StreamMsgType, StreamResponseType};

use self::framing::{read_frame, write_frame, write_raw_frame, write_raw_frame_no_flush, Frame};
use self::protocol::{
    build_credentials_payload, build_ping_payload, build_subscribe_payload, parse_contract_message,
    parse_disconnect_reason, parse_req_response, Contract, SubscriptionKind, PING_INTERVAL_MS,
    RECONNECT_DELAY_MS, TOO_MANY_REQUESTS_DELAY_MS,
};

/// Tick data events from the FPSS stream.
///
/// These are the hot-path events decoded from FIT wire format and
/// delta-decompressed. All fields are raw integer values; use
/// `Price::new(price, price_type).to_f64()` for human-readable prices.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FpssData {
    /// Decoded quote tick (code 21). 11 FIT fields + contract_id.
    Quote {
        contract_id: i32,
        ms_of_day: i32,
        bid_size: i32,
        bid_exchange: i32,
        bid: i32,
        bid_condition: i32,
        ask_size: i32,
        ask_exchange: i32,
        ask: i32,
        ask_condition: i32,
        price_type: i32,
        date: i32,
    },
    /// Decoded trade tick (code 22). 16 FIT fields + contract_id.
    Trade {
        contract_id: i32,
        ms_of_day: i32,
        sequence: i32,
        ext_condition1: i32,
        ext_condition2: i32,
        ext_condition3: i32,
        ext_condition4: i32,
        condition: i32,
        size: i32,
        exchange: i32,
        price: i32,
        condition_flags: i32,
        price_flags: i32,
        volume_type: i32,
        records_back: i32,
        price_type: i32,
        date: i32,
    },
    /// Decoded open interest tick (code 23). 3 FIT fields + contract_id.
    OpenInterest {
        contract_id: i32,
        ms_of_day: i32,
        open_interest: i32,
        date: i32,
    },
    /// Decoded OHLCVC bar (code 24 or trade-derived).
    Ohlcvc {
        contract_id: i32,
        ms_of_day: i32,
        open: i32,
        high: i32,
        low: i32,
        close: i32,
        volume: i32,
        count: i32,
        price_type: i32,
        date: i32,
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
    RawData { code: u8, payload: Vec<u8> },
    /// Placeholder default for ring buffer pre-allocation.
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

/// Real-time streaming client for ThetaData's FPSS servers.
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
    /// Active subscriptions for reconnection.
    active_subs: Mutex<Vec<(SubscriptionKind, Contract)>>,
    /// Server-assigned contract ID mapping.
    contract_map: Arc<Mutex<HashMap<i32, Contract>>>,
    /// The server address we connected to.
    server_addr: String,
}

// SAFETY: All fields are either Send+Sync or behind Mutex/Atomic.
unsafe impl Sync for FpssClient {}

impl FpssClient {
    /// Connect to a ThetaData FPSS server, authenticate, and start processing
    /// events via the provided callback.
    ///
    /// The callback runs on the Disruptor's consumer thread -- keep it fast.
    /// For heavy processing, push events to your own queue from the callback.
    ///
    /// # Sequence (from `FPSSClient.java`)
    ///
    /// 1. Try each server in `SERVERS` until one connects (blocking TLS over TCP)
    /// 2. Send CREDENTIALS (code 0) with email + password
    /// 3. Wait for METADATA (code 3) = login success, or DISCONNECTED (code 12) = failure
    /// 4. Start ping heartbeat (100ms interval, std::thread with sleep loop)
    /// 5. Start I/O thread (blocking TLS read -> Disruptor ring -> callback)
    ///
    /// Source: `FPSSClient.connect()` and `FPSSClient.sendCredentials()`.
    pub fn connect<F>(creds: &Credentials, ring_size: usize, handler: F) -> Result<Self, Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        let (stream, server_addr) = connection::connect()?;
        Self::connect_with_stream(creds, stream, server_addr, ring_size, handler)
    }

    /// Connect using a pre-established stream (for testing with mock sockets).
    pub(crate) fn connect_with_stream<F>(
        creds: &Credentials,
        mut stream: connection::FpssStream,
        server_addr: String,
        ring_size: usize,
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
                return Err(Error::FpssDisconnected(format!(
                    "server rejected login: {reason:?}"
                )));
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
            .map_err(|e| Error::Fpss(format!("failed to set read timeout: {e}")))?;

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
                );
            })
            .expect("failed to spawn fpss-io thread");

        // Spawn the ping thread: sends PING command every 100ms.
        let ping_shutdown = Arc::clone(&shutdown);
        let ping_authenticated = Arc::clone(&authenticated);

        let ping_handle = thread::Builder::new()
            .name("fpss-ping".to_owned())
            .spawn(move || {
                ping_loop(ping_cmd_tx, ping_shutdown, ping_authenticated);
            })
            .expect("failed to spawn fpss-ping thread");

        Ok(FpssClient {
            cmd_tx,
            io_handle: Some(io_handle),
            ping_handle: Some(ping_handle),
            shutdown,
            authenticated,
            next_req_id: AtomicI32::new(1),
            active_subs: Mutex::new(Vec::new()),
            contract_map,
            server_addr,
        })
    }

    /// Subscribe to quote data for a contract.
    ///
    /// Returns the request ID assigned to this subscription.
    ///
    /// # Wire protocol (from `PacketStream.addQuote()`)
    ///
    /// Sends code 21 (QUOTE) with payload `[req_id: i32 BE] [contract bytes]`.
    /// Server responds with code 40 (REQ_RESPONSE).
    pub fn subscribe_quotes(&self, contract: &Contract) -> Result<i32, Error> {
        self.subscribe(SubscriptionKind::Quote, contract)
    }

    /// Subscribe to trade data for a contract.
    ///
    /// Source: `PacketStream.addTrade()` -- sends code 22 (TRADE).
    pub fn subscribe_trades(&self, contract: &Contract) -> Result<i32, Error> {
        self.subscribe(SubscriptionKind::Trade, contract)
    }

    /// Subscribe to open interest data for a contract.
    ///
    /// Source: `PacketStream.addOpenInterest()` -- sends code 23 (OPEN_INTEREST).
    pub fn subscribe_open_interest(&self, contract: &Contract) -> Result<i32, Error> {
        self.subscribe(SubscriptionKind::OpenInterest, contract)
    }

    /// Subscribe to all trades for a security type (full trade stream).
    ///
    /// # Wire protocol (from `PacketStream.java`)
    ///
    /// Sends code 22 (TRADE) with 5-byte payload `[req_id: i32 BE] [sec_type: u8]`.
    /// The server distinguishes this from per-contract subscriptions by payload length.
    pub fn subscribe_full_trades(
        &self,
        sec_type: crate::types::enums::SecType,
    ) -> Result<i32, Error> {
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = protocol::build_full_type_subscribe_payload(req_id, sec_type);

        self.cmd_tx
            .send(IoCommand::WriteFrame {
                code: StreamMsgType::Trade,
                payload,
            })
            .map_err(|_| Error::Fpss("I/O thread has exited".to_string()))?;

        tracing::debug!(req_id, sec_type = ?sec_type, "sent full trade subscription");
        Ok(req_id)
    }

    /// Unsubscribe from quote data for a contract.
    ///
    /// Source: `PacketStream.removeQuote()` -- sends code 51 (REMOVE_QUOTE).
    pub fn unsubscribe_quotes(&self, contract: &Contract) -> Result<i32, Error> {
        self.unsubscribe(SubscriptionKind::Quote, contract)
    }

    /// Unsubscribe from trade data for a contract.
    ///
    /// Source: `PacketStream.removeTrade()` -- sends code 52 (REMOVE_TRADE).
    pub fn unsubscribe_trades(&self, contract: &Contract) -> Result<i32, Error> {
        self.unsubscribe(SubscriptionKind::Trade, contract)
    }

    /// Unsubscribe from open interest data for a contract.
    ///
    /// Source: `PacketStream.removeOpenInterest()` -- sends code 53.
    pub fn unsubscribe_open_interest(&self, contract: &Contract) -> Result<i32, Error> {
        self.unsubscribe(SubscriptionKind::OpenInterest, contract)
    }

    /// Internal subscribe implementation.
    fn subscribe(&self, kind: SubscriptionKind, contract: &Contract) -> Result<i32, Error> {
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = build_subscribe_payload(req_id, contract);
        let code = kind.subscribe_code();

        self.cmd_tx
            .send(IoCommand::WriteFrame { code, payload })
            .map_err(|_| Error::Fpss("I/O thread has exited".to_string()))?;

        // Track for reconnection
        {
            let mut subs = self.active_subs.lock().unwrap();
            subs.push((kind, contract.clone()));
        }

        tracing::debug!(
            req_id,
            kind = ?kind,
            contract = %contract,
            "sent subscription"
        );
        Ok(req_id)
    }

    /// Internal unsubscribe implementation.
    fn unsubscribe(&self, kind: SubscriptionKind, contract: &Contract) -> Result<i32, Error> {
        self.check_connected()?;

        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let payload = build_subscribe_payload(req_id, contract);
        let code = kind.unsubscribe_code();

        self.cmd_tx
            .send(IoCommand::WriteFrame { code, payload })
            .map_err(|_| Error::Fpss("I/O thread has exited".to_string()))?;

        // Remove from tracked subscriptions
        {
            let mut subs = self.active_subs.lock().unwrap();
            subs.retain(|(k, c)| !(k == &kind && c == contract));
        }

        tracing::debug!(
            req_id,
            kind = ?kind,
            contract = %contract,
            "sent unsubscribe"
        );
        Ok(req_id)
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
        self.contract_map.lock().unwrap().clone()
    }

    /// Look up a single contract by its server-assigned ID.
    ///
    /// Much cheaper than [`contract_map()`](Self::contract_map) for the hot path
    /// where callers decode FIT ticks and need to resolve individual contract IDs.
    pub fn contract_lookup(&self, id: i32) -> Option<Contract> {
        self.contract_map.lock().unwrap().get(&id).cloned()
    }

    /// Get a snapshot of currently active subscriptions.
    pub fn active_subscriptions(&self) -> Vec<(SubscriptionKind, Contract)> {
        self.active_subs.lock().unwrap().clone()
    }

    /// Verify connection is live before sending.
    fn check_connected(&self) -> Result<(), Error> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(Error::Fpss("client is shut down".to_string()));
        }
        if !self.authenticated.load(Ordering::Acquire) {
            return Err(Error::Fpss("not authenticated".to_string()));
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
        let frame = read_frame(stream)?
            .ok_or_else(|| Error::Fpss("connection closed during login handshake".to_string()))?;

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
                return Err(Error::Fpss(format!("server error during login: {msg}")));
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

/// The I/O thread owns the TLS stream. It does three things in a loop:
///
/// 1. Attempt a blocking read (with short timeout) for incoming frames
/// 2. Drain the command channel for outgoing writes (subscribe, ping, etc.)
/// 3. Publish decoded events into the Disruptor ring
///
/// This thread IS the Disruptor producer. Events flow directly from the TLS
/// socket into the ring buffer with zero intermediate channels.
#[allow(clippy::too_many_arguments)]
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
                    handler(evt);
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

    // Split the stream into buffered read/write.
    // We use BufReader for efficient small reads (FPSS frames are tiny: 2-257 bytes).
    // BufWriter batches small writes (pings, subscribe frames).
    let mut reader = BufReader::new(stream);

    // Track consecutive read timeouts to detect the 10s overall timeout.
    // With 50ms per attempt, 200 consecutive timeouts = 10 seconds.
    let max_consecutive_timeouts = (protocol::READ_TIMEOUT_MS / 50).max(1);
    let mut consecutive_timeouts: u64 = 0;

    // Per-contract delta state for FIT decompression.
    // Key: (msg_type_code, contract_id), Value: previous absolute tick fields.
    // Each tick type has its own field count:
    //   Quote=11, Trade=16, OpenInterest=3, Ohlcvc=9
    let mut delta_state: DeltaState = DeltaState::new();

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        // --- Phase 1: Try to read a frame (short blocking read) ---
        match read_frame(&mut reader) {
            Ok(Some(frame)) => {
                consecutive_timeouts = 0;

                let events = decode_frame(
                    &frame,
                    &authenticated,
                    &contract_map,
                    &shutdown,
                    &mut delta_state,
                );

                for evt in events {
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
                break;
            }
            Err(ref e) if is_read_timeout(e) => {
                // Read timeout -- this is expected with our 50ms timeout.
                // Check if we've exceeded the overall 10s threshold.
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
                    break;
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
                break;
            }
        }

        // --- Phase 2: Drain command channel (non-blocking) ---
        // Process all pending write commands.
        loop {
            match cmd_rx.try_recv() {
                Ok(IoCommand::WriteFrame { code, payload }) => {
                    // Get mutable access to the underlying stream through BufReader.
                    let writer = reader.get_mut();
                    // Only flush on PING frames — let other writes batch.
                    // Source: Java terminal only flushes on pings.
                    let result = if code == StreamMsgType::Ping {
                        write_raw_frame(writer, code, &payload)
                    } else {
                        write_raw_frame_no_flush(writer, code, &payload)
                    };
                    if let Err(e) = result {
                        tracing::warn!(error = %e, "failed to write frame");
                        // Don't break the read loop for write errors -- the read
                        // loop will detect the broken connection on the next read.
                    }
                }
                Ok(IoCommand::Shutdown) => {
                    // Send STOP to server before exiting.
                    let stop_payload = protocol::build_stop_payload();
                    let writer = reader.get_mut();
                    let _ = write_raw_frame(writer, StreamMsgType::Stop, &stop_payload);
                    tracing::debug!("sent STOP, I/O thread exiting");
                    // Signal shutdown so the outer loop exits.
                    shutdown.store(true, Ordering::Release);
                    // Break inner drain loop.
                    break;
                }
                Err(std_mpsc::TryRecvError::Empty) => break,
                Err(std_mpsc::TryRecvError::Disconnected) => {
                    // All senders dropped -- client was dropped without calling shutdown.
                    tracing::debug!("command channel disconnected, I/O thread exiting");
                    shutdown.store(true, Ordering::Release);
                    break;
                }
            }
        }
    }

    // Producer drop joins the Disruptor consumer thread and drains remaining events.
    tracing::debug!("fpss-io thread exiting");
}

/// Check if an error is a read timeout (WouldBlock or TimedOut).
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

/// Number of FIT fields per tick type (excluding the contract_id which is the
/// first FIT field). The FIT decoder returns `n_fields` total, where field [0]
/// is the contract_id and fields [1..] are the tick data.
const QUOTE_FIELDS: usize = 11;
const TRADE_FIELDS: usize = 16;
const OI_FIELDS: usize = 3;
const OHLCVC_FIELDS: usize = 9;

/// Per-contract OHLCVC accumulator, updated on every Trade event.
struct OhlcvcAccumulator {
    open: i32,
    high: i32,
    low: i32,
    close: i32,
    volume: i32,
    count: i32,
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

    #[allow(clippy::too_many_arguments)]
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
        self.volume = volume;
        self.count = count;
        self.price_type = price_type;
        self.date = date;
        self.initialized = true;
    }

    fn process_trade(&mut self, ms_of_day: i32, price: i32, size: i32, price_type: i32, date: i32) {
        if !self.initialized {
            self.open = price;
            self.high = price;
            self.low = price;
            self.close = price;
            self.volume = size;
            self.count = 1;
            self.price_type = price_type;
            self.date = date;
            self.ms_of_day = ms_of_day;
            self.initialized = true;
        } else {
            self.ms_of_day = ms_of_day;
            let adjusted_price = change_price_type(price, price_type, self.price_type);
            self.volume += size;
            self.count += 1;
            if adjusted_price > self.high {
                self.high = adjusted_price;
            }
            if adjusted_price < self.low {
                self.low = adjusted_price;
            }
            self.close = adjusted_price;
        }
    }
}

/// Convert a price from one price_type to another (mirrors Java PriceCalcUtils.changePriceType).
fn change_price_type(price: i32, price_type: i32, new_price_type: i32) -> i32 {
    if price == 0 || price_type == new_price_type {
        return price;
    }
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
    let exp = new_price_type - price_type;
    if exp <= 0 {
        let idx = (-exp) as usize;
        if idx < POW10.len() {
            price * POW10[idx]
        } else {
            price
        }
    } else {
        let idx = exp as usize;
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
}

impl DeltaState {
    fn new() -> Self {
        Self {
            prev: HashMap::new(),
            ohlcvc: HashMap::new(),
        }
    }

    /// Clear all accumulated delta state.
    ///
    /// Called on START/STOP (market open/close) signals to reset delta
    /// decompression, matching Java's behavior where `Tick.readID()` starts
    /// fresh after a session boundary.
    fn clear(&mut self) {
        self.prev.clear();
        self.ohlcvc.clear();
    }

    /// Decode FIT payload and apply delta decompression.
    ///
    /// The ENTIRE payload is FIT-encoded. The first FIT field (alloc[0]) is the
    /// contract_id. Tick data fields start at alloc[1..].
    ///
    /// This matches the Java terminal's `FPSSClient` which calls:
    /// ```java
    /// fitReader.open(p.data(), 0, p.len());  // FIT starts at offset 0
    /// int size = fitReader.readChanges(alloc); // alloc[0] = contract_id
    /// Contract c = idToContract.get(alloc[0]); // first field IS the contract_id
    /// ```
    ///
    /// Returns `(contract_id, tick_fields)` or `None` if payload is too short
    /// or the FIT row is a DATE marker.
    fn decode_tick(
        &mut self,
        msg_code: u8,
        payload: &[u8],
        expected_fields: usize,
    ) -> Option<(i32, Vec<i32>)> {
        if payload.is_empty() {
            return None;
        }

        // Allocate for contract_id (1 field) + tick data fields.
        let total_fields = expected_fields + 1;
        let mut alloc = vec![0i32; total_fields];

        let mut reader = FitReader::new(payload);
        let n = reader.read_changes(&mut alloc);

        if reader.is_date {
            // DATE marker row -- skip (no user-visible data).
            return None;
        }

        if n == 0 {
            return None;
        }

        // First FIT field is the contract_id.
        let contract_id = alloc[0];

        // Tick data is alloc[1..]. Extract into its own vec.
        let mut fields: Vec<i32> = alloc[1..total_fields].to_vec();

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
        }
        // else: first row for this contract -- values are already absolute.

        // Store as the new previous state (tick fields only, not contract_id).
        self.prev.insert(key, fields.clone());

        Some((contract_id, fields))
    }
}

/// Decode a frame into an FpssEvent (if it maps to one).
///
/// This is the frame dispatch logic from `FPSSClient.java`'s reader thread.
/// Tick data frames (Quote, Trade, OpenInterest, Ohlcvc) are FIT-decoded and
/// delta-decompressed before being emitted as typed events.
fn decode_frame(
    frame: &Frame,
    authenticated: &AtomicBool,
    contract_map: &Mutex<HashMap<i32, Contract>>,
    shutdown: &AtomicBool,
    delta_state: &mut DeltaState,
) -> Vec<FpssEvent> {
    match frame.code {
        StreamMsgType::Metadata => {
            // Can arrive again after reconnection
            let permissions = String::from_utf8_lossy(&frame.payload).to_string();
            tracing::debug!(permissions = %permissions, "received METADATA");
            authenticated.store(true, Ordering::Release);
            vec![FpssEvent::Control(FpssControl::LoginSuccess {
                permissions,
            })]
        }

        StreamMsgType::Contract => match parse_contract_message(&frame.payload) {
            Ok((id, contract)) => {
                tracing::debug!(id, contract = %contract, "contract assigned");
                contract_map.lock().unwrap().insert(id, contract.clone());
                vec![FpssEvent::Control(FpssControl::ContractAssigned {
                    id,
                    contract,
                })]
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse CONTRACT message");
                vec![FpssEvent::Control(FpssControl::Error {
                    message: format!("failed to parse CONTRACT message: {e}"),
                })]
            }
        },

        StreamMsgType::Quote => {
            let code = frame.code as u8;
            match delta_state.decode_tick(code, &frame.payload, QUOTE_FIELDS) {
                Some((contract_id, f)) => vec![FpssEvent::Data(FpssData::Quote {
                    contract_id,
                    ms_of_day: f[0],
                    bid_size: f[1],
                    bid_exchange: f[2],
                    bid: f[3],
                    bid_condition: f[4],
                    ask_size: f[5],
                    ask_exchange: f[6],
                    ask: f[7],
                    ask_condition: f[8],
                    price_type: f[9],
                    date: f[10],
                })],
                None => vec![FpssEvent::RawData {
                    code: frame.code as u8,
                    payload: frame.payload.clone(),
                }],
            }
        }

        StreamMsgType::Trade => {
            let code = frame.code as u8;
            match delta_state.decode_tick(code, &frame.payload, TRADE_FIELDS) {
                Some((contract_id, f)) => {
                    let (ms_of_day, size, price) = (f[0], f[7], f[9]);
                    let (price_type, date) = (f[14], f[15]);
                    let trade_event = FpssEvent::Data(FpssData::Trade {
                        contract_id,
                        ms_of_day,
                        sequence: f[1],
                        ext_condition1: f[2],
                        ext_condition2: f[3],
                        ext_condition3: f[4],
                        ext_condition4: f[5],
                        condition: f[6],
                        size,
                        exchange: f[8],
                        price,
                        condition_flags: f[10],
                        price_flags: f[11],
                        volume_type: f[12],
                        records_back: f[13],
                        price_type,
                        date,
                    });
                    // Java only updates OHLCVC if lastOHLCVC already exists
                    // (seeded by a prior server OHLCVC message). We match this:
                    // don't emit derived OHLCVC unless the server has seeded one.
                    if let Some(acc) = delta_state.ohlcvc.get_mut(&contract_id) {
                        if acc.initialized {
                            acc.process_trade(ms_of_day, price, size, price_type, date);
                            let ohlcvc_event = FpssEvent::Data(FpssData::Ohlcvc {
                                contract_id,
                                ms_of_day: acc.ms_of_day,
                                open: acc.open,
                                high: acc.high,
                                low: acc.low,
                                close: acc.close,
                                volume: acc.volume,
                                count: acc.count,
                                price_type: acc.price_type,
                                date: acc.date,
                            });
                            vec![trade_event, ohlcvc_event]
                        } else {
                            vec![trade_event]
                        }
                    } else {
                        vec![trade_event]
                    }
                }
                None => vec![FpssEvent::RawData {
                    code: frame.code as u8,
                    payload: frame.payload.clone(),
                }],
            }
        }

        StreamMsgType::OpenInterest => {
            let code = frame.code as u8;
            match delta_state.decode_tick(code, &frame.payload, OI_FIELDS) {
                Some((contract_id, f)) => vec![FpssEvent::Data(FpssData::OpenInterest {
                    contract_id,
                    ms_of_day: f[0],
                    open_interest: f[1],
                    date: f[2],
                })],
                None => vec![FpssEvent::RawData {
                    code: frame.code as u8,
                    payload: frame.payload.clone(),
                }],
            }
        }

        StreamMsgType::Ohlcvc => {
            let code = frame.code as u8;
            match delta_state.decode_tick(code, &frame.payload, OHLCVC_FIELDS) {
                Some((contract_id, f)) => {
                    let acc = delta_state
                        .ohlcvc
                        .entry(contract_id)
                        .or_insert_with(OhlcvcAccumulator::new);
                    acc.init_from_server(f[0], f[1], f[2], f[3], f[4], f[5], f[6], f[7], f[8]);
                    vec![FpssEvent::Data(FpssData::Ohlcvc {
                        contract_id,
                        ms_of_day: f[0],
                        open: f[1],
                        high: f[2],
                        low: f[3],
                        close: f[4],
                        volume: f[5],
                        count: f[6],
                        price_type: f[7],
                        date: f[8],
                    })]
                }
                None => vec![FpssEvent::RawData {
                    code: frame.code as u8,
                    payload: frame.payload.clone(),
                }],
            }
        }

        StreamMsgType::ReqResponse => match parse_req_response(&frame.payload) {
            Ok((req_id, result)) => {
                tracing::debug!(req_id, result = ?result, "subscription response");
                vec![FpssEvent::Control(FpssControl::ReqResponse {
                    req_id,
                    result,
                })]
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse REQ_RESPONSE");
                vec![FpssEvent::Control(FpssControl::Error {
                    message: format!("failed to parse REQ_RESPONSE: {e}"),
                })]
            }
        },

        StreamMsgType::Start => {
            tracing::info!("market open signal received");
            delta_state.clear();
            contract_map.lock().unwrap().clear(); // Java: idToContract.clear()
            vec![FpssEvent::Control(FpssControl::MarketOpen)]
        }

        StreamMsgType::Stop => {
            tracing::info!("market close signal received");
            delta_state.clear();
            contract_map.lock().unwrap().clear(); // Java: idToContract.clear()
            vec![FpssEvent::Control(FpssControl::MarketClose)]
        }

        StreamMsgType::Error => {
            let message = String::from_utf8_lossy(&frame.payload).to_string();
            tracing::warn!(message = %message, "server error");
            vec![FpssEvent::Control(FpssControl::ServerError { message })]
        }

        StreamMsgType::Disconnected => {
            let reason = parse_disconnect_reason(&frame.payload);
            tracing::warn!(reason = ?reason, "server disconnected us");
            authenticated.store(false, Ordering::Release);

            // Permanent errors -- no reconnect will fix these.
            if reconnect_delay(reason).is_none() {
                tracing::error!(reason = ?reason, "permanent disconnect -- stopping");
                shutdown.store(true, Ordering::Release);
            }

            vec![FpssEvent::Control(FpssControl::Disconnected { reason })]
        }

        // Ignore frame types we don't handle (e.g., server sending PING)
        other => {
            tracing::trace!(code = ?other, "ignoring unhandled frame type");
            vec![]
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
pub fn reconnect<F>(
    creds: &Credentials,
    previous_subs: Vec<(SubscriptionKind, Contract)>,
    delay_ms: u64,
    ring_size: usize,
    handler: F,
) -> Result<FpssClient, Error>
where
    F: FnMut(&FpssEvent) + Send + 'static,
{
    tracing::info!(delay_ms, "waiting before FPSS reconnection");
    thread::sleep(Duration::from_millis(delay_ms));

    let client = FpssClient::connect(creds, ring_size, handler)?;

    // Re-subscribe all previous subscriptions with req_id = -1
    // Source: FPSSClient.java -- reconnect logic uses req_id = -1 for re-subscriptions
    for (kind, contract) in &previous_subs {
        let payload = build_subscribe_payload(-1, contract);
        let code = kind.subscribe_code();

        client
            .cmd_tx
            .send(IoCommand::WriteFrame { code, payload })
            .map_err(|_| Error::Fpss("I/O thread exited during reconnect".to_string()))?;

        tracing::debug!(
            kind = ?kind,
            contract = %contract,
            "re-subscribed after reconnect (req_id=-1)"
        );
    }

    // Store the re-subscribed list
    {
        let mut subs = client.active_subs.lock().unwrap();
        *subs = previous_subs;
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
            ms_of_day: 0,
            sequence: 0,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: 100,
            exchange: 0,
            price: 15025,
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            price_type: 8,
            date: 20240315,
        });
        match &data_evt {
            FpssEvent::Data(FpssData::Trade {
                contract_id, price, ..
            }) => {
                assert_eq!(*contract_id, 42);
                assert_eq!(*price, 15025);
            }
            other => panic!("expected Data(Trade), got {other:?}"),
        }
        let ctrl = FpssEvent::Control(FpssControl::MarketOpen);
        assert!(matches!(&ctrl, FpssEvent::Control(FpssControl::MarketOpen)));
    }
}
