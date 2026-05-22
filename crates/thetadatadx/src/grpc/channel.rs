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

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use h2::client::{self, SendRequest};
use http::header::{HeaderName, HeaderValue};
use http::uri::{Authority, Scheme};
use http::{Method, Request, Uri};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::codec::{Codec, CodecError};
use super::decoder_pool::DecoderHandle;
use super::status::{Status, StatusParseError, GRPC_STATUS};
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
///
/// `#[non_exhaustive]` so downstream `match` arms must include a
/// wildcard; new variants land without breaking semver.
#[derive(Debug, Error)]
#[non_exhaustive]
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
    /// h2 stream-level error scoped to the specific stream this RPC
    /// opened. Covers `RST_STREAM` from the peer (any reason code:
    /// `CANCEL`, `REFUSED_STREAM`, `INTERNAL_ERROR`, etc.) plus any
    /// h2 library-detected protocol error that affects only this
    /// stream. The h2 connection itself is healthy and the next RPC
    /// on the same channel can succeed — the pool should *not*
    /// recycle the channel on this variant. Connection-level death
    /// (GOAWAY, IO failure, peer shutdown, open-phase connection
    /// drops) surfaces through [`Self::ConnectionClosed`] instead.
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
    /// The per-call deadline elapsed before the RPC completed. The
    /// `Duration` is the deadline that fired; the underlying h2 stream
    /// is dropped when this error surfaces, sending RST_STREAM to the
    /// server.
    #[error("rpc deadline {duration_ms}ms elapsed")]
    DeadlineExceeded {
        /// The deadline (in milliseconds) the caller supplied.
        duration_ms: u64,
    },
    /// Connection-level death — the h2 connection is no longer
    /// usable for any further RPC. Covers:
    /// - `GOAWAY` (either direction): the peer is refusing further
    ///   streams on this connection.
    /// - IO failure at the h2 transport layer: socket closed,
    ///   read/write returned an error, TLS layer terminated.
    /// - Connection drops observed during the open phase
    ///   (`ready()` / `send_request()` / `send_data()` failures
    ///   on a connection that died before admitting the stream).
    ///
    /// Distinct from per-stream resets (see [`Self::H2Stream`]):
    /// pool consumers should recycle the channel on this variant
    /// rather than retry on a dead transport.
    #[error("h2 connection closed: {0}")]
    ConnectionClosed(String),
}

/// One HTTP/2 connection to a gRPC server.
///
/// Owns the spawned h2 connection driver task via `connection_task`
/// (`Option<JoinHandle<()>>`), which is `!Clone`. `Channel` is never
/// cloned anywhere in the codebase — each pool entry holds one
/// `Channel` and recycles it on `ConnectionClosed`. New streams open
/// through the inner `SendRequest<Bytes>` clone, not through a
/// `Channel` clone.
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
    /// Per-frame decode ceiling propagated to every [`Codec`] this
    /// channel constructs. Mirrors `DirectConfig::mdds.max_message_size`
    /// so the configured limit is load-bearing on the in-house transport
    /// (the previous tonic-backed path honoured it at the tonic Channel
    /// builder; the in-house path threads it through here).
    max_message_size: usize,
    /// `:scheme` pseudo-header for every outbound request on this
    /// channel — `https` over TLS, `http` over plaintext h2c. gRPC
    /// pins the scheme to the underlying transport; strict L7 proxies
    /// and routers reject the mismatch.
    scheme: Scheme,
    /// Number of currently-open streams on this channel. Incremented
    /// at request dispatch, decremented when the [`ServerStreaming`]
    /// adapter is dropped. The [`super::ChannelPool`] uses this as a
    /// proxy for h2 stream-credit availability — picking the channel
    /// with the fewest in-flight streams avoids head-of-line blocking
    /// when one channel hits `MAX_CONCURRENT_STREAMS` saturation
    /// while others still have credit.
    ///
    /// `Arc` so the count survives both the `Channel` (for the pool's
    /// peek) and the in-flight [`ServerStreaming`] (for the decrement
    /// at drop time). Atomic with `Relaxed` ordering — strict
    /// sequential consistency is not required for load-balancing
    /// hints.
    in_flight: Arc<AtomicUsize>,
    /// Health flag — `true` once an upstream `ConnectionClosed` (h2
    /// `GOAWAY` / IO failure / vendor cascade) is observed on any
    /// RPC dispatched through this channel. The pool's
    /// [`super::ChannelPool::next`] picker treats dead channels as
    /// last-resort: they are skipped while at least one live channel
    /// remains in the pool, so subsequent RPCs route around the
    /// known-bad connection instead of repeatedly handing it out and
    /// observing the same terminal error (issue #577 #3). The flag
    /// is set by [`Self::mark_dead`] from the classifier hook in
    /// `crate::mdds::macros`; once set it stays set for the
    /// `Channel`'s lifetime — recycling is handled by reconstructing
    /// the pool slot, not by resurrecting a dead channel in place.
    ///
    /// `Arc` so the flag survives both the channel (read by the
    /// picker) and any `ServerStreaming` adapter that holds a clone
    /// (set by the classifier when a stream observes the cascade).
    /// `AtomicBool` with `Relaxed` reads / `Release` writes — the
    /// picker tolerates a slightly stale read (a live channel
    /// observed as live for one extra pick is fine; a dead channel
    /// observed as dead one pick later is fine too).
    dead: Arc<AtomicBool>,
    /// Decoder ring this channel routes zstd + protobuf decode work
    /// to. `None` means decode runs inline on the tokio reactor
    /// (legacy behaviour, retained for the unit-test paths that
    /// construct `Channel` without a pool wired up); `Some(handle)`
    /// hands every chunk to a dedicated decoder thread so the
    /// reactor never blocks on a multi-millisecond zstd payload.
    decoder: Option<DecoderHandle>,
    /// Handle on the background task that drives the h2 connection.
    ///
    /// Dropping `send_request` is sufficient for a clean shutdown —
    /// the h2 connection winds down and the task exits naturally. The
    /// handle is retained so `Channel::Drop` can call `.abort()` as a
    /// belt-and-braces guard against the case where the connection
    /// future is parked on a slow socket and would otherwise outlive
    /// the `Channel` for an unbounded interval (e.g. peer never sends
    /// the final GOAWAY ACK before the test runner moves on). The
    /// abort is idempotent — a task that already finished is a no-op.
    ///
    /// `Option` so [`Drop`] can take ownership without leaving a
    /// half-moved field; `Send` + `'static` because the task is
    /// dispatched onto the multi-thread tokio runtime.
    connection_task: Option<tokio::task::JoinHandle<()>>,
}

