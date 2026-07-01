//! Server configuration for direct `ThetaData` access.
//!
//! # Server topology
//!
//! `ThetaData` runs two server types in their NJ datacenter:
//!
//! ## Historical service (historical data)
//!
//! Historical requests connect to a single endpoint over TLS:
//! ```text
//! mdds-01.thetadata.us:443
//! ```
//!
//! ## Streaming service (real-time streaming)
//!
//! Streaming uses a multi-host config with round-robin failover:
//! ```text
//! FPSS_NJ_HOSTS=nj-a.thetadata.us:20000,nj-a.thetadata.us:20001,
//!               nj-b.thetadata.us:20000,nj-b.thetadata.us:20001
//! ```
//!
//! The connection layer iterates through configured hosts on connection failure.
//!
//! # Layout
//!
//! [`DirectConfig`] is composed of eight nested sub-configs:
//!
//! | Field           | Type                                                |
//! |-----------------|-----------------------------------------------------|
//! | `historical`    | [`HistoricalConfig`] — gRPC host/port/TLS/keepalive       |
//! | `streaming`     | [`StreamingConfig`] — TCP hosts, queue/ring, flush mode  |
//! | `flatfiles`     | [`FlatFilesConfig`] — FLATFILES retry budget        |
//! | `reconnect`     | [`ReconnectConfig`] — wait cadence + policy         |
//! | `retry`         | [`RetryPolicy`] — exponential backoff for historical |
//! | `auth`          | [`AuthConfig`] — Nexus URL + `client_type`          |
//! | `metrics`       | [`MetricsConfig`] — Prometheus exporter port        |
//! | `runtime`       | [`RuntimeConfig`] — tokio worker thread sizing      |

mod auth;
mod env;
mod environment;
mod flatfiles;
mod fpss;
mod mdds;
mod metrics;
mod reconnect;
mod retry;
mod runtime;

use crate::error::Error;

pub use auth::{AuthConfig, DEFAULT_CLIENT_TYPE, DEFAULT_NEXUS_URL};
pub use env::{
    ENV_CLIENT_TYPE, ENV_HISTORICAL_HOST, ENV_HISTORICAL_PORT, ENV_HISTORICAL_TYPE, ENV_NEXUS_URL,
    ENV_STREAMING_HOST, ENV_STREAMING_PORT, ENV_STREAMING_TYPE,
};
pub use environment::{HistoricalEnvironment, StreamingEnvironment};
pub use flatfiles::{bounds as flatfiles_bounds, FlatFilesConfig};
pub use fpss::{
    bounds as streaming_bounds, HostSelectionPolicy, StreamingConfig, StreamingFlushMode,
    StreamingWaitStrategy,
};
pub use mdds::HistoricalConfig;
pub use metrics::MetricsConfig;
pub use reconnect::{
    bounds as reconnect_bounds, ReconnectAttemptClass, ReconnectAttemptLimits, ReconnectConfig,
    ReconnectPolicy, RATE_LIMITED_JITTER_WINDOW,
};
pub use retry::RetryPolicy;
pub use runtime::RuntimeConfig;

pub use crate::backoff::JitterMode;

/// Configuration for connecting to `ThetaData` servers directly.
///
/// Use [`DirectConfig::production()`] for the standard NJ production servers.
///
/// # Layout
///
/// Fields are grouped into eight nested sub-configs ([`HistoricalConfig`],
/// [`StreamingConfig`], [`FlatFilesConfig`], [`ReconnectConfig`], [`RetryPolicy`],
/// [`AuthConfig`], [`MetricsConfig`], [`RuntimeConfig`]). Read accessors on [`DirectConfig`]
/// preserve the field-style naming used by older callers; writes go through
/// the nested struct (e.g. `cfg.streaming.ring_size = N`).
///
/// # Environment variable overrides
///
/// [`DirectConfig::production()`] reads the following environment variables
/// and applies them on top of the hardcoded defaults. Explicit builder
/// setters (`.with_metrics_port(...)` etc.) take precedence over env vars,
/// which in turn take precedence over the hardcoded defaults.
///
/// | Variable | Type | Effect |
/// |---|---|---|
/// | `THETADATA_HISTORICAL_TYPE` | `PROD`/`STAGE` | selects the historical environment + auth marker. Case-insensitive. |
/// | `THETADATA_STREAMING_TYPE` | `PROD`/`DEV` | selects the streaming environment. Case-insensitive. |
/// | `THETADATA_HISTORICAL_HOST` | host | overrides `historical.host` |
/// | `THETADATA_HISTORICAL_PORT` | u16  | overrides `historical.port` |
/// | `THETADATA_NEXUS_URL` | url  | overrides the Nexus auth URL |
/// | `THETADATA_STREAMING_HOST` | host | overrides the primary streaming host |
/// | `THETADATA_STREAMING_PORT` | u16  | overrides the primary streaming port |
/// | `THETADATA_CLIENT_TYPE` | str | overrides `auth.client_type` |
/// | `THETADATA_EMAIL`       | str | credential helper ([`crate::auth`]) |
/// | `THETADATA_PASSWORD`    | str | credential helper ([`crate::auth`]) |
///
/// The historical and streaming channels are selected
/// independently: `THETADATA_HISTORICAL_TYPE` chooses the historical cluster and the
/// auth marker (production or staging), `THETADATA_STREAMING_TYPE` chooses the
/// streaming cluster (production or dev), and neither affects the other. The
/// typed [`DirectConfig::with_historical_environment`] /
/// [`DirectConfig::with_streaming_environment`] are the programmatic
/// equivalents.
///
/// The explicit host/port overrides are recorded first and the environments
/// are selected last: selecting an environment rebuilds that channel's routing
/// and then patches the recorded overrides on top, so an explicit
/// `THETADATA_HISTORICAL_HOST` / `THETADATA_STREAMING_HOST` /
/// `THETADATA_STREAMING_PORT` wins over the environment default while the
/// selected environment's **failover** hosts are preserved. It is independent
/// of the credential — it works the same with either an api-key or an
/// email/password login.
///
/// The "explicit host wins over the environment default" precedence holds
/// for **every** path that sets a host, not only the env-var ordering above.
/// An explicit host survives a later
/// [`DirectConfig::with_historical_environment`] /
/// [`DirectConfig::with_streaming_environment`] / [`DirectConfig::stage`] /
/// [`DirectConfig::dev`], while the channel's selector (and, for the historical
/// channel, the auth marker) still flips. This is modelled by **provenance,
/// not value comparison**: the two host fields are encapsulated (a host can
/// only be set through a tracked setter), and each setter records the host as a
/// typed override that environment selection re-applies on top of the selected
/// cluster.
///
/// The host is set the same way regardless of source — the process env, a
/// `.env`, a config file, or the programmatic
/// [`DirectConfig::set_historical_host`] / [`DirectConfig::set_streaming_hosts`]
/// setters all funnel through the override-recording path, so every source
/// shares one precedence. There is no untracked direct-write path: the fields
/// are crate-private, so environment selection never has to guess whether a
/// field value was a caller edit or a leftover default.
///
/// The streaming override has two tiers — a primary host/port patch (env-var
/// / `.env`) that keeps the environment's failover hosts, and a full explicit
/// host list (the config-file `[streaming] hosts` power-user list, or
/// [`DirectConfig::set_streaming_hosts`]) that wins outright. The full list
/// wins as an explicit choice even when it happens to equal the current
/// environment's own host vector — it is honoured by provenance, never
/// dropped by a value match. A plain `production()` / `stage()` / `dev()` with
/// no override yields that environment's full cluster, unchanged.
///
/// A malformed `THETADATA_HISTORICAL_PORT` / `THETADATA_STREAMING_PORT` (a
/// non-integer) is ignored with a `tracing::warn!`, keeping the current value.
/// An unrecognized environment selector, by contrast, FAILS LOUD: a
/// `THETADATA_HISTORICAL_TYPE` that is not `PROD` / `STAGE` (including the now-removed
/// `DEV`) or a `THETADATA_STREAMING_TYPE` that is not `PROD` / `DEV` (including
/// `STAGE`) is a hard error naming the valid set, never a silent fallback, so a
/// stale or cross-channel selector cannot quietly route to the wrong cluster.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct DirectConfig {
    /// Historical tuning.
    pub historical: HistoricalConfig,
    /// Streaming tuning.
    pub streaming: StreamingConfig,
    /// FLATFILES retry tuning.
    pub flatfiles: FlatFilesConfig,
    /// Reconnection cadence + policy.
    pub reconnect: ReconnectConfig,
    /// Historical retry policy.
    pub retry: RetryPolicy,
    /// Nexus auth endpoint + client type.
    pub auth: AuthConfig,
    /// Prometheus exporter binding.
    pub metrics: MetricsConfig,
    /// Async runtime tuning.
    pub runtime: RuntimeConfig,
    /// Target historical environment (production or staging). Defaults
    /// to [`HistoricalEnvironment::Prod`]; [`DirectConfig::stage`] selects
    /// [`HistoricalEnvironment::Stage`]. Selects the cluster the historical
    /// channel dials and the auth wire marker carried on the auth request
    /// (staging carries the staging marker, production carries none). The
    /// streaming channel is selected independently via
    /// [`Self::streaming_environment`].
    pub historical_environment: HistoricalEnvironment,
    /// Target streaming environment (production or dev). Defaults to
    /// [`StreamingEnvironment::Prod`]; [`DirectConfig::dev`] selects
    /// [`StreamingEnvironment::Dev`]. Selects the cluster the streaming
    /// channel dials and nothing else — it never affects auth, so a dev
    /// session authenticates byte-identically to a production one. The
    /// historical channel is selected independently via
    /// [`Self::historical_environment`].
    pub streaming_environment: StreamingEnvironment,
    /// Explicit historical host override (env-var / `.env` / config-file).
    /// When set, environment selection ([`Self::apply_historical_environment`])
    /// uses this host instead of the environment's default so an explicit host
    /// wins over the environment — the precedence documented on this
    /// struct. Not part of the public field set; reset implicitly by
    /// [`Default`] and the preset constructors.
    historical_host_override: Option<String>,
    /// Explicit primary streaming host override (`THETADATA_STREAMING_HOST`
    /// / `.env`). Patches the primary slot of the selected environment's
    /// streaming hosts in [`Self::apply_streaming_environment`], leaving that
    /// environment's failover hosts in place.
    streaming_primary_host_override: Option<String>,
    /// Explicit primary streaming port override (`THETADATA_STREAMING_PORT`
    /// / `.env`). Patches the primary slot's port independently of the
    /// host, so a port-only override keeps the selected environment's host
    /// cluster and only re-points the primary port.
    streaming_primary_port_override: Option<u16>,
    /// Explicit full streaming host list (the config-file `[streaming]
    /// hosts` power-user list). When set, it wins outright in
    /// [`Self::apply_streaming_environment`]: environment selection does not
    /// touch the streaming hosts at all.
    streaming_hosts_full_override: Option<Vec<(String, u16)>>,
}

impl DirectConfig {
    /// Default Nexus auth URL (matches the upstream production endpoint).
    pub const DEFAULT_NEXUS_URL: &'static str = DEFAULT_NEXUS_URL;

    /// Default `QueryInfo.client_type`.
    pub const DEFAULT_CLIENT_TYPE: &'static str = DEFAULT_CLIENT_TYPE;

    /// Production configuration for `ThetaData`'s NJ datacenter.
    ///
    /// - Historical: `mdds-01.thetadata.us:443` (TLS)
    /// - Streaming: 4 NJ hosts with round-robin failover
    /// - Timeouts: matched to ThetaData's published connection parameters
    ///
    /// Environment variables listed on [`DirectConfig`] are layered on
    /// top of these defaults.
    ///
    /// # Panics
    ///
    /// Panics if an environment variable names an invalid environment selector
    /// (the panic names the offending key, its value, and the valid set), or if
    /// the resulting configuration fails [`Self::validate`]. The hardcoded
    /// defaults are always in range, so the latter fires only when an
    /// environment override pushes a knob out of bounds.
    #[must_use]
    pub fn production() -> Self {
        let mut config = Self::production_defaults();
        // An unrecognized `THETADATA_HISTORICAL_TYPE` / `THETADATA_STREAMING_TYPE` fails
        // loud here rather than silently keeping production: a stale or
        // mistyped selector (including a cross-channel value such as
        // `DEV` on `THETADATA_HISTORICAL_TYPE`) must surface, never quietly route to the wrong
        // cluster. Surface the underlying error verbatim so the panic names the
        // offending key, its value, and the valid set for that channel rather
        // than a generic string an operator cannot act on.
        env::apply_env_overrides(&mut config)
            .unwrap_or_else(|e| panic!("invalid environment selector: {e}"));
        config
            .validate()
            .expect("production defaults are within validated bounds")
    }

    /// Production defaults without env-var overrides. Tests use this to
    /// assert the hardcoded shape in isolation; every caller that wants
    /// env-var precedence should reach for [`DirectConfig::production`].
    #[must_use]
    pub(crate) fn production_defaults() -> Self {
        Self {
            historical: HistoricalConfig::production_defaults(),
            streaming: StreamingConfig::production_defaults(),
            flatfiles: FlatFilesConfig::production_defaults(),
            reconnect: ReconnectConfig::production_defaults(),
            retry: RetryPolicy::default(),
            auth: AuthConfig::production_defaults(),
            metrics: MetricsConfig::default(),
            runtime: RuntimeConfig::default(),
            historical_environment: HistoricalEnvironment::Prod,
            streaming_environment: StreamingEnvironment::Prod,
            // The canonical defaults are the environment's own cluster, not
            // a caller override, so environment selection is free to move
            // them. The override layer records these when an explicit host
            // (or port, or full host list) is supplied.
            historical_host_override: None,
            streaming_primary_host_override: None,
            streaming_primary_port_override: None,
            streaming_hosts_full_override: None,
        }
    }

    /// Route the historical channel at `env`.
    ///
    /// Sets [`Self::historical_environment`] and points the historical host at
    /// that environment's cluster. This is the single place that maps a
    /// [`HistoricalEnvironment`] to its host, so [`Self::production`],
    /// [`Self::stage`], and [`Self::with_historical_environment`] all stay in
    /// agreement without duplicating host literals. The streaming channel is
    /// untouched — [`Self::apply_streaming_environment`] routes it
    /// independently.
    ///
    /// A recorded historical host override (env-var / `.env` / config-file) is
    /// layered on top of the environment's cluster so an explicit host wins
    /// over the environment's default — the precedence documented on the
    /// struct. The environment marker still flips regardless, since it routes
    /// the auth session.
    ///
    /// Only the cluster routing changes; every tuning knob (timeouts,
    /// ring size, retry policy, ...) is left as-is.
    ///
    /// Resolution is purely provenance-driven: the host field is encapsulated,
    /// so the recorded override is the single source of truth for whether a
    /// caller set a host. There is no value comparison and no untracked
    /// direct-write to detect — a host set via [`Self::set_historical_host`]
    /// (or the env-var / `.env` / config-file layers, which funnel through the
    /// same override-recording path) is honoured here, and an unset channel
    /// takes the selected environment's cluster.
    fn apply_historical_environment(&mut self, env: HistoricalEnvironment) {
        self.historical_environment = env;
        // Historical (gRPC) targets the cluster host on the same TLS port;
        // only the host differs between environments. A recorded override
        // wins over the environment default so it survives environment
        // selection. The streaming channel is untouched — the two are
        // independent.
        self.historical.host = self
            .historical_host_override
            .clone()
            .unwrap_or_else(|| env.host().to_string());
    }

    fn apply_streaming_environment(&mut self, env: StreamingEnvironment) {
        self.streaming_environment = env;
        // Streaming: take the selected environment's host set as the base and
        // layer the recorded streaming overrides on top. Rebuilding from `env`
        // every call is what makes an env-var primary override keep tracking
        // the current environment's failover hosts across a later switch. The
        // historical channel and the auth marker are untouched — the streaming
        // environment selects only the streaming hosts.
        let base = env.hosts();
        self.streaming.hosts = self.resolve_streaming_hosts(base);
    }

    /// Apply the recorded streaming host overrides on top of a `base` host
    /// set, returning the resolved host list.
    ///
    /// This is the single accounting path for the streaming override
    /// precedence, shared by [`Self::apply_streaming_environment`] (where
    /// `base` is the selected environment's hosts) and the override setters
    /// (where `base` is the current environment's hosts), so they can never
    /// drift. Resolution
    /// honours the documented most-recent-wins rule across both tiers:
    ///
    /// * A full host list ([`Self::streaming_hosts_full_override`], set via
    ///   [`Self::set_streaming_hosts`] or the config-file `[streaming] hosts`
    ///   power-user list) replaces `base` outright — `base` is discarded
    ///   entirely, even when the list equals it.
    /// * A recorded primary host ([`Self::streaming_primary_host_override`])
    ///   and/or primary port ([`Self::streaming_primary_port_override`]) then
    ///   patch the primary slot of whatever host set survived above (the full
    ///   override's list when set, otherwise `base`), keeping its failover
    ///   hosts.
    ///
    /// Recency is encoded by the setters rather than a stamp: a later full
    /// list clears the primary overrides it supersedes ([`Self::set_streaming_hosts`]),
    /// so a primary override is present here only when it was recorded *after*
    /// the most recent full list. Applying it on top of the full list is
    /// therefore exactly "newest wins" — a primary `THETADATA_STREAMING_HOST`
    /// / `THETADATA_STREAMING_PORT` set after a full override re-points the
    /// primary slot instead of being swallowed by the full list. When only one
    /// tier is set the result is identical to the single-override case.
    fn resolve_streaming_hosts(&self, base: Vec<(String, u16)>) -> Vec<(String, u16)> {
        // The full override replaces the base; a primary override recorded
        // after it still patches the primary slot on top (newest wins).
        let mut hosts = self.streaming_hosts_full_override.clone().unwrap_or(base);
        if let Some(first) = hosts.first_mut() {
            if let Some(host) = &self.streaming_primary_host_override {
                first.0 = host.clone();
            }
            if let Some(port) = self.streaming_primary_port_override {
                first.1 = port;
            }
        }
        hosts
    }

    /// Re-resolve the live host fields from the recorded overrides for the
    /// CURRENT environment.
    ///
    /// Keeps the invariant that the live host fields always equal what the
    /// recorded overrides imply for [`Self::historical_environment`] /
    /// [`Self::streaming_environment`]. The override setters call this the
    /// moment they record a value, so the live field a getter reads is
    /// consistent without waiting for the next environment switch.
    fn reapply_overrides_to_live_fields(&mut self) {
        self.historical.host = self
            .historical_host_override
            .clone()
            .unwrap_or_else(|| self.historical_environment.host().to_string());
        self.streaming.hosts = self.resolve_streaming_hosts(self.streaming_environment.hosts());
    }

