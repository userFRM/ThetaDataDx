//! Subscription kind classification for FPSS subscribe / unsubscribe paths.
//!
//! Source: `PacketStream.addQuote()` uses code 21, `addTrade()` uses 22,
//! `addOpenInterest()` uses 23.
//!
//! # Public surface
//!
//! Two enums are exported here:
//!
//! - [`SubscriptionKind`] — wire-level discriminator (Quote / Trade /
//!   OpenInterest). Used internally by the io_loop, the `active_subs`
//!   tracker, and the public `active_subscriptions()` snapshot.
//! - [`Subscription`] — typed, fluent value returned by
//!   [`super::Contract::quote`] / [`super::Contract::trade`] /
//!   [`super::Contract::open_interest`] and by [`SecTypeExt::full_trades`]
//!   / [`SecTypeExt::full_open_interest`]. The polymorphic
//!   `client.subscribe(Subscription)` / `client.subscribe_many(...)` paths
//!   dispatch on this enum.
//!
//! See `report/ThetaDataDxClient_API_DX_Review_Rust_Python.md` for the
//! refactor target shape this module implements.

use tdbe::types::enums::{SecType, StreamMsgType};

use super::contract::Contract;

/// Returns the `StreamMsgType` code for subscribing to a given data type.
///
/// Source: `PacketStream.addQuote()` uses code 21, `addTrade()` uses 22,
/// `addOpenInterest()` uses 23.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionKind {
    Quote,
    Trade,
    OpenInterest,
}

impl SubscriptionKind {
    /// Message code for subscribing (Client->Server).
    #[must_use]
    pub fn subscribe_code(self) -> StreamMsgType {
        match self {
            Self::Quote => StreamMsgType::Quote,
            Self::Trade => StreamMsgType::Trade,
            Self::OpenInterest => StreamMsgType::OpenInterest,
        }
    }

    /// Message code for unsubscribing (Client->Server).
    #[must_use]
    pub fn unsubscribe_code(self) -> StreamMsgType {
        match self {
            Self::Quote => StreamMsgType::RemoveQuote,
            Self::Trade => StreamMsgType::RemoveTrade,
            Self::OpenInterest => StreamMsgType::RemoveOpenInterest,
        }
    }
}

// ---------------------------------------------------------------------------
// Fluent Subscription value
// ---------------------------------------------------------------------------

/// Tick kind for full-stream (security-type-scoped) subscriptions.
///
/// Full-stream subscriptions are strictly a strict subset of
/// [`SubscriptionKind`]: the FPSS server accepts full-stream Trade
/// and OpenInterest, but never full-stream Quote (the quote feed is
/// addressed per-contract only). Modeling that asymmetry as a
/// dedicated enum keeps "give me every option trade" and "give me
/// every quote on this contract" from looking interchangeable on the
/// public surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FullSubscriptionKind {
    /// Full-stream trade subscription.
    Trades,
    /// Full-stream open-interest subscription.
    OpenInterest,
}

/// Typed, fluent market-data subscription.
///
/// Returned by [`Contract::quote`] / [`Contract::trade`] /
/// [`Contract::open_interest`] (per-contract scope) and by
/// [`SecTypeExt::full_trades`] / [`SecTypeExt::full_open_interest`]
/// (full-stream scope). Pass to
/// [`crate::ThetaDataDxClient::subscribe`] /
/// [`crate::ThetaDataDxClient::subscribe_many`] to install on the live
/// streaming session.
///
/// ```rust,no_run
/// # use thetadatadx::fpss::protocol::{Contract, SecTypeExt};
/// # use tdbe::types::enums::SecType;
/// let stock_quote   = Contract::stock("AAPL").quote();
/// let opt_trade     = Contract::option("SPY", "20260620", "550", "C").unwrap().trade();
/// let full_opt_oi   = SecType::Option.full_open_interest();
/// let _all = vec![stock_quote, opt_trade, full_opt_oi];
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Subscription {
    /// Per-contract subscription scoped to one tick kind.
    Contract {
        /// Target contract.
        contract: Contract,
        /// Tick kind (Quote / Trade / OpenInterest).
        kind: SubscriptionKind,
    },
    /// Full-stream subscription scoped to a security type. Only Trade
    /// and OpenInterest are valid full-stream kinds (see
    /// [`FullSubscriptionKind`]).
    Full {
        /// Target security type.
        sec_type: SecType,
        /// Full-stream tick kind.
        kind: FullSubscriptionKind,
    },
}

impl Subscription {
    /// Construct a per-contract subscription.
    #[must_use]
    pub fn for_contract(contract: Contract, kind: SubscriptionKind) -> Self {
        Self::Contract { contract, kind }
    }

    /// Construct a full-stream subscription.
    #[must_use]
    pub fn full(sec_type: SecType, kind: FullSubscriptionKind) -> Self {
        Self::Full { sec_type, kind }
    }
}

/// Fluent constructors on [`Contract`] for the per-contract subscription
/// shapes accepted by [`crate::ThetaDataDxClient::subscribe`].
///
/// Singular by design — each method returns exactly one
/// [`Subscription`] value. Plural aliases (`quotes()`, `trades()`,
/// `open_interests()`) are intentionally omitted; bulk helpers live on
/// `Watchlist` / `OptionSeries` (Phase 7 of the report) and route
/// through [`crate::ThetaDataDxClient::subscribe_many`].
impl Contract {
    /// Per-contract Quote subscription.
    ///
    /// ```
    /// use thetadatadx::fpss::protocol::{Contract, Subscription, SubscriptionKind};
    ///
    /// let sub = Contract::stock("AAPL").quote();
    /// if let Subscription::Contract { contract, kind } = sub {
    ///     assert_eq!(contract.symbol, "AAPL");
    ///     assert_eq!(kind, SubscriptionKind::Quote);
    /// } else {
    ///     panic!("per-contract Quote subscription must round-trip as `Contract` variant");
    /// }
    /// ```
    #[must_use]
    pub fn quote(&self) -> Subscription {
        Subscription::Contract {
            contract: self.clone(),
            kind: SubscriptionKind::Quote,
        }
    }

