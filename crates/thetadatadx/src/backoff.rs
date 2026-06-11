//! Shared backoff and jitter primitives for every retry surface in the
//! SDK: the streaming reconnect driver, the historical-channel retry
//! policy, the historical-channel transport reconnect, and the flatfile
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
//! total work and contention versus equal jitter or no jitter; see
//! <https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/>.

use std::time::Duration;

use rand::Rng;

/// Jitter strategy applied to a computed backoff delay.
///
/// The streaming reconnect driver exposes this knob through
/// [`crate::config::ReconnectConfig::jitter`]; bindings encode it as
/// the lowercase strings `"full"`, `"equal"`, `"decorrelated"`, and
/// `"none"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum JitterMode {
    /// Sample uniformly from `[0, delay]` (AWS full jitter). Best
    /// fleet-level de-synchronisation; an individual retry may fire
    /// almost immediately.
    #[default]
    Full,
    /// `delay / 2 + uniform(0, delay / 2)` (AWS equal jitter). Keeps a
    /// per-attempt floor of half the deterministic delay while still
    /// spreading a fleet across the upper half of the window.
    Equal,
    /// Decorrelated walk: `min(cap, uniform(initial, prev * 3))`. Each
    /// delay is sampled relative to the previous one rather than the
    /// attempt number, which spreads long-running retry sessions even
    /// when their attempt counters align.
    Decorrelated,
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
            Self::Equal => "equal",
            Self::Decorrelated => "decorrelated",
            Self::None => "none",
        }
    }

    /// Parse the cross-binding string encoding (case-insensitive).
    /// Returns `Option::None` for unrecognised input.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "full" => Some(Self::Full),
            "equal" => Some(Self::Equal),
            "decorrelated" => Some(Self::Decorrelated),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    /// Jitter the deterministic `delay` for one retry attempt.
    ///
    /// `schedule` carries the ladder bounds plus the previous jittered
    /// delay that [`JitterMode::Decorrelated`] walks from; the other
    /// modes ignore everything except `delay`. The returned value is
    /// recorded back into `schedule` so the decorrelated walk advances.
    ///
    /// Guarantees, by mode:
    ///
    /// * `Full` — result in `[0, delay]`.
    /// * `Equal` — result in `[delay / 2, delay]`.
    /// * `Decorrelated` — result in `[min(initial, cap), cap]`.
    /// * `None` — result is exactly `delay`.
    pub(crate) fn sample(self, delay: Duration, schedule: &mut BackoffSchedule) -> Duration {
        let sampled = match self {
            Self::None => delay,
            Self::Full => uniform_duration(Duration::ZERO, delay),
            Self::Equal => {
                let half = delay / 2;
                half + uniform_duration(Duration::ZERO, delay - half)
            }
            Self::Decorrelated => {
                let prev = schedule.prev.unwrap_or(schedule.initial);
                let upper = prev
                    .saturating_mul(3)
                    .clamp(schedule.initial, schedule.cap.max(schedule.initial));
                uniform_duration(schedule.initial.min(schedule.cap), upper)
            }
        };
        schedule.prev = Some(sampled);
        sampled
    }
}

/// Ladder bounds plus the per-burst state the decorrelated walk needs.
///
/// One value lives per retry burst (e.g. per consecutive-reconnect
/// sequence in the streaming I/O loop); call
/// [`BackoffSchedule::reset`] when the burst ends so the next burst
/// starts the walk from `initial` again.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BackoffSchedule {
    /// First-attempt delay; doubles per attempt.
    pub(crate) initial: Duration,
    /// Upper bound on the deterministic ladder.
    pub(crate) cap: Duration,
    /// Previous jittered delay ([`JitterMode::Decorrelated`] state).
    pub(crate) prev: Option<Duration>,
}

impl BackoffSchedule {
    pub(crate) fn new(initial: Duration, cap: Duration) -> Self {
        Self {
            initial,
            cap,
            prev: None,
        }
    }

    /// Deterministic capped delay for a 1-based `attempt` number.
    pub(crate) fn deterministic(&self, attempt: u32) -> Duration {
        capped_exponential(self.initial, self.cap, attempt)
    }

