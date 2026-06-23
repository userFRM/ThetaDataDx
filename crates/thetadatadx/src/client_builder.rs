//! Fluent client builder: the headline ergonomic for constructing a
//! [`Client`] with the API key (or email + password) and the target
//! environment selected inline at the client.
//!
//! The API key is a first-class, directly-passed argument here — it has
//! its own setters ([`ClientBuilder::api_key`],
//! [`ClientBuilder::api_key_from_env`],
//! [`ClientBuilder::api_key_from_dotenv`]) distinct from the email +
//! password pair ([`ClientBuilder::email_password`]). The lower-level
//! typed path ([`Client::connect`] with a pre-built [`Credentials`] +
//! [`DirectConfig`]) stays available for power users; this builder
//! composes the same two values internally and calls it.
//!
//! ```rust,no_run
//! use thetadatadx::Client;
//!
//! # async fn doc() -> Result<(), thetadatadx::Error> {
//! // API key inline, staging environment, one fluent chain.
//! let client = Client::builder()
//!     .api_key("td1_example_key")
//!     .stage()
//!     .connect()
//!     .await?;
//! # Ok(()) }
//! ```
//!
//! Exactly one authentication source must be set. Setting none, or two
//! different ones (for example an API key AND an email + password),
//! returns a clear error from [`ClientBuilder::connect`] before any
//! network round-trip. Secret material supplied to the builder is held in
//! [`zeroize::Zeroizing`] buffers and never appears in the builder's
//! `Debug` output.

use std::path::PathBuf;

use zeroize::Zeroizing;

use crate::auth::Credentials;
use crate::client::Client;
use crate::config::{DirectConfig, HistoricalEnvironment, StreamingEnvironment};
use crate::error::Error;

/// How the builder will source the authentication credential.
///
/// Exactly one variant is selected across the lifetime of a
/// [`ClientBuilder`]. The secret-bearing variants wrap their material in
/// [`Zeroizing`] so a builder dropped before [`ClientBuilder::connect`]
/// still wipes the plaintext.
enum AuthSource {
    /// Inline API key.
    ApiKey { key: Zeroizing<String> },
    /// Source the API key from the `THETADATA_API_KEY` environment
    /// variable, resolved at [`ClientBuilder::connect`] time.
    ApiKeyFromEnv,
    /// Source the credential from a `.env`-format file at connect time.
    /// Reads `THETADATA_API_KEY`, or `THETADATA_EMAIL` +
    /// `THETADATA_PASSWORD`, via [`Credentials::from_dotenv`].
    DotenvFile { path: PathBuf },
    /// Inline email + password.
    EmailPassword {
        email: String,
        password: Zeroizing<String>,
    },
    /// A two-line `creds.txt` file at connect time, via
    /// [`Credentials::from_file`].
    CredentialsFile { path: PathBuf },
    /// A fully pre-built [`Credentials`] value — the escape hatch that
    /// covers every existing factory.
    Prebuilt { credentials: Credentials },
    /// Sentinel recorded when two *different* auth sources were set on
    /// the same builder. Carries the labels of both so
    /// [`ClientBuilder::connect`] can name the conflict. Never resolved
    /// to a credential — [`AuthSource::into_resolved_source`] turns it
    /// into an [`Error::Config`].
    Conflict {
        first: &'static str,
        second: &'static str,
    },
}

