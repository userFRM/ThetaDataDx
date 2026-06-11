//! FPSS (TCP streaming) sub-configuration.

/// Controls when the FPSS write buffer is flushed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FpssFlushMode {
    /// Flush only on PING frames. Lower syscall overhead, up to one
    /// ping interval of additional latency.
    #[default]
    Batched,
    /// Flush after every frame write. Lowest latency, higher syscall overhead.
    Immediate,
}

/// How the streaming client orders the configured FPSS hosts for the
/// initial connect and every reconnect.
///
/// The production host list spans two physical machines with two ports
/// each. Ordering decides both steady-state load placement (which host
/// a freshly-started client lands on) and failover behaviour (which
/// host a reconnecting client tries next).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostSelectionPolicy {
    /// Group hosts by hostname (fault domain), shuffle the group order
    /// and the ports within each group per client, then interleave
    /// across groups. Default.
    ///
    /// Two effects: a fleet of clients distributes uniformly across
    /// the fault domains instead of all dialling the first declared
    /// host, and consecutive failover attempts cross fault domains —
    /// the second attempt lands on a different physical machine, not a
    /// second port on the machine that just failed.
    #[default]
    Shuffled,
    /// Use the declared order verbatim. Escape hatch for deployments
    /// that pin traffic to a specific host for locality or compliance
    /// reasons.
    FixedOrder,
}

impl HostSelectionPolicy {
    /// Canonical lowercase string for this policy, matching the
    /// cross-binding encoding.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shuffled => "shuffled",
            Self::FixedOrder => "fixed_order",
        }
    }

    /// Parse the cross-binding string encoding (case-insensitive).
    /// Returns `None` for unrecognised input.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "shuffled" => Some(Self::Shuffled),
            "fixed_order" => Some(Self::FixedOrder),
            _ => None,
        }
    }
}

