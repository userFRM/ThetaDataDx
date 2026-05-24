//! FPSS session lifecycle: the [`reconnect_delay`] classifier that maps
//! a `RemoveReason` to a retry policy.

use tdbe::types::enums::RemoveReason;

use super::protocol::{RECONNECT_DELAY_MS, TOO_MANY_REQUESTS_DELAY_MS};

/// Determine the reconnect delay based on the disconnect reason, using
/// the wire-constant defaults.
///
/// The classifier maps a `RemoveReason` to a retry delay (or `None` for
/// permanent errors). Callers that need to honour caller-tuned cadences
/// from [`crate::config::ReconnectConfig`] should use
/// [`reconnect_delay_for`] instead — this entry point is kept for
/// internal predicate use (the `.is_none()` permanent-reason check) and
/// for tests that pin the wire constants.
///
/// # Intentional divergence from upstream
///
/// Upstream only treats `AccountAlreadyConnected` (code 6) as a permanent
/// error, retrying forever on invalid credentials — which burns rate limits
/// and never succeeds. We treat all 7 credential/account error codes as
/// permanent because no amount of retrying will fix bad credentials. This
/// is a deliberate improvement over the upstream behavior.
#[must_use]
pub fn reconnect_delay(reason: RemoveReason) -> Option<u64> {
    reconnect_delay_for(reason, RECONNECT_DELAY_MS, TOO_MANY_REQUESTS_DELAY_MS)
}

/// Same classification as [`reconnect_delay`] but uses caller-supplied
/// `wait_ms` / `wait_rate_limited_ms` cadences from
/// [`crate::config::ReconnectConfig`] for the two transient classes.
/// Permanent reasons still short-circuit to `None`.
///
/// Wiring the config values through this entry point closes the
/// "defined-but-not-connected" gap on
/// [`crate::config::ReconnectConfig::wait_ms`] /
/// [`crate::config::ReconnectConfig::wait_rate_limited_ms`].
#[must_use]
pub fn reconnect_delay_for(
    reason: RemoveReason,
    wait_ms: u64,
    wait_rate_limited_ms: u64,
) -> Option<u64> {
    match reason {
        // Permanent errors -- no amount of reconnection will fix bad credentials.
        // Upstream only checks AccountAlreadyConnected here; we extend this to
        // all credential errors as a deliberate improvement on upstream's
        // burn-rate-limit-on-bad-creds behaviour.
        RemoveReason::AccountAlreadyConnected
        | RemoveReason::InvalidCredentials
        | RemoveReason::InvalidLoginValues
        | RemoveReason::InvalidLoginSize
        | RemoveReason::FreeAccount
        | RemoveReason::ServerUserDoesNotExist
        | RemoveReason::InvalidCredentialsNullUser => None,
        RemoveReason::TooManyRequests => Some(wait_rate_limited_ms),
        _ => Some(wait_ms),
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
    fn reconnect_delay_for_honours_caller_cadence() {
        // Caller-tuned wait values flow through verbatim for both
        // transient classes.
        assert_eq!(
            reconnect_delay_for(RemoveReason::ServerRestarting, 500, 60_000),
            Some(500)
        );
        assert_eq!(
            reconnect_delay_for(RemoveReason::TooManyRequests, 500, 60_000),
            Some(60_000)
        );
        // Permanent reasons stay permanent regardless of cadence.
        assert_eq!(
            reconnect_delay_for(RemoveReason::InvalidCredentials, 500, 60_000),
            None
        );
    }

    #[test]
    fn reconnect_policy_default_is_auto() {
        let policy: ReconnectPolicy = Default::default();
        assert!(matches!(policy, ReconnectPolicy::Auto(_)));
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