impl AuthSource {
    /// Stable label used in the conflict error so the message names both
    /// the source already set and the one that collided.
    fn label(&self) -> &'static str {
        match self {
            AuthSource::ApiKey { .. } => "api_key",
            AuthSource::ApiKeyFromEnv => "api_key_from_env",
            AuthSource::DotenvFile { .. } => "api_key_from_dotenv / from_dotenv",
            AuthSource::EmailPassword { .. } => "email_password",
            AuthSource::CredentialsFile { .. } => "credentials_file",
            AuthSource::Prebuilt { .. } => "credentials",
            AuthSource::Conflict { .. } => "<conflict>",
        }
    }

    /// Resolve the source into a concrete [`Credentials`]. Performed at
    /// [`ClientBuilder::connect`] time so the env / file reads happen
    /// once, immediately before the network round-trip.
    fn resolve(self) -> Result<Credentials, Error> {
        match self {
            AuthSource::ApiKey { key } => Ok(Credentials::api_key(key.as_str())),
            // `THETADATA_API_KEY` is the canonical key variable; an unset
            // or whitespace-only value is a configuration error rather than
            // a silent fallback, because the caller explicitly asked for
            // the env source. `Credentials::from_env` is the strict,
            // no-file-fallback resolver shared with the other bindings.
            AuthSource::ApiKeyFromEnv => Credentials::from_env(),
            AuthSource::DotenvFile { path } => Credentials::from_dotenv(path),
            AuthSource::EmailPassword { email, password } => {
                Ok(Credentials::new(email, password.as_str()))
            }
            AuthSource::CredentialsFile { path } => Credentials::from_file(path),
            AuthSource::Prebuilt { credentials } => Ok(credentials),
            // A conflict is converted to an error by
            // `into_resolved_source` before `resolve` is ever reached, so
            // this arm is defensive only.
            AuthSource::Conflict { first, second } => Err(Error::config_invalid(
                "auth",
                format!("conflicting authentication sources: {first} and {second}"),
            )),
        }
    }
}

/// How the builder will resolve the target [`DirectConfig`].
enum EnvSource {
    /// No environment selected — default to [`DirectConfig::production`].
    Default,
    /// Select per-channel environments on top of [`DirectConfig::production`].
    /// Either channel may be left unset (production); the historical and
    /// streaming channels are chosen independently.
    Environment {
        historical: Option<HistoricalEnvironment>,
        streaming: Option<StreamingEnvironment>,
    },
    /// Use a caller-supplied [`DirectConfig`] verbatim. The config and
    /// environment setters resolve in call order, last one wins: a later
    /// environment setter replaces this config, and this config replaces an
    /// earlier environment selection.
    Config { config: Box<DirectConfig> },
    /// Source the environment from a `.env`-format file at connect time,
    /// via [`DirectConfig::from_dotenv`].
    DotenvFile { path: PathBuf },
}

impl EnvSource {
    /// Resolve the source into a concrete [`DirectConfig`].
    fn resolve(self) -> Result<DirectConfig, Error> {
        match self {
            EnvSource::Default => Ok(DirectConfig::production()),
            EnvSource::Environment {
                historical,
                streaming,
            } => {
                let mut config = DirectConfig::production();
                if let Some(env) = historical {
                    config = config.with_historical_environment(env);
                }
                if let Some(env) = streaming {
                    config = config.with_streaming_environment(env);
                }
                Ok(config)
            }
            EnvSource::Config { config } => Ok(*config),
            EnvSource::DotenvFile { path } => DirectConfig::from_dotenv(path),
        }
    }

    /// Fold a per-channel selection into the current source, preserving any
    /// already-selected channel so `.stage().dev()` composes to
    /// historical-staging + streaming-dev. A `Config` / `DotenvFile` source is
    /// replaced (last-setter-wins on the kind), matching the prior behavior.
    fn with_channel(
        self,
        historical: Option<HistoricalEnvironment>,
        streaming: Option<StreamingEnvironment>,
    ) -> Self {
        let (prev_h, prev_s) = match self {
            EnvSource::Environment {
                historical,
                streaming,
            } => (historical, streaming),
            _ => (None, None),
        };
        EnvSource::Environment {
            historical: historical.or(prev_h),
            streaming: streaming.or(prev_s),
        }
    }
}

/// Fluent builder for [`Client`].
///
/// Construct one with [`Client::builder`], set exactly one authentication
/// source plus an optional environment, then call
/// [`connect`](Self::connect). The module-level documentation describes the
/// full surface and the validation rules.
///
/// The builder deliberately does not derive [`Debug`]; its hand-written
/// impl redacts every secret so a `{:?}` of an in-flight builder cannot
/// leak the API key or password into logs or panic output.
#[must_use = "a ClientBuilder does nothing until `.connect()` is awaited"]
pub struct ClientBuilder {
    auth: Option<AuthSource>,
    env: EnvSource,
}

impl ClientBuilder {
    /// Start a fresh builder. Reached through [`Client::builder`].
    pub(crate) fn new() -> Self {
        Self {
            auth: None,
            env: EnvSource::Default,
        }
    }