/// FPSS streaming client tuning.
///
/// The timing knobs (`timeout_ms`, `ping_interval_ms`,
/// `connect_timeout_ms`, `io_read_slice_ms`, `data_watchdog_ms`, the
/// keepalive trio) and `ring_size` are wired into the runtime: the
/// values flow through [`crate::fpss::FpssClientBuilder`] into the
/// connection, framing, and ping layers.
/// [`crate::DirectConfig::validate`] rejects out-of-range values before
/// the connect attempt.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FpssConfig {
    /// FPSS hosts.
    ///
    /// The connection layer iterates through these on connection
    /// failure, in the order produced by [`Self::host_selection`].
    /// Default: ThetaData's NJ `FPSS_NJ_HOSTS`.
    pub hosts: Vec<(String, u16)>,

    /// Per-client host ordering policy. Default
    /// [`HostSelectionPolicy::Shuffled`] — see the enum docs for the
    /// fleet-distribution and fault-domain-failover rationale.
    pub host_selection: HostSelectionPolicy,

    /// Optional seed for the [`HostSelectionPolicy::Shuffled`] order.
    ///
    /// `None` (default) derives a fresh per-client seed, so every
    /// client instance shuffles independently. Supplying a value makes
    /// the order deterministic — useful for fleet sharding (give each
    /// deployment slot a stable seed) and for tests that assert a
    /// specific order. Ignored under
    /// [`HostSelectionPolicy::FixedOrder`].
    pub host_shuffle_seed: Option<u64>,

    /// FPSS read timeout in milliseconds.
    ///
    /// Drives the per-connection initial socket read timeout, the framing
    /// layer's mid-frame stall budget, and the I/O loop's overall
    /// "no frames received" deadline that triggers
    /// [`tdbe::types::enums::RemoveReason::TimedOut`]. Default `3_000`
    /// — the server heartbeats every ~100 ms on a quiet session, so
    /// three seconds of total silence is ~30 missed heartbeats and a
    /// dead link is declared quickly instead of after the previous
    /// 10 s default. Validated to the range `[100, 60_000]` ms.
    pub timeout_ms: u64,

    /// FPSS event ring buffer size (slots).
    ///
    /// MUST be a power of two (the ring wraps the index with
    /// `i & (cap - 1)`) and at least `64`. `FpssClient::connect`
    /// returns [`crate::error::Error::Config`] on a non-power-of-two
    /// or below-minimum value — silent rounding is rejected so the
    /// caller's stated buffer budget is never rewritten under their
    /// feet. Larger rings absorb more burst traffic but use more
    /// memory (~`ring_size * sizeof(Option<FpssEvent>)`).
    pub ring_size: usize,

    /// FPSS heartbeat ping interval in milliseconds.
    ///
    /// The FPSS server expects a heartbeat at this cadence and may
    /// disconnect if it falls silent. Default `250` — the server's own
    /// ~100 ms heartbeat is the primary liveness signal in the
    /// reverse direction; the client ping mainly proves write-side
    /// health, and a 4 Hz cadence does that without contributing to
    /// inbound-frame pressure on a recovering upstream. Validated to
    /// the range `[100, 300_000]` ms — sub-100 ms values are rejected
    /// so a misconfiguration does not flood the upstream.
    pub ping_interval_ms: u64,

    /// Per-server TCP connect timeout in milliseconds. Default `2000`.
    ///
    /// Plumbed through [`std::net::TcpStream::connect_timeout`] in the
    /// connection layer so a slow / unreachable host fails fast and the
    /// next configured host gets a try. Validated to the range
    /// `[1_000, 60_000]` ms.
    pub connect_timeout_ms: u64,

    /// Per-iteration blocking-read slice (ms) for the streaming I/O
    /// loop. Default `25`.
    ///
    /// The I/O loop alternates between a short blocking read and a
    /// drain of the outbound command queue; this knob is the read
    /// slice. Shorter slices service outbound commands (subscribes,
    /// pings) more promptly at slightly higher idle CPU; longer slices
    /// trade latency for fewer wakeups. The overall no-frames deadline
    /// is [`Self::timeout_ms`], enforced on a wall clock — the slice
    /// size does not change detection accuracy. Validated to the range
    /// `[10, 500]` ms.
    pub io_read_slice_ms: u64,

    /// Last-frame watchdog (ms). Default `30_000`; `0` disables.
    ///
    /// A hard wall-clock backstop above the read timeout: when no
    /// frame of any kind (data, heartbeat, control) has arrived for
    /// this long, the I/O loop declares the connection dead and
    /// force-reconnects, regardless of the read-timeout slice
    /// accounting. With the default 3 s [`Self::timeout_ms`] the read
    /// timeout normally fires first; the watchdog matters for
    /// deployments that widen the read timeout, and it feeds the
    /// public staleness clock
    /// (`millis_since_last_event`) every binding exposes for
    /// operator-side monitoring.
    pub data_watchdog_ms: u64,

    /// TCP keepalive idle time (seconds) before the kernel sends the
    /// first probe on an otherwise-silent FPSS socket. Default `5`.
    /// Validated to `[1, 7_200]` s.
    ///
    /// Keepalive is transport-level defence-in-depth below the
    /// application-level read timeout: a peer that vanishes without a
    /// FIN/RST (kernel panic, NAT idle expiry, gateway restart) is
    /// detected by the kernel in roughly
    /// `idle + interval * retries` seconds (~9 s at the defaults)
    /// instead of the platform default of over two hours.
    pub keepalive_idle_secs: u64,

    /// Interval (seconds) between TCP keepalive probes once the idle
    /// threshold has passed without traffic. Default `2`. Validated to
    /// `[1, 75]` s.
    pub keepalive_interval_secs: u64,

    /// Number of unanswered TCP keepalive probes after which the
    /// kernel declares the connection dead. Default `2`. Validated to
    /// `[1, 10]`. Not configurable on every platform; where the OS
    /// does not expose the knob the idle/interval pair still applies.
    pub keepalive_retries: u32,

    /// Controls when the FPSS write buffer is flushed.
    ///
    /// - [`FpssFlushMode::Batched`] (default): only flush on PING frames.
    ///   Lower syscall overhead.
    /// - [`FpssFlushMode::Immediate`]: flush after every frame write. Lowest
    ///   latency, higher syscall overhead.
    pub flush_mode: FpssFlushMode,

    /// Whether to derive OHLCVC bars locally from trade events.
    ///
    /// When `true` (default), the FPSS client emits derived `FpssData::Ohlcvc`
    /// events after each trade. When `false`, only server-sent OHLCVC frames
    /// (wire code 24) are emitted, reducing per-trade throughput overhead.
    pub derive_ohlcvc: bool,
}