    /// Per-contract Trade subscription.
    #[must_use]
    pub fn trade(&self) -> Subscription {
        Subscription::Contract {
            contract: self.clone(),
            kind: SubscriptionKind::Trade,
        }
    }

    /// Per-contract OpenInterest subscription.
    #[must_use]
    pub fn open_interest(&self) -> Subscription {
        Subscription::Contract {
            contract: self.clone(),
            kind: SubscriptionKind::OpenInterest,
        }
    }
}

/// Fluent constructors on [`SecType`] for full-stream subscriptions.
///
/// `SecType` lives in the `tdbe` crate, so the fluent methods are
/// provided as an extension trait imported here. Bring it into scope
/// via [`crate::prelude::*`] or
/// `use thetadatadx::fpss::protocol::SecTypeExt`.
pub trait SecTypeExt: Copy {
    /// Full-stream Trade subscription for this security type.
    fn full_trades(self) -> Subscription;
    /// Full-stream OpenInterest subscription for this security type.
    fn full_open_interest(self) -> Subscription;
}

impl SecTypeExt for SecType {
    fn full_trades(self) -> Subscription {
        Subscription::Full {
            sec_type: self,
            kind: FullSubscriptionKind::Trades,
        }
    }
    fn full_open_interest(self) -> Subscription {
        Subscription::Full {
            sec_type: self,
            kind: FullSubscriptionKind::OpenInterest,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_kind_codes() {
        assert_eq!(
            SubscriptionKind::Quote.subscribe_code(),
            StreamMsgType::Quote
        );
        assert_eq!(
            SubscriptionKind::Quote.unsubscribe_code(),
            StreamMsgType::RemoveQuote
        );
        assert_eq!(
            SubscriptionKind::Trade.subscribe_code(),
            StreamMsgType::Trade
        );
        assert_eq!(
            SubscriptionKind::Trade.unsubscribe_code(),
            StreamMsgType::RemoveTrade
        );
        assert_eq!(
            SubscriptionKind::OpenInterest.subscribe_code(),
            StreamMsgType::OpenInterest
        );
        assert_eq!(
            SubscriptionKind::OpenInterest.unsubscribe_code(),
            StreamMsgType::RemoveOpenInterest
        );
    }

    // ---- Fluent Subscription tests ----------------------------------

    #[test]
    fn contract_quote_returns_per_contract_subscription() {
        let c = Contract::stock("AAPL");
        let sub = c.quote();
        assert_eq!(
            sub,
            Subscription::Contract {
                contract: c,
                kind: SubscriptionKind::Quote,
            }
        );
    }

    #[test]
    fn contract_trade_returns_per_contract_subscription() {
        let c = Contract::option("SPY", "20260620", "550", "C").unwrap();
        let sub = c.trade();
        match sub {
            Subscription::Contract { contract, kind } => {
                assert_eq!(contract, c);
                assert_eq!(kind, SubscriptionKind::Trade);
            }
            other => panic!("expected Contract variant, got {other:?}"),
        }
    }

    #[test]
    fn contract_open_interest_returns_per_contract_subscription() {
        let c = Contract::option("SPY", "20260620", "550", "P").unwrap();
        let sub = c.open_interest();
        assert!(matches!(
            sub,
            Subscription::Contract {
                kind: SubscriptionKind::OpenInterest,
                ..
            }
        ));
    }

    #[test]
    fn sec_type_full_trades_returns_full_subscription() {
        let sub = SecType::Option.full_trades();
        assert_eq!(
            sub,
            Subscription::Full {
                sec_type: SecType::Option,
                kind: FullSubscriptionKind::Trades,
            }
        );
    }

    #[test]
    fn sec_type_full_open_interest_returns_full_subscription() {
        let sub = SecType::Stock.full_open_interest();
        assert_eq!(
            sub,
            Subscription::Full {
                sec_type: SecType::Stock,
                kind: FullSubscriptionKind::OpenInterest,
            }
        );
    }

    #[test]
    fn subscription_for_contract_constructor() {
        let c = Contract::stock("MSFT");
        let sub = Subscription::for_contract(c.clone(), SubscriptionKind::Trade);
        assert_eq!(
            sub,
            Subscription::Contract {
                contract: c,
                kind: SubscriptionKind::Trade,
            }
        );
    }

    #[test]
    fn subscription_full_constructor() {
        let sub = Subscription::full(SecType::Index, FullSubscriptionKind::Trades);
        assert!(matches!(
            sub,
            Subscription::Full {
                sec_type: SecType::Index,
                kind: FullSubscriptionKind::Trades,
            }
        ));
    }

    #[test]
    fn fluent_subscriptions_iterate_homogeneously() {
        // Assertion the report calls out: stock quotes, option trades,
        // and full-stream OI must all sit in one `Vec<Subscription>`
        // for `subscribe_many`.
        let opt = Contract::option("SPY", "20260620", "550", "C").unwrap();
        let subs: Vec<Subscription> = vec![
            Contract::stock("AAPL").quote(),
            opt.trade(),
            SecType::Option.full_open_interest(),
        ];
        assert_eq!(subs.len(), 3);
    }
}
