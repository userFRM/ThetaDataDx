//! `MddsClient` struct, connection lifecycle, and session/transport state.
//!
//! This module owns the MDDS gRPC client type: its fields (session UUID,
//! channel, config, request semaphore, subscription tiers), its
//! [`connect`](MddsClient::connect) constructor, and the small read-only
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
use crate::grpc::{default_decoder_thread_count, Channel, ChannelPool, DecoderPool};
use crate::mdds::tier::SubscriptionTier;
use crate::proto;

/// Version string sent in `QueryInfo.terminal_version`.
const TERMINAL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// MDDS gRPC client for `ThetaData` server access.
///
/// Connects to MDDS (gRPC, historical data) without requiring the Java
/// terminal. Authenticates via the Nexus HTTP API, then issues gRPC
/// requests to the upstream MDDS server.
///
/// # Example
///
/// ```rust,no_run
/// use thetadatadx::{ThetaDataDxClient, Credentials, DirectConfig};
///
/// # async fn run() -> Result<(), thetadatadx::Error> {
/// let creds = Credentials::from_file("creds.txt")?;
/// let tdx = ThetaDataDxClient::connect(&creds, DirectConfig::production()).await?;
///
/// let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
/// println!("{} EOD ticks", eod.len());
/// # Ok(())
/// # }
/// ```
pub struct MddsClient {
    /// Shared, mutable session token. Every request reads the current
    /// UUID via this handle; `Unauthenticated` responses trigger a
    /// single-shot refresh that swaps the UUID in place. See
    /// [`crate::auth::SessionToken`].
    session: SessionToken,
    /// Pool of in-house gRPC channels to the MDDS server. Round-robin
    /// dispatch lets workloads exceed the per-connection
    /// `MAX_CONCURRENT_STREAMS` ceiling.
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
    /// The Java terminal limits concurrent requests to `2^subscription_tier`
    /// (Free=1, Value=2, Standard=4, Pro=8). This semaphore enforces the same
    /// bound to prevent server-side rate limiting / 429 disconnects.
    pub(crate) request_semaphore: Arc<tokio::sync::Semaphore>,
    /// Per-asset subscription tiers captured from the Nexus auth response.
    /// `None` for asset classes the auth response omits or for unknown
    /// wire values (the wire byte is preserved in the structured logs at
    /// connect time but never silently coerced into a tier).
    stock_tier: Option<SubscriptionTier>,
    options_tier: Option<SubscriptionTier>,
}

// ── Infrastructure (not generated — these are session/transport methods, not ThetaData endpoints) ──

impl MddsClient {
    /// Connect to `ThetaData` servers directly (no JVM terminal needed).
    ///
    /// 1. Authenticates against the Nexus HTTP API to obtain a session UUID.
    /// 2. Opens a gRPC channel (TLS) to the MDDS server.
    ///
    /// The FPSS (real-time streaming) connection is not established here;
    /// it will be added in a future release.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error> {
        // Step 1: Authenticate against Nexus API using the configured URL
        // (env-var / builder overridable). `config.auth.nexus_url` already
        // reflects that precedence via `DirectConfig::production()`.
        tracing::info!(nexus_url = %config.auth.nexus_url, "authenticating with Nexus API");
        let auth_resp = auth::authenticate_at(&config.auth.nexus_url, creds).await?;
        let session_uuid = auth_resp.session_id.clone();

        tracing::debug!(
            session_id_prefix = %&session_uuid[..8.min(session_uuid.len())],
            stock_tier = ?auth_resp.user.as_ref().and_then(|u| u.stock_subscription),
            "session established (session_id redacted)"
        );

        // Step 2: Open the gRPC channel pool to MDDS.
        let host = config.mdds.host.clone();
        let port = config.mdds.port;
        tracing::debug!(host = %host, port, tls = config.mdds.tls, "connecting to MDDS gRPC");

        let pool_size = effective_pool_size(&config, &auth_resp);
        let channels = open_channel_pool(&host, port, config.mdds.tls, pool_size, &config).await?;
        tracing::info!(
            pool_size,
            "MDDS gRPC channel pool connected ({} h2 connections)",
            pool_size
        );

        let mut query_parameters = HashMap::new();
        // QueryInfo always includes `"client": "terminal"`.
        query_parameters.insert("client".to_string(), "terminal".to_string());

        // Auto-detect concurrency from subscription tier when config is 0.
        // Bound is 2^subscription_tier (FREE=1, VALUE=2, STANDARD=4, PRO=8).
        let concurrent = if config.mdds.concurrent_requests == 0 {
            auth_resp
                .user
                .as_ref()
                .map_or(2, crate::auth::nexus::AuthUser::max_concurrent_requests)
        } else {
            config.mdds.concurrent_requests
        };

        let request_semaphore = Arc::new(tokio::sync::Semaphore::new(concurrent));

