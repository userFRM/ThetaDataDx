//! FLATFILES legacy-MDDS sub-configuration.
//!
//! The flatfile driver runs one TLS round-trip per request — connect to
//! a host from `ALLOWED_MDDS_HOSTS`, send credentials, send the request,
//! stream the response. Both the connect/auth phase and the streaming
//! loop can fail with transient errors (mid-stream truncation, momentary
//! socket-level blip on the legacy host). This sub-config carries the
//! retry budget the driver applies to those transient failures.
//!
//! Terminal failures (bad credentials, permanently-rejected request
//! parameters) are surfaced immediately regardless of the budget — see
//! [`crate::flatfiles::FlatFilesUnavailableReason::is_transient`].

use std::time::Duration;

/// FLATFILES retry tuning.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FlatFilesConfig {
    /// Total attempts for a single `flatfile_request_raw` call,
    /// including the first attempt. `1` disables retry; values
    /// greater than `1` enable exponential backoff between attempts.
    ///
    /// Validated to the range `[1, 100]` — typical production use fits
    /// inside that envelope. A misconfiguration outside the range is
    /// rejected at [`crate::config::DirectConfig::validate`] time
    /// rather than silently capped.
    pub max_attempts: u32,

    /// Delay used for the first retry (attempt 1). Doubles per attempt
    /// up to `max_backoff`.
    pub initial_backoff: Duration,

    /// Upper bound on the computed backoff delay, regardless of
    /// attempt number.
    pub max_backoff: Duration,

    /// Apply AWS-style full jitter to each retry delay (uniform over
    /// `[0, capped_backoff]`). Default `true` — backfill traffic after
    /// an upstream outage hits the same hosts from many clients at
    /// once, and an un-jittered ladder lands those retries in
    /// lockstep. Disable only for tests that assert exact timings.
    pub jitter: bool,
}

impl FlatFilesConfig {
    /// Production defaults: 10 attempts total, 1 s initial backoff,
    /// 30 s ceiling, full jitter. The deterministic ladder is
    /// 1 s, 2 s, 4 s, 8 s, 16 s, then 30 s flat — roughly three
    /// minutes of runway, sized so a historical backfill keeps
    /// retrying across a multi-minute upstream outage instead of
    /// surfacing an error within seconds of the disconnect.
    #[must_use]
    pub fn production_defaults() -> Self {
        Self {
            max_attempts: 10,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            jitter: true,
        }
    }

    /// Compute the deterministic sleep ceiling before the next retry,
    /// capped at `max_backoff`. `attempt` is 1-based (attempt 1 =
    /// first retry after the initial call failed). Jitter is NOT
    /// applied here — see [`Self::delay_for_attempt`] for the value
    /// the driver actually sleeps.
    #[must_use]
    pub fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        crate::backoff::capped_exponential(self.initial_backoff, self.max_backoff, attempt)
    }

    /// Sleep delay before the next retry: the capped deterministic
    /// ladder from [`Self::backoff_for_attempt`], full-jittered across
    /// `[0, ceiling]` when [`Self::jitter`] is set.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let ceiling = self.backoff_for_attempt(attempt);
        if self.jitter {
            crate::backoff::uniform_duration(Duration::ZERO, ceiling)
        } else {
            ceiling
        }
    }
}

impl Default for FlatFilesConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}

/// Validation bounds for the wired FLATFILES knobs.
pub mod bounds {
    /// Allowed range for [`super::FlatFilesConfig::max_attempts`].
    pub const MAX_ATTEMPTS: std::ops::RangeInclusive<u32> = 1..=100;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_a_multi_minute_outage() {
        let cfg = FlatFilesConfig::default();
        assert_eq!(cfg.max_attempts, 10);
        assert_eq!(cfg.initial_backoff, Duration::from_secs(1));
        assert_eq!(cfg.max_backoff, Duration::from_secs(30));
        assert!(cfg.jitter);
        // Un-jittered runway across the full budget: 1+2+4+8+16+30*4
        // = 151 s of sleep, plus per-attempt connect time. Pins the
        // "minutes, not seconds" envelope.
        let total: Duration = (1..cfg.max_attempts)
            .map(|a| cfg.backoff_for_attempt(a))
            .sum();
        assert!(
            total >= Duration::from_secs(120),
            "flatfile retry runway must span minutes; got {total:?}"
        );
    }

    #[test]
    fn backoff_doubles_then_caps() {
        let cfg = FlatFilesConfig::default();
        assert_eq!(cfg.backoff_for_attempt(0), Duration::ZERO);
        assert_eq!(cfg.backoff_for_attempt(1), Duration::from_secs(1));
        assert_eq!(cfg.backoff_for_attempt(2), Duration::from_secs(2));
        assert_eq!(cfg.backoff_for_attempt(3), Duration::from_secs(4));
        assert_eq!(cfg.backoff_for_attempt(4), Duration::from_secs(8));
        assert_eq!(cfg.backoff_for_attempt(5), Duration::from_secs(16));
        // Capped at max_backoff from attempt 6 onward.
        assert_eq!(cfg.backoff_for_attempt(6), Duration::from_secs(30));
        assert_eq!(cfg.backoff_for_attempt(31), Duration::from_secs(30));
    }

    #[test]
    fn jittered_delay_bounded_by_ladder() {
        let cfg = FlatFilesConfig::default();
        for attempt in 1..=12 {
            let ceiling = cfg.backoff_for_attempt(attempt);
            for _ in 0..32 {
                assert!(cfg.delay_for_attempt(attempt) <= ceiling);
            }
        }
        let deterministic = FlatFilesConfig {
            jitter: false,
            ..FlatFilesConfig::default()
        };
        assert_eq!(
            deterministic.delay_for_attempt(3),
            deterministic.backoff_for_attempt(3),
            "jitter=false must return the exact ladder value"
        );
    }
}