    /// Record an auth source, rejecting a second, different one.
    ///
    /// Setting the same kind of source twice overwrites (last writer
    /// wins); setting two *different* sources is a conflict that surfaces
    /// from [`Self::connect`]. We defer the conflict error to `connect`
    /// rather than panic so the fluent chain stays infallible up to the
    /// single terminal `Result`.
    fn set_auth(mut self, source: AuthSource) -> Self {
        self.auth = match self.auth.take() {
            None => Some(source),
            Some(existing) => {
                // Same variant → overwrite silently (re-stating the same
                // intent). Different variant → keep a sentinel that
                // `connect` turns into a conflict error naming both.
                if std::mem::discriminant(&existing) == std::mem::discriminant(&source) {
                    Some(source)
                } else {
                    Some(AuthSource::conflict(existing, source))
                }
            }
        };
        self
    }

    // ─── Authentication setters (the API key is first-class) ──────────

    /// Authenticate with an inline API key — the primary, directly-passed
    /// auth argument.
    pub fn api_key(self, key: impl Into<String>) -> Self {
        self.set_auth(AuthSource::ApiKey {
            key: Zeroizing::new(key.into()),
        })
    }

    /// Source the API key from the `THETADATA_API_KEY` environment
    /// variable, read at [`connect`](Self::connect) time.
    pub fn api_key_from_env(self) -> Self {
        self.set_auth(AuthSource::ApiKeyFromEnv)
    }

    /// Source the API key from a `.env`-format file, read at
    /// [`connect`](Self::connect) time.
    ///
    /// The file uses the common `.env` grammar; `THETADATA_API_KEY`
    /// selects an API key, otherwise `THETADATA_EMAIL` +
    /// `THETADATA_PASSWORD` build email + password credentials. The same
    /// file can carry `THETADATA_MDDS_TYPE` for [`Self::from_dotenv`].
    pub fn api_key_from_dotenv(self, path: impl Into<PathBuf>) -> Self {
        self.set_auth(AuthSource::DotenvFile { path: path.into() })
    }

    /// Authenticate with an inline email + password pair.
    pub fn email_password(self, email: impl Into<String>, password: impl Into<String>) -> Self {
        self.set_auth(AuthSource::EmailPassword {
            email: email.into(),
            password: Zeroizing::new(password.into()),
        })
    }

    /// Authenticate from a two-line `creds.txt` file (line 1 = email,
    /// line 2 = password), read at [`connect`](Self::connect) time.
    pub fn credentials_file(self, path: impl Into<PathBuf>) -> Self {
        self.set_auth(AuthSource::CredentialsFile { path: path.into() })
    }

    /// Authenticate with a pre-built [`Credentials`] value.
    ///
    /// The escape hatch that accepts any credential produced by an
    /// existing [`Credentials`] factory (for example
    /// [`Credentials::from_env_or_file`]), so the builder covers every
    /// sourcing path without a setter per factory.
    pub fn credentials(self, credentials: Credentials) -> Self {
        self.set_auth(AuthSource::Prebuilt { credentials })
    }

    // ─── Environment setters (optional, default production) ───────────

    /// Select the historical (MDDS) [`HistoricalEnvironment`]. Equivalent to
    /// the `THETADATA_MDDS_TYPE` env var and to
    /// [`DirectConfig::with_historical_environment`]. Composes with a streaming
    /// selection — `.streaming_environment(..).historical_environment(..)`
    /// keeps both.
    pub fn historical_environment(mut self, environment: HistoricalEnvironment) -> Self {
        self.env = self.env.with_channel(Some(environment), None);
        self
    }

    /// Select the streaming (FPSS) [`StreamingEnvironment`]. Equivalent to the
    /// `THETADATA_FPSS_TYPE` env var and to
    /// [`DirectConfig::with_streaming_environment`]. Composes with a historical
    /// selection.
    pub fn streaming_environment(mut self, environment: StreamingEnvironment) -> Self {
        self.env = self.env.with_channel(None, Some(environment));
        self
    }

