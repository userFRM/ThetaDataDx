//! MDDS (gRPC) sub-configuration.
//!
//! Defaults match what the v3 terminal sends in production. See ADR-001
//! (`docs/architecture/ADR-001-java-terminal-parity.md`) for the Java
//! terminal parity reverse-engineering source.

/// MDDS gRPC client tuning.
#[derive(Debug, Clone)]
pub struct MddsConfig {
    /// MDDS gRPC hostname (v3 path).
    pub host: String,

    /// MDDS gRPC port (443 for TLS in production).
    pub port: u16,

    /// Whether to use TLS for the MDDS gRPC connection.
    /// Always `true` in production (standard gRPC-over-TLS on port 443).
    pub tls: bool,

    /// Max concurrent in-flight gRPC requests.
    ///
    /// JVM equivalent: `2^subscription_tier` (Free=1, Value=2, Standard=4, Pro=8).
    /// Set to 0 to auto-detect from the subscription tier returned by Nexus auth.
    pub concurrent_requests: usize,

    /// Max inbound gRPC message size in bytes.
    ///
    /// JVM equivalent: `maxInboundMessageSize(0x100000 * config.messageSize())`,
    /// default 4MB, max 10MB.
    pub max_message_size: usize,

    /// gRPC keepalive interval in seconds (`keepAliveTime(30, SECONDS)`).
    pub keepalive_secs: u64,

    /// gRPC keepalive timeout in seconds (`keepAliveTimeout(10, SECONDS)`).
    pub keepalive_timeout_secs: u64,

    /// gRPC flow control: initial stream window size in KB.
    ///
    /// Maps to `tonic::transport::Endpoint::initial_stream_window_size`.
    /// Default 64 KB matches HTTP/2 spec default.
    pub window_size_kb: usize,

    /// gRPC flow control: initial connection window size in KB.
    ///
    /// Maps to `tonic::transport::Endpoint::initial_connection_window_size`.
    /// Default 64 KB. Increase for high-throughput bulk queries.
    pub connection_window_size_kb: usize,

    /// TCP connect timeout for the MDDS gRPC channel, in seconds.
    ///
    /// Bounds the time the tonic endpoint will spend establishing a TCP +
    /// TLS handshake before failing fast. Default `10s` matches the upper
    /// bound observed on the wire; production deployments behind NAT / VPN
    /// can raise this to absorb slow handshakes without altering keepalive
    /// cadence.
    pub connect_timeout_secs: u64,
}

impl MddsConfig {
    /// Production defaults.
    #[must_use]
    pub fn production_defaults() -> Self {
        Self {
            host: "mdds-01.thetadata.us".to_string(),
            port: 443,
            tls: true,
            concurrent_requests: 0,
            max_message_size: 4 * 1024 * 1024,
            keepalive_secs: 30,
            keepalive_timeout_secs: 10,
            window_size_kb: 64,
            connection_window_size_kb: 64,
            connect_timeout_secs: 10,
        }
    }
}

impl Default for MddsConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}
