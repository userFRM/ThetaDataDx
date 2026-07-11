//! Shared backoff and jitter primitives for every retry surface in the
//! SDK: the streaming reconnect driver, the market-data-channel retry
//! policy, the market-data-channel transport reconnect, and the flatfile
//! retry loop.
//!
//! # Why one module
//!
//! Every retry surface needs the same two ingredients — a capped
//! exponential delay ladder and a jitter sampler that de-synchronises a
//! fleet of clients retrying against the same recovering endpoint. When
//! each surface rolls its own, the samplers drift apart (one applies
//! jitter, another does not) and a fleet-wide disconnect turns into a
//! synchronised retry burst at exactly `base * 2^n` milliseconds. This
//! module is the single source of truth for both ingredients.
//!
//! # Jitter background
//!
//! The default [`JitterMode::Full`] follows AWS's *full jitter*
//! analysis: sampling uniformly from `[0, capped_delay]` minimises
//! total work and contention versus no jitter; see
//! <https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/>.

use std::time::Duration;

use rand::RngExt;

/// Jitter strategy applied to a computed backoff delay.
///
/// The streaming reconnect driver exposes this knob through
/// [`crate::config::ReconnectConfig::jitter`]; bindings encode it as
/// the lowercase strings `"full"` and `"none"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum JitterMode {
    /// Sample uniformly from `[0, delay]` (AWS full jitter). Best
    /// fleet-level de-synchronisation; an individual retry may fire
    /// almost immediately.
    #[default]
    Full,
    /// No jitter — the deterministic capped delay. Useful for tests
    /// that assert exact timings; not recommended for fleets.
    None,
}

impl JitterMode {
    /// Canonical lowercase string for this mode, matching the
    /// cross-binding encoding.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::None => "none",
        }
    }

    /// Parse the cross-binding string encoding (case-insensitive).
    /// Returns `Option::None` for unrecognised input.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "full" => Some(Self::Full),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    /// Jitter the deterministic `delay` for one retry attempt.
    ///
    /// Guarantees, by mode:
    ///
    /// * `Full` — result in `[0, delay]`.
    /// * `None` — result is exactly `delay`.
    #[must_use]
    pub(crate) fn sample(self, delay: Duration) -> Duration {
        match self {
            Self::None => delay,
            Self::Full => uniform_duration(Duration::ZERO, delay),
        }
    }
}

/// Deterministic exponential-ladder bounds for a retry burst.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BackoffSchedule {
    /// First-attempt delay; doubles per attempt.
    pub(crate) initial: Duration,
    /// Upper bound on the deterministic ladder.
    pub(crate) cap: Duration,
}

impl BackoffSchedule {
    pub(crate) fn new(initial: Duration, cap: Duration) -> Self {
        Self { initial, cap }
    }

    /// Deterministic capped delay for a 1-based `attempt` number.
    pub(crate) fn deterministic(&self, attempt: u32) -> Duration {
        capped_exponential(self.initial, self.cap, attempt)
    }
}

/// Deterministic capped exponential ladder shared by every retry
/// surface: `min(cap, initial * 2^(attempt - 1))`.
///
/// `attempt` is 1-based — attempt `1` returns `initial`, attempt `2`
/// returns `initial * 2`, and so on up to `cap`. `attempt == 0`
/// returns [`Duration::ZERO`]. The shift is clamped so pathological
/// attempt numbers saturate at `cap` instead of wrapping.
#[must_use]
pub fn capped_exponential(initial: Duration, cap: Duration, attempt: u32) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let shift = (attempt - 1).min(31);
    let base_nanos = initial.as_nanos();
    let scaled_nanos = base_nanos.checked_shl(shift).unwrap_or(u128::MAX);
    let nanos = scaled_nanos.min(cap.as_nanos());
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// Uniform sample from `[lo, hi]` (inclusive). Returns `lo` when the
/// range is empty or inverted.
pub(crate) fn uniform_duration(lo: Duration, hi: Duration) -> Duration {
    if hi <= lo {
        return lo;
    }
    let lo_nanos = u64::try_from(lo.as_nanos()).unwrap_or(u64::MAX);
    let hi_nanos = u64::try_from(hi.as_nanos()).unwrap_or(u64::MAX);
    if hi_nanos <= lo_nanos {
        return lo;
    }
    let sampled = rand::rng().random_range(lo_nanos..=hi_nanos);
    Duration::from_nanos(sampled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_exponential_ladder_doubles_then_caps() {
        let initial = Duration::from_millis(250);
        let cap = Duration::from_secs(30);
        assert_eq!(capped_exponential(initial, cap, 0), Duration::ZERO);
        assert_eq!(capped_exponential(initial, cap, 1), initial);
        assert_eq!(
            capped_exponential(initial, cap, 2),
            Duration::from_millis(500)
        );
        assert_eq!(capped_exponential(initial, cap, 3), Duration::from_secs(1));
        assert_eq!(
            capped_exponential(initial, cap, 8),
            Duration::from_secs(32).min(cap)
        );
        // Attempt 8 = 250ms * 128 = 32s, capped at 30s.
        assert_eq!(capped_exponential(initial, cap, 8), cap);
        // Way past the cap: stays at the cap, no overflow.
        assert_eq!(capped_exponential(initial, cap, 64), cap);
        assert_eq!(capped_exponential(initial, cap, u32::MAX), cap);
    }

    #[test]
    fn full_jitter_stays_within_zero_to_delay() {
        let delay = Duration::from_secs(4);
        for _ in 0..256 {
            let sampled = JitterMode::Full.sample(delay);
            assert!(sampled <= delay, "full jitter must not exceed the delay");
        }
    }

    #[test]
    fn none_jitter_returns_exact_delay() {
        let delay = Duration::from_millis(1_234);
        assert_eq!(JitterMode::None.sample(delay), delay);
    }

    #[test]
    fn uniform_duration_handles_degenerate_ranges() {
        let d = Duration::from_millis(7);
        assert_eq!(uniform_duration(d, d), d);
        assert_eq!(uniform_duration(d, Duration::ZERO), d);
        for _ in 0..64 {
            let sampled = uniform_duration(Duration::ZERO, d);
            assert!(sampled <= d);
        }
    }

    #[test]
    fn jitter_mode_string_round_trip() {
        for mode in [JitterMode::Full, JitterMode::None] {
            assert_eq!(JitterMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(JitterMode::parse("FULL"), Some(JitterMode::Full));
        assert_eq!(JitterMode::parse("bogus"), None);
        assert_eq!(JitterMode::default(), JitterMode::Full);
    }
}
