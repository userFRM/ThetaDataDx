//! Typed subscription-tier enum for the MDDS client.
//!
//! ThetaData's Nexus auth response carries a small integer per asset class
//! that encodes the customer's subscription level. The JVM terminal uses
//! it to size the concurrent-request semaphore as `2^tier`. We keep the
//! same wire shape (the integer comes straight off the JSON response) but
//! lift it into a typed enum the moment it crosses into Rust state, so
//! the rest of the SDK never compares against magic numbers.

/// Customer subscription tier, decoded from the Nexus auth `subscription`
/// integer.
///
/// Discriminants mirror the wire byte:
/// - `Free` = 0
/// - `Value` = 1
/// - `Standard` = 2
/// - `Pro` = 3
///
/// The `2^tier` concurrent-request bound used by the terminal's gRPC
/// connection manager is exposed via [`Self::max_concurrent_requests`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(i32)]
pub enum SubscriptionTier {
    /// Free tier — 1 concurrent request.
    Free = 0,
    /// Value tier — 2 concurrent requests.
    Value = 1,
    /// Standard tier — 4 concurrent requests.
    Standard = 2,
    /// Pro / Professional tier — 8 concurrent requests.
    Pro = 3,
}

impl SubscriptionTier {
    /// Concurrent in-flight gRPC requests permitted at this tier.
    ///
    /// Computed as `2^tier`.
    #[must_use]
    pub const fn max_concurrent_requests(self) -> usize {
        1usize << self as u32
    }

    /// Decode the wire byte returned by the Nexus auth API.
    ///
    /// Returns `None` for unknown values so callers can decide whether to
    /// fall back to a conservative default (typically `Free`) or surface a
    /// diagnostic. The SDK never silently coerces unknown tiers.
    #[must_use]
    pub const fn from_wire(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Free),
            1 => Some(Self::Value),
            2 => Some(Self::Standard),
            3 => Some(Self::Pro),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SubscriptionTier;

    #[test]
    fn from_wire_rejects_unknown() {
        assert_eq!(SubscriptionTier::from_wire(-1), None);
        assert_eq!(SubscriptionTier::from_wire(4), None);
        assert_eq!(SubscriptionTier::from_wire(99), None);
    }

    #[test]
    fn max_concurrent_requests_powers_of_two() {
        assert_eq!(SubscriptionTier::Free.max_concurrent_requests(), 1);
        assert_eq!(SubscriptionTier::Value.max_concurrent_requests(), 2);
        assert_eq!(SubscriptionTier::Standard.max_concurrent_requests(), 4);
        assert_eq!(SubscriptionTier::Pro.max_concurrent_requests(), 8);
    }

    #[test]
    fn from_wire_maps_known_discriminants() {
        assert_eq!(SubscriptionTier::from_wire(0), Some(SubscriptionTier::Free));
        assert_eq!(
            SubscriptionTier::from_wire(1),
            Some(SubscriptionTier::Value)
        );
        assert_eq!(
            SubscriptionTier::from_wire(2),
            Some(SubscriptionTier::Standard)
        );
        assert_eq!(SubscriptionTier::from_wire(3), Some(SubscriptionTier::Pro));
    }
}
