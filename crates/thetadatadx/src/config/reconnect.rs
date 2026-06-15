//! FPSS reconnection sub-configuration.

use std::sync::Arc;
use std::time::Duration;

use crate::tdbe::types::enums::RemoveReason;

use crate::backoff::JitterMode;

/// Width of the jitter window added on top of the rate-limited
/// reconnect floor.
///
/// A rate-limited disconnect carries a server-instructed cooldown
/// ([`ReconnectConfig::wait_rate_limited_ms`], default 130 s) that must
/// be honoured as a floor — reconnecting earlier re-triggers the
/// throttle. Jitter therefore samples `[floor, floor + window]` rather
/// than `[0, floor]`: a fleet of throttled clients scatters across a
/// 30 s window after the cooldown instead of reconnecting in lockstep
/// at exactly `floor`.
pub const RATE_LIMITED_JITTER_WINDOW: Duration = Duration::from_secs(30);

/// Controls FPSS reconnection behavior after a disconnect.
///
/// # Default
///
/// [`ReconnectPolicy::Auto`] uses attempt counters split by
/// failure class (see [`ReconnectAttemptLimits`]):
///
/// * **Permanent** reasons (invalid credentials, account issues) —
///   short-circuit immediately. No retries regardless of budget.
/// * **Rate-limited** transient (`TooManyRequests`) — long-patient
///   backoff. Default budget 100 attempts at a 130 s floor each, so
///   the loop keeps trying for up to ~3.6 h before giving up.
/// * **Server-restart** transient (`ServerRestarting`) — a pool bounce
///   announcement. Default budget 60 attempts at a flat 5 s cadence,
///   a ~5 minute window matched to typical restart durations.
/// * **Generic** transient (TimedOut, Unspecified, unknown codes, …) —
///   exponential backoff from 250 ms doubling to a 30 s cap, budgeted
///   by both an attempt count (default 30) and a wall-clock envelope
///   (default 5 minutes). Unknown disconnect codes land here
///   deliberately: an unrecognised code is more likely transient than
///   permanent, so the catch-all carries the long multi-minute budget.
///
/// Every computed delay is jittered per [`ReconnectConfig::jitter`]
/// (default [`JitterMode::Full`]) so a fleet of clients dropped by the
/// same upstream event does not reconnect in lockstep.
///
/// The counters reset after a configurable stable window of continuous
/// data flow (default 60 s) so a connection that runs cleanly for a
/// minute then drops gets the full retry budget again rather than
/// burning through one cycle's worth and falling off.
///
/// # Custom
///
/// Supply a closure that receives the disconnect reason and attempt number (1-based)
/// and returns `Some(delay)` to reconnect after that delay, or `None` to stop.
///
/// The closure only ever receives **retriable** reasons. Permanent
/// reasons (the set [`ReconnectAttemptLimits::class_for`] maps to
/// `None`: invalid credentials, account conflicts, …) short-circuit
/// the I/O loop before the closure is consulted — no return value can
/// turn a credential rejection into a retry loop.
#[derive(Clone)]
#[non_exhaustive]
pub enum ReconnectPolicy {
    /// Auto-reconnect with attempt budgets split by failure class.
    Auto(ReconnectAttemptLimits),
    /// No auto-reconnect. User calls `reconnect_streaming()` manually.
    Manual,
    /// User-provided function: `(reason, attempt_number) -> Option<Duration>`.
    ///
    /// Return `Some(delay)` to reconnect after `delay`, `None` to stop.
    /// `attempt_number` starts at 1 and increments on each consecutive reconnect.
    ///
    /// Only retriable reasons reach the closure; permanent reasons
    /// (bad credentials, account conflicts) stop the I/O loop before
    /// the closure runs.
    Custom(Arc<dyn Fn(RemoveReason, u32) -> Option<Duration> + Send + Sync>),
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self::Auto(ReconnectAttemptLimits::default())
    }
}