impl FpssConfig {
    /// Production defaults for ThetaData's NJ datacenter.
    #[must_use]
    pub fn production_defaults() -> Self {
        Self {
            hosts: vec![
                ("nj-a.thetadata.us".to_string(), 20000),
                ("nj-a.thetadata.us".to_string(), 20001),
                ("nj-b.thetadata.us".to_string(), 20000),
                ("nj-b.thetadata.us".to_string(), 20001),
            ],
            host_selection: HostSelectionPolicy::Shuffled,
            host_shuffle_seed: None,
            timeout_ms: 3_000,
            ring_size: 131_072,
            ping_interval_ms: 250,
            connect_timeout_ms: 2_000,
            io_read_slice_ms: 25,
            data_watchdog_ms: 30_000,
            keepalive_idle_secs: 5,
            keepalive_interval_secs: 2,
            keepalive_retries: 2,
            flush_mode: FpssFlushMode::Batched,
            derive_ohlcvc: true,
        }
    }
}

/// Validation bounds for the wired FPSS knobs. Out-of-range values
/// are rejected at config-load time by [`crate::config::DirectConfig::validate`].
pub mod bounds {
    /// Allowed range for [`super::FpssConfig::timeout_ms`], in milliseconds.
    pub const TIMEOUT_MS: std::ops::RangeInclusive<u64> = 100..=60_000;
    /// Allowed range for [`super::FpssConfig::connect_timeout_ms`], in milliseconds.
    pub const CONNECT_TIMEOUT_MS: std::ops::RangeInclusive<u64> = 1_000..=60_000;
    /// Allowed range for [`super::FpssConfig::ping_interval_ms`], in milliseconds.
    pub const PING_INTERVAL_MS: std::ops::RangeInclusive<u64> = 100..=300_000;
    /// Allowed range for [`super::FpssConfig::io_read_slice_ms`], in milliseconds.
    pub const IO_READ_SLICE_MS: std::ops::RangeInclusive<u64> = 10..=500;
    /// Allowed range for [`super::FpssConfig::keepalive_idle_secs`], in seconds.
    pub const KEEPALIVE_IDLE_SECS: std::ops::RangeInclusive<u64> = 1..=7_200;
    /// Allowed range for [`super::FpssConfig::keepalive_interval_secs`], in seconds.
    pub const KEEPALIVE_INTERVAL_SECS: std::ops::RangeInclusive<u64> = 1..=75;
    /// Allowed range for [`super::FpssConfig::keepalive_retries`].
    pub const KEEPALIVE_RETRIES: std::ops::RangeInclusive<u32> = 1..=10;
}

impl Default for FpssConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_defaults_resilience_shape() {
        let cfg = FpssConfig::production_defaults();
        assert_eq!(cfg.timeout_ms, 3_000);
        assert_eq!(cfg.ping_interval_ms, 250);
        assert_eq!(cfg.io_read_slice_ms, 25);
        assert_eq!(cfg.data_watchdog_ms, 30_000);
        assert_eq!(cfg.keepalive_idle_secs, 5);
        assert_eq!(cfg.keepalive_interval_secs, 2);
        assert_eq!(cfg.keepalive_retries, 2);
        assert_eq!(cfg.host_selection, HostSelectionPolicy::Shuffled);
        assert_eq!(cfg.host_shuffle_seed, None);
        // Kernel-side half-open detection at the defaults:
        // idle + interval * retries = 5 + 2*2 = 9 seconds.
        let detection = cfg.keepalive_idle_secs
            + cfg.keepalive_interval_secs * u64::from(cfg.keepalive_retries);
        assert_eq!(detection, 9);
    }

    #[test]
    fn host_selection_policy_string_round_trip() {
        for policy in [
            HostSelectionPolicy::Shuffled,
            HostSelectionPolicy::FixedOrder,
        ] {
            assert_eq!(HostSelectionPolicy::parse(policy.as_str()), Some(policy));
        }
        assert_eq!(
            HostSelectionPolicy::parse("SHUFFLED"),
            Some(HostSelectionPolicy::Shuffled)
        );
        assert_eq!(HostSelectionPolicy::parse("bogus"), None);
        assert_eq!(
            HostSelectionPolicy::default(),
            HostSelectionPolicy::Shuffled
        );
    }
}
