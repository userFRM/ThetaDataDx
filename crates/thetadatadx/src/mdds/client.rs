//! `HistoricalClient` struct, connection lifecycle, and session/transport state.
//!
//! This module owns the MDDS client type: its fields (session UUID,
//! channel, config, request semaphore, subscription tiers), its
//! [`connect`](HistoricalClient::connect) constructor, and the small read-only
//! getters (`session_uuid`, `config`, `stock_tier`, `options_tier`, `channel`)
//! that expose client state to callers.
//!
//! Per-request helpers (`collect_stream`, `for_each_chunk`) live in
//! [`super::stream`]; the cross-cutting wire helpers
//! (`normalize_expiration`, `wire_strike_opt`, `wire_right_opt`) in
//! [`super::wire_semantics`]; date validation in [`super::validate`];
//! generated endpoint method bodies in [`super::endpoints`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::{self, Credentials, SessionToken};
use crate::config::DirectConfig;
use crate::error::Error;
use crate::grpc::{Channel, ChannelPool, ChannelTuning};
use crate::mdds::tier::SubscriptionTier;
use crate::proto;

/// Version string sent in `QueryInfo.terminal_version`.
const TERMINAL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// MDDS client for `ThetaData` server access.
///
/// Connects to MDDS (gRPC, historical data) without requiring the JVM
/// terminal. Authenticates via the Nexus HTTP API, then issues gRPC
/// requests to the upstream MDDS server.
///
/// # Example
///
/// ```rust,no_run
/// use thetadatadx::{Client, Credentials, DirectConfig};
///
/// # async fn run() -> Result<(), thetadatadx::Error> {
/// let creds = Credentials::from_file("creds.txt")?;
/// let client = Client::connect(&creds, DirectConfig::production()).await?;
///
/// let eod = client.historical().stock_history_eod("AAPL", "20240101", "20240301").await?;
/// println!("{} EOD ticks", eod.len());
/// # Ok(())
/// # }
/// ```
pub struct HistoricalClient {
    /// Shared, mutable session token. Every request reads the current
    /// UUID via this handle; `Unauthenticated` responses trigger a
    /// single-shot refresh that swaps the UUID in place. See
    /// `crate::auth::SessionToken`.
    session: SessionToken,
    /// Pool of gRPC channels to the MDDS server. Least-loaded
    /// dispatch lets workloads exceed the per-connection
    /// `MAX_CONCURRENT_STREAMS` ceiling and gives each in-flight
    /// request its own connection-level flow-control window.
    channels: ChannelPool,
    /// Configuration snapshot (retained for diagnostics/reconnect).
    config: DirectConfig,
    /// Reused `query_parameters` map. `client = "terminal"` is the only
    /// static entry; we clone this into every `QueryInfo` so per-call
    /// allocation stays flat instead of rebuilding the HashMap each time.
    query_parameters: HashMap<String, String>,
    /// `QueryInfo.client_type` value resolved at connect time (config
    /// builder > env var > `rust-thetadatadx` default). Kept as an owned
    /// `String` so it is cloned once per call — cheaper than rebuilding
    /// from config every request.
    client_type: String,
    /// Semaphore limiting concurrent in-flight gRPC requests.
    ///
    /// The JVM terminal limits concurrent requests to `2^subscription_tier`
    /// (Free=1, Value=2, Standard=4, Pro=8). This semaphore enforces the same
    /// bound to prevent server-side rate limiting / 429 disconnects.
    pub(crate) request_semaphore: Arc<tokio::sync::Semaphore>,
    /// Per-asset subscription tiers captured from the Nexus auth response.
    /// `None` for asset classes the auth response omits or for unknown
    /// wire values (the wire byte is preserved in the structured logs at
    /// connect time but never silently coerced into a tier).
    stock_tier: Option<SubscriptionTier>,
    options_tier: Option<SubscriptionTier>,
    indices_tier: Option<SubscriptionTier>,
    interest_rate_tier: Option<SubscriptionTier>,
}

