//! Fluent contract-first TypeScript surface.
//!
//! Mirrors the contract-first Rust API:
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

use napi::Either;
use thetadatadx::fpss::protocol::{self, FullSubscriptionKind, SecTypeExt as _, SubscriptionKind};

/// JS-visible `SecType` (frozen security-type enum). Construction
/// happens via the four named factories: `SecType.stock()`,
/// `SecType.option()`, `SecType.index()`, `SecType.rate()`. Returns
/// flow into `secType.fullTrades()` /
/// `secType.fullOpenInterest()` to build a full-stream
/// `Subscription`.
#[napi(js_name = "SecType")]
pub struct SecType {
    pub(crate) inner: thetadatadx::SecType,
}

impl SecType {
    pub(crate) fn from_inner(inner: thetadatadx::SecType) -> Self {
        Self { inner }
    }
}

#[napi]
impl SecType {
    /// `SecType.stock()` — equity-side full-stream constructor.
    #[napi(factory)]
    pub fn stock() -> Self {
        Self::from_inner(thetadatadx::SecType::Stock)
    }

    /// `SecType.option()` — option-side full-stream constructor.
    #[napi(factory)]
    pub fn option() -> Self {
        Self::from_inner(thetadatadx::SecType::Option)
    }

    /// `SecType.index()` — index-side full-stream constructor.
    #[napi(factory)]
    pub fn index() -> Self {
        Self::from_inner(thetadatadx::SecType::Index)
    }

    /// `SecType.rate()` — rate-side full-stream constructor.
    #[napi(factory)]
    pub fn rate() -> Self {
        Self::from_inner(thetadatadx::SecType::Rate)
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
        self.inner.as_str().to_string()
    }

    /// String rendering for `console.log` / template literals. Returns
    /// the symbolic name (`"OPTION"`), matching the Python `SecType`
    /// `__str__`. Without it a `SecType` instance prints as an opaque
    /// `SecType {}` because its getters do not surface on inspection.
    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.as_str().to_string()
    }
}

/// The expiration / strike / right of an option leg, passed to
/// `Contract.option(symbol, leg)` as a single object with named keys.
///
/// Naming the three values — all of which are strings — keeps the
/// contract identity non-transposable: `{ expiration, strike, right }`
/// cannot silently accept a swapped pair the way three adjacent
/// positional string arguments could.
#[napi(object)]
pub struct OptionLeg {
    /// Expiration date as `YYYYMMDD` (e.g. `"20260620"`).
    pub expiration: String,
    /// Strike price in dollars, as a number or string (`550`, `550.5`,
    /// `"550"` are equivalent).
    pub strike: Either<f64, String>,
    /// Option right: `"C"` / `"CALL"` / `"P"` / `"PUT"`
    /// (case-insensitive).
    pub right: String,
}

