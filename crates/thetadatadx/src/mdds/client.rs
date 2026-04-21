//! `MddsClient` struct, connection lifecycle, and session/transport state.
//!
//! This module owns the MDDS gRPC client type: its fields (session UUID,
//! channel, config, request semaphore, subscription tiers), its
//! [`connect`](MddsClient::connect) constructor, and the small read-only
//! getters (`session_uuid`, `config`, `stock_tier`, `options_tier`, `channel`)
//! that expose client state to callers.
//!
//! Per-request helpers (`collect_stream`, `for_each_chunk`) live in
//! [`super::stream`]; parameter canonicalization (`normalize_*`, `wire_*`) in
//! [`super::normalize`]; date validation in [`super::validate`]; generated
//! endpoint method bodies in [`super::endpoints`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::{self, Credentials};
use crate::config::DirectConfig;
use crate::error::Error;
use crate::proto;
use crate::proto::beta_theta_terminal_client::BetaThetaTerminalClient;

/// Crate version embedded in `QueryInfo.terminal_version` so `ThetaData` can
/// identify this client in server-side logs.
const CLIENT_TYPE: &str = "rust-thetadatadx";

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
/// use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};
///
/// # async fn run() -> Result<(), thetadatadx::Error> {
/// let creds = Credentials::from_file("creds.txt")?;
/// let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
///
/// let eod = tdx.stock_history_eod("AAPL", "20240101", "20240301").await?;
/// println!("{} EOD ticks", eod.len());
/// # Ok(())
/// # }
/// ```
pub struct MddsClient {
    /// Session UUID from Nexus auth (embedded in every request).
    session_uuid: String,
    /// gRPC channel to MDDS server.
    pub(super) channel: tonic::transport::Channel,
    /// Configuration snapshot (retained for diagnostics/reconnect).
    pub(super) config: DirectConfig,
    /// Pre-built `QueryInfo` template — cloned per-request instead of allocating
    /// new Strings each time.
    query_info_template: proto::QueryInfo,
    /// Semaphore limiting concurrent in-flight gRPC requests.
    ///
    /// The Java terminal limits concurrent requests to `2^subscription_tier`
    /// (Free=1, Value=2, Standard=4, Pro=16). This semaphore enforces the same
    /// bound to prevent server-side rate limiting / 429 disconnects.
    pub(crate) request_semaphore: Arc<tokio::sync::Semaphore>,
    /// Per-asset subscription tiers captured from the Nexus auth response.
    stock_tier: Option<i32>,
    options_tier: Option<i32>,
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
        // Step 1: Authenticate against Nexus API.
        tracing::info!("authenticating with Nexus API");
        let auth_resp = auth::authenticate(creds).await?;
        let session_uuid = auth_resp.session_id.clone();

        tracing::debug!(
            session_id_prefix = %&session_uuid[..8.min(session_uuid.len())],
            stock_tier = ?auth_resp.user.as_ref().and_then(|u| u.stock_subscription),
            "session established (session_id redacted)"
        );

        // Step 2: Open gRPC channel to MDDS.
        let mdds_uri = config.mdds_uri();
        tracing::debug!(uri = %mdds_uri, "connecting to MDDS gRPC");

        let endpoint = tonic::transport::Channel::from_shared(mdds_uri.clone())
            .map_err(|e| Error::Config(format!("invalid MDDS URI '{mdds_uri}': {e}")))?
            .keep_alive_timeout(Duration::from_secs(config.mdds_keepalive_timeout_secs))
            .http2_keep_alive_interval(Duration::from_secs(config.mdds_keepalive_secs))
            .initial_stream_window_size(
                u32::try_from(config.mdds_window_size_kb * 1024).unwrap_or(u32::MAX),
            )
            .initial_connection_window_size(
                u32::try_from(config.mdds_connection_window_size_kb * 1024).unwrap_or(u32::MAX),
            )
            .connect_timeout(Duration::from_secs(10));

        let endpoint = if config.mdds_tls {
            endpoint.tls_config(tonic::transport::ClientTlsConfig::new().with_enabled_roots())?
        } else {
            endpoint
        };

        let channel = endpoint.connect().await?;
        tracing::info!("MDDS gRPC channel connected");

        let mut query_parameters = HashMap::new();
        // The Java terminal includes "client": "terminal" in every QueryInfo.
        // Source: MddsConnectionManager in decompiled terminal.
        query_parameters.insert("client".to_string(), "terminal".to_string());

