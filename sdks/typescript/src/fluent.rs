//! Fluent contract-first TypeScript surface.
//!
//! Mirrors the Rust API laid out in
//! `report/ThetaDataDxClient_API_DX_Review_Rust_Python.md`:
//!
//! ```ts
//! import { Contract, SecType } from "thetadatadx";
//!
//! const stock  = Contract.stock("AAPL");
//! const option = Contract.option("SPY", "20260620", "550", "C");
//!
//! client.subscribe(stock.quote());
//! client.subscribe(option.trade());
//! client.subscribe(SecType.option().fullTrades());
//! client.subscribeMany([stock.quote(), option.quote()]);
//! ```

use std::sync::{Arc, Mutex};

use thetadatadx::fpss::protocol::{self, FullSubscriptionKind, SecTypeExt as _, SubscriptionKind};

/// JS-visible `SecType` (frozen security-type enum). Construction
/// happens via the four named factories: `SecType.stock()`,
/// `SecType.option()`, `SecType.index()`, `SecType.rate()`. Returns
/// flow into `secType.fullTrades()` /
/// `secType.fullOpenInterest()` to build a full-stream
/// `Subscription`.
#[napi(js_name = "SecType")]
pub struct SecType {
    pub(crate) inner: tdbe::types::enums::SecType,
}

impl SecType {
    pub(crate) fn from_inner(inner: tdbe::types::enums::SecType) -> Self {
        Self { inner }
    }
}

#[napi]
impl SecType {
    /// `SecType.stock()` — equity-side full-stream constructor.
    #[napi(factory)]
    pub fn stock() -> Self {
        Self::from_inner(tdbe::types::enums::SecType::Stock)
    }

    /// `SecType.option()` — option-side full-stream constructor.
    #[napi(factory)]
    pub fn option() -> Self {
        Self::from_inner(tdbe::types::enums::SecType::Option)
    }

    /// `SecType.index()` — index-side full-stream constructor.
    #[napi(factory)]
    pub fn index() -> Self {
        Self::from_inner(tdbe::types::enums::SecType::Index)
    }

    /// `SecType.rate()` — rate-side full-stream constructor.
    #[napi(factory)]
    pub fn rate() -> Self {
        Self::from_inner(tdbe::types::enums::SecType::Rate)
    }

    /// Full-stream Trade subscription for this security type.
    #[napi(js_name = "fullTrades")]
    pub fn full_trades(&self) -> Subscription {
        Subscription {
            inner: Arc::new(Mutex::new(self.inner.full_trades())),
        }
    }

    /// Full-stream OpenInterest subscription for this security type.
    #[napi(js_name = "fullOpenInterest")]
    pub fn full_open_interest(&self) -> Subscription {
        Subscription {
            inner: Arc::new(Mutex::new(self.inner.full_open_interest())),
        }
    }

    /// Symbolic name (`"STOCK"`, `"OPTION"`, `"INDEX"`, `"RATE"`).
    #[napi(getter)]
    pub fn name(&self) -> String {
        format!("{:?}", self.inner).to_uppercase()
    }
}

/// Fluent contract identifier — stock or option.
///
/// Exposed to JS as `ContractRef` (the bare `Contract` name on the JS
/// side already refers to the FPSS event payload data object — see
/// `fpss_event_classes.rs`). The `index.d.ts` post-processor emits an
/// `export const Contract = ContractRef` alias so users still write
/// `Contract.stock("AAPL")` per the documented surface.
#[napi(js_name = "ContractRef")]
pub struct ContractRef {
    pub(crate) inner: protocol::Contract,
}

impl ContractRef {
    pub(crate) fn from_inner(inner: protocol::Contract) -> Self {
        Self { inner }
    }
}

#[napi]
impl ContractRef {
    /// Construct a stock contract.
    #[napi(factory)]
    pub fn stock(symbol: String) -> Self {
        Self::from_inner(protocol::Contract::stock(&symbol))
    }

