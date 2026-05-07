//! Unified `ThetaData` client -- single entry point, one auth, lazy FPSS.
//!
//! Connect once. Use historical data immediately. Streaming connects
//! on-demand when you first subscribe -- not at startup.
//!
//! ```rust,no_run
//! use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), thetadatadx::Error> {
//!     // One connect, one auth. FPSS is NOT connected yet.
//!     // Or inline: Credentials::new("user@example.com", "your-password")
//!     let tdx = ThetaDataDx::connect(
//!         &Credentials::from_file("creds.txt")?,
//!         DirectConfig::production(),
//!     ).await?;
//!
//!     // Historical -- works immediately
//!     let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
//!
//!     // Streaming -- FPSS connects lazily on first subscribe
//!     use thetadatadx::fpss::{FpssData, FpssEvent};
//!     use thetadatadx::fpss::protocol::Contract;
//!     tdx.start_streaming(|event| {
//!         if let FpssEvent::Data(FpssData::Trade { price, size, .. }) = event {
//!             println!("trade {price} x {size}");
//!         }
//!     })?;
//!     tdx.subscribe_quotes(&Contract::stock("AAPL"))?;
//!
//!     Ok(())
//! }
//! ```

use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::auth::Credentials;
use crate::config::DirectConfig;
use crate::error::Error;
use crate::fpss::protocol::{Contract, SubscriptionKind};
use crate::fpss::{FpssClient, FpssEvent};
use crate::mdds::MddsClient;
use tdbe::types::enums::SecType;

/// Snapshot of the streaming side of the unified client.
///
/// One [`ArcSwap`] cell so every read path collapses to a single
/// atomic load. The previous design carried a separate
/// `Mutex<Option<StreamingDispatcher>>` alongside the [`FpssClient`];
/// after the post-#513 single-queue rewrite the user callback runs
/// directly on the Disruptor consumer thread inside [`FpssClient`],
/// so the slot only needs to track the live client.
///
/// Lifecycle: `Idle` (constructed) → `Live` (`start_streaming`
/// succeeded) → `Stopped` (`stop_streaming` returned). A subsequent
/// `start_streaming` from `Stopped` swaps back to `Live`; `Idle` is
/// reachable only at construction time, never re-entered after a
/// successful start.
enum StreamingSlot {
    /// `start_streaming()` has not been called yet.
    Idle,
    /// Streaming connection is established. The user callback runs
    /// inside the [`FpssClient`]'s Disruptor consumer thread (panic
    /// isolated via `catch_unwind`); ring-buffer overflow is reported
    /// through [`FpssClient::dropped_count`].
    Live { client: Arc<FpssClient> },
    /// `stop_streaming()` ran (or `Drop` did). Distinguishes "was
    /// started, then stopped" from "never started" for
    /// [`ConnectionStatus::Disconnected`] vs
    /// [`ConnectionStatus::NotStarted`].
    Stopped,
}

/// Subscription tier information captured at authentication time.
#[derive(Debug, Clone)]
pub struct SubscriptionInfo {
    /// Stock data subscription tier (e.g. "Free", "Value", "Standard", "Pro").
    pub stock: String,
    /// Options data subscription tier (e.g. "Free", "Value", "Standard", "Pro").
    pub options: String,
}

/// Current state of the streaming connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionStatus {
    /// `start_streaming()` has not been called yet.
    NotStarted,
    /// Connected and authenticated.
    Connected,
    /// Currently attempting to reconnect after an involuntary disconnect.
    Reconnecting,
    /// Explicitly stopped or failed to connect.
    Disconnected,
}

/// Unified `ThetaData` client.
///
/// Authenticates once at connect time. Historical data (MDDS gRPC) is
/// available immediately. Streaming (FPSS TCP) connects lazily when
/// you call [`start_streaming`](Self::start_streaming).
///
/// All historical endpoint methods are available via `Deref` to
/// [`MddsClient`]. Streaming methods are on this struct directly.
pub struct ThetaDataDx {
    historical: MddsClient,
    creds: Credentials,
    /// Streaming-side state machine. See [`StreamingSlot`] for the
    /// `Idle → Live → Stopped` lifecycle. The
    /// [`ArcSwap`] makes `is_streaming` / `connection_status` /
    /// `with_streaming` single-atomic-load reads — the previous design
    /// took two `Mutex` locks plus an `AtomicBool` for the same answer.
    state: ArcSwap<StreamingSlot>,
}

