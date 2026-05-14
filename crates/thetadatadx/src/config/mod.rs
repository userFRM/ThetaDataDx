//! Server configuration for direct `ThetaData` access.
//!
//! # Server topology (from decompiled Java + `config_0.properties`)
//!
//! `ThetaData` runs two server types in their NJ datacenter:
//!
//! ## MDDS — Market Data Distribution Server (gRPC, historical data)
//!
//! The v1/v2 config listed multiple socket-level hosts:
//! ```text
//! MDDS_NJ_HOSTS=nj-a.thetadata.us:12000,nj-a.thetadata.us:12001,
//!               nj-b.thetadata.us:12000,nj-b.thetadata.us:12001
//! ```
//!
//! But the v3 terminal uses a **single gRPC endpoint** over TLS:
//! ```text
//! mdds-01.thetadata.us:443
//! ```
//!
//! The v3 code path constructs a gRPC channel to `mdds-01.thetadata.us:443`
//! with TLS, ignoring the multi-host config entirely.
//!
//! ## FPSS — Feed Processing Streaming Server (TCP, real-time streaming)
//!
//! FPSS still uses the multi-host config with round-robin failover:
//! ```text
//! FPSS_NJ_HOSTS=nj-a.thetadata.us:20000,nj-a.thetadata.us:20001,
//!               nj-b.thetadata.us:20000,nj-b.thetadata.us:20001
//! ```
//!
//! Source: `FpssConnectionManager` in decompiled terminal — iterates through
//! hosts on connection failure.
//!
//! # Layout
//!
//! [`DirectConfig`] is composed of seven nested sub-configs:
//!
//! | Field           | Type                                                |
//! |-----------------|-----------------------------------------------------|
//! | `mdds`          | [`MddsConfig`] — gRPC host/port/TLS/keepalive       |
//! | `fpss`          | [`FpssConfig`] — TCP hosts, queue/ring, flush mode  |
//! | `reconnect`     | [`ReconnectConfig`] — wait cadence + policy         |
//! | `retry`         | [`RetryPolicy`] — exponential backoff for MDDS gRPC |
//! | `auth`          | [`AuthConfig`] — Nexus URL + `client_type`          |
//! | `metrics`       | [`MetricsConfig`] — Prometheus exporter port        |
//! | `runtime`       | [`RuntimeConfig`] — tokio worker thread sizing      |

mod auth;
mod env;
mod fpss;
mod mdds;
mod metrics;
mod reconnect;
mod retry;
mod runtime;

use crate::error::Error;

pub use auth::{AuthConfig, DEFAULT_CLIENT_TYPE, DEFAULT_NEXUS_URL};
pub use env::{
    ENV_CLIENT_TYPE, ENV_FPSS_HOST, ENV_FPSS_PORT, ENV_MDDS_HOST, ENV_MDDS_PORT, ENV_NEXUS_URL,
};
pub use fpss::{bounds as fpss_bounds, FpssConfig, FpssFlushMode};
pub use mdds::MddsConfig;
pub use metrics::MetricsConfig;
pub use reconnect::{ReconnectConfig, ReconnectPolicy};
pub use retry::RetryPolicy;
pub use runtime::RuntimeConfig;

