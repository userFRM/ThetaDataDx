//! gRPC channel: one HTTP/2 connection at a time, redialled on demand.
//!
//! A [`Channel`] holds the request sender of one HTTP/2 client
//! connection to a gRPC server; the connection itself is driven by its
//! own task. When the connection ends (GOAWAY, IO failure, peer close),
//! that task removes the sender and the next dispatched RPC dials a new
//! connection. An RPC in flight on the dead connection observes
//! [`ChannelError::ConnectionClosed`] and the caller's retry shell
//! (`crate::mdds::macros::classify_error`) re-dispatches it onto the new
//! connection or a sibling pool member. The production constructors
//! carry a dial timeout (`connect_timeout`) so a redial to an
//! unreachable or black-holed peer fails fast as a retryable transport
//! fault rather than hanging once per-call deadlines are disabled.
//!
//! The connection task removes the sender, rather than the next RPC
//! replacing it, because of how the HTTP/2 client fails queued
//! requests. Each request is queued for the connection task, which
//! fails whatever is still queued when it exits. A request queued at
//! the instant the task exits can land after that sweep, and is then
//! failed only when the last sender for the connection is dropped. A
//! sender kept until the next RPC notices the dead connection leaves
//! such a request unanswered for as long as the channel stays idle:
//! without a deadline, for ever.
//!
//! [`Channel::server_streaming`] sends a single server-streaming RPC and
//! returns a [`ServerStreaming`] that yields decoded response messages.
//! Per-chunk payload decode (zstd + prost `DataTable`) runs inline on
//! the request task, keeping each chunk on its producing connection and
//! avoiding a cross-thread hand-off for this workload.
//!
//! # Connector
//!
//! TLS rides through a custom connector (`GrpcConnector`) so the
//! existing single-provider rustls configuration (`ring`, webpki roots,
//! `h2` ALPN) is reused verbatim and the dependency graph keeps exactly
//! one `CryptoProvider`. The same connector serves plaintext h2c for
//! mock servers and sidecar deployments.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use http::header::{HeaderValue, USER_AGENT};
use http::uri::{PathAndQuery, Scheme, Uri};
use hyper::client::conn::http2;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::status::Status;
use super::stream::ServerStreaming;

/// User-agent reported on every request.
const USER_AGENT_VALUE: &str = concat!("thetadatadx-grpc/", env!("CARGO_PKG_VERSION"));

/// Error type of the request path beneath [`Channel`]: dial, handshake
/// and HTTP/2 faults, classified by walking their source chains.
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// HTTP/2 session tuning threaded from `DirectConfig::mdds` —
/// flow-control windows and keepalive cadence. The short channel
/// constructors use [`ChannelTuning::default`] (the 64 KiB HTTP/2 spec
/// windows, 30 s / 10 s keepalive — a deliberately untuned connection,
/// below the larger production-config defaults); `MarketDataClient::connect`
/// threads the operator's configured values through
/// [`Channel::connect_tls_tuned`] / [`Channel::connect_h2c_tuned`].
#[derive(Debug, Clone, Copy)]
pub struct ChannelTuning {
    /// Initial per-stream flow-control window, in bytes. Mirrors
    /// `MarketDataConfig::stream_window_size_kb`.
    pub initial_stream_window_size: u32,
    /// Initial connection-level flow-control window, in bytes.
    /// Mirrors `MarketDataConfig::connection_window_size_kb`.
    pub initial_connection_window_size: u32,
    /// Interval between HTTP/2 keepalive PING frames. Mirrors
    /// `MarketDataConfig::keepalive_secs`.
    pub keepalive_interval: Duration,
    /// How long to wait for a keepalive PING acknowledgement before
    /// declaring the connection dead. Mirrors
    /// `MarketDataConfig::keepalive_timeout_secs`.
    pub keepalive_timeout: Duration,
}

