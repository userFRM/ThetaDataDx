//! Subscription kind classification for FPSS subscribe / unsubscribe paths.
//!
//! Source: `PacketStream.addQuote()` uses code 21, `addTrade()` uses 22,
//! `addOpenInterest()` uses 23.

use tdbe::types::enums::StreamMsgType;

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
}