// ── Infrastructure (not generated — these are session/transport methods, not ThetaData endpoints) ──

impl HistoricalClient {
    /// Connect to `ThetaData` servers directly (no JVM terminal needed).
    ///
    /// 1. Authenticates against the Nexus HTTP API to obtain a session UUID.
    /// 2. Opens a gRPC channel (TLS) to the MDDS server.
    ///
    /// The FPSS (real-time streaming) connection is not established here;
    /// this constructor covers only the MDDS historical-data channel.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error> {
        // Step 1: Authenticate against Nexus API using the configured URL
        // (env-var / builder overridable). `config.auth.nexus_url` already
        // reflects that precedence via `DirectConfig::production()`.
        //
        // The Nexus URL itself encodes deployment topology that operators
        // rarely need at `info` — keep the URL behind `trace` verbosity
        // so production deployments do not record it by default. Mirrors
        // the same downgrade applied to `auth/nexus.rs`.
        tracing::info!("authenticating with Nexus API");
        tracing::trace!(nexus_url = %config.auth.nexus_url, "Nexus auth URL");
        // Auth is driven by the historical (MDDS) environment only; the
        // streaming environment never affects the auth marker.
        let auth_resp =
            auth::authenticate_at(&config.auth.nexus_url, creds, config.historical_environment)
                .await?;
        let session_uuid = auth_resp.session_id.clone();

        tracing::debug!(
            stock_tier = ?auth_resp.user.as_ref().and_then(|u| u.stock_subscription),
            "session established (session_id redacted)"
        );

        // Step 2: Open the gRPC channel pool to MDDS.
        let host = config.historical.host.clone();
        let port = config.historical.port;
        tracing::debug!(host = %host, port, tls = config.historical.tls, "connecting to MDDS");

        let pool_size = effective_pool_size(&auth_resp);
        let channels =
            open_channel_pool(&host, port, config.historical.tls, pool_size, &config).await?;
        tracing::info!(
            pool_size,
            "MDDS channel pool connected ({} h2 connections)",
            pool_size
        );

        let mut query_parameters = HashMap::new();
        // QueryInfo always includes `"client": "terminal"`.
        query_parameters.insert("client".to_string(), "terminal".to_string());

        // The request semaphore must match the resolved channel pool
        // size so the (N+1)-th in-flight RPC can never claim a permit
        // before there's a channel free to carry it. `pool_size`
        // already reflects the tier-derived resolution from
        // `effective_pool_size`; reusing it keeps the semaphore and
        // the channel count strictly coupled.
        let request_semaphore = Arc::new(tokio::sync::Semaphore::new(pool_size));

        tracing::debug!(pool_size, "request semaphore initialized");

        let stock_tier = auth_resp
            .user
            .as_ref()
            .and_then(|u| u.stock_subscription)
            .and_then(SubscriptionTier::from_wire);
        let options_tier = auth_resp
            .user
            .as_ref()
            .and_then(|u| u.options_subscription)
            .and_then(SubscriptionTier::from_wire);
        let indices_tier = auth_resp
            .user
            .as_ref()
            .and_then(|u| u.indices_subscription)
            .and_then(SubscriptionTier::from_wire);
        let interest_rate_tier = auth_resp
            .user
            .as_ref()
            .and_then(|u| u.interest_rate_subscription)
            .and_then(SubscriptionTier::from_wire);

        let session = SessionToken::new(
            session_uuid,
            config.auth.nexus_url.clone(),
            config.historical_environment,
            creds.clone(),
        );
        let client_type = config.auth.client_type.clone();

