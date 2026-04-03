//! Server configuration for direct ThetaData access.
//!
//! # Server topology (from decompiled Java + `config_0.properties`)
//!
//! ThetaData runs two server types in their NJ datacenter:
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
//! Source: `MddsConnectionManager` in decompiled terminal — the v3 code path
//! constructs a gRPC channel to `mdds-01.thetadata.us:443` with TLS, ignoring
//! the multi-host config entirely.
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

use crate::error::Error;

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

/// Configuration for connecting to ThetaData servers directly.
///
/// Use [`DirectConfig::production()`] for the standard NJ production servers.
#[derive(Debug, Clone)]
pub struct DirectConfig {
    // -- MDDS (gRPC) --
    /// MDDS gRPC hostname.
    ///
    /// Source: `MddsConnectionManager` in decompiled terminal (v3 path).
    pub mdds_host: String,

    /// MDDS gRPC port (443 for TLS in production).
    pub mdds_port: u16,

    /// Whether to use TLS for the MDDS gRPC connection.
    /// Always `true` in production (standard gRPC-over-TLS on port 443).
    pub mdds_tls: bool,

    // -- FPSS (TCP) --
    /// FPSS TCP hosts with round-robin failover.
    ///
    /// Source: `FPSS_NJ_HOSTS` in `config_0.properties` — the terminal
    /// iterates through these on connection failure.
    pub fpss_hosts: Vec<(String, u16)>,

    // -- FPSS tuning --
    /// FPSS connection/read timeout in milliseconds.
    ///
    /// Source: `FPSS_TIMEOUT=10000` in `config_0.properties`.
    pub fpss_timeout_ms: u64,

    /// FPSS event channel buffer depth.
    /// Caller should pass this to `FpssClient::connect(creds, fpss_queue_depth)`.
    /// Increase if stream events are being dropped under high volume.
    ///
    /// JVM equivalent: `FPSS_QUEUE_DEPTH=1000000` in `config_0.properties`.
    ///
    /// NOTE: Not automatically wired — caller must pass to `FpssClient::connect()`.
    pub fpss_queue_depth: usize,

    /// FPSS disruptor ring buffer size (slots, will be rounded up to a power of 2).
    ///
    /// The LMAX Disruptor ring buffer used for lock-free event dispatch requires
    /// a power-of-2 size. This value is rounded up automatically. Larger rings
    /// absorb more burst traffic but use more memory (~`ring_size * sizeof(Option<FpssEvent>)`).
    ///
    /// Derived from `fpss_queue_depth` by default. Override for fine-grained control.
    pub fpss_ring_size: usize,

    /// FPSS heartbeat ping interval in milliseconds.
    /// The protocol requires pings every 100ms; changing this may cause disconnects.
    ///
    /// Source: `FPSSClient.startPinging()` — timer period = 100ms.
    ///
    /// NOTE: Not automatically wired — the ping loop uses `protocol::PING_INTERVAL_MS`.
    /// Override that constant or pass this value when a configurable ping loop is added.
    pub fpss_ping_interval_ms: u64,

    /// Per-server TCP connect timeout in milliseconds.
    ///
    /// Source: `FPSSClient` — `socket.connect(addr, 2000)`.
    ///
    /// NOTE: Not automatically wired — the connection module uses `protocol::CONNECT_TIMEOUT_MS`.
    /// Override that constant or pass this value when a configurable connect is added.
    pub fpss_connect_timeout_ms: u64,

    /// Controls when the FPSS write buffer is flushed.
    ///
    /// - [`FpssFlushMode::Batched`] (default): only flush on PING frames (~100ms),
    ///   matching the Java terminal. Lower syscall overhead.
    /// - [`FpssFlushMode::Immediate`]: flush after every frame write. Lowest
    ///   latency, higher syscall overhead.
    pub fpss_flush_mode: FpssFlushMode,

