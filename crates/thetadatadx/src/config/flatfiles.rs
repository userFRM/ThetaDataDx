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
pub struct FlatFilesConfig {
    /// Total attempts for a single `flatfile_request_raw` call,
    /// including the first attempt. `1` disables retry; values
    /// greater than `1` enable exponential backoff between attempts.
    ///
    /// Validated to the range `[1, 10]` — typical production use fits
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
}

impl FlatFilesConfig {
    /// Production defaults: 3 attempts total, 1s initial backoff,
    /// 4s ceiling — so a sustained transient failure (e.g. a 30s
    /// network blip on the legacy host) surfaces within ~7s of wall
    /// clock instead of either failing fast (single attempt) or
    /// hanging the caller for a runaway retry chain.
    #[must_use]
    pub fn production_defaults() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(4),
        }
    }

    /// Compute the sleep delay before the next retry, capped at
    /// `max_backoff`. `attempt` is 1-based (attempt 1 = first retry
    /// after the initial call failed).
    #[must_use]
    pub fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }
        // `shift = attempt - 1` so attempt 1 = base, attempt 2 = base*2,
        // attempt 3 = base*4. Clamp to 31 so `checked_shl` cannot
        // overflow for pathological inputs.
        let shift = (attempt - 1).min(31);
        let base_nanos = self.initial_backoff.as_nanos();
        let scaled_nanos = base_nanos.checked_shl(shift).unwrap_or(u128::MAX);
        let max_nanos = self.max_backoff.as_nanos();
        let nanos = scaled_nanos.min(max_nanos);
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
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
    pub const MAX_ATTEMPTS: std::ops::RangeInclusive<u32> = 1..=10;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_three_attempts() {
        let cfg = FlatFilesConfig::default();
        assert_eq!(cfg.max_attempts, 3);
        assert_eq!(cfg.initial_backoff, Duration::from_secs(1));
        assert_eq!(cfg.max_backoff, Duration::from_secs(4));
    }

    #[test]
    fn backoff_doubles_then_caps() {
        let cfg = FlatFilesConfig::default();
        assert_eq!(cfg.backoff_for_attempt(0), Duration::ZERO);
        assert_eq!(cfg.backoff_for_attempt(1), Duration::from_secs(1));
        assert_eq!(cfg.backoff_for_attempt(2), Duration::from_secs(2));
        assert_eq!(cfg.backoff_for_attempt(3), Duration::from_secs(4));
        // Capped at max_backoff.
        assert_eq!(cfg.backoff_for_attempt(4), Duration::from_secs(4));
        assert_eq!(cfg.backoff_for_attempt(31), Duration::from_secs(4));
    }
}
