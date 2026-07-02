//! Streaming (TCP) sub-configuration.

/// Controls when the streaming write buffer is flushed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamingFlushMode {
    /// Flush only on PING frames. Lower syscall overhead, up to one
    /// ping interval of additional latency.
    #[default]
    Batched,
    /// Flush after every frame write. Lowest latency, higher syscall overhead.
    Immediate,
}

impl StreamingFlushMode {
    /// Canonical lowercase string for this mode, matching the
    /// cross-binding encoding.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Batched => "batched",
            Self::Immediate => "immediate",
        }
    }

    /// Parse the cross-binding string encoding (case-insensitive).
    /// Returns `None` for unrecognised input.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "batched" => Some(Self::Batched),
            "immediate" => Some(Self::Immediate),
            _ => None,
        }
    }
}

/// How the streaming client orders the configured streaming hosts for the
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
    /// The connection layer iterates through these on connection
    /// failure, in the order produced by [`Self::host_selection`].
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
    /// server may disconnect if it falls silent. Default `250` — the ping
    /// mainly proves write-side health at a 4 Hz cadence without adding
    /// inbound-frame pressure on a recovering upstream. Reverse-direction
    /// liveness is the inbound frame and ping stream (~250 ms on an active
    /// session), which [`Self::timeout_ms`] guards. Validated to the range
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

    /// Controls when the streaming write buffer is flushed.
    ///
    /// - [`StreamingFlushMode::Batched`] (default): only flush on PING frames.
    ///   Lower syscall overhead.
    /// - [`StreamingFlushMode::Immediate`]: flush after every frame write. Lowest
    ///   latency, higher syscall overhead.
    pub flush_mode: StreamingFlushMode,

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
            host_selection: HostSelectionPolicy::Shuffled,
            host_shuffle_seed: None,
            timeout_ms: 10_000,
            ring_size: 131_072,
            ping_interval_ms: 250,
            connect_timeout_ms: 2_000,
            io_read_slice_ms: 25,
            keepalive_idle_secs: 5,
            keepalive_interval_secs: 2,
            keepalive_retries: 2,
            flush_mode: StreamingFlushMode::Batched,
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
        assert_eq!(cfg.ping_interval_ms, 250);
        assert_eq!(cfg.io_read_slice_ms, 25);
        assert_eq!(cfg.keepalive_idle_secs, 5);
        assert_eq!(cfg.keepalive_interval_secs, 2);
        assert_eq!(cfg.keepalive_retries, 2);
        assert_eq!(cfg.host_selection, HostSelectionPolicy::Shuffled);
        assert_eq!(cfg.host_shuffle_seed, None);
        assert_eq!(cfg.consumer_cpu, None);
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