/// Configuration for connecting to `ThetaData` servers directly.
///
/// Use [`DirectConfig::production()`] for the standard NJ production servers.
///
/// # Layout
///
/// Fields are grouped into seven nested sub-configs ([`MddsConfig`],
/// [`FpssConfig`], [`ReconnectConfig`], [`RetryPolicy`], [`AuthConfig`],
/// [`MetricsConfig`], [`RuntimeConfig`]). Read accessors on [`DirectConfig`]
/// preserve the field-style naming used by older callers; writes go through
/// the nested struct (e.g. `cfg.fpss.ring_size = N`).
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
/// | `THETADATA_MDDS_HOST` | host | overrides `mdds.host` |
/// | `THETADATA_MDDS_PORT` | u16  | overrides `mdds.port` |
/// | `THETADATA_NEXUS_URL` | url  | overrides the Nexus auth URL |
/// | `THETADATA_FPSS_HOST` | host | overrides the primary FPSS host |
/// | `THETADATA_FPSS_PORT` | u16  | overrides the primary FPSS port |
/// | `THETADATA_CLIENT_TYPE` | str | overrides `auth.client_type` |
/// | `THETADATA_EMAIL`       | str | credential helper ([`crate::auth`]) |
/// | `THETADATA_PASSWORD`    | str | credential helper ([`crate::auth`]) |
///
/// Malformed values (e.g. a non-integer `THETADATA_MDDS_PORT`) are ignored
/// with a `tracing::warn!` — the hardcoded default is retained so a typo
/// in the environment never silently breaks production.
#[derive(Debug, Clone, Default)]
pub struct DirectConfig {
    /// MDDS gRPC tuning.
    pub mdds: MddsConfig,
    /// FPSS streaming tuning.
    pub fpss: FpssConfig,
    /// Reconnection cadence + policy.
    pub reconnect: ReconnectConfig,
    /// MDDS retry policy.
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
    /// All values extracted from the decompiled Java terminal:
    /// - MDDS: `mdds-01.thetadata.us:443` (gRPC over TLS)
    /// - FPSS: 4 hosts from `config_0.properties` `FPSS_NJ_HOSTS`
    /// - Timeouts: from `config_0.properties`
    ///
    /// Environment variables listed on [`DirectConfig`] are layered on
    /// top of these defaults.
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
            mdds: MddsConfig::production_defaults(),
            fpss: FpssConfig::production_defaults(),
            reconnect: ReconnectConfig::production_defaults(),
            retry: RetryPolicy::default(),
            auth: AuthConfig::production_defaults(),
            metrics: MetricsConfig::default(),
            runtime: RuntimeConfig::default(),
        }
    }

    /// Dev FPSS configuration.
    ///
    /// Connects to `ThetaData`'s dev FPSS servers (port 20200) which replay
    /// a random historical trading day in an infinite loop at maximum speed.
    /// Designed for development and testing when markets are closed.
    ///
    /// MDDS (historical) still uses production servers -- there is no dev MDDS.
    ///
    /// Source: `config.toml` `fpss_dev_hosts` and
    /// <https://docs.thetadata.us/Streaming/Getting-Started.html>
    ///
    /// Note: dev server replays data at max speed, so queue and ring sizes
    /// match production to avoid drops. Some contracts may not exist on
    /// the replayed day.
    #[must_use]
    pub fn dev() -> Self {
        let mut config = Self::production();
        // Source: config.toml fpss_dev_hosts
        config.fpss.hosts = vec![
            ("nj-a.thetadata.us".to_string(), 20200),
            ("test-server.thetadata.us".to_string(), 20200),
            ("test-server.thetadata.us".to_string(), 20201),
        ];
        config
            .validate()
            .expect("dev preset is within validated bounds")
    }

    /// Stage FPSS configuration.
    ///
    /// Connects to `ThetaData`'s staging FPSS servers (port 20100).
    /// Frequent reboots, testing data. Not stable.
    ///
    /// MDDS (historical) still uses production servers.
    ///
    /// Source: `config.toml` `fpss_stage_hosts`
    #[must_use]
    pub fn stage() -> Self {
        let mut config = Self::production();
        // Source: config.toml fpss_stage_hosts
        config.fpss.hosts = vec![
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
    /// Returns the configuration with MDDS HTTP/2 window sizes clamped
    /// into `[64, 1024]` KB on success. Returns
    /// [`Error::Config`](crate::error::Error::Config) when any wired FPSS
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
    /// Returns [`Error::Config`](crate::error::Error::Config) when an FPSS
    /// timing knob is out of range.
    pub fn validate(mut self) -> Result<Self, Error> {
        // u64 → i64: every bound fits comfortably under i64::MAX (max
        // bound is 300_000 ms). `saturating_cast` would be overkill;
        // a checked `try_from` documents the invariant.
        let to_i64 = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        if !fpss_bounds::TIMEOUT_MS.contains(&self.fpss.timeout_ms) {
            return Err(Error::config_out_of_range(
                "fpss.timeout_ms",
                to_i64(self.fpss.timeout_ms),
                to_i64(*fpss_bounds::TIMEOUT_MS.start()),
                to_i64(*fpss_bounds::TIMEOUT_MS.end()),
            ));
        }
        if !fpss_bounds::CONNECT_TIMEOUT_MS.contains(&self.fpss.connect_timeout_ms) {
            return Err(Error::config_out_of_range(
                "fpss.connect_timeout_ms",
                to_i64(self.fpss.connect_timeout_ms),
                to_i64(*fpss_bounds::CONNECT_TIMEOUT_MS.start()),
                to_i64(*fpss_bounds::CONNECT_TIMEOUT_MS.end()),
            ));
        }
        if !fpss_bounds::PING_INTERVAL_MS.contains(&self.fpss.ping_interval_ms) {
            return Err(Error::config_out_of_range(
                "fpss.ping_interval_ms",
                to_i64(self.fpss.ping_interval_ms),
                to_i64(*fpss_bounds::PING_INTERVAL_MS.start()),
                to_i64(*fpss_bounds::PING_INTERVAL_MS.end()),
            ));
        }
        // Validate ring_size eagerly so a bad config fails fast rather
        // than waiting for the connect attempt. Re-validation happens
        // at `FpssClient::connect` for callers that bypass `validate`.
        if let Err(e) = crate::fpss::ring::check_ring_size(self.fpss.ring_size) {
            return Err(Error::config_invalid("fpss.ring_size", e.to_string()));
        }
        // Same contract for the MDDS decoder pool ring. The MDDS
        // pool uses the same shared validator (`crate::util::ring`)
        // so the failure mode is identical: non-power-of-two sizes
        // would force a modulo on every consumer tick.
        if let Err(e) = crate::util::ring::check_ring_size(self.mdds.decoder_ring_size) {
            return Err(Error::config_invalid(
                "mdds.decoder_ring_size",
                e.to_string(),
            ));
        }
        self.mdds.window_size_kb = self.mdds.window_size_kb.clamp(64, 1_024);
        self.mdds.connection_window_size_kb = self.mdds.connection_window_size_kb.clamp(64, 1_024);
        Ok(self)
    }

    /// Build the MDDS gRPC endpoint URI.
    ///
    /// Returns a URI suitable for `tonic::transport::Channel::from_static()`.
    #[must_use]
    pub fn mdds_uri(&self) -> String {
        let scheme = if self.mdds.tls { "https" } else { "http" };
        format!("{}://{}:{}", scheme, self.mdds.host, self.mdds.port)
    }

    /// Set whether to derive OHLCVC bars locally from trade events.
    ///
    /// When `false`, only server-sent OHLCVC frames are emitted,
    /// reducing per-trade throughput overhead.
    #[must_use]
    pub fn derive_ohlcvc(mut self, enabled: bool) -> Self {
        self.fpss.derive_ohlcvc = enabled;
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

    /// Parse FPSS hosts from a comma-separated `host:port,host:port,...` string.
    ///
    /// This is the format used in `config_0.properties` for `FPSS_NJ_HOSTS`.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn parse_fpss_hosts(hosts_str: &str) -> Result<Vec<(String, u16)>, Error> {
        let mut result = Vec::new();

        for entry in hosts_str.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }

            let (host, port_str) = entry.rsplit_once(':').ok_or_else(|| {
                Error::config_invalid("fpss.hosts", format!("invalid host:port entry: '{entry}'"))
            })?;

            let port: u16 = port_str.parse().map_err(|e| {
                Error::config_invalid("fpss.hosts", format!("invalid port in '{entry}': {e}"))
            })?;

            result.push((host.to_string(), port));
        }

        if result.is_empty() {
            return Err(Error::config_missing("fpss.hosts"));
        }

        Ok(result)
    }
}

