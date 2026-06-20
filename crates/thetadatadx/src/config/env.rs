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
/// keeps production. Applied before the explicit host overrides below, so
/// an explicit `THETADATA_HISTORICAL_HOST` / `THETADATA_STREAMING_HOST`
/// still wins over the environment default.
pub const ENV_MDDS_TYPE: &str = "THETADATA_MDDS_TYPE";

/// Historical host.
pub const ENV_HISTORICAL_HOST: &str = "THETADATA_HISTORICAL_HOST";
/// Historical port.
pub const ENV_HISTORICAL_PORT: &str = "THETADATA_HISTORICAL_PORT";
/// Nexus auth base URL override.
pub const ENV_NEXUS_URL: &str = "THETADATA_NEXUS_URL";
/// Streaming hostname override. Replaces the primary streaming host slot;
/// fallback hosts are preserved.
pub const ENV_STREAMING_HOST: &str = "THETADATA_STREAMING_HOST";
/// Streaming port override. Pairs with [`ENV_STREAMING_HOST`].
pub const ENV_STREAMING_PORT: &str = "THETADATA_STREAMING_PORT";
/// `QueryInfo.client_type` override — steer server-side quotas and
/// dashboards to treat a deployment as a named fleet.
pub const ENV_CLIENT_TYPE: &str = "THETADATA_CLIENT_TYPE";

/// Apply the documented [`DirectConfig`] env-var matrix on top of the
/// receiver. Unknown / malformed values are logged and skipped so a
/// typo never silently flips production to the wrong endpoint.
pub(super) fn apply_env_overrides(cfg: &mut DirectConfig) {
    // Environment selector first, so it sets the cluster default before
    // the explicit host overrides below — an explicit host:port always
    // wins over the environment default. An empty value is treated as
    // unset; an unrecognized value is logged and skipped (lenient, matching
    // the malformed-port handling) so a typo never silently flips the
    // cluster to the wrong endpoint.
    if let Ok(mdds_type) = std::env::var(ENV_MDDS_TYPE) {
        let trimmed = mdds_type.trim();
        if !trimmed.is_empty() {
            match Environment::parse(trimmed) {
                Some(env) => cfg.apply_environment(env),
                None => tracing::warn!(
                    env = ENV_MDDS_TYPE,
                    value = %mdds_type,
                    "ignoring unrecognized env var (expected PROD or STAGE); keeping current environment"
                ),
            }
        }
    }
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
    // Streaming host/port are mirrored as a (host, port) tuple in the
    // primary slot. If only one of the pair is set we keep the
    // default for the other half rather than guessing.
    let env_host = std::env::var(ENV_STREAMING_HOST).ok();
    let env_port = std::env::var(ENV_STREAMING_PORT).ok();
    if env_host.is_some() || env_port.is_some() {
        if cfg.streaming.hosts.is_empty() {
            // Empty defaults would mean "no primary to override".
            // Skip silently — production_defaults seeds 4 hosts, so
            // this only fires for hand-built configs.
            tracing::warn!(
                "ignoring THETADATA_STREAMING_HOST / THETADATA_STREAMING_PORT; \
                 DirectConfig has no streaming hosts to override"
            );
        } else {
            let (default_host, default_port) = cfg.streaming.hosts[0].clone();
            let host = env_host
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map_or(default_host, str::to_string);
            let port = env_port
                .as_deref()
                .and_then(|raw| match raw.trim().parse::<u16>() {
                    Ok(p) if p > 0 => Some(p),
                    _ => {
                        tracing::warn!(
                            env = ENV_STREAMING_PORT,
                            value = %raw,
                            "ignoring malformed env var; keeping hardcoded default"
                        );
                        None
                    }
                })
                .unwrap_or(default_port);
            cfg.streaming.hosts[0] = (host, port);
            cfg.mark_streaming_hosts_overridden();
        }
    }
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

    // Environment selector first, so it sets the cluster default before the
    // explicit host override below — an explicit host always wins over the
    // environment default. An empty value reads as unset; an unrecognized
    // value is logged and skipped (lenient, matching the process-env path).
    if let Some(mdds_type) = lookup(pairs, ENV_MDDS_TYPE) {
        match Environment::parse(mdds_type) {
            Some(env) => cfg.apply_environment(env),
            None => tracing::warn!(
                env = ENV_MDDS_TYPE,
                value = %mdds_type,
                "ignoring unrecognized .env value (expected PROD or STAGE); keeping current environment"
            ),
        }
    }
    if let Some(host) = lookup(pairs, ENV_HISTORICAL_HOST) {
        cfg.set_historical_host_override(host.to_string());
    }
    if let Some(host) = lookup(pairs, ENV_STREAMING_HOST) {
        if cfg.streaming.hosts.is_empty() {
            tracing::warn!(
                "ignoring THETADATA_STREAMING_HOST from .env; \
                 DirectConfig has no streaming hosts to override"
            );
        } else {
            let port = cfg.streaming.hosts[0].1;
            cfg.streaming.hosts[0] = (host.to_string(), port);
            cfg.mark_streaming_hosts_overridden();
        }
    }
}
