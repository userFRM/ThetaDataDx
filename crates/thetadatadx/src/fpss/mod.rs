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
//! # use thetadatadx::fpss::{FpssClient, FpssEvent};
//! # use thetadatadx::auth::Credentials;
//! # fn example() -> Result<(), thetadatadx::error::Error> {
//! let creds = Credentials { email: "user@example.com".into(), password: "pw".into() };
//! let client = FpssClient::connect(&creds, 4096, |event: &FpssEvent| {
//!     // Runs on the Disruptor consumer thread -- keep it fast.
//!     // Push to your own queue for heavy processing.
//!     match event {
//!         FpssEvent::QuoteData { payload } => { /* update atomic, push to ring */ }
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
//!  ┌─────────────┐  cmd channel   ┌──────────────────┐  publish()  ┌────────────────┐
//!  │ FpssClient   │──────────────►│ I/O thread        │───────────►│ Disruptor Ring  │
//!  │              │               │ (std::thread)      │            │ (SPSC, lock-    │
//!  │ .subscribe() │               │ blocking TLS read  │            │  free, pre-     │
//!  │ .unsubscribe │               │ + write drain      │            │  allocated)     │
//!  │ .shutdown()  │               └──────────────────┘            └───────┬────────┘
//!  └─────────────┘               ┌──────────────────┐                     │ consumer
//!                                 │ Ping thread       │                     ▼
//!                                 │ (std::thread,     │            ┌────────────────┐
//!                                 │  sleep loop)      │            │ User handler(F) │
//!                                 └──────────────────┘            │ (zero-alloc)    │
//!                                                                  └────────────────┘
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
use crate::error::Error;
use crate::types::enums::{RemoveReason, StreamMsgType, StreamResponseType};

use self::framing::{read_frame, write_frame, write_raw_frame, Frame};
use self::protocol::{
    build_credentials_payload, build_ping_payload, build_subscribe_payload, parse_contract_message,
    parse_disconnect_reason, parse_req_response, Contract, SubscriptionKind, PING_INTERVAL_MS,
    RECONNECT_DELAY_MS, TOO_MANY_REQUESTS_DELAY_MS,
};