impl Channel {
    /// Open a plaintext HTTP/2 (h2c) connection to a gRPC server using
    /// the default per-frame decode ceiling
    /// ([`super::codec::DEFAULT_MAX_MESSAGE_SIZE`]).
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
        Self::connect_h2c_with_max_message_size(host, port, super::codec::DEFAULT_MAX_MESSAGE_SIZE)
            .await
    }

    /// Same as [`Self::connect_h2c`] with an explicit per-frame decode
    /// ceiling. Callers thread this from `DirectConfig::mdds.max_message_size`
    /// so the configured limit applies to every RPC dispatched on this
    /// channel; oversized response frames surface as
    /// [`ChannelError::Codec`] with [`super::codec::CodecError::FrameTooLarge`].
    ///
    /// # Errors
    ///
    /// Same as [`Self::connect_h2c`].
    pub async fn connect_h2c_with_max_message_size(
        host: &str,
        port: u16,
        max_message_size: usize,
    ) -> Result<Self, ChannelError> {
        let stream = TcpStream::connect((host, port))
            .await
            .map_err(|e| ChannelError::Tcp {
                host: host.to_string(),
                port,
                source: e,
            })?;
        let _ = stream.set_nodelay(true);
        Self::handshake(stream, host, port, max_message_size, Scheme::HTTP).await
    }

    /// Open a TLS-protected HTTP/2 connection to a gRPC server using
    /// the default per-frame decode ceiling.
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
        Self::connect_tls_with_max_message_size(
            host,
            port,
            tls,
            super::codec::DEFAULT_MAX_MESSAGE_SIZE,
        )
        .await
    }

    /// Same as [`Self::connect_tls`] with an explicit per-frame decode
    /// ceiling threaded from `DirectConfig::mdds.max_message_size`.
    ///
    /// # Errors
    ///
    /// Same as [`Self::connect_tls`].
    pub async fn connect_tls_with_max_message_size(
        host: &str,
        port: u16,
        tls: Arc<rustls::ClientConfig>,
        max_message_size: usize,
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
        Self::handshake(tls_stream, host, port, max_message_size, Scheme::HTTPS).await
    }

    /// Drive the h2 client handshake over an already-connected IO stream
    /// and spawn the connection-driver task.
    ///
    /// `scheme` is the `:scheme` pseudo-header value to use on every
    /// request — `https` for TLS transports, `http` for plaintext h2c.
    async fn handshake<IO>(
        io: IO,
        host: &str,
        port: u16,
        max_message_size: usize,
        scheme: Scheme,
    ) -> Result<Self, ChannelError>
    where
        IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (send_request, connection) = client::handshake(io)
            .await
            .map_err(|e| ChannelError::H2Handshake(e.to_string()))?;

        // Drive the h2 connection on a dedicated task. When the
        // `Channel` is dropped, `send_request` drops, which lets the
        // connection wind down naturally; the task exits at that point.
        // The handle is retained on `Channel::connection_task` so the
        // `Drop` impl can call `.abort()` as a belt-and-braces guard
        // against a connection future parked on a slow socket
        // outliving the `Channel` (cf. the test fixture pattern that
        // tears down the server before the client).
        let connection_task = tokio::spawn(async move {
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
            max_message_size,
            scheme,
            in_flight: Arc::new(AtomicUsize::new(0)),
            dead: Arc::new(AtomicBool::new(false)),
            decoder: None,
            connection_task: Some(connection_task),
        })
    }

    /// Attach a [`DecoderHandle`] to this channel so subsequent RPCs
    /// route their zstd + protobuf decode work to the pool's
    /// dedicated threads instead of running it inline on the tokio
    /// reactor. Returns `self` for builder-style chaining at
    /// `ChannelPool` construction.
    #[must_use]
    pub fn with_decoder(mut self, handle: DecoderHandle) -> Self {
        self.decoder = Some(handle);
        self
    }

    /// Number of currently-open streams on this channel. The pool
    /// uses this as a load-balancing hint: a channel with no
    /// in-flight streams is freshly available, a channel near its h2
    /// `MAX_CONCURRENT_STREAMS` ceiling is saturated. Relaxed load —
    /// the value is a hint, not a hard barrier.
    #[doc(hidden)]
    #[must_use]
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Whether this channel has been marked dead.
    ///
    /// A channel becomes dead when any RPC dispatched on it observes
    /// [`ChannelError::ConnectionClosed`] (h2 `GOAWAY` / IO failure /
    /// upstream cascade). The pool's
    /// [`super::ChannelPool::next`] picker treats dead channels as
    /// last-resort -- they are skipped while at least one live
    /// channel remains so subsequent RPCs route around the
    /// known-bad connection instead of repeatedly handing it out
    /// and observing the same terminal error (issue #577 #3).
    /// Relaxed load -- the picker tolerates a slightly stale read.
    #[doc(hidden)]
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Relaxed)
    }

    /// Mark this channel dead so future picker passes route around
    /// it. Idempotent -- a second call after the flag has already
    /// flipped is a no-op. The flag is `Release`-stored so the
    /// picker observes it as soon as it next reads the channel.
    ///
    /// Wired from the classifier hook in `crate::mdds::macros` --
    /// every `Error::Transport { kind: ConnectionClosed, .. }` on a
    /// streaming or unary attempt marks the source channel dead
    /// before the retry loop spins.
    pub(crate) fn mark_dead(&self) {
        self.dead.store(true, Ordering::Release);
    }

    /// Clone the death-flag handle so an outbound `ServerStreaming`
    /// adapter can mark the channel dead from its own classifier
    /// without holding a `&Channel` borrow. Crate-private --
    /// callers outside the gRPC layer use [`Self::mark_dead`] on the
    /// borrow they already hold via the `ChannelLease`.
    pub(crate) fn dead_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.dead)
    }

    /// Per-frame decode ceiling honoured by every RPC dispatched on
    /// this channel. Mirrors `DirectConfig::mdds.max_message_size`;
    /// each [`Codec`] this channel constructs uses this value rather
    /// than the codec module's compile-time default.
    #[must_use]
    pub const fn max_message_size(&self) -> usize {
        self.max_message_size
    }

    /// Take a pre-dispatch in-flight token. Used by
    /// [`super::ChannelPool::next`] to atomically reserve a slot on
    /// this channel at pick time, before the async dispatch future
    /// is even polled. Under burst contention this guarantees every
    /// concurrent `pool.next()` observer sees the prior reservations
    /// and routes around the loaded channel; without the
    /// pre-dispatch reservation a `join_all` batch of dispatches all
    /// see `in_flight = 0` and pin to the same channel.
    ///
    /// The returned token's `Drop` decrements the counter. The
    /// `ChannelLease` that holds it transfers ownership into the
    /// resulting `ServerStreaming` via an alternate dispatch entry
    /// point so the in-flight count drops back to a single
    /// commitment after the open path completes, not zero.
    pub(crate) fn reserve_in_flight(&self) -> InFlightToken {
        InFlightToken::new(Arc::clone(&self.in_flight))
    }

    /// Try to reserve a slot atomically with a load-balancing
    /// guardrail: commit only if the channel's in-flight count at
    /// the time of reservation is `<= expected_max`. Returns the
    /// pre-fetch_add value on success so the caller can verify the
    /// channel really was as lightly loaded as the picker thought.
    ///
    /// This is the load-balancing primitive [`super::ChannelPool::next`]
    /// uses to close the pick/reserve race: under true concurrency
    /// two tasks may both scan and both pick the same least-loaded
    /// channel before either reservation lands. The CAS-style
    /// retry pattern lets the loser bail out and re-scan rather
    /// than pin to a now-saturated channel.
    ///
    /// On failure the returned `usize` is the observed pre-bump
    /// count; the counter has already been incremented and then
    /// decremented (the increment is observable to other threads
    /// momentarily, which is acceptable for a load-balancing hint
    /// — `in_flight_count` is documented as a hint, not a barrier).
    pub(crate) fn try_reserve_in_flight(
        &self,
        expected_max: usize,
    ) -> Result<InFlightToken, usize> {
        let prior = self.in_flight.fetch_add(1, Ordering::AcqRel);
        if prior <= expected_max {
            // Reservation committed. The InFlightToken's Drop is
            // what releases it. Reconstruct the token from the
            // already-bumped counter — we do NOT want a second
            // fetch_add. `from_committed` is the in-module helper
            // for exactly this shape.
            Ok(InFlightToken::from_committed(Arc::clone(&self.in_flight)))
        } else {
            // Race lost — channel got busier than we thought. Roll
            // back the speculative reservation and let the caller
            // retry.
            self.in_flight.fetch_sub(1, Ordering::Release);
            Err(prior)
        }
    }

    /// `:scheme` pseudo-header this channel sends on every request —
    /// `"https"` over TLS, `"http"` over plaintext h2c. The gRPC
    /// HTTP/2 spec pins the scheme to the underlying transport so
    /// strict L7 proxies and routers accept the request.
    ///
    /// Hidden from the public docs — exposed for integration tests
    /// that need to confirm the channel records the right scheme for
    /// each transport.
    #[doc(hidden)]
    #[must_use]
    pub fn scheme_str(&self) -> &'static str {
        if self.scheme == Scheme::HTTPS {
            "https"
        } else {
            "http"
        }
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
        let frame = Codec::<Req, Resp>::encode(&req)?;
        self.server_streaming_frame::<Resp>(method, frame, None)
            .await
    }

    /// Same as [`Self::server_streaming`] with a per-call deadline.
    ///
    /// The deadline covers the entire RPC: opening the h2 stream,
    /// sending the request, receiving every DATA frame, and parsing
    /// the trailers. On elapse the underlying h2 stream is dropped
    /// (sending RST_STREAM to the server) and
    /// [`ChannelError::DeadlineExceeded`] surfaces on the next poll
    /// of the returned stream — or directly from this call if the
    /// open phase itself blew the deadline.
    ///
    /// # Errors
    ///
    /// Same as [`Self::server_streaming`], plus
    /// [`ChannelError::DeadlineExceeded`] when the deadline elapses
    /// during the open phase.
    pub async fn server_streaming_with_deadline<Req, Resp>(
        &self,
        method: &'static str,
        req: Req,
        deadline: Duration,
    ) -> Result<ServerStreaming<Resp>, ChannelError>
    where
        Req: prost::Message,
        Resp: prost::Message + Default,
    {
        let frame = Codec::<Req, Resp>::encode(&req)?;
        self.server_streaming_frame::<Resp>(method, frame, Some(deadline))
            .await
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
        deadline: Option<Duration>,
    ) -> Result<ServerStreaming<Resp>, ChannelError>
    where
        Resp: prost::Message + Default,
    {
        let start = tokio::time::Instant::now();
        let uri = Uri::builder()
            .scheme(self.scheme.clone())
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

        // gRPC spec: when the client enforces a per-call deadline, advertise
        // it via the `grpc-timeout` header so the server can short-circuit
        // and release resources rather than completing work the client will
        // discard. Format is `<positive-int><unit>` where unit is one of
        // `H` / `M` / `S` / `m` / `u` / `n` and the integer fits in 8 ASCII
        // digits.
        //   <https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md#requests>
        if let Some(d) = deadline {
            if let Some(encoded) = encode_grpc_timeout(d) {
                if let Ok(v) = HeaderValue::from_str(&encoded) {
                    headers.insert(HeaderName::from_static("grpc-timeout"), v);
                }
            }
        }

        // Stream is about to be dispatched on the wire — record it on
        // the channel's in-flight counter BEFORE awaiting `ready()` so
        // the pool's load-balancing picker sees every concurrent
        // dispatch the moment it commits, not just the ones that have
        // already cleared the h2 ready barrier. Without this
        // pre-increment a thundering herd of dispatches all observe
        // `in_flight = 0` on the same channel, race past `ready()`,
        // and pin to one h2 stream-credit window while the other pool
        // members stay idle. The token's `Drop` decrements the
        // counter; the error paths below short-circuit by dropping
        // the local `token`, the success path moves it into the
        // `ServerStreaming` adapter so the decrement fires when the
        // response stream ends.
        let token = InFlightToken::new(Arc::clone(&self.in_flight));

        // Wait for the h2 connection to admit a new stream. `ready()`
        // consumes the `SendRequest` and yields it back when the
        // connection has window space; the original clone has already
        // served its purpose so this is a clean ownership move.
        //
        // `ready()` / `send_request()` / `send_data()` all surface
        // their `h2::Error` through `classify_h2_error` so a `GOAWAY`
        // arriving during the open phase routes through
        // [`ChannelError::ConnectionClosed`] — letting the pool
        // recycle the connection rather than treating a dead transport
        // as a stream-level fault. The local helper also flips this
        // channel's death-flag on `ConnectionClosed` so the pool
        // picker routes subsequent RPCs around the dead channel
        // (issue #577 #3) without waiting for a streaming-phase
        // observation.
        let mark_on_close = |e: h2::Error| -> ChannelError {
            let classified = classify_h2_error(e);
            if matches!(classified, ChannelError::ConnectionClosed(_)) {
                self.mark_dead();
            }
            classified
        };
        let ready_fut = self.send_request.clone().ready();
        let mut sender = match deadline {
            Some(d) => match tokio::time::timeout(d, ready_fut).await {
                Ok(r) => r,
                Err(_) => return Err(deadline_error(d)),
            },
            None => ready_fut.await,
        }
        .map_err(mark_on_close)?;

        // `end_of_stream = false`: we'll send the data frame next.
        let (response_fut, mut send_body) = sender
            .send_request(request, false)
            .map_err(mark_on_close)?;

        // Single DATA frame carries the framed request payload, with
        // end_of_stream = true so the server can begin its response
        // immediately. Server-streaming RPCs send exactly one request
        // message.
        send_body
            .send_data(frame, true)
            .map_err(mark_on_close)?;

        let response = match deadline {
            Some(d) => {
                // Re-budget: subtract time already spent on the ready+send.
                let remaining = d.saturating_sub(start.elapsed());
                if remaining.is_zero() {
                    return Err(deadline_error(d));
                }
                match tokio::time::timeout(remaining, response_fut).await {
                    Ok(r) => r,
                    Err(_) => return Err(deadline_error(d)),
                }
            }
            None => response_fut.await,
        }
        .map_err(mark_on_close)?;

        if response.status() != http::StatusCode::OK {
            return Err(ChannelError::UnexpectedHttpStatus(
                response.status().as_u16(),
            ));
        }

        // Trailers-only encoding: a legal gRPC reply where the initial
        // HEADERS frame already carries `grpc-status` (and optional
        // `grpc-message`), END_STREAM is set on that frame, and no DATA
        // frames follow. Servers use this to refuse RPCs upfront
        // (e.g. Unauthenticated on an expired session). We must
        // classify it as `Rpc { status }` here — falling through to
        // the body stream would surface `StatusParse::Missing` because
        // h2's `poll_trailers` returns `Ok(None)` when the trailers
        // shared a frame with the headers.
        //
        // gRPC HTTP/2 wire spec:
        //   <https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md#responses>
        //   "Trailers-Only is permitted for calls that produce an
        //    immediate error."
        if response.headers().contains_key(GRPC_STATUS) {
            match Status::from_trailers(response.headers()) {
                Ok(status) if status.is_ok() => {
                    // Trailers-only OK is theoretically legal (a unary-
                    // shaped response with no payload). Drop the body
                    // and surface an already-closed stream so callers
                    // observe the OK terminus. The in-flight counter
                    // settles via `token`'s `Drop` on return: the
                    // local was built above (pre-`ready()`) and is
                    // *not* moved into the returned stream on this
                    // branch, so leaving scope here decrements the
                    // channel's counter exactly once.
                    drop(response.into_body());
                    return Ok(ServerStreaming::<Resp>::already_closed());
                }
                Ok(status) => return Err(ChannelError::Rpc { status }),
                Err(e) => return Err(ChannelError::StatusParse(e)),
            }
        }

        let recv_body = response.into_body();
        let codec = Codec::<(), Resp>::with_max_message_size(self.max_message_size);
        // Move the in-flight token into the ServerStreaming so its
        // Drop decrements the channel counter exactly when the
        // stream ends. If the channel has a decoder handle attached,
        // clone it onto the stream so per-chunk heavy decode work
        // routes to the dedicated thread pool. Also attach the
        // channel's death-flag handle (issue #577 #3) so a poll
        // that surfaces `ConnectionClosed` flips the flag and the
        // pool picker routes subsequent `next()`s to a live
        // channel.
        let stream = match deadline {
            Some(d) => {
                let remaining = d.saturating_sub(start.elapsed());
                if remaining.is_zero() {
                    return Err(deadline_error(d));
                }
                ServerStreaming::<Resp>::with_deadline_and_codec(recv_body, remaining, codec)
                    .with_in_flight_token(token)
                    .with_channel_dead_handle(self.dead_handle())
            }
            None => ServerStreaming::<Resp>::with_codec(recv_body, codec)
                .with_in_flight_token(token)
                .with_channel_dead_handle(self.dead_handle()),
        };
        Ok(if let Some(decoder) = self.decoder.as_ref() {
            stream.with_decoder(decoder.clone())
        } else {
            stream
        })
    }
}

