//! `MarketDataClient` struct, connection lifecycle, and session/transport state.
//!
//! This module owns the MDDS client type: its fields (session UUID,
//! channel, config, request semaphore, subscription tiers), its
//! [`connect`](MarketDataClient::connect) constructor, and the small read-only
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
use crate::FlatFiles;

/// Version string sent in `QueryInfo.terminal_version`.
const TERMINAL_VERSION: &str = env!("CARGO_PKG_VERSION");

tokio::task_local! {
    /// Request-semaphore addresses of EVERY streaming delivery handler
    /// the current task is executing, innermost last — scoped by
    /// [`in_delivery_scope`] around every user handler await
    /// (`deliver_chunk_slices_async` / `deliver_chunk_ticks_async` in
    /// [`super::stream`]).
    ///
    /// Same-client requests issued inside a handler's scope must not
    /// block on that client's request pool — the pull whose handler is
    /// running holds its permits until the handler returns, so the wait
    /// can never end. [`MarketDataClient::acquire_request_permit`]
    /// tests membership here to fail such an acquire fast, and
    /// `auto_plan` reads it to keep a handler-issued pull off the
    /// fan-out path. A set rather than a single slot because handler
    /// scopes NEST across clients (client A's handler awaits a stream
    /// on client B, whose handler calls back into A): the inner scope
    /// must not eclipse the outer client's guard.
    pub(crate) static DELIVERY_HANDLER_SEMAPHORES: Vec<usize>;
}

/// Run `fut` inside `sem_addr`'s delivery-handler scope while
/// RETAINING every enclosing handler scope, so a nested cross-client
/// handler keeps the outer client's reentrancy guard armed. The inner
/// scope unwinds when `fut` resolves, restoring the enclosing set.
pub(crate) async fn in_delivery_scope<F: std::future::Future>(
    sem_addr: usize,
    fut: F,
) -> F::Output {
    let mut addrs = DELIVERY_HANDLER_SEMAPHORES
        .try_with(Vec::clone)
        .unwrap_or_default();
    addrs.push(sem_addr);
    DELIVERY_HANDLER_SEMAPHORES.scope(addrs, fut).await
}

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
/// let eod = client.market_data().stock_history_eod("AAPL", "20240101", "20240301").await?;
/// println!("{} EOD ticks", eod.len());
/// # Ok(())
/// # }
/// ```
pub struct MarketDataClient {
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
    /// Sized like the channel pool: `market_data.max_concurrent_requests`
    /// when set, else the account's tier allowance from the auth
    /// response (see `effective_pool_size`). The server enforces the
    /// account's real concurrent-request allowance; a client that fires
    /// past it sees `ResourceExhausted`, which the retry shell backs off
    /// and replays.
    pub(crate) request_semaphore: Arc<tokio::sync::Semaphore>,
    /// Per-asset subscription tiers captured from the Nexus auth response.
    /// `None` for asset classes the auth response omits or for unknown
    /// wire values (the wire byte is preserved in the structured logs at
    /// connect time but never silently coerced into a tier).
    stock_tier: Option<SubscriptionTier>,
    options_tier: Option<SubscriptionTier>,
    indices_tier: Option<SubscriptionTier>,
    interest_rate_tier: Option<SubscriptionTier>,
    /// Credentials retained for the flat-file surface. Flat files open a
    /// fresh, self-authenticating connection to the legacy MDDS port per
    /// call (independent of the gRPC channel pool above), so `flat_files()`
    /// needs the account credentials directly. Mirrors the unified
    /// [`crate::Client`], which carries the same field for the same reason.
    creds: Credentials,
}

// ── Infrastructure (not generated — these are session/transport methods, not ThetaData endpoints) ──

