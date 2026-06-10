//! TLS TCP connection to FPSS servers.
//!
//! # Transport
//!
//! TLS over TCP with:
//! - `TCP_NODELAY = true` (Nagle disabled for low latency)
//! - Connect timeout: 2 seconds
//! - Read timeout: 10 seconds
//! - Tries servers in order until one connects: `nj-a:20000`, `nj-a:20001`,
//!   `nj-b:20000`, `nj-b:20001`
//!
//! # Implementation
//!
//! Uses `std::net::TcpStream` + `rustls::StreamOwned` for blocking TLS I/O.
//! No tokio, no async -- pure blocking I/O on `std::thread`.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Once};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use crate::auth::Credentials;
use crate::config::FpssFlushMode;
use crate::config::ReconnectPolicy;

use super::pinning::PinnedVerifier;
#[cfg(test)]
use super::protocol::CONNECT_TIMEOUT_MS;

/// Type alias for the TLS-wrapped TCP stream (blocking).
pub type FpssStream = StreamOwned<ClientConnection, TcpStream>;

/// Parameter bundle for [`super::FpssClient::connect_with_stream`].
///
/// Carries every connection-side knob plus the user callback. Bundled
/// into a struct so the call site stays linear instead of as a
/// positional list of a dozen heterogeneous arguments — and so the
/// configurable timeouts (`connect_timeout`, `read_timeout`,
/// `ping_interval`) plumbed in 9.1.0 don't grow the call signature.
pub(crate) struct ConnectWithStreamArgs<'a> {
    pub creds: &'a Credentials,
    pub stream: FpssStream,
    pub server_addr: String,
    pub hosts: &'a [(String, u16)],
    pub ring_size: usize,
    pub derive_ohlcvc: bool,
    pub flush_mode: FpssFlushMode,
    pub policy: ReconnectPolicy,
    /// Reconnect cadence (ms) for generic transient drops. Mirrors
    /// [`crate::config::ReconnectConfig::wait_ms`].
    pub wait_ms: u64,
    /// Reconnect cadence (ms) for `TooManyRequests` drops. Mirrors
    /// [`crate::config::ReconnectConfig::wait_rate_limited_ms`].
    pub wait_rate_limited_ms: u64,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub ping_interval: Duration,
}

