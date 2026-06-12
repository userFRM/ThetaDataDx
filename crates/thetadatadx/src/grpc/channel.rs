//! gRPC channel over the reference Rust gRPC stack.
//!
//! A [`Channel`] owns one `tonic::transport::Channel` — one HTTP/2
//! connection to a gRPC server, driven by the underlying stack's
//! connection task. When the connection dies (GOAWAY, IO failure), the
//! underlying stack reconnects lazily on the next dispatched RPC; the
//! in-flight RPC observes [`ChannelError::ConnectionClosed`] and the
//! caller's retry shell (`crate::mdds::macros::classify_error`)
//! re-dispatches onto the recovered connection or a sibling pool member.
//!
//! [`Channel::server_streaming`] sends a single server-streaming RPC and
//! returns a [`ServerStreaming`] that yields decoded response messages.
//! Per-chunk payload decode (zstd + prost `DataTable`) runs inline on
//! the request task — the measured-fastest shape for this workload.
//!
//! # Connector
//!
//! TLS rides through a custom connector ([`GrpcConnector`]) so the
//! existing single-provider rustls configuration (`ring`, webpki roots,
//! `h2` ALPN) is reused verbatim — the underlying stack's own TLS
//! features stay disabled and the dependency graph keeps exactly one
//! `CryptoProvider`. The same connector serves plaintext h2c for mock
//! servers and sidecar deployments.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use http::uri::{PathAndQuery, Scheme, Uri};
use hyper_util::rt::TokioIo;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::status::Status;
use super::stream::ServerStreaming;

/// User-agent reported on every request. The underlying stack appends
/// its own product token after this prefix.
const USER_AGENT_PREFIX: &str = "thetadatadx-grpc";

/// HTTP/2 session tuning threaded from `DirectConfig::mdds` —
/// flow-control windows and keepalive cadence. The short channel
/// constructors use [`ChannelTuning::default`] (the HTTP/2 spec
/// windows, 30 s / 10 s keepalive — the values production config
/// defaults to); `MddsClient::connect` threads the operator's
/// configured values through [`Channel::connect_tls_tuned`] /
/// [`Channel::connect_h2c_tuned`].
#[derive(Debug, Clone, Copy)]
pub struct ChannelTuning {
    /// Initial per-stream flow-control window, in bytes. Mirrors
    /// `MddsConfig::window_size_kb`.
    pub initial_stream_window_size: u32,
    /// Initial connection-level flow-control window, in bytes.
    /// Mirrors `MddsConfig::connection_window_size_kb`.
    pub initial_connection_window_size: u32,
    /// Interval between HTTP/2 keepalive PING frames. Mirrors
    /// `MddsConfig::keepalive_secs`.
    pub keepalive_interval: Duration,
    /// How long to wait for a keepalive PING acknowledgement before
    /// declaring the connection dead. Mirrors
    /// `MddsConfig::keepalive_timeout_secs`.
    pub keepalive_timeout: Duration,
}

impl Default for ChannelTuning {
    fn default() -> Self {
        Self {
            // HTTP/2 spec initial windows (64 KiB) — the same wire
            // shape the production config defaults to and the shape
            // the transport comparison was measured at.
            initial_stream_window_size: 64 * 1024,
            initial_connection_window_size: 64 * 1024,
            keepalive_interval: Duration::from_secs(30),
            keepalive_timeout: Duration::from_secs(10),
        }
    }
}

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
    /// The HTTP/2 session could not be established over an already-
    /// connected transport (handshake or SETTINGS exchange failed).
    #[error("h2 handshake: {0}")]
    H2Handshake(String),
    /// h2 stream-level error scoped to the specific stream this RPC
    /// opened. Covers `RST_STREAM` from the peer (any reason code:
    /// `CANCEL`, `REFUSED_STREAM`, `INTERNAL_ERROR`, etc.). The h2
    /// connection itself is healthy and the next RPC on the same
    /// channel can succeed. Connection-level death surfaces through
    /// [`Self::ConnectionClosed`] instead.
    #[error("h2 stream: {0}")]
    H2Stream(String),
    /// Failed to build the request URI or `:path` for the RPC.
    #[error("invalid method path {path:?}: {message}")]
    InvalidPath {
        /// Path the caller supplied.
        path: String,
        /// Diagnostic message from the URI parser.
        message: String,
    },
    /// The server returned a non-OK gRPC status.
    #[error("rpc failed: {status}")]
    Rpc {
        /// The parsed status returned by the server.
        status: Status,
    },
    /// The per-call deadline elapsed before the RPC completed. The
    /// underlying h2 stream is dropped when this error surfaces,
    /// sending RST_STREAM to the server.
    #[error("rpc deadline {duration_ms}ms elapsed")]
    DeadlineExceeded {
        /// The deadline (in milliseconds) the caller supplied.
        duration_ms: u64,
    },
    /// Connection-level death — the HTTP/2 connection that carried (or
    /// was about to carry) this RPC is no longer usable. Covers
    /// `GOAWAY` in either direction, IO failure at the transport
    /// layer, peer shutdown, and reconnect-path connect failures.
    ///
    /// The underlying stack reacts by lazily reconnecting on the next
    /// dispatched RPC; the caller's retry shell re-dispatches and
    /// observes the fresh connection.
    #[error("h2 connection closed: {0}")]
    ConnectionClosed(String),
}

