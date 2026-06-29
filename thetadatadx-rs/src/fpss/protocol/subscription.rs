//! Subscription kind classification for FPSS subscribe / unsubscribe paths.
//!
//! Wire codes: quote subscriptions use code 21, trade 22, open interest 23.
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

use crate::tdbe::types::enums::{SecType, StreamMsgType};

use super::contract::Contract;

/// Returns the `StreamMsgType` code for subscribing to a given data type.
///
/// Wire codes: `Quote` = 21, `Trade` = 22, `OpenInterest` = 23,
/// `MarketValue` = 25.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubscriptionKind {
    /// Top-of-book quote (bid/ask) stream for the contract.
    Quote,
    /// Trade (last-sale) stream for the contract.
    Trade,
    /// Open-interest stream for the contract.
    OpenInterest,
    /// Market-value (mark/settlement) stream for the contract.
    MarketValue,
}

impl SubscriptionKind {
    /// Message code for subscribing (Client->Server).
    #[must_use]
    pub fn subscribe_code(self) -> StreamMsgType {
        match self {
            Self::Quote => StreamMsgType::Quote,
            Self::Trade => StreamMsgType::Trade,
            Self::OpenInterest => StreamMsgType::OpenInterest,
            Self::MarketValue => StreamMsgType::MarketValue,
        }
    }

    /// Message code for unsubscribing (Client->Server).
    #[must_use]
    pub fn unsubscribe_code(self) -> StreamMsgType {
        match self {
            Self::Quote => StreamMsgType::RemoveQuote,
            Self::Trade => StreamMsgType::RemoveTrade,
            Self::OpenInterest => StreamMsgType::RemoveOpenInterest,
            Self::MarketValue => StreamMsgType::RemoveMarketValue,
        }
    }

    /// Stable snake_case wire-kind label for a per-contract subscription,
    /// identical across every binding.
    ///
    /// This is the single source of the per-contract subscription-kind
    /// string the C ABI (`thetadatadx_unified_active_subscriptions`), the C++
    /// `FluentSubscription::kind_string`, and the Python / TypeScript
    /// `Subscription.kind` accessors all surface. Returning a fixed label
    /// here keeps the bindings from drifting onto the enum's `Debug`
    /// spelling, which is PascalCase and would diverge per language.
    #[must_use]
    pub fn kind_str(self) -> &'static str {
        match self {
            Self::Quote => "quote",
            Self::Trade => "trade",
            Self::OpenInterest => "open_interest",
            Self::MarketValue => "market_value",
        }
    }

    /// Stable snake_case label for this kind when carried as a
    /// *full-stream* subscription, or `None` if the kind has no
    /// full-stream subscription form.
    ///
    /// Full-stream snapshots
    /// ([`crate::StreamSurface::active_full_subscriptions`]) store the
    /// kind as a [`SubscriptionKind`], but the cross-binding label carries
    /// the `full_` prefix so a full-stream open-interest row never reads
    /// the same as a per-contract one. Only `Trade` and `OpenInterest` are
    /// requestable as full-stream subscriptions; `Quote` and `MarketValue`
    /// have no standalone full-stream form and return `None` (the binding
    /// drops the row), matching the Python / TypeScript projections. This
    /// is about subscription shape, not the data carried: a `full_trades`
    /// subscription still delivers quote and OHLC messages — the last
    /// BBO/NBBO and a bar are broadcast for each traded contract before its
    /// trade — surfaced as `Quote` and `Ohlcvc` events.
    #[must_use]
    pub fn full_kind_str(self) -> Option<&'static str> {
        match self {
            Self::Trade => Some("full_trades"),
            Self::OpenInterest => Some("full_open_interest"),
            Self::Quote | Self::MarketValue => None,
        }
    }
}

/// Identity of a subscribe frame awaiting its server `REQ_RESPONSE`.
///
/// A subscribe is tracked in `active_subs` / `active_full_subs` the instant
/// the frame is handed to the I/O thread, but the server answers
/// asynchronously and may reject it (`Error` / `MaxStreamsReached` /
/// `InvalidPerms`). The wire carries no contract or security type back in the
/// `REQ_RESPONSE` — only the `req_id` allocated at send time — so the sender
/// records that identity here, keyed by `req_id`, and the reader consults it
/// when the response lands to remove a rejected entry from the tracked set.
/// Without this correlation a rejected subscription would be re-replayed on
/// every reconnect and over-reported by `active_subscriptions()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingSub {
    /// A per-contract subscribe keyed by `(kind, contract)`.
    Contract(SubscriptionKind, Contract),
    /// A full-stream subscribe keyed by `(kind, sec_type)`.
    Full(SubscriptionKind, SecType),
}