/// Per-failure-class attempt budgets the [`ReconnectPolicy::Auto`] driver
/// enforces in the FPSS I/O loop.
///
/// Splitting the cap by failure class keeps the long-patient
/// rate-limited budget (`TooManyRequests`, 130 s spacing) independent
/// of the fast exponential generic-transient ladder — a sustained
/// throttle should be ridden out for hours, while a generic outage is
/// retried aggressively at first and then at a 30 s cadence until the
/// wall-clock envelope expires.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ReconnectAttemptLimits {
    /// Maximum consecutive reconnect attempts on a generic transient
    /// failure (TimedOut, Unspecified, unknown codes, …) before giving
    /// up. Default `30`.
    ///
    /// With the default exponential ladder (250 ms doubling to a 30 s
    /// cap, full jitter) the un-jittered schedule spans ~12 minutes of
    /// sleep across 30 attempts, so [`Self::max_elapsed`] (default
    /// 5 minutes) is normally the binding constraint. Raise both
    /// values together for deployments that must ride out longer
    /// outages unattended.
    pub max_attempts: u32,

    /// Maximum consecutive reconnect attempts on a `TooManyRequests`
    /// rate-limited transient before giving up. Default `100` — at
    /// the 130 s floor per attempt this absorbs ~3.6 h of sustained
    /// throttling without operator intervention. The wall-clock
    /// envelope ([`Self::max_elapsed`]) deliberately does NOT apply to
    /// this class.
    pub max_rate_limited_attempts: u32,

    /// Maximum consecutive reconnect attempts on a `ServerRestarting`
    /// disconnect before giving up. Default `60` — at the flat 5 s
    /// cadence ([`super::ReconnectConfig::wait_server_restart_ms`])
    /// this covers a ~5 minute pool bounce.
    pub max_server_restart_attempts: u32,

    /// Wall-clock envelope for one consecutive-reconnect sequence on
    /// the generic-transient and server-restart classes, measured from
    /// the first attempt of the sequence. Once exceeded the I/O loop
    /// stops reconnecting and emits a terminal
    /// `ReconnectsExhausted` control event. [`Duration::ZERO`]
    /// disables the envelope (attempt budgets only). Default `300 s`.
    ///
    /// The rate-limited class is exempt: its 130 s-per-attempt floor
    /// makes the attempt count the natural budget unit there, and a
    /// 5 minute envelope would defeat the multi-hour throttle budget.
    pub max_elapsed: Duration,

    /// Continuous successful-data-flow window after which the
    /// per-class attempt counters reset. A connection that runs
    /// cleanly for at least this long picks up a fresh retry budget
    /// when it next drops. Default `60 s`.
    ///
    /// The window requires at least one successful frame read on the
    /// current session: a session that authenticates but drops before
    /// any frame arrives does not arm the window, so a sequence of
    /// connect-then-immediate-drop cycles keeps consuming the same
    /// budget (and eventually the wall-clock envelope) rather than
    /// resetting it.
    pub stable_window: Duration,
}

impl Default for ReconnectAttemptLimits {
    fn default() -> Self {
        Self {
            max_attempts: 30,
            max_rate_limited_attempts: 100,
            max_server_restart_attempts: 60,
            max_elapsed: Duration::from_secs(300),
            stable_window: Duration::from_secs(60),
        }
    }
}

impl ReconnectAttemptLimits {
    /// Classify a [`RemoveReason`] into the matching attempt-budget
    /// counter for the [`ReconnectPolicy::Auto`] driver. Reasons that
    /// [`crate::fpss::reconnect_delay`] treats as permanent return
    /// `None` — the caller is expected to short-circuit on permanent
    /// reasons before consulting the budget.
    #[must_use]
    pub fn class_for(reason: RemoveReason) -> Option<ReconnectAttemptClass> {
        match reason {
            RemoveReason::AccountAlreadyConnected
            | RemoveReason::InvalidCredentials
            | RemoveReason::InvalidLoginValues
            | RemoveReason::InvalidLoginSize
            | RemoveReason::FreeAccount
            | RemoveReason::ServerUserDoesNotExist
            | RemoveReason::InvalidCredentialsNullUser => None,
            RemoveReason::TooManyRequests => Some(ReconnectAttemptClass::RateLimited),
            RemoveReason::ServerRestarting => Some(ReconnectAttemptClass::ServerRestart),
            _ => Some(ReconnectAttemptClass::Transient),
        }
    }