impl MarketDataClient {
    /// Connect to `ThetaData` servers directly (no JVM terminal needed).
    ///
    /// 1. Authenticates against the Nexus HTTP API to obtain a session UUID.
    /// 2. Opens a gRPC channel (TLS) to the MDDS server.
    ///
    /// The FPSS (real-time streaming) connection is not established here;
    /// this constructor covers only the MDDS market-data-data channel.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn connect(creds: &Credentials, config: DirectConfig) -> Result<Self, Error> {
        // Belt-and-braces: run the config invariants here, the single funnel
        // every market-data connect path routes through (`Client::connect`,
        // `connect_with_api_key`, the builder's `EnvSource::Config`, and this
        // constructor directly). The typed construction paths do not all run
        // `validate`, so a hand-built config could otherwise drive the
        // market-data channel with an out-of-range `max_message_size` (an
        // unbounded decode budget) or a zero port. Mirrors the streaming-side
        // re-check at `StreamingClient::connect`. Idempotent when the caller
        // already validated.
        let config = config.validate()?;
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
        // Auth is driven by the market-data (MDDS) environment only; the
        // streaming environment never affects the auth marker.
        let auth_resp = auth::authenticate_at(
            &config.auth.nexus_url,
            creds,
            config.market_data_environment,
        )
        .await?;
        let session_uuid = auth_resp.session_id.clone();

        tracing::debug!(
            stock_tier = ?auth_resp.user.as_ref().and_then(|u| u.stock_subscription),
            "session established (session_id redacted)"
        );

        // Step 2: Open the gRPC channel pool to MDDS.
        let host = config.market_data.host.clone();
        let port = config.market_data.port;
        tracing::debug!(host = %host, port, tls = config.market_data.tls, "connecting to MDDS");

        let pool_size = effective_pool_size(&config, &auth_resp);
        let channels =
            open_channel_pool(&host, port, config.market_data.tls, pool_size, &config).await?;
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
        // already reflects the override-or-tier-default resolution from
        // `effective_pool_size`; reusing it keeps the semaphore and the
        // channel count strictly coupled.
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
            config.market_data_environment,
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
            creds: creds.clone(),
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

    /// Owned dispatch handle for the bulk-fetch shard machinery.
    ///
    /// Snapshots the `Arc`-backed pieces one top-level request needs
    /// (semaphore, session token, channel pool, retry policy, `QueryInfo`
    /// template) so a spawned shard task dispatches exactly like this
    /// client without borrowing it — spawned futures must be `'static`.
    pub(crate) fn shard_dispatch(&self) -> crate::mdds::shard::ShardDispatch {
        crate::mdds::shard::ShardDispatch::new(
            Arc::clone(&self.request_semaphore),
            self.session.clone(),
            self.channels.clone(),
            self.config.retry,
            // Template with an empty UUID; each attempt stamps the UUID
            // from its own session snapshot.
            self.build_query_info(String::new()),
        )
    }

    /// Size of the gRPC channel pool — the configured
    /// `max_concurrent_requests` budget, which caps how wide a
    /// bulk-fetch shard plan may fan out.
    pub(crate) fn pool_size(&self) -> usize {
        self.channels.size()
    }

    /// Whether the current task is inside one of THIS client's streaming
    /// delivery handlers (the async chunk deliverers scope
    /// [`DELIVERY_HANDLER_SEMAPHORES`] around every user handler await).
    /// Keyed on the request semaphore's address so a handler driving a
    /// different client is not confused with re-entry into this one;
    /// membership across ALL active scopes, so this client's guard stays
    /// armed while a nested handler on another client is executing.
    pub(crate) fn in_delivery_handler(&self) -> bool {
        let addr = Arc::as_ptr(&self.request_semaphore) as usize;
        DELIVERY_HANDLER_SEMAPHORES
            .try_with(|addrs| addrs.contains(&addr))
            .unwrap_or(false)
    }

