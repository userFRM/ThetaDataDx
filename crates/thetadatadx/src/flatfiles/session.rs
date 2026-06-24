//! TLS connection setup + auth handshake against a single MDDS legacy host.
//!
//! Splits cleanly into:
//! - [`connect_tls`] — async TCP + TLS handshake using SPKI pinning.
//! - [`login`] — CREDENTIALS + VERSION write, plus reading the
//!   SESSION_TOKEN + METADATA confirmation pair.
//!
//! The auth flow does **not** wait for a `CONNECTED` (msg=4) frame: the
//! production server only emits SESSION_TOKEN +
//! METADATA on success and never the explicit CONNECTED frame, so receipt
//! of either pair is treated as auth-success. The `[MDDS] CONNECTED: ...,
//! Bundle: ...` status line is constructed from the METADATA payload.

use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::ClientConfig;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use crate::auth::Credentials;
use crate::error::{AuthErrorKind, Error};
use crate::flatfiles::framing::{msg, read_frame, write_frame, Frame};
use crate::flatfiles::mdds_spki::MddsSpkiVerifier;
use crate::flatfiles::types::FlatFilesUnavailableReason;
use crate::fpss::protocol::build_login_payload;

/// Established, authenticated MDDS connection.
pub(crate) struct AuthedSession {
    /// Authenticated TLS stream, ready to carry FLAT_FILE request/response
    /// frames.
    pub stream: TlsStream<TcpStream>,
    /// Bundle string from the METADATA frame, e.g.
    /// `"STOCK.STANDARD, OPTION.PRO, INDEX.FREE"`. Useful for surfacing in
    /// debug logs; does not affect protocol behaviour.
    pub bundle: String,
}

/// Hostname + port pair.
pub(crate) struct MddsHost<'a> {
    pub host: &'a str,
    pub port: u16,
}

/// Open a TLS connection to a single MDDS host with SPKI pinning.
pub(crate) async fn connect_tls(target: MddsHost<'_>) -> Result<TlsStream<TcpStream>, Error> {
    // Build the config with an explicit ring provider so the handshake needs
    // no process-global default. ring is the sole provider in the dep graph.
    let cfg =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(MddsSpkiVerifier::new())
            .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(cfg));
    let server_name: ServerName<'static> =
        ServerName::try_from(target.host.to_string()).map_err(|e| {
            Error::config_invalid("mdds.sni", format!("invalid SNI {}: {e}", target.host))
        })?;

    let tcp = TcpStream::connect((target.host, target.port)).await?;
    tcp.set_nodelay(true)?;
    let tls = connector.connect(server_name, tcp).await?;
    Ok(tls)
}

/// Build the VERSION payload — `[u32 BE jsonlen][json_utf8]`.
///
/// The JVM terminal serialises a large host-property map; the server
/// only inspects the `terminal.version` key. A minimal map keeps the
/// server's MDC log showing a recognisable client identity without leaking
/// host details.
fn build_version_payload() -> Vec<u8> {
    // Hand-rolled to avoid pulling in serde_json for a 1-line constant. The
    // exact JSON shape the vendor sends is `{"key":"value", ...}` — we
    // emit a single key.
    let json = br#"{"terminal.version":"1.8.6-A","client":"thetadatadx"}"#;
    let mut buf = Vec::with_capacity(4 + json.len());
    buf.extend_from_slice(&u32::try_from(json.len()).unwrap().to_be_bytes());
    buf.extend_from_slice(json);
    buf
}

/// Maximum number of frames consumed during the legacy MDDS handshake
/// before we surface an `Auth(Timeout)` failure. The server sends at
/// most 4 frames on a successful login (SESSION_TOKEN, METADATA,
/// optional CONNECTED, optional PING) plus a slack of 2 to absorb any
/// late server heartbeat without erroring the auth path.
const LOGIN_FRAME_BUDGET: usize = 6;

/// Run the CREDENTIALS + VERSION login on an already-established TLS stream.
///
/// On success returns the bundle string. On failure returns
/// `Error::FlatFilesUnavailable` with the underlying reason.
pub(crate) async fn login(
    stream: &mut TlsStream<TcpStream>,
    creds: &Credentials,
) -> Result<String, Error> {
    // CREDENTIALS frame.
    let creds_payload = build_login_payload(creds)?;
    write_frame(stream, msg::CREDENTIALS, -1, &creds_payload).await?;

    // VERSION frame.
    let version_payload = build_version_payload();
    write_frame(stream, msg::VERSION, -1, &version_payload).await?;

    // Read frames until we have either auth-success (SESSION_TOKEN +
    // METADATA) or a DISCONNECTED. The order observed live is
    // SESSION_TOKEN → METADATA; we accept either order defensively.
    let mut session_token_seen = false;
    let mut bundle: Option<String> = None;
    for _ in 0..LOGIN_FRAME_BUDGET {
        let frame: Frame = read_frame(stream).await?;
        match frame.msg {
            msg::SESSION_TOKEN => session_token_seen = true,
            msg::METADATA => {
                bundle = Some(String::from_utf8_lossy(&frame.payload).into_owned());
            }
            msg::CONNECTED => {
                // Older server builds emit this; treat as confirmation.
                session_token_seen = true;
            }
            msg::PING => {
                // Server heartbeat during auth — ignore.
            }
            msg::DISCONNECTED => {
                let reason_code = if frame.payload.len() >= 2 {
                    u16::from_be_bytes([frame.payload[0], frame.payload[1]])
                } else {
                    0
                };
                let _ = stream.shutdown().await;
                return Err(Error::FlatFilesUnavailable(
                    FlatFilesUnavailableReason::AuthRejected { reason_code },
                ));
            }
            other => {
                return Err(Error::Auth {
                    kind: AuthErrorKind::ServerError,
                    message: format!(
                        "unexpected historical frame during login: msg={other} size={}",
                        frame.payload.len()
                    ),
                });
            }
        }
        if session_token_seen && bundle.is_some() {
            break;
        }
    }
    let bundle = bundle.ok_or_else(|| Error::Auth {
        kind: AuthErrorKind::ServerError,
        message: "historical auth did not return METADATA bundle".into(),
    })?;
    if !session_token_seen {
        return Err(Error::Auth {
            kind: AuthErrorKind::ServerError,
            message: "historical auth did not return SESSION_TOKEN".into(),
        });
    }
    Ok(bundle)
}

