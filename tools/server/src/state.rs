//! Shared application state for the REST + WebSocket server.
//!
//! Holds the unified `Client` client, connection flags, per-client
//! WebSocket channels, and shutdown plumbing. All fields are `Send + Sync`
//! behind `Arc` so axum can cheaply clone state into each handler.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use thetadatadx::Client;
use tokio::sync::{mpsc, RwLock};

/// Default per-client channel capacity. Matches the old `broadcast::channel(4096)`.
///
/// At ~10k events/sec peak (market open), 4096 gives ~400ms of headroom
/// before a slow WebSocket consumer starts dropping events.  Each slot is
/// an `Arc<str>` (~16 bytes), so 4096 slots cost ~64KB per client.
///
/// An operator can override this with the `THETADATADX_WS_CLIENT_CAPACITY`
/// env var (see [`ws_client_capacity`]) — a larger buffer trades memory for
/// more headroom against a slow consumer on a high-rate stream.
const WS_CLIENT_CAPACITY: usize = 4096;

/// Environment variable that overrides the per-client WS channel capacity.
const ENV_WS_CLIENT_CAPACITY: &str = "THETADATADX_WS_CLIENT_CAPACITY";

/// Resolve the per-client WS channel capacity from the environment, falling
/// back to [`WS_CLIENT_CAPACITY`] when `THETADATADX_WS_CLIENT_CAPACITY` is
/// unset, unparseable, or zero. A zero-capacity bounded `mpsc::channel`
/// would reject every `try_send`, so it is treated as invalid and ignored.
fn ws_client_capacity() -> usize {
    ws_client_capacity_from(std::env::var(ENV_WS_CLIENT_CAPACITY).ok().as_deref())
}

/// Pure core of [`ws_client_capacity`], split out so the override and the
/// fallback-on-invalid behaviour can be tested without touching env state.
fn ws_client_capacity_from(raw: Option<&str>) -> usize {
    raw.and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(WS_CLIENT_CAPACITY)
}

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
    /// Unified client (historical via Deref to HistoricalClient, streaming via start_streaming).
    client: Client,
    /// Whether MDDS is connected (true after successful init).
    mdds_connected: AtomicBool,
    /// Whether FPSS is connected (set by the FPSS bridge callback).
    fpss_connected: AtomicBool,
    /// Per-client channels: FPSS events -> WebSocket clients (zero-copy fan-out).
    ws_clients: WsClients,
    /// Shutdown signal.
    shutdown: tokio::sync::Notify,
    /// Close signal of the currently active WebSocket session, if any.
    ///
    /// Single-client semantics with REPLACEMENT: when a new client
    /// connects, the previous session's `Notify` fires and that session
    /// closes its socket — matching the legacy terminal, which drops the
    /// existing client to let the new one in. A plain `Mutex` (never held
    /// across `.await`) is sufficient; the critical sections are
    /// pointer swaps.
    ws_session: std::sync::Mutex<Option<Arc<tokio::sync::Notify>>>,
    /// Monotonic count of FPSS events dropped on the bounded
    /// callback->broadcast handoff (see `ws::start_fpss_bridge`). Mirrors the
    /// FPSS SDK's per-handle `dropped_events()` counter so operators can
    /// scrape one number to detect WS-side back-pressure independent of the
    /// SDK-side event ring overrun counter.
    fpss_broadcast_dropped: AtomicU64,
}

