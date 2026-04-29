//! TLS connection setup + auth handshake against a single MDDS legacy host.
//!
//! Splits cleanly into:
//! - [`connect_tls`] — async TCP + TLS handshake using SPKI pinning.
//! - [`login`] — CREDENTIALS + VERSION write, plus reading the
//!   SESSION_TOKEN + METADATA confirmation pair.
//!
//! The auth flow does **not** wait for a `CONNECTED` (msg=4) frame: live
//! observation shows the production server only emits SESSION_TOKEN +
//! METADATA on success and never the explicit CONNECTED frame. Treating
//! receipt of either pair as auth-success matches the vendor terminal's
//! own log line `[MDDS] CONNECTED: ..., Bundle: ...` which is constructed
//! from the METADATA payload.

use std::sync::Arc;

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

/// Established, authenticated MDDS connection.
pub(crate) struct AuthedSession {
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
    let cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(MddsSpkiVerifier::new())
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(cfg));
    let server_name: ServerName<'static> = ServerName::try_from(target.host.to_string())
        .map_err(|e| Error::Config(format!("invalid SNI {}: {e}", target.host)))?;

    let tcp = TcpStream::connect((target.host, target.port)).await?;
    tcp.set_nodelay(true)?;
    let tls = connector.connect(server_name, tcp).await?;
    Ok(tls)
}

/// Build the CREDENTIALS payload.
///
/// Layout (verified live):
/// ```text
/// [u8 0x00][u16 BE userlen][user_utf8][pass_utf8]
/// ```
/// The leading byte is `0x00`. The password length is implicit
/// (`payload.len() - 3 - userlen`).
fn build_credentials_payload(user: &str, pass: &str) -> Vec<u8> {
    let u = user.as_bytes();
    let p = pass.as_bytes();
    let userlen: u16 = u16::try_from(u.len()).expect("email cannot exceed 65535 bytes");
    let mut payload = Vec::with_capacity(3 + u.len() + p.len());
    payload.push(0x00);
    payload.extend_from_slice(&userlen.to_be_bytes());
    payload.extend_from_slice(u);
    payload.extend_from_slice(p);
    payload
}

/// Build the VERSION payload — `[u32 BE jsonlen][json_utf8]`.
///
/// The vendor terminal serialises every JVM system property; the server
/// only inspects the `terminal.version` key. We send a minimal map so the
/// server's MDC log shows a recognisable client identity without leaking
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

/// Run the CREDENTIALS + VERSION login on an already-established TLS stream.
///
/// On success returns the bundle string. On failure returns
/// `Error::FlatFilesUnavailable` with the underlying reason.
pub(crate) async fn login(
    stream: &mut TlsStream<TcpStream>,
    creds: &Credentials,
) -> Result<String, Error> {
    // CREDENTIALS frame.
    let creds_payload = build_credentials_payload(&creds.email, creds.password());
    write_frame(stream, msg::CREDENTIALS, -1, &creds_payload).await?;

    // VERSION frame.
    let version_payload = build_version_payload();
    write_frame(stream, msg::VERSION, -1, &version_payload).await?;

    // Read frames until we have either auth-success (SESSION_TOKEN +
    // METADATA) or a DISCONNECTED. The order observed live is
    // SESSION_TOKEN → METADATA; we accept either order defensively.
    let mut session_token_seen = false;
    let mut bundle: Option<String> = None;
    for _ in 0..6 {
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
                        "unexpected MDDS frame during login: msg={other} size={}",
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
        message: "MDDS auth did not return METADATA bundle".into(),
    })?;
    if !session_token_seen {
        return Err(Error::Auth {
            kind: AuthErrorKind::ServerError,
            message: "MDDS auth did not return SESSION_TOKEN".into(),
        });
    }
    Ok(bundle)
}

/// Convenience: connect to the first reachable host in a list, then auth.
pub(crate) async fn connect_and_login<'a>(
    hosts: &[MddsHost<'a>],
    creds: &Credentials,
) -> Result<AuthedSession, Error> {
    let mut last_err: Option<Error> = None;
    for host in hosts {
        match connect_tls(MddsHost {
            host: host.host,
            port: host.port,
        })
        .await
        {
            Ok(mut stream) => match login(&mut stream, creds).await {
                Ok(bundle) => return Ok(AuthedSession { stream, bundle }),
                Err(e) => last_err = Some(e),
            },
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| Error::Config("no MDDS hosts configured".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_payload_layout_is_stable() {
        // Verifies leading byte is 0x00 and userlen is BE u16.
        let p = build_credentials_payload("a@b.c", "pw");
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
}