/// Convenience: connect to the first reachable host in a list, then auth.
///
/// Retries only on transient connect-layer failures (TCP, TLS, I/O). A
/// semantic server rejection — the credentials were rejected, the auth
/// frame was malformed, the server emitted a `DISCONNECTED` — is
/// short-circuited: replaying it across every MDDS host is pointless,
/// risks rate-limiting the account, and the original error already
/// describes what the server objected to.
///
/// `connect_timeout` bounds the combined TCP + TLS handshake **and** auth
/// exchange for a single host. A host that accepts the socket but never
/// finishes the TLS handshake, or never returns the auth frames, would
/// otherwise block the whole request forever; on expiry the attempt
/// moves on to the next host (or surfaces a transient timeout the retry
/// ladder reconnects on).
pub(crate) async fn connect_and_login<'a>(
    hosts: &[MddsHost<'a>],
    creds: &Credentials,
    connect_timeout: Duration,
) -> Result<AuthedSession, Error> {
    let mut last_err: Option<Error> = None;
    for host in hosts {
        let attempt = async {
            let mut stream = connect_tls(MddsHost {
                host: host.host,
                port: host.port,
            })
            .await?;
            let bundle = login(&mut stream, creds).await?;
            Ok::<AuthedSession, Error>(AuthedSession { stream, bundle })
        };
        match tokio::time::timeout(connect_timeout, attempt).await {
            Ok(Ok(session)) => return Ok(session),
            Ok(Err(e)) if is_terminal_login_error(&e) => return Err(e),
            Ok(Err(e)) => last_err = Some(e),
            Err(_) => {
                // Treated as a transient I/O timeout so the retry ladder
                // reconnects; a stuck handshake on one host should not
                // fail the whole request without a reconnect attempt.
                last_err = Some(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "historical connect/login to {}:{} timed out after {}s",
                        host.host,
                        host.port,
                        connect_timeout.as_secs()
                    ),
                )));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| Error::config_missing("mdds.hosts")))
}

/// A login error the server has authoritatively decided — no point
/// retrying against another host.
fn is_terminal_login_error(err: &Error) -> bool {
    matches!(err, Error::FlatFilesUnavailable(_) | Error::Auth { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_payload_layout_is_stable() {
        // Verifies leading byte is 0x00 and userlen is BE u16 for a
        // password credential routed through the login-payload dispatcher.
        let creds = Credentials::new("a@b.c", "pw");
        let p = build_login_payload(&creds).expect("valid creds");
        assert_eq!(p[0], 0x00);
        assert_eq!(&p[1..3], &5u16.to_be_bytes());
        assert_eq!(&p[3..8], b"a@b.c");
        assert_eq!(&p[8..10], b"pw");
        assert_eq!(p.len(), 10);
    }

    #[test]
    fn version_payload_has_length_prefix() {
        let p = build_version_payload();
        let len = u32::from_be_bytes(p[0..4].try_into().unwrap()) as usize;
        assert_eq!(p.len(), 4 + len);
        // first character of the JSON body must be '{'.
        assert_eq!(p[4], b'{');
    }

    #[tokio::test]
    async fn connect_and_login_times_out_on_unreachable_host() {
        use crate::auth::Credentials;
        use std::time::Instant;

        // 192.0.2.1 is TEST-NET-1 (RFC 5737): guaranteed non-routable, so
        // the TCP connect stalls until our timeout fires rather than
        // failing fast. Without the connect bound this call would hang.
        let hosts = [MddsHost {
            host: "192.0.2.1",
            port: 12_000,
        }];
        let creds = Credentials::new("user@example.com", "pw");
        let budget = Duration::from_millis(150);
        let started = Instant::now();
        let result = connect_and_login(&hosts, &creds, budget).await;
        let elapsed = started.elapsed();

        // `AuthedSession` is not `Debug`; match the variants by hand so a
        // success path fails the test without needing a `Debug` bound.
        let err = match result {
            Ok(_) => panic!("connect to a black-hole host must fail, not succeed"),
            Err(e) => e,
        };
        // The bound must actually fire — a hung host cannot block forever.
        assert!(
            elapsed < Duration::from_secs(5),
            "connect must abandon the host near its timeout, took {elapsed:?}"
        );
        // Classified transient so the retry ladder reconnects.
        assert!(
            error_is_transient_for_test(&err),
            "connect timeout must be transient, got {err:?}"
        );
    }

    /// Mirror of the request-layer transient classifier for the single
    /// case this test exercises: a timed-out connect surfaces as
    /// `Error::Io`, which the retry loop treats as transient.
    fn error_is_transient_for_test(err: &Error) -> bool {
        matches!(err, Error::Io(_))
    }
}