impl AppState {
    /// Create new app state wrapping a connected `Client`.
    pub fn new(client: Client) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                mdds_connected: AtomicBool::new(true),
                fpss_connected: AtomicBool::new(false),
                ws_clients: Arc::new(RwLock::new(Vec::new())),
                shutdown: tokio::sync::Notify::new(),
                ws_session: std::sync::Mutex::new(None),
                fpss_broadcast_dropped: AtomicU64::new(0),
            }),
        }
    }

    /// Increment the FPSS broadcast-drop counter and return the post-increment
    /// value. Called from the event-dispatch callback when the bounded
    /// callback->broadcast channel rejects a `try_send` because the broadcast
    /// task is lagging (back-pressure) or has exited (shutdown).
    pub fn record_fpss_broadcast_drop(&self) -> u64 {
        self.inner
            .fpss_broadcast_dropped
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1)
    }

    /// Borrow the unified `Client` client.
    pub fn client(&self) -> &Client {
        &self.inner.client
    }

    /// MDDS connection status string matching the JVM terminal.
    pub fn mdds_status(&self) -> &'static str {
        if self.inner.mdds_connected.load(Ordering::Acquire) {
            "CONNECTED"
        } else {
            "DISCONNECTED"
        }
    }

    /// FPSS connection status string matching the JVM terminal.
    pub fn fpss_status(&self) -> &'static str {
        if self.inner.fpss_connected.load(Ordering::Acquire) {
            "CONNECTED"
        } else {
            "DISCONNECTED"
        }
    }

    /// Mark the streaming connection as connected or disconnected.
    pub fn set_streaming_connected(&self, connected: bool) {
        self.inner
            .fpss_connected
            .store(connected, Ordering::Release);
    }

    /// Register a new WS client, returning the receiver half of its channel.
    pub async fn register_ws_client(&self) -> mpsc::Receiver<Arc<str>> {
        let (tx, rx) = mpsc::channel(ws_client_capacity());
        self.inner.ws_clients.write().await.push(tx);
        rx
    }

    /// Fan out a JSON event to all connected WebSocket clients (zero-copy).
    ///
    /// Each client receives an `Arc::clone` of the same backing string --
    /// the JSON payload is serialized exactly once regardless of client count.
    ///
    /// Async because the broadcast task now runs inside `tokio::spawn`
    /// (see `ws.rs`). Earlier revisions ran this from the FPSS event ring
    /// `std::thread` and used `blocking_read`, which panics inside a
    /// tokio runtime. Using `read().await` yields the executor while
    /// waiting on the `RwLock`, matching the async context.
    ///
    /// If a per-client channel is full, that single slow client's event is
    /// dropped and a warning is logged -- the same backpressure semantics as
    /// the old `broadcast::channel`'s `Lagged` behavior.
    ///
    /// If a per-client channel is `Closed` (the receiver side was dropped
    /// because the WS handler exited), the dead sender is pruned inline --
    /// we re-acquire the lock in write mode and `retain` on `!is_closed()`
    /// before returning. This keeps the client list tight on a bursty
    /// disconnect storm instead of waiting for the next
    /// `cleanup_ws_clients()` at connection-close time, and prevents the
    /// `try_send` hot path from revisiting the same dead sender on every
    /// subsequent broadcast.
    pub async fn broadcast_ws(&self, event: Arc<str>) {
        let mut saw_closed = false;
        {
            let clients = self.inner.ws_clients.read().await;
            for tx in clients.iter() {
                match tx.try_send(Arc::clone(&event)) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        tracing::warn!("WebSocket client lagged, dropped event");
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        saw_closed = true;
                    }
                }
            }
        }
        if saw_closed {
            self.inner
                .ws_clients
                .write()
                .await
                .retain(|tx| !tx.is_closed());
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

    /// Begin a WebSocket session, atomically replacing any active one.
    ///
    /// Returns the close signal for the NEW session. If another session
    /// was active, its close signal fires so that session sends a Close
    /// frame and exits — the legacy terminal's drop-the-existing-client
    /// behavior. `Notify::notify_one` stores a permit when the previous
    /// session has not reached its `notified().await` yet, so the
    /// replacement signal can never be lost to a race.
    pub fn begin_ws_session(&self) -> Arc<tokio::sync::Notify> {
        let session = Arc::new(tokio::sync::Notify::new());
        let previous = self
            .inner
            .ws_session
            .lock()
            .expect("ws session lock is never poisoned: critical sections cannot panic")
            .replace(Arc::clone(&session));
        if let Some(previous) = previous {
            tracing::info!("new WebSocket client connected; closing the existing session");
            previous.notify_one();
        }
        session
    }

    /// End a WebSocket session previously begun with
    /// [`Self::begin_ws_session`].
    ///
    /// Clears the active slot only when `session` is still the current
    /// one — a replaced session exiting late must not evict its
    /// replacement.
    pub fn end_ws_session(&self, session: &Arc<tokio::sync::Notify>) {
        let mut slot = self
            .inner
            .ws_session
            .lock()
            .expect("ws session lock is never poisoned: critical sections cannot panic");
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            *slot = None;
        }
    }

    /// Signal graceful server shutdown. Stops FPSS streaming if active.
    pub fn shutdown(&self) {
        self.inner.client.stream().stop_streaming();
        self.inner.shutdown.notify_waiters();
    }

    /// Wait for the shutdown signal.
    pub async fn shutdown_signal(&self) {
        self.inner.shutdown.notified().await;
    }
}