/// Fluent contract identifier — stock or option.
///
/// Use `Contract.stock("AAPL")` / `Contract.option(...)` to build one.
/// The class is also exported under the name `ContractRef`; `Contract`
/// is an alias for it, so the two names are interchangeable.
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

    /// Construct an option contract. The expiration / strike / right
    /// travel in a single `OptionLeg` object with named keys —
    /// `Contract.option("SPY", { expiration: "20260620", strike: "550",
    /// right: "C" })` — rather than as adjacent positional strings, so a
    /// swapped expiration/strike/right pair cannot pass silently. `right`
    /// accepts `"C"` / `"CALL"` / `"P"` / `"PUT"` (case-insensitive);
    /// `strike` is the price in dollars as a number or string (`550`,
    /// `550.5`, and `"550"` are equivalent).
    #[napi(factory)]
    pub fn option(symbol: String, leg: OptionLeg) -> napi::Result<Self> {
        let strike = match leg.strike {
            Either::A(dollars) => dollars.to_string(),
            Either::B(text) => text,
        };
        protocol::Contract::option(
            &symbol,
            protocol::OptionLeg {
                expiration: &leg.expiration,
                strike: &strike,
                right: &leg.right,
            },
        )
        .map(Self::from_inner)
        .map_err(crate::to_napi_err)
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

    /// Per-contract market-value subscription.
    #[napi(js_name = "marketValue")]
    pub fn market_value(&self) -> Subscription {
        Subscription {
            inner: Arc::new(Mutex::new(self.inner.market_value())),
        }
    }

    /// Underlying symbol (e.g. `"AAPL"`, `"SPY"`).
    #[napi(getter)]
    pub fn symbol(&self) -> String {
        self.inner.symbol.to_string()
    }

    /// Security type as an upper-case string (`"STOCK"`, `"OPTION"`,
    /// `"INDEX"`).
    #[napi(getter, js_name = "secType")]
    pub fn sec_type(&self) -> String {
        self.inner.sec_type.as_str().to_string()
    }

    /// Expiration date as a `YYYYMMDD` integer; `null` for non-options.
    #[napi(getter)]
    pub fn expiration(&self) -> Option<i32> {
        self.inner.expiration
    }

    /// Strike price in dollars; `null` for non-options. Reads back the
    /// same notation `Contract.option(.., strike, ..)` takes, and joins
    /// directly against historical-row `strike` columns.
    #[napi(getter)]
    pub fn strike(&self) -> Option<f64> {
        self.inner.strike_dollars()
    }

    /// Option right (`"C"` / `"P"`); `null` for non-options.
    #[napi(getter)]
    pub fn right(&self) -> Option<String> {
        self.inner
            .is_call
            .map(|c| if c { "C".to_string() } else { "P".to_string() })
    }

    /// String rendering for `console.log` / template literals, e.g.
    /// `"SPY OPTION 20260620 C 550"` or `"AAPL STOCK"`. The strike reads
    /// in dollars, matching the `strike` getter. Delegates to
    /// the same core rendering the Python `Contract` `__str__` uses, so
    /// the two bindings print a contract identically. Without it a
    /// `ContractRef` prints as an opaque `ContractRef {}` because its
    /// getters do not surface on inspection.
    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        format!("{}", self.inner)
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
    /// One of `"quote"`, `"trade"`, `"open_interest"`,
    /// `"market_value"`, `"full_trades"`, `"full_open_interest"` — the
    /// wire-level kind.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &*guard {
            protocol::Subscription::Contract { kind, .. } => match kind {
                SubscriptionKind::Quote => "quote",
                SubscriptionKind::Trade => "trade",
                SubscriptionKind::OpenInterest => "open_interest",
                SubscriptionKind::MarketValue => "market_value",
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

    /// The bound contract for per-contract subscriptions, `null` for
    /// full-stream subscriptions.
    #[napi(getter)]
    pub fn contract(&self) -> Option<ContractRef> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &*guard {
            protocol::Subscription::Contract { contract, .. } => {
                Some(ContractRef::from_inner(contract.clone()))
            }
            _ => None,
        }
    }

    /// The security type for full-stream subscriptions, `null` for
    /// per-contract subscriptions.
    #[napi(getter, js_name = "secType")]
    pub fn sec_type(&self) -> Option<SecType> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &*guard {
            protocol::Subscription::Full { sec_type, .. } => Some(SecType::from_inner(*sec_type)),
            _ => None,
        }
    }

    /// String rendering for `console.log` / template literals, e.g.
    /// `"Subscription(Trade, SPY OPTION 20260620 C 550)"` or
    /// `"Subscription(full Trades, OPTION)"`. Mirrors the Python
    /// `Subscription` `__repr__`. Without it a `Subscription` prints as
    /// an opaque `Subscription {}` because its getters do not surface on
    /// inspection.
    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &*guard {
            protocol::Subscription::Contract { contract, kind } => {
                format!("Subscription({kind:?}, {contract})")
            }
            protocol::Subscription::Full { sec_type, kind } => {
                format!("Subscription(full {kind:?}, {sec_type:?})")
            }
            _ => "Subscription(<unknown>)".to_string(),
        }
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