        Ok(Self {
            session,
            channels,
            config,
            query_parameters,
            client_type,
            request_semaphore,
            stock_tier,
            options_tier,
            indices_tier,
            interest_rate_tier,
        })
    }

    /// Construct a `QueryInfo` around a caller-supplied UUID. Used by
    /// the retry wrapper to pin an in-flight attempt to the exact UUID
    /// seen by `crate::auth::SessionToken::snapshot` — so when a
    /// concurrent refresh advances the token, we don't accidentally
    /// mix old and new UUIDs on the same request.
    pub(crate) fn build_query_info(&self, uuid: String) -> proto::QueryInfo {
        proto::QueryInfo {
            auth_token: Some(proto::AuthToken { session_uuid: uuid }),
            query_parameters: self.query_parameters.clone(),
            client_type: self.client_type.clone(),
            // The JVM terminal fills this with its own build git commit
            // hash. This SDK has no such commit to report, so we leave it
            // empty; the server accepts empty strings here.
            terminal_git_commit: String::new(),
            terminal_version: TERMINAL_VERSION.to_string(),
        }
    }

    /// Access the shared session token. Crate-internal — the retry
    /// wrapper snapshots + refreshes through this.
    pub(crate) fn session(&self) -> &SessionToken {
        &self.session
    }

    /// Pick the next gRPC channel for an outbound RPC.
    ///
    /// Each call advances the round-robin cursor in the underlying
    /// [`ChannelPool`], spreading load across multiple HTTP/2
    /// connections so workloads exceed the per-connection
    /// `MAX_CONCURRENT_STREAMS` ceiling.
    ///
    /// Returns a [`crate::grpc::ChannelLease`] that pre-reserves a
    /// slot on the picked channel so concurrent dispatches observe
    /// the reservation immediately rather than racing on a stale
    /// `in_flight = 0` snapshot. The lease derefs to `&Channel` so
    /// the call shape stays unchanged.
    pub(crate) fn channel(&self) -> crate::grpc::ChannelLease<'_> {
        self.channels.next()
    }

    /// Return a reference to the underlying config for diagnostics.
    #[must_use]
    pub fn config(&self) -> &DirectConfig {
        &self.config
    }

    /// Return the session UUID. Reads through the shared session token
    /// so the value reflects any mid-session refresh.
    pub async fn session_uuid(&self) -> String {
        self.session.current_uuid().await
    }

    /// Stock subscription tier captured at authentication time, decoded
    /// from the Nexus auth response. `None` when the response omits the
    /// stock tier or carries an unknown wire value.
    #[doc(hidden)]
    #[must_use]
    pub fn stock_tier(&self) -> Option<SubscriptionTier> {
        self.stock_tier
    }

    /// Options subscription tier captured at authentication time. Same
    /// semantics as [`Self::stock_tier`].
    #[doc(hidden)]
    #[must_use]
    pub fn options_tier(&self) -> Option<SubscriptionTier> {
        self.options_tier
    }

    /// Indices subscription tier captured at authentication time. Same
    /// semantics as [`Self::stock_tier`].
    #[doc(hidden)]
    #[must_use]
    pub fn indices_tier(&self) -> Option<SubscriptionTier> {
        self.indices_tier
    }

    /// Interest-rate / Treasury curve subscription tier captured at
    /// authentication time. Same semantics as [`Self::stock_tier`].
    #[doc(hidden)]
    #[must_use]
    pub fn interest_rate_tier(&self) -> Option<SubscriptionTier> {
        self.interest_rate_tier
    }

    /// Test-only constructor that bypasses the Nexus auth handshake.
    ///
    /// Gated behind the private `__test-helpers` feature flag so the
    /// symbol never enters the published rlib for downstream
    /// consumers. The integration tests under `tests/` activate the
    /// feature via their `[[test]] required-features` row in `Cargo.toml`.
    ///
    /// `channels` must be non-empty (panics otherwise via
    /// `ChannelPool::from_channels`). Supply a usable mock-backed
    /// channel so the pool's `Drop` order does not trip an
    /// unconnected-channel assertion.
    #[cfg(any(test, feature = "__test-helpers"))]
    #[doc(hidden)]
    #[must_use]
    pub fn for_endpoint_routing_test(
        config: DirectConfig,
        channels: ChannelPool,
        request_semaphore: Arc<tokio::sync::Semaphore>,
    ) -> Self {
        let creds = Credentials::new("test", "test");
        let session = SessionToken::new(
            "00000000-0000-0000-0000-000000000000".to_string(),
            config.auth.nexus_url.clone(),
            config.historical_environment,
            creds,
        );
        let mut query_parameters = HashMap::new();
        query_parameters.insert("client".to_string(), "terminal".to_string());
        let client_type = config.auth.client_type.clone();
        Self {
            session,
            channels,
            config,
            query_parameters,
            client_type,
            request_semaphore,
            stock_tier: None,
            options_tier: None,
            indices_tier: None,
            interest_rate_tier: None,
        }
    }
}