    /// Construct an index contract.
    #[napi(factory)]
    pub fn index(symbol: String) -> Self {
        Self::from_inner(protocol::Contract::index(&symbol))
    }

    /// Construct an option contract. `right` accepts `"C"` / `"CALL"`
    /// / `"P"` / `"PUT"` (case-insensitive).
    #[napi(factory)]
    pub fn option(
        symbol: String,
        expiration: String,
        strike: String,
        right: String,
    ) -> napi::Result<Self> {
        protocol::Contract::option(&symbol, &expiration, &strike, &right)
            .map(Self::from_inner)
            .map_err(|e| napi::Error::from_reason(e.to_string()))
    }

    /// Per-contract Quote subscription.
    #[napi]
    pub fn quote(&self) -> Subscription {
        Subscription {
            inner: Arc::new(Mutex::new(self.inner.quote())),
        }
    }

    /// Per-contract Trade subscription.
    #[napi]
    pub fn trade(&self) -> Subscription {
        Subscription {
            inner: Arc::new(Mutex::new(self.inner.trade())),
        }
    }

    /// Per-contract OpenInterest subscription.
    #[napi(js_name = "openInterest")]
    pub fn open_interest(&self) -> Subscription {
        Subscription {
            inner: Arc::new(Mutex::new(self.inner.open_interest())),
        }
    }

    #[napi(getter)]
    pub fn symbol(&self) -> String {
        self.inner.symbol.to_string()
    }

    #[napi(getter, js_name = "secType")]
    pub fn sec_type(&self) -> String {
        format!("{:?}", self.inner.sec_type).to_uppercase()
    }

    #[napi(getter)]
    pub fn expiration(&self) -> Option<i32> {
        self.inner.expiration
    }

    #[napi(getter)]
    pub fn strike(&self) -> Option<i32> {
        self.inner.strike
    }

    #[napi(getter)]
    pub fn right(&self) -> Option<String> {
        self.inner
            .is_call
            .map(|c| if c { "C".to_string() } else { "P".to_string() })
    }
}

/// Typed market-data subscription.
///
/// Returned by `Contract.quote()` / `.trade()` / `.openInterest()`
/// (per-contract scope) and by `SecType.option().fullTrades()` /
/// `.fullOpenInterest()` (full-stream scope). Pass to
/// `client.subscribe(sub)` or `client.subscribeMany([...])`.
#[napi(js_name = "Subscription")]
pub struct Subscription {
    /// `Arc<Mutex<...>>` so the napi-rs `Reference` machinery can hand
    /// `&self` views to JS (which marshals through `getter`/method
    /// calls) while the inner enum is also consumable from Rust into
    /// `subscribe(...)` paths.
    pub(crate) inner: Arc<Mutex<protocol::Subscription>>,
}

#[napi]
impl Subscription {
    /// One of `"quote"`, `"trade"`, `"open_interest"`, `"full_trades"`,
    /// `"full_open_interest"` — the wire-level kind.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &*guard {
            protocol::Subscription::Contract { kind, .. } => match kind {
                SubscriptionKind::Quote => "quote",
                SubscriptionKind::Trade => "trade",
                SubscriptionKind::OpenInterest => "open_interest",
                _ => "unknown",
            },
            protocol::Subscription::Full { kind, .. } => match kind {
                FullSubscriptionKind::Trades => "full_trades",
                FullSubscriptionKind::OpenInterest => "full_open_interest",
                _ => "unknown",
            },
            _ => "unknown",
        }
        .to_string()
    }

    /// `true` for full-stream (security-type-scoped) subscriptions.
    #[napi(getter, js_name = "isFull")]
    pub fn is_full(&self) -> bool {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        matches!(&*guard, protocol::Subscription::Full { .. })
    }
}

impl Subscription {
    /// Take the inner subscription out, leaving an unreachable
    /// placeholder behind. Used by `client.subscribe(sub)` because the
    /// Rust core takes the `Subscription` value by move; cloning the
    /// inner enum keeps the JS handle reusable.
    pub(crate) fn snapshot(&self) -> protocol::Subscription {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