    /// One tier permit for a top-level request.
    ///
    /// A request awaited inline from inside this client's own streaming
    /// delivery handler must never WAIT here: the active pull holds its
    /// permit(s) until the handler returns, and its sibling bands queue
    /// on the handler lock while holding theirs, so a full pool can
    /// only refill after the handler completes — waiting is a cycle
    /// that never resolves. Inside a handler the acquire is therefore
    /// non-blocking: a free permit (the pool has headroom, e.g.
    /// `shard_concurrency` below the pool size) is taken and the call
    /// proceeds as normal; an exhausted pool fails fast with
    /// [`Error::HandlerReentrancy`] instead of deadlocking the stream.
    /// Outside a handler this is the plain semaphore wait.
    pub(crate) async fn acquire_request_permit(
        &self,
    ) -> Result<tokio::sync::SemaphorePermit<'_>, Error> {
        if self.in_delivery_handler() {
            return match self.request_semaphore.try_acquire() {
                Ok(permit) => Ok(permit),
                Err(tokio::sync::TryAcquireError::NoPermits) => Err(Error::HandlerReentrancy),
                Err(tokio::sync::TryAcquireError::Closed) => {
                    Err(Error::config_internal("request semaphore closed"))
                }
            };
        }
        self.request_semaphore
            .acquire()
            .await
            .map_err(|_| Error::config_internal("request semaphore closed"))
    }

    /// Deterministically tear the market-data client down.
    ///
    /// The market-data client owns only the `Arc`-backed gRPC channel pool —
    /// a set of idle HTTP/2 connections with no worker thread and no streaming
    /// state machine — so there is nothing to signal or join here: the pool's
    /// connections are released when the last client handle is dropped (RAII).
    /// This method exists so the market-data surface matches the unified
    /// [`crate::Client::close`] across every binding (`close()` /
    /// context-manager exit / destructor); the deterministic point of release
    /// is the binding dropping its owning handle on `close`. Idempotent.
    pub fn close(&self) {
        // No streaming dispatcher and no owned worker thread: the channel pool
        // releases on handle drop. Kept as an explicit no-op so the base/
        // market-data lifecycle surface is uniform across bindings rather than
        // silently absent on the market-data-only path.
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
            config.market_data_environment,
            creds.clone(),
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
            creds,
        }
    }
}