/// Channel-pool sizing — resolved purely from the subscription tier.
///
/// The server enforces a hard per-tier cap on concurrent in-flight
/// gRPC requests (Free=1 / Value=2 / Standard=4 / Pro=8). The SDK
/// sizes the channel pool to exactly that cap so the live pool always
/// stays inside the server-side ceiling — there is no caller-supplied
/// concurrency knob to over-provision and trip per-RPC
/// `ResourceExhausted` rejections.
///
/// # Resolution
///
/// 1. tier resolved from the auth response → that tier's cap.
/// 2. no tier on the auth response (anonymous channel, dev harness)
///    → `DEFAULT_POOL_SIZE`.
fn effective_pool_size(auth_resp: &crate::auth::nexus::AuthResponse) -> usize {
    const DEFAULT_POOL_SIZE: usize = 4;
    let from_tier = auth_resp
        .user
        .as_ref()
        .map_or(0, crate::auth::nexus::AuthUser::max_concurrent_requests);
    if from_tier > 0 {
        from_tier
    } else {
        DEFAULT_POOL_SIZE
    }
}

/// Open `pool_size` independent gRPC channels and wrap them in a
/// [`ChannelPool`]. Channels are opened sequentially so a transient
/// failure on the first call fails the whole pool fast rather than
/// leaving a half-built pool behind.
///
/// Each channel is built with `config.historical.max_message_size` so the
/// configured per-frame ceiling propagates to every RPC dispatched on
/// the pool — oversized response frames are rejected by the decode
/// layer rather than buffered past the configured bound.
///
/// Per-chunk payload decode (zstd + protobuf) runs inline on each
/// request's task rather than a dedicated decode pool, keeping each
/// chunk on its producing connection and avoiding cross-thread
/// hand-off at every production-reachable concurrency, including
/// multi-chunk fan-in.
async fn open_channel_pool(
    host: &str,
    port: u16,
    tls: bool,
    pool_size: usize,
    config: &DirectConfig,
) -> Result<ChannelPool, Error> {
    let connect_timeout = Duration::from_secs(config.historical.connect_timeout_secs);
    let max_message_size = config.historical.max_message_size;
    // HTTP/2 session tuning from the operator's config: flow-control
    // windows (`window_size_kb` / `connection_window_size_kb`, already
    // clamped to [64, 1024] KB by `DirectConfig::validate`) and the
    // keepalive cadence (`keepalive_secs` / `keepalive_timeout_secs`).
    let tuning = ChannelTuning {
        initial_stream_window_size: u32::try_from(
            config.historical.window_size_kb.saturating_mul(1024),
        )
        .unwrap_or(u32::MAX),
        initial_connection_window_size: u32::try_from(
            config
                .historical
                .connection_window_size_kb
                .saturating_mul(1024),
        )
        .unwrap_or(u32::MAX),
        keepalive_interval: Duration::from_secs(config.historical.keepalive_secs.max(1)),
        keepalive_timeout: Duration::from_secs(config.historical.keepalive_timeout_secs.max(1)),
    };
    // `rustls::ClientConfig` is designed for `Arc` sharing across
    // connections — the root store + ALPN list are immutable after
    // construction. Build once and clone the `Arc` into every
    // channel in the pool rather than rebuilding the webpki roots
    // and the cipher-suite tables on each iteration.
    let tls_config = if tls {
        Some(build_rustls_config()?)
    } else {
        None
    };
    let mut channels = Vec::with_capacity(pool_size);
    for idx in 0..pool_size {
        let channel = if let Some(tls_config) = tls_config.as_ref() {
            tokio::time::timeout(
                connect_timeout,
                Channel::connect_tls_tuned(
                    host,
                    port,
                    tls_config.clone(),
                    max_message_size,
                    tuning,
                    connect_timeout,
                ),
            )
            .await
            .map_err(|_| {
                // A connect timeout means the server was unreachable or
                // black-holed, not that the caller's config is wrong. Classify
                // it as a transport fault (`ConnectionClosed`) so the retry
                // shell treats it as transient/retryable like other transport
                // faults, instead of a terminal `Config` misconfiguration.
                Error::Transport {
                    kind: crate::error::TransportErrorKind::ConnectionClosed,
                    message: format!(
                        "tls connect to {host}:{port} timed out after {}s",
                        config.historical.connect_timeout_secs
                    ),
                }
            })?
        } else {
            tokio::time::timeout(
                connect_timeout,
                Channel::connect_h2c_tuned(host, port, max_message_size, tuning, connect_timeout),
            )
            .await
            .map_err(|_| {
                // See the TLS branch above: a connect timeout is an
                // unreachable-server transport fault, not a config error, so it
                // is classified `ConnectionClosed` and stays retryable.
                Error::Transport {
                    kind: crate::error::TransportErrorKind::ConnectionClosed,
                    message: format!(
                        "h2c connect to {host}:{port} timed out after {}s",
                        config.historical.connect_timeout_secs
                    ),
                }
            })?
        }
        .map_err(|e| {
            // Route through the canonical `From<ChannelError> for Error`
            // so every transport-fault category (TCP / TLS /
            // InvalidServerName / H2Handshake / H2Stream /
            // ConnectionClosed) maps to the right `TransportErrorKind`
            // without a local duplicate match. Preserve the
            // channel-index hint by re-wrapping the `Transport`-shaped
            // output's message — other variants (Timeout / Grpc) keep
            // their original shape so retry classifiers downstream
            // still dispatch correctly. SSOT for the kind-map lives in
            // `error::From<ChannelError> for Error`.
            match Error::from(e) {
                Error::Transport { kind, message } => Error::Transport {
                    kind,
                    message: format!("channel {idx}: {message}"),
                },
                other => other,
            }
        })?;
        channels.push(channel);
    }
    Ok(ChannelPool::from_channels(channels))
}