/// One gRPC channel to a server.
///
/// Wraps a `tonic::transport::Channel` (one HTTP/2 connection with
/// lazy in-place reconnect) plus the per-channel state the pool and
/// dispatch paths need: the per-frame decode ceiling, the `:scheme`
/// the channel speaks, and the in-flight stream counter the
/// [`super::ChannelPool`] uses for least-loaded picks.
pub struct Channel {
    /// Underlying gRPC channel. Cloning is cheap (a handle onto the
    /// shared connection); one clone is taken per dispatched RPC.
    inner: tonic::transport::Channel,
    /// Per-frame decode ceiling propagated to every RPC dispatched on
    /// this channel. Mirrors `DirectConfig::mdds.max_message_size`;
    /// response frames above it are rejected by the decode layer
    /// before allocation.
    max_message_size: usize,
    /// `:scheme` this channel speaks — `https` over TLS, `http` over
    /// plaintext h2c. Derived from the connect constructor; the
    /// underlying stack pins the request pseudo-header to the
    /// endpoint URI's scheme, so the field exists only for the
    /// test-surface accessor ([`Self::scheme_str`]).
    #[cfg(any(test, feature = "__test-helpers"))]
    scheme: Scheme,
    /// Number of currently-open streams on this channel. Incremented
    /// at request dispatch, decremented when the [`ServerStreaming`]
    /// adapter is dropped. The [`super::ChannelPool`] uses this as a
    /// load-balancing hint — picking the channel with the fewest
    /// in-flight streams avoids head-of-line blocking when one
    /// channel is saturated while others have credit.
    ///
    /// `Arc` so the count survives both the `Channel` (for the pool's
    /// peek) and the in-flight [`ServerStreaming`] (for the decrement
    /// at drop time). Relaxed ordering — strict sequential
    /// consistency is not required for load-balancing hints.
    in_flight: Arc<AtomicUsize>,
}

impl Channel {
    /// Open a plaintext HTTP/2 (h2c) connection to a gRPC server using
    /// the default per-frame decode ceiling.
    ///
    /// Intended for local-mock and sidecar deployments where TLS is
    /// terminated upstream. Production MDDS callers should use
    /// [`Channel::connect_tls_with_max_message_size`].
    ///
    /// # Errors
    ///
    /// Returns a [`ChannelError`] when the TCP connect or HTTP/2
    /// session establishment fails.
    ///
    /// Reachable only when the `__test-helpers` private feature is
    /// enabled; production callers use the `_with_max_message_size`
    /// variant exclusively.
    #[cfg(feature = "__test-helpers")]
    pub async fn connect_h2c(host: &str, port: u16) -> Result<Self, ChannelError> {
        Self::connect_h2c_with_max_message_size(host, port, DEFAULT_MAX_MESSAGE_SIZE).await
    }

    /// Same as [`Self::connect_h2c`] with an explicit per-frame decode
    /// ceiling; oversized response frames are rejected by the decode
    /// layer and surface as [`ChannelError::Rpc`] with the canonical
    /// `OutOfRange` status the underlying stack emits for over-limit
    /// messages.
    ///
    /// Reachable only under the `__test-helpers` private feature —
    /// production callers go through [`Self::connect_h2c_tuned`] so the
    /// configured HTTP/2 session tuning applies.
    ///
    /// # Errors
    ///
    /// Same as [`Self::connect_h2c`].
    #[cfg(feature = "__test-helpers")]
    pub async fn connect_h2c_with_max_message_size(
        host: &str,
        port: u16,
        max_message_size: usize,
    ) -> Result<Self, ChannelError> {
        Self::connect(host, port, None, max_message_size, ChannelTuning::default()).await
    }

