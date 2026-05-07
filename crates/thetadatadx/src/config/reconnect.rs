//! FPSS reconnection sub-configuration.

use std::sync::Arc;
use std::time::Duration;

use tdbe::types::enums::RemoveReason;

/// Controls FPSS reconnection behavior after a disconnect.
///
/// # Default
///
/// [`ReconnectPolicy::Auto`] matches the Java terminal's `handleInvoluntaryDisconnect()`:
/// permanent errors stop immediately, `TooManyRequests` waits 130s, everything else
/// waits 2s, up to 5 attempts.
///
/// # Custom
///
/// Supply a closure that receives the disconnect reason and attempt number (1-based)
/// and returns `Some(delay)` to reconnect after that delay, or `None` to stop.
#[derive(Clone, Default)]
pub enum ReconnectPolicy {
    /// Auto-reconnect matching Java terminal behavior (default).
    ///
    /// - Permanent errors (invalid credentials, account issues): no reconnect.
    /// - `TooManyRequests`: 130s wait.
    /// - All others: 2s wait.
    /// - Up to 5 consecutive reconnect attempts before giving up.
    #[default]
    Auto,
    /// No auto-reconnect. User calls `reconnect_streaming()` manually.
    Manual,
    /// User-provided function: `(reason, attempt_number) -> Option<Duration>`.
    ///
    /// Return `Some(delay)` to reconnect after `delay`, `None` to stop.
    /// `attempt_number` starts at 1 and increments on each consecutive reconnect.
    Custom(Arc<dyn Fn(RemoveReason, u32) -> Option<Duration> + Send + Sync>),
}

impl std::fmt::Debug for ReconnectPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "Auto"),
            Self::Manual => write!(f, "Manual"),
            Self::Custom(_) => write!(f, "Custom(...)"),
        }
    }
}

/// FPSS auto-reconnection cadence + policy.
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Delay before attempting reconnection after a disconnect, in milliseconds.
    ///
    /// Source: `FPSSClient.RECONNECT_DELAY_MS = 2000` in decompiled terminal.
    /// Note: `config_0.properties` has `RECONNECT_WAIT=1000` but the Java code
    /// uses the constant `2000` at runtime.
    ///
    /// NOTE: Not automatically wired — consumed by
    /// [`crate::ThetaDataDx::reconnect_streaming`] / the FPSS auto-reconnect path.
    pub wait_ms: u64,

    /// Delay before reconnecting after a `TooManyRequests` disconnect, in milliseconds.
    ///
    /// Source: `FPSSClient.handleInvoluntaryDisconnect()` — 130 second wait.
    ///
    /// NOTE: Not automatically wired — consumed by
    /// [`crate::ThetaDataDx::reconnect_streaming`] / the FPSS auto-reconnect path.
    pub wait_rate_limited_ms: u64,

    /// Controls FPSS auto-reconnection behavior after involuntary disconnect.
    ///
    /// Default: [`ReconnectPolicy::Auto`] — matches Java terminal behavior.
    pub policy: ReconnectPolicy,
}

impl ReconnectConfig {
    /// Production defaults — matches the Java terminal's reconnect cadence.
    #[must_use]
    pub fn production_defaults() -> Self {
        Self {
            wait_ms: 2_000,
            wait_rate_limited_ms: 130_000,
            policy: ReconnectPolicy::Auto,
        }
    }
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}
