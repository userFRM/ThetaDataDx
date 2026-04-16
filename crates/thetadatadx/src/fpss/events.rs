//! FPSS event types: data, control, and the I/O command channel.
//!
//! These are the wire-protocol-agnostic value types that flow from the I/O
//! thread into the Disruptor ring and out to user callbacks.

use std::sync::Arc;

use tdbe::types::enums::{RemoveReason, StreamMsgType, StreamResponseType};

use super::protocol::Contract;

/// Tick data events from the FPSS stream.
///
/// These are the hot-path events decoded from FIT wire format and
/// delta-decompressed. All price fields are decoded to `f64` at parse time.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FpssData {
    /// Decoded quote tick (code 21).
    Quote {
        contract_id: i32,
        /// Resolved symbol string (e.g. "AAPL"). Empty if contract not yet assigned.
        symbol: Arc<str>,
        ms_of_day: i32,
        bid_size: i32,
        bid_exchange: i32,
        bid: f64,
        bid_condition: i32,
        ask_size: i32,
        ask_exchange: i32,
        ask: f64,
        ask_condition: i32,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
    /// Decoded trade tick (code 22).
    Trade {
        contract_id: i32,
        /// Resolved symbol string (e.g. "AAPL"). Empty if contract not yet assigned.
        symbol: Arc<str>,
        ms_of_day: i32,
        sequence: i32,
        ext_condition1: i32,
        ext_condition2: i32,
        ext_condition3: i32,
        ext_condition4: i32,
        condition: i32,
        size: i32,
        exchange: i32,
        price: f64,
        condition_flags: i32,
        price_flags: i32,
        volume_type: i32,
        records_back: i32,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
    /// Decoded open interest tick (code 23).
    OpenInterest {
        contract_id: i32,
        /// Resolved symbol string (e.g. "AAPL"). Empty if contract not yet assigned.
        symbol: Arc<str>,
        ms_of_day: i32,
        open_interest: i32,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
    /// Decoded OHLCVC bar (code 24 or trade-derived).
    ///
    /// `volume` and `count` are `i64` to avoid overflow on high-volume symbols.
    Ohlcvc {
        contract_id: i32,
        /// Resolved symbol string (e.g. "AAPL"). Empty if contract not yet assigned.
        symbol: Arc<str>,
        ms_of_day: i32,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: i64,
        count: i64,
        date: i32,
        /// Wall-clock nanoseconds since UNIX epoch, captured at frame decode time.
        received_at_ns: u64,
    },
}

/// Control/lifecycle events from the FPSS stream.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum FpssControl {
    /// Login succeeded (METADATA code 3).
    ///
    /// `permissions` is the server's "Bundle" string, copied verbatim from the
    /// METADATA frame payload as UTF-8. **It is opaque diagnostic metadata, not
    /// a structured permission set.** The Java terminal (`FPSSClient.perms`,
    /// source of truth for the wire protocol) does not parse it: it logs the
    /// value as `[FPSS] CONNECTED: [host], Bundle: <perms>` and uses non-null
    /// as the `isVerified()` sentinel — that's it.
    ///
    /// **For feature gating, use [`crate::auth::AuthUser`] instead**.
    /// The Nexus REST endpoint exposes per-asset subscription tiers
    /// (`stock_subscription`, `options_subscription`, `indices_subscription`,
    /// `interest_rate_subscription`, each `0=FREE / 1=VALUE / 2=STANDARD /
    /// 3=PRO`), which is the canonical surface the Java terminal itself uses
    /// to compute concurrency limits and gate features.
    ///
    /// Treat this field as a log/diagnostic string only. Do not parse it.
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
    /// Auto-reconnect is about to attempt reconnection.
    ///
    /// Emitted before sleeping for the delay. `attempt` is 1-based.
    Reconnecting {
        reason: RemoveReason,
        attempt: u32,
        delay_ms: u64,
    },
    /// Auto-reconnect succeeded -- connection is live again.
    Reconnected,
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
    ///
    /// Filtered from user callbacks -- only visible to internal code.
    #[doc(hidden)]
    RawData { code: u8, payload: Vec<u8> },
    /// Placeholder default for ring buffer pre-allocation.
    ///
    /// Filtered from user callbacks -- only visible to internal code.
    #[doc(hidden)]
    #[default]
    Empty,
}

// ---------------------------------------------------------------------------
// Command channel -- FpssClient -> I/O thread
// ---------------------------------------------------------------------------

/// Commands sent from the `FpssClient` handle to the I/O thread.
pub(super) enum IoCommand {
    /// Write a raw frame (code + payload) to the TLS stream.
    WriteFrame {
        code: StreamMsgType,
        payload: Vec<u8>,
    },
    /// Graceful shutdown: send STOP, then exit the I/O loop.
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdbe::types::price::Price;

    #[test]
    fn fpss_event_default_exists() {
        let _evt: FpssEvent = Default::default();
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
