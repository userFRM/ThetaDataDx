//! FPSS session lifecycle: the [`reconnect_delay`] classifier that maps
//! a `RemoveReason` to a retry policy.

use tdbe::types::enums::RemoveReason;

use super::protocol::{RECONNECT_DELAY_MS, TOO_MANY_REQUESTS_DELAY_MS};

/// Determine the reconnect delay based on the disconnect reason.
///
/// Source: `FPSSClient.java` -- reconnect logic checks `RemoveReason` to decide delay.
///
/// # Intentional divergence from Java (see docs/java-parity-checklist.md)
///
/// Java only treats `AccountAlreadyConnected` (code 6) as a permanent error,
/// retrying forever on invalid credentials — which burns rate limits and never
/// succeeds. We treat all 7 credential/account error codes as permanent because
/// no amount of retrying will fix bad credentials. This is a deliberate
/// improvement over the Java behavior.
#[must_use]
pub fn reconnect_delay(reason: RemoveReason) -> Option<u64> {
    match reason {
        // Permanent errors -- no amount of reconnection will fix bad credentials.
        // Java only checks AccountAlreadyConnected here; we extend this to all
        // credential errors. See docs/java-parity-checklist.md (Reconnection).
        RemoveReason::AccountAlreadyConnected
        | RemoveReason::InvalidCredentials
        | RemoveReason::InvalidLoginValues
        | RemoveReason::InvalidLoginSize
        | RemoveReason::FreeAccount
        | RemoveReason::ServerUserDoesNotExist
        | RemoveReason::InvalidCredentialsNullUser => None,
        RemoveReason::TooManyRequests => Some(TOO_MANY_REQUESTS_DELAY_MS),
        _ => Some(RECONNECT_DELAY_MS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReconnectPolicy;
    use std::time::Duration;

    #[test]
    fn reconnect_delay_permanent() {
        // All credential / account errors are permanent -- no reconnect.
        assert_eq!(reconnect_delay(RemoveReason::AccountAlreadyConnected), None);
        assert_eq!(reconnect_delay(RemoveReason::InvalidCredentials), None);
        assert_eq!(reconnect_delay(RemoveReason::InvalidLoginValues), None);
        assert_eq!(reconnect_delay(RemoveReason::InvalidLoginSize), None);
        assert_eq!(reconnect_delay(RemoveReason::FreeAccount), None);
        assert_eq!(reconnect_delay(RemoveReason::ServerUserDoesNotExist), None);
        assert_eq!(
            reconnect_delay(RemoveReason::InvalidCredentialsNullUser),
            None
        );
    }

    #[test]
    fn reconnect_delay_too_many_requests() {
        assert_eq!(
            reconnect_delay(RemoveReason::TooManyRequests),
            Some(TOO_MANY_REQUESTS_DELAY_MS)
        );
    }

    #[test]
    fn reconnect_delay_normal() {
        assert_eq!(
            reconnect_delay(RemoveReason::ServerRestarting),
            Some(RECONNECT_DELAY_MS)
        );
        assert_eq!(
            reconnect_delay(RemoveReason::Unspecified),
            Some(RECONNECT_DELAY_MS)
        );
        assert_eq!(
            reconnect_delay(RemoveReason::TimedOut),
            Some(RECONNECT_DELAY_MS)
        );
    }

    #[test]
    fn reconnect_policy_default_is_auto() {
        let policy: ReconnectPolicy = Default::default();
        assert!(matches!(policy, ReconnectPolicy::Auto));
    }

    #[test]
    fn reconnect_policy_custom_works() {
        let policy = ReconnectPolicy::Custom(std::sync::Arc::new(|reason, attempt| {
            if attempt > 3 {
                return None;
            }
            match reason {
                RemoveReason::TooManyRequests => Some(Duration::from_secs(60)),
                _ => Some(Duration::from_secs(1)),
            }
        }));
        if let ReconnectPolicy::Custom(f) = &policy {
            assert_eq!(f(RemoveReason::TimedOut, 1), Some(Duration::from_secs(1)));
            assert_eq!(
                f(RemoveReason::TooManyRequests, 2),
                Some(Duration::from_secs(60))
            );
            assert_eq!(f(RemoveReason::TimedOut, 4), None);
        } else {
            panic!("expected Custom");
        }
    }
}
