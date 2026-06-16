//! Server configuration for direct `ThetaData` access.
//!
//! # Server topology
//!
//! `ThetaData` runs two server types in their NJ datacenter:
//!
//! ## Historical — Market Data Distribution Server (historical data)
//!
//! Historical requests connect to a single endpoint over TLS:
//! ```text
//! mdds-01.thetadata.us:443
//! ```
//!
//! ## Streaming — Feed Processing Stream Server (real-time streaming)
//!
//! Streaming uses a multi-host config with round-robin failover:
//! ```text
//! FPSS_NJ_HOSTS=nj-a.thetadata.us:20000,nj-a.thetadata.us:20001,
//!               nj-b.thetadata.us:20000,nj-b.thetadata.us:20001
//! ```
//!
//! The connection layer iterates through configured hosts on connection failure.
//!
//! # Layout
//!
//! [`DirectConfig`] is composed of eight nested sub-configs:
//!
//! | Field           | Type                                                |
//! |-----------------|-----------------------------------------------------|
//! | `historical`    | [`HistoricalConfig`] — gRPC host/port/TLS/keepalive       |
//! | `streaming`     | [`StreamingConfig`] — TCP hosts, queue/ring, flush mode  |
//! | `flatfiles`     | [`FlatFilesConfig`] — FLATFILES retry budget        |
//! | `reconnect`     | [`ReconnectConfig`] — wait cadence + policy         |
//! | `retry`         | [`RetryPolicy`] — exponential backoff for historical |
//! | `auth`          | [`AuthConfig`] — Nexus URL + `client_type`          |
//! | `metrics`       | [`MetricsConfig`] — Prometheus exporter port        |
//! | `runtime`       | [`RuntimeConfig`] — tokio worker thread sizing      |

mod auth;
mod env;
mod flatfiles;
mod fpss;
mod mdds;
mod metrics;
mod reconnect;
mod retry;
mod runtime;

use crate::error::Error;

pub use auth::{AuthConfig, DEFAULT_CLIENT_TYPE, DEFAULT_NEXUS_URL};
pub use env::{
    ENV_CLIENT_TYPE, ENV_HISTORICAL_HOST, ENV_HISTORICAL_PORT, ENV_NEXUS_URL, ENV_STREAMING_HOST,
    ENV_STREAMING_PORT,
};
pub use flatfiles::{bounds as flatfiles_bounds, FlatFilesConfig};
pub use fpss::{
    bounds as streaming_bounds, HostSelectionPolicy, StreamingConfig, StreamingFlushMode,
    StreamingWaitStrategy,
};
pub use mdds::HistoricalConfig;
pub use metrics::MetricsConfig;
pub use reconnect::{
    ReconnectAttemptClass, ReconnectAttemptLimits, ReconnectConfig, ReconnectPolicy,
    RATE_LIMITED_JITTER_WINDOW,
};
pub use retry::RetryPolicy;
pub use runtime::RuntimeConfig;

pub use crate::backoff::JitterMode;

/// Configuration for connecting to `ThetaData` servers directly.
///
/// Use [`DirectConfig::production()`] for the standard NJ production servers.
///
/// # Layout
///
/// Fields are grouped into eight nested sub-configs ([`HistoricalConfig`],
/// [`StreamingConfig`], [`FlatFilesConfig`], [`ReconnectConfig`], [`RetryPolicy`],
/// [`AuthConfig`], [`MetricsConfig`], [`RuntimeConfig`]). Read accessors on [`DirectConfig`]
/// preserve the field-style naming used by older callers; writes go through
/// the nested struct (e.g. `cfg.streaming.ring_size = N`).
///
/// # Environment variable overrides
///
/// [`DirectConfig::production()`] reads the following environment variables
/// and applies them on top of the hardcoded defaults. Explicit builder
/// setters (`.with_metrics_port(...)` etc.) take precedence over env vars,
/// which in turn take precedence over the hardcoded defaults.
///
/// | Variable | Type | Effect |
/// |---|---|---|
/// | `THETADATA_HISTORICAL_HOST` | host | overrides `historical.host` |
/// | `THETADATA_HISTORICAL_PORT` | u16  | overrides `historical.port` |
/// | `THETADATA_NEXUS_URL` | url  | overrides the Nexus auth URL |
/// | `THETADATA_STREAMING_HOST` | host | overrides the primary streaming host |
/// | `THETADATA_STREAMING_PORT` | u16  | overrides the primary streaming port |
/// | `THETADATA_CLIENT_TYPE` | str | overrides `auth.client_type` |
/// | `THETADATA_EMAIL`       | str | credential helper ([`crate::auth`]) |
/// | `THETADATA_PASSWORD`    | str | credential helper ([`crate::auth`]) |
///
/// Malformed values (e.g. a non-integer `THETADATA_HISTORICAL_PORT`) are ignored
/// with a `tracing::warn!` — the hardcoded default is retained so a typo
/// in the environment never silently breaks production.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct DirectConfig {
    /// Historical tuning.
    pub historical: HistoricalConfig,
    /// Streaming tuning.
    pub streaming: StreamingConfig,
    /// FLATFILES retry tuning.
    pub flatfiles: FlatFilesConfig,
    /// Reconnection cadence + policy.
    pub reconnect: ReconnectConfig,
    /// Historical retry policy.
    pub retry: RetryPolicy,
    /// Nexus auth endpoint + client type.
    pub auth: AuthConfig,
    /// Prometheus exporter binding.
    pub metrics: MetricsConfig,
    /// Async runtime tuning.
    pub runtime: RuntimeConfig,
}

impl DirectConfig {
    /// Default Nexus auth URL (matches the upstream production endpoint).
    pub const DEFAULT_NEXUS_URL: &'static str = DEFAULT_NEXUS_URL;

    /// Default `QueryInfo.client_type`.
    pub const DEFAULT_CLIENT_TYPE: &'static str = DEFAULT_CLIENT_TYPE;

    /// Production configuration for `ThetaData`'s NJ datacenter.
    ///
    /// - Historical: `mdds-01.thetadata.us:443` (TLS)
    /// - Streaming: 4 NJ hosts with round-robin failover (`FPSS_NJ_HOSTS`)
    /// - Timeouts: matched to ThetaData's published connection parameters
    ///
    /// Environment variables listed on [`DirectConfig`] are layered on
    /// top of these defaults.
    ///
    /// # Panics
    ///
    /// Panics if the resulting configuration fails [`Self::validate`].
    /// The hardcoded defaults are always in range, so this fires only
    /// when an environment override pushes a knob out of bounds.
    #[must_use]
    pub fn production() -> Self {
        let mut config = Self::production_defaults();
        env::apply_env_overrides(&mut config);
        config
            .validate()
            .expect("production defaults are within validated bounds")
    }

    /// Production defaults without env-var overrides. Tests use this to
    /// assert the hardcoded shape in isolation; every caller that wants
    /// env-var precedence should reach for [`DirectConfig::production`].
    #[must_use]
    pub(crate) fn production_defaults() -> Self {
        Self {
            historical: HistoricalConfig::production_defaults(),
            streaming: StreamingConfig::production_defaults(),
            flatfiles: FlatFilesConfig::production_defaults(),
            reconnect: ReconnectConfig::production_defaults(),
            retry: RetryPolicy::default(),
            auth: AuthConfig::production_defaults(),
            metrics: MetricsConfig::default(),
            runtime: RuntimeConfig::default(),
        }
    }

