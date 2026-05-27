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
use std::sync::{Arc, OnceLock, RwLock};
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
    indices_tier: Option<SubscriptionTier>,
    interest_rate_tier: Option<SubscriptionTier>,
    /// Lazily-built [`crate::rest::RestClient`] cache keyed by
    /// `base_url`. The REST fallback shims (`option_history_*_with_fallback`,
    /// defined in [`super::fallback`]) previously built a fresh
    /// `RestClient` per call, dragging the per-call cost up by a
    /// `reqwest::Client` construction (TLS context + connection pool
    /// init) on every fallback dispatch. One handle per distinct base
    /// URL is shared for the lifetime of the [`MddsClient`] -- reuses
    /// the underlying HTTP/2 connection pool across calls.
    pub(crate) rest_clients: OnceLock<RwLock<HashMap<String, Arc<crate::rest::RestClient>>>>,
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
        //
        // The Nexus URL itself encodes deployment topology that operators
        // rarely need at `info` — keep the URL behind `trace` verbosity
        // so production deployments do not record it by default. Mirrors
        // the same downgrade applied to `auth/nexus.rs`.
        tracing::info!("authenticating with Nexus API");
        tracing::trace!(nexus_url = %config.auth.nexus_url, "Nexus auth URL");
        let auth_resp = auth::authenticate_at(&config.auth.nexus_url, creds).await?;
        let session_uuid = auth_resp.session_id.clone();

        tracing::debug!(
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

        // The request semaphore must match the resolved channel pool
        // size so the (N+1)-th in-flight RPC can never claim a permit
        // before there's a channel free to carry it. `pool_size`
        // already reflects the tier-clamped, auto-detected resolution
        // from `effective_pool_size`; reusing it keeps the semaphore
        // and the channel count strictly coupled.
        let request_semaphore = Arc::new(tokio::sync::Semaphore::new(pool_size));

        tracing::debug!(
            mdds_concurrent_requests = pool_size,
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
            indices_tier,
            interest_rate_tier,
            rest_clients: OnceLock::new(),
        })
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

    /// Indices subscription tier captured at authentication time. Same
    /// semantics as [`Self::stock_tier`].
    #[must_use]
    pub fn indices_tier(&self) -> Option<SubscriptionTier> {
        self.indices_tier
    }

    /// Interest-rate / Treasury curve subscription tier captured at
    /// authentication time. Same semantics as [`Self::stock_tier`].
    #[must_use]
    pub fn interest_rate_tier(&self) -> Option<SubscriptionTier> {
        self.interest_rate_tier
    }

    /// Test-only constructor that bypasses the Nexus auth handshake.
    ///
    /// Gated behind the private `__test-helpers` feature flag so the
    /// symbol never enters the published rlib for downstream
    /// consumers. The integration test at
    /// `tests/test_with_fallback_end_date.rs` activates the feature
    /// via its `[[test]] required-features` row in `Cargo.toml`.
    ///
    /// `channels` must be non-empty (panics otherwise via
    /// `ChannelPool::from_channels`). Callers exercising only the REST
    /// arm should still supply a usable mock-backed channel so the
    /// pool's `Drop` order does not trip an unconnected-channel
    /// assertion.
    #[cfg(any(test, feature = "__test-helpers"))]
    #[doc(hidden)]
    #[must_use]
    pub fn for_fallback_test(
        config: DirectConfig,
        channels: ChannelPool,
        request_semaphore: Arc<tokio::sync::Semaphore>,
    ) -> Self {
        let creds = Credentials::new("test", "test");
        let session = SessionToken::new(
            "00000000-0000-0000-0000-000000000000".to_string(),
            config.auth.nexus_url.clone(),
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
            rest_clients: OnceLock::new(),
        }
    }
}

