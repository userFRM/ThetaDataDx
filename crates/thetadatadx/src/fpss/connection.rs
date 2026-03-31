//! TLS TCP connection to FPSS servers.
//!
//! # Transport (from decompiled Java -- `FPSSClient.java`)
//!
//! The Java terminal connects via `SSLSocket` (TLS over TCP) with:
//! - `TCP_NODELAY = true` (Nagle disabled for low latency)
//! - Connect timeout: 2 seconds
//! - Read timeout: 10 seconds
//! - Tries servers in order until one connects: `nj-a:20000`, `nj-a:20001`,
//!   `nj-b:20000`, `nj-b:20001`
//!
//! Source: `FPSSClient.connect()` and `FPSSClient.SERVERS` in decompiled terminal.
//!
//! # Rust implementation
//!
//! Uses `std::net::TcpStream` + `rustls::StreamOwned` for blocking TLS I/O,
//! matching the Java `SSLSocketFactory.createSocket()` behavior exactly.
//! No tokio, no async -- pure blocking I/O on `std::thread`.

use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use tdx_encoding::protocol::{CONNECT_TIMEOUT_MS, READ_TIMEOUT_MS, SERVERS};

/// Type alias for the TLS-wrapped TCP stream (blocking).
pub type FpssStream = StreamOwned<ClientConnection, TcpStream>;

/// Establish a TLS connection to the first reachable FPSS server.
///
/// Tries each server in [`SERVERS`] in order. Returns the stream and
/// connected server address on success, or the last error if all fail.
///
/// # Connection sequence (from `FPSSClient.connect()`)
///
/// 1. TCP connect with 2s timeout
/// 2. `TCP_NODELAY = true`
/// 3. Set read timeout to 10s (matches Java `socket.setSoTimeout(10000)`)
/// 4. TLS handshake via system trust store
///
/// Source: `FPSSClient.connect()` in decompiled terminal.
pub fn connect() -> Result<(FpssStream, String), crate::error::Error> {
    connect_to_servers(SERVERS)
}

/// Connect to a specific server list (for testing or custom endpoints).
///
/// Same behavior as [`connect`] but accepts an arbitrary server list.
pub fn connect_to_servers(
    servers: &[(&str, u16)],
) -> Result<(FpssStream, String), crate::error::Error> {
    let mut last_err = None;
    let connect_timeout = Duration::from_millis(CONNECT_TIMEOUT_MS);
    let read_timeout = Duration::from_millis(READ_TIMEOUT_MS);

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

    Err(last_err.unwrap_or_else(|| crate::error::Error::Fpss("no servers configured".to_string())))
}

/// Build a shared rustls `ClientConfig` with the webpki root certificates.
fn tls_client_config() -> Arc<ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Arc::new(config)
}

/// Attempt a single blocking TLS connection to one server.
///
/// # Steps (matching `FPSSClient.connect()`)
///
/// 1. `TcpStream::connect_timeout` -- matches Java `socket.connect(addr, 2000)`
/// 2. `set_nodelay(true)` -- matches Java `socket.setTcpNoDelay(true)`
/// 3. `set_read_timeout` -- matches Java `socket.setSoTimeout(10000)`
/// 4. Blocking TLS handshake via rustls `StreamOwned`
fn try_connect(
    host: &str,
    port: u16,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Result<FpssStream, crate::error::Error> {
    let addr = format!("{host}:{port}");
    let sock_addr: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| crate::error::Error::Fpss(format!("invalid address '{addr}': {e}")))?;

    // TCP connect with timeout
    let tcp = TcpStream::connect_timeout(&sock_addr, connect_timeout)?;

    // TCP_NODELAY = true (matches Java: socket.setTcpNoDelay(true))
    tcp.set_nodelay(true)?;

    // Read timeout (matches Java: socket.setSoTimeout(10000))
    tcp.set_read_timeout(Some(read_timeout))?;

    // TLS handshake (blocking) using rustls with webpki root certificates.
    let server_name = ServerName::try_from(host.to_owned())
        .map_err(|e| crate::error::Error::Fpss(format!("invalid TLS server name '{host}': {e}")))?;

    let tls_conn = ClientConnection::new(tls_client_config(), server_name)
        .map_err(|e| crate::error::Error::Fpss(format!("TLS setup for {addr} failed: {e}")))?;

    // StreamOwned performs the TLS handshake lazily on first read/write.
    // The first write_frame (CREDENTIALS) will drive the handshake to completion.
    let tls_stream = StreamOwned::new(tls_conn, tcp);

    Ok(tls_stream)
}

/// Connect to a specific server address (for testing or when the caller
/// already knows which server to use).
///
/// This bypasses the server rotation logic.
pub fn connect_to(host: &str, port: u16) -> Result<FpssStream, crate::error::Error> {
    let connect_timeout = Duration::from_millis(CONNECT_TIMEOUT_MS);
    let read_timeout = Duration::from_millis(READ_TIMEOUT_MS);
    try_connect(host, port, connect_timeout, read_timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_list_has_four_entries() {
        // Sanity check: the hardcoded server list from Java has 4 entries.
        assert_eq!(SERVERS.len(), 4);
        assert_eq!(SERVERS[0], ("nj-a.thetadata.us", 20000));
        assert_eq!(SERVERS[1], ("nj-a.thetadata.us", 20001));
        assert_eq!(SERVERS[2], ("nj-b.thetadata.us", 20000));
        assert_eq!(SERVERS[3], ("nj-b.thetadata.us", 20001));
    }

    #[test]
    fn connect_timeout_matches_java() {
        assert_eq!(CONNECT_TIMEOUT_MS, 2_000);
    }
}
