//! gRPC channel over HTTP/2.
//!
//! A [`Channel`] owns one HTTP/2 connection to a gRPC server. The
//! connection is driven by a background tokio task spawned at
//! [`Channel::connect_tls`] / [`Channel::connect_h2c`] time; the task
//! is cancelled when the [`Channel`] is dropped.
//!
//! [`Channel::server_streaming`] sends a single server-streaming RPC:
//! it POSTs a framed prost request over a new HTTP/2 stream, then
//! returns a [`ServerStreaming`] that yields decoded response messages
//! and ends in a parsed [`super::Status`].
//!
//! The connection's `SendRequest<Bytes>` is cheap to clone — h2
//! serializes outbound streams internally — so a single [`Channel`]
//! safely multiplexes concurrent RPCs from many tasks.

use std::sync::Arc;

use bytes::Bytes;
use h2::client::{self, SendRequest};
use http::header::{HeaderName, HeaderValue};
use http::uri::{Authority, Scheme};
use http::{Method, Request, Uri};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::codec::{Codec, CodecError};
use super::status::StatusParseError;
use super::stream::ServerStreaming;

/// `content-type: application/grpc+proto` — the wire type for prost-encoded
/// gRPC bodies.
const CONTENT_TYPE_GRPC_PROTO: &str = "application/grpc+proto";
/// `te: trailers` — required by gRPC over HTTP/2 to opt into the
/// trailers-as-status contract.
const TE_TRAILERS: &str = "trailers";
/// User-agent reported in each `:user-agent` request pseudo-header.
const USER_AGENT_PREFIX: &str = "thetadatadx-grpc";

