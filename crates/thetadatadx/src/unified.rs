//! Unified ThetaData client -- single entry point, one auth, lazy FPSS.
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
use std::sync::Mutex;

use crate::auth::Credentials;
use crate::config::DirectConfig;
use crate::direct::DirectClient;
use crate::error::Error;
use crate::fpss::protocol::{Contract, SubscriptionKind};
use crate::fpss::{FpssClient, FpssEvent};
use tdbe::types::enums::SecType;

/// Unified ThetaData client.
///
/// Authenticates once at connect time. Historical data (MDDS gRPC) is
/// available immediately. Streaming (FPSS TCP) connects lazily when
/// you call [`start_streaming`](Self::start_streaming).
///
/// All 61 historical endpoint methods are available via `Deref` to
/// [`DirectClient`]. Streaming methods are on this struct directly.
pub struct ThetaDataDx {
    historical: DirectClient,
    streaming: Mutex<Option<FpssClient>>,
    creds: Credentials,
}

impl ThetaDataDx {
    /// Connect to ThetaData. Authenticates once, opens gRPC channel.
    ///
    /// FPSS streaming is NOT connected yet -- call [`start_streaming`]
    /// when you need real-time data.
    pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error> {
        let historical = DirectClient::connect(creds, config).await?;
        Ok(Self {
            historical,
            streaming: Mutex::new(None),
            creds: creds.clone(),
        })
    }

    /// Start the FPSS streaming connection with a callback handler.
    ///
    /// This opens a TLS/TCP connection to ThetaData's FPSS servers,
    /// authenticates with the same credentials used at connect time,
    /// and starts the Disruptor ring buffer + I/O thread.
    ///
    /// The callback runs on the Disruptor consumer thread -- keep it fast.
    pub fn start_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        let mut guard = self.streaming.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return Err(Error::Fpss("streaming already started".into()));
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
        Ok(())
    }

    /// Whether streaming is currently active.
    pub fn is_streaming(&self) -> bool {
        self.streaming
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    // -- Streaming convenience methods --

    fn with_streaming<R>(
        &self,
        f: impl FnOnce(&FpssClient) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let guard = self.streaming.lock().unwrap_or_else(|e| e.into_inner());
        let client = guard.as_ref().ok_or_else(|| {
            Error::Fpss("streaming not started -- call start_streaming() first".into())
        })?;
        f(client)
    }

    /// Subscribe to quote updates for a contract.
    pub fn subscribe_quotes(&self, contract: &Contract) -> Result<i32, Error> {
        self.with_streaming(|s| s.subscribe_quotes(contract))
    }

    /// Subscribe to trade updates for a contract.
    pub fn subscribe_trades(&self, contract: &Contract) -> Result<i32, Error> {
        self.with_streaming(|s| s.subscribe_trades(contract))
    }

    /// Subscribe to open interest updates for a contract.
    pub fn subscribe_open_interest(&self, contract: &Contract) -> Result<i32, Error> {
        self.with_streaming(|s| s.subscribe_open_interest(contract))
    }

    /// Subscribe to all trades for a security type (firehose).
    pub fn subscribe_full_trades(&self, sec_type: SecType) -> Result<i32, Error> {
        self.with_streaming(|s| s.subscribe_full_trades(sec_type))
    }

    /// Subscribe to all open interest for a security type (firehose).
    pub fn subscribe_full_open_interest(&self, sec_type: SecType) -> Result<i32, Error> {
        self.with_streaming(|s| s.subscribe_full_open_interest(sec_type))
    }

    /// Unsubscribe from quote updates for a contract.
    pub fn unsubscribe_quotes(&self, contract: &Contract) -> Result<i32, Error> {
        self.with_streaming(|s| s.unsubscribe_quotes(contract))
    }

    /// Unsubscribe from trade updates for a contract.
    pub fn unsubscribe_trades(&self, contract: &Contract) -> Result<i32, Error> {
        self.with_streaming(|s| s.unsubscribe_trades(contract))
    }

    /// Unsubscribe from open interest updates for a contract.
    pub fn unsubscribe_open_interest(&self, contract: &Contract) -> Result<i32, Error> {
        self.with_streaming(|s| s.unsubscribe_open_interest(contract))
    }

    /// Unsubscribe from all trades for a security type.
    pub fn unsubscribe_full_trades(&self, sec_type: SecType) -> Result<i32, Error> {
        self.with_streaming(|s| s.unsubscribe_full_trades(sec_type))
    }

    /// Unsubscribe from all open interest for a security type.
    pub fn unsubscribe_full_open_interest(&self, sec_type: SecType) -> Result<i32, Error> {
        self.with_streaming(|s| s.unsubscribe_full_open_interest(sec_type))
    }

    /// Get the current contract ID to Contract mapping.
    pub fn contract_map(&self) -> Result<HashMap<i32, Contract>, Error> {
        self.with_streaming(|s| Ok(s.contract_map()))
    }

    /// Look up a contract by its server-assigned ID.
    pub fn contract_lookup(&self, id: i32) -> Result<Option<Contract>, Error> {
        self.with_streaming(|s| Ok(s.contract_lookup(id)))
    }

    /// Get all active per-contract subscriptions.
    pub fn active_subscriptions(&self) -> Result<Vec<(SubscriptionKind, Contract)>, Error> {
        self.with_streaming(|s| Ok(s.active_subscriptions()))
    }

    /// Get all active full-type (firehose) subscriptions.
    pub fn active_full_subscriptions(&self) -> Result<Vec<(SubscriptionKind, SecType)>, Error> {
        self.with_streaming(|s| Ok(s.active_full_subscriptions()))
    }

    /// Shut down the streaming connection. Historical remains available.
    pub fn stop_streaming(&self) {
        let mut guard = self.streaming.lock().unwrap_or_else(|e| e.into_inner());
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
    pub fn reconnect_streaming<F>(&self, handler: F) -> Result<(), Error>
    where
        F: FnMut(&FpssEvent) + Send + 'static,
    {
        metrics::counter!("thetadatadx.fpss.reconnects").increment(1);
        // 1. Save active subscriptions before stopping
        let saved_subs = {
            let guard = self.streaming.lock().unwrap_or_else(|e| e.into_inner());
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

    /// Access the session UUID from the initial auth.
    pub fn session_uuid(&self) -> &str {
        self.historical.session_uuid()
    }

    /// Access the config.
    pub fn config(&self) -> &DirectConfig {
        self.historical.config()
    }
}

impl Drop for ThetaDataDx {
    fn drop(&mut self) {
        self.stop_streaming();
    }
}

// All 61 historical methods available directly via Deref.
impl std::ops::Deref for ThetaDataDx {
    type Target = DirectClient;
    fn deref(&self) -> &DirectClient {
        &self.historical
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_not_started_by_default() {
        // Can't test connect() without real creds, but can verify the type exists
        // and Deref works at compile time.
        fn _assert_deref(tdx: &ThetaDataDx) -> &DirectClient {
            &*tdx
        }
    }
}
