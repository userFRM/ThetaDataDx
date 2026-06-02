//! Exponential-backoff retry policy for transient gRPC errors on MDDS.

use std::time::Duration;

/// Exponential-backoff retry policy for transient gRPC errors on MDDS.
///
/// Only wired on status codes `Unavailable`, `DeadlineExceeded`, and
/// `ResourceExhausted`. Permission / credential failures route through
/// the separate auto-refresh path (see the in-crate `MddsClient` wrappers)
/// and are never retried by this policy.
///
/// # Jitter
///
/// With `jitter = true` (default) the sleep duration follows AWS's
/// *full jitter* pattern: `delay = rand(0, min(max_delay, initial *
/// 2^attempt))`. Full jitter provably minimises retry-storm contention
/// relative to equal jitter or no jitter; see
/// <https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/>.
///
/// With `jitter = false` the delay is the deterministic backoff
/// `min(max_delay, initial * 2^attempt)`. Useful for tests that
/// need to assert exact timings.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Delay used for the first retry (attempt 1). Doubles per attempt.
    pub initial_delay: Duration,
    /// Upper bound on the computed backoff delay, regardless of attempt.
    pub max_delay: Duration,
    /// Total attempt budget. `1` disables retry (single call only);
    /// `0` still permits the initial call but allows no retries.
    pub max_attempts: u32,
    /// Apply AWS-style full jitter to each retry delay.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(30),
            max_attempts: 5,
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
            jitter_sample(capped)
        } else {
            capped
        }
    }

    /// Deterministic capped backoff (no jitter). Exposed for tests that
    /// need to assert the upper-bound envelope for a given attempt.
    #[must_use]
    pub fn capped_backoff(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }
        // `shift = attempt - 1` so attempt 1 = base, attempt 2 = base*2,
        // attempt 3 = base*4. `u32::checked_shl(shift)` overflows
        // exactly when `shift >= 32`; clamp before shifting.
        let shift = (attempt - 1).min(31);
        let base_nanos = self.initial_delay.as_nanos();
        let scaled_nanos = base_nanos.checked_shl(shift).unwrap_or(u128::MAX);
        let max_nanos = self.max_delay.as_nanos();
        let nanos = scaled_nanos.min(max_nanos);
        // `Duration::from_nanos` takes u64 — clamp rather than truncate.
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }
}

/// Full-jitter sampler: uniform on `[0, ceiling]`. Uses the `Instant`-
/// derived nanosecond clock as an entropy source so we do not pull in
/// a dedicated RNG crate — sufficient for jitter randomisation where
/// the statistical quality requirement is "any non-pathological spread
/// across callers", not cryptographic randomness.
fn jitter_sample(ceiling: Duration) -> Duration {
    let ceiling_nanos = ceiling.as_nanos();
    if ceiling_nanos == 0 {
        return Duration::ZERO;
    }
    // `Instant::elapsed` inside a test might return 0 on some CI
    // schedulers; folding both `elapsed` and a process-local counter
    // guarantees the sampler advances even then.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tick = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now_nanos = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    )
    .unwrap_or(u64::MAX);
    // Reason: splitmix64 constants — documented mixer, fine for jitter.
    let mut seed = now_nanos ^ tick.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    seed ^= seed >> 30;
    seed = seed.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    seed ^= seed >> 27;
    seed = seed.wrapping_mul(0x94D0_49BB_1331_11EB);
    seed ^= seed >> 31;
    let ceiling_u128 = ceiling_nanos;
    let bounded = u128::from(seed) % (ceiling_u128 + 1);
    Duration::from_nanos(u64::try_from(bounded).unwrap_or(u64::MAX))
}
