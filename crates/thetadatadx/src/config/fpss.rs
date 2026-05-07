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
#[derive(Debug, Clone)]
pub struct FpssConfig {
    /// FPSS TCP hosts with round-robin failover.
    ///
    /// Source: `FPSS_NJ_HOSTS` in `config_0.properties` — the terminal
    /// iterates through these on connection failure.
    pub hosts: Vec<(String, u16)>,

    /// FPSS connection/read timeout in milliseconds.
    ///
    /// Source: `FPSS_TIMEOUT=10000` in `config_0.properties`.
    pub timeout_ms: u64,

    /// FPSS event channel buffer depth.
    /// Caller should pass this to `FpssClient::connect(creds, queue_depth)`.
    /// Increase if stream events are being dropped under high volume.
    ///
    /// JVM equivalent: `FPSS_QUEUE_DEPTH=1000000` in `config_0.properties`.
    ///
    /// NOTE: Not automatically wired — caller must pass to `FpssClient::connect()`.
    pub queue_depth: usize,

    /// FPSS disruptor ring buffer size (slots, will be rounded up to a power of 2).
    ///
    /// The LMAX Disruptor ring buffer used for lock-free event dispatch requires
    /// a power-of-2 size. This value is rounded up automatically. Larger rings
    /// absorb more burst traffic but use more memory (~`ring_size * sizeof(Option<FpssEvent>)`).
    ///
    /// Derived from `queue_depth` by default. Override for fine-grained control.
    pub ring_size: usize,

    /// FPSS heartbeat ping interval in milliseconds.
    /// The protocol requires pings every 100ms; changing this may cause disconnects.
    ///
    /// NOTE: Not automatically wired — the ping loop uses `protocol::PING_INTERVAL_MS`.
    /// Override that constant or pass this value when a configurable ping loop is added.
    pub ping_interval_ms: u64,

    /// Per-server TCP connect timeout in milliseconds. Default `2000`.
    ///
    /// NOTE: Not automatically wired — the connection module uses `protocol::CONNECT_TIMEOUT_MS`.
    /// Override that constant or pass this value when a configurable connect is added.
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
            queue_depth: 1_000_000,
            ring_size: 131_072,
            ping_interval_ms: 100,
            connect_timeout_ms: 2_000,
            flush_mode: FpssFlushMode::Batched,
            derive_ohlcvc: true,
        }
    }
}

impl Default for FpssConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}
