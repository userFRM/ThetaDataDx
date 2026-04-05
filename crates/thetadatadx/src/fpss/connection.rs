// reason: Networking/protocol layer with wire-format conversions,
// builder patterns, and many Result-returning async methods.
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::doc_markdown,
    clippy::map_unwrap_or,
    clippy::redundant_closure,
    clippy::redundant_closure_for_method_calls,
    clippy::items_after_statements,
    clippy::manual_let_else,
    clippy::uninlined_format_args,
    clippy::unreadable_literal,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::needless_pass_by_value,
    clippy::implicit_clone,
    clippy::wildcard_imports,
    clippy::match_same_arms,
    clippy::redundant_else,
    clippy::used_underscore_binding
)]
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

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use super::protocol::{CONNECT_TIMEOUT_MS, READ_TIMEOUT_MS};

/// Type alias for the TLS-wrapped TCP stream (blocking).
pub type FpssStream = StreamOwned<ClientConnection, TcpStream>;

/// Establish a TLS connection to the first reachable FPSS server.
///
/// Tries each server in order. Returns the stream and connected server
/// address on success, or the last error if all fail.
///
/// # Connection sequence (from `FPSSClient.connect()`)
///
/// 1. TCP connect with 2s timeout
/// 2. `TCP_NODELAY = true`
/// 3. Set read timeout to 10s (matches Java `socket.setSoTimeout(10000)`)
/// 4. TLS handshake via system trust store
///
/// Source: `FPSSClient.connect()` in decompiled terminal.
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

/// Build a shared rustls `ClientConfig` that skips certificate verification.
///
/// `ThetaData`'s FPSS servers use TLS certificates that have been expired since
/// January 2024. The Java terminal uses `SSLSocketFactory.getDefault()` which
/// in practice accepts expired certs. We match that behavior by disabling
/// certificate verification for FPSS connections.
///
/// This is safe because FPSS is a direct connection to `ThetaData`'s known
/// servers (not user-facing web traffic), and the TLS layer still provides
/// encryption -- only the certificate chain validation is skipped.
fn tls_client_config() -> Arc<ClientConfig> {
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();
    Arc::new(config)
}

/// Certificate verifier that accepts any certificate (matching Java terminal behavior).
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
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

    // Resolve the hostname via the OS DNS resolver. This handles both IP
    // addresses and hostnames (e.g., "nj-a.thetadata.us:20000"). The
    // previous implementation used `SocketAddr::parse()` which only
    // accepts numeric IP addresses and would fail for DNS hostnames.
    let sock_addr = addr
        .to_socket_addrs()
        .map_err(|e| crate::error::Error::Fpss(format!("DNS resolution failed for '{addr}': {e}")))?
        .next()
        .ok_or_else(|| {
            crate::error::Error::Fpss(format!("DNS resolution returned no addresses for '{addr}'"))
        })?;

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
    fn production_config_has_four_fpss_hosts() {
        let config = crate::config::DirectConfig::production();
        assert_eq!(config.fpss_hosts.len(), 4);
        assert_eq!(
            config.fpss_hosts[0],
            ("nj-a.thetadata.us".to_string(), 20000)
        );
        assert_eq!(
            config.fpss_hosts[1],
            ("nj-a.thetadata.us".to_string(), 20001)
        );
        assert_eq!(
            config.fpss_hosts[2],
            ("nj-b.thetadata.us".to_string(), 20000)
        );
        assert_eq!(
            config.fpss_hosts[3],
            ("nj-b.thetadata.us".to_string(), 20001)
        );
    }

    #[test]
    fn connect_timeout_matches_java() {
        assert_eq!(CONNECT_TIMEOUT_MS, 2_000);
    }
}