/// Build a `rustls::ClientConfig` with webpki roots and `h2` advertised
/// in the ALPN list. gRPC over HTTP/2 requires the connection to
/// negotiate to `h2`.
fn build_rustls_config() -> Result<Arc<rustls::ClientConfig>, Error> {
    let mut root_store = rustls::RootCertStore::empty();
    for cert in webpki_roots::TLS_SERVER_ROOTS.iter().cloned() {
        root_store.roots.push(cert);
    }
    // Build the config with an explicit ring provider so the handshake needs
    // no process-global default. ring is the sole provider in the dep graph.
    let mut config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(root_store)
    .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec()];
    Ok(Arc::new(config))
}

#[cfg(test)]
mod pool_size_tests {
    use super::effective_pool_size;
    use crate::auth::nexus::AuthResponse;
    #[cfg(feature = "__internal")]
    use crate::auth::nexus::AuthUser;

    /// Build an AuthResponse whose user reports the given subscription
    /// wire byte. `AuthUser::max_concurrent_requests` maps that into
    /// the per-tier cap (`2^tier`).
    #[cfg(feature = "__internal")]
    fn auth_with_tier(stock_sub: Option<i32>) -> AuthResponse {
        AuthResponse {
            session_id: "session".to_string(),
            user: Some(AuthUser {
                email: None,
                stock_subscription: stock_sub,
                options_subscription: None,
                indices_subscription: None,
                interest_rate_subscription: None,
            }),
            session_created: None,
        }
    }

