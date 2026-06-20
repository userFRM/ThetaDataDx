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
//! Precedence is documented on `DirectConfig`: explicit builder setter >
//! env var > hardcoded default.

use super::{DirectConfig, Environment};

/// Target server environment selector (`PROD` / `STAGE`, case-insensitive).
/// Equivalent to ThetaData's `mdds_type` option. `STAGE` points every
/// cluster-bound channel at the staging environment; `PROD` (or unset)
/// keeps production. The explicit host/port overrides are recorded first
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

/// Apply the documented [`DirectConfig`] env-var matrix on top of the
/// receiver. Unknown / malformed values are logged and skipped so a
/// typo never silently flips production to the wrong endpoint.
pub(super) fn apply_env_overrides(cfg: &mut DirectConfig) {
    // Record the explicit host/port overrides into the typed override fields
    // FIRST, then select the environment LAST: `apply_environment` rebuilds
    // the cluster routing from the selected environment and patches the
    // recorded overrides on top, so an explicit host:port always wins over
    // the environment default while the environment's failover hosts are
    // preserved. Recording before selecting is what lets the override survive
    // the environment rewrite (and a later `with_environment` switch).
    if let Ok(host) = std::env::var(ENV_HISTORICAL_HOST) {
        let trimmed = host.trim();
        if !trimmed.is_empty() {
            cfg.set_historical_host_override(trimmed.to_string());
        }
    }
    if let Ok(port_str) = std::env::var(ENV_HISTORICAL_PORT) {
        match port_str.trim().parse::<u16>() {
            Ok(port) if port > 0 => cfg.historical.port = port,
            _ => tracing::warn!(
                env = ENV_HISTORICAL_PORT,
                value = %port_str,
                "ignoring malformed env var; keeping hardcoded default"
            ),
        }
    }
    if let Ok(url) = std::env::var(ENV_NEXUS_URL) {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            cfg.auth.nexus_url = trimmed.to_string();
        }
    }
    if let Ok(client_type) = std::env::var(ENV_CLIENT_TYPE) {
        let trimmed = client_type.trim();
        if !trimmed.is_empty() {
            cfg.auth.client_type = trimmed.to_string();
        }
    }
    // Streaming host and port are independent overrides of the primary slot:
    // a host-only override keeps the environment's primary port, a port-only
    // override keeps the environment's primary host cluster, and the
    // environment's failover hosts are always preserved. Recording the port
    // alone must NOT suppress the host rebuild.
    if let Ok(host) = std::env::var(ENV_STREAMING_HOST) {
        let trimmed = host.trim();
        if !trimmed.is_empty() {
            cfg.set_streaming_primary_host_override(trimmed.to_string());
        }
    }
    if let Ok(port_str) = std::env::var(ENV_STREAMING_PORT) {
        match port_str.trim().parse::<u16>() {
            Ok(port) if port > 0 => cfg.set_streaming_primary_port_override(port),
            _ => tracing::warn!(
                env = ENV_STREAMING_PORT,
                value = %port_str,
                "ignoring malformed env var; keeping hardcoded default"
            ),
        }
    }

    // Environment selector last: rebuild the cluster routing for the selected
    // environment, applying the overrides recorded above. An empty value is
    // treated as unset; an unrecognized value is logged and skipped (lenient,
    // matching the malformed-port handling) so a typo never silently flips
    // the cluster to the wrong endpoint. When no (or an unrecognized)
    // selector is present we still re-apply the CURRENT environment so a host
    // override recorded above patches the existing cluster.
    let selected = match std::env::var(ENV_MDDS_TYPE) {
        Ok(mdds_type) if !mdds_type.trim().is_empty() => match Environment::parse(mdds_type.trim())
        {
            Some(env) => env,
            None => {
                tracing::warn!(
                    env = ENV_MDDS_TYPE,
                    value = %mdds_type,
                    "ignoring unrecognized env var (expected PROD or STAGE); keeping current environment"
                );
                cfg.environment
            }
        },
        _ => cfg.environment,
    };
    cfg.apply_environment(selected);
}

/// Apply the environment-selector and host overrides carried by a parsed
/// `.env` file on top of the receiver.
///
/// This is the `.env`-file analogue of [`apply_env_overrides`]: it reads the
/// same keys with the same precedence (`THETADATA_MDDS_TYPE` first to set the
/// cluster default, then an explicit `THETADATA_HISTORICAL_HOST` /
/// `THETADATA_STREAMING_HOST` wins over that default) but sources the values
/// from the `(key, value)` pairs a `.env` file parsed to, rather than from the
/// process environment. Only the documented configuration keys are read here;
/// the credential keys in the same file are handled by
/// [`crate::auth::Credentials::from_dotenv`].
///
/// Unknown / empty values are skipped leniently, matching the process-env
/// path, so a typo in the `.env` never silently flips the cluster to the wrong
/// endpoint.
pub(super) fn apply_dotenv_overrides(cfg: &mut DirectConfig, pairs: &[(String, &str)]) {
    use crate::auth::dotenv::lookup;

    // Mirror the process-env path: record the explicit host/port overrides
    // FIRST, then select the environment LAST so `apply_environment` rebuilds
    // the cluster routing and patches the overrides on top. An explicit host
    // wins over the environment default while the environment's failover hosts
    // are preserved. `lookup` already returns trimmed, non-empty values.
    if let Some(host) = lookup(pairs, ENV_HISTORICAL_HOST) {
        cfg.set_historical_host_override(host.to_string());
    }
    if let Some(host) = lookup(pairs, ENV_STREAMING_HOST) {
        cfg.set_streaming_primary_host_override(host.to_string());
    }
    if let Some(port_str) = lookup(pairs, ENV_STREAMING_PORT) {
        match port_str.parse::<u16>() {
            Ok(port) if port > 0 => cfg.set_streaming_primary_port_override(port),
            _ => tracing::warn!(
                env = ENV_STREAMING_PORT,
                value = %port_str,
                "ignoring malformed .env value; keeping hardcoded default"
            ),
        }
    }

    // Environment selector last: rebuild the cluster routing for the selected
    // environment, applying the overrides recorded above. An empty value
    // reads as unset; an unrecognized value is logged and skipped (lenient,
    // matching the process-env path). When no (or an unrecognized) selector is
    // present we re-apply the CURRENT environment so a host override recorded
    // above patches the existing cluster.
    let selected = match lookup(pairs, ENV_MDDS_TYPE) {
        Some(mdds_type) => match Environment::parse(mdds_type) {
            Some(env) => env,
            None => {
                tracing::warn!(
                    env = ENV_MDDS_TYPE,
                    value = %mdds_type,
                    "ignoring unrecognized .env value (expected PROD or STAGE); keeping current environment"
                );
                cfg.environment
            }
        },
        None => cfg.environment,
    };
    cfg.apply_environment(selected);
}