// ── Read accessors (back-compat for the old flat field names) ────────────
//
// External callers that still spell config reads as `config.mdds_host(...)`
// should call these accessor methods. Field-syntax reads (`config.mdds_host`)
// no longer compile and must migrate to the nested form
// (`config.mdds.host`); see the commit body for the migration table.
impl DirectConfig {
    /// MDDS gRPC hostname.
    #[must_use]
    pub fn mdds_host(&self) -> &str {
        &self.mdds.host
    }
    /// MDDS gRPC port.
    #[must_use]
    pub fn mdds_port(&self) -> u16 {
        self.mdds.port
    }
    /// Whether MDDS uses TLS.
    #[must_use]
    pub fn mdds_tls(&self) -> bool {
        self.mdds.tls
    }
    /// MDDS concurrent in-flight requests budget.
    #[must_use]
    pub fn mdds_concurrent_requests(&self) -> usize {
        self.mdds.concurrent_requests
    }
    /// MDDS max inbound message size, in bytes.
    #[must_use]
    pub fn mdds_max_message_size(&self) -> usize {
        self.mdds.max_message_size
    }
    /// MDDS keepalive ping interval, in seconds.
    #[must_use]
    pub fn mdds_keepalive_secs(&self) -> u64 {
        self.mdds.keepalive_secs
    }
    /// MDDS keepalive ping timeout, in seconds.
    #[must_use]
    pub fn mdds_keepalive_timeout_secs(&self) -> u64 {
        self.mdds.keepalive_timeout_secs
    }
    /// MDDS HTTP/2 stream window size, in KB.
    #[must_use]
    pub fn mdds_window_size_kb(&self) -> usize {
        self.mdds.window_size_kb
    }
    /// MDDS HTTP/2 connection window size, in KB.
    #[must_use]
    pub fn mdds_connection_window_size_kb(&self) -> usize {
        self.mdds.connection_window_size_kb
    }
    /// MDDS TCP connect timeout, in seconds.
    #[must_use]
    pub fn mdds_connect_timeout_secs(&self) -> u64 {
        self.mdds.connect_timeout_secs
    }

