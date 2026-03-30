//! Shared application state for the REST + WebSocket server.
//!
//! Holds the unified `ThetaDataDx` client, connection flags, WebSocket
//! broadcast channel, and shutdown plumbing. All fields are `Send + Sync`
//! behind `Arc` so axum can cheaply clone state into each handler.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use thetadatadx::direct::DirectClient;
use thetadatadx::fpss::protocol::Contract;
use thetadatadx::ThetaDataDx;
use tokio::sync::broadcast;

/// Capacity of the broadcast channel used to fan out FPSS events to WebSocket
/// clients.  4096 is chosen because:
///
/// - FPSS can burst ~10k events/sec during market open (quotes + trades +
///   OHLCVC for all subscribed contracts).  A buffer of 4096 gives ~400ms of
///   headroom before a slow WebSocket consumer starts losing messages (at
///   which point `broadcast::Receiver` returns `Lagged` and the consumer
///   catches up to the tail).
/// - Each message is a small JSON string (~200-500 bytes), so 4096 slots cost
///   roughly 1-2 MB of memory -- negligible on a server.
/// - Powers of two are preferred because tokio's broadcast channel internally
///   masks indices, making power-of-two sizes slightly more efficient.
const WS_BROADCAST_CAPACITY: usize = 4096;

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
    /// Broadcast channel: FPSS events -> WebSocket clients.
    ws_tx: broadcast::Sender<String>,
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
        let (ws_tx, _) = broadcast::channel(WS_BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                tdx,
                mdds_connected: AtomicBool::new(true),
                fpss_connected: AtomicBool::new(false),
                ws_tx,
                shutdown: tokio::sync::Notify::new(),
                ws_connected: AtomicBool::new(false),
                contract_map: Arc::new(Mutex::new(HashMap::new())),
                shutdown_token,
            }),
        }
    }

    /// Borrow the `DirectClient` (via Deref) for issuing gRPC requests.
    pub fn client(&self) -> &DirectClient {
        &self.inner.tdx
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

    /// Get a new broadcast receiver for WebSocket events.
    pub fn subscribe_ws(&self) -> broadcast::Receiver<String> {
        self.inner.ws_tx.subscribe()
    }

    /// Send a JSON event string to all connected WebSocket clients.
    pub fn broadcast_ws(&self, event: String) {
        // Ignore send errors (no receivers = nobody connected).
        let _ = self.inner.ws_tx.send(event);
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