    #[cfg(feature = "__internal")]
    #[test]
    fn pool_size_tracks_tier_cap() {
        // The pool is sized to exactly the tier cap: Free=1, Value=2,
        // Standard=4, Pro=8 (subscription wire bytes 0..=3).
        for (sub_byte, expected) in [(0, 1), (1, 2), (2, 4), (3, 8)] {
            let auth = auth_with_tier(Some(sub_byte));
            assert_eq!(effective_pool_size(&auth), expected);
        }
    }

    #[test]
    fn pool_size_falls_back_to_default_when_no_tier() {
        // No auth user (anonymous channel, dev harness) — the
        // hardcoded `4` is the last resort.
        let auth = AuthResponse {
            session_id: "session".to_string(),
            user: None,
            session_created: None,
        };
        assert_eq!(effective_pool_size(&auth), 4);
    }
}

#[cfg(test)]
mod connect_timeout_tests {
    use super::open_channel_pool;
    use crate::config::DirectConfig;
    use crate::error::{Error, TransportErrorKind};

    /// A connect that exceeds the configured timeout must surface as a
    /// transport-class fault (`ConnectionClosed`), NOT a config error. The
    /// retry shell classifies `Transport { ConnectionClosed }` as transient
    /// (`crate::mdds::macros::classify_error`), so a black-holed / unreachable
    /// server is retried instead of being misreported as caller
    /// misconfiguration.
    ///
    /// The TLS path drives this deterministically: the client must receive the
    /// server's `ServerHello` before `connect_tls_tuned` resolves, so a peer
    /// that accepts the TCP connection but never speaks TLS holds the eager
    /// connect open until the `tokio::time::timeout` in `open_channel_pool`
    /// elapses. (An h2c connect cannot be used here: the hyper HTTP/2 client
    /// handshake resolves as soon as it has sent its own preface, without
    /// awaiting the server's SETTINGS, so a stalled h2c peer is reported ready.)
    #[tokio::test]
    async fn connect_timeout_is_transport_not_config() {
        // Bind a listener that accepts connections and then stalls forever:
        // the kernel completes the TCP handshake, but the server never sends a
        // TLS `ServerHello`, so the gRPC TLS connect hangs until our 1 s
        // deadline fires.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback listener");
        let addr = listener.local_addr().expect("listener addr");
        let accept = tokio::spawn(async move {
            // Hold every accepted stream so the peer never closes the socket;
            // the connect must time out rather than see a reset.
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });

        let mut config = DirectConfig::production_defaults();
        // Short, deterministic deadline; TLS so the handshake blocks on the
        // peer's first flight rather than resolving optimistically.
        config.historical.connect_timeout_secs = 1;
        config.historical.tls = true;

        let result = open_channel_pool(&addr.ip().to_string(), addr.port(), true, 1, &config).await;

        accept.abort();

        match result {
            Err(Error::Transport { kind, message }) => {
                assert_eq!(
                    kind,
                    TransportErrorKind::ConnectionClosed,
                    "a connect timeout must be a retryable transport fault; got message: {message}"
                );
                assert!(
                    message.contains("timed out"),
                    "message should describe the timeout: {message}"
                );
            }
            Err(Error::Config { .. }) => {
                panic!("connect timeout misreported as a (terminal) Config error")
            }
            Err(other) => panic!("expected Error::Transport(ConnectionClosed), got {other:?}"),
            Ok(_) => panic!("connect to a stalled peer must not succeed"),
        }
    }
}