impl Drop for Channel {
    /// Cancel the background h2 connection-driver task on drop.
    ///
    /// Dropping `send_request` (one of the `Channel`'s other fields)
    /// is sufficient for a clean shutdown of a healthy connection —
    /// the h2 connection winds down and the task returns from its
    /// `.await`. The explicit `.abort()` here is belt-and-braces for
    /// the case where the connection future is parked on a slow or
    /// half-closed socket and would otherwise outlive the `Channel`
    /// for an unbounded interval. `.abort()` is idempotent on a task
    /// that already finished, so the common clean-shutdown path pays
    /// at most one extra atomic on `Drop`.
    fn drop(&mut self) {
        if let Some(handle) = self.connection_task.take() {
            handle.abort();
        }
    }
}

/// Encode a [`Duration`] as a gRPC `grpc-timeout` header value.
///
/// Picks the smallest unit that fits the budget in at most 8 ASCII
/// digits, per the gRPC HTTP/2 spec:
///   `Timeout -> "grpc-timeout" TimeoutValue TimeoutUnit`
///   `TimeoutValue -> {positive integer, 8 digits max}`
///   `TimeoutUnit -> Hour / Minute / Second / Millisecond / Microsecond / Nanosecond`
///
/// Returns `None` for a zero deadline — callers should surface
/// `DeadlineExceeded` directly rather than send a header the server
/// will reject.
fn encode_grpc_timeout(d: Duration) -> Option<String> {
    if d.is_zero() {
        return None;
    }
    const MAX_VALUE: u128 = 99_999_999;
    let nanos = d.as_nanos();
    if nanos <= MAX_VALUE {
        return Some(format!("{nanos}n"));
    }
    let micros = d.as_micros();
    if micros <= MAX_VALUE {
        return Some(format!("{micros}u"));
    }
    let millis = d.as_millis();
    if millis <= MAX_VALUE {
        return Some(format!("{millis}m"));
    }
    let secs = d.as_secs() as u128;
    if secs <= MAX_VALUE {
        return Some(format!("{secs}S"));
    }
    let minutes = secs / 60;
    if minutes <= MAX_VALUE {
        return Some(format!("{minutes}M"));
    }
    let hours = secs / 3_600;
    // Hours saturate at the 8-digit ceiling; the spec accepts at most
    // 8 digits in any unit so anything larger than ~11_415 years just
    // pegs at the cap.
    Some(format!("{}H", hours.min(MAX_VALUE)))
}