        let query_info_template = proto::QueryInfo {
            auth_token: Some(proto::AuthToken {
                session_uuid: session_uuid.clone(),
            }),
            query_parameters,
            client_type: CLIENT_TYPE.to_string(),
            // Intentional divergence from Java (see jvm-deviations.md):
            // Java fills this with the terminal's build git commit hash.
            // We are not the Java terminal and have no git commit to report,
            // so we leave it empty. The server accepts empty strings here.
            terminal_git_commit: String::new(),
            terminal_version: TERMINAL_VERSION.to_string(),
        };

        // Auto-detect concurrency from subscription tier when config is 0.
        // Source: Java terminal uses 2^subscription_tier (FREE=1, VALUE=2, STANDARD=4, PRO=8).
        let concurrent = if config.mdds_concurrent_requests == 0 {
            auth_resp
                .user
                .as_ref()
                .map_or(2, crate::auth::nexus::AuthUser::max_concurrent_requests)
        } else {
            config.mdds_concurrent_requests
        };

        let request_semaphore = Arc::new(tokio::sync::Semaphore::new(concurrent));

        tracing::debug!(
            mdds_concurrent_requests = concurrent,
            auto_detected = config.mdds_concurrent_requests == 0,
            "request semaphore initialized"
        );

        let stock_tier = auth_resp.user.as_ref().and_then(|u| u.stock_subscription);
        let options_tier = auth_resp.user.as_ref().and_then(|u| u.options_subscription);

        Ok(Self {
            session_uuid,
            channel,
            config,
            query_info_template,
            request_semaphore,
            stock_tier,
            options_tier,
        })
    }

    /// Return a clone of the pre-built `QueryInfo` template.
    ///
    /// The template is constructed once at connection time, avoiding per-call
    /// String allocations for session UUID, client type, and version.
    #[inline]
    pub(crate) fn query_info(&self) -> proto::QueryInfo {
        self.query_info_template.clone()
    }

    /// Create a new gRPC stub from the shared channel.
    ///
    /// Tonic channels are cheap to clone (internally Arc'd), and stubs take
    /// `&mut self` for each call, so we mint a fresh stub per request to
    /// allow concurrent requests without external `Mutex`.
    pub(crate) fn stub(&self) -> BetaThetaTerminalClient<tonic::transport::Channel> {
        BetaThetaTerminalClient::new(self.channel.clone())
            // MDDS can return large DataTables (e.g. full day of trades).
            // Uses the config-specified max message size.
            .max_decoding_message_size(self.config.mdds_max_message_size)
    }

    /// Return a reference to the underlying config for diagnostics.
    #[must_use]
    pub fn config(&self) -> &DirectConfig {
        &self.config
    }

    /// Return the session UUID string.
    #[must_use]
    pub fn session_uuid(&self) -> &str {
        &self.session_uuid
    }

    /// Stock subscription tier from Nexus auth response (0=Free, 1=Value, 2=Standard, 3=Pro).
    #[must_use]
    pub fn stock_tier(&self) -> Option<i32> {
        self.stock_tier
    }

    /// Options subscription tier from Nexus auth response (0=Free, 1=Value, 2=Standard, 3=Pro).
    #[must_use]
    pub fn options_tier(&self) -> Option<i32> {
        self.options_tier
    }

    // ═══════════════════════════════════════════════════════════════════
    //  Raw query — escape hatch for unwrapped endpoints
    // ═══════════════════════════════════════════════════════════════════

    /// Execute a raw gRPC query and return the merged `DataTable`.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn raw_query<F, Fut>(&self, call: F) -> Result<proto::DataTable, Error>
    where
        F: FnOnce(BetaThetaTerminalClient<tonic::transport::Channel>) -> Fut,
        Fut: std::future::Future<Output = Result<tonic::Streaming<proto::ResponseData>, Error>>,
    {
        let stream = call(self.stub()).await?;
        self.collect_stream(stream).await
    }

    /// Get a `QueryInfo` for use with [`raw_query`](Self::raw_query).
    #[must_use]
    pub fn raw_query_info(&self) -> proto::QueryInfo {
        self.query_info()
    }

    /// Get direct access to the underlying gRPC channel.
    #[must_use]
    pub fn channel(&self) -> &tonic::transport::Channel {
        &self.channel
    }
}
