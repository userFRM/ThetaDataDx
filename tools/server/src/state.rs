//! Shared application state for the REST + WebSocket server.
//!
//! Holds the unified `ThetaDataDx` client, connection flags, per-client
//! WebSocket channels, and shutdown plumbing. All fields are `Send + Sync`
//! behind `Arc` so axum can cheaply clone state into each handler.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::ThetaDataDx;
use tokio::sync::{mpsc, RwLock};

/// Per-client channel capacity. Matches the old `broadcast::channel(4096)`.
///
/// At ~10k events/sec peak (market open), 4096 gives ~400ms of headroom
/// before a slow WebSocket consumer starts dropping events.  Each slot is
/// an `Arc<str>` (~16 bytes), so 4096 slots cost ~64KB per client.
const WS_CLIENT_CAPACITY: usize = 4096;

/// Connected WS clients. Each gets its own bounded mpsc sender so the FPSS
/// callback can fan out a single `Arc<str>` (serialized once) without cloning
/// the JSON payload per client.
pub type WsClients = Arc<RwLock<Vec<mpsc::Sender<Arc<str>>>>>;

/// Shared server state, cloned into every axum handler.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    /// Unified client (historical via Deref to DirectClient, streaming via start_streaming).
    tdx: ThetaDataDx,
    /// Whether MDDS is connected (true after successful init).
    mdds_connected: AtomicBool,
    /// Whether FPSS is connected (set by the FPSS bridge callback).
    fpss_connected: AtomicBool,
    /// Per-client channels: FPSS events -> WebSocket clients (zero-copy fan-out).
    ws_clients: WsClients,
    /// Shutdown signal.
    shutdown: tokio::sync::Notify,
    /// WebSocket single-connection enforcement.
    ws_connected: AtomicBool,
    /// Server-assigned contract ID -> Contract mapping (updated by FPSS callback).
    contract_map: Arc<Mutex<HashMap<i32, Contract>>>,
    /// Random token required by the shutdown endpoint.
    shutdown_token: String,
}

impl AppState {
    /// Create new app state wrapping a connected `ThetaDataDx`.
    pub fn new(tdx: ThetaDataDx, shutdown_token: String) -> Self {
        Self {
            inner: Arc::new(Inner {
                tdx,
                mdds_connected: AtomicBool::new(true),
                fpss_connected: AtomicBool::new(false),
                ws_clients: Arc::new(RwLock::new(Vec::new())),
                shutdown: tokio::sync::Notify::new(),
                ws_connected: AtomicBool::new(false),
                contract_map: Arc::new(Mutex::new(HashMap::new())),
                shutdown_token,
            }),
        }
    }

    /// Borrow the unified `ThetaDataDx` client.
    pub fn tdx(&self) -> &ThetaDataDx {
        &self.inner.tdx
    }

    /// MDDS connection status string matching the Java terminal.
    pub fn mdds_status(&self) -> &'static str {
        if self.inner.mdds_connected.load(Ordering::Acquire) {
            "CONNECTED"
        } else {
            "DISCONNECTED"
        }
    }

    /// FPSS connection status string matching the Java terminal.
    pub fn fpss_status(&self) -> &'static str {
        if self.inner.fpss_connected.load(Ordering::Acquire) {
            "CONNECTED"
        } else {
            "DISCONNECTED"
        }
    }

    /// Mark FPSS as connected or disconnected.
    pub fn set_fpss_connected(&self, connected: bool) {
        self.inner
            .fpss_connected
            .store(connected, Ordering::Release);
    }

    /// Register a new WS client, returning the receiver half of its channel.
    pub async fn register_ws_client(&self) -> mpsc::Receiver<Arc<str>> {
        let (tx, rx) = mpsc::channel(WS_CLIENT_CAPACITY);
        self.inner.ws_clients.write().await.push(tx);
        rx
    }

    /// Fan out a JSON event to all connected WebSocket clients (zero-copy).
    ///
    /// Each client receives an `Arc::clone` of the same backing string --
    /// the JSON payload is serialized exactly once regardless of client count.
    ///
    /// Called from the FPSS Disruptor consumer thread (a plain `std::thread`,
    /// not a tokio task), so `blocking_read()` is safe and cannot panic.
    /// This ensures events are never silently dropped for all clients just
    /// because one client is connecting/disconnecting.
    ///
    /// If a per-client channel is full, that single slow client's event is
    /// dropped and a warning is logged -- the same backpressure semantics as
    /// the old `broadcast::channel`'s `Lagged` behavior.
    pub fn broadcast_ws(&self, event: Arc<str>) {
        let clients = self.inner.ws_clients.blocking_read();
        for tx in clients.iter() {
            if let Err(mpsc::error::TrySendError::Full(_)) = tx.try_send(Arc::clone(&event)) {
                tracing::warn!("WebSocket client lagged, dropped event");
            }
            // TrySendError::Closed is fine -- cleanup_ws_clients will prune it.
        }
    }

    /// Remove senders whose receivers have been dropped (client disconnected).
    pub async fn cleanup_ws_clients(&self) {
        self.inner
            .ws_clients
            .write()
            .await
            .retain(|tx| !tx.is_closed());
    }

    /// Shared contract map for FPSS -> WS bridge JSON serialization.
    pub fn contract_map(&self) -> Arc<Mutex<HashMap<i32, Contract>>> {
        Arc::clone(&self.inner.contract_map)
    }

    /// Try to acquire the single WebSocket connection slot.
    /// Returns `true` if this caller got it, `false` if already taken.
    pub fn try_acquire_ws(&self) -> bool {
        self.inner
            .ws_connected
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Release the WebSocket connection slot.
    pub fn release_ws(&self) {
        self.inner.ws_connected.store(false, Ordering::Release);
    }

    /// Validate a shutdown token against the one generated at startup.
    pub fn validate_shutdown_token(&self, token: &str) -> bool {
        self.inner.shutdown_token == token
    }

    /// Signal graceful server shutdown. Stops FPSS streaming if active.
    pub fn shutdown(&self) {
        self.inner.tdx.stop_streaming();
        self.inner.shutdown.notify_waiters();
    }

    /// Wait for the shutdown signal.
    pub async fn shutdown_signal(&self) {
        self.inner.shutdown.notified().await;
    }
}