// ---------------------------------------------------------------------------
// Fluent Subscription value
// ---------------------------------------------------------------------------

/// Tick kind for full-stream (security-type-scoped) subscriptions.
///
/// The *requestable* full-stream kinds are a subset of
/// [`SubscriptionKind`]: the server accepts a full-stream Trade or
/// OpenInterest subscription, but there is no standalone full-stream
/// Quote subscription — a quote-only feed is addressed per-contract.
/// This constrains what you can subscribe to, not what a full-stream
/// subscription delivers: a full-trade subscription carries quote, OHLC,
/// and trade messages for each traded contract (the last BBO/NBBO and a
/// bar precede the trade), surfaced as `Quote`, `Ohlcvc`, and `Trade`
/// events. Modeling the subscription asymmetry as a dedicated enum keeps
/// "give me every option trade" and "give me every quote on this
/// contract" from looking interchangeable on the public surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FullSubscriptionKind {
    /// Full-stream trade subscription.
    Trades,
    /// Full-stream open-interest subscription.
    OpenInterest,
}

impl FullSubscriptionKind {
    /// Stable snake_case wire-kind label, identical across every binding.
    ///
    /// Full-stream kinds carry the `full_` prefix
    /// (`full_trades` / `full_open_interest`) so a full-stream
    /// open-interest subscription never renders the same label as a
    /// per-contract one (both would be the bare enum `Debug` spelling
    /// `OpenInterest` otherwise). The C ABI
    /// (`thetadatadx_unified_active_full_subscriptions`), the C++
    /// `FluentSubscription::kind_string`, and the Python / TypeScript
    /// `Subscription.kind` accessors all read this label.
    #[must_use]
    pub fn kind_str(self) -> &'static str {
        match self {
            Self::Trades => "full_trades",
            Self::OpenInterest => "full_open_interest",
        }
    }
}

/// Typed, fluent market-data subscription.
///
/// Returned by [`Contract::quote`] / [`Contract::trade`] /
/// [`Contract::open_interest`] (per-contract scope) and by
/// [`SecTypeExt::full_trades`] / [`SecTypeExt::full_open_interest`]
/// (full-stream scope). Pass to
/// [`crate::StreamSurface::subscribe`] /
/// [`crate::StreamSurface::subscribe_many`] to install on the live
/// streaming session.
///
/// ```rust,no_run
/// # use thetadatadx::fpss::protocol::{Contract, OptionLeg, SecTypeExt};
/// # use thetadatadx::SecType;
/// let stock_quote   = Contract::stock("AAPL").quote();
/// let opt_trade     = Contract::option("SPY", OptionLeg { expiration: "20260620", strike: "550", right: "C" }).unwrap().trade();
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

/// Fluent constructors on [`Contract`] for the per-contract subscription
/// shapes accepted by [`crate::StreamSurface::subscribe`].
///
/// Singular by design — each method returns exactly one
/// [`Subscription`] value. Plural aliases (`quotes()`, `trades()`,
/// `open_interests()`) are intentionally omitted; bulk helpers live on
/// `Watchlist` / `OptionSeries` and route through
/// [`crate::StreamSurface::subscribe_many`].
impl Contract {
    /// Per-contract Quote subscription.
    ///
    /// ```
    /// use thetadatadx::fpss::protocol::{Contract, Subscription, SubscriptionKind};
    ///
    /// let sub = Contract::stock("AAPL").quote();
    /// if let Subscription::Contract { contract, kind } = sub {
    ///     assert_eq!(&*contract.symbol, "AAPL");
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

