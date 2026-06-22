//! Environment-variable override layer for [`DirectConfig`].
//!
//! Two groups:
//!
//! * Compatibility set (`THETADATA_HISTORICAL_HOST`,
//!   `THETADATA_HISTORICAL_PORT`, `THETADATA_EMAIL`, `THETADATA_PASSWORD`)
//!   — environment variable names operators use to configure the
//!   historical endpoint; setting them steers an existing shell config
//!   without a code change.
//! * DX extensions — cover surfaces that were previously hardcoded (Nexus
//!   URL, streaming host/port, `client_type`) so site operators can steer
//!   traffic at a staging cluster without a code change.
//!
//! Two entry points read this matrix: [`apply_env_overrides`] sources the
//! values from the process environment, and [`apply_dotenv_overrides`] sources
//! them from a parsed `.env` file. Both run the single shared
//! [`apply_overrides`] body, so they honour the **same** key set with the
//! **same** precedence and can never drift — only the value source and the
//! diagnostic wording differ.
//!
//! Precedence is documented on `DirectConfig`: explicit builder setter >
//! env var > hardcoded default.

use super::{DirectConfig, Environment};

/// Target server environment selector (`PROD` / `STAGE` / `DEV`,
/// case-insensitive). Equivalent to ThetaData's `mdds_type` option.
/// `STAGE` points every cluster-bound channel at the staging environment;
/// `DEV` points the streaming channel at the dev replay cluster while auth
/// and historical stay on production; `PROD` (or unset) keeps production.
/// The explicit host/port overrides are recorded first
/// and the environment is selected last, so an explicit
/// `THETADATA_HISTORICAL_HOST` / `THETADATA_STREAMING_HOST` /
/// `THETADATA_STREAMING_PORT` patches the selected environment's cluster
/// (the failover hosts still track that environment) rather than being
/// overwritten by it.
pub const ENV_MDDS_TYPE: &str = "THETADATA_MDDS_TYPE";

/// Historical host.
pub const ENV_HISTORICAL_HOST: &str = "THETADATA_HISTORICAL_HOST";
/// Historical port.
pub const ENV_HISTORICAL_PORT: &str = "THETADATA_HISTORICAL_PORT";
/// Nexus auth base URL override.
pub const ENV_NEXUS_URL: &str = "THETADATA_NEXUS_URL";
/// Streaming hostname override. Replaces the host of the primary streaming
/// slot; the selected environment's failover hosts are preserved.
pub const ENV_STREAMING_HOST: &str = "THETADATA_STREAMING_HOST";
/// Streaming port override. Replaces the port of the primary streaming slot
/// independently of [`ENV_STREAMING_HOST`]: a port-only override keeps the
/// selected environment's primary host and only re-points its port.
pub const ENV_STREAMING_PORT: &str = "THETADATA_STREAMING_PORT";
/// `QueryInfo.client_type` override — steer server-side quotas and
/// dashboards to treat a deployment as a named fleet.
pub const ENV_CLIENT_TYPE: &str = "THETADATA_CLIENT_TYPE";

/// Where an override value was sourced from.
///
/// Selects the diagnostic wording emitted when a value is malformed or
/// unrecognized so a `tracing::warn!` reads naturally for the path that
/// produced it (`env var` for the process environment, `.env value` for a
/// parsed `.env` file). It does not change WHICH keys are read or HOW they are
/// applied — both paths run the identical [`apply_overrides`] body — so the two
/// sources can never drift in key coverage or precedence.
#[derive(Clone, Copy)]
enum Source {
    /// Values read from the process environment (`std::env::var`).
    ProcessEnv,
    /// Values read from a parsed `.env` file's `(key, value)` pairs.
    DotEnv,
}

impl Source {
    /// Diagnostic phrase for a malformed value (e.g. a non-integer port).
    fn malformed_value(self) -> &'static str {
        match self {
            Source::ProcessEnv => "ignoring malformed env var; keeping hardcoded default",
            Source::DotEnv => "ignoring malformed .env value; keeping hardcoded default",
        }
    }

    /// Diagnostic phrase for an unrecognized `THETADATA_MDDS_TYPE`.
    fn unrecognized_mdds_type(self) -> &'static str {
        match self {
            Source::ProcessEnv => {
                "ignoring unrecognized env var (expected PROD, STAGE, or DEV); keeping current environment"
            }
            Source::DotEnv => {
                "ignoring unrecognized .env value (expected PROD, STAGE, or DEV); keeping current environment"
            }
        }
    }
}