    /// Open a plaintext (h2c) connection with an explicit per-frame
    /// decode ceiling and HTTP/2 session tuning (flow-control windows,
    /// keepalive cadence), both threaded from `DirectConfig::mdds`.
    ///
    /// # Errors
    ///
    /// Returns a [`ChannelError`] when the TCP connect or HTTP/2
    /// session establishment fails.
    pub async fn connect_h2c_tuned(
        host: &str,
        port: u16,
        max_message_size: usize,
        tuning: ChannelTuning,
    ) -> Result<Self, ChannelError> {
        Self::connect(host, port, None, max_message_size, tuning).await
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
    /// or HTTP/2 session establishment fails.
    ///
    /// Reachable only when the `__test-helpers` private feature is
    /// enabled; production callers use the `_with_max_message_size`
    /// variant exclusively.
    #[cfg(feature = "__test-helpers")]
    pub async fn connect_tls(
        host: &str,
        port: u16,
        tls: Arc<rustls::ClientConfig>,
    ) -> Result<Self, ChannelError> {
        Self::connect_tls_with_max_message_size(host, port, tls, DEFAULT_MAX_MESSAGE_SIZE).await
    }

    /// Same as [`Self::connect_tls`] with an explicit per-frame decode
    /// ceiling.
    ///
    /// Reachable only under the `__test-helpers` private feature —
    /// production callers go through [`Self::connect_tls_tuned`] so the
    /// configured HTTP/2 session tuning applies.
    ///
    /// # Errors
    ///
    /// Same as [`Self::connect_tls`].
    #[cfg(feature = "__test-helpers")]
    pub async fn connect_tls_with_max_message_size(
        host: &str,
        port: u16,
        tls: Arc<rustls::ClientConfig>,
        max_message_size: usize,
    ) -> Result<Self, ChannelError> {
        Self::connect(
            host,
            port,
            Some(tls),
            max_message_size,
            ChannelTuning::default(),
        )
        .await
    }

    /// Open a TLS-protected connection with an explicit per-frame
    /// decode ceiling and HTTP/2 session tuning (flow-control windows,
    /// keepalive cadence), both threaded from `DirectConfig::mdds`.
    ///
    /// The supplied `rustls::ClientConfig` is used verbatim for the
    /// initial connect and every in-place reconnect, so SPKI pinning,
    /// ALPN, and session-resumption configuration land identically
    /// across connection cycles.
    ///
    /// # Errors
    ///
    /// Returns a [`ChannelError`] when the TCP connect, TLS handshake,
    /// or HTTP/2 session establishment fails.
    pub async fn connect_tls_tuned(
        host: &str,
        port: u16,
        tls: Arc<rustls::ClientConfig>,
        max_message_size: usize,
        tuning: ChannelTuning,
    ) -> Result<Self, ChannelError> {
        Self::connect(host, port, Some(tls), max_message_size, tuning).await
    }

    /// Shared connect path: build the endpoint, attach the custom
    /// TCP(+TLS) connector, and open the connection eagerly so a dead
    /// target fails the constructor rather than the first RPC.
    async fn connect(
        host: &str,
        port: u16,
        tls: Option<Arc<rustls::ClientConfig>>,
        max_message_size: usize,
        tuning: ChannelTuning,
    ) -> Result<Self, ChannelError> {
        let scheme = if tls.is_some() {
            Scheme::HTTPS
        } else {
            Scheme::HTTP
        };
        let uri = format!("{scheme}://{host}:{port}");
        let endpoint = tonic::transport::Endpoint::from_shared(uri.clone())
            .map_err(|e| ChannelError::InvalidPath {
                path: uri.clone(),
                message: e.to_string(),
            })?
            .user_agent(format!("{USER_AGENT_PREFIX}/{}", env!("CARGO_PKG_VERSION")))
            .map_err(|e| ChannelError::InvalidPath {
                path: uri,
                message: format!("user-agent: {e}"),
            })?
            .tcp_nodelay(true)
            .initial_stream_window_size(tuning.initial_stream_window_size)
            .initial_connection_window_size(tuning.initial_connection_window_size)
            .http2_keep_alive_interval(tuning.keepalive_interval)
            .keep_alive_timeout(tuning.keepalive_timeout);
        let connector = GrpcConnector {
            host: Arc::from(host),
            port,
            tls,
        };
        let inner = endpoint
            .connect_with_connector(connector)
            .await
            .map_err(|e| classify_connect_error(host, port, &e))?;
        Ok(Self {
            inner,
            max_message_size,
            #[cfg(any(test, feature = "__test-helpers"))]
            scheme,
            in_flight: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Number of currently-open streams on this channel. The pool
    /// uses this as a load-balancing hint: a channel with no
    /// in-flight streams is freshly available, a saturated channel
    /// is steered around. Relaxed load — the value is a hint, not a
    /// hard barrier.
    #[doc(hidden)]
    #[must_use]
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Per-frame decode ceiling honoured by every RPC dispatched on
    /// this channel. Mirrors `DirectConfig::mdds.max_message_size`.
    ///
    /// Exposed under `__test-helpers` for integration tests that verify
    /// the configured ceiling propagates from `DirectConfig` to every
    /// channel construct.
    #[cfg(feature = "__test-helpers")]
    #[must_use]
    pub const fn max_message_size(&self) -> usize {
        self.max_message_size
    }

    /// `:scheme` pseudo-header this channel sends on every request —
    /// `"https"` over TLS, `"http"` over plaintext h2c. The gRPC
    /// HTTP/2 spec pins the scheme to the underlying transport so
    /// strict L7 proxies and routers accept the request.
    ///
    /// Hidden from the public docs — exposed for integration tests
    /// that need to confirm the channel records the right scheme for
    /// each transport.
    #[cfg(any(test, feature = "__test-helpers"))]
    #[doc(hidden)]
    #[must_use]
    pub fn scheme_str(&self) -> &'static str {
        if self.scheme == Scheme::HTTPS {
            "https"
        } else {
            "http"
        }
    }

    /// Take a pre-dispatch in-flight token. Used by
    /// [`super::ChannelPool::next`] to atomically reserve a slot on
    /// this channel at pick time, before the async dispatch future
    /// is even polled. Under burst contention this guarantees every
    /// concurrent `pool.next()` observer sees the prior reservations
    /// and routes around the loaded channel.
    ///
    /// The returned token's `Drop` decrements the counter.
    pub(crate) fn reserve_in_flight(&self) -> InFlightToken {
        InFlightToken::new(Arc::clone(&self.in_flight))
    }

    /// Try to reserve a slot atomically with a load-balancing
    /// guardrail: commit only if the channel's in-flight count at
    /// the time of reservation is `<= expected_max`. Returns the
    /// pre-bump count on failure so the caller can re-scan.
    ///
    /// This is the load-balancing primitive [`super::ChannelPool::next`]
    /// uses to close the pick/reserve race: under true concurrency
    /// two tasks may both scan and both pick the same least-loaded
    /// channel before either reservation lands. The CAS-style retry
    /// pattern lets the loser bail out and re-scan rather than pin to
    /// a now-saturated channel.
    pub(crate) fn try_reserve_in_flight(
        &self,
        expected_max: usize,
    ) -> Result<InFlightToken, usize> {
        let prior = self.in_flight.fetch_add(1, Ordering::AcqRel);
        if prior <= expected_max {
            // Reservation committed; the token's Drop releases it.
            // `from_committed` skips the second fetch_add.
            Ok(InFlightToken::from_committed(Arc::clone(&self.in_flight)))
        } else {
            // Race lost — channel got busier than the scan thought.
            // Roll back the speculative reservation; the momentary
            // over-count is acceptable for a load-balancing hint.
            self.in_flight.fetch_sub(1, Ordering::Release);
            Err(prior)
        }
    }

    /// Issue a server-streaming RPC.
    ///
    /// `method` is the fully-qualified gRPC path including the leading
    /// `/`, e.g. `"/BetaEndpoints.BetaThetaTerminal/GetStockListSymbols"`.
    /// The returned [`ServerStreaming`] decodes response frames as the
    /// server emits them.
    ///
    /// # Errors
    ///
    /// Returns a [`ChannelError`] when the request cannot be built or
    /// the RPC fails to open.
    pub async fn server_streaming<Req, Resp>(
        &self,
        method: &'static str,
        req: Req,
    ) -> Result<ServerStreaming<Resp>, ChannelError>
    where
        Req: prost::Message + Send + Sync + 'static,
        Resp: prost::Message + Default + Send + Sync + 'static,
    {
        self.server_streaming_inner(method, req, None).await
    }

    /// Same as [`Self::server_streaming`] with a per-call deadline.
    ///
    /// The deadline covers the entire RPC: opening the stream,
    /// sending the request, receiving every response frame, and the
    /// trailers. It is advertised to the server via the `grpc-timeout`
    /// request header and enforced locally; on elapse the underlying
    /// h2 stream is dropped (sending RST_STREAM to the server) and
    /// [`ChannelError::DeadlineExceeded`] surfaces — directly from
    /// this call if the open phase blew the deadline, or on the next
    /// poll of the returned stream otherwise.
    ///
    /// # Errors
    ///
    /// Same as [`Self::server_streaming`], plus
    /// [`ChannelError::DeadlineExceeded`].
    ///
    /// Reachable only under `__test-helpers` — production deadlines are
    /// handled at the `MddsClient` layer via `tokio::time::timeout`
    /// around the streaming consumer.
    #[cfg(feature = "__test-helpers")]
    pub async fn server_streaming_with_deadline<Req, Resp>(
        &self,
        method: &'static str,
        req: Req,
        deadline: Duration,
    ) -> Result<ServerStreaming<Resp>, ChannelError>
    where
        Req: prost::Message + Send + Sync + 'static,
        Resp: prost::Message + Default + Send + Sync + 'static,
    {
        self.server_streaming_inner(method, req, Some(deadline))
            .await
    }

    /// Shared dispatch path for the deadline and no-deadline variants.
    async fn server_streaming_inner<Req, Resp>(
        &self,
        method: &'static str,
        req: Req,
        deadline: Option<Duration>,
    ) -> Result<ServerStreaming<Resp>, ChannelError>
    where
        Req: prost::Message + Send + Sync + 'static,
        Resp: prost::Message + Default + Send + Sync + 'static,
    {
        let start = tokio::time::Instant::now();
        let path = PathAndQuery::try_from(method).map_err(|e| ChannelError::InvalidPath {
            path: method.to_string(),
            message: e.to_string(),
        })?;

        // Record the stream on the channel's in-flight counter BEFORE
        // the dispatch awaits, so the pool's load-balancing picker
        // sees every concurrent dispatch the moment it commits. The
        // token moves into the `ServerStreaming` on success (the
        // decrement fires when the response stream ends); error paths
        // drop it on return.
        let token = InFlightToken::new(Arc::clone(&self.in_flight));

        let mut grpc = tonic::client::Grpc::new(self.inner.clone())
            .max_decoding_message_size(self.max_message_size);
        let mut request = tonic::Request::new(req);
        let deadline_ms = deadline.map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        if let Some(d) = deadline {
            // Advertised via the `grpc-timeout` request header (so the
            // server can release resources on expiry) and enforced
            // client-side by the underlying stack's timeout layer for
            // the open phase. The streaming phase is enforced by the
            // wrapper below.
            request.set_timeout(d);
        }

        let codec = tonic_prost::ProstCodec::<Req, Resp>::default();
        let open = async {
            grpc.ready()
                .await
                .map_err(|e| classify_dispatch_error(&e, deadline_ms))?;
            grpc.server_streaming(request, path, codec)
                .await
                .map_err(|status| classify_status(status, deadline_ms))
        };
        // The underlying gRPC implementation panics while parsing a
        // status whose `grpc-status-details-bin` value is not valid
        // base64, and a trailers-only response parses that header
        // inside this open await. A malformed trailer from the wire
        // must not unwind into the caller's task, so the open future
        // is polled inside `catch_unwind` and a caught panic surfaces
        // as the terminal undecodable-trailer status (see
        // [`classify_poll_panic`]). `AssertUnwindSafe` is sound here:
        // the future is dropped on the panic path and never polled
        // again.
        let mut open = std::pin::pin!(open);
        let open = std::future::poll_fn(move |cx| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| open.as_mut().poll(cx)))
                .unwrap_or_else(|payload| Poll::Ready(Err(classify_poll_panic(payload))))
        });
        let response = match deadline {
            Some(d) => match tokio::time::timeout(d, open).await {
                Ok(r) => r,
                Err(_) => return Err(deadline_error(d)),
            },
            None => open.await,
        }?;

        let streaming = response.into_inner();
        let stream = ServerStreaming::new(streaming, self.max_message_size, token);
        Ok(match deadline {
            Some(d) => {
                let remaining = d.saturating_sub(start.elapsed());
                if remaining.is_zero() {
                    return Err(deadline_error(d));
                }
                stream.with_deadline(remaining, deadline_ms.unwrap_or(u64::MAX))
            }
            None => stream,
        })
    }
}