    // -- MDDS tuning --
    /// Max concurrent in-flight gRPC requests.
    ///
    /// JVM equivalent: `2^subscription_tier` (Free=1, Value=2, Standard=4, Pro=8).
    /// Set to 0 to auto-detect from the subscription tier returned by Nexus auth.
    pub mdds_concurrent_requests: usize,

    /// Max inbound gRPC message size in bytes.
    ///
    /// JVM equivalent: `maxInboundMessageSize(0x100000 * config.messageSize())`,
    /// default 4MB, max 10MB.
    pub mdds_max_message_size: usize,

    /// gRPC keepalive interval in seconds.
    ///
    /// Source: `ChannelProvider` — `keepAliveTime(30, SECONDS)`.
    pub mdds_keepalive_secs: u64,

    /// gRPC keepalive timeout in seconds.
    ///
    /// Source: `ChannelProvider` — `keepAliveTimeout(10, SECONDS)`.
    pub mdds_keepalive_timeout_secs: u64,

    /// gRPC flow control: initial stream window size in KB.
    ///
    /// Maps to `tonic::transport::Endpoint::initial_stream_window_size`.
    /// Default 64 KB matches HTTP/2 spec default.
    pub mdds_window_size_kb: usize,

    /// gRPC flow control: initial connection window size in KB.
    ///
    /// Maps to `tonic::transport::Endpoint::initial_connection_window_size`.
    /// Default 64 KB. Increase for high-throughput bulk queries.
    pub mdds_connection_window_size_kb: usize,

    // -- Reconnection --
    /// Delay before attempting reconnection after a disconnect, in milliseconds.
    ///
    /// Source: `FPSSClient.RECONNECT_DELAY_MS = 2000` in decompiled terminal.
    /// Note: `config_0.properties` has `RECONNECT_WAIT=1000` but the Java code
    /// uses the constant `2000` at runtime.
    ///
    /// NOTE: Not automatically wired — caller should pass to `fpss::reconnect()`.
    pub reconnect_wait_ms: u64,

    /// Delay before reconnecting after a TooManyRequests disconnect, in milliseconds.
    ///
    /// Source: `FPSSClient.handleInvoluntaryDisconnect()` — 130 second wait.
    ///
    /// NOTE: Not automatically wired — caller should pass to `fpss::reconnect()`.
    pub reconnect_wait_rate_limited_ms: u64,

    // -- Threading --
    /// Number of tokio worker threads. `None` = tokio default (number of CPU cores).
    ///
    /// JVM equivalent: `-Xmx` + `HTTP_CONCURRENCY` thread pool sizing.
    ///
    /// NOTE: Not automatically wired — caller should use this when building
    /// a custom tokio runtime.
    pub tokio_worker_threads: Option<usize>,
}