    /// Forget the decorrelated-walk state. Call when a retry burst
    /// ends (successful data flow / stable window) so the next burst
    /// restarts from `initial`.
    pub(crate) fn reset(&mut self) {
        self.prev = None;
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

/// Process-local entropy word for seeding shuffles and cursor offsets
/// where no caller-supplied seed exists. Folds the wall clock with a
/// process-local counter so two clients constructed in the same
/// nanosecond still diverge.
pub(crate) fn entropy_u64() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tick = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    )
    .unwrap_or(u64::MAX);
    let pid = u64::from(std::process::id());
    splitmix64(now ^ pid.rotate_left(32) ^ tick.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// splitmix64 finaliser — documented mixer, used here only to spread
/// seed bits, never for cryptographic purposes.
pub(crate) fn splitmix64(mut seed: u64) -> u64 {
    seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    seed ^= seed >> 30;
    seed = seed.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    seed ^= seed >> 27;
    seed = seed.wrapping_mul(0x94D0_49BB_1331_11EB);
    seed ^= seed >> 31;
    seed
}

/// Apply hash-stable decorrelated jitter (±10 %) to a transport
/// reconnect backoff window.
///
/// Uses a `DefaultHasher` over `(host, port, attempt)` so the
/// per-client schedule is stable across runs (useful for tests) while
/// diverging across deployments. The output is clamped to
/// `[base * 0.9, base * 1.1]`, so a budget cap on `base` still holds
/// within ~10 %. The historical-channel transport reconnect uses this
/// shape; surfaces that need fleet-level de-synchronisation should
/// prefer [`JitterMode::Full`].
pub(crate) fn hash_stable_jitter(base: Duration, host: &str, port: u16, attempt: u32) -> Duration {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    host.hash(&mut hasher);
    port.hash(&mut hasher);
    attempt.hash(&mut hasher);
    // Map hash low bits to a [-1.0, 1.0) signed offset.
    let raw = f64::from(hasher.finish() as u32) / f64::from(u32::MAX);
    let signed = raw * 2.0 - 1.0;
    let factor = 1.0 + signed * 0.1; // +/- 10%
    let scaled = (base.as_millis() as f64 * factor).max(1.0);
    Duration::from_millis(scaled as u64)
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
        let mut schedule =
            BackoffSchedule::new(Duration::from_millis(250), Duration::from_secs(30));
        let delay = Duration::from_secs(4);
        for _ in 0..256 {
            let sampled = JitterMode::Full.sample(delay, &mut schedule);
            assert!(sampled <= delay, "full jitter must not exceed the delay");
        }
    }

    #[test]
    fn equal_jitter_keeps_half_delay_floor() {
        let mut schedule =
            BackoffSchedule::new(Duration::from_millis(250), Duration::from_secs(30));
        let delay = Duration::from_secs(4);
        for _ in 0..256 {
            let sampled = JitterMode::Equal.sample(delay, &mut schedule);
            assert!(sampled >= delay / 2, "equal jitter floor is delay/2");
            assert!(sampled <= delay, "equal jitter must not exceed the delay");
        }
    }

    #[test]
    fn decorrelated_jitter_walks_within_initial_and_cap() {
        let initial = Duration::from_millis(250);
        let cap = Duration::from_secs(30);
        let mut schedule = BackoffSchedule::new(initial, cap);
        for attempt in 1..64_u32 {
            let base = schedule.deterministic(attempt);
            let sampled = JitterMode::Decorrelated.sample(base, &mut schedule);
            assert!(
                sampled >= initial,
                "decorrelated floor is the initial delay"
            );
            assert!(sampled <= cap, "decorrelated ceiling is the cap");
        }
    }

    #[test]
    fn none_jitter_returns_exact_delay() {
        let mut schedule =
            BackoffSchedule::new(Duration::from_millis(250), Duration::from_secs(30));
        let delay = Duration::from_millis(1_234);
        assert_eq!(JitterMode::None.sample(delay, &mut schedule), delay);
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
        for mode in [
            JitterMode::Full,
            JitterMode::Equal,
            JitterMode::Decorrelated,
            JitterMode::None,
        ] {
            assert_eq!(JitterMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(JitterMode::parse("FULL"), Some(JitterMode::Full));
        assert_eq!(JitterMode::parse("bogus"), None);
        assert_eq!(JitterMode::default(), JitterMode::Full);
    }

    #[test]
    fn hash_stable_jitter_is_reproducible_and_bounded() {
        let base = Duration::from_secs(10);
        let a = hash_stable_jitter(base, "host-a", 443, 3);
        let b = hash_stable_jitter(base, "host-a", 443, 3);
        assert_eq!(a, b, "same inputs must yield the same jitter");
        let other = hash_stable_jitter(base, "host-b", 443, 3);
        // Bounded to +/- 10% (plus the 1ms floor).
        for v in [a, other] {
            assert!(v >= Duration::from_millis(9_000));
            assert!(v <= Duration::from_millis(11_000));
        }
    }

    #[test]
    fn entropy_words_diverge() {
        let a = entropy_u64();
        let b = entropy_u64();
        assert_ne!(a, b, "consecutive entropy words must differ");
    }

    #[test]
    fn backoff_schedule_reset_clears_walk_state() {
        let mut schedule = BackoffSchedule::new(Duration::from_millis(100), Duration::from_secs(5));
        let _ = JitterMode::Decorrelated.sample(Duration::from_millis(100), &mut schedule);
        assert!(schedule.prev.is_some());
        schedule.reset();
        assert!(schedule.prev.is_none());
    }
}
