//! Streaming (TCP) sub-configuration.

/// How the streaming event-ring consumer waits when the ring is
/// momentarily empty.
///
/// The choice is a CPU-vs-latency trade. [`Spin`](Self::Spin) and
/// [`BusySpin`](Self::BusySpin) **both hold ~100% of one core** whenever
/// the stream is connected — they differ only in scheduler jitter, not
/// CPU. Only [`Park`](Self::Park) and [`Backoff`](Self::Backoff) lower
/// idle CPU. If you are picking between `Spin` and `BusySpin` to save
/// CPU: neither does — use `Backoff` (automatic) or `Park` (fixed sleep).
///
/// Two further strategies were considered and deliberately left out: a
/// condvar-style `Block` (the ~100 ms FPSS ping keeps the ring active, so
/// blocking's edge over `Backoff` — 0% vs ~1% idle CPU, instant vs
/// interval wake — is marginal, and it would add lost-wakeup-prone
/// signalling on the producer hot path) and a pure `Yield` (redundant
/// with `Spin`).
///
/// Bindings encode the mode as the lowercase strings `"spin"`,
/// `"busyspin"`, `"park"`, and `"backoff"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WaitMode {
    /// Adaptive spin: a short busy-spin then a `yield_now` ramp, then a
    /// `spin_loop` hint — never sleeps. The default, and byte-for-byte the
    /// historical behaviour. The `yield_now` ramp lets the OS run other
    /// runnable threads between polls, so it is the friendlier of the two
    /// never-sleep modes on a shared core. ~100% of one core.
    #[default]
    Spin,
    /// Pure busy-spin: a single `spin_loop` hint per empty poll, no
    /// `yield_now`, no sleep — the tightest re-poll loop. Lowest and
    /// least-jittery delivery latency because the consumer never
    /// deschedules; in exchange it never yields the core, so pair it with
    /// a dedicated / isolated core (see [`StreamingConfig::consumer_cpu`]).
    /// Same ~100% of one core as `Spin`, just without the yield. For
    /// latency-critical consumers on dedicated hardware.
    BusySpin,
    /// Park: the same spin + `yield_now` ramp as `Spin`, then
    /// `thread::sleep(park_interval)` instead of spinning. Idle CPU drops
    /// to ~0-1% of a core; an event arriving while parked is delivered up
    /// to one park interval late (plus OS timer slack). The sleep length
    /// is [`StreamingConfig::park_interval_us`]. For consumers that idle
    /// through closed markets and accept a fixed latency floor even when
    /// active.
    Park,
    /// Backoff (automatic): spins and yields at full responsiveness while
    /// events are flowing, and after a short idle window with no events
    /// escalates to `thread::sleep(park_interval)` until events resume,
    /// then snaps back to spinning. Low latency when active, low CPU when
    /// idle, with no latency floor during active trading — the hands-free
    /// choice for a 24/7 consumer. Reuses
    /// [`StreamingConfig::park_interval_us`] as its idle sleep length.
    Backoff,
}

impl WaitMode {
    /// Canonical lowercase string for this mode, matching the
    /// cross-binding encoding.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spin => "spin",
            Self::BusySpin => "busyspin",
            Self::Park => "park",
            Self::Backoff => "backoff",
        }
    }

    /// Parse the cross-binding string encoding (case-insensitive).
    /// Returns `Option::None` for unrecognised input.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "spin" => Some(Self::Spin),
            "busyspin" => Some(Self::BusySpin),
            "park" => Some(Self::Park),
            "backoff" => Some(Self::Backoff),
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

    /// Consumer idle-wait mode for the streaming event ring.
    ///
    /// [`WaitMode::Spin`] (default) and [`WaitMode::BusySpin`] both hold
    /// ~100% of one core while connected and differ only in jitter;
    /// [`WaitMode::Park`] and [`WaitMode::Backoff`] lower idle CPU.
    /// `Backoff` is the hands-free low-latency-when-active /
    /// low-CPU-when-idle choice for a 24/7 consumer. Applied at connect
    /// time to both the unified `Client` dispatcher and a standalone
    /// `StreamingClient`. See [`WaitMode`].
    pub wait_mode: WaitMode,

    /// Sleep length, in microseconds, for [`WaitMode::Park`] and the idle
    /// phase of [`WaitMode::Backoff`]. Default `1000` (= 1 ms, keeping the
    /// historical park behaviour). Ignored by `Spin` / `BusySpin`.
    ///
    /// This is the worst-case delivery latency added to an event that
    /// arrives while the consumer is parked, and it bounds how many frames
    /// accumulate on the ring per wake. The OS timer honours sleeps down to
    /// ~50 us; below that, kernel timer slack dominates, so `50` is the
    /// validated floor. A `100` us park is a valid low-latency option —
    /// measured a few percent of a core in live premarket, well under the ~100% a spin holds — a bit
    /// more idle CPU than the `1000` us (1 ms) default, since a shorter
    /// interval wakes more often, for ~150 us wake latency.
    /// The client pings the server every ~100 ms
    /// ([`Self::ping_interval_ms`]), so the session is never idle much
    /// longer than that cadence: a park beyond ~100 ms (100_000 us) does
    /// not find a quieter ring, it just wakes to a larger backlog of
    /// control frames — no extra CPU saving, only more latency and higher
    /// ring occupancy. Validated to `[50, 1_000_000]` us: the `1_000_000`
    /// us (1 s) ceiling is a guardrail (it bounds worst-case latency to one
    /// second and keeps a park accidentally left on into active trading
    /// from overrunning the default `131_072`-slot ring before the next
    /// drain), not a recommendation — keep it at or below the ~100 ms ping
    /// cadence.
    pub park_interval_us: u64,
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
            wait_mode: WaitMode::Spin,
            park_interval_us: 1_000,
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
    /// Allowed range for [`super::StreamingConfig::park_interval_us`], in
    /// microseconds. The `50` us floor is the practical OS timer resolution
    /// (below it, kernel timer slack dominates); the `1_000_000` us (1 s)
    /// ceiling is a guardrail against a park left on into active trading
    /// (see the field docs), not a recommended value.
    pub const PARK_INTERVAL_US: std::ops::RangeInclusive<u64> = 50..=1_000_000;
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
        // Default wait mode is the historical low-latency spin, with the
        // park interval defaulted to 1000 us (= 1 ms) but unused until
        // Park/Backoff is selected.
        assert_eq!(cfg.wait_mode, WaitMode::Spin);
        assert_eq!(cfg.park_interval_us, 1_000);
        // Kernel-side half-open detection at the defaults:
        // idle + interval * retries = 5 + 2*2 = 9 seconds.
        let detection = cfg.keepalive_idle_secs
            + cfg.keepalive_interval_secs * u64::from(cfg.keepalive_retries);
        assert_eq!(detection, 9);
    }

    #[test]
    fn wait_mode_label_round_trip() {
        for mode in [
            WaitMode::Spin,
            WaitMode::BusySpin,
            WaitMode::Park,
            WaitMode::Backoff,
        ] {
            assert_eq!(WaitMode::parse(mode.as_str()), Some(mode));
        }
        // Case-insensitive, and unknown labels reject.
        assert_eq!(WaitMode::parse("BACKOFF"), Some(WaitMode::Backoff));
        assert_eq!(WaitMode::parse("block"), None);
    }
}