    /// Set an explicit historical host.
    ///
    /// This is the only supported way to point the historical (gRPC) channel
    /// at a host. The host is recorded as a tracked override, so it survives a
    /// later `apply_historical_environment` (and therefore
    /// [`Self::with_historical_environment`] / [`Self::stage`] / [`Self::dev`]):
    /// an explicit host wins over the environment's default — the precedence
    /// documented on the struct. The most recent call wins over any earlier
    /// override (an env-var / `.env` / config-file host or a prior `set_*`
    /// call). The live field is updated immediately so a subsequent
    /// [`Self::historical_host`] read reflects the new host.
    pub fn set_historical_host(&mut self, host: impl Into<String>) {
        self.historical_host_override = Some(host.into());
        self.reapply_overrides_to_live_fields();
    }

    /// Set the full explicit streaming host list.
    ///
    /// This is the only supported way to replace the streaming channel's whole
    /// host set. The list is recorded as a full override that wins outright
    /// over environment selection: `apply_streaming_environment` does not touch
    /// the streaming hosts when it is set, and the list is honoured even when
    /// it happens to equal the selected environment's own host vector (it is
    /// kept by provenance, never dropped by a value match). The most recent
    /// call wins, superseding any earlier full or primary streaming override.
    /// The live field is updated immediately so a subsequent
    /// [`Self::streaming_hosts`] read reflects the new list.
    ///
    /// An **empty** list is ignored (logged and skipped): recording an empty
    /// full override would leave the streaming channel with no host to dial and
    /// silently swallow any primary host/port override that environment
    /// selection layers on top. A caller that wants to reset to the
    /// environment's own cluster selects it via
    /// [`Self::with_streaming_environment`] / [`Self::dev`] rather than
    /// passing an empty list.
    pub fn set_streaming_hosts(&mut self, hosts: Vec<(String, u16)>) {
        if hosts.is_empty() {
            tracing::warn!(
                "ignoring empty streaming host list; keeping the current streaming hosts"
            );
            return;
        }
        // A full list is the winning tier, so it supersedes any earlier
        // primary host/port patch: clear the stale primary overrides it
        // replaces before recording the new list.
        self.streaming_primary_host_override = None;
        self.streaming_primary_port_override = None;
        self.streaming_hosts_full_override = Some(hosts);
        self.reapply_overrides_to_live_fields();
    }

    /// Record an explicit historical host override.
    ///
    /// The internal entry point the env-var / `.env` / config-file layers use;
    /// [`Self::set_historical_host`] is the public equivalent. Recording an
    /// override makes the host survive a later
    /// [`Self::apply_historical_environment`], so an explicit host wins over the
    /// environment's default — the precedence documented on the struct. The
    /// override is mirrored onto the live field immediately so a getter reflects
    /// it before the next switch.
    pub(crate) fn set_historical_host_override(&mut self, host: String) {
        self.historical_host_override = Some(host);
        self.reapply_overrides_to_live_fields();
    }

    /// Record an explicit primary streaming host override
    /// (`THETADATA_STREAMING_HOST` / `.env`).
    ///
    /// Patches the primary slot of whatever environment is selected when
    /// [`Self::apply_streaming_environment`] next runs, keeping that
    /// environment's failover hosts. Independent of the port override. Mirrored
    /// onto the live [`Self::streaming`] field immediately so the field always
    /// reflects the recorded override.
    pub(crate) fn set_streaming_primary_host_override(&mut self, host: String) {
        self.streaming_primary_host_override = Some(host);
        self.reapply_overrides_to_live_fields();
    }

    /// Record an explicit primary streaming port override
    /// (`THETADATA_STREAMING_PORT` / `.env`).
    ///
    /// Patches the primary slot's port of whatever environment is selected
    /// when [`Self::apply_streaming_environment`] next runs, independently of
    /// the host — a port-only override keeps the environment's host cluster and
    /// only re-points the primary port. Mirrored onto the live
    /// [`Self::streaming`] field immediately so the field always reflects the
    /// recorded override.
    pub(crate) fn set_streaming_primary_port_override(&mut self, port: u16) {
        self.streaming_primary_port_override = Some(port);
        self.reapply_overrides_to_live_fields();
    }

    /// Record an explicit full streaming host list (the config-file
    /// `[streaming] hosts` power-user list).
    ///
    /// The internal entry point the config-file loader uses;
    /// [`Self::set_streaming_hosts`] is the public equivalent and carries the
    /// shared recording logic. When recorded, the list wins outright in
    /// [`Self::apply_streaming_environment`]: environment selection does not
    /// touch the streaming hosts at all. Only the config-file loader supplies a
    /// full host
    /// list, so this setter is gated on that feature; the field stays `None`
    /// (and the override is inert) without it.
    #[cfg(feature = "config-file")]
    pub(crate) fn set_streaming_hosts_full_override(&mut self, hosts: Vec<(String, u16)>) {
        self.set_streaming_hosts(hosts);
    }

    /// Select the historical environment, returning the updated config.
    ///
    /// The programmatic equivalent of the `THETADATA_HISTORICAL_TYPE`
    /// (`PROD` / `STAGE`) env var: it points the historical host and the auth
    /// wire marker at the chosen environment. The streaming channel is
    /// unaffected — select it independently with
    /// [`Self::with_streaming_environment`].
    ///
    /// `DirectConfig::production().with_historical_environment(HistoricalEnvironment::Stage)`
    /// is the historical half of [`DirectConfig::stage`].
    ///
    /// Works the same with either credential form (api-key or
    /// email/password) — the environment is independent of the credential.
    ///
    /// # Panics
    ///
    /// Panics if the resulting configuration fails [`Self::validate`].
    /// Only the cluster routing changes, so this fires only when a tuning
    /// knob was already out of range before the call.
    #[must_use]
    pub fn with_historical_environment(mut self, env: HistoricalEnvironment) -> Self {
        self.apply_historical_environment(env);
        self.validate()
            .expect("environment switch leaves tuning knobs unchanged")
    }

    /// Select the streaming environment, returning the updated config.
    ///
    /// The programmatic equivalent of the `THETADATA_STREAMING_TYPE`
    /// (`PROD` / `DEV`) env var: it points the streaming hosts at the chosen
    /// environment and nothing else. Auth and the historical channel are
    /// unaffected — a dev session authenticates byte-identically to a
    /// production one. Select the historical channel independently with
    /// [`Self::with_historical_environment`].
    ///
    /// `DirectConfig::production().with_streaming_environment(StreamingEnvironment::Dev)`
    /// is the streaming half of [`DirectConfig::dev`].
    ///
    /// # Panics
    ///
    /// Panics if the resulting configuration fails [`Self::validate`].
    /// Only the cluster routing changes, so this fires only when a tuning
    /// knob was already out of range before the call.
    #[must_use]
    pub fn with_streaming_environment(mut self, env: StreamingEnvironment) -> Self {
        self.apply_streaming_environment(env);
        self.validate()
            .expect("environment switch leaves tuning knobs unchanged")
    }

    /// Source the target environment from a `.env`-format file.
    ///
    /// Starts from the hardcoded production defaults and applies the cluster
    /// keys carried by the file:
    ///
    /// - `THETADATA_HISTORICAL_TYPE` (`PROD` / `STAGE`, case-insensitive) selects the
    ///   historical environment via [`HistoricalEnvironment::parse`], pointing
    ///   the historical host and the auth marker at the chosen cluster — the
    ///   file-sourced equivalent of the [`THETADATA_HISTORICAL_TYPE`](Self::production)
    ///   env var and of [`Self::with_historical_environment`].
    /// - `THETADATA_STREAMING_TYPE` (`PROD` / `DEV`, case-insensitive) selects the
    ///   streaming environment via [`StreamingEnvironment::parse`], pointing the
    ///   streaming hosts at the chosen cluster — the file-sourced equivalent of
    ///   the [`THETADATA_STREAMING_TYPE`](Self::production) env var and of
    ///   [`Self::with_streaming_environment`]. The two channels are selected
    ///   independently and neither affects the other.
    /// - `THETADATA_HISTORICAL_HOST` / `THETADATA_STREAMING_HOST`, when
    ///   present, override the historical and primary streaming hosts. They
    ///   are layered on top of the environment selection, so an explicit host
    ///   wins over the environment default — the same precedence as the
    ///   process-env path.
    ///
    /// The file uses the common `.env` grammar (one `KEY=VALUE` per line, with
    /// an optional `export ` prefix, `#` comment lines, blank lines, and
    /// optional matching quotes). It is the **same** file format and the same
    /// keys that [`crate::auth::Credentials::from_dotenv`] reads for the
    /// credential, so a single `.env` can carry both `THETADATA_API_KEY` and
    /// `THETADATA_HISTORICAL_TYPE`: the credential reader picks up the secret keys
    /// and this reader picks up the cluster keys.
    ///
    /// An empty selector reads as unset (the production default is kept), and a
    /// file that carries only a credential key (for example just
    /// `THETADATA_API_KEY`) yields the production configuration without error.
    /// An *unrecognized* `THETADATA_HISTORICAL_TYPE` / `THETADATA_STREAMING_TYPE` (including
    /// a cross-channel value such as `DEV` on `THETADATA_HISTORICAL_TYPE` or `STAGE` on
    /// `THETADATA_STREAMING_TYPE`) is a returned error naming the valid set, never a
    /// silent fallback.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if the file cannot be read, if a selector names
    /// an invalid environment, or whatever [`Self::validate`] returns for the
    /// resulting configuration.
    pub fn from_dotenv(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        // Start from the hardcoded production defaults, NOT from
        // [`Self::production`]: a file-sourced config must be deterministic
        // from its defaults plus the file, so the ambient process env never
        // leaks in. A `.env` that selects `PROD` must yield the prod cluster
        // even when the shell has `THETADATA_HISTORICAL_TYPE=STAGE` (or a stray
        // `THETADATA_HISTORICAL_HOST`) left over. `with_dotenv` layers only
        // the parsed file pairs via `apply_dotenv_overrides`, which reads the
        // file, never the process env.
        Self::production_defaults().with_dotenv(path)
    }

    /// Apply a `.env` file's environment selection and host overrides onto an
    /// existing configuration, returning the updated config.
    ///
    /// This is the builder companion to [`Self::from_dotenv`]: where
    /// `from_dotenv` starts from [`Self::production`], `with_dotenv` layers the
    /// same `.env`-sourced cluster overrides on top of the receiver, leaving
    /// every tuning knob the caller already set in place. The keys, precedence,
    /// and lenient handling are identical to [`Self::from_dotenv`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if the file cannot be read, or whatever
    /// [`Self::validate`] returns for the resulting configuration.
    pub fn with_dotenv(mut self, path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        // Wrap the buffer in `Zeroizing`: the same `.env` may carry
        // `THETADATA_API_KEY` (read by `Credentials::from_dotenv`), so the
        // on-disk secret bytes are wiped on drop even though this reader only
        // consumes the non-secret cluster keys.
        let contents =
            zeroize::Zeroizing::new(std::fs::read_to_string(path).map_err(|e| Error::Config {
                kind: crate::error::ConfigErrorKind::Io(format!(
                    "failed to read .env file {}: {}",
                    path.display(),
                    e
                )),
                message: ".env file unreadable".to_string(),
                source: Some(Box::new(e)),
            })?);
        let pairs = crate::auth::dotenv::parse(&contents);
        // An unrecognized `THETADATA_HISTORICAL_TYPE` / `THETADATA_STREAMING_TYPE` in the
        // file is a returned error naming the valid set, not a silent fallback.
        env::apply_dotenv_overrides(&mut self, &pairs)?;
        self.validate()
    }

    /// Dev streaming configuration.
    ///
    /// Connects to `ThetaData`'s dev streaming servers (port 20200) which replay
    /// a random historical trading day in an infinite loop at maximum speed.
    /// Designed for development and testing when markets are closed.
    ///
    /// Historical data still uses production servers -- there is no dev historical.
    ///
    /// Dev selects the streaming dev-replay cluster ONLY: the streaming
    /// channel dials the dev replay hosts ([`StreamingEnvironment::Dev`]) while
    /// the historical channel and the auth wire marker stay on production —
    /// there is no dev historical, and the two clients are selected
    /// independently. A dev session authenticates byte-identically to a
    /// production one. Because the environment fully determines the cluster, a
    /// later override on a `dev()` config (an env-var / `.env` host,
    /// [`Self::set_streaming_hosts`], a config-file host list) patches the dev
    /// cluster instead of dropping it back to production — the override layer
    /// rebuilds from the dev base, not a stale prod base.
    ///
    /// Source: <https://docs.thetadata.us/Streaming/Getting-Started.html>
    ///
    /// Note: dev server replays data at max speed, so queue and ring sizes
    /// match production to avoid drops. Some contracts may not exist on
    /// the replayed day.
    ///
    /// # Panics
    ///
    /// Panics if the preset fails [`Self::validate`] — only reachable
    /// when an environment override pushes a knob out of bounds, since
    /// the preset's own values are in range.
    #[must_use]
    pub fn dev() -> Self {
        let mut config = Self::production();
        // Select streaming-dev only; historical and auth stay on production.
        // The override layer (`reapply_overrides_to_live_fields`) rebuilds from
        // the dev base, so a later streaming override on a dev config patches
        // the dev cluster instead of silently reverting to production. A
        // recorded `THETADATA_STREAMING_HOST` / `_PORT` override from
        // `production()` above is carried through and patches the dev primary
        // slot, exactly as it would for any streaming environment.
        config.apply_streaming_environment(StreamingEnvironment::Dev);
        config
            .validate()
            .expect("dev preset is within validated bounds")
    }