#[cfg(test)]
mod tests {
    use super::{ws_client_capacity_from, WS_CLIENT_CAPACITY};
    use std::sync::Arc;

    /// Unset env falls back to the compiled-in default capacity.
    #[test]
    fn ws_capacity_defaults_when_env_unset() {
        assert_eq!(ws_client_capacity_from(None), WS_CLIENT_CAPACITY);
    }

    /// A valid override is read verbatim.
    #[test]
    fn ws_capacity_reads_env_override() {
        assert_eq!(ws_client_capacity_from(Some("8192")), 8192);
        assert_eq!(ws_client_capacity_from(Some("  256  ")), 256);
    }

    /// Unparseable or zero values fall back to the default rather than
    /// producing a useless zero-capacity channel.
    #[test]
    fn ws_capacity_invalid_falls_back_to_default() {
        assert_eq!(ws_client_capacity_from(Some("0")), WS_CLIENT_CAPACITY);
        assert_eq!(
            ws_client_capacity_from(Some("not-a-number")),
            WS_CLIENT_CAPACITY
        );
        assert_eq!(ws_client_capacity_from(Some("")), WS_CLIENT_CAPACITY);
    }

    /// Stand-in for the `Inner.ws_session` slot so the begin/end
    /// semantics can be pinned without constructing a live
    /// `Client`. Mirrors `AppState::begin_ws_session` /
    /// `end_ws_session` exactly.
    struct SessionSlot(std::sync::Mutex<Option<Arc<tokio::sync::Notify>>>);

    impl SessionSlot {
        fn new() -> Self {
            Self(std::sync::Mutex::new(None))
        }

        fn begin(&self) -> Arc<tokio::sync::Notify> {
            let session = Arc::new(tokio::sync::Notify::new());
            let previous = self.0.lock().unwrap().replace(Arc::clone(&session));
            if let Some(previous) = previous {
                previous.notify_one();
            }
            session
        }

        fn end(&self, session: &Arc<tokio::sync::Notify>) {
            let mut slot = self.0.lock().unwrap();
            if slot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, session))
            {
                *slot = None;
            }
        }

        fn active(&self) -> bool {
            self.0.lock().unwrap().is_some()
        }
    }

    /// A second session begin fires the first session's close signal —
    /// the replacement contract the WS handler's select loop relies on.
    #[tokio::test]
    async fn second_session_fires_first_sessions_close_signal() {
        let slot = SessionSlot::new();
        let first = slot.begin();
        let _second = slot.begin();

        // `notify_one` stores a permit, so the displaced session
        // observes the signal even though it subscribes after the swap.
        tokio::time::timeout(std::time::Duration::from_secs(1), first.notified())
            .await
            .expect("displaced session must observe its close signal");
    }

    /// A replaced session exiting late must not evict its replacement
    /// from the active slot.
    #[tokio::test]
    async fn stale_session_end_does_not_evict_replacement() {
        let slot = SessionSlot::new();
        let first = slot.begin();
        let second = slot.begin();

        slot.end(&first);
        assert!(slot.active(), "replacement session must stay active");

        slot.end(&second);
        assert!(!slot.active(), "current session end clears the slot");
    }

    #[tokio::test]
    async fn first_session_begin_fires_no_signal() {
        let slot = SessionSlot::new();
        let only = slot.begin();
        let waited =
            tokio::time::timeout(std::time::Duration::from_millis(50), only.notified()).await;
        assert!(
            waited.is_err(),
            "a lone session must not receive a close signal"
        );
    }
}
