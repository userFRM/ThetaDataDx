//! Exponential-backoff retry policy for transient gRPC errors on MDDS.

use std::time::Duration;

/// Exponential-backoff retry policy for transient gRPC errors on MDDS.
///
/// Only wired on status codes `Unavailable`, `DeadlineExceeded`, and
/// `ResourceExhausted`. Permission / credential failures route through
/// the separate auto-refresh path and are never retried by this policy.
///
/// # Budget shape
///
/// Two independent stop conditions bound a retry sequence; whichever
/// trips first ends it:
///
/// * `max_attempts` — total attempt count, including the first call.
/// * `max_elapsed` — wall-clock envelope measured from the first
///   attempt. With the default 30 s `max_delay` cap, an attempt-count
///   budget alone is hard to reason about in wall-clock terms;
///   `max_elapsed` lets operators state "retry for up to five minutes"
///   directly. [`Duration::ZERO`] disables the envelope (attempt
///   budget only).
///
/// The defaults (20 attempts, 5 minute envelope) ride through a
/// multi-minute upstream outage: the ladder reaches the 30 s cap at
/// attempt 8 and the envelope cuts the sequence off at five minutes of
/// wall clock.
///
/// # Jitter
///
/// With `jitter = true` (default) the sleep duration follows AWS's
/// *full jitter* pattern: `delay = rand(0, min(max_delay, initial *
/// 2^attempt))`. Full jitter provably minimises retry-storm contention
/// relative to equal jitter or no jitter; see
/// <https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/>
/// and [`crate::backoff`] for the shared sampler.
///
/// With `jitter = false` the delay is the deterministic backoff
/// `min(max_delay, initial * 2^attempt)`. Useful for tests that
/// need to assert exact timings.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Delay used for the first retry (attempt 1). Doubles per attempt.
    pub initial_delay: Duration,
    /// Upper bound on the computed backoff delay, regardless of attempt.
    pub max_delay: Duration,
    /// Total attempt budget. `1` disables retry (single call only);
    /// `0` still permits the initial call but allows no retries.
    pub max_attempts: u32,
    /// Wall-clock envelope for one retry sequence, measured from the
    /// first attempt. Once exceeded, the next transient failure
    /// surfaces to the caller instead of scheduling another retry.
    /// [`Duration::ZERO`] disables the envelope. Default `300 s`.
    pub max_elapsed: Duration,
    /// Apply AWS-style full jitter to each retry delay.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(30),
            max_attempts: 20,
            max_elapsed: Duration::from_secs(300),
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// Build a policy with retry disabled — single attempt, no backoff.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            initial_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
            max_attempts: 1,
            max_elapsed: Duration::ZERO,
            jitter: false,
        }
    }

    /// Compute the sleep delay before the next retry.
    ///
    /// `attempt` is 1-based (attempt 1 = first retry after the initial
    /// call failed). The returned duration is:
    ///
    /// * capped at `max_delay`,
    /// * exponentiated as `initial_delay * 2^(attempt - 1)`,
    /// * jittered (when `self.jitter`) across `[0, capped_delay]`.
    ///
    /// Overflow in `initial_delay * 2^(attempt - 1)` saturates at
    /// `max_delay` rather than wrapping, so pathological `attempt`
    /// values never yield a zero delay.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let capped = self.capped_backoff(attempt);
        if self.jitter {
            crate::backoff::uniform_duration(Duration::ZERO, capped)
        } else {
            capped
        }
    }

    /// Deterministic capped backoff (no jitter). Exposed for tests that
    /// need to assert the upper-bound envelope for a given attempt.
    #[must_use]
    pub fn capped_backoff(&self, attempt: u32) -> Duration {
        crate::backoff::capped_exponential(self.initial_delay, self.max_delay, attempt)
    }

    /// Whether a retry sequence that started `elapsed` ago is still
    /// inside the wall-clock envelope. Always `true` when the envelope
    /// is disabled (`max_elapsed == 0`).
    #[must_use]
    pub fn within_elapsed_budget(&self, elapsed: Duration) -> bool {
        self.max_elapsed.is_zero() || elapsed <= self.max_elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budget_covers_a_multi_minute_outage() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_attempts, 20);
        assert_eq!(policy.max_elapsed, Duration::from_secs(300));
        assert!(policy.jitter);
        // The deterministic ladder reaches the 30s cap at attempt 8;
        // attempts 8..20 ride the cap. The un-jittered total exceeds
        // the 5-minute envelope, so `max_elapsed` is the effective
        // bound — exactly the operator-facing contract.
        let total: Duration = (1..policy.max_attempts)
            .map(|a| policy.capped_backoff(a))
            .sum();
        assert!(
            total >= Duration::from_secs(300),
            "attempt budget must outlast the wall-clock envelope so \
             max_elapsed is the binding constraint; got {total:?}"
        );
    }

    #[test]
    fn capped_backoff_ladder_shape() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.capped_backoff(0), Duration::ZERO);
        assert_eq!(policy.capped_backoff(1), Duration::from_millis(250));
        assert_eq!(policy.capped_backoff(2), Duration::from_millis(500));
        assert_eq!(policy.capped_backoff(3), Duration::from_secs(1));
        assert_eq!(policy.capped_backoff(8), Duration::from_secs(30));
        assert_eq!(policy.capped_backoff(20), Duration::from_secs(30));
        assert_eq!(policy.capped_backoff(u32::MAX), Duration::from_secs(30));
    }

    #[test]
    fn jittered_delay_bounded_by_capped_backoff() {
        let policy = RetryPolicy::default();
        for attempt in 1..=24 {
            let ceiling = policy.capped_backoff(attempt);
            for _ in 0..32 {
                assert!(policy.delay_for_attempt(attempt) <= ceiling);
            }
        }
    }

    #[test]
    fn elapsed_budget_semantics() {
        let policy = RetryPolicy::default();
        assert!(policy.within_elapsed_budget(Duration::ZERO));
        assert!(policy.within_elapsed_budget(Duration::from_secs(300)));
        assert!(!policy.within_elapsed_budget(Duration::from_secs(301)));

        let unbounded = RetryPolicy {
            max_elapsed: Duration::ZERO,
            ..RetryPolicy::default()
        };
        assert!(
            unbounded.within_elapsed_budget(Duration::from_secs(86_400)),
            "zero max_elapsed disables the envelope"
        );
    }

    #[test]
    fn disabled_policy_single_attempt() {
        let policy = RetryPolicy::disabled();
        assert_eq!(policy.max_attempts, 1);
        assert_eq!(policy.max_elapsed, Duration::ZERO);
        assert!(!policy.jitter);
        assert_eq!(policy.delay_for_attempt(1), Duration::ZERO);
    }
}