/// Build a [`ChannelError::DeadlineExceeded`] from a `Duration` so the
/// two open-phase error sites stay in lockstep.
fn deadline_error(d: Duration) -> ChannelError {
    ChannelError::DeadlineExceeded {
        duration_ms: u64::try_from(d.as_millis()).unwrap_or(u64::MAX),
    }
}

/// Classify an [`h2::Error`] into the matching [`ChannelError`].
///
/// Connection-level failures surface as
/// [`ChannelError::ConnectionClosed`] so pool consumers can recycle
/// the channel:
/// - `GOAWAY` (either direction) — the connection refuses new streams.
/// - IO errors at the h2 layer — the transport is gone.
///
/// Per-stream `RST_STREAM` (`CANCEL`, `REFUSED_STREAM`, `INTERNAL_ERROR`,
/// any reason code) is *stream-level*: only the offending stream is
/// dead, the h2 connection itself is healthy and the next RPC on the
/// same channel can succeed. Misclassifying these as connection-level
/// would force the pool to recycle a still-good channel and burn retry
/// budgets. They surface as [`ChannelError::H2Stream`].
///
/// Everything else (library-detected protocol violations, user errors,
/// bare `Reason` values) is stream-level too — they don't justify
/// tearing the whole channel down.
///
/// HTTP/2 spec § 7 (Error Codes) is the canonical list of reason
/// codes; the per-stream / connection-level distinction here matches
/// the wire-level scope of each frame type.
/// Drop guard for the in-flight stream counter on [`Channel`].
///
/// Created at request dispatch (incrementing the counter) and moved
/// into the [`ServerStreaming`] for the response. When the stream is
/// dropped — either by exhausting the body, by an error, or by the
/// caller cancelling — the token's [`Drop`] decrements the counter so
/// the [`super::ChannelPool`] sees the channel return to a non-
/// saturated state.
#[derive(Debug)]
pub(crate) struct InFlightToken {
    counter: Arc<AtomicUsize>,
}