    /// Target the historical staging cluster (streaming stays on production).
    /// Shorthand for `.historical_environment(HistoricalEnvironment::Stage)`;
    /// matches [`DirectConfig::stage`].
    pub fn stage(self) -> Self {
        self.historical_environment(HistoricalEnvironment::Stage)
    }

    /// Target the streaming dev-replay cluster (historical stays on
    /// production). Shorthand for
    /// `.streaming_environment(StreamingEnvironment::Dev)`; matches
    /// [`DirectConfig::dev`].
    pub fn dev(self) -> Self {
        self.streaming_environment(StreamingEnvironment::Dev)
    }

    /// Target production on both channels (the default). Shorthand for
    /// selecting [`HistoricalEnvironment::Prod`] + [`StreamingEnvironment::Prod`].
    pub fn production(self) -> Self {
        self.historical_environment(HistoricalEnvironment::Prod)
            .streaming_environment(StreamingEnvironment::Prod)
    }

    /// Use a fully built [`DirectConfig`] verbatim.
    ///
    /// The config and the environment setters resolve in call order, last
    /// one wins: this config replaces an earlier
    /// [`Self::environment`] / [`Self::stage`] selection, and a later
    /// `environment` / `stage` / `production` call replaces this config.
    pub fn config(mut self, config: DirectConfig) -> Self {
        self.env = EnvSource::Config {
            config: Box::new(config),
        };
        self
    }

    /// Source the target environment from a `.env`-format file.
    ///
    /// Reads `THETADATA_MDDS_TYPE` (and the optional host overrides) via
    /// [`DirectConfig::from_dotenv`]. The same file can carry
    /// `THETADATA_API_KEY` for [`Self::api_key_from_dotenv`], so a single
    /// `.env` can drive both the credential and the environment.
    pub fn from_dotenv(mut self, path: impl Into<PathBuf>) -> Self {
        self.env = EnvSource::DotenvFile { path: path.into() };
        self
    }

    // ─── Terminal ─────────────────────────────────────────────────────

    /// Build the [`Credentials`] + [`DirectConfig`] and connect.
    ///
    /// Validates that exactly one authentication source was set, resolves
    /// any env / file sources, then delegates to [`Client::connect`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when no auth source was set or when two
    /// different sources were set (a conflict), before any network
    /// round-trip. Otherwise returns whatever [`Credentials`] resolution
    /// or [`Client::connect`] returns (network, authentication, parsing,
    /// or config-validation failure).
    pub async fn connect(self) -> Result<Client, Error> {
        let auth = self.auth.ok_or_else(|| {
            Error::config_invalid(
                "auth",
                "no authentication source set — call one of api_key, api_key_from_env, \
                 api_key_from_dotenv, email_password, credentials_file, or credentials",
            )
        })?;
        // Surface a deferred conflict (two different auth sources) as a
        // clear error before resolving anything.
        let auth = auth.into_resolved_source()?;
        let env = self.env;
        // The file-backed sources (`credentials_file`, `api_key_from_dotenv`,
        // `from_dotenv`) read from disk with synchronous `std::fs`. Running
        // that on the async worker would block the runtime thread, so resolve
        // both the credential and the config on a blocking-pool thread. The
        // inline sources (`api_key`, `email_password`, an explicit env)
        // perform no I/O and resolve there for free. Behavior and errors are
        // identical to resolving inline — only the thread differs.
        let (creds, config) = tokio::task::spawn_blocking(move || {
            let creds = auth.resolve()?;
            let config = env.resolve()?;
            Ok::<(Credentials, DirectConfig), Error>((creds, config))
        })
        .await
        .map_err(|e| {
            // The closure never panics on the resolution paths; a join error
            // here means the blocking task was cancelled or the worker
            // panicked, which is an internal invariant violation.
            Error::config_internal(format!("credential resolution task failed to join: {e}"))
        })??;
        Client::connect(&creds, config).await
    }
}

impl AuthSource {
    /// Build the sentinel that records a conflict between two different
    /// auth sources. Carrying both labels lets [`ClientBuilder::connect`]
    /// name exactly what collided.
    fn conflict(first: AuthSource, second: AuthSource) -> AuthSource {
        AuthSource::Conflict {
            first: first.label(),
            second: second.label(),
        }
    }

