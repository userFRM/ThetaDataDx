//! FPSS reconnection sub-configuration.

use std::sync::Arc;
use std::time::Duration;

use tdbe::types::enums::RemoveReason;

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
///   backoff. Default budget 100 attempts at 130 s each, so the loop
///   keeps trying for up to ~3.6 h before giving up.
/// * **Generic** transient (TimedOut, ServerRestarting, Unspecified,
///   etc.) — short-patient backoff. Default budget 3 attempts at 2 s
///   each.
///
/// The counter resets after a configurable stable window of continuous
/// data flow (default 60 s) so a connection that runs cleanly for a
/// minute then drops gets the full retry budget again rather than
/// burning through one cycle's worth and falling off.
///
/// # Custom
///
/// Supply a closure that receives the disconnect reason and attempt number (1-based)
/// and returns `Some(delay)` to reconnect after that delay, or `None` to stop.
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
/// Splitting the cap by failure class fixes the previous behaviour
/// where rate-limited transients (`TooManyRequests`, 130 s spacing)
/// burned through the same 5-attempt budget as generic transients
/// (TimedOut / ServerRestarting, 2 s spacing) — the rate-limited path
/// gave up after ~10 minutes when the desired behaviour is to keep
/// trying for hours through a sustained throttle.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ReconnectAttemptLimits {
    /// Maximum consecutive reconnect attempts on a generic transient
    /// failure (TimedOut, ServerRestarting, Unspecified, …) before
    /// giving up. Default `3`.
    pub max_attempts: u32,

    /// Maximum consecutive reconnect attempts on a `TooManyRequests`
    /// rate-limited transient before giving up. Default `100` — at
    /// 130 s per attempt this absorbs ~3.6 h of sustained throttling
    /// without operator intervention.
    pub max_rate_limited_attempts: u32,

    /// Continuous successful-data-flow window after which the
    /// transient attempt counter resets. A connection that runs
    /// cleanly for at least this long picks up a fresh retry budget
    /// when it next drops. Default `60 s`.
    pub stable_window: Duration,
}

impl Default for ReconnectAttemptLimits {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            max_rate_limited_attempts: 100,
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
            _ => Some(ReconnectAttemptClass::Transient),
        }
    }

    /// Maximum attempts for the given attempt class.
    #[must_use]
    pub fn budget_for(&self, class: ReconnectAttemptClass) -> u32 {
        match class {
            ReconnectAttemptClass::Transient => self.max_attempts,
            ReconnectAttemptClass::RateLimited => self.max_rate_limited_attempts,
        }
    }
}

/// Reconnect attempt counter classification used by
/// [`ReconnectPolicy::Auto`]. Each class carries its own counter and
/// budget; the counters reset independently when the stable window
/// elapses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReconnectAttemptClass {
    /// Generic transient (TimedOut, ServerRestarting, Unspecified, …).
    Transient,
    /// `TooManyRequests` rate-limited transient.
    RateLimited,
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
    /// Delay before attempting reconnection after a disconnect, in milliseconds.
    ///
    /// Wire constant: `RECONNECT_DELAY_MS = 2000`. Note: `config_0.properties`
    /// has `RECONNECT_WAIT=1000` but the runtime uses the constant `2000`.
    ///
    /// Plumbed into the FPSS I/O loop through
    /// [`crate::fpss::FpssClientBuilder::reconnect_wait_ms`] at
    /// [`crate::ThetaDataDxClient::start_streaming`] /
    /// [`crate::ThetaDataDxClient::reconnect_streaming`] connect time —
    /// the [`ReconnectPolicy::Auto`] arm honours this value for generic
    /// transient drops (TimedOut, ServerRestarting, Unspecified, …) via
    /// [`crate::fpss::reconnect_delay_for`].
    pub wait_ms: u64,

    /// Delay before reconnecting after a `TooManyRequests` disconnect, in milliseconds.
    ///
    /// Involuntary-disconnect handler waits 130 seconds in this case.
    ///
    /// Plumbed into the FPSS I/O loop through
    /// [`crate::fpss::FpssClientBuilder::reconnect_wait_rate_limited_ms`]
    /// at connect time and honoured by the [`ReconnectPolicy::Auto`]
    /// arm via [`crate::fpss::reconnect_delay_for`] for
    /// `TooManyRequests` drops.
    pub wait_rate_limited_ms: u64,

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
            wait_ms: 2_000,
            wait_rate_limited_ms: 130_000,
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
    fn class_for_rate_limited_splits_from_generic() {
        assert_eq!(
            ReconnectAttemptLimits::class_for(RemoveReason::TooManyRequests),
            Some(ReconnectAttemptClass::RateLimited),
        );
        assert_eq!(
            ReconnectAttemptLimits::class_for(RemoveReason::TimedOut),
            Some(ReconnectAttemptClass::Transient),
        );
        assert_eq!(
            ReconnectAttemptLimits::class_for(RemoveReason::ServerRestarting),
            Some(ReconnectAttemptClass::Transient),
        );
        assert_eq!(
            ReconnectAttemptLimits::class_for(RemoveReason::Unspecified),
            Some(ReconnectAttemptClass::Transient),
        );
    }

    #[test]
    fn default_budgets_separate_rate_limited_from_generic() {
        let limits = ReconnectAttemptLimits::default();
        assert_eq!(limits.max_attempts, 3);
        assert_eq!(limits.max_rate_limited_attempts, 100);
        assert_eq!(limits.stable_window, Duration::from_secs(60));
        assert_eq!(limits.budget_for(ReconnectAttemptClass::Transient), 3);
        assert_eq!(
            limits.budget_for(ReconnectAttemptClass::RateLimited),
            100,
            "rate-limited budget must absorb sustained TooManyRequests"
        );
    }
}