/// Pool sizing — `concurrent_requests` from `DirectConfig` is the
/// explicit caller intent, **clamped to the subscription tier cap**.
/// Subscription tier supplies the default when `concurrent_requests = 0`
/// and the upper bound when it's set explicitly.
///
/// # Why clamp explicit caller intent
///
/// ThetaData enforces a hard server-side cap on concurrent in-flight
/// gRPC requests per tier (Free=1 / Value=2 / Standard=4 / Pro=8).
/// A caller asking for `concurrent_requests = 32` on a Pro tier
/// previously got 32 channels opened; the (Pro_cap + 1)-th RPC then
/// failed with an upstream `ResourceExhausted` per-stream rejection,
/// which the SDK retried on a different channel, producing a confusing
/// "everything fails intermittently" symptom. Clamping locally
/// surfaces the misconfiguration as a `tracing::warn!` on connect and
/// keeps the live pool inside the tier's headroom.
///
/// The `override_tier_clamp` escape hatch on `MddsConfig` bypasses
/// the clamp — test-only, used to reproduce the over-provisioning
/// failure mode against a stubbed auth response.
///
/// # Resolution ladder
///
/// 1. `concurrent_requests > 0` ∧ `from_tier > 0` ∧ `!override` →
///    `min(from_config, from_tier)` (clamp + warn if `from_config > from_tier`)
/// 2. `concurrent_requests > 0` ∧ (`from_tier = 0` ∨ `override`) →
///    `from_config` (no tier reference available, or operator bypass)
/// 3. `concurrent_requests = 0` ∧ `from_tier > 0` → `from_tier` (auto-detect)
/// 4. `concurrent_requests = 0` ∧ `from_tier = 0` → `DEFAULT_POOL_SIZE`
fn effective_pool_size(
    config: &DirectConfig,
    auth_resp: &crate::auth::nexus::AuthResponse,
) -> usize {
    const DEFAULT_POOL_SIZE: usize = 4;
    let from_config = config.mdds.concurrent_requests;
    let from_tier = auth_resp
        .user
        .as_ref()
        .map_or(0, crate::auth::nexus::AuthUser::max_concurrent_requests);
    if from_config > 0 {
        // Explicit caller intent — clamp to tier cap so a configured
        // value above the server-side ceiling does not produce
        // confusing per-RPC `ResourceExhausted` rejections downstream.
        // The escape hatch (`override_tier_clamp`) bypasses the clamp
        // for tests that need to reproduce the misconfiguration.
        if from_tier > 0 && !config.mdds.override_tier_clamp && from_config > from_tier {
            tracing::warn!(
                configured = from_config,
                tier_cap = from_tier,
                "mdds.concurrent_requests exceeds subscription tier cap — clamping to tier cap; \
                 set MddsConfig.override_tier_clamp = true to bypass (tests only)"
            );
            return from_tier;
        }
        return from_config.max(1);
    }
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
/// rather than the tokio reactor. Stage-1 (zstd decompress) sizing
/// is driven by [`default_decoder_thread_count`]; stage-2
/// (prost-decode + Tick-build) sizing is driven by
/// [`crate::config::MddsConfig::decode_threads`] /
/// [`crate::config::MddsConfig::decode_queue_depth`].
async fn open_channel_pool(
    host: &str,
    port: u16,
    tls: bool,
    pool_size: usize,
    config: &DirectConfig,
) -> Result<ChannelPool, Error> {
    let connect_timeout = Duration::from_secs(config.mdds.connect_timeout_secs);
    let max_message_size = config.mdds.max_message_size;
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
                Channel::connect_tls_with_max_message_size(
                    host,
                    port,
                    tls_config.clone(),
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
        .map_err(|e| {
            // Route through the canonical `From<ChannelError> for Error`
            // so every transport-fault category (TCP / TLS /
            // InvalidServerName / H2Handshake / H2Stream / Codec /
            // EmptyResponse / UnexpectedHttpStatus / ConnectionClosed)
            // maps to the right `TransportErrorKind` without a local
            // duplicate match. Preserve the channel-index hint by
            // re-wrapping the `Transport`-shaped output's message —
            // other variants (Timeout / Grpc) keep their original
            // shape so retry classifiers downstream still dispatch
            // correctly. SSOT for the kind-map lives in
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
    let stage1_threads = default_decoder_thread_count();
    let stage2_threads = config
        .mdds
        .decode_threads
        .map_or_else(default_stage2_thread_count, |n| n.max(1));
    let queue_depth = config
        .mdds
        .decode_queue_depth
        .map_or_else(|| default_stage2_queue_depth(pool_size), |n| n.max(1));
    let decoder_pool = DecoderPool::new_two_stage(
        stage1_threads,
        config.mdds.decoder_ring_size,
        stage2_threads,
        queue_depth,
    )
    .map_err(Error::from)?;
    tracing::debug!(
        stage1_threads = decoder_pool.len(),
        stage2_threads,
        decoder_ring_size = config.mdds.decoder_ring_size,
        queue_depth,
        "MDDS two-stage decode pipeline initialised"
    );
    Ok(ChannelPool::from_channels_with_decoders(
        channels,
        decoder_pool,
    ))
}

/// Default stage-2 worker count when [`crate::config::MddsConfig::decode_threads`]
/// is `None`. Uses [`std::thread::available_parallelism`] with a
/// minimum of `1` — stage-2 is parser-bound, so the full core
/// count is the right starting point (unlike stage-1 which is
/// capped against the per-channel decoder count).
#[must_use]
fn default_stage2_thread_count() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(2)
        .max(1)
}

/// Default queue depth between stage-1 and stage-2 when
/// [`crate::config::MddsConfig::decode_queue_depth`] is `None`.
/// Sizes to `pool_size * 64` so a 64-way burst on every channel
/// has headroom without exhausting buffer memory.
#[must_use]
fn default_stage2_queue_depth(pool_size: usize) -> usize {
    pool_size.saturating_mul(64).max(64)
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
    fn explicit_below_tier_cap_honoured() {
        // Caller asks for exactly 1 channel; auth response carries a
        // Pro tier (subscription byte 3 -> 2^3 = 8 concurrent). The
        // explicit caller intent is below the tier cap, so honour it
        // exactly without inflating.
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
    fn explicit_above_tier_cap_clamped() {
        // Caller asks for 32 channels but tier is Free (byte 0 -> 1
        // concurrent). The clamp folds the configured value to the
        // tier cap so the per-RPC `ResourceExhausted` rejections
        // never surface — the local warn is the SDK's friendly
        // boundary against the server-side cap.
        let mut config = DirectConfig::production_defaults();
        config.mdds.concurrent_requests = 32;
        let auth = auth_with_tier(Some(0));
        assert_eq!(effective_pool_size(&config, &auth), 1);
    }

    #[test]
    fn explicit_above_pro_cap_clamped_to_pro() {
        // Pro tier permits 8. Caller asks for 16. Clamp to 8.
        let mut config = DirectConfig::production_defaults();
        config.mdds.concurrent_requests = 16;
        let auth = auth_with_tier(Some(3));
        assert_eq!(effective_pool_size(&config, &auth), 8);
    }

    #[test]
    fn override_tier_clamp_bypasses_clamp() {
        // The internal escape hatch lets tests reproduce the
        // over-provisioning failure mode against a stubbed Free-tier
        // auth response. With the override on, the configured value
        // passes through unmodified — useful for asserting downstream
        // behaviour against an explicitly mis-sized pool.
        let mut config = DirectConfig::production_defaults();
        config.mdds.concurrent_requests = 16;
        config.mdds.override_tier_clamp = true;
        let auth = auth_with_tier(Some(0));
        assert_eq!(effective_pool_size(&config, &auth), 16);
    }

    #[test]
    fn no_tier_response_does_not_clamp() {
        // Auth response carries no tier (anonymous channel, dev
        // harness, etc.). The clamp arm is skipped entirely — the
        // configured value passes through. The default-pool fallback
        // only triggers for `concurrent_requests = 0`.
        let mut config = DirectConfig::production_defaults();
        config.mdds.concurrent_requests = 16;
        let auth = AuthResponse {
            session_id: "session".to_string(),
            user: None,
            session_created: None,
        };
        assert_eq!(effective_pool_size(&config, &auth), 16);
    }
}