impl DirectConfig {
    /// Production configuration for ThetaData's NJ datacenter.
    ///
    /// All values extracted from the decompiled Java terminal:
    /// - MDDS: `mdds-01.thetadata.us:443` (gRPC over TLS)
    /// - FPSS: 4 hosts from `config_0.properties` `FPSS_NJ_HOSTS`
    /// - Timeouts: from `config_0.properties`
    pub fn production() -> Self {
        Self {
            // Source: MddsConnectionManager (v3 gRPC path)
            mdds_host: "mdds-01.thetadata.us".to_string(),
            mdds_port: 443,
            mdds_tls: true,

            // Source: config_0.properties FPSS_NJ_HOSTS
            fpss_hosts: vec![
                ("nj-a.thetadata.us".to_string(), 20000),
                ("nj-a.thetadata.us".to_string(), 20001),
                ("nj-b.thetadata.us".to_string(), 20000),
                ("nj-b.thetadata.us".to_string(), 20001),
            ],

            // Source: config_0.properties
            fpss_timeout_ms: 10_000,
            fpss_queue_depth: 1_000_000,    // FPSS_QUEUE_DEPTH
            fpss_ring_size: 131_072,        // 2^17, covers ~13s at 10k events/sec
            fpss_ping_interval_ms: 100,     // FPSSClient.startPinging()
            fpss_connect_timeout_ms: 2_000, // FPSSClient socket.connect timeout
            fpss_flush_mode: FpssFlushMode::Batched,

            // Concurrency: 0 = auto-detect from subscription tier at auth time.
            mdds_concurrent_requests: 0,

            // Source: ChannelProvider in decompiled terminal
            mdds_max_message_size: 4 * 1024 * 1024, // 4MB default
            mdds_keepalive_secs: 30,
            mdds_keepalive_timeout_secs: 10,

            // gRPC flow control (HTTP/2 spec defaults)
            mdds_window_size_kb: 64,
            mdds_connection_window_size_kb: 64,

            // Source: FPSSClient.RECONNECT_DELAY_MS = 2000 in decompiled terminal
            reconnect_wait_ms: 2_000,
            reconnect_wait_rate_limited_ms: 130_000, // FPSSClient: 130s for TooManyRequests

            // Default: use all CPU cores
            tokio_worker_threads: None,
        }
    }

    /// Dev FPSS configuration.
    ///
    /// Connects to ThetaData's dev FPSS servers (port 20200) which replay
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
    pub fn dev() -> Self {
        let mut config = Self::production();
        // Source: config.toml fpss_dev_hosts
        config.fpss_hosts = vec![
            ("nj-a.thetadata.us".to_string(), 20200),
            ("test-server.thetadata.us".to_string(), 20200),
            ("test-server.thetadata.us".to_string(), 20201),
        ];
        config
    }

    /// Stage FPSS configuration.
    ///
    /// Connects to ThetaData's staging FPSS servers (port 20100).
    /// Frequent reboots, testing data. Not stable.
    ///
    /// MDDS (historical) still uses production servers.
    ///
    /// Source: `config.toml` `fpss_stage_hosts`
    pub fn stage() -> Self {
        let mut config = Self::production();
        // Source: config.toml fpss_stage_hosts
        config.fpss_hosts = vec![
            ("nj-a.thetadata.us".to_string(), 20100),
            ("test-server.thetadata.us".to_string(), 20100),
            ("test-server.thetadata.us".to_string(), 20101),
        ];
        config
    }

    /// Build the MDDS gRPC endpoint URI.
    ///
    /// Returns a URI suitable for `tonic::transport::Channel::from_static()`.
    pub fn mdds_uri(&self) -> String {
        let scheme = if self.mdds_tls { "https" } else { "http" };
        format!("{}://{}:{}", scheme, self.mdds_host, self.mdds_port)
    }

    /// Parse FPSS hosts from a comma-separated `host:port,host:port,...` string.
    ///
    /// This is the format used in `config_0.properties` for `FPSS_NJ_HOSTS`.
    pub fn parse_fpss_hosts(hosts_str: &str) -> Result<Vec<(String, u16)>, Error> {
        let mut result = Vec::new();

        for entry in hosts_str.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }

            let (host, port_str) = entry
                .rsplit_once(':')
                .ok_or_else(|| Error::Config(format!("invalid host:port entry: '{entry}'")))?;

            let port: u16 = port_str
                .parse()
                .map_err(|e| Error::Config(format!("invalid port in '{entry}': {e}")))?;

            result.push((host.to_string(), port));
        }

        if result.is_empty() {
            return Err(Error::Config("no FPSS hosts provided".to_string()));
        }

        Ok(result)
    }
}

// ── Config file loading (behind `config-file` feature) ──────────────────────