    /// Turn a recorded source into a resolvable one, or surface a
    /// deferred conflict as an error. A non-conflict source passes
    /// through unchanged.
    fn into_resolved_source(self) -> Result<AuthSource, Error> {
        match self {
            AuthSource::Conflict { first, second } => Err(Error::config_invalid(
                "auth",
                format!(
                    "conflicting authentication sources: {first} and {second} were both set; \
                     set exactly one"
                ),
            )),
            other => Ok(other),
        }
    }
}

impl std::fmt::Debug for ClientBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact every secret-bearing field. The auth source is rendered
        // by its label only (never the key, password, email, or path
        // contents that could carry secrets in a `.env`), and the env
        // source by its kind. A `{:?}` of an in-flight builder must be
        // safe to drop into a log line or a panic message.
        let auth = match &self.auth {
            None => "<unset>",
            Some(AuthSource::Conflict { .. }) => "<conflict>",
            Some(source) => source.label(),
        };
        let env = match &self.env {
            EnvSource::Default => "default",
            EnvSource::Environment { .. } => "environment",
            EnvSource::Config { .. } => "config",
            EnvSource::DotenvFile { .. } => "dotenv",
        };
        f.debug_struct("ClientBuilder")
            .field("auth", &auth)
            .field("env", &env)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolve a builder's auth + env without a network round-trip, so the
    /// happy-path tests can assert the composed `Credentials` and
    /// `DirectConfig` shape directly.
    fn resolve(b: ClientBuilder) -> Result<(Credentials, DirectConfig), Error> {
        let auth = b.auth.expect("auth set").into_resolved_source()?;
        let creds = auth.resolve()?;
        let config = b.env.resolve()?;
        Ok((creds, config))
    }

    #[test]
    fn api_key_inline_builds_api_key_credentials() {
        let (creds, config) = resolve(ClientBuilder::new().api_key("  td1_example  ")).unwrap();
        assert!(creds.is_api_key());
        assert_eq!(creds.api_key_secret(), Some("td1_example"));
        assert_eq!(config.historical_environment(), HistoricalEnvironment::Prod);
        assert_eq!(config.streaming_environment(), StreamingEnvironment::Prod);
    }

    #[test]
    fn email_password_builds_password_credentials() {
        let (creds, _) =
            resolve(ClientBuilder::new().email_password("User@Example.COM", "hunter2")).unwrap();
        assert!(!creds.is_api_key());
        assert_eq!(creds.email(), Some("user@example.com"));
        assert_eq!(creds.password(), Some("hunter2"));
    }

    #[test]
    fn stage_selects_historical_staging_only() {
        let (_, config) = resolve(ClientBuilder::new().api_key("k").stage()).unwrap();
        assert_eq!(
            config.historical_environment(),
            HistoricalEnvironment::Stage
        );
        assert_eq!(config.historical_host(), "mdds-stage.thetadata.us");
        // Streaming stays on production — `stage()` is historical-only.
        assert_eq!(config.streaming_environment(), StreamingEnvironment::Prod);
    }

    #[test]
    fn dev_selects_streaming_dev_only() {
        let (_, config) = resolve(ClientBuilder::new().api_key("k").dev()).unwrap();
        assert_eq!(config.streaming_environment(), StreamingEnvironment::Dev);
        // Historical stays on production — `dev()` is streaming-only.
        assert_eq!(config.historical_environment(), HistoricalEnvironment::Prod);
    }

    #[test]
    fn per_channel_selectors_compose() {
        // `.stage().dev()` selects historical-staging AND streaming-dev — the
        // two channels are independent and both selections survive.
        let (_, config) = resolve(ClientBuilder::new().api_key("k").stage().dev()).unwrap();
        assert_eq!(
            config.historical_environment(),
            HistoricalEnvironment::Stage
        );
        assert_eq!(config.streaming_environment(), StreamingEnvironment::Dev);
    }

    #[test]
    fn explicit_environment_prod_round_trips() {
        let (_, config) = resolve(
            ClientBuilder::new()
                .api_key("k")
                .historical_environment(HistoricalEnvironment::Prod)
                .streaming_environment(StreamingEnvironment::Prod),
        )
        .unwrap();
        assert_eq!(config.historical_environment(), HistoricalEnvironment::Prod);
        assert_eq!(config.streaming_environment(), StreamingEnvironment::Prod);
    }

    #[test]
    fn config_override_wins_over_environment() {
        // A full config set AFTER `.stage()` must win: the config carries
        // the strongest routing intent.
        let (_, config) = resolve(
            ClientBuilder::new()
                .api_key("k")
                .stage()
                .config(DirectConfig::production()),
        )
        .unwrap();
        assert_eq!(config.historical_environment(), HistoricalEnvironment::Prod);
    }

    #[test]
    fn prebuilt_credentials_pass_through() {
        let prebuilt = Credentials::api_key_with_email("a@b.com", "key-xyz");
        let (creds, _) = resolve(ClientBuilder::new().credentials(prebuilt)).unwrap();
        assert!(creds.is_api_key());
        assert_eq!(creds.api_key_secret(), Some("key-xyz"));
        assert_eq!(creds.email(), Some("a@b.com"));
    }

    #[tokio::test]
    async fn no_auth_source_is_an_error() {
        // `Client` does not implement `Debug`, so map the `Ok` arm away
        // before asserting on the error.
        let msg = match ClientBuilder::new().connect().await {
            Ok(_) => panic!("expected an error for a builder with no auth source"),
            Err(e) => e.to_string(),
        };
        assert!(msg.contains("no authentication source"), "got: {msg}");
    }

    #[tokio::test]
    async fn conflicting_auth_sources_are_an_error() {
        let msg = match ClientBuilder::new()
            .api_key("k")
            .email_password("a@b.com", "pw")
            .connect()
            .await
        {
            Ok(_) => panic!("expected a conflict error"),
            Err(e) => e.to_string(),
        };
        assert!(msg.contains("conflicting authentication"), "got: {msg}");
        assert!(msg.contains("api_key"), "got: {msg}");
        assert!(msg.contains("email_password"), "got: {msg}");
    }

    #[tokio::test]
    async fn file_backed_source_resolution_error_surfaces_through_connect() {
        // `connect()` resolves the file-backed credential on a blocking-pool
        // thread (so the synchronous `std::fs` read never blocks the async
        // worker). The resolution error must surface unchanged — identical to
        // resolving the same source inline.
        let missing = std::env::temp_dir().join(format!(
            "thetadatadx-no-such-creds-{}.txt",
            std::process::id()
        ));
        // Inline resolution (the test-only path) for the same source.
        let inline_err = resolve(ClientBuilder::new().credentials_file(&missing))
            .expect_err("missing credentials file must error inline");
        // Resolution routed through connect()'s spawn_blocking offload.
        let connect_err = match ClientBuilder::new()
            .credentials_file(&missing)
            .connect()
            .await
        {
            Ok(_) => panic!("expected an error for a missing credentials file"),
            Err(e) => e,
        };
        // Same surfaced message — the offload changes the thread, not the
        // error.
        assert_eq!(inline_err.to_string(), connect_err.to_string());
    }

    #[test]
    fn same_source_twice_overwrites_without_conflict() {
        // Re-stating the same kind of source is not a conflict; the last
        // value wins.
        let (creds, _) = resolve(ClientBuilder::new().api_key("first").api_key("second")).unwrap();
        assert_eq!(creds.api_key_secret(), Some("second"));
    }

    #[test]
    fn debug_redacts_secrets() {
        let b = ClientBuilder::new()
            .api_key("super-secret-key")
            .email_password("user@example.com", "hunter2");
        // The second (different) source makes this a conflict sentinel,
        // so build a clean single-source builder for the redaction check.
        let _ = b;
        let single = ClientBuilder::new().api_key("super-secret-key");
        let rendered = format!("{single:?}");
        assert!(
            !rendered.contains("super-secret-key"),
            "Debug leaked api key: {rendered}"
        );
        assert!(rendered.contains("api_key"), "got: {rendered}");

        let pw = ClientBuilder::new().email_password("user@example.com", "hunter2");
        let rendered = format!("{pw:?}");
        assert!(
            !rendered.contains("hunter2") && !rendered.contains("user@example.com"),
            "Debug leaked email/password: {rendered}"
        );
    }
}