    /// Streaming hosts for the dev preset (test-only accessor).
    ///
    /// A thin delegate to [`StreamingEnvironment::Dev`]'s hosts — the single
    /// source of truth lives on the streaming environment, so production code
    /// reads the dev cluster through [`StreamingEnvironment::hosts`]. Retained
    /// so the streaming allowlist coverage test and the config regression tests can
    /// name the dev host set directly; a new dev host added to
    /// [`StreamingEnvironment::hosts`] flows through here so it can never drift
    /// out of the streaming hostname allowlist.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn dev_streaming_hosts() -> Vec<(String, u16)> {
        StreamingEnvironment::Dev.hosts()
    }

    /// Staging environment configuration.
    ///
    /// Selects `ThetaData`'s historical staging cluster ONLY:
    ///
    /// - Historical environment: [`HistoricalEnvironment::Stage`], so the
    ///   historical channel dials `mdds-stage.thetadata.us:443` (TLS) and the
    ///   auth request carries the staging marker so the server routes the
    ///   session to staging.
    /// - Streaming stays on production. There is no streaming staging cluster,
    ///   and the two clients are selected independently, so `stage()` leaves
    ///   the streaming channel on the production hosts.
    ///
    /// Staging is used to validate against pre-release server changes;
    /// it is less stable than production and subject to frequent reboots.
    ///
    /// # Panics
    ///
    /// Panics if the preset fails [`Self::validate`] — only reachable
    /// when an environment override pushes a knob out of bounds, since
    /// the preset's own values are in range.
    #[must_use]
    pub fn stage() -> Self {
        let mut config = Self::production();
        // Select historical-staging (host + auth marker) only; streaming stays
        // on production since streaming has no staging cluster.
        config.apply_historical_environment(HistoricalEnvironment::Stage);
        config
            .validate()
            .expect("stage preset is within validated bounds")
    }

    /// Validate configuration values and reject out-of-range tuning knobs.
    ///
    /// Returns the configuration with historical HTTP/2 window sizes clamped
    /// into `[64, 1024]` KB on success. Returns
    /// [`Error::Config`] when any wired streaming
    /// knob (`timeout_ms`, `connect_timeout_ms`, `ping_interval_ms`)
    /// falls outside its documented range — silent rounding would
    /// rewrite the caller's stated tuning under their feet, so an
    /// invalid value is reported up front instead.
    ///
    /// Called automatically by [`production()`](Self::production),
    /// [`dev()`](Self::dev), and [`stage()`](Self::stage). Also useful
    /// after loading from a TOML file or modifying fields
    /// programmatically.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when an streaming
    /// timing knob is out of range.
    pub fn validate(mut self) -> Result<Self, Error> {
        // u64 → i64: every bound fits comfortably under i64::MAX (max
        // bound is 300_000 ms). `saturating_cast` would be overkill;
        // a checked `try_from` documents the invariant.
        let to_i64 = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        if !streaming_bounds::TIMEOUT_MS.contains(&self.streaming.timeout_ms) {
            return Err(Error::config_out_of_range(
                "streaming.timeout_ms",
                to_i64(self.streaming.timeout_ms),
                to_i64(*streaming_bounds::TIMEOUT_MS.start()),
                to_i64(*streaming_bounds::TIMEOUT_MS.end()),
            ));
        }
        if !streaming_bounds::CONNECT_TIMEOUT_MS.contains(&self.streaming.connect_timeout_ms) {
            return Err(Error::config_out_of_range(
                "streaming.connect_timeout_ms",
                to_i64(self.streaming.connect_timeout_ms),
                to_i64(*streaming_bounds::CONNECT_TIMEOUT_MS.start()),
                to_i64(*streaming_bounds::CONNECT_TIMEOUT_MS.end()),
            ));
        }
        if !streaming_bounds::PING_INTERVAL_MS.contains(&self.streaming.ping_interval_ms) {
            return Err(Error::config_out_of_range(
                "streaming.ping_interval_ms",
                to_i64(self.streaming.ping_interval_ms),
                to_i64(*streaming_bounds::PING_INTERVAL_MS.start()),
                to_i64(*streaming_bounds::PING_INTERVAL_MS.end()),
            ));
        }
        if !streaming_bounds::IO_READ_SLICE_MS.contains(&self.streaming.io_read_slice_ms) {
            return Err(Error::config_out_of_range(
                "streaming.io_read_slice_ms",
                to_i64(self.streaming.io_read_slice_ms),
                to_i64(*streaming_bounds::IO_READ_SLICE_MS.start()),
                to_i64(*streaming_bounds::IO_READ_SLICE_MS.end()),
            ));
        }
        // `data_watchdog_ms` is a wall-clock backstop above the read
        // timeout: `0` disables it, and any enabled value must sit inside
        // its band and at or above `timeout_ms` so the backstop cannot
        // fire before the read timeout it is meant to backstop.
        if self.streaming.data_watchdog_ms != 0 {
            if !streaming_bounds::DATA_WATCHDOG_MS.contains(&self.streaming.data_watchdog_ms) {
                return Err(Error::config_out_of_range(
                    "streaming.data_watchdog_ms",
                    to_i64(self.streaming.data_watchdog_ms),
                    to_i64(*streaming_bounds::DATA_WATCHDOG_MS.start()),
                    to_i64(*streaming_bounds::DATA_WATCHDOG_MS.end()),
                ));
            }
            if self.streaming.data_watchdog_ms < self.streaming.timeout_ms {
                return Err(Error::config_invalid(
                    "streaming.data_watchdog_ms",
                    format!(
                        "data_watchdog_ms ({}) must be 0 (disabled) or >= timeout_ms ({})",
                        self.streaming.data_watchdog_ms, self.streaming.timeout_ms
                    ),
                ));
            }
        }
        if !streaming_bounds::KEEPALIVE_IDLE_SECS.contains(&self.streaming.keepalive_idle_secs) {
            return Err(Error::config_out_of_range(
                "streaming.keepalive_idle_secs",
                to_i64(self.streaming.keepalive_idle_secs),
                to_i64(*streaming_bounds::KEEPALIVE_IDLE_SECS.start()),
                to_i64(*streaming_bounds::KEEPALIVE_IDLE_SECS.end()),
            ));
        }
        if !streaming_bounds::KEEPALIVE_INTERVAL_SECS
            .contains(&self.streaming.keepalive_interval_secs)
        {
            return Err(Error::config_out_of_range(
                "streaming.keepalive_interval_secs",
                to_i64(self.streaming.keepalive_interval_secs),
                to_i64(*streaming_bounds::KEEPALIVE_INTERVAL_SECS.start()),
                to_i64(*streaming_bounds::KEEPALIVE_INTERVAL_SECS.end()),
            ));
        }
        if !streaming_bounds::KEEPALIVE_RETRIES.contains(&self.streaming.keepalive_retries) {
            return Err(Error::config_out_of_range(
                "streaming.keepalive_retries",
                i64::from(self.streaming.keepalive_retries),
                i64::from(*streaming_bounds::KEEPALIVE_RETRIES.start()),
                i64::from(*streaming_bounds::KEEPALIVE_RETRIES.end()),
            ));
        }
        if !streaming_bounds::WAIT_SPIN_ITERS.contains(&self.streaming.wait_spin_iters) {
            return Err(Error::config_out_of_range(
                "streaming.wait_spin_iters",
                i64::from(self.streaming.wait_spin_iters),
                i64::from(*streaming_bounds::WAIT_SPIN_ITERS.start()),
                i64::from(*streaming_bounds::WAIT_SPIN_ITERS.end()),
            ));
        }
        if !streaming_bounds::WAIT_YIELD_ITERS.contains(&self.streaming.wait_yield_iters) {
            return Err(Error::config_out_of_range(
                "streaming.wait_yield_iters",
                i64::from(self.streaming.wait_yield_iters),
                i64::from(*streaming_bounds::WAIT_YIELD_ITERS.start()),
                i64::from(*streaming_bounds::WAIT_YIELD_ITERS.end()),
            ));
        }
        if !streaming_bounds::WAIT_PARK_US.contains(&self.streaming.wait_park_us) {
            return Err(Error::config_out_of_range(
                "streaming.wait_park_us",
                to_i64(self.streaming.wait_park_us),
                to_i64(*streaming_bounds::WAIT_PARK_US.start()),
                to_i64(*streaming_bounds::WAIT_PARK_US.end()),
            ));
        }
        if self.reconnect.replay_burst_size == 0 {
            return Err(Error::config_invalid(
                "reconnect.replay_burst_size",
                "replay_burst_size must be at least 1".to_string(),
            ));
        }
        // Generic-transient ladder: both the base delay and its exponential
        // ceiling must be positive and in band. A `0` base (or a `0` cap that
        // drags the base down through the ordering check below) would yield a
        // 0 ms reconnect busy-loop on every generic transient, so both ends are
        // band-checked the same way the sibling cadences are.
        if !reconnect_bounds::WAIT_MS.contains(&self.reconnect.wait_ms) {
            return Err(Error::config_out_of_range(
                "reconnect.wait_ms",
                to_i64(self.reconnect.wait_ms),
                to_i64(*reconnect_bounds::WAIT_MS.start()),
                to_i64(*reconnect_bounds::WAIT_MS.end()),
            ));
        }
        if !reconnect_bounds::WAIT_MS.contains(&self.reconnect.wait_max_ms) {
            return Err(Error::config_out_of_range(
                "reconnect.wait_max_ms",
                to_i64(self.reconnect.wait_max_ms),
                to_i64(*reconnect_bounds::WAIT_MS.start()),
                to_i64(*reconnect_bounds::WAIT_MS.end()),
            ));
        }
        if self.reconnect.wait_max_ms < self.reconnect.wait_ms {
            return Err(Error::config_invalid(
                "reconnect.wait_max_ms",
                format!(
                    "wait_max_ms ({}) must be >= wait_ms ({})",
                    self.reconnect.wait_max_ms, self.reconnect.wait_ms
                ),
            ));
        }
        // Reconnect cadence trio: the rate-limited and server-restart floors
        // must be positive delays, and the replay pace must stay within band.
        // A `0` cadence would busy-loop the reconnect driver; bands are
        // band-checked up front the same way the streaming knobs are.
        if !reconnect_bounds::WAIT_RATE_LIMITED_MS.contains(&self.reconnect.wait_rate_limited_ms) {
            return Err(Error::config_out_of_range(
                "reconnect.wait_rate_limited_ms",
                to_i64(self.reconnect.wait_rate_limited_ms),
                to_i64(*reconnect_bounds::WAIT_RATE_LIMITED_MS.start()),
                to_i64(*reconnect_bounds::WAIT_RATE_LIMITED_MS.end()),
            ));
        }
        if !reconnect_bounds::WAIT_SERVER_RESTART_MS
            .contains(&self.reconnect.wait_server_restart_ms)
        {
            return Err(Error::config_out_of_range(
                "reconnect.wait_server_restart_ms",
                to_i64(self.reconnect.wait_server_restart_ms),
                to_i64(*reconnect_bounds::WAIT_SERVER_RESTART_MS.start()),
                to_i64(*reconnect_bounds::WAIT_SERVER_RESTART_MS.end()),
            ));
        }
        if !reconnect_bounds::REPLAY_PACE_MS.contains(&self.reconnect.replay_pace_ms) {
            return Err(Error::config_out_of_range(
                "reconnect.replay_pace_ms",
                to_i64(self.reconnect.replay_pace_ms),
                to_i64(*reconnect_bounds::REPLAY_PACE_MS.start()),
                to_i64(*reconnect_bounds::REPLAY_PACE_MS.end()),
            ));
        }
        // Auto-reconnect per-class attempt budgets: every class budget must
        // be at least one attempt so the driver can make forward progress and
        // within band so a typo cannot spin effectively forever. Only the
        // `Auto` policy carries budgets; `Manual` / `Custom` have none to
        // check.
        if let ReconnectPolicy::Auto(limits) = &self.reconnect.policy {
            for (field, value) in [
                ("reconnect.max_attempts", limits.max_attempts),
                (
                    "reconnect.max_rate_limited_attempts",
                    limits.max_rate_limited_attempts,
                ),
                (
                    "reconnect.max_server_restart_attempts",
                    limits.max_server_restart_attempts,
                ),
            ] {
                if !reconnect_bounds::ATTEMPT_BUDGET.contains(&value) {
                    return Err(Error::config_out_of_range(
                        field,
                        i64::from(value),
                        i64::from(*reconnect_bounds::ATTEMPT_BUDGET.start()),
                        i64::from(*reconnect_bounds::ATTEMPT_BUDGET.end()),
                    ));
                }
            }
        }
        // Historical retry policy: the backoff ceiling cannot sit below the
        // initial delay (mirrors the flatfiles `max_backoff >= initial_backoff`
        // invariant), or the exponential ladder would start above its own cap.
        if self.retry.max_delay < self.retry.initial_delay {
            return Err(Error::config_invalid(
                "retry.max_delay",
                format!(
                    "max_delay ({:?}) must be >= initial_delay ({:?})",
                    self.retry.max_delay, self.retry.initial_delay
                ),
            ));
        }
        // The streaming channel needs at least one host to dial. An empty list
        // is reachable from a full override of `vec![]` (the public
        // `set_streaming_hosts` setter); the config-file `[streaming] hosts`
        // path already rejects empty in `parse_streaming_hosts`. Reject it here
        // too so every construction path fails fast at build time with a clear
        // field error rather than at the connect attempt with a generic "no
        // servers configured".
        if self.streaming.hosts.is_empty() {
            return Err(Error::config_missing("streaming.hosts"));
        }
        // Validate ring_size eagerly so a bad config fails fast rather
        // than waiting for the connect attempt. Re-validation happens
        // at `StreamingClient::connect` for callers that bypass `validate`.
        if let Err(e) = crate::fpss::ring::check_ring_size(self.streaming.ring_size) {
            return Err(Error::config_invalid("streaming.ring_size", e.to_string()));
        }
        self.historical.window_size_kb = self.historical.window_size_kb.clamp(64, 1_024);
        self.historical.connection_window_size_kb =
            self.historical.connection_window_size_kb.clamp(64, 1_024);
        if !flatfiles_bounds::MAX_ATTEMPTS.contains(&self.flatfiles.max_attempts) {
            return Err(Error::config_out_of_range(
                "flatfiles.max_attempts",
                i64::from(self.flatfiles.max_attempts),
                i64::from(*flatfiles_bounds::MAX_ATTEMPTS.start()),
                i64::from(*flatfiles_bounds::MAX_ATTEMPTS.end()),
            ));
        }
        if self.flatfiles.max_backoff < self.flatfiles.initial_backoff {
            return Err(Error::config_invalid(
                "flatfiles.max_backoff",
                format!(
                    "max_backoff ({:?}) must be >= initial_backoff ({:?})",
                    self.flatfiles.max_backoff, self.flatfiles.initial_backoff
                ),
            ));
        }
        // Flat-file HTTP timeouts must be positive: a `0` connect or read
        // timeout aborts every request instantly. Band-checked up front like
        // the streaming timeouts.
        if !flatfiles_bounds::CONNECT_TIMEOUT_SECS.contains(&self.flatfiles.connect_timeout_secs) {
            return Err(Error::config_out_of_range(
                "flatfiles.connect_timeout_secs",
                to_i64(self.flatfiles.connect_timeout_secs),
                to_i64(*flatfiles_bounds::CONNECT_TIMEOUT_SECS.start()),
                to_i64(*flatfiles_bounds::CONNECT_TIMEOUT_SECS.end()),
            ));
        }
        if !flatfiles_bounds::READ_TIMEOUT_SECS.contains(&self.flatfiles.read_timeout_secs) {
            return Err(Error::config_out_of_range(
                "flatfiles.read_timeout_secs",
                to_i64(self.flatfiles.read_timeout_secs),
                to_i64(*flatfiles_bounds::READ_TIMEOUT_SECS.start()),
                to_i64(*flatfiles_bounds::READ_TIMEOUT_SECS.end()),
            ));
        }
        Ok(self)
    }

    /// Build the historical endpoint URI.
    ///
    /// Returns the gRPC base URI for the historical service.
    #[must_use]
    pub fn historical_uri(&self) -> String {
        let scheme = if self.historical.tls { "https" } else { "http" };
        format!(
            "{}://{}:{}",
            scheme, self.historical.host, self.historical.port
        )
    }

    /// Set the port the Prometheus exporter should bind to when the
    /// `metrics-prometheus` cargo feature is enabled. The exporter
    /// exposes `/metrics` over HTTP on `0.0.0.0:<port>`.
    #[must_use]
    pub fn with_metrics_port(mut self, port: u16) -> Self {
        self.metrics.port = Some(port);
        self
    }

    /// Override the retry policy for transient gRPC errors.
    #[must_use]
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry = policy;
        self
    }

    /// Override the Nexus auth URL. Intended for staging deployments —
    /// production should use [`ENV_NEXUS_URL`] or the default.
    #[must_use]
    pub fn with_nexus_url(mut self, url: impl Into<String>) -> Self {
        self.auth.nexus_url = url.into();
        self
    }

    /// Override `QueryInfo.client_type`. Appears in server-side logs
    /// and dashboards; useful for tagging a deployment fleet.
    #[must_use]
    pub fn with_client_type(mut self, client_type: impl Into<String>) -> Self {
        self.auth.client_type = client_type.into();
        self
    }

    /// Parse streaming hosts from a comma-separated `host:port,host:port,...` string.
    ///
    /// This is the format used in `config_0.properties` for `FPSS_NJ_HOSTS`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when an entry lacks a `host:port` split,
    /// when a port does not parse as a `u16`, or when the input yields no
    /// hosts at all.
    pub fn parse_streaming_hosts(hosts_str: &str) -> Result<Vec<(String, u16)>, Error> {
        let mut result = Vec::new();

        for entry in hosts_str.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }

            let (host, port_str) = entry.rsplit_once(':').ok_or_else(|| {
                Error::config_invalid(
                    "streaming.hosts",
                    format!("invalid host:port entry: '{entry}'"),
                )
            })?;

            let port: u16 = port_str.parse().map_err(|e| {
                Error::config_invalid("streaming.hosts", format!("invalid port in '{entry}': {e}"))
            })?;

            result.push((host.to_string(), port));
        }

        if result.is_empty() {
            return Err(Error::config_missing("streaming.hosts"));
        }

        Ok(result)
    }
}

// ── Read accessors ───────────────────────────────────────────────────────
impl DirectConfig {
    /// Historical hostname.
    #[must_use]
    pub fn historical_host(&self) -> &str {
        &self.historical.host
    }

    /// Streaming host list.
    #[must_use]
    pub fn streaming_hosts(&self) -> &[(String, u16)] {
        &self.streaming.hosts
    }

    /// Target historical environment (production or staging).
    #[must_use]
    pub fn historical_environment(&self) -> HistoricalEnvironment {
        self.historical_environment
    }

    /// Target streaming environment (production or dev).
    #[must_use]
    pub fn streaming_environment(&self) -> StreamingEnvironment {
        self.streaming_environment
    }
}

// ── Config file loading (behind `config-file` feature) ──────────────────────

#[cfg(feature = "config-file")]
mod config_file {
    use super::{
        DirectConfig, ReconnectAttemptLimits, ReconnectPolicy, RetryPolicy, StreamingFlushMode,
        StreamingWaitStrategy,
    };
    use crate::error::Error;
    use serde::Deserialize;