/// Default upper bound on a single decoded frame, in bytes. Matches
/// the reference stack's decoder default so the test constructors do
/// not silently accept frames a production decoder would reject.
#[cfg(feature = "__test-helpers")]
const DEFAULT_MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

/// Build a [`ChannelError::DeadlineExceeded`] from a `Duration` so the
/// open-phase error sites stay in lockstep.
fn deadline_error(d: Duration) -> ChannelError {
    ChannelError::DeadlineExceeded {
        duration_ms: u64::try_from(d.as_millis()).unwrap_or(u64::MAX),
    }
}

// ─── Error classification ───────────────────────────────────────────

/// Classify a `tonic::Status` observed at RPC open or mid-stream into
/// the crate's [`ChannelError`] taxonomy.
///
/// A status with no error source is a genuine server-sent status
/// (parsed from `grpc-status` trailers or a trailers-only response
/// head) — it surfaces as [`ChannelError::Rpc`]. This includes the
/// statuses the decode layer synthesizes locally for protocol-shape
/// violations (over-limit frames map to the canonical `OutOfRange`,
/// malformed framing to `Internal`), which carry no source either —
/// both are terminal for the retry shell, matching the previous
/// transport's codec-error classification.
///
/// A status WITH a source chain is a locally-synthesized wrapper
/// around a transport fault; the chain is walked for the precise
/// cause:
///
/// - `tonic::TimeoutExpired` — the `grpc-timeout` enforcement fired;
///   surfaces as [`ChannelError::DeadlineExceeded`].
/// - `tonic::ConnectError` — the lazy reconnect path failed to dial;
///   connection-level, surfaces as [`ChannelError::ConnectionClosed`].
/// - [`h2::Error`] — scoped by the same rules the previous transport
///   used: `GOAWAY` / IO failure / "inactive stream" are
///   connection-level ([`ChannelError::ConnectionClosed`]); per-stream
///   `RST_STREAM` (any reason code) and library-detected per-stream
///   protocol errors are stream-level ([`ChannelError::H2Stream`]).
///   HTTP/2 spec § 7 (Error Codes) is the canonical scope list.
/// - [`std::io::Error`] — transport gone; connection-level.
/// - An exhausted chain falls back to connection-level: an unknown
///   local transport fault is retried on a fresh pick, mirroring the
///   previous transport's open-phase classification.
pub(crate) fn classify_status(status: tonic::Status, deadline_ms: Option<u64>) -> ChannelError {
    use std::error::Error as _;
    if status.source().is_none() {
        return ChannelError::Rpc {
            status: Status::from_tonic(&status),
        };
    }
    let mut source = status.source();
    while let Some(err) = source {
        if err.downcast_ref::<tonic::TimeoutExpired>().is_some() {
            return ChannelError::DeadlineExceeded {
                duration_ms: deadline_ms.unwrap_or(0),
            };
        }
        if err.downcast_ref::<tonic::ConnectError>().is_some() {
            return ChannelError::ConnectionClosed(err.to_string());
        }
        if let Some(h2) = err.downcast_ref::<h2::Error>() {
            return classify_h2_error(h2);
        }
        if err.downcast_ref::<std::io::Error>().is_some() {
            return ChannelError::ConnectionClosed(err.to_string());
        }
        source = err.source();
    }
    ChannelError::ConnectionClosed(status.to_string())
}