    /// Maximum attempts for the given attempt class.
    #[must_use]
    pub fn budget_for(&self, class: ReconnectAttemptClass) -> u32 {
        match class {
            ReconnectAttemptClass::Transient => self.max_attempts,
            ReconnectAttemptClass::RateLimited => self.max_rate_limited_attempts,
            ReconnectAttemptClass::ServerRestart => self.max_server_restart_attempts,
        }
    }

    /// Whether the wall-clock envelope applies to `class`. The
    /// rate-limited class rides its attempt budget alone — see
    /// [`Self::max_elapsed`].
    #[must_use]
    pub fn elapsed_budget_applies(class: ReconnectAttemptClass) -> bool {
        !matches!(class, ReconnectAttemptClass::RateLimited)
    }
}

/// Reconnect attempt counter classification used by
/// [`ReconnectPolicy::Auto`]. Each class carries its own counter and
/// budget; the counters reset independently when the stable window
/// elapses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReconnectAttemptClass {
    /// Generic transient (TimedOut, Unspecified, unknown codes, …).
    Transient,
    /// `TooManyRequests` rate-limited transient.
    RateLimited,
    /// `ServerRestarting` pool-bounce transient.
    ServerRestart,
}

impl std::fmt::Debug for ReconnectPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto(limits) => write!(f, "Auto({limits:?})"),
            Self::Manual => write!(f, "Manual"),
            Self::Custom(_) => write!(f, "Custom(...)"),
        }
    }
}

/// FPSS auto-reconnection cadence + policy.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ReconnectConfig {
    /// Initial reconnect delay (ms) for generic transient drops
    /// (TimedOut, Unspecified, unknown codes, …). The
    /// [`ReconnectPolicy::Auto`] driver doubles this per consecutive
    /// attempt up to [`Self::wait_max_ms`], then applies
    /// [`Self::jitter`]. Default `250`.
    ///
    /// Plumbed into the FPSS I/O loop through
    /// [`crate::fpss::StreamingClientBuilder::reconnect_wait_ms`] at
    /// [`crate::Client::start_streaming`] /
    /// [`crate::Client::reconnect_streaming`] connect time.
    pub wait_ms: u64,

    /// Upper bound (ms) on the exponential generic-transient reconnect
    /// ladder. Default `30_000` — the ladder reaches this cap at
    /// attempt 8 with the default `wait_ms` and stays there for the
    /// remainder of the budget.
    pub wait_max_ms: u64,

    /// Floor delay (ms) before reconnecting after a `TooManyRequests`
    /// disconnect. Default `130_000` — the upstream-instructed
    /// rate-limit cooldown. Jitter samples `[floor, floor +`
    /// [`RATE_LIMITED_JITTER_WINDOW`]`]` so the cooldown is always
    /// honoured in full.
    pub wait_rate_limited_ms: u64,

    /// Flat reconnect cadence (ms) for `ServerRestarting` disconnects.
    /// Default `5_000` — a pool bounce wants patient, evenly-spaced
    /// retries rather than an aggressive exponential ramp. Jittered
    /// per [`Self::jitter`].
    pub wait_server_restart_ms: u64,

    /// Jitter strategy applied to every computed reconnect delay.
    /// Default [`JitterMode::Full`]. See [`crate::backoff`] for the
    /// mode semantics; [`JitterMode::None`] restores deterministic
    /// delays for tests.
    pub jitter: JitterMode,

    /// Number of subscription-replay frames written per burst when the
    /// auto-reconnect path restores saved subscriptions onto a fresh
    /// session. Default `50`. After each burst the I/O loop flushes
    /// and pauses [`Self::replay_pace_ms`] so a recovering upstream is
    /// not handed thousands of subscribe frames in one syscall.
    /// Minimum `1` (validated).
    pub replay_burst_size: u32,

    /// Pause (ms) between subscription-replay bursts. Default `5`.
    /// `0` removes the pause (bursts still flush individually). The
    /// actual pause is jittered ±20 % so a fleet of reconnecting
    /// clients does not flush replay bursts in phase.
    pub replay_pace_ms: u64,

    /// Controls FPSS auto-reconnection behavior after involuntary disconnect.
    ///
    /// Default: [`ReconnectPolicy::Auto`].
    pub policy: ReconnectPolicy,
}