    /// FPSS host list.
    #[must_use]
    pub fn fpss_hosts(&self) -> &[(String, u16)] {
        &self.fpss.hosts
    }
    /// FPSS read timeout, in milliseconds.
    #[must_use]
    pub fn fpss_timeout_ms(&self) -> u64 {
        self.fpss.timeout_ms
    }
    /// FPSS disruptor ring buffer size.
    #[must_use]
    pub fn fpss_ring_size(&self) -> usize {
        self.fpss.ring_size
    }
    /// FPSS heartbeat ping interval, in milliseconds.
    #[must_use]
    pub fn fpss_ping_interval_ms(&self) -> u64 {
        self.fpss.ping_interval_ms
    }
    /// FPSS TCP connect timeout, in milliseconds.
    #[must_use]
    pub fn fpss_connect_timeout_ms(&self) -> u64 {
        self.fpss.connect_timeout_ms
    }
    /// FPSS write-buffer flush mode.
    #[must_use]
    pub fn fpss_flush_mode(&self) -> FpssFlushMode {
        self.fpss.flush_mode
    }
    /// Whether to derive OHLCVC bars locally from trade events.
    #[must_use]
    pub fn derive_ohlcvc_enabled(&self) -> bool {
        self.fpss.derive_ohlcvc
    }

    /// FPSS reconnect wait, in milliseconds.
    #[must_use]
    pub fn reconnect_wait_ms(&self) -> u64 {
        self.reconnect.wait_ms
    }
    /// FPSS reconnect wait after `TooManyRequests`, in milliseconds.
    #[must_use]
    pub fn reconnect_wait_rate_limited_ms(&self) -> u64 {
        self.reconnect.wait_rate_limited_ms
    }
    /// FPSS reconnect policy.
    #[must_use]
    pub fn reconnect_policy(&self) -> &ReconnectPolicy {
        &self.reconnect.policy
    }

    /// MDDS retry policy.
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
    use super::{DirectConfig, FpssFlushMode, ReconnectPolicy, RetryPolicy};
    use crate::error::Error;
    use serde::Deserialize;

    /// TOML-level representation of the config file.
    ///
    /// Unknown keys are silently ignored (`#[serde(default)]` on each section).
    /// Missing sections fall back to production defaults.
    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    struct ConfigFile {
        mdds: MddsSection,
        fpss: FpssSection,
        grpc: GrpcSection,
        auth: AuthSection,
    }

