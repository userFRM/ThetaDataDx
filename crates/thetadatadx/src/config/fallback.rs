//! Fallback policy -- routes h2-cascading endpoints over the local
//! Terminal's REST API (issue #571).
//!
//! Issue #571 documents an upstream Terminal bug where 2022-era
//! options' pre-extension 6-field NBBO storage rows trip a
//! length-rigid `IllegalArgumentException` in the Java `QuoteTick` /
//! `TradeQuoteTick` constructors, cascading the entire h2 stream.
//! [`FallbackPolicy`] lets a caller route the four affected endpoints
//! (`option_history_quote`, `option_history_trade_quote`,
//! `option_history_greeks_implied_volatility`,
//! `option_history_greeks_first_order`) over the REST transport
//! ([`crate::rest`]) instead of cancelling the call when the gRPC
//! cascade strikes.

/// Default base URL for [`FallbackPolicy::RestAlways`] and friends.
///
/// Re-export of [`crate::rest::client::DEFAULT_TERMINAL_BASE_URL`] so
/// both transports share a single source of truth — bumping the
/// Terminal default in one place propagates everywhere it's used.
pub const DEFAULT_REST_BASE_URL: &str = crate::rest::client::DEFAULT_TERMINAL_BASE_URL;

/// Policy controlling REST fallback for h2-cascading endpoints.
///
/// `#[non_exhaustive]` so additional variants (`RestOnAnyTransport`,
/// `RestForSymbols`, ...) can land without a breaking API change.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FallbackPolicy {
    /// REST fallback is disabled -- every request goes over gRPC
    /// regardless of failure mode. Default for back-compat.
    #[default]
    Disabled,

    /// Fall back to REST only when gRPC returns the
    /// [`crate::error::TransportErrorKind::ConnectionClosed`] signature
    /// associated with the issue #571 h2 cascade. Cheaper for current
    /// (post-2023) workloads where the gRPC path is the fast common
    /// case, but pays one failed round trip per affected request.
    RestOnH2Disconnect {
        /// Terminal REST base URL.
        base_url: String,
    },

    /// Route every request whose `start_date` falls before `before` --
    /// expressed as `YYYYMMDD` integer -- directly to REST without
    /// attempting gRPC first. Use when the caller knows the symbol /
    /// date range is squarely inside the 2022-era legacy-row window
    /// (saves the failed round-trip cost on every call). Requests on
    /// or after `before` flow through gRPC as normal.
    ///
    /// `before` is `YYYYMMDD` so the user can keep the integer-date
    /// convention used elsewhere in the SDK rather than reaching for
    /// `chrono::NaiveDate`.
    RestAlwaysForDateRange {
        /// Terminal REST base URL.
        base_url: String,
        /// `YYYYMMDD` cutoff. Requests with `start_date < before`
        /// route to REST; requests on or after `before` flow through
        /// gRPC. The boundary value follows half-open interval
        /// semantics ("up to but not including `before`").
        before: i32,
    },

    /// Always route the affected endpoints over REST regardless of
    /// the requested date range. Use when the caller wants a single
    /// transport for every quote-bearing call (e.g. running against a
    /// patched Terminal that ALSO has REST-only column extensions the
    /// gRPC path does not yet expose).
    RestAlways {
        /// Terminal REST base URL.
        base_url: String,
    },
}

impl FallbackPolicy {
    /// Returns the REST base URL the policy would target on a
    /// fallback, or `None` for [`Self::Disabled`].
    #[must_use]
    pub fn base_url(&self) -> Option<&str> {
        match self {
            Self::Disabled => None,
            Self::RestOnH2Disconnect { base_url }
            | Self::RestAlwaysForDateRange { base_url, .. }
            | Self::RestAlways { base_url } => Some(base_url),
        }
    }

    /// Whether the policy would pre-route a request with `start_date`
    /// to REST without trying gRPC first. Drives the "save the failed
    /// round trip" optimization on [`Self::RestAlwaysForDateRange`]
    /// and [`Self::RestAlways`].
    ///
    /// `start_date` is `YYYYMMDD` (e.g. `20240605`); the same integer
    /// shape every endpoint method takes.
    #[must_use]
    pub fn pre_routes_to_rest(&self, start_date: i32) -> bool {
        match self {
            Self::Disabled | Self::RestOnH2Disconnect { .. } => false,
            Self::RestAlwaysForDateRange { before, .. } => start_date < *before,
            Self::RestAlways { .. } => true,
        }
    }

    /// Whether the policy would fall back to REST AFTER observing the
    /// gRPC error described by issue #571 (h2 connection closed
    /// mid-stream). Always true for non-Disabled policies; the
    /// per-variant nuance is on `pre_routes_to_rest`.
    #[must_use]
    pub fn falls_back_on_h2_disconnect(&self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_routes_nothing() {
        let p = FallbackPolicy::Disabled;
        assert!(!p.pre_routes_to_rest(20_220_414));
        assert!(!p.pre_routes_to_rest(20_240_605));
        assert!(!p.falls_back_on_h2_disconnect());
        assert!(p.base_url().is_none());
    }

    #[test]
    fn rest_on_h2_disconnect_does_not_pre_route() {
        let p = FallbackPolicy::RestOnH2Disconnect {
            base_url: "http://127.0.0.1:25503".to_string(),
        };
        assert!(!p.pre_routes_to_rest(20_220_414));
        assert!(p.falls_back_on_h2_disconnect());
        assert_eq!(p.base_url(), Some("http://127.0.0.1:25503"));
    }

    #[test]
    fn rest_always_for_date_range_pre_routes_pre_2023_only() {
        let p = FallbackPolicy::RestAlwaysForDateRange {
            base_url: "http://127.0.0.1:25503".to_string(),
            before: 20_230_101,
        };
        // 2022 -- pre-route to REST.
        assert!(p.pre_routes_to_rest(20_220_414));
        // 2023-01-01 itself -- on the boundary, gRPC (half-open).
        assert!(!p.pre_routes_to_rest(20_230_101));
        // 2024 -- gRPC as normal.
        assert!(!p.pre_routes_to_rest(20_240_605));
        assert!(p.falls_back_on_h2_disconnect());
    }

    #[test]
    fn rest_always_pre_routes_every_call() {
        let p = FallbackPolicy::RestAlways {
            base_url: "http://127.0.0.1:25503".to_string(),
        };
        assert!(p.pre_routes_to_rest(20_220_414));
        assert!(p.pre_routes_to_rest(20_240_605));
        assert!(p.pre_routes_to_rest(20_300_101));
        assert!(p.falls_back_on_h2_disconnect());
    }
}