    /// Per-contract market-value subscription, matching the JVM
    /// terminal's per-contract market value stream.
    ///
    /// The market value is a calculated theoretical price derived from
    /// the real-time bid/ask (see [`crate::fpss::StreamData::MarketValue`]).
    /// It is offered per-contract only — there is no full-stream
    /// market-value broadcast, so this lives on [`Contract`] and not on
    /// [`SecTypeExt`].
    #[must_use]
    pub fn market_value(&self) -> Subscription {
        Subscription::Contract {
            contract: self.clone(),
            kind: SubscriptionKind::MarketValue,
        }
    }
}

/// Fluent constructors on [`SecType`] for full-stream subscriptions.
///
/// [`SecType`] lives in the data-format layer, so the fluent methods are
/// provided as an extension trait imported here. Bring it into scope
/// via the [`crate::prelude`] glob or
/// `use thetadatadx::fpss::protocol::SecTypeExt`.
pub trait SecTypeExt: Copy {
    /// Full-stream Trade subscription for this security type.
    ///
    /// Constructing the value is infallible, but only [`SecType::Stock`] and
    /// [`SecType::Option`] have an upstream full-stream broadcast: passing a
    /// subscription for any other security type to
    /// [`crate::StreamSurface::subscribe`] returns an [`Error::Config`]
    /// at subscribe time. Subscribe to indices and rates per-contract
    /// instead (for example `Contract::index("VIX").trade()`).
    ///
    /// [`Error::Config`]: crate::error::Error::Config
    fn full_trades(self) -> Subscription;
    /// Full-stream OpenInterest subscription for this security type.
    ///
    /// Constructing the value is infallible, but only [`SecType::Stock`] and
    /// [`SecType::Option`] have an upstream full-stream broadcast: passing a
    /// subscription for any other security type to
    /// [`crate::StreamSurface::subscribe`] returns an [`Error::Config`]
    /// at subscribe time. Subscribe to indices and rates per-contract
    /// instead (for example `Contract::index("VIX").open_interest()`).
    ///
    /// [`Error::Config`]: crate::error::Error::Config
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
    use super::super::contract::OptionLeg;
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
    fn subscription_kind_str_is_snake_case() {
        assert_eq!(SubscriptionKind::Quote.kind_str(), "quote");
        assert_eq!(SubscriptionKind::Trade.kind_str(), "trade");
        assert_eq!(SubscriptionKind::OpenInterest.kind_str(), "open_interest");
        assert_eq!(SubscriptionKind::MarketValue.kind_str(), "market_value");
    }

    #[test]
    fn subscription_kind_full_str_prefixes_and_filters() {
        assert_eq!(SubscriptionKind::Trade.full_kind_str(), Some("full_trades"));
        assert_eq!(
            SubscriptionKind::OpenInterest.full_kind_str(),
            Some("full_open_interest")
        );
        // Quote / MarketValue have no full-stream form on the wire.
        assert_eq!(SubscriptionKind::Quote.full_kind_str(), None);
        assert_eq!(SubscriptionKind::MarketValue.full_kind_str(), None);
    }

    #[test]
    fn full_subscription_kind_str_is_prefixed() {
        assert_eq!(FullSubscriptionKind::Trades.kind_str(), "full_trades");
        assert_eq!(
            FullSubscriptionKind::OpenInterest.kind_str(),
            "full_open_interest"
        );
    }

    #[test]
    fn full_open_interest_never_collides_with_per_contract() {
        // The collision the snake_case labels exist to prevent: a
        // per-contract OI and a full-stream OI must read differently.
        assert_ne!(
            SubscriptionKind::OpenInterest.kind_str(),
            FullSubscriptionKind::OpenInterest.kind_str()
        );
    }

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
        let c = Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20260620",
                strike: "550",
                right: "C",
            },
        )
        .unwrap();
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
        let c = Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20260620",
                strike: "550",
                right: "P",
            },
        )
        .unwrap();
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
    fn fluent_subscriptions_iterate_homogeneously() {
        // Assertion the report calls out: stock quotes, option trades,
        // and full-stream OI must all sit in one `Vec<Subscription>`
        // for `subscribe_many`.
        let opt = Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20260620",
                strike: "550",
                right: "C",
            },
        )
        .unwrap();
        let subs: Vec<Subscription> = vec![
            Contract::stock("AAPL").quote(),
            opt.trade(),
            SecType::Option.full_open_interest(),
        ];
        assert_eq!(subs.len(), 3);
    }
}
