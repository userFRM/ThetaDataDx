//! FPSS session lifecycle: manual [`reconnect`] and the
//! [`reconnect_delay`] classifier that maps a `RemoveReason` to a
//! retry policy.

use std::thread;
use std::time::Duration;

use tdbe::types::enums::RemoveReason;

use crate::auth::Credentials;
use crate::config::{FpssFlushMode, ReconnectPolicy};
use crate::error::Error;

use super::events::{FpssEvent, IoCommand};
use super::protocol::{
    self, build_subscribe_payload, Contract, SubscriptionKind, RECONNECT_DELAY_MS,
    TOO_MANY_REQUESTS_DELAY_MS,
};
use super::FpssClient;

/// Reconnect an FPSS client after a disconnect.
///
/// # Behavior (from `FPSSClient.java`)
///
/// 1. Wait `delay_ms` before attempting reconnection
/// 2. Establish a new TLS connection
/// 3. Re-authenticate
/// 4. Re-subscribe all previously active subscriptions with `req_id = -1`
///
/// On `TOO_MANY_REQUESTS`: wait 130 seconds before reconnecting.
/// On `ACCOUNT_ALREADY_CONNECTED`: do NOT reconnect (permanent error).
///
/// Source: `FPSSClient.java` reconnection logic in the main loop.
#[allow(clippy::too_many_arguments)] // Reason: reconnection requires all FPSS state (subs, config, credentials) in one call.
#[allow(clippy::missing_errors_doc)] // Reason: internal function, doc is on the module-level reconnect docs above.
pub fn reconnect<F>(
    creds: &Credentials,
    hosts: &[(String, u16)],
    previous_subs: Vec<(SubscriptionKind, Contract)>,
    previous_full_subs: Vec<(SubscriptionKind, tdbe::types::enums::SecType)>,
    delay_ms: u64,
    ring_size: usize,
    flush_mode: FpssFlushMode,
    policy: ReconnectPolicy,
    derive_ohlcvc: bool,
    handler: F,
) -> Result<FpssClient, Error>
where
    F: FnMut(&FpssEvent) + Send + 'static,
{
    tracing::info!(delay_ms, "waiting before FPSS reconnection");
    thread::sleep(Duration::from_millis(delay_ms));

    let client = FpssClient::connect(
        creds,
        hosts,
        ring_size,
        flush_mode,
        policy,
        derive_ohlcvc,
        handler,
    )?;

    // Re-subscribe all previous per-contract subscriptions with req_id = -1
    // Source: FPSSClient.java -- reconnect logic uses req_id = -1 for re-subscriptions
    for (kind, contract) in &previous_subs {
        let payload = build_subscribe_payload(-1, contract);
        let code = kind.subscribe_code();

        client.send_cmd(IoCommand::WriteFrame { code, payload })?;

        tracing::debug!(
            kind = ?kind,
            contract = %contract,
            "re-subscribed after reconnect (req_id=-1)"
        );
    }

    // Re-subscribe all previous full-type (firehose) subscriptions with req_id = -1
    for (kind, sec_type) in &previous_full_subs {
        let payload = protocol::build_full_type_subscribe_payload(-1, *sec_type);
        let code = kind.subscribe_code();

        client.send_cmd(IoCommand::WriteFrame { code, payload })?;

        tracing::debug!(
            kind = ?kind,
            sec_type = ?sec_type,
            "re-subscribed full-type after reconnect (req_id=-1)"
        );
    }

    // Store the re-subscribed lists
    {
        let mut subs = client
            .active_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *subs = previous_subs;
    }
    {
        let mut subs = client
            .active_full_subs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *subs = previous_full_subs;
    }

    Ok(client)
}

/// Determine the reconnect delay based on the disconnect reason.
///
/// Source: `FPSSClient.java` -- reconnect logic checks `RemoveReason` to decide delay.
///
/// # Intentional divergence from Java (see jvm-deviations.md)
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
        // credential errors. See jvm-deviations.md "Permanent Disconnect".
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
