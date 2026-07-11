//! Streaming (TCP) sub-configuration.

/// Streaming client tuning.
///
/// The timing knobs (`timeout_ms`, `ping_interval_ms`,
/// `connect_timeout_ms`, `io_read_slice_ms`, the
/// keepalive trio) and `ring_size` are wired into the runtime: the
/// values flow through [`crate::fpss::StreamingClientBuilder`] into the
/// connection, framing, and ping layers.
/// [`crate::DirectConfig::validate`] rejects out-of-range values before
/// the connect attempt.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StreamingConfig {
    /// Streaming hosts.
    ///
    /// The connection layer iterates through these in declared order on
    /// connection failure (a reconnect tries the last-known-good host
    /// first, then the rest in order).
    /// Default: ThetaData's NJ `FPSS_NJ_HOSTS`.
    ///
    /// Set through [`DirectConfig::set_streaming_hosts`] so the write is
    /// recorded as an explicit full-list override that survives environment
    /// selection; read through [`DirectConfig::streaming_hosts`]. The field
    /// is crate-private so the only way to point the streaming channel at a
    /// host set is the tracked setter — there is no untracked direct-write
    /// path for environment selection to second-guess.
    ///
    /// [`DirectConfig::set_streaming_hosts`]: crate::config::DirectConfig::set_streaming_hosts
    /// [`DirectConfig::streaming_hosts`]: crate::config::DirectConfig::streaming_hosts
    pub(crate) hosts: Vec<(String, u16)>,

    /// Streaming read timeout in milliseconds.
    ///
    /// Drives the per-connection initial socket read timeout, the framing
    /// layer's mid-frame stall budget, and the I/O loop's overall
    /// "no frames received" deadline that triggers
    /// [`crate::RemoveReason::TimedOut`]. Default `10_000`, matching the
    /// terminal's streaming socket read timeout. Right after a
    /// full-market subscribe the server can fall fully silent (no
    /// frames, no pings) for a few seconds while it sets the
    /// subscription up; a 10 s deadline rides through that gap where a
    /// shorter one trips inside it and forces an unnecessary reconnect.
    /// The ~100 ms cadence is the client's ping to the server, not an
    /// inbound heartbeat; inbound frames and pings arrive roughly every
    /// ~250 ms on an active session. Validated to the range
    /// `[100, 60_000]` ms.
    pub timeout_ms: u64,

    /// Streaming event ring buffer size (slots).
    ///
    /// MUST be a power of two (the ring wraps the index with
    /// `i & (cap - 1)`) and at least `64`. `StreamingClient::connect`
    /// returns [`crate::error::Error::Config`] on a non-power-of-two
    /// or below-minimum value — silent rounding is rejected so the
    /// caller's stated buffer budget is never rewritten under their
    /// feet. Larger rings absorb more burst traffic but use more
    /// memory (~`ring_size * sizeof(Option<StreamEvent>)`).
    pub ring_size: usize,

    /// Streaming heartbeat ping interval in milliseconds.
    ///
    /// This is the client's outbound ping cadence to the server, and the
    /// server may disconnect if it falls silent. Default `100`, matching
    /// the terminal's pinger (a fixed 100 ms period after a 2 s warm-up).
    /// The ping also flushes any queued outbound control frames, so this
    /// interval bounds subscribe / unsubscribe latency. Reverse-direction
    /// liveness is the inbound frame stream, which [`Self::timeout_ms`]
    /// guards. Validated to the range `[100, 300_000]` ms — sub-100 ms
    /// values are rejected so a misconfiguration does not flood the upstream.
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

    /// TCP keepalive idle time (seconds) before the kernel sends the
    /// first probe on an otherwise-silent streaming socket. Default `5`.
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

    /// Optional CPU core to pin the streaming event-ring consumer thread
    /// to.
    ///
    /// `None` (default) leaves the consumer under the OS scheduler — the
    /// historical behaviour. `Some(core_id)` pins the tick-consumer
    /// thread to that core for deterministic, low-jitter delivery; pair
    /// with an isolated core (e.g. `isolcpus`) for best results. An
    /// out-of-range or offline core is a best-effort no-op at the
    /// affinity layer (a `warn` is logged) rather than a hard error.
    pub consumer_cpu: Option<usize>,
}

impl StreamingConfig {
    /// Streaming host list.
    ///
    /// Read accessor for the crate-private [`Self::hosts`] field. The host
    /// set is written through [`DirectConfig::set_streaming_hosts`] so a
    /// caller-supplied list is recorded as a tracked override; this getter
    /// is the supported way to read it back (including from the SDK
    /// bindings, which snapshot a [`StreamingConfig`]).
    ///
    /// [`DirectConfig::set_streaming_hosts`]: crate::config::DirectConfig::set_streaming_hosts
    #[must_use]
    pub fn hosts(&self) -> &[(String, u16)] {
        &self.hosts
    }

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
            timeout_ms: 10_000,
            ring_size: 131_072,
            ping_interval_ms: 100,
            connect_timeout_ms: 2_000,
            io_read_slice_ms: 25,
            keepalive_idle_secs: 5,
            keepalive_interval_secs: 2,
            keepalive_retries: 2,
            consumer_cpu: None,
        }
    }
}

/// Validation bounds for the wired streaming knobs. Out-of-range values
/// are rejected at config-load time by [`crate::config::DirectConfig::validate`].
pub mod bounds {
    /// Allowed range for [`super::StreamingConfig::timeout_ms`], in milliseconds.
    pub const TIMEOUT_MS: std::ops::RangeInclusive<u64> = 100..=60_000;
    /// Allowed range for [`super::StreamingConfig::connect_timeout_ms`], in milliseconds.
    pub const CONNECT_TIMEOUT_MS: std::ops::RangeInclusive<u64> = 1_000..=60_000;
    /// Allowed range for [`super::StreamingConfig::ping_interval_ms`], in milliseconds.
    pub const PING_INTERVAL_MS: std::ops::RangeInclusive<u64> = 100..=300_000;
    /// Allowed range for [`super::StreamingConfig::io_read_slice_ms`], in milliseconds.
    pub const IO_READ_SLICE_MS: std::ops::RangeInclusive<u64> = 10..=500;
    /// Allowed range for [`super::StreamingConfig::keepalive_idle_secs`], in seconds.
    pub const KEEPALIVE_IDLE_SECS: std::ops::RangeInclusive<u64> = 1..=7_200;
    /// Allowed range for [`super::StreamingConfig::keepalive_interval_secs`], in seconds.
    pub const KEEPALIVE_INTERVAL_SECS: std::ops::RangeInclusive<u64> = 1..=75;
    /// Allowed range for [`super::StreamingConfig::keepalive_retries`].
    pub const KEEPALIVE_RETRIES: std::ops::RangeInclusive<u32> = 1..=10;
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_defaults_resilience_shape() {
        let cfg = StreamingConfig::production_defaults();
        assert_eq!(cfg.timeout_ms, 10_000);
        assert_eq!(cfg.ping_interval_ms, 100);
        assert_eq!(cfg.io_read_slice_ms, 25);
        assert_eq!(cfg.keepalive_idle_secs, 5);
        assert_eq!(cfg.keepalive_interval_secs, 2);
        assert_eq!(cfg.keepalive_retries, 2);
        assert_eq!(cfg.consumer_cpu, None);
        // Kernel-side half-open detection at the defaults:
        // idle + interval * retries = 5 + 2*2 = 9 seconds.
        let detection = cfg.keepalive_idle_secs
            + cfg.keepalive_interval_secs * u64::from(cfg.keepalive_retries);
        assert_eq!(detection, 9);
    }
}
