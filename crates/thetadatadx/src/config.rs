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

/// Configuration for connecting to ThetaData servers directly.
///
/// Use [`DirectConfig::production()`] for the standard NJ production servers.
#[derive(Debug, Clone)]
pub struct DirectConfig {
    // ── MDDS (gRPC) ──
    /// MDDS gRPC hostname.
    ///
    /// Source: `MddsConnectionManager` in decompiled terminal (v3 path).
    pub mdds_host: String,

    /// MDDS gRPC port (443 for TLS in production).
    pub mdds_port: u16,

    /// Whether to use TLS for the MDDS gRPC connection.
    /// Always `true` in production (standard gRPC-over-TLS on port 443).
    pub mdds_tls: bool,

    // ── FPSS (TCP) ──
    /// FPSS TCP hosts with round-robin failover.
    ///
    /// Source: `FPSS_NJ_HOSTS` in `config_0.properties` — the terminal
    /// iterates through these on connection failure.
    pub fpss_hosts: Vec<(String, u16)>,

    // ── FPSS tuning ──
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

    // ── MDDS tuning ──
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

    // ── Reconnection ──
    /// Delay before attempting reconnection after a disconnect, in milliseconds.
    ///
    /// Source: `RECONNECT_WAIT=1000` in `config_0.properties`.
    ///
    /// NOTE: Not automatically wired — caller should pass to `fpss::reconnect()`.
    pub reconnect_wait_ms: u64,

    /// Delay before reconnecting after a TooManyRequests disconnect, in milliseconds.
    ///
    /// Source: `FPSSClient.handleInvoluntaryDisconnect()` — 130 second wait.
    ///
    /// NOTE: Not automatically wired — caller should pass to `fpss::reconnect()`.
    pub reconnect_wait_rate_limited_ms: u64,

    // ── Threading ──
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

            // Source: ChannelProvider in decompiled terminal
            mdds_max_message_size: 4 * 1024 * 1024, // 4MB default
            mdds_keepalive_secs: 30,
            mdds_keepalive_timeout_secs: 10,

            // Source: config_0.properties RECONNECT_WAIT
            reconnect_wait_ms: 1_000,
            reconnect_wait_rate_limited_ms: 130_000, // FPSSClient: 130s for TooManyRequests

            // Default: use all CPU cores
            tokio_worker_threads: None,
        }
    }

    /// Dev/staging configuration.
    ///
    /// Uses the same servers as production (ThetaData has no public staging
    /// environment), but with more aggressive timeouts for faster iteration.
    pub fn dev() -> Self {
        Self {
            mdds_host: "mdds-01.thetadata.us".to_string(),
            mdds_port: 443,
            mdds_tls: true,

            fpss_hosts: vec![
                ("nj-a.thetadata.us".to_string(), 20000),
                ("nj-a.thetadata.us".to_string(), 20001),
            ],

            fpss_timeout_ms: 5_000,
            fpss_queue_depth: 100_000,
            fpss_ring_size: 65_536, // 2^16, smaller for dev
            fpss_ping_interval_ms: 100,
            fpss_connect_timeout_ms: 2_000,

            mdds_max_message_size: 4 * 1024 * 1024,
            mdds_keepalive_secs: 30,
            mdds_keepalive_timeout_secs: 10,

            reconnect_wait_ms: 500,
            reconnect_wait_rate_limited_ms: 130_000,

            tokio_worker_threads: None,
        }
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
    fn dev_config_shorter_timeouts() {
        let prod = DirectConfig::production();
        let dev = DirectConfig::dev();
        assert!(dev.fpss_timeout_ms < prod.fpss_timeout_ms);
        assert!(dev.reconnect_wait_ms < prod.reconnect_wait_ms);
    }
}
