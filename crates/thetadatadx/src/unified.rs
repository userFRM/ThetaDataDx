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

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::auth::Credentials;
use crate::config::DirectConfig;
use crate::error::Error;
use crate::fpss::protocol::{Contract, SubscriptionKind};
use crate::fpss::{FpssClient, FpssEvent};
use crate::mdds::MddsClient;
use tdbe::types::enums::SecType;

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
/// All 61 historical endpoint methods are available via `Deref` to
/// [`MddsClient`]. Streaming methods are on this struct directly.
pub struct ThetaDataDx {
    historical: MddsClient,
    streaming: Mutex<Option<FpssClient>>,
    creds: Credentials,
    /// Set to `true` once `start_streaming()` succeeds; never cleared.
    /// Used by `connection_status()` to distinguish "never started" from
    /// "was started but the client was dropped/stopped".
    was_streaming: AtomicBool,
}

impl ThetaDataDx {
    /// Connect to `ThetaData`. Authenticates once, opens gRPC channel.
    ///
    /// FPSS streaming is NOT connected yet -- call [`start_streaming`]
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
            streaming: Mutex::new(None),
            creds: creds.clone(),
            was_streaming: AtomicBool::new(false),
        })
    }

    /// Start the FPSS streaming connection with a callback handler.
    ///
    /// This opens a TLS/TCP connection to `ThetaData`'s FPSS servers,
    /// authenticates with the same credentials used at connect time,
    /// and starts the Disruptor ring buffer + I/O thread.
    ///
    /// The callback runs on the Disruptor consumer thread -- keep it fast.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn start_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        let mut guard = self
            .streaming
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.is_some() {
            return Err(Error::Fpss {
                kind: crate::error::FpssErrorKind::ConnectionRefused,
                message: "streaming already started".into(),
            });
        }
        let config = self.historical.config();
        let client = FpssClient::connect(
            &self.creds,
            &config.fpss_hosts,
            config.fpss_ring_size,
            config.fpss_flush_mode,
            config.reconnect_policy.clone(),
            config.derive_ohlcvc,
            handler,
        )?;
        *guard = Some(client);
        self.was_streaming.store(true, Ordering::Release);
        Ok(())
    }

    /// Whether streaming is currently active.
    pub fn is_streaming(&self) -> bool {
        self.streaming
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    // -- Streaming convenience methods --

    fn with_streaming<R>(
        &self,
        f: impl FnOnce(&FpssClient) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let guard = self
            .streaming
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let client = guard.as_ref().ok_or_else(|| Error::Fpss {
            kind: crate::error::FpssErrorKind::Disconnected,
            message: "streaming not started -- call start_streaming() first".into(),
        })?;
        f(client)
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

    /// Subscribe to all trades for a security type (firehose).
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn subscribe_full_trades(&self, sec_type: SecType) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_full_trades(sec_type))
    }

    /// Subscribe to all open interest for a security type (firehose).
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

    // -----------------------------------------------------------------------
    // Ergonomic stock/option shortcuts (mirror FpssClient surface).
    // -----------------------------------------------------------------------

    /// Subscribe to real-time quotes for a stock symbol.
    /// # Errors
    ///
    /// Returns an error on network or authentication failure.
    pub fn subscribe_quotes_stock(&self, symbol: &str) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_quotes_stock(symbol))
    }

    /// Subscribe to real-time trades for a stock symbol.
    /// # Errors
    ///
    /// Returns an error on network or authentication failure.
    pub fn subscribe_trades_stock(&self, symbol: &str) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_trades_stock(symbol))
    }

    /// Subscribe to open interest updates for a stock symbol.
    /// # Errors
    ///
    /// Returns an error on network or authentication failure.
    pub fn subscribe_open_interest_stock(&self, symbol: &str) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_open_interest_stock(symbol))
    }

    /// Subscribe to real-time quotes for an option contract.
    /// # Errors
    ///
    /// Returns `Error::Config` on parse failure, or a network error on send.
    pub fn subscribe_quotes_option(
        &self,
        root: &str,
        exp: &str,
        strike: &str,
        right: &str,
    ) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_quotes_option(root, exp, strike, right))
    }

    /// Subscribe to real-time trades for an option contract.
    /// # Errors
    ///
    /// Returns `Error::Config` on parse failure, or a network error on send.
    pub fn subscribe_trades_option(
        &self,
        root: &str,
        exp: &str,
        strike: &str,
        right: &str,
    ) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_trades_option(root, exp, strike, right))
    }

    /// Subscribe to open interest updates for an option contract.
    /// # Errors
    ///
    /// Returns `Error::Config` on parse failure, or a network error on send.
    pub fn subscribe_open_interest_option(
        &self,
        root: &str,
        exp: &str,
        strike: &str,
        right: &str,
    ) -> Result<(), Error> {
        self.with_streaming(|s| s.subscribe_open_interest_option(root, exp, strike, right))
    }

    /// Unsubscribe from quote data for a stock symbol.
    /// # Errors
    ///
    /// Returns an error on network or authentication failure.
    pub fn unsubscribe_quotes_stock(&self, symbol: &str) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_quotes_stock(symbol))
    }

    /// Unsubscribe from trade data for a stock symbol.
    /// # Errors
    ///
    /// Returns an error on network or authentication failure.
    pub fn unsubscribe_trades_stock(&self, symbol: &str) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_trades_stock(symbol))
    }

    /// Unsubscribe from open interest data for a stock symbol.
    /// # Errors
    ///
    /// Returns an error on network or authentication failure.
    pub fn unsubscribe_open_interest_stock(&self, symbol: &str) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_open_interest_stock(symbol))
    }

    /// Unsubscribe from quote data for an option contract.
    /// # Errors
    ///
    /// Returns `Error::Config` on parse failure, or a network error on send.
    pub fn unsubscribe_quotes_option(
        &self,
        root: &str,
        exp: &str,
        strike: &str,
        right: &str,
    ) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_quotes_option(root, exp, strike, right))
    }

    /// Unsubscribe from trade data for an option contract.
    /// # Errors
    ///
    /// Returns `Error::Config` on parse failure, or a network error on send.
    pub fn unsubscribe_trades_option(
        &self,
        root: &str,
        exp: &str,
        strike: &str,
        right: &str,
    ) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_trades_option(root, exp, strike, right))
    }

    /// Unsubscribe from open interest data for an option contract.
    /// # Errors
    ///
    /// Returns `Error::Config` on parse failure, or a network error on send.
    pub fn unsubscribe_open_interest_option(
        &self,
        root: &str,
        exp: &str,
        strike: &str,
        right: &str,
    ) -> Result<(), Error> {
        self.with_streaming(|s| s.unsubscribe_open_interest_option(root, exp, strike, right))
    }

    /// Get the current contract ID to Contract mapping.
    ///
    /// Values are `Arc<Contract>` — the same refcounted contract every
    /// decoded FPSS event carries. Cloning the map clones Arcs, not
    /// underlying `Contract` values.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn contract_map(&self) -> Result<HashMap<i32, Arc<Contract>>, Error> {
        self.with_streaming(|s| Ok(s.contract_map()))
    }

    /// Look up a contract by its server-assigned ID.
    ///
    /// Returns `Arc<Contract>` so callers share the same heap allocation
    /// as the I/O thread cache and every decoded data event.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn contract_lookup(&self, id: i32) -> Result<Option<Arc<Contract>>, Error> {
        self.with_streaming(|s| Ok(s.contract_lookup(id)))
    }

    /// Get all active per-contract subscriptions.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn active_subscriptions(&self) -> Result<Vec<(SubscriptionKind, Contract)>, Error> {
        self.with_streaming(|s| Ok(s.active_subscriptions()))
    }

    /// Get all active full-type (firehose) subscriptions.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn active_full_subscriptions(&self) -> Result<Vec<(SubscriptionKind, SecType)>, Error> {
        self.with_streaming(|s| Ok(s.active_full_subscriptions()))
    }

    /// Shut down the streaming connection. Historical remains available.
    pub fn stop_streaming(&self) {
        let mut guard = self
            .streaming
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(client) = guard.take() {
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
    /// 4. Re-subscribe all saved subscriptions
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn reconnect_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        metrics::counter!("thetadatadx.fpss.reconnects").increment(1);
        // 1. Save active subscriptions before stopping
        let saved_subs = {
            let guard = self
                .streaming
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match guard.as_ref() {
                Some(client) => (
                    client.active_subscriptions(),
                    client.active_full_subscriptions(),
                ),
                None => (Vec::new(), Vec::new()),
            }
        };

        // 2. Stop streaming
        self.stop_streaming();

        // 3. Start a new streaming connection
        self.start_streaming(handler)?;

        // 4. Re-subscribe all saved subscriptions
        let (per_contract, full_type) = saved_subs;

        for (kind, contract) in &per_contract {
            let result = match kind {
                SubscriptionKind::Quote => self.subscribe_quotes(contract),
                SubscriptionKind::Trade => self.subscribe_trades(contract),
                SubscriptionKind::OpenInterest => self.subscribe_open_interest(contract),
            };
            if let Err(e) = result {
                tracing::warn!(
                    kind = ?kind,
                    contract = %contract,
                    error = %e,
                    "failed to re-subscribe after reconnect"
                );
            }
        }

        for (kind, sec_type) in &full_type {
            let result = match kind {
                SubscriptionKind::Trade => self.subscribe_full_trades(*sec_type),
                SubscriptionKind::OpenInterest => self.subscribe_full_open_interest(*sec_type),
                SubscriptionKind::Quote => {
                    tracing::warn!("full-type Quote subscription is not supported, skipping");
                    continue;
                }
            };
            if let Err(e) = result {
                tracing::warn!(
                    kind = ?kind,
                    sec_type = ?sec_type,
                    error = %e,
                    "failed to re-subscribe full-type after reconnect"
                );
            }
        }

        Ok(())
    }

    /// Get the current streaming connection status.
    pub fn connection_status(&self) -> ConnectionStatus {
        let guard = self
            .streaming
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match guard.as_ref() {
            None => {
                // Client is gone (never set, or taken/dropped by stop_streaming).
                if self.was_streaming.load(Ordering::Acquire) {
                    ConnectionStatus::Disconnected
                } else {
                    ConnectionStatus::NotStarted
                }
            }
            Some(client) => {
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
        let tier = |level: Option<i32>| match level {
            Some(0) => "Free".to_string(),
            Some(1) => "Value".to_string(),
            Some(2) => "Standard".to_string(),
            Some(3) => "Pro".to_string(),
            Some(n) => format!("Unknown({n})"),
            None => "Unknown".to_string(),
        };
        SubscriptionInfo {
            stock: tier(self.historical.stock_tier()),
            options: tier(self.historical.options_tier()),
        }
    }
}

impl Drop for ThetaDataDx {
    fn drop(&mut self) {
        self.stop_streaming();
    }
}

// All 61 historical methods available directly via Deref.
impl std::ops::Deref for ThetaDataDx {
    type Target = MddsClient;
    fn deref(&self) -> &MddsClient {
        &self.historical
    }
}