#[cfg(feature = "config-file")]
mod config_file {
    use super::{DirectConfig, FpssFlushMode};
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
                host: prod.mdds_host,
                port: prod.mdds_port,
                tls: prod.mdds_tls,
                keepalive_time_secs: prod.mdds_keepalive_secs,
                keepalive_timeout_secs: prod.mdds_keepalive_timeout_secs,
                max_message_size: prod.mdds_max_message_size,
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
        queue_depth: usize,
        ring_size: usize,
        flush_mode: String,
    }

    impl Default for FpssSection {
        fn default() -> Self {
            let prod = DirectConfig::production();
            Self {
                hosts: FpssHosts::Array(
                    prod.fpss_hosts
                        .iter()
                        .map(|(h, p)| format!("{h}:{p}"))
                        .collect(),
                ),
                connect_timeout: prod.fpss_connect_timeout_ms,
                read_timeout: prod.fpss_timeout_ms,
                ping_interval: prod.fpss_ping_interval_ms,
                reconnect_wait: prod.reconnect_wait_ms,
                reconnect_wait_rate_limited: prod.reconnect_wait_rate_limited_ms,
                queue_depth: prod.fpss_queue_depth,
                ring_size: prod.fpss_ring_size,
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
                prod.fpss_hosts
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
                window_size_kb: prod.mdds_window_size_kb,
                connection_window_size_kb: prod.mdds_connection_window_size_kb,
                max_message_size_mb: prod.mdds_max_message_size / (1024 * 1024),
                concurrent_requests: prod.mdds_concurrent_requests,
            }
        }
    }

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    struct AuthSection {
        #[allow(dead_code)]
        creds_file: Option<String>,
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
                let (host, port_str) = entry
                    .rsplit_once(':')
                    .ok_or_else(|| Error::Config(format!("invalid host:port entry: '{entry}'")))?;
                let port: u16 = port_str
                    .parse()
                    .map_err(|e| Error::Config(format!("invalid port in '{entry}': {e}")))?;
                result.push((host.to_string(), port));
            }
            if result.is_empty() {
                return Err(Error::Config("no FPSS hosts provided".to_string()));
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
        /// queue_depth = 1000000
        /// flush_mode = "batched"  # or "immediate"
        ///
        /// [grpc]
        /// window_size_kb = 64
        /// connection_window_size_kb = 64
        /// concurrent_requests = 0  # 0 = auto from tier
        /// ```
        pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
            let contents = std::fs::read_to_string(path.as_ref()).map_err(|e| {
                Error::Config(format!(
                    "failed to read config file '{}': {e}",
                    path.as_ref().display()
                ))
            })?;
            Self::from_toml_str(&contents)
        }

        /// Parse configuration from a TOML string.
        ///
        /// Same semantics as [`from_file`](Self::from_file) but takes a string directly.
        pub fn from_toml_str(toml_str: &str) -> Result<Self, Error> {
            let cf: ConfigFile = toml::from_str(toml_str)
                .map_err(|e| Error::Config(format!("failed to parse TOML config: {e}")))?;

            let flush_mode = match cf.fpss.flush_mode.to_lowercase().as_str() {
                "immediate" => FpssFlushMode::Immediate,
                _ => FpssFlushMode::Batched,
            };

            // If [grpc].max_message_size_mb is set, it overrides [mdds].max_message_size.
            // The grpc section value is in MB; the mdds section value is in bytes.
            let max_message_size = if cf.grpc.max_message_size_mb
                != DirectConfig::production().mdds_max_message_size / (1024 * 1024)
            {
                cf.grpc.max_message_size_mb * 1024 * 1024
            } else {
                cf.mdds.max_message_size
            };

            Ok(DirectConfig {
                mdds_host: cf.mdds.host,
                mdds_port: cf.mdds.port,
                mdds_tls: cf.mdds.tls,

                fpss_hosts: cf.fpss.hosts.parse()?,
                fpss_timeout_ms: cf.fpss.read_timeout,
                fpss_queue_depth: cf.fpss.queue_depth,
                fpss_ring_size: cf.fpss.ring_size,
                fpss_ping_interval_ms: cf.fpss.ping_interval,
                fpss_connect_timeout_ms: cf.fpss.connect_timeout,
                fpss_flush_mode: flush_mode,

                mdds_concurrent_requests: cf.grpc.concurrent_requests,
                mdds_max_message_size: max_message_size,
                mdds_keepalive_secs: cf.mdds.keepalive_time_secs,
                mdds_keepalive_timeout_secs: cf.mdds.keepalive_timeout_secs,
                mdds_window_size_kb: cf.grpc.window_size_kb,
                mdds_connection_window_size_kb: cf.grpc.connection_window_size_kb,

                reconnect_wait_ms: cf.fpss.reconnect_wait,
                reconnect_wait_rate_limited_ms: cf.fpss.reconnect_wait_rate_limited,

                tokio_worker_threads: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_mdds_uri() {
        let config = DirectConfig::production();
        assert_eq!(config.mdds_uri(), "https://mdds-01.thetadata.us:443");
    }

    #[test]
    fn production_has_four_fpss_hosts() {
        let config = DirectConfig::production();
        assert_eq!(config.fpss_hosts.len(), 4);
    }

    #[test]
    fn parse_fpss_hosts_basic() {
        let hosts =
            DirectConfig::parse_fpss_hosts("nj-a.thetadata.us:20000,nj-a.thetadata.us:20001")
                .unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0], ("nj-a.thetadata.us".to_string(), 20000));
        assert_eq!(hosts[1], ("nj-a.thetadata.us".to_string(), 20001));
    }

    #[test]
    fn parse_fpss_hosts_from_config_properties() {
        // Exact string from config_0.properties
        let hosts = DirectConfig::parse_fpss_hosts(
            "nj-a.thetadata.us:20000,nj-a.thetadata.us:20001,nj-b.thetadata.us:20000,nj-b.thetadata.us:20001",
        )
        .unwrap();
        assert_eq!(hosts.len(), 4);
        assert_eq!(hosts[2].0, "nj-b.thetadata.us");
        assert_eq!(hosts[2].1, 20000);
    }

    #[test]
    fn parse_fpss_hosts_trims_whitespace() {
        let hosts =
            DirectConfig::parse_fpss_hosts(" nj-a.thetadata.us:20000 , nj-b.thetadata.us:20001 ")
                .unwrap();
        assert_eq!(hosts.len(), 2);
    }

    #[test]
    fn parse_fpss_hosts_skips_empty_entries() {
        let hosts =
            DirectConfig::parse_fpss_hosts("nj-a.thetadata.us:20000,,nj-b.thetadata.us:20001")
                .unwrap();
        assert_eq!(hosts.len(), 2);
    }

    #[test]
    fn parse_fpss_hosts_rejects_empty() {
        let err = DirectConfig::parse_fpss_hosts("").unwrap_err();
        assert!(err.to_string().contains("no FPSS hosts"));
    }

    #[test]
    fn parse_fpss_hosts_rejects_bad_port() {
        let err = DirectConfig::parse_fpss_hosts("host:notaport").unwrap_err();
        assert!(err.to_string().contains("invalid port"));
    }

    #[test]
    fn parse_fpss_hosts_rejects_no_port() {
        let err = DirectConfig::parse_fpss_hosts("hostonly").unwrap_err();
        assert!(err.to_string().contains("invalid host:port"));
    }

    #[test]
    fn dev_config_uses_dev_fpss_hosts() {
        let prod = DirectConfig::production();
        let dev = DirectConfig::dev();
        // Dev uses port 20200 (replay server), production uses 20000
        assert_eq!(dev.fpss_hosts[0].1, 20200);
        assert_eq!(prod.fpss_hosts[0].1, 20000);
        // Dev inherits production timeouts/queue/ring (replay is max speed)
        assert_eq!(dev.fpss_timeout_ms, prod.fpss_timeout_ms);
        assert_eq!(dev.fpss_queue_depth, prod.fpss_queue_depth);
        assert_eq!(dev.fpss_ring_size, prod.fpss_ring_size);
    }

    #[test]
    fn stage_config_uses_stage_fpss_hosts() {
        let stage = DirectConfig::stage();
        assert_eq!(stage.fpss_hosts[0].1, 20100);
    }

    // -- Config file tests (only compiled with the `config-file` feature) --

    #[cfg(feature = "config-file")]
    mod config_file_tests {
        use crate::config::{DirectConfig, FpssFlushMode};

        #[test]
        fn empty_toml_gives_production_defaults() {
            let config = DirectConfig::from_toml_str("").unwrap();
            let prod = DirectConfig::production();
            assert_eq!(config.mdds_host, prod.mdds_host);
            assert_eq!(config.mdds_port, prod.mdds_port);
            assert_eq!(config.fpss_hosts.len(), prod.fpss_hosts.len());
            assert_eq!(config.fpss_queue_depth, prod.fpss_queue_depth);
        }

        #[test]
        fn partial_toml_overrides_only_specified() {
            let toml = r#"
                [mdds]
                host = "custom.example.com"
                port = 8443

                [fpss]
                queue_depth = 500000
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.mdds_host, "custom.example.com");
            assert_eq!(config.mdds_port, 8443);
            assert_eq!(config.fpss_queue_depth, 500000);
            // Unspecified fields keep production defaults
            assert!(config.mdds_tls);
        }

        #[test]
        fn fpss_hosts_as_array() {
            let toml = r#"
                [fpss]
                hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.fpss_hosts.len(), 2);
            assert_eq!(
                config.fpss_hosts[0],
                ("host-a.example.com".to_string(), 20000)
            );
            assert_eq!(
                config.fpss_hosts[1],
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
            assert_eq!(config.fpss_hosts.len(), 2);
            assert_eq!(config.fpss_hosts[0].0, "host-a.example.com");
        }

        #[test]
        fn flush_mode_immediate() {
            let toml = r#"
                [fpss]
                flush_mode = "immediate"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.fpss_flush_mode, FpssFlushMode::Immediate);
        }

        #[test]
        fn flush_mode_batched_by_default() {
            let toml = r#"
                [fpss]
                flush_mode = "batched"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.fpss_flush_mode, FpssFlushMode::Batched);
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
            assert_eq!(config.mdds_window_size_kb, 128);
            assert_eq!(config.mdds_connection_window_size_kb, 256);
            assert_eq!(config.mdds_concurrent_requests, 4);
        }

        #[test]
        fn grpc_max_message_size_mb_overrides_mdds_bytes() {
            let toml = r#"
                [grpc]
                max_message_size_mb = 8
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.mdds_max_message_size, 8 * 1024 * 1024);
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
            assert_eq!(config.mdds_port, 443);
        }

        #[test]
        fn full_config_default_toml_parses() {
            // Validate that config.default.toml (shipped with the crate) can be parsed.
            let default_toml = include_str!("../../../config.default.toml");
            let config = DirectConfig::from_toml_str(default_toml).unwrap();
            assert_eq!(config.mdds_host, "mdds-01.thetadata.us");
            assert_eq!(config.mdds_port, 443);
            assert_eq!(config.fpss_hosts.len(), 4);
        }

        #[test]
        fn invalid_toml_returns_error() {
            let result = DirectConfig::from_toml_str("this is not valid toml [[[");
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("TOML"));
        }
    }

    // -- Metrics tests --

    #[test]
    fn metrics_counter_compiles_and_runs() {
        // Verify that metrics macros resolve without a recorder installed.
        // The `metrics` crate is designed to no-op when no recorder is set.
        metrics::counter!("thetadatadx.test.counter", "tag" => "value").increment(1);
        metrics::histogram!("thetadatadx.test.histogram").record(42.0);
    }
}
