//! FPSS (TCP streaming) sub-configuration.

/// Controls when the FPSS write buffer is flushed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FpssFlushMode {
    /// Flush only on PING frames (every 100ms). Matches Java terminal.
    /// Lower syscall overhead, up to 100ms additional latency.
    #[default]
    Batched,
    /// Flush after every frame write. Lowest latency, higher syscall overhead.
    Immediate,
}

/// FPSS streaming client tuning.
///
/// All four legacy `*_ms` knobs (`timeout_ms`, `ping_interval_ms`,
/// `connect_timeout_ms`) and the disruptor `ring_size` are wired into
/// the runtime: the values flow through [`crate::fpss::FpssConnectArgs`]
/// into [`crate::fpss::FpssClient::connect`], which threads them to the
/// connection, framing, and ping layers. [`crate::DirectConfig::validate`]
/// rejects out-of-range values before the connect attempt.
///
/// The pre-Disruptor `queue_depth` knob (a separate event channel size)
/// was removed in 9.1.0: the post-SSOT pipeline has exactly ONE queue
/// (the Disruptor ring), so `ring_size` is the single source of truth
/// for the streaming buffer budget.
#[derive(Debug, Clone)]
pub struct FpssConfig {
    /// FPSS TCP hosts with round-robin failover.
    ///
    /// Source: `FPSS_NJ_HOSTS` in `config_0.properties` — the terminal
    /// iterates through these on connection failure.
    pub hosts: Vec<(String, u16)>,

    /// FPSS read timeout in milliseconds.
    ///
    /// Source: `FPSS_TIMEOUT=10000` in `config_0.properties`. Drives
    /// the per-connection initial socket read timeout, the framing
    /// layer's mid-frame stall budget, and the I/O loop's overall
    /// "no data received" deadline that triggers
    /// [`tdbe::types::enums::RemoveReason::TimedOut`]. Validated to
    /// the range `[100, 60_000]` ms.
    pub timeout_ms: u64,

    /// FPSS disruptor ring buffer size (slots).
    ///
    /// MUST be a power of two (the Disruptor wraps the index with
    /// `i & (cap - 1)`) and at least `64`. `FpssClient::connect`
    /// returns [`crate::error::Error::Config`] on a non-power-of-two
    /// or below-minimum value — silent rounding is rejected so the
    /// caller's stated buffer budget is never rewritten under their
    /// feet. Larger rings absorb more burst traffic but use more
    /// memory (~`ring_size * sizeof(Option<FpssEvent>)`).
    pub ring_size: usize,

    /// FPSS heartbeat ping interval in milliseconds.
    ///
    /// Default `100` matches the Java terminal's `scheduleAtFixedRate`
    /// cadence; the FPSS server expects a heartbeat at this rhythm and
    /// may disconnect if it falls silent. Validated to the range
    /// `[100, 300_000]` ms — sub-100 ms values are rejected so a
    /// misconfiguration does not flood the upstream.
    pub ping_interval_ms: u64,

    /// Per-server TCP connect timeout in milliseconds. Default `2000`.
    ///
    /// Plumbed through [`std::net::TcpStream::connect_timeout`] in the
    /// connection layer so a slow / unreachable host fails fast and the
    /// next configured host gets a try. Validated to the range
    /// `[1_000, 60_000]` ms.
    pub connect_timeout_ms: u64,

    /// Controls when the FPSS write buffer is flushed.
    ///
    /// - [`FpssFlushMode::Batched`] (default): only flush on PING frames (~100ms).
    ///   Lower syscall overhead.
    /// - [`FpssFlushMode::Immediate`]: flush after every frame write. Lowest
    ///   latency, higher syscall overhead.
    pub flush_mode: FpssFlushMode,

    /// Whether to derive OHLCVC bars locally from trade events.
    ///
    /// When `true` (default), the FPSS client emits derived `FpssData::Ohlcvc`
    /// events after each trade. When `false`, only server-sent OHLCVC frames
    /// (wire code 24) are emitted, reducing per-trade throughput overhead.
    ///
    /// The Java terminal always derives OHLCVC with no way to disable it.
    pub derive_ohlcvc: bool,
}

impl FpssConfig {
    /// Production defaults — extracted from the decompiled Java terminal.
    #[must_use]
    pub fn production_defaults() -> Self {
        Self {
            hosts: vec![
                ("nj-a.thetadata.us".to_string(), 20000),
                ("nj-a.thetadata.us".to_string(), 20001),
                ("nj-b.thetadata.us".to_string(), 20000),
                ("nj-b.thetadata.us".to_string(), 20001),
            ],
            timeout_ms: 10_000,
            ring_size: 131_072,
            ping_interval_ms: 100,
            connect_timeout_ms: 2_000,
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
}

impl Default for FpssConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}