    /// Dev streaming configuration.
    ///
    /// Connects to `ThetaData`'s dev streaming servers (port 20200) which replay
    /// a random historical trading day in an infinite loop at maximum speed.
    /// Designed for development and testing when markets are closed.
    ///
    /// Historical data still uses production servers -- there is no dev historical.
    ///
    /// Source: `config.toml` `fpss_dev_hosts` and
    /// <https://docs.thetadata.us/Streaming/Getting-Started.html>
    ///
    /// Note: dev server replays data at max speed, so queue and ring sizes
    /// match production to avoid drops. Some contracts may not exist on
    /// the replayed day.
    ///
    /// # Panics
    ///
    /// Panics if the preset fails [`Self::validate`] — only reachable
    /// when an environment override pushes a knob out of bounds, since
    /// the preset's own values are in range.
    #[must_use]
    pub fn dev() -> Self {
        let mut config = Self::production();
        // Source: config.toml fpss_dev_hosts
        config.streaming.hosts = vec![
            ("nj-a.thetadata.us".to_string(), 20200),
            ("test-server.thetadata.us".to_string(), 20200),
            ("test-server.thetadata.us".to_string(), 20201),
        ];
        config
            .validate()
            .expect("dev preset is within validated bounds")
    }

    /// Stage streaming configuration.
    ///
    /// Connects to `ThetaData`'s staging streaming servers (port 20100).
    /// Frequent reboots, testing data. Not stable.
    ///
    /// Historical data still uses production servers.
    ///
    /// Source: `config.toml` `fpss_stage_hosts`
    ///
    /// # Panics
    ///
    /// Panics if the preset fails [`Self::validate`] — only reachable
    /// when an environment override pushes a knob out of bounds, since
    /// the preset's own values are in range.
    #[must_use]
    pub fn stage() -> Self {
        let mut config = Self::production();
        // Source: config.toml fpss_stage_hosts
        config.streaming.hosts = vec![
            ("nj-a.thetadata.us".to_string(), 20100),
            ("test-server.thetadata.us".to_string(), 20100),
            ("test-server.thetadata.us".to_string(), 20101),
        ];
        config
            .validate()
            .expect("stage preset is within validated bounds")
    }

    /// Validate configuration values and reject out-of-range tuning knobs.
    ///
    /// Returns the configuration with historical HTTP/2 window sizes clamped
    /// into `[64, 1024]` KB on success. Returns
    /// [`Error::Config`] when any wired streaming
    /// knob (`timeout_ms`, `connect_timeout_ms`, `ping_interval_ms`)
    /// falls outside its documented range — silent rounding would
    /// rewrite the caller's stated tuning under their feet, so an
    /// invalid value is reported up front instead.
    ///
    /// Called automatically by [`production()`](Self::production),
    /// [`dev()`](Self::dev), and [`stage()`](Self::stage). Also useful
    /// after loading from a TOML file or modifying fields
    /// programmatically.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when an streaming
    /// timing knob is out of range.
    pub fn validate(mut self) -> Result<Self, Error> {
        // u64 → i64: every bound fits comfortably under i64::MAX (max
        // bound is 300_000 ms). `saturating_cast` would be overkill;
        // a checked `try_from` documents the invariant.
        let to_i64 = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        if !streaming_bounds::TIMEOUT_MS.contains(&self.streaming.timeout_ms) {
            return Err(Error::config_out_of_range(
                "streaming.timeout_ms",
                to_i64(self.streaming.timeout_ms),
                to_i64(*streaming_bounds::TIMEOUT_MS.start()),
                to_i64(*streaming_bounds::TIMEOUT_MS.end()),
            ));
        }
        if !streaming_bounds::CONNECT_TIMEOUT_MS.contains(&self.streaming.connect_timeout_ms) {
            return Err(Error::config_out_of_range(
                "streaming.connect_timeout_ms",
                to_i64(self.streaming.connect_timeout_ms),
                to_i64(*streaming_bounds::CONNECT_TIMEOUT_MS.start()),
                to_i64(*streaming_bounds::CONNECT_TIMEOUT_MS.end()),
            ));
        }
        if !streaming_bounds::PING_INTERVAL_MS.contains(&self.streaming.ping_interval_ms) {
            return Err(Error::config_out_of_range(
                "streaming.ping_interval_ms",
                to_i64(self.streaming.ping_interval_ms),
                to_i64(*streaming_bounds::PING_INTERVAL_MS.start()),
                to_i64(*streaming_bounds::PING_INTERVAL_MS.end()),
            ));
        }
        if !streaming_bounds::IO_READ_SLICE_MS.contains(&self.streaming.io_read_slice_ms) {
            return Err(Error::config_out_of_range(
                "streaming.io_read_slice_ms",
                to_i64(self.streaming.io_read_slice_ms),
                to_i64(*streaming_bounds::IO_READ_SLICE_MS.start()),
                to_i64(*streaming_bounds::IO_READ_SLICE_MS.end()),
            ));
        }
        if !streaming_bounds::KEEPALIVE_IDLE_SECS.contains(&self.streaming.keepalive_idle_secs) {
            return Err(Error::config_out_of_range(
                "streaming.keepalive_idle_secs",
                to_i64(self.streaming.keepalive_idle_secs),
                to_i64(*streaming_bounds::KEEPALIVE_IDLE_SECS.start()),
                to_i64(*streaming_bounds::KEEPALIVE_IDLE_SECS.end()),
            ));
        }
        if !streaming_bounds::KEEPALIVE_INTERVAL_SECS
            .contains(&self.streaming.keepalive_interval_secs)
        {
            return Err(Error::config_out_of_range(
                "streaming.keepalive_interval_secs",
                to_i64(self.streaming.keepalive_interval_secs),
                to_i64(*streaming_bounds::KEEPALIVE_INTERVAL_SECS.start()),
                to_i64(*streaming_bounds::KEEPALIVE_INTERVAL_SECS.end()),
            ));
        }
        if !streaming_bounds::KEEPALIVE_RETRIES.contains(&self.streaming.keepalive_retries) {
            return Err(Error::config_out_of_range(
                "streaming.keepalive_retries",
                i64::from(self.streaming.keepalive_retries),
                i64::from(*streaming_bounds::KEEPALIVE_RETRIES.start()),
                i64::from(*streaming_bounds::KEEPALIVE_RETRIES.end()),
            ));
        }
        if !streaming_bounds::WAIT_SPIN_ITERS.contains(&self.streaming.wait_spin_iters) {
            return Err(Error::config_out_of_range(
                "streaming.wait_spin_iters",
                i64::from(self.streaming.wait_spin_iters),
                i64::from(*streaming_bounds::WAIT_SPIN_ITERS.start()),
                i64::from(*streaming_bounds::WAIT_SPIN_ITERS.end()),
            ));
        }
        if !streaming_bounds::WAIT_YIELD_ITERS.contains(&self.streaming.wait_yield_iters) {
            return Err(Error::config_out_of_range(
                "streaming.wait_yield_iters",
                i64::from(self.streaming.wait_yield_iters),
                i64::from(*streaming_bounds::WAIT_YIELD_ITERS.start()),
                i64::from(*streaming_bounds::WAIT_YIELD_ITERS.end()),
            ));
        }
        if !streaming_bounds::WAIT_PARK_US.contains(&self.streaming.wait_park_us) {
            return Err(Error::config_out_of_range(
                "streaming.wait_park_us",
                to_i64(self.streaming.wait_park_us),
                to_i64(*streaming_bounds::WAIT_PARK_US.start()),
                to_i64(*streaming_bounds::WAIT_PARK_US.end()),
            ));
        }
        if self.reconnect.replay_burst_size == 0 {
            return Err(Error::config_invalid(
                "reconnect.replay_burst_size",
                "replay_burst_size must be at least 1".to_string(),
            ));
        }
        if self.reconnect.wait_max_ms < self.reconnect.wait_ms {
            return Err(Error::config_invalid(
                "reconnect.wait_max_ms",
                format!(
                    "wait_max_ms ({}) must be >= wait_ms ({})",
                    self.reconnect.wait_max_ms, self.reconnect.wait_ms
                ),
            ));
        }
        // Validate ring_size eagerly so a bad config fails fast rather
        // than waiting for the connect attempt. Re-validation happens
        // at `StreamingClient::connect` for callers that bypass `validate`.
        if let Err(e) = crate::fpss::ring::check_ring_size(self.streaming.ring_size) {
            return Err(Error::config_invalid("streaming.ring_size", e.to_string()));
        }
        self.historical.window_size_kb = self.historical.window_size_kb.clamp(64, 1_024);
        self.historical.connection_window_size_kb =
            self.historical.connection_window_size_kb.clamp(64, 1_024);
        if !flatfiles_bounds::MAX_ATTEMPTS.contains(&self.flatfiles.max_attempts) {
            return Err(Error::config_out_of_range(
                "flatfiles.max_attempts",
                i64::from(self.flatfiles.max_attempts),
                i64::from(*flatfiles_bounds::MAX_ATTEMPTS.start()),
                i64::from(*flatfiles_bounds::MAX_ATTEMPTS.end()),
            ));
        }
        if self.flatfiles.max_backoff < self.flatfiles.initial_backoff {
            return Err(Error::config_invalid(
                "flatfiles.max_backoff",
                format!(
                    "max_backoff ({:?}) must be >= initial_backoff ({:?})",
                    self.flatfiles.max_backoff, self.flatfiles.initial_backoff
                ),
            ));
        }
        Ok(self)
    }