        tracing::debug!(
            mdds_concurrent_requests = concurrent,
            auto_detected = config.mdds.concurrent_requests == 0,
            "request semaphore initialized"
        );

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

        let session = SessionToken::new(session_uuid, config.auth.nexus_url.clone(), creds.clone());
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
        })
    }

    /// Build a fresh `QueryInfo` pinned to the current session UUID.
    ///
    /// Returned value is owned — every field is cloned from shared state.
    /// The session UUID is read from [`SessionToken`] so a mid-session
    /// refresh (see [`Self::session`]) automatically propagates to every
    /// subsequent request without rebuilding the client.
    pub(crate) async fn query_info(&self) -> proto::QueryInfo {
        let uuid = self.session.current_uuid().await;
        self.build_query_info(uuid)
    }

    /// Construct a `QueryInfo` around a caller-supplied UUID. Used by
    /// the retry wrapper to pin an in-flight attempt to the exact UUID
    /// seen by [`crate::auth::SessionToken::snapshot`] — so when a
    /// concurrent refresh advances the token, we don't accidentally
    /// mix old and new UUIDs on the same request.
    pub(crate) fn build_query_info(&self, uuid: String) -> proto::QueryInfo {
        proto::QueryInfo {
            auth_token: Some(proto::AuthToken { session_uuid: uuid }),
            query_parameters: self.query_parameters.clone(),
            client_type: self.client_type.clone(),
            // Intentional divergence from Java (see jvm-deviations.md):
            // Java fills this with the terminal's build git commit hash.
            // We are not the Java terminal and have no git commit to report,
            // so we leave it empty. The server accepts empty strings here.
            terminal_git_commit: String::new(),
            terminal_version: TERMINAL_VERSION.to_string(),
        }
    }

    /// Access the shared session token. Crate-internal — the retry
    /// wrapper snapshots + refreshes through this.
    pub(crate) fn session(&self) -> &SessionToken {
        &self.session
    }

    /// Pick the next in-house gRPC channel for an outbound RPC.
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

    /// Return the session UUID. Reads through the shared
    /// [`SessionToken`] so the value reflects any mid-session refresh.
    pub async fn session_uuid(&self) -> String {
        self.session.current_uuid().await
    }

    /// Stock subscription tier captured at authentication time, decoded
    /// from the Nexus auth response. `None` when the response omits the
    /// stock tier or carries an unknown wire value.
    #[must_use]
    pub fn stock_tier(&self) -> Option<SubscriptionTier> {
        self.stock_tier
    }

    /// Options subscription tier captured at authentication time. Same
    /// semantics as [`Self::stock_tier`].
    #[must_use]
    pub fn options_tier(&self) -> Option<SubscriptionTier> {
        self.options_tier
    }
}