impl Default for ChannelTuning {
    fn default() -> Self {
        Self {
            // HTTP/2 spec initial windows (64 KiB) — a deliberately
            // untuned connection for the short constructors, below the
            // larger production-config defaults.
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
    /// opened. Covers a terminal `RST_STREAM` from the peer (a reason
    /// code that may have had server-side effect: `CANCEL`,
    /// `INTERNAL_ERROR`, etc.). The h2 connection itself is healthy and
    /// the next RPC on the same channel can succeed, but this stream's
    /// outcome is undefined, so the retry shell treats it as terminal.
    /// Connection-level death surfaces through [`Self::ConnectionClosed`]
    /// instead, and a not-processed `REFUSED_STREAM` through
    /// [`Self::H2StreamRefused`].
    #[error("h2 stream: {0}")]
    H2Stream(String),
    /// h2 `RST_STREAM` with reason `REFUSED_STREAM`: the server did not
    /// process this stream at all (RFC 7540 § 8.1.4). The h2 connection
    /// is healthy and, because no work was started, the RPC is safe to
    /// re-dispatch — the retry shell classifies this as transient and
    /// retries on the next pool pick. Kept distinct from the terminal
    /// [`Self::H2Stream`] so a not-processed reset never surfaces to the
    /// caller as a hard failure.
    #[error("h2 stream refused: {0}")]
    H2StreamRefused(String),
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
    /// The channel dials a fresh connection on the next dispatched
    /// RPC; the caller's retry shell re-dispatches and observes it.
    #[error("h2 connection closed: {0}")]
    ConnectionClosed(String),
}

/// One gRPC channel to a server.
///
/// Wraps a [`Transport`] (one HTTP/2 connection at a time, redialled
/// on demand) plus the per-channel state the pool and dispatch paths
/// need: the per-frame decode ceiling, the `:scheme` the channel
/// speaks, and the in-flight stream counter the [`super::ChannelPool`]
/// uses for least-loaded picks.
pub struct Channel {
    /// Request path. Cloning is cheap (a handle onto the shared
    /// connection state); one clone is taken per dispatched RPC.
    transport: Transport,
    /// `scheme://host:port` of the server. Every request URI is this
    /// origin plus the method path, which pins the `:scheme` and
    /// `:authority` pseudo-headers to the transport and target.
    origin: Uri,
    /// Per-frame decode ceiling propagated to every RPC dispatched on
    /// this channel. Mirrors `DirectConfig::mdds.max_message_size`;
    /// response frames above it are rejected by the decode layer
    /// before allocation.
    max_message_size: usize,
    /// `:scheme` this channel speaks — `https` over TLS, `http` over
    /// plaintext h2c. Derived from the connect constructor; the request
    /// pseudo-header follows [`Self::origin`], so the field exists only
    /// for the test-surface accessor ([`Self::scheme_str`]).
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
        Self::connect(
            host,
            port,
            None,
            max_message_size,
            ChannelTuning::default(),
            None,
        )
        .await
    }

    /// Open a plaintext (h2c) connection with an explicit per-frame
    /// decode ceiling and HTTP/2 session tuning (flow-control windows,
    /// keepalive cadence), both threaded from `DirectConfig::mdds`.
    ///
    /// `connect_timeout` bounds every dial (the eager open and each
    /// redial after the connection ends), so a black-holed target fails
    /// fast as a retryable transport fault instead of hanging the RPC
    /// that dials it.
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
        connect_timeout: Duration,
    ) -> Result<Self, ChannelError> {
        Self::connect(
            host,
            port,
            None,
            max_message_size,
            tuning,
            Some(connect_timeout),
        )
        .await
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
            None,
        )
        .await
    }

    /// Open a TLS-protected connection with an explicit per-frame
    /// decode ceiling and HTTP/2 session tuning (flow-control windows,
    /// keepalive cadence), both threaded from `DirectConfig::mdds`.
    ///
    /// The supplied `rustls::ClientConfig` is used verbatim for the
    /// initial connect and every redial, so SPKI pinning,
    /// ALPN, and session-resumption configuration land identically
    /// across connection cycles.
    ///
    /// `connect_timeout` bounds every dial (the eager open and each
    /// redial after the connection ends), so a black-holed target fails
    /// fast as a retryable transport fault instead of hanging the RPC
    /// that dials it.
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
        connect_timeout: Duration,
    ) -> Result<Self, ChannelError> {
        Self::connect(
            host,
            port,
            Some(tls),
            max_message_size,
            tuning,
            Some(connect_timeout),
        )
        .await
    }

    /// Shared connect path: build the transport around the custom
    /// TCP(+TLS) connector and open the connection eagerly so a dead
    /// target fails the constructor rather than the first RPC.
    ///
    /// When `connect_timeout` is `Some`, the transport applies it to
    /// every dial: the eager open below and each redial after a dropped
    /// connection. Without it a redial to a black-holed peer would hang
    /// the RPC that triggered it once per-call deadlines are disabled,
    /// and the retry shell would never observe a retryable transport
    /// fault. The caller may additionally wrap the eager open in its own
    /// timeout; the two are complementary, and this one is the only
    /// bound that covers the redials.
    async fn connect(
        host: &str,
        port: u16,
        tls: Option<Arc<rustls::ClientConfig>>,
        max_message_size: usize,
        tuning: ChannelTuning,
        connect_timeout: Option<Duration>,
    ) -> Result<Self, ChannelError> {
        let scheme = if tls.is_some() {
            Scheme::HTTPS
        } else {
            Scheme::HTTP
        };
        let origin = format!("{scheme}://{host}:{port}");
        let origin = Uri::try_from(origin.as_str()).map_err(|e| ChannelError::InvalidPath {
            path: origin.clone(),
            message: e.to_string(),
        })?;
        let connector = GrpcConnector {
            host: Arc::from(host),
            port,
            tls,
        };
        let transport = Transport::new(connector, tuning, connect_timeout);
        transport
            .dial()
            .await
            .map_err(|e| classify_connect_error(host, port, &*e))?;
        Ok(Self {
            transport,
            origin,
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
    /// handled at the `MarketDataClient` layer via `tokio::time::timeout`
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

        // The transport is always ready (it dials inside the call), so
        // the client's readiness wait is skipped.
        let mut grpc =
            tonic::client::Grpc::with_origin(self.transport.clone(), self.origin.clone())
                .max_decoding_message_size(self.max_message_size);
        let mut request = tonic::Request::new(req);
        if let Some(d) = deadline {
            // Advertised via the `grpc-timeout` request header so the
            // server can release resources on expiry. Enforced locally
            // by the timeout around the open phase below and by the
            // stream wrapper for the streaming phase.
            request.set_timeout(d);
        }

        let codec = tonic_prost::ProstCodec::<Req, Resp>::default();
        let open = async {
            grpc.server_streaming(request, path, codec)
                .await
                .map_err(classify_status)
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
                stream.with_deadline(remaining, u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
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

// ─── Transport ──────────────────────────────────────────────────────

/// Request path beneath [`Channel`]: one HTTP/2 connection at a time,
/// dialled on demand and redialled after it ends.
///
/// The live connection's request sender is stored here, and the
/// connection's own task removes it as soon as the connection ends (the
/// module docs explain why that cannot wait for the next RPC). An RPC
/// holds a clone of the sender only while it enqueues its request, so
/// the stored sender is the last one, and dropping it fails any request
/// the dead connection left queued.
#[derive(Clone)]
struct Transport {
    shared: Arc<TransportShared>,
}

struct TransportShared {
    connector: GrpcConnector,
    /// HTTP/2 session settings: the flow-control windows and keepalive
    /// cadence from [`ChannelTuning`], with keepalive driven by the
    /// runtime's timer.
    http2: http2::Builder<TokioExecutor>,
    /// Bound on each dial (TCP connect plus TLS handshake).
    connect_timeout: Option<Duration>,
    /// Sender of the live connection, tagged with that connection's
    /// sequence number so the task of a connection that has already
    /// been replaced never removes its successor's sender.
    live: Mutex<Option<(u64, http2::SendRequest<tonic::body::Body>)>>,
    /// Serialises dials, so RPCs that find no live connection at the
    /// same time share one new connection instead of opening one each.
    dial_lock: tokio::sync::Mutex<()>,
    /// Sequence number of the next connection.
    next_connection: AtomicU64,
}

impl Transport {
    fn new(
        connector: GrpcConnector,
        tuning: ChannelTuning,
        connect_timeout: Option<Duration>,
    ) -> Self {
        let mut http2 = http2::Builder::new(TokioExecutor::new());
        http2
            .timer(TokioTimer::new())
            .initial_stream_window_size(tuning.initial_stream_window_size)
            .initial_connection_window_size(tuning.initial_connection_window_size)
            .keep_alive_interval(tuning.keepalive_interval)
            .keep_alive_timeout(tuning.keepalive_timeout);
        Self {
            shared: Arc::new(TransportShared {
                connector,
                http2,
                connect_timeout,
                live: Mutex::new(None),
                dial_lock: tokio::sync::Mutex::new(()),
                next_connection: AtomicU64::new(0),
            }),
        }
    }

    /// Sender of the live connection, or `None` when there is no
    /// connection or it has closed.
    fn live_sender(&self) -> Option<http2::SendRequest<tonic::body::Body>> {
        lock(&self.shared.live)
            .as_ref()
            .filter(|(_, sender)| !sender.is_closed())
            .map(|(_, sender)| sender.clone())
    }

    /// Dial a new connection, make it the live one and return its
    /// sender.
    ///
    /// A dial that exceeds the connect timeout fails with an
    /// `std::io::Error` of kind `TimedOut`, which the classifiers map to
    /// the retryable [`ChannelError::ConnectionClosed`].
    async fn dial(&self) -> Result<http2::SendRequest<tonic::body::Body>, BoxError> {
        let shared = &self.shared;
        let io = match shared.connect_timeout {
            Some(limit) => tokio::time::timeout(limit, shared.connector.connect())
                .await
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("dial timed out after {limit:?}"),
                    )
                })??,
            None => shared.connector.connect().await?,
        };
        let (sender, connection) = shared.http2.handshake(io).await?;
        let id = shared.next_connection.fetch_add(1, Ordering::Relaxed);
        *lock(&shared.live) = Some((id, sender.clone()));
        let shared = Arc::downgrade(&self.shared);
        tokio::spawn(async move {
            // The connection future owns the request queue, so the queue
            // is swept by the time this statement completes. Removing
            // the stored sender afterwards drops the last sender, which
            // fails any request that landed after the sweep.
            let ended = connection.await;
            if let Err(e) = ended {
                tracing::debug!(error = %e, "gRPC connection ended with an error");
            }
            if let Some(shared) = shared.upgrade() {
                let mut live = lock(&shared.live);
                if live.as_ref().is_some_and(|(live_id, _)| *live_id == id) {
                    *live = None;
                }
            }
        });
        Ok(sender)
    }
}

impl tower_service::Service<http::Request<tonic::body::Body>> for Transport {
    type Response = http::Response<hyper::body::Incoming>;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, BoxError>> + Send>>;

    /// Always ready: a missing or closed connection is redialled inside
    /// [`Self::call`], so the dial error reaches the RPC that caused it.
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: http::Request<tonic::body::Body>) -> Self::Future {
        request
            .headers_mut()
            .insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
        let transport = self.clone();
        Box::pin(async move {
            // The sender clone is scoped to the enqueue: an RPC that held
            // it while awaiting its response would keep the connection's
            // queue alive and could strand its own request.
            let response = {
                let mut sender = match transport.live_sender() {
                    Some(sender) => sender,
                    None => {
                        let _dialling = transport.shared.dial_lock.lock().await;
                        match transport.live_sender() {
                            Some(sender) => sender,
                            None => transport.dial().await?,
                        }
                    }
                };
                sender.send_request(request)
            };
            Ok(response.await?)
        })
    }
}

/// Lock a mutex whose data stays consistent across a panic (a plain
/// `Option` swap), so a poisoned lock is still usable.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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
/// a protocol-shape violation is deterministic, so both are terminal
/// for the retry shell rather than transient.
///
/// A status WITH a source chain is a locally-synthesized wrapper
/// around a transport fault; the chain is walked for the precise
/// cause:
///
/// - [`h2::Error`] — scoped by HTTP/2 error semantics: `GOAWAY` / IO
///   failure / "inactive stream" are connection-level
///   ([`ChannelError::ConnectionClosed`]); per-stream `RST_STREAM`
///   (any reason code) and library-detected per-stream protocol errors
///   are stream-level ([`ChannelError::H2Stream`]). HTTP/2 spec § 7
///   (Error Codes) is the canonical scope list.
/// - [`std::io::Error`] — transport gone; connection-level.
///   A failed or timed-out redial lands here too, since the
///   connector's errors carry their IO cause.
/// - An exhausted chain falls back to connection-level: an unknown
///   local transport fault (a request the dead connection never sent,
///   a dial refused for an invalid server name) is treated as transient
///   and retried on a fresh pick.
pub(crate) fn classify_status(status: tonic::Status) -> ChannelError {
    use std::error::Error as _;
    if status.source().is_none() {
        return ChannelError::Rpc {
            status: Status::from_tonic(&status),
        };
    }
    let mut source = status.source();
    while let Some(err) = source {
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

/// Classify an [`h2::Error`] into the matching [`ChannelError`].
///
/// Connection-level failures surface as
/// [`ChannelError::ConnectionClosed`]:
/// - `GOAWAY` (either direction) — the connection refuses new streams.
/// - IO errors at the h2 layer — the transport is gone.
/// - The "inactive stream" user error — an operation targeted a stream
///   whose underlying connection already died.
///
/// Per-stream `RST_STREAM` is *stream-level*: only the offending stream
/// is dead, the h2 connection itself is healthy and the next RPC on the
/// same channel can succeed. Misclassifying these as connection-level
/// would recycle a still-good connection. The reset reason decides
/// retry-safety:
///
/// - `REFUSED_STREAM` means the server did not process the stream at all
///   (RFC 7540 § 8.1.4), so the RPC is safe to re-dispatch — it surfaces
///   as the transient [`ChannelError::H2StreamRefused`].
/// - Every other reason (`CANCEL`, `INTERNAL_ERROR`, any other code) may
///   have had server-side effect; the stream's outcome is undefined, so
///   it surfaces as the terminal [`ChannelError::H2Stream`].
fn classify_h2_error(e: &h2::Error) -> ChannelError {
    if e.is_go_away() || e.is_io() {
        return ChannelError::ConnectionClosed(e.to_string());
    }
    let msg = e.to_string();
    if msg.contains("inactive stream") {
        return ChannelError::ConnectionClosed(msg);
    }
    // A not-processed reset is retry-safe; only this reason code carries
    // that guarantee, so every other reset stays terminal.
    if e.reason() == Some(h2::Reason::REFUSED_STREAM) {
        return ChannelError::H2StreamRefused(msg);
    }
    ChannelError::H2Stream(msg)
}

/// Walk a dial error's source chain and classify the connect-time fault
/// precisely. The custom connector's [`ConnectorError`] carries the
/// TCP / TLS / server-name distinction; anything after a successful
/// connector dial is the HTTP/2 session establishment.
///
/// A dial that exceeded the connect timeout surfaces here as a bare
/// `std::io::Error` with `ErrorKind::TimedOut` (see [`Transport::dial`]).
/// A timed-out dial means the target was unreachable or black-holed,
/// which is the same retryable transport fault as a dropped connection,
/// so it is classified [`ChannelError::ConnectionClosed`] (Transient for
/// the retry shell), not the terminal [`ChannelError::Tcp`] a concrete
/// connect refusal would carry. This keeps the eager open in lockstep
/// with a redial inside an RPC, which [`classify_status`] maps to
/// `ConnectionClosed` as well.
fn classify_connect_error(
    host: &str,
    port: u16,
    err: &(dyn std::error::Error + 'static),
) -> ChannelError {
    let mut source = Some(err);
    while let Some(inner) = source {
        if let Some(conn) = inner.downcast_ref::<ConnectorError>() {
            return conn.to_channel_error(host, port);
        }
        if let Some(h2) = inner.downcast_ref::<h2::Error>() {
            return classify_h2_error(h2);
        }
        if let Some(io) = inner.downcast_ref::<std::io::Error>() {
            if io.kind() == std::io::ErrorKind::TimedOut {
                return ChannelError::ConnectionClosed(format!(
                    "connect to {host}:{port} timed out: {io}"
                ));
            }
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

/// TCP(+TLS) dial error produced by `GrpcConnector`.
/// [`classify_connect_error`] recovers it by downcasting the dial
/// error's source chain so connect failures keep their precise
/// [`ChannelError`] taxonomy.
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

/// Dials TCP and, when a rustls config is present, runs the TLS
/// handshake with the crate's single-provider configuration (`ring`
/// provider, webpki roots, `h2` ALPN, as built by `crate::mdds::client`).
/// Invoked once at eager connect and again for every redial, so every
/// connection is wire-equivalent to the original.
struct GrpcConnector {
    host: Arc<str>,
    port: u16,
    /// `Some(tls_config)` for HTTPS, `None` for h2c.
    tls: Option<Arc<rustls::ClientConfig>>,
}

impl GrpcConnector {
    async fn connect(&self) -> Result<TokioIo<MaybeTlsStream>, ConnectorError> {
        let stream = TcpStream::connect((&*self.host, self.port))
            .await
            .map_err(|source| ConnectorError::Tcp { source })?;
        let _ = stream.set_nodelay(true);
        let Some(config) = &self.tls else {
            return Ok(TokioIo::new(MaybeTlsStream::Plain(stream)));
        };
        let server_name =
            rustls::pki_types::ServerName::try_from(self.host.to_string()).map_err(|_| {
                ConnectorError::InvalidServerName {
                    host: self.host.to_string(),
                }
            })?;
        let tls_stream = TlsConnector::from(Arc::clone(config))
            .connect(server_name, stream)
            .await
            .map_err(|source| ConnectorError::Tls { source })?;
        Ok(TokioIo::new(MaybeTlsStream::Tls(Box::new(tls_stream))))
    }
}

/// Transport IO: plaintext TCP or client-side TLS over TCP. One
/// concrete type so the connector's output is nameable; both arms
/// forward the async IO traits verbatim.
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
        match classify_status(status) {
            ChannelError::H2Stream(_) => {}
            other => panic!("bare h2 Reason must classify stream-level, got {other:?}"),
        }
    }

    /// A status with no source is a genuine server status and must
    /// surface as `Rpc` with the crate's own `Status` payload.
    #[test]
    fn sourceless_status_classifies_as_rpc() {
        let status = tonic::Status::new(tonic::Code::PermissionDenied, "tier insufficient");
        match classify_status(status) {
            ChannelError::Rpc { status } => {
                assert_eq!(status.code(), 7);
                assert_eq!(status.message(), "tier insufficient");
            }
            other => panic!("expected Rpc, got {other:?}"),
        }
    }

    /// An `io::Error` in the chain is connection-level.
    #[test]
    fn io_error_classifies_as_connection_closed() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "peer reset");
        let status = tonic::Status::from_error(Box::new(io));
        match classify_status(status) {
            ChannelError::ConnectionClosed(_) => {}
            other => panic!("expected ConnectionClosed, got {other:?}"),
        }
    }

    /// A `REFUSED_STREAM` reset means the server never processed the
    /// stream (RFC 7540 § 8.1.4), so it must classify as the retry-safe
    /// [`ChannelError::H2StreamRefused`] — the retry shell re-dispatches
    /// it instead of surfacing a terminal failure. Pinned because the
    /// reason-code branch is the whole of the not-processed-reset fix.
    #[test]
    fn refused_stream_reset_classifies_as_h2_stream_refused() {
        let refused: h2::Error = h2::Reason::REFUSED_STREAM.into();
        match classify_h2_error(&refused) {
            ChannelError::H2StreamRefused(_) => {}
            other => panic!("REFUSED_STREAM must classify as H2StreamRefused, got {other:?}"),
        }
    }

    /// Companion: a reset with a reason other than `REFUSED_STREAM`
    /// (here `CANCEL`) may have had server-side effect, so it stays the
    /// terminal [`ChannelError::H2Stream`]. Only the not-processed reason
    /// is promoted to retry-safe; every other per-stream reset keeps its
    /// undefined outcome.
    #[test]
    fn other_reset_reason_stays_terminal_h2_stream() {
        for reason in [
            h2::Reason::CANCEL,
            h2::Reason::INTERNAL_ERROR,
            h2::Reason::NO_ERROR,
        ] {
            let err: h2::Error = reason.into();
            match classify_h2_error(&err) {
                ChannelError::H2Stream(_) => {}
                other => panic!("reset {reason:?} must stay terminal H2Stream, got {other:?}"),
            }
        }
    }

    /// End-to-end through the status classifier: a tonic status whose
    /// source chain carries a `REFUSED_STREAM` h2 error must surface as
    /// the retry-safe `H2StreamRefused`, so the path the dispatch layer
    /// actually walks honours the not-processed reset.
    #[test]
    fn refused_stream_through_status_chain_is_retry_safe() {
        let refused: h2::Error = h2::Reason::REFUSED_STREAM.into();
        let status = tonic::Status::from_error(Box::new(refused));
        match classify_status(status) {
            ChannelError::H2StreamRefused(_) => {}
            other => panic!("expected H2StreamRefused from the status chain, got {other:?}"),
        }
    }

    /// A connection's request sender must be released the moment the
    /// connection ends, not when the next RPC finds it closed. A request
    /// enqueued at the instant the connection task exits can miss that
    /// task's final sweep of its queue, and is then failed only when the
    /// last sender is dropped; a sender kept until the next RPC leaves
    /// that request waiting for as long as the channel stays idle. The
    /// race itself spans a few instructions inside the runtime's channel
    /// and cannot be forced from here, so this pins the property that
    /// resolves it: with no RPC issued, the server's close alone empties
    /// the live slot.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connection_end_releases_its_request_sender() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept");
            h2::server::handshake(socket)
                .await
                .expect("server handshake")
        });
        let channel = Channel::connect(
            "127.0.0.1",
            port,
            None,
            4 * 1024 * 1024,
            ChannelTuning::default(),
            None,
        )
        .await
        .expect("h2c connect");
        assert!(
            lock(&channel.transport.shared.live).is_some(),
            "a freshly opened connection is live"
        );

        // Close the server side of the connection.
        drop(server.await.expect("server task"));

        let released = async {
            while lock(&channel.transport.shared.live).is_some() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(5), released)
            .await
            .expect("the ended connection's request sender must be released without another RPC");
    }
}