/// Apply the documented [`DirectConfig`] override matrix on top of the
/// receiver, sourcing each key's value through `get`.
///
/// This is the single body shared by the process-env ([`apply_env_overrides`])
/// and `.env`-file ([`apply_dotenv_overrides`]) paths: both read the **same**
/// key set with the **same** precedence, differing only in how a key maps to a
/// value (`std::env::var` vs a parsed `.env` lookup) and in the diagnostic
/// wording (`source`). Factoring the per-key application here is what keeps the
/// two paths from drifting — adding a key updates both at once.
///
/// `get` returns the trimmed, non-empty value for a key, or `None` when the key
/// is absent or blank (an empty / all-whitespace value reads as unset so a
/// blank never wins precedence and builds an empty host override or flips the
/// cluster). Unknown / malformed values are logged via `source` and skipped so
/// a typo never silently flips production to the wrong endpoint.
fn apply_overrides<F>(cfg: &mut DirectConfig, get: F, source: Source)
where
    F: Fn(&str) -> Option<String>,
{
    // Record the explicit host/port/url/client_type overrides into the typed
    // override fields FIRST, then select the environment LAST: `apply_environment`
    // rebuilds the cluster routing from the selected environment and patches the
    // recorded host overrides on top, so an explicit host:port always wins over
    // the environment default while the environment's failover hosts are
    // preserved. Recording before selecting is what lets the override survive
    // the environment rewrite (and a later `with_environment` switch).
    if let Some(host) = get(ENV_HISTORICAL_HOST) {
        cfg.set_historical_host_override(host);
    }
    if let Some(port_str) = get(ENV_HISTORICAL_PORT) {
        match port_str.parse::<u16>() {
            Ok(port) if port > 0 => cfg.historical.port = port,
            _ => tracing::warn!(
                env = ENV_HISTORICAL_PORT,
                value = %port_str,
                "{}",
                source.malformed_value()
            ),
        }
    }
    // Nexus auth URL and client_type are cluster-bound auth knobs: an operator
    // that redirects the cluster (`THETADATA_MDDS_TYPE=STAGE`) and supplies a
    // staging `THETADATA_NEXUS_URL` in the same source expects auth to follow
    // the cluster, not keep POSTing production. Environment selection routes
    // only the historical + streaming hosts (it does not touch `auth`), so the
    // Nexus URL override is the only thing that re-points auth — it must be
    // honoured from every source that carries it.
    if let Some(url) = get(ENV_NEXUS_URL) {
        cfg.auth.nexus_url = url;
    }
    if let Some(client_type) = get(ENV_CLIENT_TYPE) {
        cfg.auth.client_type = client_type;
    }
    // Streaming host and port are independent overrides of the primary slot:
    // a host-only override keeps the environment's primary port, a port-only
    // override keeps the environment's primary host cluster, and the
    // environment's failover hosts are always preserved. Recording the port
    // alone must NOT suppress the host rebuild.
    if let Some(host) = get(ENV_STREAMING_HOST) {
        cfg.set_streaming_primary_host_override(host);
    }
    if let Some(port_str) = get(ENV_STREAMING_PORT) {
        match port_str.parse::<u16>() {
            Ok(port) if port > 0 => cfg.set_streaming_primary_port_override(port),
            _ => tracing::warn!(
                env = ENV_STREAMING_PORT,
                value = %port_str,
                "{}",
                source.malformed_value()
            ),
        }
    }

    // Environment selector last: rebuild the cluster routing for the selected
    // environment, applying the overrides recorded above. A blank value reads
    // as unset (via `get`); an unrecognized value is logged and skipped
    // (lenient, matching the malformed-port handling) so a typo never silently
    // flips the cluster to the wrong endpoint. When no (or an unrecognized)
    // selector is present we still re-apply the CURRENT environment so a host
    // override recorded above patches the existing cluster.
    let selected = match get(ENV_MDDS_TYPE) {
        Some(mdds_type) => match Environment::parse(&mdds_type) {
            Some(env) => env,
            None => {
                tracing::warn!(
                    env = ENV_MDDS_TYPE,
                    value = %mdds_type,
                    "{}",
                    source.unrecognized_mdds_type()
                );
                cfg.environment
            }
        },
        None => cfg.environment,
    };
    cfg.apply_environment(selected);
}

/// Apply the documented [`DirectConfig`] env-var matrix on top of the
/// receiver. Unknown / malformed values are logged and skipped so a
/// typo never silently flips production to the wrong endpoint.
pub(super) fn apply_env_overrides(cfg: &mut DirectConfig) {
    apply_overrides(
        cfg,
        |key| {
            std::env::var(key).ok().and_then(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })
        },
        Source::ProcessEnv,
    );
}

/// Apply the environment-selector and override matrix carried by a parsed
/// `.env` file on top of the receiver.
///
/// This is the `.env`-file analogue of [`apply_env_overrides`]: it reads the
/// **same** keys with the **same** precedence (`THETADATA_MDDS_TYPE` first to
/// set the cluster default, then an explicit `THETADATA_HISTORICAL_HOST` /
/// `THETADATA_STREAMING_HOST` wins over that default) but sources the values
/// from the `(key, value)` pairs a `.env` file parsed to, rather than from the
/// process environment. Both paths run the identical [`apply_overrides`] body,
/// so the credential/cluster key set is guaranteed not to drift between them.
/// Only the documented configuration keys are read here; the credential keys in
/// the same file are handled by [`crate::auth::Credentials::from_dotenv`].
///
/// Unknown / empty values are skipped leniently, matching the process-env
/// path, so a typo in the `.env` never silently flips the cluster to the wrong
/// endpoint.
pub(super) fn apply_dotenv_overrides(cfg: &mut DirectConfig, pairs: &[(String, &str)]) {
    use crate::auth::dotenv::lookup;

    // `lookup` already returns trimmed, non-empty values (or `None` for an
    // absent or blank key), matching the contract `apply_overrides` expects.
    apply_overrides(
        cfg,
        |key| lookup(pairs, key).map(str::to_string),
        Source::DotEnv,
    );
}