/// Events emitted by the FPSS background read loop.
///
/// Subscribers receive these through the Disruptor callback. The enum is
/// non-exhaustive to allow adding new event types without breaking downstream.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum FpssEvent {
    /// Login succeeded. Payload is the permissions string from METADATA (code 3).
    ///
    /// Source: `FPSSClient.onMetadata()` -- server sends permissions as UTF-8.
    LoginSuccess { permissions: String },

    /// Server sent a CONTRACT assignment (code 20).
    ///
    /// The server assigns a numeric ID to each contract on first subscription.
    /// Subsequent data messages reference this ID instead of the full contract.
    ///
    /// Source: `FPSSClient.onContract()`.
    ContractAssigned { id: i32, contract: Contract },

    /// Raw quote data arrived (code 21, server->client direction).
    ///
    /// Payload is the raw FIT-encoded bytes. The caller must decode using
    /// `codec::fit::FitReader` once that module is available.
    ///
    /// Source: `FPSSClient.onQuote()` -- dispatches to registered listeners.
    QuoteData { payload: Vec<u8> },

    /// Raw trade data arrived (code 22, server->client direction).
    ///
    /// Source: `FPSSClient.onTrade()`.
    TradeData { payload: Vec<u8> },

    /// Raw open interest data arrived (code 23, server->client direction).
    ///
    /// Source: `FPSSClient.onOpenInterest()`.
    OpenInterestData { payload: Vec<u8> },

    /// Raw OHLCVC snapshot arrived (code 24, server->client direction).
    ///
    /// Source: `FPSSClient.onOHLCVC()`.
    OhlcvcData { payload: Vec<u8> },

    /// Subscription response (code 40).
    ///
    /// Source: `FPSSClient.onReqResponse()`.
    ReqResponse {
        req_id: i32,
        result: StreamResponseType,
    },

    /// Market open signal (code 30).
    ///
    /// Source: `FPSSClient.onStart()`.
    #[default]
    MarketOpen,

    /// Market close / stop signal (code 32).
    ///
    /// Source: `FPSSClient.onStop()`.
    MarketClose,

    /// Server error message (code 11). Payload is UTF-8 error text.
    ///
    /// Source: `FPSSClient.onError()`.
    ServerError { message: String },

    /// Server disconnected us (code 12). Contains the parsed reason.
    ///
    /// Source: `FPSSClient.onDisconnected()`.
    Disconnected { reason: RemoveReason },

    /// Protocol-level parse error (e.g. malformed CONTRACT or REQ_RESPONSE).
    ///
    /// Callers should log these; they indicate protocol-level corruption or
    /// version mismatch with the server.
    Error { message: String },
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
        slot.event = Some(FpssEvent::LoginSuccess { permissions });
    });

    // Split the stream into buffered read/write.
    // We use BufReader for efficient small reads (FPSS frames are tiny: 2-257 bytes).
    // BufWriter batches small writes (pings, subscribe frames).
    let mut reader = BufReader::new(stream);

    // Track consecutive read timeouts to detect the 10s overall timeout.
    // With 50ms per attempt, 200 consecutive timeouts = 10 seconds.
    let max_consecutive_timeouts = (protocol::READ_TIMEOUT_MS / 50).max(1);
    let mut consecutive_timeouts: u64 = 0;

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        // --- Phase 1: Try to read a frame (short blocking read) ---
        match read_frame(&mut reader) {
            Ok(Some(frame)) => {
                consecutive_timeouts = 0;

                let event = decode_frame(&frame, &authenticated, &contract_map, &shutdown);

                if let Some(evt) = event {
                    producer.publish(|slot| {
                        slot.event = Some(evt);
                    });
                }
            }
            Ok(None) => {
                // Clean EOF
                tracing::warn!("FPSS connection closed by server");
                producer.publish(|slot| {
                    slot.event = Some(FpssEvent::Disconnected {
                        reason: RemoveReason::Unspecified,
                    });
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
                        slot.event = Some(FpssEvent::Disconnected {
                            reason: RemoveReason::TimedOut,
                        });
                    });
                    authenticated.store(false, Ordering::Release);
                    break;
                }
                // Otherwise, fall through to drain commands.
            }
            Err(e) => {
                tracing::error!(error = %e, "FPSS read error");
                producer.publish(|slot| {
                    slot.event = Some(FpssEvent::Disconnected {
                        reason: RemoveReason::Unspecified,
                    });
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
                    if let Err(e) = write_raw_frame(writer, code, &payload) {
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

/// Decode a frame into an FpssEvent (if it maps to one).
///
/// This is the frame dispatch logic from `FPSSClient.java`'s reader thread.
fn decode_frame(
    frame: &Frame,
    authenticated: &AtomicBool,
    contract_map: &Mutex<HashMap<i32, Contract>>,
    shutdown: &AtomicBool,
) -> Option<FpssEvent> {
    match frame.code {
        StreamMsgType::Metadata => {
            // Can arrive again after reconnection
            let permissions = String::from_utf8_lossy(&frame.payload).to_string();
            tracing::debug!(permissions = %permissions, "received METADATA");
            authenticated.store(true, Ordering::Release);
            Some(FpssEvent::LoginSuccess { permissions })
        }

        StreamMsgType::Contract => match parse_contract_message(&frame.payload) {
            Ok((id, contract)) => {
                tracing::debug!(id, contract = %contract, "contract assigned");
                contract_map.lock().unwrap().insert(id, contract.clone());
                Some(FpssEvent::ContractAssigned { id, contract })
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse CONTRACT message");
                Some(FpssEvent::Error {
                    message: format!("failed to parse CONTRACT message: {e}"),
                })
            }
        },

        StreamMsgType::Quote => Some(FpssEvent::QuoteData {
            payload: frame.payload.clone(),
        }),

        StreamMsgType::Trade => Some(FpssEvent::TradeData {
            payload: frame.payload.clone(),
        }),

        StreamMsgType::OpenInterest => Some(FpssEvent::OpenInterestData {
            payload: frame.payload.clone(),
        }),

        StreamMsgType::Ohlcvc => Some(FpssEvent::OhlcvcData {
            payload: frame.payload.clone(),
        }),

        StreamMsgType::ReqResponse => match parse_req_response(&frame.payload) {
            Ok((req_id, result)) => {
                tracing::debug!(req_id, result = ?result, "subscription response");
                Some(FpssEvent::ReqResponse { req_id, result })
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse REQ_RESPONSE");
                Some(FpssEvent::Error {
                    message: format!("failed to parse REQ_RESPONSE: {e}"),
                })
            }
        },

        StreamMsgType::Start => {
            tracing::info!("market open signal received");
            Some(FpssEvent::MarketOpen)
        }

        StreamMsgType::Stop => {
            tracing::info!("market close signal received");
            Some(FpssEvent::MarketClose)
        }

        StreamMsgType::Error => {
            let message = String::from_utf8_lossy(&frame.payload).to_string();
            tracing::warn!(message = %message, "server error");
            Some(FpssEvent::ServerError { message })
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

            Some(FpssEvent::Disconnected { reason })
        }

        // Ignore frame types we don't handle (e.g., server sending PING)
        other => {
            tracing::trace!(code = ?other, "ignoring unhandled frame type");
            None
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

    loop {
        thread::sleep(interval);

        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        if !authenticated.load(Ordering::Relaxed) {
            // Don't send pings if not authenticated
            continue;
        }

        let cmd = IoCommand::WriteFrame {
            code: StreamMsgType::Ping,
            payload: ping_payload.clone(),
        };
        if cmd_tx.send(cmd).is_err() {
            // I/O thread has exited
            break;
        }
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
pub fn reconnect_delay(reason: RemoveReason) -> Option<u64> {
    match reason {
        // Permanent errors -- no amount of reconnection will fix bad credentials.
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
        // Ensure FpssEvent implements Default (needed for ring slot init).
        let _evt: FpssEvent = Default::default();
    }
}
