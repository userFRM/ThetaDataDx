//! Fallback policy — routes the historical-quote endpoints over the
//! local Terminal's REST API instead of MDDS gRPC.
//!
//! Most callers leave the default [`FallbackPolicy::Disabled`] and
//! talk to MDDS over gRPC for every endpoint. [`FallbackPolicy::RestAlways`]
//! is the user-facing escape hatch for operators who explicitly want
//! to drive every historical-quote call through a locally-running
//! Terminal's REST surface (e.g. when the local Terminal exposes
//! column extensions the upstream gRPC service does not yet expose,
//! or when network policy disallows direct MDDS access from the
//! caller's environment). REST traffic still authenticates against
//! the Nexus session; only the wire transport changes.

/// Default base URL for [`FallbackPolicy::RestAlways`].
///
/// Re-export of [`crate::rest::client::DEFAULT_TERMINAL_BASE_URL`] so
/// both transports share a single source of truth — bumping the
/// Terminal default in one place propagates everywhere it's used.
pub const DEFAULT_REST_BASE_URL: &str = crate::rest::client::DEFAULT_TERMINAL_BASE_URL;

/// Policy controlling REST routing for the historical-quote endpoints.
///
/// `#[non_exhaustive]` so additional variants can land without a
/// breaking API change.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FallbackPolicy {
    /// REST routing is disabled — every request goes over gRPC.
    /// Default; matches the behaviour of every other endpoint.
    #[default]
    Disabled,

    /// Always route the historical-quote endpoints over REST regardless
    /// of the requested date range. Use when the caller wants a single
    /// transport for every quote-bearing call.
    RestAlways {
        /// Terminal REST base URL.
        base_url: String,
    },
}

impl FallbackPolicy {
    /// Returns the REST base URL the policy would target, or `None`
    /// for [`Self::Disabled`].
    #[must_use]
    pub fn base_url(&self) -> Option<&str> {
        match self {
            Self::Disabled => None,
            Self::RestAlways { base_url } => Some(base_url),
        }
    }

    /// Whether the policy unconditionally routes the historical-quote
    /// endpoints to REST.
    #[must_use]
    pub const fn routes_to_rest(&self) -> bool {
        matches!(self, Self::RestAlways { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_routes_nothing() {
        let p = FallbackPolicy::Disabled;
        assert!(!p.routes_to_rest());
        assert!(p.base_url().is_none());
    }

    #[test]
    fn rest_always_routes_every_call() {
        let p = FallbackPolicy::RestAlways {
            base_url: "http://127.0.0.1:25503".to_string(),
        };
        assert!(p.routes_to_rest());
        assert_eq!(p.base_url(), Some("http://127.0.0.1:25503"));
    }
}