    #[derive(Debug, Deserialize)]
    #[serde(default)]
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
                host: prod.mdds.host,
                port: prod.mdds.port,
                tls: prod.mdds.tls,
                keepalive_time_secs: prod.mdds.keepalive_secs,
                keepalive_timeout_secs: prod.mdds.keepalive_timeout_secs,
                max_message_size: prod.mdds.max_message_size,
            }
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(default)]
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
    }

    impl Default for FpssSection {
        fn default() -> Self {
            let prod = DirectConfig::production();
            Self {
                hosts: FpssHosts::Array(
                    prod.fpss
                        .hosts
                        .iter()
                        .map(|(h, p)| format!("{h}:{p}"))
                        .collect(),
                ),
                connect_timeout: prod.fpss.connect_timeout_ms,
                read_timeout: prod.fpss.timeout_ms,
                ping_interval: prod.fpss.ping_interval_ms,
                reconnect_wait: prod.reconnect.wait_ms,
                reconnect_wait_rate_limited: prod.reconnect.wait_rate_limited_ms,
                ring_size: prod.fpss.ring_size,
                flush_mode: "batched".to_string(),
            }
        }
    }

    /// FPSS hosts can be specified as either a TOML array or a comma-separated string.
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
                prod.fpss
                    .hosts
                    .iter()
                    .map(|(h, p)| format!("{h}:{p}"))
                    .collect(),
            )
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(default)]
    struct GrpcSection {
        window_size_kb: usize,
        connection_window_size_kb: usize,
        max_message_size_mb: usize,
        concurrent_requests: usize,
    }

    impl Default for GrpcSection {
        fn default() -> Self {
            let prod = DirectConfig::production();
            Self {
                window_size_kb: prod.mdds.window_size_kb,
                connection_window_size_kb: prod.mdds.connection_window_size_kb,
                max_message_size_mb: prod.mdds.max_message_size / (1024 * 1024),
                concurrent_requests: prod.mdds.concurrent_requests,
            }
        }
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    struct AuthSection {
        #[serde(rename = "creds_file")]
        _creds_file: Option<String>,
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
                        "fpss.hosts",
                        format!("invalid host:port entry: '{entry}'"),
                    )
                })?;
                let port: u16 = port_str.parse().map_err(|e| {
                    Error::config_invalid("fpss.hosts", format!("invalid port in '{entry}': {e}"))
                })?;
                result.push((host.to_string(), port));
            }
            if result.is_empty() {
                return Err(Error::config_missing("fpss.hosts"));
            }
            Ok(result)
        }
    }

    impl DirectConfig {
        /// Load configuration from a TOML file.
        ///
        /// The file format matches `config.default.toml` shipped with the crate.
        /// Missing sections and keys fall back to [`DirectConfig::production()`] defaults.
        /// Unknown keys are silently ignored.
        ///
        /// # Example file
        ///
        /// ```toml
        /// [mdds]
        /// host = "mdds-01.thetadata.us"
        /// port = 443
        /// tls = true
        ///
        /// [fpss]
        /// hosts = ["nj-a.thetadata.us:20000", "nj-b.thetadata.us:20000"]
        /// reconnect_wait = 2000
        /// queue_depth = 1_000_000
        /// flush_mode = "batched"  # or "immediate"
        ///
        /// [grpc]
        /// window_size_kb = 64
        /// connection_window_size_kb = 64
        /// concurrent_requests = 0  # 0 = auto from tier
        /// ```
        /// # Errors
        ///
        /// Returns an error on network, authentication, or parsing failure.
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
        /// # Errors
        ///
        /// Returns an error on network, authentication, or parsing failure.
        pub fn from_toml_str(toml_str: &str) -> Result<Self, Error> {
            let cf: ConfigFile =
                toml::from_str(toml_str).map_err(|e| Error::config_toml(e.to_string()))?;

            let flush_mode = match cf.fpss.flush_mode.to_lowercase().as_str() {
                "immediate" => FpssFlushMode::Immediate,
                _ => FpssFlushMode::Batched,
            };

            // If [grpc].max_message_size_mb is set, it overrides [mdds].max_message_size.
            // The grpc section value is in MB; the mdds section value is in bytes.
            let max_message_size = if cf.grpc.max_message_size_mb
                != DirectConfig::production().mdds.max_message_size / (1024 * 1024)
            {
                cf.grpc.max_message_size_mb * 1024 * 1024
            } else {
                cf.mdds.max_message_size
            };

            let mut out = DirectConfig::production_defaults();
            out.mdds.host = cf.mdds.host;
            out.mdds.port = cf.mdds.port;
            out.mdds.tls = cf.mdds.tls;
            out.mdds.concurrent_requests = cf.grpc.concurrent_requests;
            out.mdds.max_message_size = max_message_size;
            out.mdds.keepalive_secs = cf.mdds.keepalive_time_secs;
            out.mdds.keepalive_timeout_secs = cf.mdds.keepalive_timeout_secs;
            out.mdds.window_size_kb = cf.grpc.window_size_kb;
            out.mdds.connection_window_size_kb = cf.grpc.connection_window_size_kb;
            // mdds.connect_timeout_secs is not yet TOML-surfaced; keep production default.

            out.fpss.hosts = cf.fpss.hosts.parse()?;
            out.fpss.timeout_ms = cf.fpss.read_timeout;
            out.fpss.ring_size = cf.fpss.ring_size;
            out.fpss.ping_interval_ms = cf.fpss.ping_interval;
            out.fpss.connect_timeout_ms = cf.fpss.connect_timeout;
            out.fpss.flush_mode = flush_mode;
            // Default: derive OHLCVC from trades (matches production default).
            // Use the builder API to disable programmatically.
            out.fpss.derive_ohlcvc = true;

            out.reconnect.wait_ms = cf.fpss.reconnect_wait;
            out.reconnect.wait_rate_limited_ms = cf.fpss.reconnect_wait_rate_limited;
            // TOML config cannot express custom closures; default to Auto.
            // Use the builder API to set Manual or Custom programmatically.
            out.reconnect.policy = ReconnectPolicy::Auto;

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
    fn production_mdds_uri() {
        // `DirectConfig::production()` reads `THETADATA_MDDS_*` env
        // vars; another test in this module (`env_overrides_apply_on_production`)
        // mutates the same env via `unsafe`, and the env is process-
        // global. Acquire the shared test guard so the two cannot
        // race when `cargo test` runs them in parallel.
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.mdds_uri(), "https://mdds-01.thetadata.us:443");
    }

    #[test]
    fn production_has_four_fpss_hosts() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.fpss.hosts.len(), 4);
    }

    #[test]
    fn production_default_reconnect_policy_is_auto() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert!(matches!(config.reconnect.policy, ReconnectPolicy::Auto));
    }

    #[test]
    fn production_mdds_connect_timeout_default_is_ten_seconds() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.mdds.connect_timeout_secs, 10);
    }

    #[test]
    fn read_accessors_match_nested_fields() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.mdds_host(), config.mdds.host.as_str());
        assert_eq!(config.fpss_ring_size(), config.fpss.ring_size);
        assert_eq!(config.metrics_port(), config.metrics.port);
        assert_eq!(
            config.tokio_worker_threads(),
            config.runtime.tokio_worker_threads
        );
        assert_eq!(config.nexus_url(), config.auth.nexus_url.as_str());
    }

    #[test]
    fn parse_fpss_hosts_parses_multi_host_csv_with_whitespace_and_empty_entries() {
        let hosts =
            DirectConfig::parse_fpss_hosts(" nj-a.thetadata.us:20000, ,nj-b.thetadata.us:20001 ")
                .unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0], ("nj-a.thetadata.us".to_string(), 20000));
        assert_eq!(hosts[1], ("nj-b.thetadata.us".to_string(), 20001));
    }

    #[test]
    fn parse_fpss_hosts_rejects_malformed_entries() {
        assert!(DirectConfig::parse_fpss_hosts("").is_err());
        assert!(DirectConfig::parse_fpss_hosts("host:notaport").is_err());
        assert!(DirectConfig::parse_fpss_hosts("hostonly").is_err());
    }

    // -- Config file tests (only compiled with the `config-file` feature) --

    #[cfg(feature = "config-file")]
    mod config_file_tests {
        use crate::config::{DirectConfig, FpssFlushMode};

        #[test]
        fn empty_toml_gives_production_defaults() {
            let config = DirectConfig::from_toml_str("").unwrap();
            let prod = DirectConfig::production();
            assert_eq!(config.mdds.host, prod.mdds.host);
            assert_eq!(config.mdds.port, prod.mdds.port);
            assert_eq!(config.fpss.hosts.len(), prod.fpss.hosts.len());
            assert_eq!(config.fpss.ring_size, prod.fpss.ring_size);
        }

        #[test]
        fn partial_toml_overrides_only_specified() {
            let toml = r#"
                [mdds]
                host = "custom.example.com"
                port = 8443

                [fpss]
                ring_size = 65536
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.mdds.host, "custom.example.com");
            assert_eq!(config.mdds.port, 8443);
            assert_eq!(config.fpss.ring_size, 65536);
            // Unspecified fields keep production defaults
            assert!(config.mdds.tls);
        }

        #[test]
        fn fpss_hosts_as_array() {
            let toml = r#"
                [fpss]
                hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.fpss.hosts.len(), 2);
            assert_eq!(
                config.fpss.hosts[0],
                ("host-a.example.com".to_string(), 20000)
            );
            assert_eq!(
                config.fpss.hosts[1],
                ("host-b.example.com".to_string(), 20001)
            );
        }

        #[test]
        fn fpss_hosts_as_csv_string() {
            let toml = r#"
                [fpss]
                hosts = "host-a.example.com:20000,host-b.example.com:20001"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.fpss.hosts.len(), 2);
            assert_eq!(config.fpss.hosts[0].0, "host-a.example.com");
        }

        #[test]
        fn flush_mode_immediate() {
            let toml = r#"
                [fpss]
                flush_mode = "immediate"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.fpss.flush_mode, FpssFlushMode::Immediate);
        }

        #[test]
        fn flush_mode_batched_by_default() {
            let toml = r#"
                [fpss]
                flush_mode = "batched"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.fpss.flush_mode, FpssFlushMode::Batched);
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
            assert_eq!(config.mdds.window_size_kb, 128);
            assert_eq!(config.mdds.connection_window_size_kb, 256);
            assert_eq!(config.mdds.concurrent_requests, 4);
        }

        #[test]
        fn grpc_max_message_size_mb_overrides_mdds_bytes() {
            let toml = r#"
                [grpc]
                max_message_size_mb = 8
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.mdds.max_message_size, 8 * 1024 * 1024);
        }

        #[test]
        fn unknown_keys_are_ignored() {
            let toml = r#"
                [mdds]
                host = "mdds-01.thetadata.us"
                port = 443
                unknown_key = "should be ignored"

                [some_unknown_section]
                foo = "bar"
            "#;
            // Should not error
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.mdds.port, 443);
        }

        #[test]
        fn full_config_default_toml_parses() {
            // Validate that config.default.toml (shipped with the crate) can be parsed.
            let default_toml = include_str!("../../config.default.toml");
            let config = DirectConfig::from_toml_str(default_toml).unwrap();
            assert_eq!(config.mdds.host, "mdds-01.thetadata.us");
            assert_eq!(config.mdds.port, 443);
            assert_eq!(config.fpss.hosts.len(), 4);
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
    fn validate_clamps_mdds_window_sizes_into_range() {
        let mut config = DirectConfig::production_defaults();
        config.mdds.window_size_kb = 2_048;
        config.mdds.connection_window_size_kb = 32;
        let config = config.validate().expect("MDDS window sizes are clamped");
        assert_eq!(config.mdds.window_size_kb, 1_024);
        assert_eq!(config.mdds.connection_window_size_kb, 64);
    }

    #[test]
    fn validate_preserves_in_range_values() {
        let config = DirectConfig::production_defaults();
        let validated = config.validate().expect("production defaults validate");
        assert_eq!(validated.mdds.window_size_kb, 64);
        assert_eq!(validated.fpss.timeout_ms, 10_000);
        assert_eq!(validated.fpss.ping_interval_ms, 100);
        assert_eq!(validated.fpss.connect_timeout_ms, 2_000);
    }

    #[test]
    fn validate_rejects_fpss_timeout_below_minimum() {
        let mut config = DirectConfig::production_defaults();
        config.fpss.timeout_ms = 50;
        let err = config.validate().expect_err("must reject below-minimum");
        let msg = err.to_string();
        assert!(msg.contains("timeout_ms"), "{msg}");
    }

    #[test]
    fn validate_rejects_fpss_timeout_above_maximum() {
        let mut config = DirectConfig::production_defaults();
        config.fpss.timeout_ms = 120_000;
        let err = config.validate().expect_err("must reject above-maximum");
        assert!(err.to_string().contains("timeout_ms"));
    }

    #[test]
    fn validate_rejects_fpss_connect_timeout_out_of_range() {
        let mut config = DirectConfig::production_defaults();
        config.fpss.connect_timeout_ms = 100;
        let err = config.validate().expect_err("100ms is below 1s minimum");
        assert!(err.to_string().contains("connect_timeout_ms"));
    }

    #[test]
    fn validate_rejects_fpss_ping_interval_out_of_range() {
        let mut config = DirectConfig::production_defaults();
        config.fpss.ping_interval_ms = 50;
        let err = config.validate().expect_err("50ms below 100ms minimum");
        assert!(err.to_string().contains("ping_interval_ms"));
    }

    #[test]
    fn validate_rejects_invalid_ring_size() {
        let mut config = DirectConfig::production_defaults();
        config.fpss.ring_size = 100; // not a power of two
        let err = config.validate().expect_err("must reject non-power-of-two");
        assert!(err.to_string().contains("ring_size"));
    }

    #[test]
    fn validate_rejects_invalid_mdds_decoder_ring_size() {
        // Same contract as `fpss.ring_size`: power of two, >= 64.
        // 100 is the canonical "not a power of two" sentinel.
        let mut config = DirectConfig::production_defaults();
        config.mdds.decoder_ring_size = 100;
        let err = config.validate().expect_err("must reject non-power-of-two");
        assert!(
            err.to_string().contains("decoder_ring_size"),
            "expected error to name the offending field: {err}"
        );
    }

    #[test]
    fn validate_rejects_decoder_ring_size_below_minimum() {
        let mut config = DirectConfig::production_defaults();
        config.mdds.decoder_ring_size = 32; // below MIN_RING_SIZE
        let err = config
            .validate()
            .expect_err("must reject sub-minimum ring size");
        assert!(err.to_string().contains("decoder_ring_size"));
    }

    #[test]
    fn mdds_decoder_defaults_match_production_baseline() {
        let mdds = crate::config::MddsConfig::production_defaults();
        // `0` is the auto-detect sentinel; `default_decoder_thread_count`
        // resolves it at connect time.
        assert_eq!(mdds.decoder_threads, 0);
        assert_eq!(mdds.decoder_ring_size, 256);
        assert!(mdds.decoder_ring_size.is_power_of_two());
    }

    // ── RetryPolicy / env var tests ──────────────────────────────────

    #[test]
    fn retry_policy_default_shape_is_stable() {
        let p = RetryPolicy::default();
        assert_eq!(p.initial_delay, std::time::Duration::from_millis(250));
        assert_eq!(p.max_delay, std::time::Duration::from_secs(30));
        assert_eq!(p.max_attempts, 5);
        assert!(p.jitter);
    }

    #[test]
    fn retry_policy_capped_backoff_doubles_each_attempt_then_caps() {
        use std::time::Duration;
        let p = RetryPolicy {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(800),
            max_attempts: 10,
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
        unsafe {
            // Reason: test-only mutation; protected by env_test_guard.
            std::env::remove_var(ENV_MDDS_HOST);
            std::env::remove_var(ENV_MDDS_PORT);
            std::env::remove_var(ENV_NEXUS_URL);
            std::env::remove_var(ENV_FPSS_HOST);
            std::env::remove_var(ENV_FPSS_PORT);
            std::env::remove_var(ENV_CLIENT_TYPE);
        }
    }

    #[test]
    fn env_overrides_apply_on_production() {
        let _guard = env_test_guard();
        clear_env_matrix();
        unsafe {
            // Reason: test-only mutation; protected by env_test_guard.
            std::env::set_var(ENV_MDDS_HOST, "mdds.staging.example.com");
            std::env::set_var(ENV_MDDS_PORT, "8443");
            std::env::set_var(ENV_NEXUS_URL, "https://nexus.staging.example.com/auth");
            std::env::set_var(ENV_CLIENT_TYPE, "rust-thetadatadx-staging");
            std::env::set_var(ENV_FPSS_HOST, "fpss.staging.example.com");
            std::env::set_var(ENV_FPSS_PORT, "21000");
        }
        let config = DirectConfig::production();
        assert_eq!(config.mdds.host, "mdds.staging.example.com");
        assert_eq!(config.mdds.port, 8443);
        assert_eq!(
            config.auth.nexus_url,
            "https://nexus.staging.example.com/auth"
        );
        assert_eq!(config.auth.client_type, "rust-thetadatadx-staging");
        assert_eq!(
            config.fpss.hosts[0],
            ("fpss.staging.example.com".to_string(), 21000)
        );
        clear_env_matrix();
    }

    #[test]
    fn builder_takes_precedence_over_env_var() {
        let _guard = env_test_guard();
        clear_env_matrix();
        unsafe {
            // Reason: test-only mutation; protected by env_test_guard.
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
        unsafe {
            // Reason: test-only mutation; protected by env_test_guard.
            std::env::set_var(ENV_MDDS_PORT, "not-a-port");
            std::env::set_var(ENV_FPSS_PORT, "0"); // reject zero
            std::env::set_var(ENV_MDDS_HOST, "   "); // whitespace-only
        }
        let config = DirectConfig::production();
        let defaults = DirectConfig::production_defaults();
        assert_eq!(config.mdds.host, defaults.mdds.host);
        assert_eq!(config.mdds.port, defaults.mdds.port);
        assert_eq!(config.fpss.hosts[0].1, defaults.fpss.hosts[0].1);
        clear_env_matrix();
    }

    #[test]
    fn production_defaults_are_not_sensitive_to_env() {
        let _guard = env_test_guard();
        clear_env_matrix();
        unsafe {
            // Reason: test-only mutation; protected by env_test_guard.
            std::env::set_var(ENV_MDDS_HOST, "ignored-by-defaults");
            std::env::set_var(ENV_MDDS_PORT, "9999");
        }
        let config = DirectConfig::production_defaults();
        assert_eq!(config.mdds.host, "mdds-01.thetadata.us");
        assert_eq!(config.mdds.port, 443);
        clear_env_matrix();
    }
}