/// Classify a panic caught at a transport poll boundary into the
/// terminal [`ChannelError`] for an undecodable status trailer.
///
/// The underlying gRPC implementation `.expect()`s the base64 decode
/// of `grpc-status-details-bin`, so a peer that sends a malformed
/// value panics whichever task polls the response. The two poll
/// boundaries that can observe such a trailer (the open-phase await
/// in [`Channel::server_streaming`] for trailers-only responses, and
/// [`ServerStreaming`]'s `poll_next` for end-of-stream trailers)
/// contain the unwind with `std::panic::catch_unwind` and route the
/// payload here.
///
/// The synthesized status follows the protocol-shape-violation
/// convention documented on [`classify_status`]: canonical `Internal`,
/// no error source, terminal for the retry shell. The panic payload
/// text rides along in the message so an unexpected panic from the
/// same boundary stays diagnosable.
pub(crate) fn classify_poll_panic(payload: Box<dyn std::any::Any + Send>) -> ChannelError {
    let detail = payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload");
    ChannelError::Rpc {
        status: Status::new(
            crate::error::GrpcStatusKind::Internal as u32,
            format!("server sent an undecodable status trailer: {detail}"),
        ),
    }
}

/// Classify a channel-level dispatch error (`ready()` failing before
/// the RPC was even sent). The error type is the transport's opaque
/// error; the source chain carries the precise cause.
fn classify_dispatch_error(
    err: &tonic::transport::Error,
    deadline_ms: Option<u64>,
) -> ChannelError {
    let _ = deadline_ms;
    classify_transport_error_chain(err)
}