    /// Build the historical endpoint URI.
    ///
    /// Returns the gRPC base URI for the historical service.
    #[must_use]
    pub fn historical_uri(&self) -> String {
        let scheme = if self.historical.tls { "https" } else { "http" };
        format!(
            "{}://{}:{}",
            scheme, self.historical.host, self.historical.port
        )
    }

    /// Set whether to derive OHLCVC bars locally from trade events.
    ///
    /// When `false`, only server-sent OHLCVC frames are emitted,
    /// reducing per-trade throughput overhead.
    #[must_use]
    pub fn derive_ohlcvc(mut self, enabled: bool) -> Self {
        self.streaming.derive_ohlcvc = enabled;
        self
    }

    /// Set the port the Prometheus exporter should bind to when the
    /// `metrics-prometheus` cargo feature is enabled. The exporter
    /// exposes `/metrics` over HTTP on `0.0.0.0:<port>`.
    #[must_use]
    pub fn with_metrics_port(mut self, port: u16) -> Self {
        self.metrics.port = Some(port);
        self
    }

    /// Override the retry policy for transient gRPC errors.
    #[must_use]
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry = policy;
        self
    }

    /// Override the Nexus auth URL. Intended for staging deployments —
    /// production should use [`ENV_NEXUS_URL`] or the default.
    #[must_use]
    pub fn with_nexus_url(mut self, url: impl Into<String>) -> Self {
        self.auth.nexus_url = url.into();
        self
    }

    /// Override `QueryInfo.client_type`. Appears in server-side logs
    /// and dashboards; useful for tagging a deployment fleet.
    #[must_use]
    pub fn with_client_type(mut self, client_type: impl Into<String>) -> Self {
        self.auth.client_type = client_type.into();
        self
    }

    /// Parse streaming hosts from a comma-separated `host:port,host:port,...` string.
    ///
    /// This is the format used in `config_0.properties` for `FPSS_NJ_HOSTS`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when an entry lacks a `host:port` split,
    /// when a port does not parse as a `u16`, or when the input yields no
    /// hosts at all.
    pub fn parse_streaming_hosts(hosts_str: &str) -> Result<Vec<(String, u16)>, Error> {
        let mut result = Vec::new();

        for entry in hosts_str.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }

            let (host, port_str) = entry.rsplit_once(':').ok_or_else(|| {
                Error::config_invalid(
                    "streaming.hosts",
                    format!("invalid host:port entry: '{entry}'"),
                )
            })?;

            let port: u16 = port_str.parse().map_err(|e| {
                Error::config_invalid("streaming.hosts", format!("invalid port in '{entry}': {e}"))
            })?;

            result.push((host.to_string(), port));
        }

        if result.is_empty() {
            return Err(Error::config_missing("streaming.hosts"));
        }

        Ok(result)
    }
}

// ── Read accessors (back-compat for the old flat field names) ────────────
//
// External callers that still spell config reads as `config.historical_host(...)`
// should call these accessor methods. Field-syntax reads (`config.historical_host`)
// no longer compile and must migrate to the nested form
// (`config.historical.host`); see the commit body for the migration table.
impl DirectConfig {
    /// Historical hostname.
    #[must_use]
    pub fn historical_host(&self) -> &str {
        &self.historical.host
    }
    /// Historical port.
    #[must_use]
    pub fn historical_port(&self) -> u16 {
        self.historical.port
    }
    /// Whether historical uses TLS.
    #[must_use]
    pub fn historical_tls(&self) -> bool {
        self.historical.tls
    }
    /// Historical concurrent in-flight requests budget.
    #[must_use]
    pub fn historical_concurrent_requests(&self) -> usize {
        self.historical.concurrent_requests
    }
    /// Historical max inbound message size, in bytes.
    #[must_use]
    pub fn historical_max_message_size(&self) -> usize {
        self.historical.max_message_size
    }
    /// Historical keepalive ping interval, in seconds.
    #[must_use]
    pub fn historical_keepalive_secs(&self) -> u64 {
        self.historical.keepalive_secs
    }
    /// Historical keepalive ping timeout, in seconds.
    #[must_use]
    pub fn historical_keepalive_timeout_secs(&self) -> u64 {
        self.historical.keepalive_timeout_secs
    }
    /// Historical HTTP/2 stream window size, in KB.
    #[must_use]
    pub fn historical_window_size_kb(&self) -> usize {
        self.historical.window_size_kb
    }
    /// Historical HTTP/2 connection window size, in KB.
    #[must_use]
    pub fn historical_connection_window_size_kb(&self) -> usize {
        self.historical.connection_window_size_kb
    }
    /// Historical TCP connect timeout, in seconds.
    #[must_use]
    pub fn historical_connect_timeout_secs(&self) -> u64 {
        self.historical.connect_timeout_secs
    }