/// Pool sizing — `concurrent_requests` from `DirectConfig` is the
/// explicit caller intent and wins outright when set (non-zero).
/// Subscription tier is a *default* the caller falls back to when
/// `concurrent_requests = 0`; tier never overrides an explicit
/// configured value.
///
/// The previous formula used `max(from_config, from_tier)`, which
/// silently inflated `concurrent_requests = 1` (one in-flight RPC)
/// to a Pro tier's 8-channel pool — that hid the caller's explicit
/// intent behind a tier-based floor. The fix: configured value wins
/// when non-zero, tier supplies the default only when configured
/// value is zero, hardcoded `4` is the last-resort default.
fn effective_pool_size(
    config: &DirectConfig,
    auth_resp: &crate::auth::nexus::AuthResponse,
) -> usize {
    const DEFAULT_POOL_SIZE: usize = 4;
    let from_config = config.mdds.concurrent_requests;
    if from_config > 0 {
        // Explicit caller intent — honour it exactly. No tier floor.
        return from_config.max(1);
    }
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
/// Each channel is built with `config.mdds.max_message_size` so the
/// configured per-frame ceiling propagates to every RPC dispatched on
/// the pool — oversized response frames surface as
/// [`crate::grpc::CodecError::FrameTooLarge`] rather than the codec
/// module's hardcoded 4 MiB default.
///
/// A dedicated [`DecoderPool`] is attached to the channel pool
/// so every RPC's zstd + protobuf decode runs on a worker thread
/// rather than the tokio reactor. Sizing is driven by
/// [`crate::config::MddsConfig::decoder_threads`] / `decoder_ring_size`,
/// falling back to [`default_decoder_thread_count`] when the
/// configured count is zero.
async fn open_channel_pool(
    host: &str,
    port: u16,
    tls: bool,
    pool_size: usize,
    config: &DirectConfig,
) -> Result<ChannelPool, Error> {
    let connect_timeout = Duration::from_secs(config.mdds.connect_timeout_secs);
    let max_message_size = config.mdds.max_message_size;
    let mut channels = Vec::with_capacity(pool_size);
    for idx in 0..pool_size {
        let channel = if tls {
            let tls_config = build_rustls_config()?;
            tokio::time::timeout(
                connect_timeout,
                Channel::connect_tls_with_max_message_size(
                    host,
                    port,
                    tls_config,
                    max_message_size,
                ),
            )
            .await
            .map_err(|_| {
                Error::config_invalid(
                    "mdds.connect_timeout_secs",
                    format!(
                        "tls connect to {host}:{port} timed out after {}s",
                        config.mdds.connect_timeout_secs
                    ),
                )
            })?
        } else {
            tokio::time::timeout(
                connect_timeout,
                Channel::connect_h2c_with_max_message_size(host, port, max_message_size),
            )
            .await
            .map_err(|_| {
                Error::config_invalid(
                    "mdds.connect_timeout_secs",
                    format!(
                        "h2c connect to {host}:{port} timed out after {}s",
                        config.mdds.connect_timeout_secs
                    ),
                )
            })?
        }
        .map_err(|e| Error::Transport(format!("channel {idx}: {e}")))?;
        channels.push(channel);
    }
    let decoder_threads = if config.mdds.decoder_threads == 0 {
        default_decoder_thread_count(pool_size)
    } else {
        config.mdds.decoder_threads
    };
    let decoder_pool =
        DecoderPool::new(decoder_threads, config.mdds.decoder_ring_size).map_err(Error::from)?;
    tracing::debug!(
        decoder_threads = decoder_pool.len(),
        decoder_ring_size = config.mdds.decoder_ring_size,
        "MDDS decoder pool initialised"
    );
    Ok(ChannelPool::from_channels_with_decoders(
        channels,
        decoder_pool,
    ))
}

/// Build a `rustls::ClientConfig` with webpki roots and `h2` advertised
/// in the ALPN list. gRPC over HTTP/2 requires the connection to
/// negotiate to `h2`.
fn build_rustls_config() -> Result<Arc<rustls::ClientConfig>, Error> {
    let mut root_store = rustls::RootCertStore::empty();
    for cert in webpki_roots::TLS_SERVER_ROOTS.iter().cloned() {
        root_store.roots.push(cert);
    }
    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec()];
    Ok(Arc::new(config))
}

#[cfg(test)]
mod pool_size_tests {
    use super::effective_pool_size;
    use crate::auth::nexus::{AuthResponse, AuthUser};
    use crate::config::DirectConfig;

    /// Build an AuthResponse whose user reports the given subscription
    /// wire bytes. `AuthUser::max_concurrent_requests` maps those into
    /// `2^tier` — the same shape the JVM terminal uses.
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

    #[test]
    fn explicit_concurrent_requests_overrides_tier() {
        // Caller asks for exactly 1 channel; auth response carries a
        // Pro tier (subscription byte 3 -> 2^3 = 8 concurrent). The
        // explicit caller intent must win — the previous behaviour
        // inflated this to 8 silently.
        let mut config = DirectConfig::production_defaults();
        config.mdds.concurrent_requests = 1;
        let auth = auth_with_tier(Some(3));
        assert_eq!(effective_pool_size(&config, &auth), 1);
    }

    #[test]
    fn explicit_concurrent_requests_capped_to_one() {
        // Edge case — concurrent_requests = 1 stays at 1. No tier
        // floor reaches in.
        let mut config = DirectConfig::production_defaults();
        config.mdds.concurrent_requests = 1;
        let auth = auth_with_tier(Some(3));
        assert_eq!(effective_pool_size(&config, &auth), 1);
    }

    #[test]
    fn auto_detect_falls_back_to_tier_when_config_is_zero() {
        // Caller signals "auto-detect" with concurrent_requests = 0;
        // tier (subscription byte 2 -> 4 concurrent) supplies the
        // default.
        let mut config = DirectConfig::production_defaults();
        config.mdds.concurrent_requests = 0;
        let auth = auth_with_tier(Some(2));
        assert_eq!(effective_pool_size(&config, &auth), 4);
    }

    #[test]
    fn auto_detect_falls_back_to_default_when_no_tier() {
        // Auto-detect + no auth user — hardcoded `4` is the last
        // resort.
        let mut config = DirectConfig::production_defaults();
        config.mdds.concurrent_requests = 0;
        let auth = AuthResponse {
            session_id: "session".to_string(),
            user: None,
            session_created: None,
        };
        assert_eq!(effective_pool_size(&config, &auth), 4);
    }

    #[test]
    fn explicit_eight_channels_with_low_tier_stays_at_eight() {
        // Caller explicitly asks for 8 channels; tier byte 0 would
        // map to 1 concurrent. The explicit intent wins.
        let mut config = DirectConfig::production_defaults();
        config.mdds.concurrent_requests = 8;
        let auth = auth_with_tier(Some(0));
        assert_eq!(effective_pool_size(&config, &auth), 8);
    }
}