/// Install the process-global rustls crypto provider exactly once.
///
/// Embedded consumers such as the Python SDK or FFI bindings do not always
/// have a top-level binary `main()` that installs this ahead of time.
fn ensure_rustls_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Establish a TLS connection to the first reachable FPSS server.
///
/// Tries each server in order. Returns the stream and connected server
/// address on success, or the last error if all fail.
///
/// # Connection sequence
///
/// 1. TCP connect with 2s timeout
/// 2. `TCP_NODELAY = true`
/// 3. Set read timeout to 10s (`socket.setSoTimeout(10000)`)
/// 4. TLS handshake via system trust store
///
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub fn connect_to_servers(
    servers: &[(&str, u16)],
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Result<(FpssStream, String), crate::error::Error> {
    ensure_rustls_crypto_provider();
    let mut last_err = None;

    for &(host, port) in servers {
        let addr = format!("{host}:{port}");
        tracing::debug!(server = %addr, "attempting FPSS connection");

        match try_connect(host, port, connect_timeout, read_timeout) {
            Ok(stream) => {
                tracing::info!(server = %addr, "FPSS connected");
                return Ok((stream, addr));
            }
            Err(e) => {
                tracing::warn!(server = %addr, error = %e, "FPSS connection failed");
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| crate::error::Error::Fpss {
        kind: crate::error::FpssErrorKind::ConnectionRefused,
        message: "no servers configured".to_string(),
    }))
}

/// Build a shared rustls `ClientConfig` pinned to the `ThetaData` FPSS
/// `SubjectPublicKeyInfo`.
///
/// `ThetaData`'s FPSS servers use TLS certificates that have been expired
/// since January 2024, so the stock `webpki` chain + validity check cannot
/// succeed. Instead of disabling verification entirely (which converts the
/// expiry problem into an open **MITM + credential harvest** hole -- the
/// very next frame after the handshake contains the user's email + password),
/// we pin on the leaf's SPKI. See [`super::pinning`] for the full rationale.
fn tls_client_config() -> Arc<ClientConfig> {
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(PinnedVerifier::new())
        .with_no_client_auth();
    Arc::new(config)
}

/// Attempt a single blocking TLS connection to one server.
///
/// # Steps
///
/// 1. `TcpStream::connect_timeout` -- `socket.connect(addr, 2000)`
/// 2. `set_nodelay(true)` -- `socket.setTcpNoDelay(true)`
/// 3. `set_read_timeout` -- `socket.setSoTimeout(10000)`
/// 4. Blocking TLS handshake via rustls `StreamOwned`
fn try_connect(
    host: &str,
    port: u16,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Result<FpssStream, crate::error::Error> {
    let addr = format!("{host}:{port}");

    // Resolve the hostname via the OS DNS resolver. This handles both IP
    // addresses and hostnames (e.g., "nj-a.thetadata.us:20000"). The
    // previous implementation used `SocketAddr::parse()` which only
    // accepts numeric IP addresses and would fail for DNS hostnames.
    let sock_addr = addr
        .to_socket_addrs()
        .map_err(|e| crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ConnectionRefused,
            message: format!("DNS resolution failed for '{addr}': {e}"),
        })?
        .next()
        .ok_or_else(|| crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ConnectionRefused,
            message: format!("DNS resolution returned no addresses for '{addr}'"),
        })?;

    // TCP connect with timeout
    let tcp = TcpStream::connect_timeout(&sock_addr, connect_timeout)?;

    // TCP_NODELAY = true (socket.setTcpNoDelay(true)).
    tcp.set_nodelay(true)?;

    // Read timeout (socket.setSoTimeout(10000)).
    tcp.set_read_timeout(Some(read_timeout))?;

    // TLS handshake (blocking) using rustls with webpki root certificates.
    let server_name =
        ServerName::try_from(host.to_owned()).map_err(|e| crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ConnectionRefused,
            message: format!("invalid TLS server name '{host}': {e}"),
        })?;

    let tls_conn = ClientConnection::new(tls_client_config(), server_name).map_err(|e| {
        crate::error::Error::Fpss {
            kind: crate::error::FpssErrorKind::ConnectionRefused,
            message: format!("TLS setup for {addr} failed: {e}"),
        }
    })?;

    // StreamOwned performs the TLS handshake lazily on first read/write.
    // The first write_frame (CREDENTIALS) will drive the handshake to completion.
    let tls_stream = StreamOwned::new(tls_conn, tcp);

    Ok(tls_stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rustls_crypto_provider_install_is_idempotent() {
        ensure_rustls_crypto_provider();
        ensure_rustls_crypto_provider();
    }

    #[test]
    fn production_config_has_four_fpss_hosts() {
        let config = crate::config::DirectConfig::production();
        assert_eq!(config.fpss.hosts.len(), 4);
        assert_eq!(
            config.fpss.hosts[0],
            ("nj-a.thetadata.us".to_string(), 20000)
        );
        assert_eq!(
            config.fpss.hosts[1],
            ("nj-a.thetadata.us".to_string(), 20001)
        );
        assert_eq!(
            config.fpss.hosts[2],
            ("nj-b.thetadata.us".to_string(), 20000)
        );
        assert_eq!(
            config.fpss.hosts[3],
            ("nj-b.thetadata.us".to_string(), 20001)
        );
    }

    #[test]
    fn connect_timeout_matches_java() {
        // Java parity reference: terminal hardcodes `socket.connect(addr, 2000)`.
        // Used as the default seed for `FpssConfig::connect_timeout_ms`; the
        // public knob now overrides this constant for callers who need to
        // dial in a different per-server connect deadline.
        assert_eq!(CONNECT_TIMEOUT_MS, 2_000);
    }

    /// `connect_to_servers` honours the caller-supplied connect timeout.
    ///
    /// We dial a non-routable RFC-5737 address. With a short
    /// `connect_timeout` the call must fail within that budget plus
    /// kernel scheduling slack; without timeout plumbing it would block
    /// for the kernel default (~75 s on Linux).
    ///
    /// This is the load-bearing assertion that `connect_timeout_ms`
    /// flows from `FpssConnectArgs` -> `connect_to_servers` ->
    /// `TcpStream::connect_timeout`.
    #[test]
    fn connect_to_servers_honors_configured_connect_timeout() {
        // 192.0.2.0/24 is the RFC-5737 TEST-NET-1 block — guaranteed
        // to be unroutable. A connect against any address in this
        // range must hit the configured deadline.
        let servers = [("192.0.2.1", 1)];
        let connect_timeout = Duration::from_millis(150);
        let read_timeout = Duration::from_millis(10_000);
        let start = std::time::Instant::now();
        let res = connect_to_servers(&servers, connect_timeout, read_timeout);
        let elapsed = start.elapsed();
        assert!(res.is_err(), "unroutable host must fail to connect");
        assert!(
            elapsed < Duration::from_millis(2_000),
            "connect_timeout = 150 ms but elapsed = {elapsed:?}; the knob is not wired"
        );
    }
}