impl ReconnectConfig {
    /// Production defaults for ThetaData's FPSS reconnect cadence.
    #[must_use]
    pub fn production_defaults() -> Self {
        Self {
            wait_ms: 250,
            wait_max_ms: 30_000,
            wait_rate_limited_ms: 130_000,
            wait_server_restart_ms: 5_000,
            jitter: JitterMode::Full,
            replay_burst_size: 50,
            replay_pace_ms: 5,
            policy: ReconnectPolicy::Auto(ReconnectAttemptLimits::default()),
        }
    }
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_for_permanent_returns_none() {
        for reason in [
            RemoveReason::AccountAlreadyConnected,
            RemoveReason::InvalidCredentials,
            RemoveReason::InvalidLoginValues,
            RemoveReason::InvalidLoginSize,
            RemoveReason::FreeAccount,
            RemoveReason::ServerUserDoesNotExist,
            RemoveReason::InvalidCredentialsNullUser,
        ] {
            assert!(
                ReconnectAttemptLimits::class_for(reason).is_none(),
                "{reason:?} should be classified as permanent"
            );
        }
    }

    #[test]
    fn class_for_splits_rate_limited_server_restart_and_generic() {
        assert_eq!(
            ReconnectAttemptLimits::class_for(RemoveReason::TooManyRequests),
            Some(ReconnectAttemptClass::RateLimited),
        );
        assert_eq!(
            ReconnectAttemptLimits::class_for(RemoveReason::ServerRestarting),
            Some(ReconnectAttemptClass::ServerRestart),
        );
        assert_eq!(
            ReconnectAttemptLimits::class_for(RemoveReason::TimedOut),
            Some(ReconnectAttemptClass::Transient),
        );
        assert_eq!(
            ReconnectAttemptLimits::class_for(RemoveReason::Unspecified),
            Some(ReconnectAttemptClass::Transient),
        );
    }

    #[test]
    fn default_budgets_per_class() {
        let limits = ReconnectAttemptLimits::default();
        assert_eq!(limits.max_attempts, 30);
        assert_eq!(limits.max_rate_limited_attempts, 100);
        assert_eq!(limits.max_server_restart_attempts, 60);
        assert_eq!(limits.max_elapsed, Duration::from_secs(300));
        assert_eq!(limits.stable_window, Duration::from_secs(60));
        assert_eq!(limits.budget_for(ReconnectAttemptClass::Transient), 30);
        assert_eq!(
            limits.budget_for(ReconnectAttemptClass::RateLimited),
            100,
            "rate-limited budget must absorb sustained TooManyRequests"
        );
        assert_eq!(limits.budget_for(ReconnectAttemptClass::ServerRestart), 60);
    }

    #[test]
    fn elapsed_budget_exempts_rate_limited_class() {
        assert!(ReconnectAttemptLimits::elapsed_budget_applies(
            ReconnectAttemptClass::Transient
        ));
        assert!(ReconnectAttemptLimits::elapsed_budget_applies(
            ReconnectAttemptClass::ServerRestart
        ));
        assert!(!ReconnectAttemptLimits::elapsed_budget_applies(
            ReconnectAttemptClass::RateLimited
        ));
    }

    #[test]
    fn default_cadence_is_exponential_with_jitter() {
        let cfg = ReconnectConfig::default();
        assert_eq!(cfg.wait_ms, 250);
        assert_eq!(cfg.wait_max_ms, 30_000);
        assert_eq!(cfg.wait_rate_limited_ms, 130_000);
        assert_eq!(cfg.wait_server_restart_ms, 5_000);
        assert_eq!(cfg.jitter, JitterMode::Full);
        assert_eq!(cfg.replay_burst_size, 50);
        assert_eq!(cfg.replay_pace_ms, 5);
    }

    #[test]
    fn server_restart_window_covers_a_pool_bounce() {
        let limits = ReconnectAttemptLimits::default();
        let cfg = ReconnectConfig::default();
        // 60 attempts at an un-jittered 5 s cadence = 300 s window.
        let window_ms = u64::from(limits.max_server_restart_attempts) * cfg.wait_server_restart_ms;
        assert_eq!(window_ms, 300_000);
    }
}