    /// Streaming host list.
    #[must_use]
    pub fn streaming_hosts(&self) -> &[(String, u16)] {
        &self.streaming.hosts
    }
    /// Streaming read timeout, in milliseconds.
    #[must_use]
    pub fn streaming_timeout_ms(&self) -> u64 {
        self.streaming.timeout_ms
    }
    /// Streaming event ring buffer size.
    #[must_use]
    pub fn streaming_ring_size(&self) -> usize {
        self.streaming.ring_size
    }
    /// Streaming heartbeat ping interval, in milliseconds.
    #[must_use]
    pub fn streaming_ping_interval_ms(&self) -> u64 {
        self.streaming.ping_interval_ms
    }
    /// Streaming connect timeout, in milliseconds.
    #[must_use]
    pub fn streaming_connect_timeout_ms(&self) -> u64 {
        self.streaming.connect_timeout_ms
    }
    /// Streaming write-buffer flush mode.
    #[must_use]
    pub fn streaming_flush_mode(&self) -> StreamingFlushMode {
        self.streaming.flush_mode
    }
    /// Streaming event-ring consumer wait strategy.
    #[must_use]
    pub fn streaming_wait_strategy(&self) -> StreamingWaitStrategy {
        self.streaming.wait_strategy
    }
    /// Optional CPU core the streaming consumer thread is pinned to;
    /// `None` leaves it under the OS scheduler.
    #[must_use]
    pub fn streaming_consumer_cpu(&self) -> Option<usize> {
        self.streaming.consumer_cpu
    }
    /// Whether to derive OHLCVC bars locally from trade events.
    #[must_use]
    pub fn derive_ohlcvc_enabled(&self) -> bool {
        self.streaming.derive_ohlcvc
    }

    /// Streaming reconnect wait, in milliseconds.
    #[must_use]
    pub fn reconnect_wait_ms(&self) -> u64 {
        self.reconnect.wait_ms
    }
    /// Streaming reconnect wait after `TooManyRequests`, in milliseconds.
    #[must_use]
    pub fn reconnect_wait_rate_limited_ms(&self) -> u64 {
        self.reconnect.wait_rate_limited_ms
    }
    /// Streaming reconnect policy.
    #[must_use]
    pub fn reconnect_policy(&self) -> &ReconnectPolicy {
        &self.reconnect.policy
    }

    /// Historical retry policy.
    #[must_use]
    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry
    }

    /// Nexus auth URL.
    #[must_use]
    pub fn nexus_url(&self) -> &str {
        &self.auth.nexus_url
    }
    /// `QueryInfo.client_type` value.
    #[must_use]
    pub fn client_type(&self) -> &str {
        &self.auth.client_type
    }

    /// Prometheus exporter port (`None` disables the exporter).
    #[must_use]
    pub fn metrics_port(&self) -> Option<u16> {
        self.metrics.port
    }

    /// Tokio worker thread count (`None` = tokio default).
    #[must_use]
    pub fn tokio_worker_threads(&self) -> Option<usize> {
        self.runtime.tokio_worker_threads
    }
}

// ── Config file loading (behind `config-file` feature) ──────────────────────

#[cfg(feature = "config-file")]
mod config_file {
    use super::{
        DirectConfig, ReconnectAttemptLimits, ReconnectPolicy, RetryPolicy, StreamingFlushMode,
        StreamingWaitStrategy,
    };
    use crate::error::Error;
    use serde::Deserialize;