/// Errors raised by [`Channel`] construction and RPC dispatch.
#[derive(Debug, Error)]
pub enum ChannelError {
    /// Underlying TCP connect failed.
    #[error("tcp connect to {host}:{port}: {source}")]
    Tcp {
        /// Host portion of the connection target.
        host: String,
        /// Port portion of the connection target.
        port: u16,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// TLS handshake failed.
    #[error("tls handshake to {host}: {source}")]
    Tls {
        /// Host portion of the connection target.
        host: String,
        /// Underlying rustls error surfaced as an I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The host string was not a valid DNS name for rustls.
    #[error("invalid server name {host:?} for TLS")]
    InvalidServerName {
        /// Host portion the caller supplied.
        host: String,
    },
    /// h2 protocol handshake failed.
    #[error("h2 handshake: {0}")]
    H2Handshake(String),
    /// h2 stream-level error from the server.
    #[error("h2 stream: {0}")]
    H2Stream(String),
    /// Failed to build the `:path` URI for the RPC.
    #[error("invalid method path {path:?}: {message}")]
    InvalidPath {
        /// Path the caller supplied.
        path: String,
        /// Diagnostic message from `http::Uri::try_from`.
        message: String,
    },
    /// The codec returned an error decoding a frame.
    #[error("codec: {0}")]
    Codec(#[from] CodecError),
    /// The response trailers did not parse into a [`super::Status`].
    #[error("status parse: {0}")]
    StatusParse(#[from] StatusParseError),
    /// The server returned a non-OK gRPC status.
    #[error("rpc failed: {status}")]
    Rpc {
        /// The parsed status returned by the server.
        status: super::Status,
    },
    /// The server's HTTP/2 response carried no body — invariant violation
    /// per the gRPC HTTP/2 contract.
    #[error("server returned no response body")]
    EmptyResponse,
    /// The server's HTTP/2 status was non-200. gRPC pins HTTP status to
    /// 200 on every RPC, success or failure — failures travel through
    /// `grpc-status`, not HTTP status.
    #[error("expected HTTP 200, got {0}")]
    UnexpectedHttpStatus(u16),
}

/// One HTTP/2 connection to a gRPC server.
///
/// Clone-cheap — the inner `SendRequest<Bytes>` is itself an h2 channel
/// handle that serializes through the connection's stream multiplexer.
pub struct Channel {
    /// Outbound stream factory. Cloning this gives a second handle to
    /// the same h2 connection; new streams it opens share the connection.
    send_request: SendRequest<Bytes>,
    /// `:authority` pseudo-header. h2 takes a [`Uri`] per request but
    /// the authority part is shared across all RPCs on this channel.
    authority: Authority,
    /// Pre-built `user-agent` header value. Built once at connect time.
    user_agent: HeaderValue,
    /// Cached `content-type` header value (`application/grpc+proto`).
    content_type: HeaderValue,
    /// Cached `te` header value (`trailers`).
    te: HeaderValue,
}

impl Channel {
    /// Open a plaintext HTTP/2 (h2c) connection to a gRPC server.
    ///
    /// Intended for local-mock and sidecar deployments where TLS is
    /// terminated upstream. Production MDDS callers should use
    /// [`Channel::connect_tls`].
    ///
    /// # Errors
    ///
    /// Returns a [`ChannelError`] when the TCP connect or h2 handshake
    /// fails.
    pub async fn connect_h2c(host: &str, port: u16) -> Result<Self, ChannelError> {
        let stream = TcpStream::connect((host, port))
            .await
            .map_err(|e| ChannelError::Tcp {
                host: host.to_string(),
                port,
                source: e,
            })?;
        let _ = stream.set_nodelay(true);
        Self::handshake(stream, host, port).await
    }

    /// Open a TLS-protected HTTP/2 connection to a gRPC server.
    ///
    /// `tls` should already advertise `h2` in its ALPN list — the gRPC
    /// HTTP/2 spec requires the connection negotiate to `h2`.
    ///
    /// # Errors
    ///
    /// Returns a [`ChannelError`] when the TCP connect, TLS handshake,
    /// or h2 handshake fails.
    pub async fn connect_tls(
        host: &str,
        port: u16,
        tls: Arc<rustls::ClientConfig>,
    ) -> Result<Self, ChannelError> {
        let stream = TcpStream::connect((host, port))
            .await
            .map_err(|e| ChannelError::Tcp {
                host: host.to_string(),
                port,
                source: e,
            })?;
        let _ = stream.set_nodelay(true);
        let connector = TlsConnector::from(tls);
        let server_name =
            rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| {
                ChannelError::InvalidServerName {
                    host: host.to_string(),
                }
            })?;
        let tls_stream =
            connector
                .connect(server_name, stream)
                .await
                .map_err(|e| ChannelError::Tls {
                    host: host.to_string(),
                    source: e,
                })?;
        Self::handshake(tls_stream, host, port).await
    }

    /// Drive the h2 client handshake over an already-connected IO stream
    /// and spawn the connection-driver task.
    async fn handshake<IO>(io: IO, host: &str, port: u16) -> Result<Self, ChannelError>
    where
        IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (send_request, connection) = client::handshake(io)
            .await
            .map_err(|e| ChannelError::H2Handshake(e.to_string()))?;

        // Drive the h2 connection on a dedicated task. When the
        // `Channel` is dropped, `send_request` drops, which lets the
        // connection wind down naturally; the task exits at that point.
        tokio::spawn(async move {
            if let Err(err) = connection.await {
                tracing::debug!(error = %err, "in-house gRPC h2 connection ended");
            }
        });

        // `host:port` is fed back into the `:authority` pseudo-header.
        let authority_string = format!("{host}:{port}");
        let authority = Authority::try_from(authority_string.as_str()).map_err(|e| {
            ChannelError::InvalidPath {
                path: authority_string.clone(),
                message: e.to_string(),
            }
        })?;

        let user_agent = HeaderValue::from_str(&format!(
            "{USER_AGENT_PREFIX}/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .expect("crate version is ASCII");
        let content_type = HeaderValue::from_static(CONTENT_TYPE_GRPC_PROTO);
        let te = HeaderValue::from_static(TE_TRAILERS);

        Ok(Self {
            send_request,
            authority,
            user_agent,
            content_type,
            te,
        })
    }

    /// Issue a server-streaming RPC.
    ///
    /// `method` is the fully-qualified gRPC path including the leading
    /// `/`, e.g. `"/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols"`.
    /// `req` is encoded through the [`Codec`] and sent as a single
    /// length-prefixed frame; the returned [`ServerStreaming`] decodes
    /// response frames as the server emits them.
    ///
    /// # Errors
    ///
    /// Returns a [`ChannelError`] when the request cannot be built,
    /// the h2 stream cannot be opened, or the server's response head
    /// is malformed (non-200, wrong content-type, etc.).
    pub async fn server_streaming<Req, Resp>(
        &self,
        method: &'static str,
        req: Req,
    ) -> Result<ServerStreaming<Resp>, ChannelError>
    where
        Req: prost::Message,
        Resp: prost::Message + Default,
    {
        let frame = Codec::<Req, Resp>::encode(&req);
        self.server_streaming_frame::<Resp>(method, frame).await
    }

    /// Lower-level variant that sends a caller-prepared length-prefixed
    /// frame. Used by tests that need to control the frame bytes
    /// directly; production callers should use [`Self::server_streaming`].
    ///
    /// # Errors
    ///
    /// Same as [`Self::server_streaming`].
    pub async fn server_streaming_frame<Resp>(
        &self,
        method: &'static str,
        frame: Bytes,
    ) -> Result<ServerStreaming<Resp>, ChannelError>
    where
        Resp: prost::Message + Default,
    {
        let uri = Uri::builder()
            .scheme(Scheme::HTTP)
            .authority(self.authority.clone())
            .path_and_query(method)
            .build()
            .map_err(|e| ChannelError::InvalidPath {
                path: method.to_string(),
                message: e.to_string(),
            })?;

        let mut request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .body(())
            .expect("static request shape is well-formed");
        let headers = request.headers_mut();
        headers.insert(http::header::CONTENT_TYPE, self.content_type.clone());
        headers.insert(HeaderName::from_static("te"), self.te.clone());
        headers.insert(http::header::USER_AGENT, self.user_agent.clone());

        // Wait for the h2 connection to admit a new stream. `ready()`
        // consumes the `SendRequest` and yields it back when the
        // connection has window space; the original clone has already
        // served its purpose so this is a clean ownership move.
        let mut sender = self
            .send_request
            .clone()
            .ready()
            .await
            .map_err(|e| ChannelError::H2Stream(format!("send_request not ready: {e}")))?;

        // `end_of_stream = false`: we'll send the data frame next.
        let (response_fut, mut send_body) = sender
            .send_request(request, false)
            .map_err(|e| ChannelError::H2Stream(format!("send_request: {e}")))?;

        // Single DATA frame carries the framed request payload, with
        // end_of_stream = true so the server can begin its response
        // immediately. Server-streaming RPCs send exactly one request
        // message.
        send_body
            .send_data(frame, true)
            .map_err(|e| ChannelError::H2Stream(format!("send_data: {e}")))?;

        let response = response_fut
            .await
            .map_err(|e| ChannelError::H2Stream(format!("await response: {e}")))?;

        if response.status() != http::StatusCode::OK {
            return Err(ChannelError::UnexpectedHttpStatus(
                response.status().as_u16(),
            ));
        }

        let recv_body = response.into_body();
        Ok(ServerStreaming::<Resp>::new(recv_body))
    }
}
