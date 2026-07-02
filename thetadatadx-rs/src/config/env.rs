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

use super::{DirectConfig, HistoricalEnvironment, StreamingEnvironment};
use crate::error::Error;

/// Historical environment selector (`PROD` / `STAGE`, case-insensitive).
/// `STAGE` points the historical host and the auth marker at the staging
/// environment; `PROD` (or unset) keeps production. The streaming channel is
/// selected separately via [`ENV_STREAMING_TYPE`]. An explicit
/// `THETADATA_HISTORICAL_HOST` is recorded first and the environment selected
/// last, so the explicit host patches the selected environment's cluster
/// rather than being overwritten by it. An unrecognized value (including
/// `DEV`, which the historical channel does not support) is a hard error
/// naming the valid set, never a silent fallback.
pub const ENV_HISTORICAL_TYPE: &str = "THETADATA_HISTORICAL_TYPE";

/// Streaming environment selector (`PROD` / `DEV`, case-insensitive). `DEV`
/// points the streaming channel at the dev replay cluster; `PROD` (or unset)
/// keeps production. It never affects auth or the historical channel. An
/// unrecognized value (including `STAGE`, which the streaming channel does not
/// support) is a hard error naming the valid set, never a silent fallback.
pub const ENV_STREAMING_TYPE: &str = "THETADATA_STREAMING_TYPE";

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

    /// Human label for the value source, for the typed error on an
    /// unrecognized environment selector.
    fn label(self) -> &'static str {
        match self {
            Source::ProcessEnv => "env var",
            Source::DotEnv => ".env value",
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
/// cluster). A malformed host/port/URL override is logged via `source` and
/// skipped so a typo there never flips production to the wrong endpoint. An
/// unrecognized environment SELECTOR is different: it is a hard error naming
/// the valid set (returned to the caller), never a silent fallback, so a stale
/// or cross-channel selector cannot quietly route to the wrong cluster.
///
/// Partial-mutation contract: the host/port/URL overrides are recorded before
/// the environment selectors are resolved, so when a selector is unrecognized
/// this returns `Err` with those overrides already applied — `cfg` may be left
/// partially mutated on the `Err` path. Both callers discard the config on
/// `Err` ([`DirectConfig::production`] panics, and the `.env` path owns its
/// receiver and drops it), so the partial state is never observed; a caller
/// that retains a config across an `Err` from this function must not assume it
/// is unchanged.
fn apply_overrides<F>(cfg: &mut DirectConfig, get: F, source: Source) -> Result<(), Error>
where
    F: Fn(&str) -> Option<String>,
{
    // Record the explicit host/port/url/client_type overrides into the typed
    // override fields FIRST, then select the environments LAST:
    // `apply_historical_environment` / `apply_streaming_environment` rebuild the
    // cluster routing from the selected environment and patch the recorded host
    // overrides on top, so an explicit host:port always wins over the
    // environment default while the environment's failover hosts are preserved.
    // Recording before selecting is what lets the override survive the
    // environment rewrite (and a later `with_historical_environment` /
    // `with_streaming_environment` switch).
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
    // that redirects the cluster (`THETADATA_HISTORICAL_TYPE=STAGE`) and supplies a
    // staging `THETADATA_NEXUS_URL` in the same source expects auth to follow
    // the cluster, not keep POSTing production. Environment selection routes
    // only the historical + streaming hosts (it does not touch `auth`), so the
    // Nexus URL override is the only thing that re-points auth — it must be
    // honoured from every source that carries it.
    if let Some(url) = get(ENV_NEXUS_URL) {
        // A value that is not an http(s) URL cannot be a Nexus base: applying
        // it verbatim would silently point auth at an unreachable endpoint.
        // Skip it and keep the current URL, matching the lenient host/port
        // handling above.
        if url.starts_with("http://") || url.starts_with("https://") {
            cfg.auth.nexus_url = url;
        } else {
            tracing::warn!(env = ENV_NEXUS_URL, value = %url, "{}", source.malformed_value());
        }
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

    // Environment selectors last: rebuild each channel's cluster routing,
    // applying the overrides recorded above. A blank value reads as unset (via
    // `get`); an UNRECOGNIZED value is a hard error naming the valid set
    // (never a silent fallback), so a stale or typo'd selector — including a
    // cross-channel value like `THETADATA_HISTORICAL_TYPE=DEV` or
    // `THETADATA_STREAMING_TYPE=STAGE` — fails loud instead of quietly keeping the
    // wrong cluster. When a selector is absent we re-apply the CURRENT
    // environment so a host override recorded above patches the existing
    // cluster.
    let historical = match get(ENV_HISTORICAL_TYPE) {
        Some(value) => HistoricalEnvironment::parse(&value).ok_or_else(|| {
            Error::config_invalid(
                ENV_HISTORICAL_TYPE,
                format!(
                    "{} {ENV_HISTORICAL_TYPE}={value:?} is not a historical environment; expected \"PROD\" or \"STAGE\"",
                    source.label()
                ),
            )
        })?,
        None => cfg.historical_environment,
    };
    cfg.apply_historical_environment(historical);

    let streaming = match get(ENV_STREAMING_TYPE) {
        Some(value) => StreamingEnvironment::parse(&value).ok_or_else(|| {
            Error::config_invalid(
                ENV_STREAMING_TYPE,
                format!(
                    "{} {ENV_STREAMING_TYPE}={value:?} is not a streaming environment; expected \"PROD\" or \"DEV\"",
                    source.label()
                ),
            )
        })?,
        None => cfg.streaming_environment,
    };
    cfg.apply_streaming_environment(streaming);
    Ok(())
}

/// Apply the documented [`DirectConfig`] env-var matrix on top of the
/// receiver. A malformed host/port override is logged and skipped; an
/// unrecognized environment selector returns a typed error naming the valid
/// set, so a typo never silently flips production to the wrong endpoint.
pub(super) fn apply_env_overrides(cfg: &mut DirectConfig) -> Result<(), Error> {
    apply_overrides(
        cfg,
        |key| {
            std::env::var(key).ok().and_then(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })
        },
        Source::ProcessEnv,
    )
}

/// Apply the environment-selector and override matrix carried by a parsed
/// `.env` file on top of the receiver.
///
/// This is the `.env`-file analogue of [`apply_env_overrides`]: it reads the
/// **same** keys with the **same** precedence (`THETADATA_HISTORICAL_TYPE` first to
/// set the cluster default, then an explicit `THETADATA_HISTORICAL_HOST` /
/// `THETADATA_STREAMING_HOST` wins over that default) but sources the values
/// from the `(key, value)` pairs a `.env` file parsed to, rather than from the
/// process environment. Both paths run the identical [`apply_overrides`] body,
/// so the credential/cluster key set is guaranteed not to drift between them.
/// Only the documented configuration keys are read here; the credential keys in
/// the same file are handled by [`crate::auth::Credentials::from_dotenv`].
///
/// A malformed host/port value is skipped leniently, matching the process-env
/// path; an unrecognized environment selector returns a typed error naming the
/// valid set, so a typo in the `.env` never silently flips the cluster to the
/// wrong endpoint.
pub(super) fn apply_dotenv_overrides(
    cfg: &mut DirectConfig,
    pairs: &[(String, &str)],
) -> Result<(), Error> {
    use crate::auth::dotenv::lookup;

    // `lookup` filters out absent/blank keys but returns the value verbatim so
    // the credential path keeps opaque secrets byte-exact. These are the
    // non-secret config keys (host / port / URL / selector), where a quoted
    // value's surrounding whitespace (`KEY=" host "`) is never significant, so
    // trim it here to match the process-env path (which trims) and the
    // "trimmed, non-empty value" contract `apply_overrides` documents.
    apply_overrides(
        cfg,
        |key| {
            lookup(pairs, key).and_then(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            })
        },
        Source::DotEnv,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive `apply_overrides` with a fixed key->value map, the same body both
    /// the process-env and `.env` paths run.
    fn apply(pairs: &[(&str, &str)]) -> Result<DirectConfig, Error> {
        // Seed from the hardcoded defaults, not `production()`: these tests
        // exercise the override layer in isolation, so reading the real process
        // environment here would make them non-hermetic (a stray `THETADATA_*`
        // in the runner could flip the baseline or panic on an invalid selector).
        let mut cfg = DirectConfig::production_defaults();
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        apply_overrides(
            &mut cfg,
            // Mirror the blank-as-unset filtering both real entry points apply
            // before `apply_overrides` sees a value.
            |key| {
                owned.iter().find(|(k, _)| k == key).and_then(|(_, v)| {
                    let trimmed = v.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                })
            },
            Source::ProcessEnv,
        )?;
        Ok(cfg)
    }

    #[test]
    fn valid_selectors_route_each_channel() {
        let cfg = apply(&[(ENV_HISTORICAL_TYPE, "STAGE"), (ENV_STREAMING_TYPE, "DEV")])
            .expect("PROD/STAGE + PROD/DEV are valid selectors");
        assert_eq!(cfg.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(cfg.streaming_environment, StreamingEnvironment::Dev);
    }

    #[test]
    fn absent_selectors_leave_the_baseline_untouched() {
        let cfg = apply(&[]).expect("no selectors is a no-op");
        assert_eq!(cfg.historical_environment, HistoricalEnvironment::Prod);
        assert_eq!(cfg.streaming_environment, StreamingEnvironment::Prod);
    }

    #[test]
    fn cross_channel_historical_selector_fails_loud() {
        // The historical channel has no dev cluster: a stale/cross-channel
        // value must NOT silently fall back to a default — it is a typed error
        // that names the key and the valid set.
        let err = apply(&[(ENV_HISTORICAL_TYPE, "DEV")])
            .expect_err("HISTORICAL_TYPE=DEV must fail loud, never fall back");
        let msg = err.to_string();
        assert!(msg.contains(ENV_HISTORICAL_TYPE), "names the key: {msg}");
        assert!(
            msg.contains("PROD") && msg.contains("STAGE"),
            "names the valid set: {msg}"
        );
    }

    #[test]
    fn cross_channel_streaming_selector_fails_loud() {
        // The streaming channel has no staging cluster.
        let err = apply(&[(ENV_STREAMING_TYPE, "STAGE")])
            .expect_err("STREAMING_TYPE=STAGE must fail loud, never fall back");
        let msg = err.to_string();
        assert!(msg.contains(ENV_STREAMING_TYPE), "names the key: {msg}");
        assert!(
            msg.contains("PROD") && msg.contains("DEV"),
            "names the valid set: {msg}"
        );
    }

    #[test]
    fn a_typoed_selector_fails_loud() {
        assert!(apply(&[(ENV_HISTORICAL_TYPE, "prdo")]).is_err());
        assert!(
            apply(&[(ENV_STREAMING_TYPE, "")]).is_ok(),
            "blank reads as unset"
        );
    }

    #[test]
    fn a_malformed_nexus_url_is_skipped_not_applied() {
        let baseline = DirectConfig::production_defaults().auth.nexus_url.clone();
        let cfg =
            apply(&[(ENV_NEXUS_URL, "not-a-url")]).expect("malformed URL is skipped, not fatal");
        assert_eq!(
            cfg.auth.nexus_url, baseline,
            "a non-http(s) value must not re-point auth"
        );
        let cfg = apply(&[(ENV_NEXUS_URL, "https://nexus.example.test")])
            .expect("a valid http(s) URL is honoured");
        assert_eq!(cfg.auth.nexus_url, "https://nexus.example.test");
    }

    #[test]
    fn dotenv_config_keys_are_trimmed_of_quoted_whitespace() {
        // A quoted value's surrounding whitespace is never significant for a
        // host override, so the `.env` path trims it just like the process-env
        // path does.
        let pairs = crate::auth::dotenv::parse(
            "THETADATA_HISTORICAL_HOST=\"  historical.example.test  \"\n",
        );
        let mut cfg = DirectConfig::production_defaults();
        apply_dotenv_overrides(&mut cfg, &pairs).expect("valid host override");
        assert_eq!(cfg.historical.host, "historical.example.test");
    }

    #[test]
    fn an_explicit_host_override_survives_the_selector_rewrite() {
        // Recording the host before selecting the environment is what lets an
        // explicit host patch the selected cluster rather than be overwritten.
        let cfg = apply(&[
            (ENV_HISTORICAL_TYPE, "STAGE"),
            (ENV_HISTORICAL_HOST, "historical.example.test"),
        ])
        .expect("valid selector + explicit host");
        assert_eq!(cfg.historical_environment, HistoricalEnvironment::Stage);
        assert_eq!(cfg.historical.host, "historical.example.test");
    }
}