    /// TOML-level representation of the config file.
    ///
    /// An unknown key or section is rejected (`#[serde(deny_unknown_fields)]`)
    /// so a misspelled knob surfaces as a load error instead of silently
    /// running the default. Missing sections fall back to production
    /// defaults (`#[serde(default)]` on each section).
    #[derive(Debug, Default, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct ConfigFile {
        historical: MddsSection,
        streaming: FpssSection,
        grpc: GrpcSection,
    }

    #[derive(Debug, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct MddsSection {
        host: String,
        port: u16,
        tls: bool,
        keepalive_time_secs: u64,
        keepalive_timeout_secs: u64,
        max_message_size: usize,
    }

    impl Default for MddsSection {
        fn default() -> Self {
            let prod = DirectConfig::production();
            Self {
                host: prod.historical.host,
                port: prod.historical.port,
                tls: prod.historical.tls,
                keepalive_time_secs: prod.historical.keepalive_secs,
                keepalive_timeout_secs: prod.historical.keepalive_timeout_secs,
                max_message_size: prod.historical.max_message_size,
            }
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct FpssSection {
        /// Hosts as `["host:port", ...]` array or `"host:port,host:port"` string.
        hosts: FpssHosts,
        connect_timeout: u64,
        read_timeout: u64,
        ping_interval: u64,
        reconnect_wait: u64,
        reconnect_wait_rate_limited: u64,
        ring_size: usize,
        flush_mode: String,
        wait_strategy: String,
        wait_spin_iters: u32,
        wait_yield_iters: u32,
        wait_park_us: u64,
        /// CPU core to pin the streaming consumer thread to. A negative
        /// value (the default `-1`) means "unpinned" — the `Option::None`
        /// form in `[streaming]` TOML, where serde cannot express a bare
        /// optional cleanly under `#[serde(default)]`.
        consumer_cpu: i64,
    }

    impl Default for FpssSection {
        fn default() -> Self {
            let prod = DirectConfig::production();
            Self {
                hosts: FpssHosts::Array(
                    prod.streaming
                        .hosts
                        .iter()
                        .map(|(h, p)| format!("{h}:{p}"))
                        .collect(),
                ),
                connect_timeout: prod.streaming.connect_timeout_ms,
                read_timeout: prod.streaming.timeout_ms,
                ping_interval: prod.streaming.ping_interval_ms,
                reconnect_wait: prod.reconnect.wait_ms,
                reconnect_wait_rate_limited: prod.reconnect.wait_rate_limited_ms,
                ring_size: prod.streaming.ring_size,
                flush_mode: "batched".to_string(),
                wait_strategy: prod.streaming.wait_strategy.as_str().to_string(),
                wait_spin_iters: prod.streaming.wait_spin_iters,
                wait_yield_iters: prod.streaming.wait_yield_iters,
                wait_park_us: prod.streaming.wait_park_us,
                consumer_cpu: prod
                    .streaming
                    .consumer_cpu
                    .and_then(|c| i64::try_from(c).ok())
                    .unwrap_or(-1),
            }
        }
    }

    /// Streaming hosts can be specified as either a TOML array or a comma-separated string.
    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    enum FpssHosts {
        Array(Vec<String>),
        Csv(String),
    }

    impl Default for FpssHosts {
        fn default() -> Self {
            let prod = DirectConfig::production();
            FpssHosts::Array(
                prod.streaming
                    .hosts
                    .iter()
                    .map(|(h, p)| format!("{h}:{p}"))
                    .collect(),
            )
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct GrpcSection {
        window_size_kb: usize,
        connection_window_size_kb: usize,
        /// Max inbound message size, in MB. `None` (key absent) leaves the
        /// `[historical].max_message_size` byte value in force; an explicit
        /// value here — including the default of `4` — overrides it. Kept
        /// distinguishable from "absent" via `Option` so setting the
        /// override to the same number as the default is still honoured as
        /// an explicit choice rather than read as unset.
        max_message_size_mb: Option<usize>,
        concurrent_requests: usize,
    }

    impl Default for GrpcSection {
        fn default() -> Self {
            let prod = DirectConfig::production();
            Self {
                window_size_kb: prod.historical.window_size_kb,
                connection_window_size_kb: prod.historical.connection_window_size_kb,
                // Absent by default so `[historical].max_message_size`
                // remains the single source of truth unless the operator
                // sets this MB-denominated override explicitly.
                max_message_size_mb: None,
                concurrent_requests: prod.historical.concurrent_requests,
            }
        }
    }

    impl FpssHosts {
        fn parse(self) -> Result<Vec<(String, u16)>, Error> {
            let entries = match self {
                FpssHosts::Array(arr) => arr,
                FpssHosts::Csv(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
            };
            let mut result = Vec::new();
            for entry in entries {
                let entry = entry.trim();
                if entry.is_empty() {
                    continue;
                }
                let (host, port_str) = entry.rsplit_once(':').ok_or_else(|| {
                    Error::config_invalid(
                        "streaming.hosts",
                        format!("invalid host:port entry: '{entry}'"),
                    )
                })?;
                let port: u16 = port_str.parse().map_err(|e| {
                    Error::config_invalid(
                        "streaming.hosts",
                        format!("invalid port in '{entry}': {e}"),
                    )
                })?;
                result.push((host.to_string(), port));
            }
            if result.is_empty() {
                return Err(Error::config_missing("streaming.hosts"));
            }
            Ok(result)
        }
    }

    impl DirectConfig {
        /// Load configuration from a TOML file.
        ///
        /// The file format matches `config.default.toml` shipped with the crate.
        /// Missing sections and keys fall back to [`DirectConfig::production()`] defaults.
        /// An unknown key or section is rejected so a typo surfaces as a load
        /// error instead of silently running the default.
        ///
        /// # Example file
        ///
        /// ```toml
        /// [historical]
        /// host = "mdds-01.thetadata.us"
        /// port = 443
        /// tls = true
        ///
        /// [streaming]
        /// hosts = ["nj-a.thetadata.us:20000", "nj-b.thetadata.us:20000"]
        /// reconnect_wait = 2000
        /// ring_size = 131072
        /// flush_mode = "batched"  # or "immediate"
        ///
        /// [grpc]
        /// window_size_kb = 64
        /// connection_window_size_kb = 64
        /// concurrent_requests = 0  # 0 = auto from tier
        /// ```
        ///
        /// # Errors
        ///
        /// Returns [`Error::Config`] when the file cannot be read, when its
        /// contents are not valid TOML, or when the parsed values fail
        /// [`Self::validate`].
        pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
            let contents = std::fs::read_to_string(path.as_ref()).map_err(|e| {
                Error::config_io(format!(
                    "failed to read config file '{}': {e}",
                    path.as_ref().display()
                ))
            })?;
            Self::from_toml_str(&contents)
        }

        /// Parse configuration from a TOML string.
        ///
        /// Same semantics as [`from_file`](Self::from_file) but takes a string directly.
        ///
        /// # Errors
        ///
        /// Returns [`Error::Config`] when the string is not valid TOML or
        /// when the parsed values fail [`Self::validate`].
        pub fn from_toml_str(toml_str: &str) -> Result<Self, Error> {
            let cf: ConfigFile =
                toml::from_str(toml_str).map_err(|e| Error::config_toml(e.to_string()))?;

            // An empty / absent value takes the documented default; any
            // other unrecognized value is a misconfiguration and is
            // rejected by name rather than silently falling back, so a
            // typo cannot quietly run a mode the operator did not pick.
            let flush_mode = match cf.streaming.flush_mode.trim().to_lowercase().as_str() {
                "" | "batched" => StreamingFlushMode::Batched,
                "immediate" => StreamingFlushMode::Immediate,
                other => {
                    return Err(Error::config_invalid(
                        "streaming.flush_mode",
                        format!(
                            "flush_mode must be one of \"batched\", \"immediate\"; got {other:?}"
                        ),
                    ));
                }
            };

            // Same contract as flush_mode: empty / absent takes the
            // default; an unrecognized value is reported by name with the
            // allowed set rather than silently defaulting.
            let wait_strategy = if cf.streaming.wait_strategy.trim().is_empty() {
                StreamingWaitStrategy::default()
            } else {
                cf.streaming
                    .wait_strategy
                    .parse::<StreamingWaitStrategy>()?
            };

            // `[historical].max_message_size` (bytes) is the canonical knob.
            // `[grpc].max_message_size_mb` (MB) is an explicit override that
            // wins when present — including when set to the same number as
            // the default — and is inert when absent.
            let max_message_size = match cf.grpc.max_message_size_mb {
                Some(mb) => mb * 1024 * 1024,
                None => cf.historical.max_message_size,
            };

            let mut out = DirectConfig::production_defaults();
            out.historical.host = cf.historical.host;
            out.historical.port = cf.historical.port;
            out.historical.tls = cf.historical.tls;
            out.historical.concurrent_requests = cf.grpc.concurrent_requests;
            out.historical.max_message_size = max_message_size;
            out.historical.keepalive_secs = cf.historical.keepalive_time_secs;
            out.historical.keepalive_timeout_secs = cf.historical.keepalive_timeout_secs;
            out.historical.window_size_kb = cf.grpc.window_size_kb;
            out.historical.connection_window_size_kb = cf.grpc.connection_window_size_kb;
            // mdds.connect_timeout_secs is not yet TOML-surfaced; keep production default.

            out.streaming.hosts = cf.streaming.hosts.parse()?;
            out.streaming.timeout_ms = cf.streaming.read_timeout;
            out.streaming.ring_size = cf.streaming.ring_size;
            out.streaming.ping_interval_ms = cf.streaming.ping_interval;
            out.streaming.connect_timeout_ms = cf.streaming.connect_timeout;
            out.streaming.flush_mode = flush_mode;
            out.streaming.wait_strategy = wait_strategy;
            out.streaming.wait_spin_iters = cf.streaming.wait_spin_iters;
            out.streaming.wait_yield_iters = cf.streaming.wait_yield_iters;
            out.streaming.wait_park_us = cf.streaming.wait_park_us;
            // A negative TOML `consumer_cpu` (default `-1`) means unpinned.
            out.streaming.consumer_cpu = usize::try_from(cf.streaming.consumer_cpu).ok();
            // Default: derive OHLCVC from trades (matches production default).
            // Use the builder API to disable programmatically.
            out.streaming.derive_ohlcvc = true;

            out.reconnect.wait_ms = cf.streaming.reconnect_wait;
            out.reconnect.wait_rate_limited_ms = cf.streaming.reconnect_wait_rate_limited;
            // TOML config cannot express custom closures; default to Auto.
            // Use the builder API to set Manual or Custom programmatically.
            out.reconnect.policy = ReconnectPolicy::Auto(ReconnectAttemptLimits::default());

            // TOML does not surface RetryPolicy / observability fields
            // today — the builder API (`with_retry_policy`,
            // `with_metrics_port`, env vars) is the opt-in path.
            out.retry = RetryPolicy::default();
            out.auth.nexus_url = DirectConfig::DEFAULT_NEXUS_URL.to_string();
            out.auth.client_type = DirectConfig::DEFAULT_CLIENT_TYPE.to_string();
            out.metrics.port = None;
            out.runtime.tokio_worker_threads = None;

            out.validate()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_historical_uri() {
        // `DirectConfig::production()` reads `THETADATA_HISTORICAL_*` env
        // vars; another test in this module (`env_overrides_apply_on_production`)
        // mutates the same env via `unsafe`, and the env is process-
        // global. Acquire the shared test guard so the two cannot
        // race when `cargo test` runs them in parallel.
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.historical_uri(), "https://mdds-01.thetadata.us:443");
    }

    #[test]
    fn production_has_four_streaming_hosts() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.streaming.hosts.len(), 4);
    }

    #[test]
    fn production_default_reconnect_policy_is_auto() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert!(matches!(config.reconnect.policy, ReconnectPolicy::Auto(_)));
    }

    #[test]
    fn production_historical_connect_timeout_default_is_ten_seconds() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.historical.connect_timeout_secs, 10);
    }

    #[test]
    fn read_accessors_match_nested_fields() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.historical_host(), config.historical.host.as_str());
        assert_eq!(config.streaming_ring_size(), config.streaming.ring_size);
        assert_eq!(config.metrics_port(), config.metrics.port);
        assert_eq!(
            config.tokio_worker_threads(),
            config.runtime.tokio_worker_threads
        );
        assert_eq!(config.nexus_url(), config.auth.nexus_url.as_str());
    }

    #[test]
    fn parse_streaming_hosts_parses_multi_host_csv_with_whitespace_and_empty_entries() {
        let hosts = DirectConfig::parse_streaming_hosts(
            " nj-a.thetadata.us:20000, ,nj-b.thetadata.us:20001 ",
        )
        .unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0], ("nj-a.thetadata.us".to_string(), 20000));
        assert_eq!(hosts[1], ("nj-b.thetadata.us".to_string(), 20001));
    }

    #[test]
    fn parse_streaming_hosts_rejects_malformed_entries() {
        assert!(DirectConfig::parse_streaming_hosts("").is_err());
        assert!(DirectConfig::parse_streaming_hosts("host:notaport").is_err());
        assert!(DirectConfig::parse_streaming_hosts("hostonly").is_err());
    }

    // -- Config file tests (only compiled with the `config-file` feature) --

    #[cfg(feature = "config-file")]
    mod config_file_tests {
        use crate::config::{DirectConfig, StreamingFlushMode, StreamingWaitStrategy};

        #[test]
        fn empty_toml_gives_production_defaults() {
            let config = DirectConfig::from_toml_str("").unwrap();
            let prod = DirectConfig::production();
            assert_eq!(config.historical.host, prod.historical.host);
            assert_eq!(config.historical.port, prod.historical.port);
            assert_eq!(config.streaming.hosts.len(), prod.streaming.hosts.len());
            assert_eq!(config.streaming.ring_size, prod.streaming.ring_size);
        }

        #[test]
        fn partial_toml_overrides_only_specified() {
            let toml = r#"
                [historical]
                host = "custom.example.com"
                port = 8443

                [streaming]
                ring_size = 65536
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.host, "custom.example.com");
            assert_eq!(config.historical.port, 8443);
            assert_eq!(config.streaming.ring_size, 65536);
            // Unspecified fields keep production defaults
            assert!(config.historical.tls);
        }

        #[test]
        fn streaming_hosts_as_array() {
            let toml = r#"
                [streaming]
                hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.streaming.hosts.len(), 2);
            assert_eq!(
                config.streaming.hosts[0],
                ("host-a.example.com".to_string(), 20000)
            );
            assert_eq!(
                config.streaming.hosts[1],
                ("host-b.example.com".to_string(), 20001)
            );
        }

        #[test]
        fn streaming_hosts_as_csv_string() {
            let toml = r#"
                [streaming]
                hosts = "host-a.example.com:20000,host-b.example.com:20001"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.streaming.hosts.len(), 2);
            assert_eq!(config.streaming.hosts[0].0, "host-a.example.com");
        }

        #[test]
        fn flush_mode_immediate() {
            let toml = r#"
                [streaming]
                flush_mode = "immediate"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.streaming.flush_mode, StreamingFlushMode::Immediate);
        }

        #[test]
        fn flush_mode_batched_by_default() {
            let toml = r#"
                [streaming]
                flush_mode = "batched"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.streaming.flush_mode, StreamingFlushMode::Batched);
        }

        #[test]
        fn wait_strategy_and_tuning_round_trip() {
            let toml = r#"
                [streaming]
                wait_strategy = "balanced"
                wait_spin_iters = 16
                wait_yield_iters = 2
                wait_park_us = 75
                consumer_cpu = 3
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(
                config.streaming.wait_strategy,
                StreamingWaitStrategy::Balanced
            );
            assert_eq!(config.streaming.wait_spin_iters, 16);
            assert_eq!(config.streaming.wait_yield_iters, 2);
            assert_eq!(config.streaming.wait_park_us, 75);
            assert_eq!(config.streaming.consumer_cpu, Some(3));
        }

        #[test]
        fn wait_strategy_defaults_low_latency_unpinned() {
            // An empty streaming section keeps the low-latency default
            // and leaves the consumer unpinned.
            let toml = r#"
                [streaming]
                ring_size = 4096
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(
                config.streaming.wait_strategy,
                StreamingWaitStrategy::LowLatency
            );
            assert_eq!(config.streaming.consumer_cpu, None);
        }

        #[test]
        fn negative_consumer_cpu_means_unpinned() {
            let toml = r#"
                [streaming]
                consumer_cpu = -1
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.streaming.consumer_cpu, None);
        }

        #[test]
        fn grpc_section_sets_window_sizes() {
            let toml = r#"
                [grpc]
                window_size_kb = 128
                connection_window_size_kb = 256
                concurrent_requests = 4
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.window_size_kb, 128);
            assert_eq!(config.historical.connection_window_size_kb, 256);
            assert_eq!(config.historical.concurrent_requests, 4);
        }

        #[test]
        fn grpc_max_message_size_mb_overrides_historical_bytes() {
            let toml = r#"
                [grpc]
                max_message_size_mb = 8
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.max_message_size, 8 * 1024 * 1024);
        }

        #[test]
        fn unknown_field_is_rejected() {
            // A misspelled knob (here `ring_size` -> `ringsize`) must
            // surface as a load error rather than parsing fine and
            // silently running the default.
            let toml = r#"
                [streaming]
                ringsize = 65536
            "#;
            let err =
                DirectConfig::from_toml_str(toml).expect_err("a misspelled field must be rejected");
            assert!(err.to_string().contains("ringsize"), "{err}");
        }

        #[test]
        fn unknown_section_is_rejected() {
            let toml = r#"
                [some_unknown_section]
                foo = "bar"
            "#;
            let err =
                DirectConfig::from_toml_str(toml).expect_err("an unknown section must be rejected");
            assert!(err.to_string().contains("some_unknown_section"), "{err}");
        }

        #[test]
        fn dead_auth_section_is_rejected() {
            // The credential path is not part of this tuning config;
            // credentials load via the credential API. A `[auth]` block
            // here is a dead knob and must be rejected, not silently
            // accepted as a no-op.
            let toml = r#"
                [auth]
                creds_file = "creds.txt"
            "#;
            let err = DirectConfig::from_toml_str(toml)
                .expect_err("the removed [auth] section must be rejected");
            assert!(err.to_string().contains("auth"), "{err}");
        }

        #[test]
        fn bad_flush_mode_is_rejected() {
            let toml = r#"
                [streaming]
                flush_mode = "imediate"
            "#;
            let err = DirectConfig::from_toml_str(toml)
                .expect_err("a misspelled flush_mode must be rejected");
            let msg = err.to_string();
            assert!(msg.contains("flush_mode"), "{msg}");
            assert!(msg.contains("imediate"), "{msg}");
        }

        #[test]
        fn bad_wait_strategy_is_rejected() {
            let toml = r#"
                [streaming]
                wait_strategy = "lowlatency"
            "#;
            let err = DirectConfig::from_toml_str(toml)
                .expect_err("a misspelled wait_strategy must be rejected");
            let msg = err.to_string();
            assert!(msg.contains("wait_strategy"), "{msg}");
            assert!(msg.contains("lowlatency"), "{msg}");
        }

        #[test]
        fn grpc_max_message_size_mb_default_value_is_honored_when_explicit() {
            // Explicitly setting the override to the same number as the
            // production default (4 MB) must still take effect as an
            // explicit choice — "set to 4" is distinguishable from "absent".
            let toml = r#"
                [historical]
                max_message_size = 8388608

                [grpc]
                max_message_size_mb = 4
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.max_message_size, 4 * 1024 * 1024);
        }

        #[test]
        fn grpc_max_message_size_absent_keeps_historical_bytes() {
            // With no [grpc] override the canonical [historical] byte value
            // stays in force.
            let toml = r#"
                [historical]
                max_message_size = 8388608
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.max_message_size, 8 * 1024 * 1024);
        }

        #[test]
        fn full_config_default_toml_parses() {
            // Validate that config.default.toml (shipped with the crate) can be parsed.
            let default_toml = include_str!("../../config.default.toml");
            let config = DirectConfig::from_toml_str(default_toml).unwrap();
            assert_eq!(config.historical.host, "mdds-01.thetadata.us");
            assert_eq!(config.historical.port, 443);
            assert_eq!(config.streaming.hosts.len(), 4);
        }

        #[test]
        fn config_default_toml_uses_canonical_section_names() {
            // The deserializer binds `[historical]` / `[streaming]`; the
            // internal vendor names `[mdds]` / `[fpss]` deserialize to
            // nothing, so a sample shipping them silently discards every
            // override. Asserting host==443 above can't catch that (those
            // equal the production defaults), so pin the section names in
            // the shipped sample directly.
            let default_toml = include_str!("../../config.default.toml");
            assert!(
                default_toml.contains("[historical]"),
                "config.default.toml must use the canonical [historical] section"
            );
            assert!(
                default_toml.contains("[streaming]"),
                "config.default.toml must use the canonical [streaming] section"
            );
            assert!(
                !default_toml.contains("[mdds]") && !default_toml.contains("[fpss]"),
                "config.default.toml must not ship the dead [mdds]/[fpss] section names"
            );
        }

        #[test]
        fn invalid_toml_returns_error() {
            let result = DirectConfig::from_toml_str("this is not valid toml [[[");
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("TOML"));
        }
    }

    // -- Validation tests --

    #[test]
    fn validate_clamps_historical_window_sizes_into_range() {
        let mut config = DirectConfig::production_defaults();
        config.historical.window_size_kb = 2_048;
        config.historical.connection_window_size_kb = 32;
        let config = config
            .validate()
            .expect("historical window sizes are clamped");
        assert_eq!(config.historical.window_size_kb, 1_024);
        assert_eq!(config.historical.connection_window_size_kb, 64);
    }

    #[test]
    fn validate_preserves_in_range_values() {
        let config = DirectConfig::production_defaults();
        let validated = config.validate().expect("production defaults validate");
        assert_eq!(validated.historical.window_size_kb, 64);
        assert_eq!(validated.streaming.timeout_ms, 3_000);
        assert_eq!(validated.streaming.ping_interval_ms, 250);
        assert_eq!(validated.streaming.connect_timeout_ms, 2_000);
        assert_eq!(validated.streaming.io_read_slice_ms, 25);
        assert_eq!(validated.streaming.data_watchdog_ms, 30_000);
        assert_eq!(validated.streaming.keepalive_idle_secs, 5);
        assert_eq!(validated.streaming.keepalive_interval_secs, 2);
        assert_eq!(validated.streaming.keepalive_retries, 2);
        assert_eq!(validated.reconnect.wait_ms, 250);
        assert_eq!(validated.reconnect.wait_max_ms, 30_000);
        assert_eq!(validated.reconnect.replay_burst_size, 50);
        assert_eq!(validated.reconnect.replay_pace_ms, 5);
    }

    #[test]
    fn validate_rejects_io_read_slice_out_of_range() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.io_read_slice_ms = 5;
        let err = config.validate().expect_err("must reject below-minimum");
        assert!(err.to_string().contains("io_read_slice_ms"));
    }

    #[test]
    fn validate_rejects_keepalive_out_of_range() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.keepalive_idle_secs = 0;
        let err = config.validate().expect_err("must reject zero idle");
        assert!(err.to_string().contains("keepalive_idle_secs"));

        let mut config = DirectConfig::production_defaults();
        config.streaming.keepalive_interval_secs = 80;
        let err = config
            .validate()
            .expect_err("must reject oversize interval");
        assert!(err.to_string().contains("keepalive_interval_secs"));

        let mut config = DirectConfig::production_defaults();
        config.streaming.keepalive_retries = 0;
        let err = config.validate().expect_err("must reject zero retries");
        assert!(err.to_string().contains("keepalive_retries"));
    }

    #[test]
    fn validate_rejects_degenerate_replay_and_ladder() {
        let mut config = DirectConfig::production_defaults();
        config.reconnect.replay_burst_size = 0;
        let err = config.validate().expect_err("must reject zero burst");
        assert!(err.to_string().contains("replay_burst_size"));

        let mut config = DirectConfig::production_defaults();
        config.reconnect.wait_ms = 60_000;
        config.reconnect.wait_max_ms = 1_000;
        let err = config.validate().expect_err("must reject inverted ladder");
        assert!(err.to_string().contains("wait_max_ms"));
    }

    #[test]
    fn validate_rejects_streaming_timeout_below_minimum() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.timeout_ms = 50;
        let err = config.validate().expect_err("must reject below-minimum");
        let msg = err.to_string();
        assert!(msg.contains("timeout_ms"), "{msg}");
    }

    #[test]
    fn validate_rejects_streaming_timeout_above_maximum() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.timeout_ms = 120_000;
        let err = config.validate().expect_err("must reject above-maximum");
        assert!(err.to_string().contains("timeout_ms"));
    }

    #[test]
    fn validate_rejects_streaming_connect_timeout_out_of_range() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.connect_timeout_ms = 100;
        let err = config.validate().expect_err("100ms is below 1s minimum");
        assert!(err.to_string().contains("connect_timeout_ms"));
    }

    #[test]
    fn validate_rejects_streaming_ping_interval_out_of_range() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.ping_interval_ms = 50;
        let err = config.validate().expect_err("50ms below 100ms minimum");
        assert!(err.to_string().contains("ping_interval_ms"));
    }

    #[test]
    fn validate_rejects_invalid_ring_size() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.ring_size = 100; // not a power of two
        let err = config.validate().expect_err("must reject non-power-of-two");
        assert!(err.to_string().contains("ring_size"));
    }

    #[test]
    fn historical_defaults_match_production_baseline() {
        let mdds = crate::config::HistoricalConfig::production_defaults();
        // Tier clamp on by default — the override is an internal
        // escape hatch only enabled by tests that need to reproduce
        // the over-provisioning failure mode.
        assert!(!mdds.override_tier_clamp);
    }

    // ── RetryPolicy / env var tests ──────────────────────────────────

    #[test]
    fn retry_policy_default_shape_is_stable() {
        let p = RetryPolicy::default();
        assert_eq!(p.initial_delay, std::time::Duration::from_millis(250));
        assert_eq!(p.max_delay, std::time::Duration::from_secs(30));
        assert_eq!(p.max_attempts, 20);
        assert_eq!(p.max_elapsed, std::time::Duration::from_secs(300));
        assert!(p.jitter);
    }

    #[test]
    fn retry_policy_capped_backoff_doubles_each_attempt_then_caps() {
        use std::time::Duration;
        let p = RetryPolicy {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(800),
            max_attempts: 10,
            max_elapsed: Duration::ZERO,
            jitter: false,
        };
        assert_eq!(p.capped_backoff(0), Duration::ZERO);
        assert_eq!(p.capped_backoff(1), Duration::from_millis(100));
        assert_eq!(p.capped_backoff(2), Duration::from_millis(200));
        assert_eq!(p.capped_backoff(3), Duration::from_millis(400));
        assert_eq!(p.capped_backoff(4), Duration::from_millis(800));
        // Saturates at max_delay; never exceeds the cap even on absurd attempt counts.
        assert_eq!(p.capped_backoff(5), Duration::from_millis(800));
        assert_eq!(p.capped_backoff(60), Duration::from_millis(800));
    }

    #[test]
    fn retry_policy_delay_for_attempt_respects_jitter_upper_bound() {
        use std::time::Duration;
        let p = RetryPolicy {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(1_000),
            max_attempts: 10,
            max_elapsed: Duration::ZERO,
            jitter: true,
        };
        // Full-jitter envelope: sample ∈ [0, capped_backoff(attempt)].
        // Exercise 200 draws per attempt to shake out off-by-one issues
        // without making the test flaky — every sample must land in
        // the closed interval above.
        for attempt in 1..=6u32 {
            let ceiling = p.capped_backoff(attempt);
            for _ in 0..200 {
                let delay = p.delay_for_attempt(attempt);
                assert!(
                    delay <= ceiling,
                    "attempt {attempt}: delay {delay:?} exceeded ceiling {ceiling:?}"
                );
            }
        }
    }

    #[test]
    fn retry_policy_delay_for_attempt_deterministic_without_jitter() {
        use std::time::Duration;
        let p = RetryPolicy {
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(400),
            max_attempts: 5,
            max_elapsed: Duration::ZERO,
            jitter: false,
        };
        // No jitter → every draw equals the capped backoff envelope.
        for attempt in 1..=4u32 {
            let expected = p.capped_backoff(attempt);
            for _ in 0..16 {
                assert_eq!(p.delay_for_attempt(attempt), expected);
            }
        }
    }

    #[test]
    fn retry_policy_disabled_yields_single_attempt() {
        use std::time::Duration;
        let p = RetryPolicy::disabled();
        assert_eq!(p.max_attempts, 1);
        assert_eq!(p.delay_for_attempt(1), Duration::ZERO);
        assert!(!p.jitter);
    }

    // `std::env` is a process-global singleton; the env-var tests use a
    // single mutex so they don't trample each other under
    // `cargo test -- --test-threads=N`. Each test keeps hold of the
    // guard for the duration of the config build + assertions.
    fn env_test_guard() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn clear_env_matrix() {
        // Unset every variable the env-override path reads so no test
        // leaks into another. The guard above pins us as the sole writer.
        // SAFETY: the env-var tests serialise through `env_test_guard()`,
        // a process-global `Mutex<()>` held for the full body of every
        // env test; this function is only called from inside that
        // critical section. The Rust 1.88 `unsafe fn` contract on
        // `std::env::remove_var` requires the caller to ensure no other
        // thread reads or writes the environment concurrently — the
        // mutex provides exactly that.
        unsafe {
            std::env::remove_var(ENV_HISTORICAL_HOST);
            std::env::remove_var(ENV_HISTORICAL_PORT);
            std::env::remove_var(ENV_NEXUS_URL);
            std::env::remove_var(ENV_STREAMING_HOST);
            std::env::remove_var(ENV_STREAMING_PORT);
            std::env::remove_var(ENV_CLIENT_TYPE);
        }
    }

    #[test]
    fn env_overrides_apply_on_production() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: `_guard` holds the process-global env-var mutex for
        // the body of this test, so no other thread observes or mutates
        // the environment while these writes land. `std::env::set_var`'s
        // 1.88 `unsafe fn` contract is therefore upheld.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_HOST, "historical.staging.example.com");
            std::env::set_var(ENV_HISTORICAL_PORT, "8443");
            std::env::set_var(ENV_NEXUS_URL, "https://nexus.staging.example.com/auth");
            std::env::set_var(ENV_CLIENT_TYPE, "rust-thetadatadx-staging");
            std::env::set_var(ENV_STREAMING_HOST, "streaming.staging.example.com");
            std::env::set_var(ENV_STREAMING_PORT, "21000");
        }
        let config = DirectConfig::production();
        assert_eq!(config.historical.host, "historical.staging.example.com");
        assert_eq!(config.historical.port, 8443);
        assert_eq!(
            config.auth.nexus_url,
            "https://nexus.staging.example.com/auth"
        );
        assert_eq!(config.auth.client_type, "rust-thetadatadx-staging");
        assert_eq!(
            config.streaming.hosts[0],
            ("streaming.staging.example.com".to_string(), 21000)
        );
        clear_env_matrix();
    }

    #[test]
    fn builder_takes_precedence_over_env_var() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: `_guard` holds the process-global env-var mutex for
        // the body of this test, so no other thread observes or mutates
        // the environment while this write lands.
        unsafe {
            std::env::set_var(ENV_CLIENT_TYPE, "env-wins-when-no-builder");
        }
        let config = DirectConfig::production().with_client_type("builder-wins");
        assert_eq!(config.auth.client_type, "builder-wins");
        clear_env_matrix();
    }

    #[test]
    fn env_overrides_skipped_when_values_malformed() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: `_guard` holds the process-global env-var mutex for
        // the body of this test, so no other thread observes or mutates
        // the environment while these writes land.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_PORT, "not-a-port");
            std::env::set_var(ENV_STREAMING_PORT, "0"); // reject zero
            std::env::set_var(ENV_HISTORICAL_HOST, "   "); // whitespace-only
        }
        let config = DirectConfig::production();
        let defaults = DirectConfig::production_defaults();
        assert_eq!(config.historical.host, defaults.historical.host);
        assert_eq!(config.historical.port, defaults.historical.port);
        assert_eq!(config.streaming.hosts[0].1, defaults.streaming.hosts[0].1);
        clear_env_matrix();
    }

    #[test]
    fn production_defaults_are_not_sensitive_to_env() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: `_guard` holds the process-global env-var mutex for
        // the body of this test, so no other thread observes or mutates
        // the environment while these writes land.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_HOST, "ignored-by-defaults");
            std::env::set_var(ENV_HISTORICAL_PORT, "9999");
        }
        let config = DirectConfig::production_defaults();
        assert_eq!(config.historical.host, "mdds-01.thetadata.us");
        assert_eq!(config.historical.port, 443);
        clear_env_matrix();
    }
}