impl InFlightToken {
    /// Increment the counter and capture it as a drop guard.
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self { counter }
    }

    /// Construct a drop-guard from a counter the caller has already
    /// incremented. Used by [`Channel::try_reserve_in_flight`] where
    /// the `fetch_add` happened as part of the CAS-style commit
    /// check — incrementing again would double-count.
    pub(crate) fn from_committed(counter: Arc<AtomicUsize>) -> Self {
        Self { counter }
    }
}

impl Drop for InFlightToken {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

fn classify_h2_error(e: h2::Error) -> ChannelError {
    if e.is_go_away() || e.is_io() {
        return ChannelError::ConnectionClosed(e.to_string());
    }
    // `h2` raises a `User` library error with body "inactive stream"
    // when an operation targets a stream whose underlying connection
    // has already died (e.g. peer closed the TCP socket between
    // SETTINGS exchange and `send_request`). The stream is inactive
    // because the connection is gone — surface it as connection-level
    // so the pool recycles the channel rather than retry on a dead
    // socket.
    let msg = e.to_string();
    if msg.contains("inactive stream") {
        return ChannelError::ConnectionClosed(msg);
    }
    // is_reset() (per-stream) and everything else (library
    // protocol error, user error, bare Reason) — the h2
    // connection itself survives.
    ChannelError::H2Stream(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderName;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Drive the in-house `Channel` handshake over an in-memory IO pair,
    /// have it issue one server-streaming RPC, and capture the inbound
    /// request's `:scheme` pseudo-header.
    ///
    /// `scheme_in` is the scheme the handshake helper records on the
    /// `Channel`; the assertion confirms the same value appears on the
    /// wire. Using `tokio::io::duplex` keeps the test fully in-process
    /// — no listener, no TCP, no TLS fixture — so both schemes can be
    /// exercised by the same harness.
    /// Mirrors the codec module's default per-frame ceiling. Constant
    /// only needs to be reachable from the unit-test harness so the
    /// scheme assertions don't conflate parameter changes.
    const DEFAULT_MAX_FOR_TEST: usize = super::super::codec::DEFAULT_MAX_MESSAGE_SIZE;

    async fn assert_scheme_round_trip(scheme_in: Scheme, scheme_on_wire: &'static str) {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let saw_scheme = Arc::new(AtomicBool::new(false));

        // Mock server side: handshake, accept one request, assert
        // the inbound `:scheme`, send a trailers-only OK response.
        let server_saw = Arc::clone(&saw_scheme);
        let expected_scheme_on_wire = scheme_on_wire.to_string();
        let server_task = tokio::spawn(async move {
            let mut conn = h2::server::handshake(server_io)
                .await
                .expect("server handshake");
            let (request, mut respond) = conn
                .accept()
                .await
                .expect("server accepts a stream")
                .expect("accept returned a valid request");
            let scheme = request
                .uri()
                .scheme_str()
                .expect("inbound request carries a :scheme pseudo-header")
                .to_string();
            assert_eq!(
                scheme, expected_scheme_on_wire,
                "wire :scheme matches the scheme the client recorded"
            );
            server_saw.store(true, Ordering::SeqCst);
            // Drain the request body so flow-control accounting matches
            // a real server.
            let mut body = request.into_body();
            while let Some(chunk) = body.data().await {
                let chunk = chunk.expect("body chunk");
                let _ = body.flow_control().release_capacity(chunk.len());
            }
            // Reply with a trailers-only OK so the client's stream
            // terminates without waiting for DATA frames.
            let mut response = http::Response::new(());
            *response.status_mut() = http::StatusCode::OK;
            response.headers_mut().insert(
                http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/grpc+proto"),
            );
            response.headers_mut().insert(
                HeaderName::from_static("grpc-status"),
                HeaderValue::from_static("0"),
            );
            // end_of_stream=true makes this a trailers-only response —
            // the client's preflight (Finding 1) classifies it as OK
            // without reaching for the body.
            let _send = respond
                .send_response(response, true)
                .expect("server sends response head");
            // Drive the connection to completion so the client sees
            // the trailers-only response before the duplex is dropped.
            // poll_close drives all pending writes and accepts the
            // graceful shutdown handshake.
            let _ = std::future::poll_fn(|cx| {
                use std::pin::Pin;
                Pin::new(&mut conn).poll_closed(cx)
            })
            .await;
        });

        // Client side: drive the in-house Channel handshake with the
        // requested scheme.
        let channel =
            Channel::handshake(client_io, "127.0.0.1", 0, DEFAULT_MAX_FOR_TEST, scheme_in)
                .await
                .expect("client handshake");
        assert_eq!(
            channel.scheme_str(),
            scheme_on_wire,
            "channel records the same scheme it was constructed with"
        );

        // Fire one RPC and observe the trailers-only OK terminus.
        let stream = channel
            .server_streaming::<crate::proto::DataValueList, crate::proto::ResponseData>(
                "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
                crate::proto::DataValueList::default(),
            )
            .await
            .expect("rpc opens");
        use tokio_stream::StreamExt;
        let mut stream = std::pin::pin!(stream);
        while let Some(item) = stream.next().await {
            item.expect("trailers-only OK yields no errors before close");
        }

        // Drop the channel so the duplex closes and the server task can
        // exit; then join the server task to surface any assertion
        // panic from inside it.
        drop(channel);
        server_task.await.expect("server task completed");
        assert!(
            saw_scheme.load(Ordering::SeqCst),
            "server side observed the inbound :scheme pseudo-header"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn channel_sends_https_scheme_when_constructed_with_https() {
        // Confirms `Channel::handshake(..., Scheme::HTTPS)` records the
        // scheme on the `Channel` and emits `:scheme = https` on every
        // outbound request. The `connect_tls` constructor wires this
        // helper with the same scheme, so the test covers the TLS-
        // backed channel's behaviour without needing a real TLS
        // fixture (in-memory `tokio::io::duplex` substitutes for the
        // TLS-protected IO stream).
        assert_scheme_round_trip(Scheme::HTTPS, "https").await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn channel_sends_http_scheme_when_constructed_with_http() {
        // Symmetric coverage of the plaintext path: ensures the
        // scheme field flows through to the wire for both transports
        // and there's no accidental constant-override anywhere.
        assert_scheme_round_trip(Scheme::HTTP, "http").await;
    }

    /// Dropping the `Channel` must abort the spawned h2 connection-
    /// driver task. Without the `JoinHandle::abort()` in `Drop`, the
    /// task would survive until the underlying socket closed itself —
    /// which, under repeated connect/disconnect cycles in a long-
    /// running consumer, lets background tasks accumulate. The check
    /// here drives one `Channel::handshake` over `tokio::io::duplex`,
    /// drops the channel, then awaits a short interval. The audit
    /// invariant: `connection_task.is_finished()` is `true` by the
    /// time the join lands.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_channel_aborts_h2_connection_task() {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        // Server side: complete the h2 handshake and then park
        // indefinitely on a `pending` future. Without the abort, the
        // client's connection-driver task would also park forever.
        let server_task = tokio::spawn(async move {
            let mut conn = h2::server::handshake(server_io)
                .await
                .expect("server handshake");
            // Park; the duplex closing on the client side will surface
            // as `accept().await` returning None / Err, which we
            // ignore — the test is interested in whether the *client*
            // task gets reaped, not in the server side's behaviour.
            let _ = conn.accept().await;
        });

        let mut channel = Channel::handshake(
            client_io,
            "127.0.0.1",
            0,
            DEFAULT_MAX_FOR_TEST,
            Scheme::HTTP,
        )
        .await
        .expect("client handshake");
        // Snapshot the handle BEFORE drop so the test can `.is_finished()`
        // it after; the `Channel`'s `Drop` would otherwise consume the
        // only handle and we'd have no observable.
        let task = channel
            .connection_task
            .take()
            .expect("Channel constructed with a connection_task");
        // Snapshot abort_handle so we can observe completion after the
        // explicit drop without holding the JoinHandle (which would
        // race the drop).
        let abort_handle = task.abort_handle();
        // Re-park the task in a JoinHandle the test owns; restore the
        // channel field as `Some` so the `Drop` impl will abort it.
        channel.connection_task = Some(task);
        drop(channel);
        // Yield + short sleep until the abort signal lands on the
        // task. `abort()` is asynchronous (it schedules the task for
        // cancellation; the actual finish happens the next time the
        // runtime polls it), so a polite poll loop with a deadline is
        // the canonical pattern.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !abort_handle.is_finished() && std::time::Instant::now() < deadline {
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            abort_handle.is_finished(),
            "Channel::Drop must abort the spawned h2 connection-driver task",
        );
        server_task.abort();
        let _ = server_task.await;
    }

    #[test]
    fn encode_grpc_timeout_picks_smallest_fitting_unit() {
        // Nanoseconds when the budget fits in 8 digits.
        assert_eq!(
            encode_grpc_timeout(Duration::from_nanos(1)).as_deref(),
            Some("1n")
        );
        // 1ms = 1_000_000ns: still fits 8 digits as nanos.
        assert_eq!(
            encode_grpc_timeout(Duration::from_millis(1)).as_deref(),
            Some("1000000n")
        );
        // 1s = 1_000_000us: 7 digits as micros (1e9 nanos exceeds 8 digits).
        assert_eq!(
            encode_grpc_timeout(Duration::from_secs(1)).as_deref(),
            Some("1000000u")
        );
        // 1 minute = 60_000_000us: 8 digits as micros.
        assert_eq!(
            encode_grpc_timeout(Duration::from_secs(60)).as_deref(),
            Some("60000000u")
        );
        // 1 hour = 3_600_000ms: 7 digits as ms.
        assert_eq!(
            encode_grpc_timeout(Duration::from_secs(3_600)).as_deref(),
            Some("3600000m")
        );
        // 10 hours = 36_000_000ms: still fits 8 digits as ms, so ms wins.
        assert_eq!(
            encode_grpc_timeout(Duration::from_secs(36_000)).as_deref(),
            Some("36000000m")
        );
        // 1000 hours = 3_600_000s: fits 8 digits as seconds (ms overflows).
        assert_eq!(
            encode_grpc_timeout(Duration::from_secs(3_600_000)).as_deref(),
            Some("3600000S")
        );
        // 100_000_000 hours: encodes as hours (everything else overflows).
        assert!(encode_grpc_timeout(Duration::from_secs(360_000_000_000))
            .as_deref()
            .unwrap()
            .ends_with('H'));
    }

    #[test]
    fn encode_grpc_timeout_zero_is_none() {
        assert_eq!(encode_grpc_timeout(Duration::ZERO), None);
    }

    /// Drive a one-shot RPC over an in-memory duplex with a deadline set
    /// and assert the inbound headers carry a well-formed
    /// `grpc-timeout` value.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn channel_emits_grpc_timeout_when_deadline_set() {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let observed: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
        let observed_server = Arc::clone(&observed);

        let server_task = tokio::spawn(async move {
            let mut conn = h2::server::handshake(server_io)
                .await
                .expect("server handshake");
            let (request, mut respond) = conn
                .accept()
                .await
                .expect("server accepts a stream")
                .expect("accept returned a valid request");
            let header_value = request
                .headers()
                .get("grpc-timeout")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            *observed_server.lock().unwrap() = header_value;
            let mut body = request.into_body();
            while let Some(chunk) = body.data().await {
                let chunk = chunk.expect("body chunk");
                let _ = body.flow_control().release_capacity(chunk.len());
            }
            let mut response = http::Response::new(());
            *response.status_mut() = http::StatusCode::OK;
            response.headers_mut().insert(
                http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/grpc+proto"),
            );
            response.headers_mut().insert(
                HeaderName::from_static("grpc-status"),
                HeaderValue::from_static("0"),
            );
            let _send = respond
                .send_response(response, true)
                .expect("server sends response head");
            let _ = std::future::poll_fn(|cx| {
                use std::pin::Pin;
                Pin::new(&mut conn).poll_closed(cx)
            })
            .await;
        });

        let channel = Channel::handshake(
            client_io,
            "127.0.0.1",
            0,
            DEFAULT_MAX_FOR_TEST,
            Scheme::HTTP,
        )
        .await
        .expect("client handshake");
        let stream = channel
            .server_streaming_with_deadline::<crate::proto::DataValueList, crate::proto::ResponseData>(
                "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
                crate::proto::DataValueList::default(),
                Duration::from_secs(5),
            )
            .await
            .expect("rpc opens with deadline");
        use tokio_stream::StreamExt;
        let mut stream = std::pin::pin!(stream);
        while let Some(item) = stream.next().await {
            item.expect("trailers-only OK yields no errors before close");
        }
        drop(channel);
        server_task.await.expect("server task completed");

        let observed = observed.lock().unwrap().clone();
        let value = observed.expect("server observed a grpc-timeout header");
        // Smallest unit fitting a 5s budget is microseconds (5_000_000u).
        assert_eq!(value, "5000000u");
    }

    /// Symmetric coverage: with NO deadline, no `grpc-timeout` header
    /// should be emitted. Server-side timeout enforcement is opt-in via
    /// the deadline-bearing constructor.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn channel_omits_grpc_timeout_when_no_deadline() {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let observed: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
        let observed_server = Arc::clone(&observed);

        let server_task = tokio::spawn(async move {
            let mut conn = h2::server::handshake(server_io)
                .await
                .expect("server handshake");
            let (request, mut respond) = conn
                .accept()
                .await
                .expect("server accepts a stream")
                .expect("accept returned a valid request");
            let header_value = request
                .headers()
                .get("grpc-timeout")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            *observed_server.lock().unwrap() = header_value;
            let mut body = request.into_body();
            while let Some(chunk) = body.data().await {
                let chunk = chunk.expect("body chunk");
                let _ = body.flow_control().release_capacity(chunk.len());
            }
            let mut response = http::Response::new(());
            *response.status_mut() = http::StatusCode::OK;
            response.headers_mut().insert(
                http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/grpc+proto"),
            );
            response.headers_mut().insert(
                HeaderName::from_static("grpc-status"),
                HeaderValue::from_static("0"),
            );
            let _send = respond
                .send_response(response, true)
                .expect("server sends response head");
            let _ = std::future::poll_fn(|cx| {
                use std::pin::Pin;
                Pin::new(&mut conn).poll_closed(cx)
            })
            .await;
        });

        let channel = Channel::handshake(
            client_io,
            "127.0.0.1",
            0,
            DEFAULT_MAX_FOR_TEST,
            Scheme::HTTP,
        )
        .await
        .expect("client handshake");
        let stream = channel
            .server_streaming::<crate::proto::DataValueList, crate::proto::ResponseData>(
                "/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols",
                crate::proto::DataValueList::default(),
            )
            .await
            .expect("rpc opens without deadline");
        use tokio_stream::StreamExt;
        let mut stream = std::pin::pin!(stream);
        while let Some(item) = stream.next().await {
            item.expect("trailers-only OK yields no errors before close");
        }
        drop(channel);
        server_task.await.expect("server task completed");

        let observed = observed.lock().unwrap().clone();
        assert!(
            observed.is_none(),
            "no deadline means no grpc-timeout header; saw {observed:?}",
        );
    }
}
