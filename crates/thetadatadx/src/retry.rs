//! Retry + auth-refresh helpers shared by every MDDS RPC.
#![allow(dead_code)]
// Reason: `RetryOutcome::Retry` / `classify_status::NeedsRefresh` are
// reachable only from generator-emitted call sites; standalone unit
// tests still exercise `retry_transient` and `classify_status` but
// the variant-construction helpers are invoked strictly from `macros.rs`,
// which rustc treats as a separate compilation unit.
//!
//! Two orthogonal failure modes call into this module:
//!
//! 1. **Transient gRPC errors** (`Unavailable`, `DeadlineExceeded`,
//!    `ResourceExhausted`) — resolved by exponential backoff governed by
//!    [`crate::config::RetryPolicy`].
//! 2. **Session expiry** (`Unauthenticated`) — resolved by
//!    re-authenticating against Nexus and retrying the call exactly once
//!    with the refreshed token. See [`crate::auth::SessionToken`].
//!
//! Both wrappers operate on a closure that returns a fresh gRPC future
//! on every attempt: tonic streams are single-shot, so the only way to
//! retry is to rebuild the request with the current session UUID.
//!
//! Credential failures (`PermissionDenied`, `NotFound`,
//! `InvalidArgument`) surface as hard errors — they cannot be fixed by
//! waiting, and a refresh would just repeat the rejection.

use std::future::Future;

use crate::config::RetryPolicy;
use crate::error::Error;

/// Outcome classification for a single RPC attempt.
///
/// The closure passed to [`retry_transient`] returns one of these on
/// every poll; the outer loop converts `Retry` into a backoff-and-loop
/// and `Terminal` into a propagated error.
pub(crate) enum RetryOutcome<T> {
    /// Call succeeded — hand the value back to the caller.
    Ok(T),
    /// Transient failure — worth another attempt after backoff.
    Retry(Error),
    /// Permanent failure — stop immediately, surface this error.
    Terminal(Error),
}

/// Classify a [`tonic::Status`] against the retry / refresh policy.
///
/// | Code | Verdict |
/// |---|---|
/// | `Unavailable`, `DeadlineExceeded`, `ResourceExhausted` | retry with backoff |
/// | `Unauthenticated` | session refresh then single retry (handled by caller) |
/// | everything else | terminal — surface to caller unchanged |
#[must_use]
pub(crate) fn classify_status(status: &tonic::Status) -> StatusClass {
    use tonic::Code;
    match status.code() {
        Code::Unavailable | Code::DeadlineExceeded | Code::ResourceExhausted => {
            StatusClass::Transient
        }
        Code::Unauthenticated => StatusClass::NeedsRefresh,
        _ => StatusClass::Terminal,
    }
}

/// See [`classify_status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusClass {
    Transient,
    NeedsRefresh,
    Terminal,
}