    /// TOML-level representation of the config file.
    ///
    /// An unknown key or section is rejected (`#[serde(deny_unknown_fields)]`)
    /// so a misspelled knob surfaces as a load error instead of silently
    /// running the default. Missing sections fall back to production
    /// defaults (`#[serde(default)]` on each section).
    #[derive(Debug, Default, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct ConfigFile {
        historical: MddsSection,
        streaming: FpssSection,
        grpc: GrpcSection,
    }

    #[derive(Debug, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct MddsSection {
        /// Historical host. `None` (key absent) leaves the environment's
        /// default host in force and records no override, so a later
        /// environment switch still re-points it; an explicit value is
        /// recorded as a host override that survives environment selection.
        host: Option<String>,
        port: u16,
        tls: bool,
        keepalive_time_secs: u64,
        keepalive_timeout_secs: u64,
        max_message_size: usize,
    }

    impl Default for MddsSection {
        fn default() -> Self {
            let prod = DirectConfig::production_defaults();
            Self {
                // Absent by default so the environment's host stays the
                // single source of truth unless the operator sets it.
                host: None,
                port: prod.historical.port,
                tls: prod.historical.tls,
                keepalive_time_secs: prod.historical.keepalive_secs,
                keepalive_timeout_secs: prod.historical.keepalive_timeout_secs,
                max_message_size: prod.historical.max_message_size,
            }
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct FpssSection {
        /// Hosts as `["host:port", ...]` array or `"host:port,host:port"`
        /// string. `None` (key absent) leaves the environment's default host
        /// set in force and records no override; an explicit list is recorded
        /// as a full override that wins outright over environment selection.
        hosts: Option<FpssHosts>,
        connect_timeout: u64,
        read_timeout: u64,
        ping_interval: u64,
        reconnect_wait: u64,
        reconnect_wait_rate_limited: u64,
        ring_size: usize,
        flush_mode: String,
        wait_strategy: String,
        wait_spin_iters: u32,
        wait_yield_iters: u32,
        wait_park_us: u64,
        /// CPU core to pin the streaming consumer thread to. A negative
        /// value (the default `-1`) means "unpinned" — the `Option::None`
        /// form in `[streaming]` TOML, where serde cannot express a bare
        /// optional cleanly under `#[serde(default)]`.
        consumer_cpu: i64,
    }

    impl Default for FpssSection {
        fn default() -> Self {
            let prod = DirectConfig::production_defaults();
            Self {
                // Absent by default so the environment's host set stays the
                // single source of truth unless the operator lists hosts.
                hosts: None,
                connect_timeout: prod.streaming.connect_timeout_ms,
                read_timeout: prod.streaming.timeout_ms,
                ping_interval: prod.streaming.ping_interval_ms,
                reconnect_wait: prod.reconnect.wait_ms,
                reconnect_wait_rate_limited: prod.reconnect.wait_rate_limited_ms,
                ring_size: prod.streaming.ring_size,
                flush_mode: "batched".to_string(),
                wait_strategy: prod.streaming.wait_strategy.as_str().to_string(),
                wait_spin_iters: prod.streaming.wait_spin_iters,
                wait_yield_iters: prod.streaming.wait_yield_iters,
                wait_park_us: prod.streaming.wait_park_us,
                consumer_cpu: prod
                    .streaming
                    .consumer_cpu
                    .and_then(|c| i64::try_from(c).ok())
                    .unwrap_or(-1),
            }
        }
    }

    /// Streaming hosts can be specified as either a TOML array or a comma-separated string.
    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    enum FpssHosts {
        Array(Vec<String>),
        Csv(String),
    }

    #[derive(Debug, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct GrpcSection {
        window_size_kb: usize,
        connection_window_size_kb: usize,
        /// Max inbound message size, in MB. `None` (key absent) leaves the
        /// `[historical].max_message_size` byte value in force; an explicit
        /// value here — including the default of `4` — overrides it. Kept
        /// distinguishable from "absent" via `Option` so setting the
        /// override to the same number as the default is still honoured as
        /// an explicit choice rather than read as unset.
        max_message_size_mb: Option<usize>,
    }

    impl GrpcSection {
        /// Upper ceiling for `[grpc] max_message_size_mb`, in megabytes.
        ///
        /// The inbound message size is a pre-allocated decode budget, so an
        /// out-of-range value is a footgun in both directions: the
        /// MB→byte conversion (`mb * 1024 * 1024`) overflows `usize` for
        /// absurd inputs, and even a value that does not overflow commits
        /// the channel to a buffer far beyond any legitimate response. The
        /// production default is 4 MB; 64 MB leaves generous headroom for
        /// the largest bulk historical chunk while keeping the budget
        /// bounded. Values above this — or a `0` that would disable the
        /// limit entirely — are rejected by name at load time.
        const MAX_MESSAGE_SIZE_MB: usize = 64;
    }

    impl Default for GrpcSection {
        fn default() -> Self {
            let prod = DirectConfig::production_defaults();
            Self {
                window_size_kb: prod.historical.window_size_kb,
                connection_window_size_kb: prod.historical.connection_window_size_kb,
                // Absent by default so `[historical].max_message_size`
                // remains the single source of truth unless the operator
                // sets this MB-denominated override explicitly.
                max_message_size_mb: None,
            }
        }
    }

    impl FpssHosts {
        fn parse(self) -> Result<Vec<(String, u16)>, Error> {
            let entries = match self {
                FpssHosts::Array(arr) => arr,
                FpssHosts::Csv(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
            };
            let mut result = Vec::new();
            for entry in entries {
                let entry = entry.trim();
                if entry.is_empty() {
                    continue;
                }
                let (host, port_str) = entry.rsplit_once(':').ok_or_else(|| {
                    Error::config_invalid(
                        "streaming.hosts",
                        format!("invalid host:port entry: '{entry}'"),
                    )
                })?;
                let port: u16 = port_str.parse().map_err(|e| {
                    Error::config_invalid(
                        "streaming.hosts",
                        format!("invalid port in '{entry}': {e}"),
                    )
                })?;
                result.push((host.to_string(), port));
            }
            if result.is_empty() {
                return Err(Error::config_missing("streaming.hosts"));
            }
            Ok(result)
        }
    }

    impl DirectConfig {
        /// Load configuration from a TOML file.
        ///
        /// The file format matches `config.default.toml` shipped with the crate.
        /// Missing sections and keys fall back to [`DirectConfig::production()`] defaults.
        /// An unknown key or section is rejected so a typo surfaces as a load
        /// error instead of silently running the default.
        ///
        /// # Example file
        ///
        /// ```toml
        /// [historical]
        /// host = "mdds-01.thetadata.us"
        /// port = 443
        /// tls = true
        ///
        /// [streaming]
        /// hosts = ["nj-a.thetadata.us:20000", "nj-b.thetadata.us:20000"]
        /// reconnect_wait = 2000
        /// ring_size = 131072
        /// flush_mode = "batched"  # or "immediate"
        ///
        /// [grpc]
        /// window_size_kb = 64
        /// connection_window_size_kb = 64
        /// ```
        ///
        /// # Errors
        ///
        /// Returns [`Error::Config`] when the file cannot be read, when its
        /// contents are not valid TOML, or when the parsed values fail
        /// [`Self::validate`].
        pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, Error> {
            let contents = std::fs::read_to_string(path.as_ref()).map_err(|e| {
                Error::config_io(format!(
                    "failed to read config file '{}': {e}",
                    path.as_ref().display()
                ))
            })?;
            Self::from_toml_str(&contents)
        }

        /// Parse configuration from a TOML string.
        ///
        /// Same semantics as [`from_file`](Self::from_file) but takes a string directly.
        ///
        /// # Errors
        ///
        /// Returns [`Error::Config`] when the string is not valid TOML or
        /// when the parsed values fail [`Self::validate`].
        pub fn from_toml_str(toml_str: &str) -> Result<Self, Error> {
            let cf: ConfigFile =
                toml::from_str(toml_str).map_err(|e| Error::config_toml(e.to_string()))?;

            // An empty / absent value takes the documented default; any
            // other unrecognized value is a misconfiguration and is
            // rejected by name rather than silently falling back, so a
            // typo cannot quietly run a mode the operator did not pick.
            let flush_mode = match cf.streaming.flush_mode.trim().to_lowercase().as_str() {
                "" | "batched" => StreamingFlushMode::Batched,
                "immediate" => StreamingFlushMode::Immediate,
                other => {
                    return Err(Error::config_invalid(
                        "streaming.flush_mode",
                        format!(
                            "flush_mode must be one of \"batched\", \"immediate\"; got {other:?}"
                        ),
                    ));
                }
            };

            // Same contract as flush_mode: empty / absent takes the
            // default; an unrecognized value is reported by name with the
            // allowed set rather than silently defaulting.
            let wait_strategy = if cf.streaming.wait_strategy.trim().is_empty() {
                StreamingWaitStrategy::default()
            } else {
                cf.streaming
                    .wait_strategy
                    .parse::<StreamingWaitStrategy>()?
            };

            // `[historical].max_message_size` (bytes) is the canonical knob.
            // `[grpc].max_message_size_mb` (MB) is an explicit override that
            // wins when present — including when set to the same number as
            // the default — and is inert when absent.
            let max_message_size = match cf.grpc.max_message_size_mb {
                // The override is a pre-allocated decode budget. Reject a
                // `0` (which would disable the limit) or an out-of-ceiling
                // value up front, and compute the byte count with
                // `checked_mul` so an absurd input is reported as a range
                // error rather than wrapping `usize` into a tiny cap.
                Some(mb) => {
                    if mb == 0 || mb > GrpcSection::MAX_MESSAGE_SIZE_MB {
                        return Err(Error::config_out_of_range(
                            "grpc.max_message_size_mb",
                            i64::try_from(mb).unwrap_or(i64::MAX),
                            1,
                            i64::try_from(GrpcSection::MAX_MESSAGE_SIZE_MB).unwrap_or(i64::MAX),
                        ));
                    }
                    mb.checked_mul(1024 * 1024).ok_or_else(|| {
                        Error::config_out_of_range(
                            "grpc.max_message_size_mb",
                            i64::try_from(mb).unwrap_or(i64::MAX),
                            1,
                            i64::try_from(GrpcSection::MAX_MESSAGE_SIZE_MB).unwrap_or(i64::MAX),
                        )
                    })?
                }
                None => cf.historical.max_message_size,
            };

            let mut out = DirectConfig::production_defaults();
            // An explicit `[historical] host` is the operator's choice, so
            // record it as a tracked override: a later
            // `with_historical_environment()` / `stage()` / `dev()` must respect
            // it rather than clobber it back to the environment default. The
            // setter mirrors the value onto the live field immediately, so this
            // path needs no separate field write. An absent key leaves the
            // production default in force and records no override, so a later
            // environment switch still re-points the host.
            if let Some(host) = cf.historical.host {
                out.set_historical_host(host);
            }
            out.historical.port = cf.historical.port;
            out.historical.tls = cf.historical.tls;
            out.historical.max_message_size = max_message_size;
            out.historical.keepalive_secs = cf.historical.keepalive_time_secs;
            out.historical.keepalive_timeout_secs = cf.historical.keepalive_timeout_secs;
            out.historical.window_size_kb = cf.grpc.window_size_kb;
            out.historical.connection_window_size_kb = cf.grpc.connection_window_size_kb;
            // mdds.connect_timeout_secs is not yet TOML-surfaced; keep production default.

            // An explicit `[streaming] hosts` list is the operator's full host
            // set (a power-user list), so record it as a full override: a
            // later `with_streaming_environment()` / `dev()` respects it
            // outright rather than rebuilding from the environment cluster. The
            // setter mirrors it onto the live field immediately, so this path
            // needs no separate field write. An absent key leaves the
            // production default host set in force and records no override, so
            // a later environment switch still re-points it.
            if let Some(hosts) = cf.streaming.hosts {
                out.set_streaming_hosts_full_override(hosts.parse()?);
            }
            out.streaming.timeout_ms = cf.streaming.read_timeout;
            out.streaming.ring_size = cf.streaming.ring_size;
            out.streaming.ping_interval_ms = cf.streaming.ping_interval;
            out.streaming.connect_timeout_ms = cf.streaming.connect_timeout;
            out.streaming.flush_mode = flush_mode;
            out.streaming.wait_strategy = wait_strategy;
            out.streaming.wait_spin_iters = cf.streaming.wait_spin_iters;
            out.streaming.wait_yield_iters = cf.streaming.wait_yield_iters;
            out.streaming.wait_park_us = cf.streaming.wait_park_us;
            // A negative TOML `consumer_cpu` (default `-1`) means unpinned.
            out.streaming.consumer_cpu = usize::try_from(cf.streaming.consumer_cpu).ok();

            out.reconnect.wait_ms = cf.streaming.reconnect_wait;
            out.reconnect.wait_rate_limited_ms = cf.streaming.reconnect_wait_rate_limited;
            // TOML config cannot express custom closures; default to Auto.
            // Use the builder API to set Manual or Custom programmatically.
            out.reconnect.policy = ReconnectPolicy::Auto(ReconnectAttemptLimits::default());

            // TOML does not surface RetryPolicy / observability fields
            // today — the builder API (`with_retry_policy`,
            // `with_metrics_port`, env vars) is the opt-in path.
            out.retry = RetryPolicy::default();
            out.auth.nexus_url = DirectConfig::DEFAULT_NEXUS_URL.to_string();
            out.auth.client_type = DirectConfig::DEFAULT_CLIENT_TYPE.to_string();
            out.metrics.port = None;
            out.runtime.tokio_worker_threads = None;

            out.validate()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_historical_uri() {
        // `DirectConfig::production()` reads `THETADATA_HISTORICAL_*` env
        // vars; another test in this module (`env_overrides_apply_on_production`)
        // mutates the same env via `unsafe`, and the env is process-
        // global. Acquire the shared test guard so the two cannot
        // race when `cargo test` runs them in parallel.
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.historical_uri(), "https://mdds-01.thetadata.us:443");
    }

    #[test]
    fn production_selects_prod_on_both_channels() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(config.streaming_environment, StreamingEnvironment::Prod);
        assert_eq!(config.historical.host, "mdds-01.thetadata.us");
    }

    #[test]
    fn stage_selects_historical_staging_and_leaves_streaming_on_prod() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::stage();
        // Historical flips to staging (host + auth marker).
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(config.historical.host, "mdds-stage.thetadata.us");
        assert_eq!(config.historical.port, 443);
        assert!(config.historical.tls);
        // Streaming stays on PRODUCTION — there is no streaming staging cluster,
        // and the channels are independent. The staging :20100 hosts must NOT
        // appear.
        assert_eq!(config.streaming_environment, StreamingEnvironment::Prod);
        assert_eq!(
            config.streaming.hosts,
            StreamingConfig::production_defaults().hosts
        );
        assert!(
            !config
                .streaming
                .hosts
                .iter()
                .any(|(_, port)| *port == 20100),
            "no streaming staging (:20100) path: {:?}",
            config.streaming.hosts
        );
    }

    #[test]
    fn dev_selects_streaming_dev_and_leaves_historical_on_prod() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::dev();
        // Streaming flips to the dev replay cluster (port 20200).
        assert_eq!(config.streaming_environment, StreamingEnvironment::Dev);
        assert_eq!(
            config.streaming.hosts,
            vec![
                ("nj-a.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20201),
            ]
        );
        // Historical stays on PRODUCTION — there is no dev historical cluster.
        assert_eq!(config.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(config.historical.host, "mdds-01.thetadata.us");
    }

    #[test]
    fn the_two_channels_select_independently() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // historical-staging + streaming-dev in one config — the channels are
        // orthogonal.
        let config = DirectConfig::production()
            .with_historical_environment(HistoricalEnvironment::Stage)
            .with_streaming_environment(StreamingEnvironment::Dev);
        assert_eq!(config.historical.host, "mdds-stage.thetadata.us");
        assert_eq!(
            config.streaming.hosts,
            vec![
                ("nj-a.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20201),
            ]
        );
        // And the mirror: historical-prod + streaming-prod is the production
        // baseline, with neither channel pulling the other.
        let baseline = DirectConfig::production();
        assert_eq!(baseline.historical.host, "mdds-01.thetadata.us");
        assert_eq!(
            baseline.streaming.hosts,
            StreamingConfig::production_defaults().hosts
        );
    }

    #[test]
    fn streaming_dev_auth_body_is_byte_identical_to_prod() {
        // Streaming dev must NOT change auth: a dev config's historical
        // environment is production, so its auth request stays byte-identical
        // to a production one (no `authEnv`). This locks the live-proven prod
        // auth path for a streaming-dev session. Asserted at the boundary the
        // auth request is built from — the HISTORICAL environment.
        use crate::auth::nexus::auth_request_json_for_test;
        let _guard = env_test_guard();
        clear_env_matrix();
        let dev_hist = DirectConfig::dev().historical_environment();
        let prod_hist = DirectConfig::production().historical_environment();
        assert_eq!(dev_hist, HistoricalEnvironment::Prod);
        assert_eq!(prod_hist, HistoricalEnvironment::Prod);
        let dev_body = auth_request_json_for_test(dev_hist);
        let prod_body = auth_request_json_for_test(prod_hist);
        assert!(
            dev_body.get("authEnv").is_none(),
            "a streaming-dev config's auth body must omit authEnv: {dev_body}"
        );
        assert_eq!(
            dev_body, prod_body,
            "a streaming-dev config's auth body must be byte-identical to production's"
        );
    }

    #[test]
    fn dev_then_set_historical_host_keeps_dev_streaming_cluster() {
        // THE BUG: a post-construction override on a `dev()` config used to
        // rebuild streaming from the (prod) environment base and silently
        // drop the dev replay cluster, reconnecting FPSS to production. With
        // dev as a first-class environment, the override layer rebuilds from
        // the DEV base, so the historical override applies AND the dev
        // streaming cluster is preserved.
        let _guard = env_test_guard();
        clear_env_matrix();
        let mut config = DirectConfig::dev();
        config.set_historical_host("custom-hist.example.com");
        assert_eq!(config.streaming_environment, StreamingEnvironment::Dev);
        assert_eq!(
            config.historical.host, "custom-hist.example.com",
            "the explicit historical override must apply"
        );
        assert_eq!(
            config.streaming.hosts,
            DirectConfig::dev_streaming_hosts(),
            "the dev streaming cluster must NOT revert to production after an override"
        );
    }

    #[test]
    fn dev_then_set_streaming_hosts_then_set_historical_keeps_both() {
        // A full streaming override on a dev config, then a later historical
        // override (which re-runs the override layer): the full streaming
        // list wins and survives, and the dev streaming marker is intact.
        let _guard = env_test_guard();
        clear_env_matrix();
        let mut config = DirectConfig::dev();
        let custom = vec![("dev-stream.example.com".to_string(), 4242)];
        config.set_streaming_hosts(custom.clone());
        config.set_historical_host("dev-hist.example.com");
        assert_eq!(config.streaming_environment, StreamingEnvironment::Dev);
        assert_eq!(config.historical.host, "dev-hist.example.com");
        assert_eq!(
            config.streaming.hosts, custom,
            "an explicit full streaming list on a dev config must survive a later override"
        );
    }

    #[test]
    fn production_has_four_streaming_hosts() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.streaming.hosts.len(), 4);
    }

    #[test]
    fn production_default_reconnect_policy_is_auto() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert!(matches!(config.reconnect.policy, ReconnectPolicy::Auto(_)));
    }

    #[test]
    fn production_historical_connect_timeout_default_is_ten_seconds() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.historical.connect_timeout_secs, 10);
    }

    #[test]
    fn read_accessors_match_nested_fields() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let config = DirectConfig::production();
        assert_eq!(config.historical_host(), config.historical.host.as_str());
        assert_eq!(config.streaming_hosts(), config.streaming.hosts.as_slice());
    }

    #[test]
    fn parse_streaming_hosts_parses_multi_host_csv_with_whitespace_and_empty_entries() {
        let hosts = DirectConfig::parse_streaming_hosts(
            " nj-a.thetadata.us:20000, ,nj-b.thetadata.us:20001 ",
        )
        .unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0], ("nj-a.thetadata.us".to_string(), 20000));
        assert_eq!(hosts[1], ("nj-b.thetadata.us".to_string(), 20001));
    }

    #[test]
    fn parse_streaming_hosts_rejects_malformed_entries() {
        assert!(DirectConfig::parse_streaming_hosts("").is_err());
        assert!(DirectConfig::parse_streaming_hosts("host:notaport").is_err());
        assert!(DirectConfig::parse_streaming_hosts("hostonly").is_err());
    }

    // -- Config file tests (only compiled with the `config-file` feature) --

    #[cfg(feature = "config-file")]
    mod config_file_tests {
        use crate::config::{DirectConfig, StreamingFlushMode, StreamingWaitStrategy};

        #[test]
        fn empty_toml_gives_production_defaults() {
            let config = DirectConfig::from_toml_str("").unwrap();
            let prod = DirectConfig::production();
            assert_eq!(config.historical.host, prod.historical.host);
            assert_eq!(config.historical.port, prod.historical.port);
            assert_eq!(config.streaming.hosts.len(), prod.streaming.hosts.len());
            assert_eq!(config.streaming.ring_size, prod.streaming.ring_size);
        }

        #[test]
        fn partial_toml_overrides_only_specified() {
            let toml = r#"
                [historical]
                host = "custom.example.com"
                port = 8443

                [streaming]
                ring_size = 65536
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.host, "custom.example.com");
            assert_eq!(config.historical.port, 8443);
            assert_eq!(config.streaming.ring_size, 65536);
            // Unspecified fields keep production defaults
            assert!(config.historical.tls);
        }

        #[test]
        fn streaming_hosts_as_array() {
            let toml = r#"
                [streaming]
                hosts = ["host-a.example.com:20000", "host-b.example.com:20001"]
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.streaming.hosts.len(), 2);
            assert_eq!(
                config.streaming.hosts[0],
                ("host-a.example.com".to_string(), 20000)
            );
            assert_eq!(
                config.streaming.hosts[1],
                ("host-b.example.com".to_string(), 20001)
            );
        }

        #[test]
        fn streaming_hosts_as_csv_string() {
            let toml = r#"
                [streaming]
                hosts = "host-a.example.com:20000,host-b.example.com:20001"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.streaming.hosts.len(), 2);
            assert_eq!(config.streaming.hosts[0].0, "host-a.example.com");
        }

        #[test]
        fn flush_mode_immediate() {
            let toml = r#"
                [streaming]
                flush_mode = "immediate"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.streaming.flush_mode, StreamingFlushMode::Immediate);
        }

        #[test]
        fn flush_mode_batched_by_default() {
            let toml = r#"
                [streaming]
                flush_mode = "batched"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.streaming.flush_mode, StreamingFlushMode::Batched);
        }

        #[test]
        fn wait_strategy_and_tuning_round_trip() {
            let toml = r#"
                [streaming]
                wait_strategy = "balanced"
                wait_spin_iters = 16
                wait_yield_iters = 2
                wait_park_us = 75
                consumer_cpu = 3
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(
                config.streaming.wait_strategy,
                StreamingWaitStrategy::Balanced
            );
            assert_eq!(config.streaming.wait_spin_iters, 16);
            assert_eq!(config.streaming.wait_yield_iters, 2);
            assert_eq!(config.streaming.wait_park_us, 75);
            assert_eq!(config.streaming.consumer_cpu, Some(3));
        }

        #[test]
        fn wait_strategy_defaults_low_latency_unpinned() {
            // An empty streaming section keeps the low-latency default
            // and leaves the consumer unpinned.
            let toml = r#"
                [streaming]
                ring_size = 4096
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(
                config.streaming.wait_strategy,
                StreamingWaitStrategy::LowLatency
            );
            assert_eq!(config.streaming.consumer_cpu, None);
        }

        #[test]
        fn negative_consumer_cpu_means_unpinned() {
            let toml = r#"
                [streaming]
                consumer_cpu = -1
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.streaming.consumer_cpu, None);
        }

        #[test]
        fn grpc_section_sets_window_sizes() {
            let toml = r#"
                [grpc]
                window_size_kb = 128
                connection_window_size_kb = 256
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.window_size_kb, 128);
            assert_eq!(config.historical.connection_window_size_kb, 256);
        }

        #[test]
        fn grpc_max_message_size_mb_overrides_historical_bytes() {
            let toml = r#"
                [grpc]
                max_message_size_mb = 8
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.max_message_size, 8 * 1024 * 1024);
        }

        #[test]
        fn unknown_field_is_rejected() {
            // A misspelled knob (here `ring_size` -> `ringsize`) must
            // surface as a load error rather than parsing fine and
            // silently running the default.
            let toml = r#"
                [streaming]
                ringsize = 65536
            "#;
            let err =
                DirectConfig::from_toml_str(toml).expect_err("a misspelled field must be rejected");
            assert!(err.to_string().contains("ringsize"), "{err}");
        }

        #[test]
        fn unknown_section_is_rejected() {
            let toml = r#"
                [some_unknown_section]
                foo = "bar"
            "#;
            let err =
                DirectConfig::from_toml_str(toml).expect_err("an unknown section must be rejected");
            assert!(err.to_string().contains("some_unknown_section"), "{err}");
        }

        #[test]
        fn dead_auth_section_is_rejected() {
            // The credential path is not part of this tuning config;
            // credentials load via the credential API. A `[auth]` block
            // here is a dead knob and must be rejected, not silently
            // accepted as a no-op.
            let toml = r#"
                [auth]
                creds_file = "creds.txt"
            "#;
            let err = DirectConfig::from_toml_str(toml)
                .expect_err("the removed [auth] section must be rejected");
            assert!(err.to_string().contains("auth"), "{err}");
        }

        #[test]
        fn bad_flush_mode_is_rejected() {
            let toml = r#"
                [streaming]
                flush_mode = "imediate"
            "#;
            let err = DirectConfig::from_toml_str(toml)
                .expect_err("a misspelled flush_mode must be rejected");
            let msg = err.to_string();
            assert!(msg.contains("flush_mode"), "{msg}");
            assert!(msg.contains("imediate"), "{msg}");
        }

        #[test]
        fn bad_wait_strategy_is_rejected() {
            let toml = r#"
                [streaming]
                wait_strategy = "lowlatency"
            "#;
            let err = DirectConfig::from_toml_str(toml)
                .expect_err("a misspelled wait_strategy must be rejected");
            let msg = err.to_string();
            assert!(msg.contains("wait_strategy"), "{msg}");
            assert!(msg.contains("lowlatency"), "{msg}");
        }

        #[test]
        fn grpc_max_message_size_mb_default_value_is_honored_when_explicit() {
            // Explicitly setting the override to the same number as the
            // production default (4 MB) must still take effect as an
            // explicit choice — "set to 4" is distinguishable from "absent".
            let toml = r#"
                [historical]
                max_message_size = 8388608

                [grpc]
                max_message_size_mb = 4
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.max_message_size, 4 * 1024 * 1024);
        }

        #[test]
        fn grpc_max_message_size_mb_at_ceiling_is_accepted() {
            let toml = r#"
                [grpc]
                max_message_size_mb = 64
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.max_message_size, 64 * 1024 * 1024);
        }

        #[test]
        fn grpc_max_message_size_mb_above_ceiling_is_rejected() {
            let toml = r#"
                [grpc]
                max_message_size_mb = 65
            "#;
            let err = DirectConfig::from_toml_str(toml)
                .expect_err("a value above the ceiling must be rejected");
            assert!(err.to_string().contains("max_message_size_mb"), "{err}");
        }

        #[test]
        fn grpc_max_message_size_mb_zero_is_rejected() {
            // `0` would disable the inbound-size limit entirely; it must be
            // reported by name rather than silently uncapping the channel.
            let toml = r#"
                [grpc]
                max_message_size_mb = 0
            "#;
            let err =
                DirectConfig::from_toml_str(toml).expect_err("a zero override must be rejected");
            assert!(err.to_string().contains("max_message_size_mb"), "{err}");
        }

        #[test]
        fn grpc_max_message_size_mb_absurd_value_does_not_panic_or_wrap() {
            // A value that would overflow the MB→byte conversion must
            // surface as a range error, never a debug panic or a release
            // wrap into a tiny garbage cap.
            let toml = format!("[grpc]\nmax_message_size_mb = {}\n", usize::MAX);
            let err = DirectConfig::from_toml_str(&toml)
                .expect_err("an absurd value must be rejected, not wrapped");
            assert!(err.to_string().contains("max_message_size_mb"), "{err}");
        }

        #[test]
        fn grpc_max_message_size_absent_keeps_historical_bytes() {
            // With no [grpc] override the canonical [historical] byte value
            // stays in force.
            let toml = r#"
                [historical]
                max_message_size = 8388608
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            assert_eq!(config.historical.max_message_size, 8 * 1024 * 1024);
        }

        #[test]
        fn full_config_default_toml_parses() {
            // Validate that config.default.toml (shipped with the crate) can be parsed.
            let default_toml = include_str!("../../config.default.toml");
            let config = DirectConfig::from_toml_str(default_toml).unwrap();
            assert_eq!(config.historical.host, "mdds-01.thetadata.us");
            assert_eq!(config.historical.port, 443);
            assert_eq!(config.streaming.hosts.len(), 4);
        }

        #[test]
        fn config_default_toml_matches_production_defaults() {
            // The shipped template fills every tuning knob explicitly, and
            // `#[serde(default)]` only backfills absent keys — so any value
            // written here OVERRIDES the code default the moment an operator
            // copies the file. Pin every parsed value to
            // `DirectConfig::production()` so the template can never silently
            // drift from the in-code defaults.
            let default_toml = include_str!("../../config.default.toml");
            let config = DirectConfig::from_toml_str(default_toml).unwrap();
            let prod = DirectConfig::production();

            // Historical (gRPC).
            assert_eq!(config.historical.host, prod.historical.host);
            assert_eq!(config.historical.port, prod.historical.port);
            assert_eq!(config.historical.tls, prod.historical.tls);
            assert_eq!(
                config.historical.keepalive_secs,
                prod.historical.keepalive_secs
            );
            assert_eq!(
                config.historical.keepalive_timeout_secs,
                prod.historical.keepalive_timeout_secs
            );
            assert_eq!(
                config.historical.max_message_size,
                prod.historical.max_message_size
            );
            assert_eq!(
                config.historical.window_size_kb,
                prod.historical.window_size_kb
            );
            assert_eq!(
                config.historical.connection_window_size_kb,
                prod.historical.connection_window_size_kb
            );

            // Streaming (TCP).
            assert_eq!(config.streaming.hosts, prod.streaming.hosts);
            assert_eq!(
                config.streaming.connect_timeout_ms,
                prod.streaming.connect_timeout_ms
            );
            assert_eq!(config.streaming.timeout_ms, prod.streaming.timeout_ms);
            assert_eq!(
                config.streaming.ping_interval_ms,
                prod.streaming.ping_interval_ms
            );
            assert_eq!(config.streaming.ring_size, prod.streaming.ring_size);
            assert_eq!(config.streaming.flush_mode, prod.streaming.flush_mode);
            assert_eq!(config.streaming.wait_strategy, prod.streaming.wait_strategy);
            assert_eq!(
                config.streaming.wait_spin_iters,
                prod.streaming.wait_spin_iters
            );
            assert_eq!(
                config.streaming.wait_yield_iters,
                prod.streaming.wait_yield_iters
            );
            assert_eq!(config.streaming.wait_park_us, prod.streaming.wait_park_us);
            assert_eq!(config.streaming.consumer_cpu, prod.streaming.consumer_cpu);

            // Reconnect cadence.
            assert_eq!(config.reconnect.wait_ms, prod.reconnect.wait_ms);
            assert_eq!(
                config.reconnect.wait_rate_limited_ms,
                prod.reconnect.wait_rate_limited_ms
            );
        }

        #[test]
        fn config_default_toml_uses_canonical_section_names() {
            // The deserializer binds `[historical]` / `[streaming]`; the
            // internal vendor names `[mdds]` / `[fpss]` deserialize to
            // nothing, so a sample shipping them silently discards every
            // override. Asserting host==443 above can't catch that (those
            // equal the production defaults), so pin the section names in
            // the shipped sample directly.
            let default_toml = include_str!("../../config.default.toml");
            assert!(
                default_toml.contains("[historical]"),
                "config.default.toml must use the canonical [historical] section"
            );
            assert!(
                default_toml.contains("[streaming]"),
                "config.default.toml must use the canonical [streaming] section"
            );
            assert!(
                !default_toml.contains("[mdds]") && !default_toml.contains("[fpss]"),
                "config.default.toml must not ship the dead [mdds]/[fpss] section names"
            );
        }

        #[test]
        fn invalid_toml_returns_error() {
            let result = DirectConfig::from_toml_str("this is not valid toml [[[");
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("TOML"));
        }

        #[test]
        fn config_file_streaming_hosts_win_over_later_environment_switch() {
            use crate::config::HistoricalEnvironment;
            // The config-file `[streaming] hosts` list is the power-user full
            // override: a later `with_historical_environment(Stage)` must respect
            // it outright (environment selection does not touch the streaming
            // hosts at all).
            let toml = r#"
                [streaming]
                hosts = ["a.example.com:1", "b.example.com:2"]
            "#;
            let config = DirectConfig::from_toml_str(toml)
                .unwrap()
                .with_historical_environment(HistoricalEnvironment::Stage);
            assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
            assert_eq!(
                config.streaming.hosts,
                vec![
                    ("a.example.com".to_string(), 1),
                    ("b.example.com".to_string(), 2),
                ],
                "the config-file full host list must win over any cluster switch"
            );
        }

        #[test]
        fn config_file_historical_host_wins_over_later_environment_switch() {
            // The config-file `[historical] host` is an explicit override that
            // must survive a later `stage()` / `with_historical_environment`
            // switch while the historical marker still flips to staging.
            let toml = r#"
                [historical]
                host = "h.example.com"
            "#;
            let config = DirectConfig::from_toml_str(toml).unwrap();
            // Build the stage routing on top of the config-file override.
            let staged = config.with_historical_environment(super::HistoricalEnvironment::Stage);
            assert_eq!(
                staged.historical_environment,
                super::HistoricalEnvironment::Stage
            );
            assert_eq!(
                staged.historical.host, "h.example.com",
                "the config-file historical host must survive the environment switch"
            );
        }

        #[test]
        fn config_file_omitting_hosts_lets_environment_switch_repoint() {
            use crate::config::{HistoricalEnvironment, StreamingConfig};
            // When the TOML omits `[streaming] hosts` and `[historical] host`,
            // no override is recorded, so a later historical environment switch
            // re-points the historical host to the staging cluster. Streaming
            // stays on production — there is no streaming staging cluster, and
            // the channels are independent.
            let toml = r#"
                [streaming]
                ring_size = 65536
            "#;
            let config = DirectConfig::from_toml_str(toml)
                .unwrap()
                .with_historical_environment(HistoricalEnvironment::Stage);
            assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
            assert_eq!(config.historical.host, "mdds-stage.thetadata.us");
            assert_eq!(
                config.streaming.hosts,
                StreamingConfig::production_defaults().hosts
            );
            // The non-host tuning knob from the file is still honoured.
            assert_eq!(config.streaming.ring_size, 65536);
        }
    }

    // -- Validation tests --

    #[test]
    fn validate_clamps_historical_window_sizes_into_range() {
        let mut config = DirectConfig::production_defaults();
        config.historical.window_size_kb = 2_048;
        config.historical.connection_window_size_kb = 32;
        let config = config
            .validate()
            .expect("historical window sizes are clamped");
        assert_eq!(config.historical.window_size_kb, 1_024);
        assert_eq!(config.historical.connection_window_size_kb, 64);
    }

    #[test]
    fn validate_preserves_in_range_values() {
        let config = DirectConfig::production_defaults();
        let validated = config.validate().expect("production defaults validate");
        assert_eq!(validated.historical.window_size_kb, 64);
        assert_eq!(validated.streaming.timeout_ms, 3_000);
        assert_eq!(validated.streaming.ping_interval_ms, 250);
        assert_eq!(validated.streaming.connect_timeout_ms, 2_000);
        assert_eq!(validated.streaming.io_read_slice_ms, 25);
        assert_eq!(validated.streaming.data_watchdog_ms, 30_000);
        assert_eq!(validated.streaming.keepalive_idle_secs, 5);
        assert_eq!(validated.streaming.keepalive_interval_secs, 2);
        assert_eq!(validated.streaming.keepalive_retries, 2);
        assert_eq!(validated.reconnect.wait_ms, 250);
        assert_eq!(validated.reconnect.wait_max_ms, 30_000);
        assert_eq!(validated.reconnect.replay_burst_size, 50);
        assert_eq!(validated.reconnect.replay_pace_ms, 5);
    }

    #[test]
    fn validate_rejects_io_read_slice_out_of_range() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.io_read_slice_ms = 5;
        let err = config.validate().expect_err("must reject below-minimum");
        assert!(err.to_string().contains("io_read_slice_ms"));
    }

    #[test]
    fn validate_accepts_disabled_data_watchdog() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.data_watchdog_ms = 0;
        let validated = config
            .validate()
            .expect("0 disables the watchdog and must validate");
        assert_eq!(validated.streaming.data_watchdog_ms, 0);
    }

    #[test]
    fn validate_rejects_data_watchdog_below_read_timeout() {
        let mut config = DirectConfig::production_defaults();
        // Above the band floor but below timeout_ms — the backstop would
        // fire before the read timeout it is meant to backstop.
        config.streaming.timeout_ms = 5_000;
        config.streaming.data_watchdog_ms = 1_000;
        let err = config
            .validate()
            .expect_err("watchdog below timeout_ms must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("data_watchdog_ms"), "{msg}");
        assert!(msg.contains("timeout_ms"), "{msg}");
    }

    #[test]
    fn validate_rejects_data_watchdog_above_maximum() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.data_watchdog_ms = 7_200_000;
        let err = config
            .validate()
            .expect_err("watchdog above the ceiling must be rejected");
        assert!(err.to_string().contains("data_watchdog_ms"));
    }

    #[test]
    fn validate_accepts_in_range_data_watchdog() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.timeout_ms = 3_000;
        config.streaming.data_watchdog_ms = 60_000;
        let validated = config
            .validate()
            .expect("an enabled watchdog at or above timeout_ms validates");
        assert_eq!(validated.streaming.data_watchdog_ms, 60_000);
    }

    #[test]
    fn validate_rejects_keepalive_out_of_range() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.keepalive_idle_secs = 0;
        let err = config.validate().expect_err("must reject zero idle");
        assert!(err.to_string().contains("keepalive_idle_secs"));

        let mut config = DirectConfig::production_defaults();
        config.streaming.keepalive_interval_secs = 80;
        let err = config
            .validate()
            .expect_err("must reject oversize interval");
        assert!(err.to_string().contains("keepalive_interval_secs"));

        let mut config = DirectConfig::production_defaults();
        config.streaming.keepalive_retries = 0;
        let err = config.validate().expect_err("must reject zero retries");
        assert!(err.to_string().contains("keepalive_retries"));
    }

    #[test]
    fn validate_rejects_degenerate_replay_and_ladder() {
        let mut config = DirectConfig::production_defaults();
        config.reconnect.replay_burst_size = 0;
        let err = config.validate().expect_err("must reject zero burst");
        assert!(err.to_string().contains("replay_burst_size"));

        let mut config = DirectConfig::production_defaults();
        config.reconnect.wait_ms = 60_000;
        config.reconnect.wait_max_ms = 1_000;
        let err = config.validate().expect_err("must reject inverted ladder");
        assert!(err.to_string().contains("wait_max_ms"));
    }

    #[test]
    fn validate_rejects_streaming_timeout_below_minimum() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.timeout_ms = 50;
        let err = config.validate().expect_err("must reject below-minimum");
        let msg = err.to_string();
        assert!(msg.contains("timeout_ms"), "{msg}");
    }

    #[test]
    fn validate_rejects_streaming_timeout_above_maximum() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.timeout_ms = 120_000;
        let err = config.validate().expect_err("must reject above-maximum");
        assert!(err.to_string().contains("timeout_ms"));
    }

    #[test]
    fn validate_rejects_streaming_connect_timeout_out_of_range() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.connect_timeout_ms = 100;
        let err = config.validate().expect_err("100ms is below 1s minimum");
        assert!(err.to_string().contains("connect_timeout_ms"));
    }

    #[test]
    fn validate_rejects_streaming_ping_interval_out_of_range() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.ping_interval_ms = 50;
        let err = config.validate().expect_err("50ms below 100ms minimum");
        assert!(err.to_string().contains("ping_interval_ms"));
    }

    #[test]
    fn validate_rejects_invalid_ring_size() {
        let mut config = DirectConfig::production_defaults();
        config.streaming.ring_size = 100; // not a power of two
        let err = config.validate().expect_err("must reject non-power-of-two");
        assert!(err.to_string().contains("ring_size"));
    }

    #[test]
    fn validate_rejects_zero_reconnect_wait_ms() {
        let mut config = DirectConfig::production_defaults();
        config.reconnect.wait_ms = 0;
        let err = config
            .validate()
            .expect_err("must reject zero base reconnect cadence");
        assert!(err.to_string().contains("wait_ms"));
    }

    #[test]
    fn validate_rejects_zero_reconnect_wait_max_ms() {
        let mut config = DirectConfig::production_defaults();
        // Drive both ends to 0 so the ordering check cannot mask the floor:
        // the band-check must reject the degenerate ceiling on its own.
        config.reconnect.wait_ms = 0;
        config.reconnect.wait_max_ms = 0;
        let err = config
            .validate()
            .expect_err("must reject zero reconnect ceiling");
        assert!(err.to_string().contains("wait_ms"));
    }

    #[test]
    fn validate_rejects_above_band_reconnect_wait_ms() {
        let mut config = DirectConfig::production_defaults();
        config.reconnect.wait_ms = 600_001;
        config.reconnect.wait_max_ms = 600_001;
        let err = config
            .validate()
            .expect_err("must reject above-band reconnect cadence");
        assert!(err.to_string().contains("wait_ms"));
    }

    #[test]
    fn validate_accepts_in_band_reconnect_wait_ms() {
        let mut config = DirectConfig::production_defaults();
        config.reconnect.wait_ms = 500;
        config.reconnect.wait_max_ms = 60_000;
        config
            .validate()
            .expect("a legitimate base/ceiling pair must validate");
    }

    #[test]
    fn validate_rejects_zero_reconnect_rate_limited_cadence() {
        let mut config = DirectConfig::production_defaults();
        config.reconnect.wait_rate_limited_ms = 0;
        let err = config.validate().expect_err("must reject zero cadence");
        assert!(err.to_string().contains("wait_rate_limited_ms"));
    }

    #[test]
    fn validate_rejects_zero_reconnect_attempt_budget() {
        let mut config = DirectConfig::production_defaults();
        config.reconnect.policy = ReconnectPolicy::Auto(ReconnectAttemptLimits {
            max_attempts: 0,
            ..ReconnectAttemptLimits::default()
        });
        let err = config
            .validate()
            .expect_err("must reject zero attempt budget");
        assert!(err.to_string().contains("max_attempts"));
    }

    #[test]
    fn validate_rejects_zero_flatfiles_connect_timeout() {
        let mut config = DirectConfig::production_defaults();
        config.flatfiles.connect_timeout_secs = 0;
        let err = config
            .validate()
            .expect_err("must reject zero connect timeout");
        assert!(err.to_string().contains("connect_timeout_secs"));
    }

    #[test]
    fn validate_rejects_inverted_retry_delays() {
        let mut config = DirectConfig::production_defaults();
        config.retry.initial_delay = std::time::Duration::from_secs(60);
        config.retry.max_delay = std::time::Duration::from_secs(1);
        let err = config
            .validate()
            .expect_err("must reject inverted retry ladder");
        assert!(err.to_string().contains("max_delay"));
    }

    #[test]
    fn historical_defaults_match_production_baseline() {
        let mdds = crate::config::HistoricalConfig::production_defaults();
        // The buffered-response warn fires at 100 MiB by default, and
        // the per-request deadline is the 300s ceiling. Channel-pool
        // concurrency carries no default here — it is resolved from
        // the subscription tier at connect time.
        assert_eq!(mdds.warn_on_buffered_threshold_bytes, 100 * 1024 * 1024);
        assert_eq!(mdds.request_timeout_secs, 300);
    }

    // ── RetryPolicy / env var tests ──────────────────────────────────

    #[test]
    fn retry_policy_default_shape_is_stable() {
        let p = RetryPolicy::default();
        assert_eq!(p.initial_delay, std::time::Duration::from_millis(250));
        assert_eq!(p.max_delay, std::time::Duration::from_secs(30));
        assert_eq!(p.max_attempts, 20);
        assert_eq!(p.max_elapsed, std::time::Duration::from_secs(300));
        assert!(p.jitter);
    }

    #[test]
    fn retry_policy_capped_backoff_doubles_each_attempt_then_caps() {
        use std::time::Duration;
        let p = RetryPolicy {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(800),
            max_attempts: 10,
            max_elapsed: Duration::ZERO,
            jitter: false,
        };
        assert_eq!(p.capped_backoff(0), Duration::ZERO);
        assert_eq!(p.capped_backoff(1), Duration::from_millis(100));
        assert_eq!(p.capped_backoff(2), Duration::from_millis(200));
        assert_eq!(p.capped_backoff(3), Duration::from_millis(400));
        assert_eq!(p.capped_backoff(4), Duration::from_millis(800));
        // Saturates at max_delay; never exceeds the cap even on absurd attempt counts.
        assert_eq!(p.capped_backoff(5), Duration::from_millis(800));
        assert_eq!(p.capped_backoff(60), Duration::from_millis(800));
    }

    #[test]
    fn retry_policy_delay_for_attempt_respects_jitter_upper_bound() {
        use std::time::Duration;
        let p = RetryPolicy {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(1_000),
            max_attempts: 10,
            max_elapsed: Duration::ZERO,
            jitter: true,
        };
        // Full-jitter envelope: sample ∈ [0, capped_backoff(attempt)].
        // Exercise 200 draws per attempt to shake out off-by-one issues
        // without making the test flaky — every sample must land in
        // the closed interval above.
        for attempt in 1..=6u32 {
            let ceiling = p.capped_backoff(attempt);
            for _ in 0..200 {
                let delay = p.delay_for_attempt(attempt);
                assert!(
                    delay <= ceiling,
                    "attempt {attempt}: delay {delay:?} exceeded ceiling {ceiling:?}"
                );
            }
        }
    }

    #[test]
    fn retry_policy_delay_for_attempt_deterministic_without_jitter() {
        use std::time::Duration;
        let p = RetryPolicy {
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(400),
            max_attempts: 5,
            max_elapsed: Duration::ZERO,
            jitter: false,
        };
        // No jitter → every draw equals the capped backoff envelope.
        for attempt in 1..=4u32 {
            let expected = p.capped_backoff(attempt);
            for _ in 0..16 {
                assert_eq!(p.delay_for_attempt(attempt), expected);
            }
        }
    }

    #[test]
    fn retry_policy_disabled_yields_single_attempt() {
        use std::time::Duration;
        let p = RetryPolicy::disabled();
        assert_eq!(p.max_attempts, 1);
        assert_eq!(p.delay_for_attempt(1), Duration::ZERO);
        assert!(!p.jitter);
    }

    // `std::env` is a process-global singleton; the env-var tests use a
    // single mutex so they don't trample each other under
    // `cargo test -- --test-threads=N`. Each test keeps hold of the
    // guard for the duration of the config build + assertions.
    fn env_test_guard() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn clear_env_matrix() {
        // Unset every variable the env-override path reads so no test
        // leaks into another. The guard above pins us as the sole writer.
        // SAFETY: the env-var tests serialise through `env_test_guard()`,
        // a process-global `Mutex<()>` held for the full body of every
        // env test; this function is only called from inside that
        // critical section. The Rust 1.88 `unsafe fn` contract on
        // `std::env::remove_var` requires the caller to ensure no other
        // thread reads or writes the environment concurrently — the
        // mutex provides exactly that.
        unsafe {
            std::env::remove_var(ENV_HISTORICAL_TYPE);
            std::env::remove_var(ENV_STREAMING_TYPE);
            std::env::remove_var(ENV_HISTORICAL_HOST);
            std::env::remove_var(ENV_HISTORICAL_PORT);
            std::env::remove_var(ENV_NEXUS_URL);
            std::env::remove_var(ENV_STREAMING_HOST);
            std::env::remove_var(ENV_STREAMING_PORT);
            std::env::remove_var(ENV_CLIENT_TYPE);
        }
    }

    #[test]
    fn historical_type_env_stage_selects_stage_cluster() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: `_guard` holds the process-global env-var mutex for the
        // body of this test, so no other thread observes or mutates the
        // environment while this write lands.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_TYPE, "STAGE");
        }
        let config = DirectConfig::production();
        // THETADATA_HISTORICAL_TYPE=STAGE yields the historical staging cluster +
        // Stage marker, identical to the `stage()` preset. Streaming stays on
        // production (no streaming staging cluster).
        let staged = DirectConfig::stage();
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(config.streaming_environment, StreamingEnvironment::Prod);
        assert_eq!(config.historical.host, staged.historical.host);
        assert_eq!(config.streaming.hosts, staged.streaming.hosts);
        clear_env_matrix();
    }

    #[test]
    fn historical_type_env_dev_panics_as_cross_channel_value() {
        // DEV is no longer a historical environment — it belongs to the
        // streaming channel (`THETADATA_STREAMING_TYPE`). A cross-channel
        // `THETADATA_HISTORICAL_TYPE=DEV` must FAIL LOUD via `production()`'s
        // `.expect`, never silently route the historical channel anywhere.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: `_guard` holds the process-global env-var mutex for the body
        // of this test, so no other thread observes or mutates the environment
        // while this write lands.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_TYPE, "DEV");
        }
        // `production()` panics on the invalid selector; catch the unwind so the
        // env matrix is still cleared even though the panic skips the tail.
        let panicked = std::panic::catch_unwind(DirectConfig::production).is_err();
        clear_env_matrix();
        assert!(
            panicked,
            "THETADATA_HISTORICAL_TYPE=DEV (a streaming-only value) must panic, not fall back"
        );
    }

    #[test]
    fn streaming_type_env_dev_selects_dev_streaming_cluster() {
        // The positive streaming counterpart: `THETADATA_STREAMING_TYPE=DEV` selects
        // the dev replay streaming cluster and nothing else — historical stays
        // on production, identical to the `dev()` preset.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_dev_panics_as_cross_channel_value`.
        unsafe {
            std::env::set_var(ENV_STREAMING_TYPE, "DEV");
        }
        let config = DirectConfig::production();
        let dev = DirectConfig::dev();
        assert_eq!(config.streaming_environment, StreamingEnvironment::Dev);
        assert_eq!(config.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(config.streaming.hosts, dev.streaming.hosts);
        assert_eq!(config.streaming.hosts, DirectConfig::dev_streaming_hosts());
        assert_eq!(config.historical.host, "mdds-01.thetadata.us");
        clear_env_matrix();
    }

    #[test]
    fn streaming_type_env_stage_panics_as_cross_channel_value() {
        // The mirror negative: STAGE is a historical-only value, so a
        // cross-channel `THETADATA_STREAMING_TYPE=STAGE` must FAIL LOUD via
        // `production()`'s `.expect`, never silently keep production streaming.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_dev_panics_as_cross_channel_value`.
        unsafe {
            std::env::set_var(ENV_STREAMING_TYPE, "STAGE");
        }
        let panicked = std::panic::catch_unwind(DirectConfig::production).is_err();
        clear_env_matrix();
        assert!(
            panicked,
            "THETADATA_STREAMING_TYPE=STAGE (a historical-only value) must panic, not fall back"
        );
    }

    #[test]
    fn historical_type_env_is_case_insensitive() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_TYPE, "  stage  ");
        }
        let config = DirectConfig::production();
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        clear_env_matrix();
    }

    #[test]
    fn historical_type_env_unrecognized_panics() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_dev_panics_as_cross_channel_value`.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_TYPE, "bogus");
        }
        // FAIL LOUD: an unrecognized selector is a hard error, so `production()`
        // panics via its `.expect` rather than silently keeping production.
        let panicked = std::panic::catch_unwind(DirectConfig::production).is_err();
        clear_env_matrix();
        assert!(
            panicked,
            "an unrecognized THETADATA_HISTORICAL_TYPE must panic, not fall back to production"
        );
    }

    #[test]
    fn explicit_historical_host_wins_over_historical_type_default() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_TYPE, "STAGE");
            std::env::set_var(ENV_HISTORICAL_HOST, "custom.example.com");
        }
        let config = DirectConfig::production();
        // The historical marker still flips to Stage, but an explicit host
        // override wins over the environment's default cluster host.
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(config.historical.host, "custom.example.com");
        clear_env_matrix();
    }

    #[test]
    fn explicit_historical_host_survives_stage_preset() {
        // Finding 2(a): an explicit `THETADATA_HISTORICAL_HOST` must survive
        // the `stage()` preset. `stage()` selects the historical staging cluster
        // for the marker, but the explicit host wins for the historical channel.
        // Streaming stays on production — there is no streaming staging cluster.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_HOST, "myhost");
        }
        let config = DirectConfig::stage();
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(
            config.historical.host, "myhost",
            "explicit historical host must survive stage()"
        );
        assert_eq!(
            config.streaming.hosts,
            StreamingConfig::production_defaults().hosts,
            "streaming stays on production; stage has no streaming cluster"
        );
        clear_env_matrix();
    }

    #[test]
    fn with_historical_environment_honors_previously_set_explicit_host() {
        // Finding 2(b): the typed `with_historical_environment(Stage)` setter,
        // applied after an explicit historical host was recorded, must keep that
        // host while still flipping the historical marker to stage. Streaming is
        // untouched and stays on production.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_HOST, "myhost");
        }
        let config =
            DirectConfig::production().with_historical_environment(HistoricalEnvironment::Stage);
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(
            config.historical.host, "myhost",
            "with_historical_environment must not clobber an explicit historical host"
        );
        assert_eq!(
            config.streaming.hosts,
            StreamingConfig::production_defaults().hosts
        );
        clear_env_matrix();
    }

    #[test]
    fn set_historical_host_survives_later_environment_switch() {
        // A host set via the tracked `set_historical_host` setter must survive
        // a later historical environment switch — the documented "explicit host
        // wins over the environment default" invariant holds for the
        // programmatic setter, not only the env-var path. Order: set-then-switch.
        let _guard = env_test_guard();
        clear_env_matrix();
        let mut config = DirectConfig::production();
        config.set_historical_host("programmatic.example.com");
        let staged = config.with_historical_environment(HistoricalEnvironment::Stage);
        assert_eq!(staged.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(
            staged.historical_host(),
            "programmatic.example.com",
            "a host set via set_historical_host must survive the environment switch"
        );
        // Streaming is untouched by a historical switch and stays on production.
        assert_eq!(
            staged.streaming_hosts(),
            StreamingConfig::production_defaults().hosts
        );
    }

    #[test]
    fn set_historical_host_after_a_switch_survives_the_next_switch() {
        // Order: switch-then-set(-then-switch). A host set AFTER an environment
        // switch must survive a SUBSEQUENT switch (and the round-trip back to
        // prod keeps it).
        let _guard = env_test_guard();
        clear_env_matrix();
        let mut config =
            DirectConfig::production().with_historical_environment(HistoricalEnvironment::Stage);
        // Set after the first switch.
        config.set_historical_host("edited-after-switch.example.com");
        let back = config.with_historical_environment(HistoricalEnvironment::Prod);
        assert_eq!(back.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(
            back.historical_host(),
            "edited-after-switch.example.com",
            "a host set after a switch must survive the next switch"
        );
        // And it persists across yet another switch.
        let staged_again = back.with_historical_environment(HistoricalEnvironment::Stage);
        assert_eq!(
            staged_again.historical_host(),
            "edited-after-switch.example.com",
            "the recorded host must persist across repeated switches"
        );
    }

    #[test]
    fn set_streaming_hosts_survive_later_environment_switch() {
        // A host list set via the tracked `set_streaming_hosts` setter must
        // survive a later historical environment switch (it wins outright, the
        // full-list tier), while the non-overridden historical host moves to the
        // staging cluster.
        let _guard = env_test_guard();
        clear_env_matrix();
        let mut config = DirectConfig::production();
        let custom = vec![
            ("prog-a.example.com".to_string(), 4100),
            ("prog-b.example.com".to_string(), 4101),
        ];
        config.set_streaming_hosts(custom.clone());
        let staged = config.with_historical_environment(HistoricalEnvironment::Stage);
        assert_eq!(staged.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(
            staged.streaming_hosts(),
            custom,
            "hosts set via set_streaming_hosts must survive the environment switch"
        );
        // The non-overridden historical host still moves to the stage cluster.
        assert_eq!(
            staged.historical_host(),
            HistoricalEnvironment::Stage.host()
        );
    }

    #[test]
    fn set_streaming_hosts_ignores_empty_list_and_keeps_primary_override() {
        // An empty full-override list must be ignored: it would leave the
        // streaming channel with no host and swallow a primary host/port
        // override. After the no-op the environment's cluster and any recorded
        // primary override are intact.
        let _guard = env_test_guard();
        clear_env_matrix();
        let mut config = DirectConfig::production();
        // Record a primary host override first.
        config.set_streaming_primary_host_override("primary.example.com".to_string());
        let prod_hosts = DirectConfig::production_defaults().streaming.hosts;
        assert_eq!(config.streaming.hosts[0].0, "primary.example.com");

        // An empty list is a no-op: it neither zeroes the host list nor
        // swallows the primary override.
        config.set_streaming_hosts(vec![]);
        assert!(
            !config.streaming.hosts.is_empty(),
            "an empty list must not clear the streaming hosts"
        );
        assert_eq!(
            config.streaming.hosts[0].0, "primary.example.com",
            "the primary override must survive an ignored empty list"
        );
        // Failover slots still track the production cluster.
        assert_eq!(&config.streaming.hosts[1..], &prod_hosts[1..]);

        // A non-empty list still wins outright as the full-override tier and
        // supersedes the primary override.
        let custom = vec![
            ("full-a.example.com".to_string(), 4100),
            ("full-b.example.com".to_string(), 4101),
        ];
        config.set_streaming_hosts(custom.clone());
        assert_eq!(config.streaming.hosts, custom);
    }

    #[test]
    fn validate_rejects_empty_streaming_hosts() {
        // `validate()` is the fail-fast backstop for every construction path:
        // an empty streaming host list is rejected at build time rather than
        // surfacing as a generic "no servers configured" at connect.
        let _guard = env_test_guard();
        clear_env_matrix();
        let mut config = DirectConfig::production_defaults();
        config.streaming.hosts = Vec::new();
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string().contains("streaming.hosts"),
            "validate must name the empty streaming.hosts field, got: {err}"
        );
    }

    #[test]
    fn set_streaming_hosts_full_list_survives_switch_even_when_equal_to_dev_hosts() {
        // A full list set via `set_streaming_hosts` is honoured by PROVENANCE,
        // not value: it must survive `with_historical_environment` even when it
        // happens to equal `dev_streaming_hosts()`. The old value-based
        // heuristic would have dropped a list that matched the dev cluster; the
        // encapsulated model keeps it because it was recorded as an override.
        let _guard = env_test_guard();
        clear_env_matrix();
        let mut config = DirectConfig::production();
        let dev_shaped = DirectConfig::dev_streaming_hosts();
        config.set_streaming_hosts(dev_shaped.clone());
        let staged = config.with_historical_environment(HistoricalEnvironment::Stage);
        assert_eq!(staged.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(
            staged.streaming_hosts(),
            dev_shaped,
            "an explicit list equal to the dev hosts must not be dropped by a value match"
        );
        // Historical (no override) still tracks the stage cluster.
        assert_eq!(
            staged.historical_host(),
            HistoricalEnvironment::Stage.host()
        );
    }

    #[test]
    fn set_both_channels_survive_together() {
        // Both channels set via their tracked setters, then a switch: each is
        // recorded independently and both survive.
        let _guard = env_test_guard();
        clear_env_matrix();
        let mut config = DirectConfig::production();
        config.set_historical_host("h.example.com");
        let custom = vec![("s.example.com".to_string(), 5000)];
        config.set_streaming_hosts(custom.clone());
        let staged = config.with_historical_environment(HistoricalEnvironment::Stage);
        assert_eq!(staged.historical_host(), "h.example.com");
        assert_eq!(staged.streaming_hosts(), custom);
    }

    #[test]
    fn no_override_presets_are_byte_identical_across_switches() {
        // Guard that the provenance model is invisible on the common paths: a
        // plain historical switch with no recorded override must yield the
        // selected environment's historical host verbatim while leaving
        // streaming on production (stage has no streaming cluster).
        let _guard = env_test_guard();
        clear_env_matrix();
        let prod_stream = StreamingConfig::production_defaults().hosts;
        // production() -> with_historical_environment(Stage) must equal stage()
        // exactly, and round-tripping back to Prod must restore the prod host.
        let staged =
            DirectConfig::production().with_historical_environment(HistoricalEnvironment::Stage);
        assert_eq!(staged.historical_host(), "mdds-stage.thetadata.us");
        assert_eq!(
            staged.streaming_hosts(),
            prod_stream.as_slice(),
            "a historical switch leaves streaming on production"
        );
        let back = staged.with_historical_environment(HistoricalEnvironment::Prod);
        assert_eq!(back.historical_host(), "mdds-01.thetadata.us");
        assert_eq!(back.streaming_hosts(), prod_stream.as_slice());
        // Repeated no-op switches keep the prod cluster.
        let still_prod = back.with_historical_environment(HistoricalEnvironment::Prod);
        assert_eq!(still_prod.historical_host(), "mdds-01.thetadata.us");
        assert_eq!(still_prod.streaming_hosts(), prod_stream.as_slice());
    }

    #[test]
    fn explicit_streaming_host_survives_stage_preset() {
        // Companion to 2(a) for the streaming channel: an explicit
        // `THETADATA_STREAMING_HOST` survives `stage()` in the primary slot.
        // `stage()` only flips the historical channel, so the streaming failover
        // hosts stay the PRODUCTION cluster's while the historical host moves to
        // the stage host.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_STREAMING_HOST, "stream.example.com");
        }
        let config = DirectConfig::stage();
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(
            config.streaming.hosts[0].0, "stream.example.com",
            "explicit streaming host must survive stage()"
        );
        assert_eq!(
            &config.streaming.hosts[1..],
            &StreamingConfig::production_defaults().hosts[1..],
            "streaming failover stays on the production cluster under stage()"
        );
        assert_eq!(
            config.historical.host,
            HistoricalEnvironment::Stage.host(),
            "non-overridden historical host must move to the stage cluster"
        );
        clear_env_matrix();
    }

    #[test]
    fn presets_unchanged_without_any_override() {
        // Finding 2(c): with no override in the environment, the plain
        // presets must produce exactly today's hosts — the override-tracking
        // change must be invisible on the common paths.
        let _guard = env_test_guard();
        clear_env_matrix();

        // Pin the FULL host vectors as literals (not via the source-of-truth
        // helpers) so this stays a hard regression guard: the override-model
        // rework must leave every no-override preset byte-identical to today.
        // Production streaming cluster, reused below: stage leaves streaming
        // here since there is no streaming staging cluster.
        let prod_stream = vec![
            ("nj-a.thetadata.us".to_string(), 20000),
            ("nj-a.thetadata.us".to_string(), 20001),
            ("nj-b.thetadata.us".to_string(), 20000),
            ("nj-b.thetadata.us".to_string(), 20001),
        ];

        let prod = DirectConfig::production();
        assert_eq!(prod.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(prod.streaming_environment, StreamingEnvironment::Prod);
        assert_eq!(prod.historical.host, "mdds-01.thetadata.us");
        assert_eq!(prod.streaming.hosts, prod_stream);

        let stage = DirectConfig::stage();
        // Stage flips only the historical channel; streaming stays on the
        // production cluster (no streaming staging).
        assert_eq!(stage.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(stage.streaming_environment, StreamingEnvironment::Prod);
        assert_eq!(stage.historical.host, "mdds-stage.thetadata.us");
        assert_eq!(stage.streaming.hosts, prod_stream);

        let dev = DirectConfig::dev();
        // Dev flips only the streaming channel; historical still uses the prod
        // host, and only the streaming hosts switch to the dev replay cluster.
        assert_eq!(dev.streaming_environment, StreamingEnvironment::Dev);
        assert_eq!(dev.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(dev.historical.host, "mdds-01.thetadata.us");
        assert_eq!(
            dev.streaming.hosts,
            vec![
                ("nj-a.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20201),
            ]
        );

        clear_env_matrix();
    }

    #[test]
    fn streaming_host_override_keeps_environment_failover() {
        // `THETADATA_STREAMING_HOST` patches only the PRIMARY slot of the
        // SELECTED streaming environment; the environment's own failover hosts
        // stay in place. Exercised against the dev cluster (the only non-prod
        // streaming environment) so the failover hosts are distinct from
        // production: the override must not suppress the whole host-vector
        // rewrite.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_STREAMING_HOST, "myhost");
        }
        let config = DirectConfig::dev();
        // Primary patched; failover hosts are the dev cluster's, not prod's.
        assert_eq!(
            config.streaming.hosts,
            vec![
                ("myhost".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20201),
            ]
        );
        assert_eq!(config.streaming.hosts[0].0, "myhost");
        assert_eq!(
            &config.streaming.hosts[1..],
            &DirectConfig::dev_streaming_hosts()[1..],
            "failover must track the dev cluster, not production"
        );
        clear_env_matrix();
    }

    #[test]
    fn streaming_port_only_override_keeps_environment_host_cluster() {
        // A port-only `THETADATA_STREAMING_PORT` (host NOT set) must patch
        // ONLY the primary port of the selected streaming environment; the host
        // cluster stays the environment's. Exercised against the dev cluster (the
        // only non-prod streaming environment). Previously a port-only override
        // suppressed the host rebuild entirely.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_STREAMING_PORT, "9999");
        }
        let config = DirectConfig::dev();
        assert_eq!(
            config.streaming.hosts,
            vec![
                ("nj-a.thetadata.us".to_string(), 9999),
                ("test-server.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20201),
            ],
            "only the primary port is patched; the host cluster stays dev"
        );
        // The host cluster is unchanged from dev; only the primary port moved.
        let dev_hosts = DirectConfig::dev_streaming_hosts();
        assert_eq!(config.streaming.hosts[0].0, dev_hosts[0].0);
        assert_eq!(config.streaming.hosts[0].1, 9999);
        assert_eq!(&config.streaming.hosts[1..], &dev_hosts[1..]);
        clear_env_matrix();
    }

    #[test]
    fn streaming_host_override_persists_across_environment_switches() {
        // A recorded `THETADATA_STREAMING_HOST` must survive repeated
        // `with_streaming_environment` switches, and the failover must always
        // track the CURRENT streaming environment (dev failover under Dev, prod
        // failover under Prod) while the overridden primary host persists.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_STREAMING_HOST, "sticky.example.com");
        }
        let dev = DirectConfig::production().with_streaming_environment(StreamingEnvironment::Dev);
        assert_eq!(dev.streaming.hosts[0].0, "sticky.example.com");
        assert_eq!(
            &dev.streaming.hosts[1..],
            &DirectConfig::dev_streaming_hosts()[1..],
            "failover tracks the dev cluster after the first switch"
        );
        let back = dev.with_streaming_environment(StreamingEnvironment::Prod);
        assert_eq!(
            back.streaming.hosts[0].0, "sticky.example.com",
            "the primary override persists across the switch"
        );
        assert_eq!(
            &back.streaming.hosts[1..],
            &StreamingConfig::production_defaults().hosts[1..],
            "failover now tracks the production cluster"
        );
        clear_env_matrix();
    }

    #[test]
    fn historical_host_override_with_dev_keeps_dev_streaming() {
        // `THETADATA_HISTORICAL_HOST` must survive `dev()` on the historical
        // channel while the streaming channel stays on the dev replay hosts.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_HOST, "hist.example.com");
        }
        let config = DirectConfig::dev();
        assert_eq!(config.streaming_environment, StreamingEnvironment::Dev);
        assert_eq!(
            config.historical.host, "hist.example.com",
            "explicit historical host must survive dev()"
        );
        assert_eq!(
            config.streaming.hosts,
            DirectConfig::dev_streaming_hosts(),
            "streaming stays on the dev replay cluster"
        );
        clear_env_matrix();
    }

    #[test]
    fn streaming_host_override_survives_dev_primary_slot() {
        // Companion: a streaming primary override must also survive `dev()`,
        // patching the dev primary while keeping the dev failover hosts —
        // `dev()` no longer clobbers the override.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_STREAMING_HOST, "devstream.example.com");
        }
        let config = DirectConfig::dev();
        let dev_hosts = DirectConfig::dev_streaming_hosts();
        assert_eq!(config.streaming.hosts[0].0, "devstream.example.com");
        assert_eq!(config.streaming.hosts[0].1, dev_hosts[0].1);
        assert_eq!(&config.streaming.hosts[1..], &dev_hosts[1..]);
        clear_env_matrix();
    }

    #[test]
    fn dev_with_primary_override_then_prod_patches_prod_not_dev_failover() {
        // `dev()` carries a recorded primary streaming override (env-var), then
        // `with_streaming_environment(Prod)`: the primary patch is a real caller
        // override and must survive, but it must patch the PRODUCTION cluster's
        // primary slot — the dev replay hosts are a preset base and must NOT pin
        // the failover. The override tracks the selected streaming environment,
        // never the dev preset.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_STREAMING_HOST, "sticky.example.com");
        }
        let prod = DirectConfig::dev().with_streaming_environment(StreamingEnvironment::Prod);
        assert_eq!(prod.streaming_environment, StreamingEnvironment::Prod);
        let prod_hosts = StreamingConfig::production_defaults().hosts;
        // Primary host patched, primary port is PROD's (not dev 20200).
        assert_eq!(prod.streaming_hosts()[0].0, "sticky.example.com");
        assert_eq!(prod.streaming_hosts()[0].1, prod_hosts[0].1);
        // Failover hosts are PROD's, never the dev replay cluster.
        assert_eq!(
            &prod.streaming_hosts()[1..],
            &prod_hosts[1..],
            "failover must track the production cluster, not the dev replay hosts"
        );
        // Historical is untouched by a streaming switch and stays on production.
        assert_eq!(prod.historical_host(), "mdds-01.thetadata.us");
        clear_env_matrix();
    }

    #[test]
    fn dev_then_historical_stage_moves_historical_and_keeps_dev_streaming() {
        // The channels are independent: the `dev()` preset selects the dev replay
        // streaming cluster while leaving historical on production. A later
        // `with_historical_environment(Stage)` must move ONLY the historical
        // channel to staging — the dev streaming cluster must be untouched, a
        // genuine split config.
        let _guard = env_test_guard();
        clear_env_matrix();
        let split = DirectConfig::dev().with_historical_environment(HistoricalEnvironment::Stage);
        assert_eq!(split.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(split.streaming_environment, StreamingEnvironment::Dev);
        assert_eq!(
            split.historical.host, "mdds-stage.thetadata.us",
            "historical must move to the stage cluster"
        );
        assert_eq!(
            split.streaming.hosts,
            DirectConfig::dev_streaming_hosts(),
            "streaming must stay on the dev replay cluster — the channels are independent"
        );
    }

    #[test]
    fn dev_then_streaming_production_discards_dev_replay_hosts() {
        // The round-trip companion: `dev().with_streaming_environment(Prod)` must
        // yield the full production streaming cluster, with the dev replay hosts
        // discarded (they are a preset base, not a caller override). Historical
        // stays on production throughout.
        let _guard = env_test_guard();
        clear_env_matrix();
        let prod = DirectConfig::dev().with_streaming_environment(StreamingEnvironment::Prod);
        assert_eq!(prod.streaming_environment, StreamingEnvironment::Prod);
        assert_eq!(prod.historical.host, "mdds-01.thetadata.us");
        assert_eq!(
            prod.streaming.hosts,
            StreamingConfig::production_defaults().hosts,
            "streaming must be the FULL production cluster, not the dev replay hosts"
        );
        // And it equals a plain production() — the dev detour leaves no trace.
        assert_eq!(
            prod.streaming.hosts,
            DirectConfig::production().streaming.hosts
        );
    }

    #[test]
    fn latest_set_historical_host_wins_over_recorded_env_override() {
        // An override recorded up front (here via the process env), THEN a
        // later `set_historical_host`, THEN a historical environment switch: the
        // most recent setter must win over the stale env override.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_HOST, "recorded.example.com");
        }
        let mut config = DirectConfig::production();
        assert_eq!(config.historical_host(), "recorded.example.com");
        // Tracked setter AFTER the env override was recorded + applied.
        config.set_historical_host("field-edited.example.com");
        let staged = config.with_historical_environment(HistoricalEnvironment::Stage);
        assert_eq!(
            staged.historical_host(),
            "field-edited.example.com",
            "the latest set_historical_host must win over the stale env override"
        );
        assert_eq!(staged.historical_environment, HistoricalEnvironment::Stage);
        clear_env_matrix();
    }

    #[test]
    fn latest_set_streaming_hosts_wins_over_recorded_env_override() {
        // Streaming companion to the historical recency test: a recorded
        // primary override, then a later `set_streaming_hosts`, then a switch —
        // the latest full list wins and the stale primary patch is dropped.
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: see `historical_type_env_stage_selects_stage_cluster`.
        unsafe {
            std::env::set_var(ENV_STREAMING_HOST, "recorded-stream.example.com");
        }
        let mut config = DirectConfig::production();
        assert_eq!(config.streaming_hosts()[0].0, "recorded-stream.example.com");
        // Tracked setter AFTER the primary override was recorded + applied.
        let edited = vec![
            ("field-stream-a.example.com".to_string(), 7000),
            ("field-stream-b.example.com".to_string(), 7001),
        ];
        config.set_streaming_hosts(edited.clone());
        let switched = config.with_streaming_environment(StreamingEnvironment::Dev);
        assert_eq!(
            switched.streaming_hosts(),
            edited,
            "the latest set_streaming_hosts must win over the stale primary override"
        );
        assert_eq!(switched.streaming_environment, StreamingEnvironment::Dev);
        clear_env_matrix();
    }

    /// Finding #4: a primary streaming override recorded AFTER a full
    /// host-list override must take effect (newest wins). The full list
    /// supplies the host set; a later `THETADATA_STREAMING_HOST` /
    /// `THETADATA_STREAMING_PORT` (here via `.env`) re-points the primary
    /// slot on top of it, keeping the full list's failover hosts. Before
    /// the fix `resolve_streaming_hosts` returned the full list outright,
    /// silently swallowing the later primary override.
    #[test]
    fn primary_override_after_full_override_wins_newest() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // Full power-user list set first (the config-file `[streaming]
        // hosts` / `set_streaming_hosts` tier).
        let full = vec![
            ("full-primary.example.com".to_string(), 6000),
            ("full-failover-a.example.com".to_string(), 6001),
            ("full-failover-b.example.com".to_string(), 6002),
        ];
        let mut config = DirectConfig::production();
        config.set_streaming_hosts(full.clone());
        assert_eq!(
            config.streaming_hosts(),
            full,
            "the full list applies before the later primary override"
        );

        // A later primary override via `.env` (host + port). No
        // THETADATA_HISTORICAL_TYPE, so the environment marker is unchanged.
        let path = write_temp_dotenv(
            "primary-after-full.env",
            "THETADATA_STREAMING_HOST=later-primary.example.com\n\
             THETADATA_STREAMING_PORT=6543\n",
        );
        let config = config
            .with_dotenv(&path)
            .expect(".env must layer onto the full override");

        // Newest wins: the primary slot is re-pointed by the later
        // override; the full list's failover hosts are preserved.
        assert_eq!(
            config.streaming_hosts(),
            vec![
                ("later-primary.example.com".to_string(), 6543),
                ("full-failover-a.example.com".to_string(), 6001),
                ("full-failover-b.example.com".to_string(), 6002),
            ],
            "a primary override set after a full override must patch the full \
             list's primary slot (newest wins), not be swallowed by it"
        );
        std::fs::remove_file(&path).ok();
        clear_env_matrix();
    }

    /// Finding #4 (reverse order): a full override recorded AFTER a
    /// primary override still wins outright. The `.env` primary override
    /// is recorded first, then `set_streaming_hosts` supersedes it — the
    /// full list replaces the host set and the stale primary patch is
    /// dropped. Complements the env-var-ordered test above; together they
    /// pin most-recent-wins in both directions.
    #[test]
    fn full_override_after_primary_override_still_wins() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // Primary override recorded first via `.env`.
        let path = write_temp_dotenv(
            "primary-before-full.env",
            "THETADATA_STREAMING_HOST=early-primary.example.com\n\
             THETADATA_STREAMING_PORT=7001\n",
        );
        let mut config = DirectConfig::production()
            .with_dotenv(&path)
            .expect(".env must source the primary override");
        assert_eq!(config.streaming_hosts()[0].0, "early-primary.example.com");
        assert_eq!(config.streaming_hosts()[0].1, 7001);

        // A later full list supersedes the primary override entirely.
        let full = vec![
            ("late-full-primary.example.com".to_string(), 8000),
            ("late-full-failover.example.com".to_string(), 8001),
        ];
        config.set_streaming_hosts(full.clone());
        assert_eq!(
            config.streaming_hosts(),
            full,
            "the later full override must win outright, dropping the stale \
             primary patch (its port 7001 must not survive)"
        );
        std::fs::remove_file(&path).ok();
        clear_env_matrix();
    }

    #[test]
    fn with_historical_environment_stage_equals_stage_preset() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let via_builder =
            DirectConfig::production().with_historical_environment(HistoricalEnvironment::Stage);
        let via_preset = DirectConfig::stage();
        assert_eq!(
            via_builder.historical_environment,
            via_preset.historical_environment
        );
        assert_eq!(
            via_builder.streaming_environment,
            via_preset.streaming_environment
        );
        assert_eq!(via_builder.historical.host, via_preset.historical.host);
        assert_eq!(via_builder.streaming.hosts, via_preset.streaming.hosts);
    }

    #[test]
    fn with_historical_environment_round_trips_prod_and_stage() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let staged =
            DirectConfig::production().with_historical_environment(HistoricalEnvironment::Stage);
        assert_eq!(staged.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(staged.historical.host, "mdds-stage.thetadata.us");
        // Switching back to Prod restores the production historical host.
        let back = staged.with_historical_environment(HistoricalEnvironment::Prod);
        assert_eq!(back.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(back.historical.host, "mdds-01.thetadata.us");
        // Streaming was never touched and stays on the production cluster.
        assert_eq!(back.streaming.hosts.len(), 4);
    }

    #[test]
    fn environment_getters_read_back_the_selected_clusters() {
        // The readback getters mirrored across the bindings: `stage()` carries
        // historical `Stage` (streaming stays `Prod`), `dev()` carries streaming
        // `Dev` (historical stays `Prod`), `production()` carries `Prod` on both.
        let _guard = env_test_guard();
        clear_env_matrix();
        assert_eq!(
            DirectConfig::stage().historical_environment(),
            HistoricalEnvironment::Stage
        );
        assert_eq!(
            DirectConfig::stage().streaming_environment(),
            StreamingEnvironment::Prod
        );
        assert_eq!(
            DirectConfig::dev().streaming_environment(),
            StreamingEnvironment::Dev
        );
        assert_eq!(
            DirectConfig::dev().historical_environment(),
            HistoricalEnvironment::Prod
        );
        assert_eq!(
            DirectConfig::production().historical_environment(),
            HistoricalEnvironment::Prod
        );
        assert_eq!(
            DirectConfig::production()
                .with_historical_environment(HistoricalEnvironment::Stage)
                .historical_environment(),
            HistoricalEnvironment::Stage
        );
        clear_env_matrix();
    }

    #[test]
    fn env_overrides_apply_on_production() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: `_guard` holds the process-global env-var mutex for
        // the body of this test, so no other thread observes or mutates
        // the environment while these writes land. `std::env::set_var`'s
        // 1.88 `unsafe fn` contract is therefore upheld.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_HOST, "historical.staging.example.com");
            std::env::set_var(ENV_HISTORICAL_PORT, "8443");
            std::env::set_var(ENV_NEXUS_URL, "https://nexus.staging.example.com/auth");
            std::env::set_var(ENV_CLIENT_TYPE, "rust-thetadatadx-staging");
            std::env::set_var(ENV_STREAMING_HOST, "streaming.staging.example.com");
            std::env::set_var(ENV_STREAMING_PORT, "21000");
        }
        let config = DirectConfig::production();
        assert_eq!(config.historical.host, "historical.staging.example.com");
        assert_eq!(config.historical.port, 8443);
        assert_eq!(
            config.auth.nexus_url,
            "https://nexus.staging.example.com/auth"
        );
        assert_eq!(config.auth.client_type, "rust-thetadatadx-staging");
        assert_eq!(
            config.streaming.hosts[0],
            ("streaming.staging.example.com".to_string(), 21000)
        );
        clear_env_matrix();
    }

    #[test]
    fn builder_takes_precedence_over_env_var() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: `_guard` holds the process-global env-var mutex for
        // the body of this test, so no other thread observes or mutates
        // the environment while this write lands.
        unsafe {
            std::env::set_var(ENV_CLIENT_TYPE, "env-wins-when-no-builder");
        }
        let config = DirectConfig::production().with_client_type("builder-wins");
        assert_eq!(config.auth.client_type, "builder-wins");
        clear_env_matrix();
    }

    #[test]
    fn env_overrides_skipped_when_values_malformed() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: `_guard` holds the process-global env-var mutex for
        // the body of this test, so no other thread observes or mutates
        // the environment while these writes land.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_PORT, "not-a-port");
            std::env::set_var(ENV_STREAMING_PORT, "0"); // reject zero
            std::env::set_var(ENV_HISTORICAL_HOST, "   "); // whitespace-only
        }
        let config = DirectConfig::production();
        let defaults = DirectConfig::production_defaults();
        assert_eq!(config.historical.host, defaults.historical.host);
        assert_eq!(config.historical.port, defaults.historical.port);
        assert_eq!(config.streaming.hosts[0].1, defaults.streaming.hosts[0].1);
        clear_env_matrix();
    }

    /// Write `body` to a uniquely-named temp `.env` file and return its path.
    /// The unique suffix keeps parallel test threads from colliding.
    fn write_temp_dotenv(suffix: &str, body: &str) -> std::path::PathBuf {
        use std::io::Write as _;
        let path = std::env::temp_dir().join(format!(
            "thetadatadx-config-dotenv-{}-{suffix}",
            std::process::id()
        ));
        let mut f = std::fs::File::create(&path).expect("create tmp .env file");
        f.write_all(body.as_bytes()).expect("write tmp .env file");
        path
    }

    #[test]
    fn from_dotenv_historical_type_stage_selects_stage_cluster() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let path = write_temp_dotenv(
            "stage.env",
            "# select staging\nTHETADATA_HISTORICAL_TYPE=STAGE\n",
        );
        let config = DirectConfig::from_dotenv(&path).expect(".env mdds-type must source");
        let staged = DirectConfig::stage();
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(config.historical.host, staged.historical.host);
        assert_eq!(config.streaming.hosts, staged.streaming.hosts);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_historical_type_is_case_insensitive_and_quoted() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let path = write_temp_dotenv("ci.env", "export THETADATA_HISTORICAL_TYPE=\"stage\"\n");
        let config = DirectConfig::from_dotenv(&path).expect(".env mdds-type must source");
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_explicit_historical_host_wins_over_historical_type() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let path = write_temp_dotenv(
            "hostwins.env",
            "THETADATA_HISTORICAL_TYPE=STAGE\nTHETADATA_HISTORICAL_HOST=custom.example.com\n",
        );
        let config = DirectConfig::from_dotenv(&path).expect(".env must source");
        // The historical marker still flips to Stage, but an explicit host
        // override wins over the environment's default cluster host.
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(config.historical.host, "custom.example.com");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_with_only_api_key_yields_production() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // A `.env` carrying only a credential key is valid for the config
        // reader: it picks up no cluster keys and returns the prod default.
        let path = write_temp_dotenv("apikeyonly.env", "THETADATA_API_KEY=td_example_key\n");
        let config = DirectConfig::from_dotenv(&path).expect("api-key-only .env must source");
        assert_eq!(config.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(config.streaming_environment, StreamingEnvironment::Prod);
        assert_eq!(config.historical.host, "mdds-01.thetadata.us");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_malformed_historical_type_returns_error() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // FAIL LOUD: an unrecognized mdds-type in a `.env` is a returned error
        // naming the valid set, never a silent fallback to production.
        let path = write_temp_dotenv("bogus.env", "THETADATA_HISTORICAL_TYPE=bogus\n");
        let err = DirectConfig::from_dotenv(&path)
            .expect_err("an unrecognized THETADATA_HISTORICAL_TYPE must return an error");
        assert!(
            err.to_string().contains("THETADATA_HISTORICAL_TYPE"),
            "the error must name the offending selector, got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_cross_channel_streaming_type_returns_error() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // The streaming companion: a cross-channel `THETADATA_STREAMING_TYPE=STAGE`
        // (STAGE is historical-only) is a returned error, never a silent
        // fallback.
        let path = write_temp_dotenv("fpss-stage.env", "THETADATA_STREAMING_TYPE=STAGE\n");
        let err = DirectConfig::from_dotenv(&path)
            .expect_err("a cross-channel THETADATA_STREAMING_TYPE must return an error");
        assert!(
            err.to_string().contains("THETADATA_STREAMING_TYPE"),
            "the error must name the offending selector, got: {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_streaming_host_overrides_primary_slot() {
        let _guard = env_test_guard();
        clear_env_matrix();
        let path = write_temp_dotenv(
            "streamhost.env",
            "THETADATA_HISTORICAL_TYPE=STAGE\nTHETADATA_STREAMING_HOST=stream.example.com\n",
        );
        let config = DirectConfig::from_dotenv(&path).expect(".env must source");
        assert_eq!(config.streaming.hosts[0].0, "stream.example.com");
        // THETADATA_HISTORICAL_TYPE=STAGE flips only historical; streaming stays on production,
        // so the production failover hosts surround the overridden primary.
        assert_eq!(
            &config.streaming.hosts[1..],
            &StreamingConfig::production_defaults().hosts[1..]
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_streaming_port_only_patches_primary_port() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // A `.env` carrying only the streaming PORT (no host) must patch the
        // selected streaming environment's primary port and keep its host
        // cluster — the `.env` path mirrors the process-env override model.
        // THETADATA_HISTORICAL_TYPE=STAGE flips only historical, so streaming stays production.
        let path = write_temp_dotenv(
            "streamport.env",
            "THETADATA_HISTORICAL_TYPE=STAGE\nTHETADATA_STREAMING_PORT=9999\n",
        );
        let config = DirectConfig::from_dotenv(&path).expect(".env must source");
        let prod_hosts = StreamingConfig::production_defaults().hosts;
        assert_eq!(config.streaming.hosts[0].0, prod_hosts[0].0);
        assert_eq!(config.streaming.hosts[0].1, 9999);
        assert_eq!(&config.streaming.hosts[1..], &prod_hosts[1..]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_nexus_url_with_stage_routes_auth_to_staging_no_split_cluster() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // An operator ships a `.env` that redirects the historical channel to
        // staging AND supplies a staging Nexus URL. Auth must follow the
        // historical cluster: the historical channel goes to the staging host
        // and auth POSTs to the staging Nexus. Streaming stays on production
        // (no streaming staging cluster).
        let staging_nexus = "https://nexus-stage.thetadata.us/identity/terminal/auth_user";
        let path = write_temp_dotenv(
            "nexus-stage.env",
            &format!(
                "THETADATA_API_KEY=td_example_key\n\
                 THETADATA_HISTORICAL_TYPE=STAGE\n\
                 THETADATA_NEXUS_URL={staging_nexus}\n"
            ),
        );
        let config = DirectConfig::from_dotenv(&path).expect(".env must source");
        let staged = DirectConfig::stage();
        // Cluster: historical on staging, streaming on production.
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(config.historical.host, staged.historical.host);
        assert_eq!(config.streaming.hosts, staged.streaming.hosts);
        // Auth: the staging Nexus URL is honoured from the SAME `.env`, so auth
        // does not keep POSTing to production while the data channels moved.
        assert_eq!(config.auth.nexus_url, staging_nexus);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_honors_historical_port_and_client_type() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // `THETADATA_HISTORICAL_PORT` and `THETADATA_CLIENT_TYPE` must be read
        // from a `.env` exactly as the process-env path reads them — the `.env`
        // path honours the same key set.
        let path = write_temp_dotenv(
            "histport.env",
            "THETADATA_HISTORICAL_PORT=8443\nTHETADATA_CLIENT_TYPE=rust-thetadatadx-fleet\n",
        );
        let config = DirectConfig::from_dotenv(&path).expect(".env must source");
        assert_eq!(config.historical.port, 8443);
        assert_eq!(config.auth.client_type, "rust-thetadatadx-fleet");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_rejects_malformed_historical_port() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // A malformed / zero historical port from a `.env` is skipped leniently
        // (logged), keeping the hardcoded default — matching the process-env
        // path's `>0` + `parse::<u16>()` guard.
        let path = write_temp_dotenv("badport.env", "THETADATA_HISTORICAL_PORT=not-a-port\n");
        let config = DirectConfig::from_dotenv(&path).expect("malformed port must not error");
        assert_eq!(
            config.historical.port,
            DirectConfig::production_defaults().historical.port
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn dev_with_dotenv_host_port_override_patches_dev_cluster() {
        // A `.env` host/port override layered onto a `dev()` config must
        // patch the DEV streaming cluster's primary slot and keep the dev
        // failover hosts — `with_dotenv` re-runs the override layer, which
        // used to rebuild streaming from the prod base and drop the dev
        // cluster. The dev environment marker is preserved.
        let _guard = env_test_guard();
        clear_env_matrix();
        let path = write_temp_dotenv(
            "dev-override.env",
            "THETADATA_STREAMING_HOST=dev-primary.example.com\nTHETADATA_STREAMING_PORT=4242\n\
             THETADATA_HISTORICAL_HOST=dev-hist.example.com\n",
        );
        let config = DirectConfig::dev()
            .with_dotenv(&path)
            .expect(".env must layer onto dev()");
        let dev_hosts = DirectConfig::dev_streaming_hosts();
        // Streaming marker stays dev; no `.env` THETADATA_STREAMING_TYPE present.
        assert_eq!(config.streaming_environment, StreamingEnvironment::Dev);
        // Historical override applied.
        assert_eq!(config.historical.host, "dev-hist.example.com");
        // Primary streaming slot patched (host AND port); failover hosts
        // remain the DEV cluster's, not production's.
        assert_eq!(config.streaming.hosts[0].0, "dev-primary.example.com");
        assert_eq!(config.streaming.hosts[0].1, 4242);
        assert_eq!(
            &config.streaming.hosts[1..],
            &dev_hosts[1..],
            "the dev streaming failover must be preserved, not reverted to production"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_streaming_type_dev_selects_dev_cluster() {
        // `THETADATA_STREAMING_TYPE=DEV` in a `.env` selects the dev streaming
        // environment: streaming on the dev replay cluster, historical on
        // production.
        let _guard = env_test_guard();
        clear_env_matrix();
        let path = write_temp_dotenv("devtype.env", "THETADATA_STREAMING_TYPE=dev\n");
        let config = DirectConfig::from_dotenv(&path).expect(".env fpss-type must source");
        let dev = DirectConfig::dev();
        assert_eq!(config.streaming_environment, StreamingEnvironment::Dev);
        assert_eq!(config.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(config.historical.host, dev.historical.host);
        assert_eq!(config.streaming.hosts, dev.streaming.hosts);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_prod_ignores_ambient_stage_env() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // The shell forces STAGE and a stage historical host, but a file-sourced
        // config must be deterministic from defaults plus the file. A `.env`
        // selecting PROD must yield the prod cluster regardless of the ambient
        // process env.
        // SAFETY: `_guard` holds the process-global env-var mutex for the body
        // of this test, so no other thread observes or mutates the environment
        // while these writes land.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_TYPE, "STAGE");
            std::env::set_var(ENV_HISTORICAL_HOST, "mdds-stage.thetadata.us");
        }
        let path = write_temp_dotenv("prodfile.env", "THETADATA_HISTORICAL_TYPE=PROD\n");
        let config = DirectConfig::from_dotenv(&path).expect(".env mdds-type must source");
        // The file says PROD, so the prod cluster wins over the ambient STAGE.
        assert_eq!(config.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(config.historical.host, "mdds-01.thetadata.us");
        let prod_defaults = DirectConfig::production_defaults();
        assert_eq!(config.streaming.hosts, prod_defaults.streaming.hosts);
        clear_env_matrix();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn from_dotenv_only_api_key_ignores_ambient_stage_env() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // A `.env` that carries no cluster keys must still ignore the ambient
        // env: it sources from defaults plus the file, never the shell.
        // SAFETY: see `from_dotenv_prod_ignores_ambient_stage_env`.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_TYPE, "STAGE");
            std::env::set_var(ENV_HISTORICAL_HOST, "mdds-stage.thetadata.us");
        }
        let path = write_temp_dotenv("nocluster.env", "THETADATA_API_KEY=td_example_key\n");
        let config = DirectConfig::from_dotenv(&path).expect("api-key-only .env must source");
        assert_eq!(config.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(config.historical.host, "mdds-01.thetadata.us");
        clear_env_matrix();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn dev_selects_streaming_dev_independently_of_ambient_mdds_stage() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // The shell forces THETADATA_HISTORICAL_TYPE=STAGE, which `production()` honors on the
        // historical channel. `dev()` then selects ONLY the streaming dev
        // cluster — the two channels are independent, so historical stays on the
        // ambient staging selection while streaming routes to the dev replay
        // cluster.
        // SAFETY: see `from_dotenv_prod_ignores_ambient_stage_env`.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_TYPE, "STAGE");
        }
        let config = DirectConfig::dev();
        assert_eq!(config.streaming_environment, StreamingEnvironment::Dev);
        // Historical follows the ambient THETADATA_HISTORICAL_TYPE=STAGE; dev does not touch it.
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(config.historical.host, "mdds-stage.thetadata.us");
        // Dev streaming hosts (port 20200/20201).
        assert_eq!(config.streaming.hosts[0].0, "nj-a.thetadata.us");
        assert_eq!(config.streaming.hosts[0].1, 20200);
        clear_env_matrix();
    }

    #[test]
    fn stage_stays_stage_under_ambient_prod_env() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // An explicit `stage()` preset must end up fully Stage even when the
        // ambient env says PROD: the preset is not silently overridden into an
        // inconsistent state.
        // SAFETY: see `from_dotenv_prod_ignores_ambient_stage_env`.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_TYPE, "PROD");
        }
        let config = DirectConfig::stage();
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(config.historical.host, "mdds-stage.thetadata.us");
        clear_env_matrix();
    }

    #[test]
    fn from_dotenv_errors_on_missing_file() {
        let err = DirectConfig::from_dotenv("/nonexistent/dir/.env").unwrap_err();
        assert!(err.to_string().contains(".env file unreadable"));
    }

    #[test]
    fn with_dotenv_layers_onto_existing_config() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // `with_dotenv` keeps tuning knobs the caller already set and only
        // layers the `.env`'s cluster overrides on top.
        let path = write_temp_dotenv("layer.env", "THETADATA_HISTORICAL_TYPE=STAGE\n");
        let base = DirectConfig::production().with_metrics_port(9100);
        let config = base.with_dotenv(&path).expect(".env must layer cleanly");
        assert_eq!(config.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(config.metrics.port, Some(9100));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn production_defaults_are_not_sensitive_to_env() {
        let _guard = env_test_guard();
        clear_env_matrix();
        // SAFETY: `_guard` holds the process-global env-var mutex for
        // the body of this test, so no other thread observes or mutates
        // the environment while these writes land.
        unsafe {
            std::env::set_var(ENV_HISTORICAL_HOST, "ignored-by-defaults");
            std::env::set_var(ENV_HISTORICAL_PORT, "9999");
        }
        let config = DirectConfig::production_defaults();
        assert_eq!(config.historical.host, "mdds-01.thetadata.us");
        assert_eq!(config.historical.port, 443);
        clear_env_matrix();
    }
}