// ── Flat files (bulk per-day distribution over the legacy MDDS port) ──
//
// Flat files are pure account-authenticated market data, so they belong on
// the market-data surface. Each call opens a fresh, self-authenticating
// connection to the legacy MDDS port (independent of this client's gRPC
// channel pool) and needs only the credentials and flat-files retry config
// this client already holds. The surface mirrors the unified
// [`crate::Client`] method-for-method — every entry routes through the same
// [`crate::FlatFiles`] view — so a standalone `MarketDataClient` reaches
// flat files exactly like the unified handle and like every language binding.
impl MarketDataClient {
    /// Flat-file namespace view: `client.flat_files().stock_eod("20240115").await?`.
    ///
    /// Cheap borrowed handle; take it per call site rather than storing it.
    #[must_use]
    pub fn flat_files(&self) -> FlatFiles<'_> {
        FlatFiles {
            creds: &self.creds,
            config: &self.config.flatfiles,
        }
    }

    /// Pull a flat-file blob for `(sec_type, req_type, date)`, decode it, and
    /// write the requested `format` to disk. Mirror of
    /// [`crate::Client::flatfile_request`].
    ///
    /// # Errors
    /// Same conditions as [`crate::Client::flatfile_request`].
    pub async fn flatfile_request(
        &self,
        sec_type: crate::flatfiles::SecType,
        req_type: crate::flatfiles::ReqType,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flat_files()
            .to_path(sec_type, req_type, date, output_path, format)
            .await
    }

    /// Pull a flat-file blob and return decoded rows in memory. Mirror of
    /// [`crate::Client::flatfile_request_decoded`].
    ///
    /// # Errors
    /// Same conditions as [`crate::Client::flatfile_request`].
    pub async fn flatfile_request_decoded(
        &self,
        sec_type: crate::flatfiles::SecType,
        req_type: crate::flatfiles::ReqType,
        date: &str,
    ) -> Result<Vec<crate::flatfiles::FlatFileRow>, Error> {
        self.flat_files().request(sec_type, req_type, date).await
    }

    /// Convenience: option open-interest flat file for `date`, written to disk.
    ///
    /// # Errors
    /// Same conditions as [`crate::Client::flatfile_request`].
    pub async fn flatfile_option_open_interest(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::OpenInterest,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: option trade-quote flat file for `date`, written to disk.
    ///
    /// # Errors
    /// Same conditions as [`crate::Client::flatfile_request`].
    pub async fn flatfile_option_trade_quote(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::TradeQuote,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: option end-of-day flat file for `date`, written to disk.
    ///
    /// # Errors
    /// Same conditions as [`crate::Client::flatfile_request`].
    pub async fn flatfile_option_eod(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Option,
            crate::flatfiles::ReqType::Eod,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: stock trade-quote flat file for `date`, written to disk.
    ///
    /// # Errors
    /// Same conditions as [`crate::Client::flatfile_request`].
    pub async fn flatfile_stock_trade_quote(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::TradeQuote,
            date,
            output_path,
            format,
        )
        .await
    }

    /// Convenience: stock end-of-day flat file for `date`, written to disk.
    ///
    /// # Errors
    /// Same conditions as [`crate::Client::flatfile_request`].
    pub async fn flatfile_stock_eod(
        &self,
        date: &str,
        output_path: impl AsRef<std::path::Path>,
        format: crate::flatfiles::FlatFileFormat,
    ) -> Result<std::path::PathBuf, Error> {
        self.flatfile_request(
            crate::flatfiles::SecType::Stock,
            crate::flatfiles::ReqType::Eod,
            date,
            output_path,
            format,
        )
        .await
    }
}

/// Channel-pool sizing — configured override, else the tier default.
///
/// - `max_concurrent_requests = Some(n)` uses `n` verbatim. There is no
///   client-side cap: the server enforces the account's real allowance,
///   so an account boosted above its base tier (e.g. 32 concurrent)
///   sets 32 and actually runs 32 wide. A pool sized past the allowance
///   is safe — the excess requests surface as `ResourceExhausted`,
///   which the retry shell backs off and replays.
/// - `None` (the default) sizes the pool to the account's subscription
///   tier from the auth response — the tier's base allowance (Free 1 /
///   Value 2 / Standard 4 / Pro 8).
/// - No tier on the auth response either (anonymous channel, dev
///   harness) → `DEFAULT_POOL_SIZE`.
///
/// The `.max(1)` floor re-applies `DirectConfig::validate`'s guarantee
/// so a hand-built config that skipped `validate` can never produce an
/// empty pool (`ChannelPool::from_channels` panics on zero channels).
fn effective_pool_size(
    config: &DirectConfig,
    auth_resp: &crate::auth::nexus::AuthResponse,
) -> usize {
    const DEFAULT_POOL_SIZE: usize = 4;
    match config.market_data.max_concurrent_requests {
        Some(n) => usize::try_from(n.max(1)).unwrap_or(usize::MAX),
        None => auth_resp.user.as_ref().map_or(
            DEFAULT_POOL_SIZE,
            crate::auth::nexus::AuthUser::max_concurrent_requests,
        ),
    }
}

/// Open `pool_size` independent gRPC channels and wrap them in a
/// [`ChannelPool`]. Channels are opened sequentially so a transient
/// failure on the first call fails the whole pool fast rather than
/// leaving a half-built pool behind.
///
/// Each channel is built with `config.market_data.max_message_size` so the
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
    let connect_timeout = Duration::from_secs(config.market_data.connect_timeout_secs);
    let max_message_size = config.market_data.max_message_size;
    // HTTP/2 session tuning from the operator's config: flow-control
    // windows (`stream_window_size_kb` / `connection_window_size_kb`, already
    // clamped to [64, 2_097_151] KB by `DirectConfig::validate`) and the
    // keepalive cadence (`keepalive_secs` / `keepalive_timeout_secs`).
    let tuning = ChannelTuning {
        initial_stream_window_size: u32::try_from(
            config
                .market_data
                .stream_window_size_kb
                .saturating_mul(1024),
        )
        .unwrap_or(u32::MAX),
        initial_connection_window_size: u32::try_from(
            config
                .market_data
                .connection_window_size_kb
                .saturating_mul(1024),
        )
        .unwrap_or(u32::MAX),
        keepalive_interval: Duration::from_secs(config.market_data.keepalive_secs.max(1)),
        keepalive_timeout: Duration::from_secs(config.market_data.keepalive_timeout_secs.max(1)),
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
                        config.market_data.connect_timeout_secs
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
                        config.market_data.connect_timeout_secs
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
mod delivery_scope_tests {
    use super::{in_delivery_scope, DELIVERY_HANDLER_SEMAPHORES};

    fn member(addr: usize) -> bool {
        DELIVERY_HANDLER_SEMAPHORES
            .try_with(|addrs| addrs.contains(&addr))
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn nested_delivery_scopes_retain_the_outer_scope() {
        // Handler scopes nest across clients (client A's handler drives
        // a stream on client B, whose handler calls back into A). The
        // inner scope must ADD to the membership set, not eclipse the
        // outer client's guard — and must unwind cleanly.
        let a = 0x1000usize;
        let b = 0x2000usize;
        assert!(!member(a) && !member(b), "no scope armed outside");
        in_delivery_scope(a, async {
            assert!(member(a));
            assert!(!member(b));
            in_delivery_scope(b, async {
                assert!(
                    member(a),
                    "the outer client's scope must stay armed inside the inner"
                );
                assert!(member(b));
            })
            .await;
            assert!(member(a), "outer scope survives the inner unwind");
            assert!(!member(b), "inner scope must unwind");
        })
        .await;
        assert!(!member(a), "outermost scope must unwind");
    }
}

#[cfg(test)]
mod pool_size_tests {
    use super::effective_pool_size;
    use crate::auth::nexus::AuthResponse;
    #[cfg(feature = "__internal")]
    use crate::auth::nexus::AuthUser;
    use crate::config::DirectConfig;

    /// An auth response with no user (anonymous channel, dev harness).
    fn auth_without_user() -> AuthResponse {
        AuthResponse {
            session_id: "session".to_string(),
            user: None,
            session_created: None,
        }
    }

    /// Build an AuthResponse whose user reports the given subscription
    /// wire byte. `AuthUser::max_concurrent_requests` maps that into
    /// the tier's base allowance (`2^tier`).
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
    fn pool_size_defaults_to_tier_allowance() {
        // `max_concurrent_requests = None` (the default) sizes the pool
        // to the tier's base allowance: Free=1, Value=2, Standard=4,
        // Pro=8 (subscription wire bytes 0..=3).
        let config = DirectConfig::production_defaults();
        for (sub_byte, expected) in [(0, 1), (1, 2), (2, 4), (3, 8)] {
            let auth = auth_with_tier(Some(sub_byte));
            assert_eq!(effective_pool_size(&config, &auth), expected);
        }
    }

    #[cfg(feature = "__internal")]
    #[test]
    fn pool_size_override_beats_tier_default() {
        // An explicit value wins over the tier in both directions: a
        // Value account (tier default 2) configured to 32 runs 32 wide.
        let mut config = DirectConfig::production_defaults();
        config.market_data.max_concurrent_requests = Some(32);
        let auth = auth_with_tier(Some(1));
        assert_eq!(effective_pool_size(&config, &auth), 32);
    }

    #[test]
    fn pool_size_tracks_configured_value_past_old_tier_ceiling() {
        // The pool follows an explicit `max_concurrent_requests` with no
        // client-side ceiling: a server-boosted account (e.g. 32
        // concurrent, above the `2^tier` Pro allowance of 8) gets the
        // full configured width.
        let mut config = DirectConfig::production_defaults();
        config.market_data.max_concurrent_requests = Some(32);
        assert_eq!(effective_pool_size(&config, &auth_without_user()), 32);
    }

    #[test]
    fn pool_size_falls_back_to_default_when_no_tier() {
        // No override and no auth user (anonymous channel, dev harness)
        // — the hardcoded `4` is the last resort.
        let config = DirectConfig::production_defaults();
        assert_eq!(effective_pool_size(&config, &auth_without_user()), 4);
    }

    #[test]
    fn pool_size_floors_zero_to_one() {
        // A hand-built config that skipped `validate` must still never
        // produce an empty channel pool.
        let mut config = DirectConfig::production_defaults();
        config.market_data.max_concurrent_requests = Some(0);
        assert_eq!(effective_pool_size(&config, &auth_without_user()), 1);
    }
}

#[cfg(test)]
mod flat_files_surface_tests {
    use super::MarketDataClient;

    /// Compile-time: the standalone market-data client carries the flat-file
    /// surface, mirroring the unified [`crate::Client`]. Referencing the fn
    /// items proves the accessor and decode entry exist with the expected
    /// receiver and return type; nothing is invoked, so no connection opens.
    #[test]
    fn market_data_client_exposes_flat_files() {
        let _accessor: for<'a> fn(&'a MarketDataClient) -> crate::FlatFiles<'a> =
            MarketDataClient::flat_files;
        let _decoded = MarketDataClient::flatfile_request_decoded;
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
        config.market_data.connect_timeout_secs = 1;
        config.market_data.tls = true;

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