/// Run `f` under the configured retry policy.
///
/// `endpoint` is the tracing / metrics label (e.g. `"stock_history_eod"`).
/// `f` must be cheap to clone-and-rerun: expect to be invoked up to
/// `policy.max_attempts` times. Each call MUST rebuild the request from
/// scratch — tonic streams cannot be replayed.
///
/// Attempts are 1-indexed. Between attempt `n` and `n+1` the loop sleeps
/// for [`RetryPolicy::delay_for_attempt(n)`]. On the last attempt we do
/// not sleep afterwards; the terminal error surfaces immediately.
pub(crate) async fn retry_transient<F, Fut, T>(
    policy: &RetryPolicy,
    endpoint: &str,
    mut f: F,
) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = RetryOutcome<T>>,
{
    // `max_attempts = 0` still permits the initial call; clamp to 1 so
    // the loop runs at least once and the caller sees whatever error
    // that first attempt produced.
    let budget = policy.max_attempts.max(1);
    let mut last_err: Option<Error> = None;
    for attempt in 1..=budget {
        match f().await {
            RetryOutcome::Ok(value) => return Ok(value),
            RetryOutcome::Terminal(err) => return Err(err),
            RetryOutcome::Retry(err) => {
                if attempt == budget {
                    last_err = Some(err);
                    break;
                }
                let delay = policy.delay_for_attempt(attempt);
                metrics::counter!(
                    "thetadatadx.grpc.retries",
                    "endpoint" => endpoint.to_string()
                )
                .increment(1);
                tracing::warn!(
                    endpoint,
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    error = %err,
                    "transient gRPC error — retrying with backoff"
                );
                last_err = Some(err);
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    // Unreachable in practice — the loop returns or records `last_err`.
    Err(last_err.unwrap_or_else(|| Error::Config("retry loop exited without result".into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn classify_status_covers_transient_refresh_and_terminal() {
        assert_eq!(
            classify_status(&tonic::Status::unavailable("")),
            StatusClass::Transient
        );
        assert_eq!(
            classify_status(&tonic::Status::deadline_exceeded("")),
            StatusClass::Transient
        );
        assert_eq!(
            classify_status(&tonic::Status::resource_exhausted("")),
            StatusClass::Transient
        );
        assert_eq!(
            classify_status(&tonic::Status::unauthenticated("")),
            StatusClass::NeedsRefresh
        );
        assert_eq!(
            classify_status(&tonic::Status::permission_denied("")),
            StatusClass::Terminal
        );
        assert_eq!(
            classify_status(&tonic::Status::not_found("")),
            StatusClass::Terminal
        );
        assert_eq!(
            classify_status(&tonic::Status::invalid_argument("")),
            StatusClass::Terminal
        );
    }

    #[tokio::test]
    async fn retry_transient_returns_immediately_on_ok() {
        let p = crate::config::RetryPolicy {
            initial_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
            max_attempts: 5,
            jitter: false,
        };
        let n = Arc::new(AtomicU32::new(0));
        let out = retry_transient(&p, "t", || {
            let n = Arc::clone(&n);
            async move {
                n.fetch_add(1, Ordering::SeqCst);
                RetryOutcome::Ok::<u32>(42)
            }
        })
        .await
        .unwrap();
        assert_eq!(out, 42);
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retry_transient_stops_on_terminal() {
        let p = crate::config::RetryPolicy {
            initial_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
            max_attempts: 5,
            jitter: false,
        };
        let n = Arc::new(AtomicU32::new(0));
        let err = retry_transient(&p, "t", || {
            let n = Arc::clone(&n);
            async move {
                n.fetch_add(1, Ordering::SeqCst);
                RetryOutcome::Terminal::<()>(Error::Config("boom".into()))
            }
        })
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Config(_)));
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retry_transient_retries_up_to_budget_then_fails() {
        // max_attempts=3 → 3 total attempts, 2 sleeps between them.
        let p = crate::config::RetryPolicy {
            initial_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
            max_attempts: 3,
            jitter: false,
        };
        let n = Arc::new(AtomicU32::new(0));
        let err = retry_transient(&p, "t", || {
            let n = Arc::clone(&n);
            async move {
                n.fetch_add(1, Ordering::SeqCst);
                RetryOutcome::Retry::<()>(Error::Config("transient".into()))
            }
        })
        .await
        .unwrap_err();
        assert_eq!(n.load(Ordering::SeqCst), 3);
        assert!(matches!(err, Error::Config(_)));
    }

    #[tokio::test]
    async fn retry_transient_succeeds_on_second_attempt() {
        let p = crate::config::RetryPolicy {
            initial_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
            max_attempts: 5,
            jitter: false,
        };
        let n = Arc::new(AtomicU32::new(0));
        let out = retry_transient(&p, "t", || {
            let n = Arc::clone(&n);
            async move {
                let attempt = n.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    RetryOutcome::Retry(Error::Config("first fails".into()))
                } else {
                    RetryOutcome::Ok(7_u32)
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(out, 7);
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }
}