impl ThetaDataDx {
    /// Connect to `ThetaData`. Authenticates once, opens gRPC channel.
    ///
    /// FPSS streaming is NOT connected yet -- call [`ThetaDataDx::start_streaming`]
    /// when you need real-time data.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error> {
        // Start the Prometheus exporter BEFORE opening the gRPC channel
        // so the first `thetadatadx.grpc.requests` counter hit is already
        // covered. No-op when the feature is disabled or `metrics_port`
        // is `None` (the default).
        crate::observability::try_install_exporter(&config)?;
        let historical = MddsClient::connect(creds, config).await?;
        Ok(Self {
            historical,
            creds: creds.clone(),
            state: ArcSwap::from_pointee(StreamingSlot::Idle),
        })
    }

    /// Helper: build a [`StreamingSlot::Live`] cell from a freshly
    /// connected [`FpssClient`].
    fn live_slot(client: FpssClient) -> StreamingSlot {
        StreamingSlot::Live {
            client: Arc::new(client),
        }
    }

    /// Helper: error returned when `start_streaming*` is called while
    /// the slot is already [`StreamingSlot::Live`].
    fn already_streaming() -> Error {
        Error::Fpss {
            kind: crate::error::FpssErrorKind::ConnectionRefused,
            message: "streaming already started".into(),
        }
    }

    /// Start the FPSS streaming connection with a callback handler.
    ///
    /// Opens a TLS/TCP connection to `ThetaData`'s FPSS servers,
    /// authenticates with the same credentials used at connect time,
    /// and starts the FPSS reader thread plus the LMAX Disruptor
    /// consumer thread.
    ///
    /// # Pipeline (single-queue SSOT, post-#513)
    ///
    /// `TLS reader thread -> Disruptor ring (try_publish, non-blocking)
    /// -> Disruptor consumer thread -> catch_unwind(user callback)`.
    ///
    /// The TLS reader publishes every decoded event into a pre-allocated
    /// LMAX Disruptor ring via `Producer::try_publish`. A single
    /// dedicated consumer thread owned by the Disruptor invokes the
    /// user callback for each event, with each invocation wrapped in
    /// [`std::panic::catch_unwind`]. Two contracts:
    ///
    /// 1. **Reader never blocks on user code.** When the consumer
    ///    falls behind and the ring is full, `try_publish` returns
    ///    [`disruptor::RingBufferFull`], the event is dropped, and
    ///    [`Self::dropped_event_count`] increments. Operators should
    ///    poll the counter on a periodic timer.
    /// 2. **User panics never kill the consumer.** A panic from user
    ///    code (or from binding glue such as PyO3 / napi) is caught,
    ///    [`Self::panic_count`] increments, and the consumer keeps
    ///    delivering subsequent events.
    /// 3. **Calling [`Self::stop_streaming`] from inside the callback
    ///    is safe.** The callback runs on the Disruptor consumer
    ///    thread, not on the thread that owns the [`FpssClient::drop`]
    ///    join. When the swap to `Stopped` releases the last
    ///    `Arc<FpssClient>` from inside the callback, `Drop` detects
    ///    the self-join case (callback thread == consumer thread that
    ///    the I/O thread's exit path joins) and detaches the join onto
    ///    a helper thread. Cleanup completes asynchronously; observers
    ///    see [`Self::is_streaming`] flip to `false` once the helper
    ///    finishes, instead of `Drop` blocking forever.
    ///
    /// This is the default and only safe entry point for general use.
    /// The `expert-mode` feature flag exposes
    /// [`Self::start_streaming_inline`] for ultra-low-latency callers
    /// who can guarantee a wait-free callback and accept the contracts
    /// listed under that method's `# Future` heading.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn start_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        // Reject a second concurrent start before paying the connect
        // cost. The post-connect slot install below revalidates this
        // because another caller may race in during the connect; the
        // upfront check is just a fast-path optimisation.
        if matches!(&**self.state.load(), StreamingSlot::Live { .. }) {
            return Err(Self::already_streaming());
        }

        let config = self.historical.config();
        let client = FpssClient::connect(
            crate::fpss::FpssConnectArgs {
                creds: &self.creds,
                hosts: &config.fpss.hosts,
                ring_size: config.fpss.ring_size,
                flush_mode: config.fpss.flush_mode,
                policy: config.reconnect.policy.clone(),
                derive_ohlcvc: config.fpss.derive_ohlcvc,
            },
            handler,
        )?;

        self.install_live(Self::live_slot(client))
    }

    /// Expert-mode entry point retained behind the `expert-mode` Cargo
    /// feature for forward compatibility with a future TLS-reader-direct
    /// dispatch path.
    ///
    /// # Feature flag
    ///
    /// Gated behind the `expert-mode` feature (off by default). Users
    /// opt in via `features = ["expert-mode"]`. The default
    /// [`Self::start_streaming`] path covers every workload that is not
    /// a provably wait-free trading loop.
    ///
    /// # Current
    ///
    /// **`start_streaming_inline` and [`Self::start_streaming`]
    /// currently deliver the user callback on the same thread — the
    /// LMAX Disruptor consumer thread.** After the single-queue
    /// rewrite tracked in #513, `start_streaming_inline` delegates to
    /// the same pipeline as [`Self::start_streaming`] (one queue, one
    /// consumer, panic-isolated via `catch_unwind`). The previous
    /// "bypass the dispatcher" shortcut no longer exists because there
    /// is no second queue to bypass.
    ///
    /// The feature gate and method name stay in place so the future
    /// path described below can ship without a SemVer-breaking rename.
    ///
    /// Calling [`Self::stop_streaming`] from inside the callback is
    /// safe today: `FpssClient::Drop` detects the consumer-thread
    /// self-join case via the `consumer_thread_id` field captured on
    /// first dispatch and detaches the join onto a helper thread.
    /// Cleanup completes asynchronously; observers see
    /// [`Self::is_streaming`] flip to `false` once the helper finishes.
    ///
    /// # Future
    ///
    /// A future revision will run the user callback **directly on the
    /// TLS reader thread**, bypassing the Disruptor ring entirely. On
    /// that future path:
    ///
    /// - The `catch_unwind` boundary will not be present — an
    ///   unhandled panic will terminate the reader thread and tear
    ///   down the FPSS session.
    /// - The reader thread owns reads, outbound command drain, and
    ///   ping heartbeat dispatch. A blocking, allocating, or
    ///   lock-holding callback will stall all three: kernel TCP
    ///   receive buffer fills, the vendor disconnects the session,
    ///   every active subscription drops.
    /// - Calling [`Self::stop_streaming`] / [`Self::reconnect_streaming`]
    ///   from inside the callback will be **forbidden**: the future
    ///   path has no second thread to detach the join onto, so the
    ///   self-join is unrecoverable rather than just asynchronous.
    ///
    /// Reach for this entry point only inside trading loops with
    /// statically-known wait-free callbacks. For Python / Node / Go
    /// bindings, WebSocket fan-out, file logging, or any callback that
    /// allocates, call [`Self::start_streaming`] instead.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    #[cfg(feature = "expert-mode")]
    pub fn start_streaming_inline<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        if matches!(&**self.state.load(), StreamingSlot::Live { .. }) {
            return Err(Self::already_streaming());
        }
        let config = self.historical.config();
        let client = FpssClient::connect(
            crate::fpss::FpssConnectArgs {
                creds: &self.creds,
                hosts: &config.fpss.hosts,
                ring_size: config.fpss.ring_size,
                flush_mode: config.fpss.flush_mode,
                policy: config.reconnect.policy.clone(),
                derive_ohlcvc: config.fpss.derive_ohlcvc,
            },
            handler,
        )?;
        self.install_live(Self::live_slot(client))
    }

    /// Atomically swap the slot to a fresh `Live` state.
    ///
    /// Rejects the install when the slot raced into `Live` between the
    /// caller's `start_streaming*` precheck and the FPSS connect
    /// returning successfully. On rejection the freshly built
    /// [`FpssClient`] is dropped, which triggers its reader-thread
    /// shutdown and detaches the dispatcher cleanly.
    fn install_live(&self, new_slot: StreamingSlot) -> Result<(), Error> {
        let new = Arc::new(new_slot);
        // CAS loop: only swap from `Idle` or `Stopped` into `Live`.
        // ArcSwap doesn't expose `compare_and_swap` on `&Arc<T>` directly
        // for non-Eq T; we instead read, decide, and rcu the state. The
        // `rcu` closure is retried until the swap is observed atomically.
        let prev = self.state.rcu(|current| match &**current {
            StreamingSlot::Live { .. } => Arc::clone(current),
            _ => Arc::clone(&new),
        });
        if matches!(&*prev, StreamingSlot::Live { .. }) {
            // Lost the race: another start_streaming installed first.
            // `new` falls out of scope and shuts down its FPSS client.
            return Err(Self::already_streaming());
        }
        Ok(())
    }

    /// Snapshot of events the TLS reader could not publish into the
    /// Disruptor ring because the consumer fell behind and the ring
    /// was full. Returns `0` when streaming has not started.
    ///
    /// Operators should poll this on a periodic timer (e.g. every
    /// second) and emit a `warn` log on any non-zero delta. A
    /// per-drop log would amplify under sustained overflow.
    #[must_use]
    pub fn dropped_event_count(&self) -> u64 {
        let snap = self.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.dropped_count(),
            StreamingSlot::Idle | StreamingSlot::Stopped => 0,
        }
    }

    /// Snapshot of user-callback panics caught by the Disruptor
    /// consumer's `catch_unwind` boundary. Each panic is also
    /// surfaced via `tracing::error!` with target
    /// `thetadatadx::fpss::io_loop`. Returns `0` when streaming has
    /// not started.
    #[must_use]
    pub fn panic_count(&self) -> u64 {
        let snap = self.state.load();
        match &**snap {
            StreamingSlot::Live { client } => client.panic_count(),
            StreamingSlot::Idle | StreamingSlot::Stopped => 0,
        }
    }

    /// Whether streaming is currently active.
    pub fn is_streaming(&self) -> bool {
        matches!(&**self.state.load(), StreamingSlot::Live { .. })
    }

    // -- Streaming convenience methods --

    fn with_streaming<R>(
        &self,
        f: impl FnOnce(&FpssClient) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let snap = self.state.load();
        match &**snap {
            StreamingSlot::Live { client } => f(client.as_ref()),
            StreamingSlot::Idle | StreamingSlot::Stopped => Err(Error::Fpss {
                kind: crate::error::FpssErrorKind::Disconnected,
                message: "streaming not started -- call start_streaming() first".into(),
            }),
        }
    }

    /// Subscribe to quote updates for a contract.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_quotes(&self, contract: &Contract) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_quotes(contract))
    }

    /// Subscribe to trade updates for a contract.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_trades(&self, contract: &Contract) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_trades(contract))
    }

    /// Subscribe to open interest updates for a contract.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_open_interest(&self, contract: &Contract) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_open_interest(contract))
    }

    /// Subscribe to quotes + trades for a contract (convenience batch).
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_all(&self, contract: &Contract) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_all(contract))
    }

    /// Subscribe to all trades for a security type (full-stream).
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_full_trades(&self, sec_type: SecType) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_full_trades(sec_type))
    }

    /// Subscribe to all open interest for a security type (full-stream).
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_full_open_interest(&self, sec_type: SecType) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_full_open_interest(sec_type))
    }

    /// Unsubscribe from quote updates for a contract.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe_quotes(&self, contract: &Contract) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_quotes(contract))
    }

    /// Unsubscribe from trade updates for a contract.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe_trades(&self, contract: &Contract) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_trades(contract))
    }

    /// Unsubscribe from open interest updates for a contract.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe_open_interest(&self, contract: &Contract) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_open_interest(contract))
    }

    /// Unsubscribe from all trades for a security type.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe_full_trades(&self, sec_type: SecType) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_full_trades(sec_type))
    }

    /// Unsubscribe from all open interest for a security type.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn unsubscribe_full_open_interest(&self, sec_type: SecType) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_full_open_interest(sec_type))
    }

    /// Get all active per-contract subscriptions.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn active_subscriptions(&self) -> Result<Vec<(SubscriptionKind, Contract)>, Error> {
        self.with_streaming(|s| Ok(s.active_subscriptions()))
    }

    /// Get all active full-type (full-stream) subscriptions.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn active_full_subscriptions(&self) -> Result<Vec<(SubscriptionKind, SecType)>, Error> {
        self.with_streaming(|s| Ok(s.active_full_subscriptions()))
    }

    /// Shut down the streaming connection. Historical remains available.
    ///
    /// Idempotent: calling on an `Idle` or `Stopped` slot is a no-op,
    /// repeated calls during the drain race are safe (only the first
    /// observer of the `Live` slot performs the shutdown sequence).
    pub fn stop_streaming(&self) {
        // Atomically swap to `Stopped`; whichever caller wins the swap
        // owns the previous `Arc<StreamingSlot>` and is the one that
        // runs the shutdown sequence.
        let prev = self.state.swap(Arc::new(StreamingSlot::Stopped));

        // Drop the FPSS client signal so its reader thread + Disruptor
        // consumer drain and exit. The actual join happens in
        // `FpssClient::Drop` when the last `Arc<FpssClient>` is dropped
        // (typically with `prev` going out of scope at end of scope).
        if let StreamingSlot::Live { client } = &*prev {
            client.shutdown();
        }
    }

    /// Reconnect the streaming connection, re-subscribing all previous subscriptions.
    ///
    /// This is the caller-driven equivalent of Java's `handleInvoluntaryDisconnect()`.
    /// It saves active subscriptions, stops the current streaming connection,
    /// starts a new one with the provided handler, and re-subscribes everything.
    ///
    /// # Sequence
    ///
    /// 1. Save active per-contract and full-type subscriptions
    /// 2. Stop the current streaming connection
    /// 3. Start a new streaming connection with the provided handler
    /// 4. Re-subscribe all saved subscriptions, collecting per-subscription
    ///    failures rather than aborting on the first error
    ///
    /// # Errors
    ///
    /// Returns [`Error::Fpss`], [`Error::Auth`], etc. when the underlying
    /// streaming session cannot be re-established (steps 1–3).
    ///
    /// Returns [`Error::PartialReconnect`] when the streaming session was
    /// re-established successfully but one or more saved subscriptions
    /// failed to restore. The variant carries the structured list of failed
    /// `(SubscriptionKind, Contract)` pairs so the caller can retry just
    /// those subscriptions or surface the partial failure to the operator.
    /// Per-subscription `tracing::warn!` lines are still emitted for
    /// operational visibility.
    pub fn reconnect_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        metrics::counter!("thetadatadx.fpss.reconnects").increment(1);
        // 1. Save active subscriptions before stopping
        let saved_subs = match &**self.state.load() {
            StreamingSlot::Live { client } => (
                client.active_subscriptions(),
                client.active_full_subscriptions(),
            ),
            StreamingSlot::Idle | StreamingSlot::Stopped => (Vec::new(), Vec::new()),
        };

        // 2. Stop streaming
        self.stop_streaming();

        // 3. Start a new streaming connection
        self.start_streaming(handler)?;

        // 4. Re-subscribe all saved subscriptions, accumulating failures
        let (per_contract, full_type) = saved_subs;
        let failed = restore_subscriptions(
            &per_contract,
            &full_type,
            |kind, contract| match kind {
                SubscriptionKind::Quote => self.subscribe_quotes(contract),
                SubscriptionKind::Trade => self.subscribe_trades(contract),
                SubscriptionKind::OpenInterest => self.subscribe_open_interest(contract),
            },
            |kind, sec_type| match kind {
                SubscriptionKind::Trade => Some(self.subscribe_full_trades(sec_type)),
                SubscriptionKind::OpenInterest => Some(self.subscribe_full_open_interest(sec_type)),
                SubscriptionKind::Quote => None,
            },
        );

        if failed.is_empty() {
            Ok(())
        } else {
            Err(Error::PartialReconnect { failed })
        }
    }

    /// Get the current streaming connection status.
    pub fn connection_status(&self) -> ConnectionStatus {
        match &**self.state.load() {
            StreamingSlot::Idle => ConnectionStatus::NotStarted,
            StreamingSlot::Stopped => ConnectionStatus::Disconnected,
            StreamingSlot::Live { client } => {
                if client.is_authenticated() {
                    ConnectionStatus::Connected
                } else {
                    // The client exists but is not authenticated -- this happens
                    // during reconnection (authenticated flag is cleared on
                    // disconnect, restored on successful re-auth).
                    ConnectionStatus::Reconnecting
                }
            }
        }
    }

    /// Access the current MDDS session UUID.
    ///
    /// Returns an owned `String` rather than `&str` because the UUID
    /// lives behind a shared [`crate::auth::SessionToken`] that may be
    /// refreshed mid-session. Reads through the token so callers always
    /// see the current value.
    pub async fn session_uuid(&self) -> String {
        self.historical.session_uuid().await
    }

    /// Access the config.
    pub fn config(&self) -> &DirectConfig {
        self.historical.config()
    }

    /// Get subscription tier information captured at authentication time.
    pub fn subscription_info(&self) -> SubscriptionInfo {
        let label = |tier: Option<crate::mdds::SubscriptionTier>| match tier {
            Some(crate::mdds::SubscriptionTier::Free) => "Free".to_string(),
            Some(crate::mdds::SubscriptionTier::Value) => "Value".to_string(),
            Some(crate::mdds::SubscriptionTier::Standard) => "Standard".to_string(),
            Some(crate::mdds::SubscriptionTier::Pro) => "Pro".to_string(),
            None => "Unknown".to_string(),
        };
        SubscriptionInfo {
            stock: label(self.historical.stock_tier()),
            options: label(self.historical.options_tier()),
        }
    }

    // ---------------------------------------------------------------------
    // FLATFILES surface (third public surface, alongside FPSS and MDDS).
    //
    // The legacy MDDS port (12000) speaks a custom binary PacketStream
    // protocol that supports a single FLAT_FILE request type. The server
    // pre-builds an INDEX + DATA blob per (sec_type, data_type, date)
    // tuple overnight and streams it back on demand. See
    // [`crate::flatfiles`] for the wire-format details and the decode /
    // writer implementation used by this surface, covering CSV and
    // JSONL output plus a typed in-memory return path.
    // ---------------------------------------------------------------------

    /// Pull a flat-file blob for `(sec_type, req_type, date)` over the legacy
    /// MDDS port, decode it, and write the requested `format` to disk.
    ///
    /// `format` selects the on-disk encoding:
    /// - [`crate::flatfiles::FlatFileFormat::Csv`] — vendor byte-format CSV
    ///   (lowercase headers, comma-separated, no quoting). Byte-matches the
    ///   legacy terminal's downloads on the same input.
    /// - [`crate::flatfiles::FlatFileFormat::Jsonl`] — JSON Lines, one
    ///   object per row.
    ///
    /// If `output_path` lacks a file extension, the format's canonical
    /// extension (`csv` / `jsonl`) is appended automatically.
    ///
    /// For columnar consumers (Parquet, Arrow IPC, polars) use
    /// [`Self::flatfile_request_decoded`] and feed the resulting
    /// `Vec<FlatFileRow>` into the writer of your choice — the SDK does
    /// not pull in Parquet / Arrow itself.
    ///
    /// # Errors
    /// Returns [`Error::FlatFilesUnavailable`] for auth / server
    /// rejection, [`Error::Config`] for malformed wire bytes, or
    /// [`Error::Io`] for local I/O issues.
    pub async fn flatfile_request(
        &self,
        sec_type: crate::flatfiles::SecType,
        req_type: crate::flatfiles::ReqType,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        crate::flatfiles::flatfile_request(
            &self.creds,
            sec_type,
            req_type,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Pull a flat-file blob and return decoded rows in memory.
    ///
    /// Same auth and stream path as [`Self::flatfile_request`], but skips
    /// the on-disk writer. Returns a `Vec<FlatFileRow>` ready to feed into
    /// an algorithm (backtester, risk model, in-memory analytics) without
    /// an intermediate file.
    ///
    /// The whole vector is materialised before the function returns; for
    /// whole-universe blobs that can be hundreds of MB.
    ///
    /// # Errors
    /// Same conditions as [`Self::flatfile_request`].
    pub async fn flatfile_request_decoded(
        &self,
        sec_type: crate::flatfiles::SecType,
        req_type: crate::flatfiles::ReqType,
        date: &str,
    ) -> Result<Vec<crate::flatfiles::FlatFileRow>, Error> {
        crate::flatfiles::flatfile_request_decoded(&self.creds, sec_type, req_type, date).await
    }

    /// Convenience: option open-interest flat file for `date`.
    pub async fn flatfile_option_open_interest(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::OpenInterest,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: option trade-quote flat file for `date`.
    pub async fn flatfile_option_trade_quote(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::TradeQuote,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: option trade flat file for `date`.
    pub async fn flatfile_option_trade(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::Trade,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: option quote flat file for `date`.
    pub async fn flatfile_option_quote(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::Quote,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: option end-of-day flat file for `date`.
    pub async fn flatfile_option_eod(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::Eod,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: stock trade-quote flat file for `date`.
    pub async fn flatfile_stock_trade_quote(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::TradeQuote,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: stock trade flat file for `date`.
    pub async fn flatfile_stock_trade(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::Trade,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: stock quote flat file for `date`.
    pub async fn flatfile_stock_quote(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::Quote,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: stock end-of-day flat file for `date`.
    pub async fn flatfile_stock_eod(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::Eod,
            date,
            output_path,
            format,
        )
        .await
    }
}

impl Drop for ThetaDataDx {
    /// Final cleanup: idempotently stops the streaming connection.
    ///
    /// `stop_streaming` swaps the state cell to `Stopped` and only
    /// signals the FPSS client when the previous slot was `Live`.
    /// The actual TLS reader + Disruptor consumer join happens when
    /// the last `Arc<FpssClient>` is dropped via `FpssClient::Drop`.
    /// Calling once from `Drop` after the user already called
    /// `stop_streaming` is therefore a no-op — the state machine
    /// guarantees the shutdown signal runs exactly once.
    fn drop(&mut self) {
        self.stop_streaming();
    }
}

// All historical methods available directly via Deref.
impl std::ops::Deref for ThetaDataDx {
    type Target = MddsClient;
    fn deref(&self) -> &MddsClient {
        &self.historical
    }
}

/// Replay every saved subscription against the freshly reconnected
/// streaming client and return the list of subscriptions that failed to
/// restore.
///
/// The two callbacks decouple the loop from the live `ThetaDataDx`
/// streaming methods so the resubscription logic is unit-testable with
/// in-memory fakes — the [`reconnect_streaming`] caller in production
/// passes through to the real `subscribe_quotes` / `subscribe_trades` /
/// `subscribe_open_interest` and `subscribe_full_*` methods, while the
/// regression test below injects closures that return canned `Err` for a
/// specific subscription pair to prove the failure list carries the right
/// structured contents.
///
/// Per-failure operational visibility is preserved: every error path emits a
/// `tracing::warn!` line carrying `kind`, `contract` (or `sec_type`), and
/// the underlying error, identical to the single-call-site loop this
/// helper replaces.
///
/// `full_subscribe` returns `Some(Result<()>)` for kinds that are valid
/// full-type subscriptions, and `None` for kinds that are not (currently
/// only `SubscriptionKind::Quote` is excluded). A `None` triggers the same
/// "skipping" warning the previous in-line loop emitted.
fn restore_subscriptions<P, F>(
    per_contract: &[(SubscriptionKind, Contract)],
    full_type: &[(SubscriptionKind, SecType)],
    mut per_subscribe: P,
    mut full_subscribe: F,
) -> Vec<(SubscriptionKind, Contract)>
where
    P: FnMut(SubscriptionKind, &Contract) -> Result<(), Error>,
    F: FnMut(SubscriptionKind, SecType) -> Option<Result<(), Error>>,
{
    let mut failed: Vec<(SubscriptionKind, Contract)> = Vec::new();

    for (kind, contract) in per_contract {
        if let Err(e) = per_subscribe(*kind, contract) {
            tracing::warn!(
                kind = ?kind,
                contract = %contract,
                error = %e,
                "failed to re-subscribe after reconnect"
            );
            failed.push((*kind, contract.clone()));
        }
    }

    for (kind, sec_type) in full_type {
        match full_subscribe(*kind, *sec_type) {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                tracing::warn!(
                    kind = ?kind,
                    sec_type = ?sec_type,
                    error = %e,
                    "failed to re-subscribe full-type after reconnect"
                );
                // Full-type subscriptions are encoded as a synthetic
                // `Contract` with an empty `root` so the structured failure
                // list stays homogeneous. Operators see the original
                // `sec_type` via the `tracing::warn!` line above.
                failed.push((*kind, Contract::full_type_marker(*sec_type)));
            }
            None => {
                tracing::warn!(
                    kind = ?kind,
                    sec_type = ?sec_type,
                    "full-type subscription is not supported for this kind, skipping"
                );
            }
        }
    }

    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lightweight stand-in for `StreamingSlot` carrying just enough
    /// shape to walk the state machine transitions without spinning up
    /// a real FPSS connection. The transitions and the `ArcSwap`
    /// install/swap mechanics are what we are validating; the live
    /// payload (`FpssClient`, `StreamingDispatcher`) is exercised by
    /// the existing FPSS integration tests.
    enum SlotMarker {
        Idle,
        Live(u32),
        Stopped,
    }

    fn variant(s: &SlotMarker) -> &'static str {
        match s {
            SlotMarker::Idle => "Idle",
            SlotMarker::Live(_) => "Live",
            SlotMarker::Stopped => "Stopped",
        }
    }

    /// Walks Idle → Live → Stopped → Live → Stopped, asserting the
    /// `ArcSwap` cell observes each transition exactly once and that
    /// the `Live` payload (here a generation counter) is preserved
    /// across re-installs.
    #[test]
    fn streaming_slot_state_machine_transitions() {
        let cell: ArcSwap<SlotMarker> = ArcSwap::from_pointee(SlotMarker::Idle);

        // Idle observed.
        assert_eq!(variant(&cell.load()), "Idle");

        // Idle → Live(1)
        let prev = cell.swap(Arc::new(SlotMarker::Live(1)));
        assert_eq!(variant(&prev), "Idle");
        assert_eq!(variant(&cell.load()), "Live");

        // Live(1) → Stopped
        let prev = cell.swap(Arc::new(SlotMarker::Stopped));
        assert!(matches!(&*prev, SlotMarker::Live(1)));
        assert_eq!(variant(&cell.load()), "Stopped");

        // Stopped → Live(2)  — the second start path
        let prev = cell.swap(Arc::new(SlotMarker::Live(2)));
        assert_eq!(variant(&prev), "Stopped");
        assert!(matches!(&**cell.load(), SlotMarker::Live(2)));

        // Live(2) → Stopped (second shutdown)
        let prev = cell.swap(Arc::new(SlotMarker::Stopped));
        assert!(matches!(&*prev, SlotMarker::Live(2)));
        assert_eq!(variant(&cell.load()), "Stopped");
    }

    /// Concurrent `start` race: only one caller observes the install,
    /// the other sees `Live` and must reject. Modeled with the same
    /// rcu CAS the real `install_live` uses.
    #[test]
    fn streaming_slot_rejects_double_install() {
        let cell: ArcSwap<SlotMarker> = ArcSwap::from_pointee(SlotMarker::Idle);

        let new1 = Arc::new(SlotMarker::Live(1));
        let prev = cell.rcu(|cur| match &**cur {
            SlotMarker::Live(_) => Arc::clone(cur),
            _ => Arc::clone(&new1),
        });
        assert!(matches!(&*prev, SlotMarker::Idle));
        assert_eq!(variant(&cell.load()), "Live");

        // Second installer races in: must observe `Live` from `prev`.
        let new2 = Arc::new(SlotMarker::Live(2));
        let prev = cell.rcu(|cur| match &**cur {
            SlotMarker::Live(_) => Arc::clone(cur),
            _ => Arc::clone(&new2),
        });
        assert!(
            matches!(&*prev, SlotMarker::Live(1)),
            "second installer must see existing Live(1) and bail"
        );
        // Cell is unchanged: still Live(1), the Live(2) install was rejected.
        assert!(matches!(&**cell.load(), SlotMarker::Live(1)));
    }

    /// Inject a single failing per-contract subscribe call and prove the
    /// returned failure list contains exactly the failed `(kind, contract)`
    /// pair — not a count, not a boolean, the real structured contents.
    #[test]
    fn restore_subscriptions_collects_failed_per_contract() {
        let aapl = Contract::stock("AAPL");
        let msft = Contract::stock("MSFT");
        let per_contract = vec![
            (SubscriptionKind::Quote, aapl.clone()),
            (SubscriptionKind::Quote, msft.clone()),
        ];
        let full_type: Vec<(SubscriptionKind, SecType)> = Vec::new();

        let failed = restore_subscriptions(
            &per_contract,
            &full_type,
            |_kind, contract| {
                if contract.symbol == "MSFT" {
                    Err(Error::Fpss {
                        kind: crate::error::FpssErrorKind::Disconnected,
                        message: "injected: MSFT subscribe rejected".to_string(),
                    })
                } else {
                    Ok(())
                }
            },
            |_, _| None,
        );

        assert_eq!(failed.len(), 1, "exactly one subscription must have failed");
        assert_eq!(failed[0].0, SubscriptionKind::Quote);
        assert_eq!(failed[0].1, msft);
    }

    /// A successful run must return an empty failure list — no false
    /// positives, no spurious entries.
    #[test]
    fn restore_subscriptions_empty_on_full_success() {
        let aapl = Contract::stock("AAPL");
        let per_contract = vec![(SubscriptionKind::Trade, aapl)];
        let full_type = vec![(SubscriptionKind::Trade, SecType::Stock)];

        let failed = restore_subscriptions(
            &per_contract,
            &full_type,
            |_, _| Ok(()),
            |_, _| Some(Ok(())),
        );

        assert!(failed.is_empty(), "no failures expected, got {failed:?}");
    }

    /// A full-type subscription failure must show up in the list with the
    /// `full_type_marker` synthetic contract carrying the right `SecType`,
    /// so callers can pattern-match the failure without losing the
    /// originally failed sec_type.
    #[test]
    fn restore_subscriptions_records_full_type_failure() {
        let per_contract: Vec<(SubscriptionKind, Contract)> = Vec::new();
        let full_type = vec![(SubscriptionKind::OpenInterest, SecType::Option)];

        let failed = restore_subscriptions(
            &per_contract,
            &full_type,
            |_, _| Ok(()),
            |_, _| {
                Some(Err(Error::Fpss {
                    kind: crate::error::FpssErrorKind::TooManyRequests,
                    message: "injected: full-type subscribe rate-limited".to_string(),
                }))
            },
        );

        assert_eq!(failed.len(), 1);
        let (kind, contract) = &failed[0];
        assert_eq!(*kind, SubscriptionKind::OpenInterest);
        assert_eq!(contract.sec_type, SecType::Option);
        assert!(
            contract.symbol.is_empty(),
            "full-type marker carries empty root, got {:?}",
            contract.symbol
        );
    }

    /// `reconnect_streaming` returns `Error::PartialReconnect` carrying the
    /// failed list when subscriptions cannot be restored — the regression
    /// test for issue #461. The variant payload is asserted by pattern-
    /// match, not just `is_err()`, so a future refactor that changes the
    /// payload shape breaks this test loudly.
    #[test]
    fn partial_reconnect_error_carries_failed_subscriptions() {
        let aapl = Contract::stock("AAPL");
        let per_contract = vec![(SubscriptionKind::Quote, aapl.clone())];
        let full_type: Vec<(SubscriptionKind, SecType)> = Vec::new();

        let failed = restore_subscriptions(
            &per_contract,
            &full_type,
            |_, _| {
                Err(Error::Fpss {
                    kind: crate::error::FpssErrorKind::Disconnected,
                    message: "injected".to_string(),
                })
            },
            |_, _| None,
        );

        // This is exactly the path `reconnect_streaming` takes when failed
        // is non-empty: build the structured `PartialReconnect` error.
        let err = if failed.is_empty() {
            None
        } else {
            Some(Error::PartialReconnect { failed })
        };

        match err {
            Some(Error::PartialReconnect { failed }) => {
                assert_eq!(failed.len(), 1);
                assert_eq!(failed[0].0, SubscriptionKind::Quote);
                assert_eq!(failed[0].1, aapl);
            }
            other => panic!("expected PartialReconnect, got {other:?}"),
        }
    }
}