/// Classify an [`h2::Error`] into the matching [`ChannelError`].
///
/// Connection-level failures surface as
/// [`ChannelError::ConnectionClosed`]:
/// - `GOAWAY` (either direction) — the connection refuses new streams.
/// - IO errors at the h2 layer — the transport is gone.
/// - The "inactive stream" user error — an operation targeted a stream
///   whose underlying connection already died.
///
/// Per-stream `RST_STREAM` (`CANCEL`, `REFUSED_STREAM`,
/// `INTERNAL_ERROR`, any reason code) is *stream-level*: only the
/// offending stream is dead, the h2 connection itself is healthy and
/// the next RPC on the same channel can succeed. Misclassifying these
/// as connection-level would recycle a still-good connection. They
/// surface as [`ChannelError::H2Stream`].
fn classify_h2_error(e: &h2::Error) -> ChannelError {
    if e.is_go_away() || e.is_io() {
        return ChannelError::ConnectionClosed(e.to_string());
    }
    let msg = e.to_string();
    if msg.contains("inactive stream") {
        return ChannelError::ConnectionClosed(msg);
    }
    ChannelError::H2Stream(msg)
}

/// Walk a transport error's source chain and classify the connect-time
/// fault precisely. The custom connector's [`ConnectorError`] carries
/// the TCP / TLS / server-name distinction; anything after a
/// successful connector dial is the HTTP/2 session establishment.
fn classify_connect_error(host: &str, port: u16, err: &tonic::transport::Error) -> ChannelError {
    use std::error::Error as _;
    let mut source: Option<&(dyn std::error::Error + 'static)> = err.source();
    while let Some(inner) = source {
        if let Some(conn) = inner.downcast_ref::<ConnectorError>() {
            return conn.to_channel_error(host, port);
        }
        if let Some(h2) = inner.downcast_ref::<h2::Error>() {
            return classify_h2_error(h2);
        }
        if let Some(io) = inner.downcast_ref::<std::io::Error>() {
            return ChannelError::Tcp {
                host: host.to_string(),
                port,
                source: std::io::Error::new(io.kind(), io.to_string()),
            };
        }
        source = inner.source();
    }
    ChannelError::H2Handshake(err.to_string())
}

/// Mid-dispatch variant of [`classify_connect_error`] without the
/// connect-target context: a `ready()` failure means the channel's
/// in-place reconnect could not produce a usable connection, which is
/// connection-level for the retry shell regardless of the precise
/// dial-phase cause.
fn classify_transport_error_chain(err: &tonic::transport::Error) -> ChannelError {
    use std::error::Error as _;
    let mut source: Option<&(dyn std::error::Error + 'static)> = err.source();
    while let Some(inner) = source {
        if let Some(h2) = inner.downcast_ref::<h2::Error>() {
            return classify_h2_error(h2);
        }
        source = inner.source();
    }
    ChannelError::ConnectionClosed(err.to_string())
}

// ─── In-flight accounting ───────────────────────────────────────────

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

// ─── Connector ──────────────────────────────────────────────────────

/// TCP(+TLS) dial error produced by [`GrpcConnector`]. Wrapped into
/// the transport stack's opaque connect error; [`classify_connect_error`]
/// recovers it by downcasting the source chain so connect failures
/// keep their precise [`ChannelError`] taxonomy.
#[derive(Debug, Error)]
enum ConnectorError {
    /// TCP connect failed.
    #[error("tcp connect: {source}")]
    Tcp {
        #[source]
        source: std::io::Error,
    },
    /// TLS handshake failed.
    #[error("tls handshake: {source}")]
    Tls {
        #[source]
        source: std::io::Error,
    },
    /// The host string was not a valid DNS name for rustls.
    #[error("invalid server name {host:?}")]
    InvalidServerName {
        /// Host the connector was built with.
        host: String,
    },
}

impl ConnectorError {
    /// Lift into the matching [`ChannelError`] with connect-target
    /// context. The underlying `io::Error` cannot be moved out of the
    /// borrowed chain, so it is reconstructed from kind + message.
    fn to_channel_error(&self, host: &str, port: u16) -> ChannelError {
        match self {
            Self::Tcp { source } => ChannelError::Tcp {
                host: host.to_string(),
                port,
                source: std::io::Error::new(source.kind(), source.to_string()),
            },
            Self::Tls { source } => ChannelError::Tls {
                host: host.to_string(),
                source: std::io::Error::new(source.kind(), source.to_string()),
            },
            Self::InvalidServerName { host } => {
                ChannelError::InvalidServerName { host: host.clone() }
            }
        }
    }
}

/// Transport-stack connector: dials TCP and, when a rustls config is
/// present, runs the TLS handshake with the crate's single-provider
/// configuration (`ring` provider, webpki roots, `h2` ALPN — built by
/// `crate::mdds::client`). Invoked once at eager connect and again by
/// the underlying stack's lazy reconnect whenever the connection dies,
/// so every reconnect lands a connection wire-equivalent to the
/// original.
#[derive(Clone)]
struct GrpcConnector {
    host: Arc<str>,
    port: u16,
    /// `Some(tls_config)` for HTTPS, `None` for h2c.
    tls: Option<Arc<rustls::ClientConfig>>,
}

impl tower_service::Service<Uri> for GrpcConnector {
    type Response = TokioIo<MaybeTlsStream>;
    type Error = ConnectorError;
    type Future =
        Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _dst: Uri) -> Self::Future {
        // The endpoint URI always matches the captured target (one
        // endpoint per channel); the captured host/port are
        // authoritative so the TLS server name never drifts from the
        // certificate verification target.
        let host = Arc::clone(&self.host);
        let port = self.port;
        let tls = self.tls.clone();
        Box::pin(async move {
            let stream = TcpStream::connect((&*host, port))
                .await
                .map_err(|source| ConnectorError::Tcp { source })?;
            let _ = stream.set_nodelay(true);
            match tls {
                None => Ok(TokioIo::new(MaybeTlsStream::Plain(stream))),
                Some(config) => {
                    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
                        .map_err(|_| ConnectorError::InvalidServerName {
                            host: host.to_string(),
                        })?;
                    let connector = TlsConnector::from(config);
                    let tls_stream = connector
                        .connect(server_name, stream)
                        .await
                        .map_err(|source| ConnectorError::Tls { source })?;
                    Ok(TokioIo::new(MaybeTlsStream::Tls(Box::new(tls_stream))))
                }
            }
        })
    }
}

/// Transport IO: plaintext TCP or client-side TLS over TCP. One
/// concrete type so the connector's `Service::Response` is nameable;
/// both arms forward the async IO traits verbatim.
enum MaybeTlsStream {
    /// Plaintext h2c.
    Plain(TcpStream),
    /// TLS-protected stream (boxed — the TLS state machine is large
    /// relative to the plain arm).
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for MaybeTlsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MaybeTlsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_flush(cx),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Self::Plain(s) => s.is_write_vectored(),
            Self::Tls(s) => s.is_write_vectored(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locally-synthesized statuses carrying a transport-fault source
    /// chain must classify by the chain, not surface as `Rpc`.
    #[test]
    fn sourced_status_classifies_as_transport_fault() {
        let goaway: h2::Error = h2::Reason::NO_ERROR.into();
        // A bare Reason (no GOAWAY / IO scope) is stream-level.
        let status = tonic::Status::from_error(Box::new(goaway));
        match classify_status(status, None) {
            ChannelError::H2Stream(_) => {}
            other => panic!("bare h2 Reason must classify stream-level, got {other:?}"),
        }
    }

    /// A status with no source is a genuine server status and must
    /// surface as `Rpc` with the crate's own `Status` payload.
    #[test]
    fn sourceless_status_classifies_as_rpc() {
        let status = tonic::Status::new(tonic::Code::PermissionDenied, "tier insufficient");
        match classify_status(status, None) {
            ChannelError::Rpc { status } => {
                assert_eq!(status.code(), 7);
                assert_eq!(status.message(), "tier insufficient");
            }
            other => panic!("expected Rpc, got {other:?}"),
        }
    }

    /// `TimeoutExpired` anywhere in the chain is the `grpc-timeout`
    /// enforcement firing — must classify as `DeadlineExceeded` with
    /// the caller's deadline value.
    #[test]
    fn timeout_expired_classifies_as_deadline() {
        let status = tonic::Status::from_error(Box::new(tonic::TimeoutExpired(())));
        match classify_status(status, Some(250)) {
            ChannelError::DeadlineExceeded { duration_ms } => assert_eq!(duration_ms, 250),
            other => panic!("expected DeadlineExceeded, got {other:?}"),
        }
    }

    /// An `io::Error` in the chain is connection-level.
    #[test]
    fn io_error_classifies_as_connection_closed() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "peer reset");
        let status = tonic::Status::from_error(Box::new(io));
        match classify_status(status, None) {
            ChannelError::ConnectionClosed(_) => {}
            other => panic!("expected ConnectionClosed, got {other:?}"),
        }
    }
}
