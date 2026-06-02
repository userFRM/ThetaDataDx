//! Environment-variable override layer for [`DirectConfig`].
//!
//! Two groups:
//!
//! * Compatibility set (`THETADATA_MDDS_HOST`, `THETADATA_MDDS_PORT`,
//!   `THETADATA_EMAIL`, `THETADATA_PASSWORD`) — environment variable names
//!   operators already use to configure existing `ThetaData` clients;
//!   reusing them here means an existing shell config keeps working.
//! * DX extensions — cover surfaces that were previously hardcoded (Nexus
//!   URL, FPSS host/port, `client_type`) so site operators can steer
//!   traffic at a staging cluster without a code change.
//!
//! Precedence is documented on `DirectConfig`: explicit builder setter >
//! env var > hardcoded default.

use super::DirectConfig;

/// MDDS gRPC host.
pub const ENV_MDDS_HOST: &str = "THETADATA_MDDS_HOST";
/// MDDS gRPC port.
pub const ENV_MDDS_PORT: &str = "THETADATA_MDDS_PORT";
/// Nexus auth base URL override.
pub const ENV_NEXUS_URL: &str = "THETADATA_NEXUS_URL";
/// FPSS hostname override. Replaces the primary FPSS host slot; fallback
/// hosts are preserved.
pub const ENV_FPSS_HOST: &str = "THETADATA_FPSS_HOST";
/// FPSS port override. Pairs with [`ENV_FPSS_HOST`].
pub const ENV_FPSS_PORT: &str = "THETADATA_FPSS_PORT";
/// `QueryInfo.client_type` override — steer server-side quotas and
/// dashboards to treat a deployment as a named fleet.
pub const ENV_CLIENT_TYPE: &str = "THETADATA_CLIENT_TYPE";

/// Apply the documented [`DirectConfig`] env-var matrix on top of the
/// receiver. Unknown / malformed values are logged and skipped so a
/// typo never silently flips production to the wrong endpoint.
pub(super) fn apply_env_overrides(cfg: &mut DirectConfig) {
    if let Ok(host) = std::env::var(ENV_MDDS_HOST) {
        let trimmed = host.trim();
        if !trimmed.is_empty() {
            cfg.mdds.host = trimmed.to_string();
        }
    }
    if let Ok(port_str) = std::env::var(ENV_MDDS_PORT) {
        match port_str.trim().parse::<u16>() {
            Ok(port) if port > 0 => cfg.mdds.port = port,
            _ => tracing::warn!(
                env = ENV_MDDS_PORT,
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
    // FPSS host/port are mirrored as a (host, port) tuple in the
    // primary slot. If only one of the pair is set we keep the
    // default for the other half rather than guessing.
    let env_host = std::env::var(ENV_FPSS_HOST).ok();
    let env_port = std::env::var(ENV_FPSS_PORT).ok();
    if env_host.is_some() || env_port.is_some() {
        if cfg.fpss.hosts.is_empty() {
            // Empty defaults would mean "no primary to override".
            // Skip silently — production_defaults seeds 4 hosts, so
            // this only fires for hand-built configs.
            tracing::warn!(
                "ignoring THETADATA_FPSS_HOST / THETADATA_FPSS_PORT; \
                 DirectConfig has no FPSS hosts to override"
            );
        } else {
            let (default_host, default_port) = cfg.fpss.hosts[0].clone();
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
                            env = ENV_FPSS_PORT,
                            value = %raw,
                            "ignoring malformed env var; keeping hardcoded default"
                        );
                        None
                    }
                })
                .unwrap_or(default_port);
            cfg.fpss.hosts[0] = (host, port);
        }
    }
}
