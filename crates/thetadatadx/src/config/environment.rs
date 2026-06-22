//! Target environment selector for `ThetaData` server access.
//!
//! Selects which `ThetaData` server cluster the SDK connects to. The
//! default is [`Environment::Prod`]; [`Environment::Stage`] points the
//! auth, historical, and streaming channels at the staging cluster, and
//! [`Environment::Dev`] points the streaming channel at the dev replay
//! cluster while the auth and historical channels stay on production.
//!
//! An [`Environment`] is two decisions in one value: it is both the
//! **cluster selector** (which hosts the historical and streaming channels
//! dial) and the **auth wire marker** (the `authEnv` object carried on the
//! Nexus auth request). The two are deliberately decoupled per variant: a
//! variant can route streaming at a distinct cluster without claiming a
//! distinct server auth env. [`Environment::Dev`] is exactly that case — it
//! selects the dev replay streaming cluster but authenticates as production
//! (no `authEnv` on the wire), because dev replay is a streaming-only
//! offering with no server-side auth env of its own. The auth marker lives
//! in [`crate::auth`]; this module owns the cluster selection.
//!
//! The selector is set as a unit by the [`DirectConfig`] presets:
//! [`DirectConfig::production`] selects [`Environment::Prod`],
//! [`DirectConfig::stage`] selects [`Environment::Stage`], and
//! [`DirectConfig::dev`] selects [`Environment::Dev`], each with its
//! matching hosts. It can also be chosen directly with
//! [`DirectConfig::with_environment`], or via the `THETADATA_MDDS_TYPE`
//! environment variable (`PROD` / `STAGE` / `DEV`).
//!
//! [`DirectConfig`]: crate::config::DirectConfig
//! [`DirectConfig::production`]: crate::config::DirectConfig::production
//! [`DirectConfig::stage`]: crate::config::DirectConfig::stage
//! [`DirectConfig::dev`]: crate::config::DirectConfig::dev
//! [`DirectConfig::with_environment`]: crate::config::DirectConfig::with_environment

/// Which `ThetaData` server environment the SDK targets.
///
/// A variant carries two decisions: the **cluster** the historical and
/// streaming channels dial, and the **auth wire marker** the Nexus auth
/// request carries. The two are decoupled per variant rather than assumed
/// to move together — see [`Environment::Dev`].
///
/// Defaults to [`Environment::Prod`]. Selecting [`Environment::Stage`]
/// (via [`DirectConfig::stage`](crate::config::DirectConfig::stage), the
/// [`with_environment`](crate::config::DirectConfig::with_environment)
/// builder, or `THETADATA_MDDS_TYPE=STAGE`) points every channel at the
/// staging cluster: the auth request carries the staging marker, the
/// historical channel connects to the staging host, and the streaming
/// channel uses the staging hosts. Selecting [`Environment::Dev`] points
/// the streaming channel at the dev replay cluster while the auth marker
/// and historical host stay on production.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Environment {
    /// Production environment — the standard live `ThetaData` cluster.
    #[default]
    Prod,
    /// Staging environment — `ThetaData`'s staging cluster, used for
    /// validating against pre-release server changes. Less stable than
    /// production and subject to frequent reboots.
    Stage,
    /// Dev streaming environment — `ThetaData`'s dev replay cluster, which
    /// replays a random historical trading day in an infinite loop at
    /// maximum speed for development and testing when markets are closed.
    ///
    /// Dev is a streaming-only cluster: the historical channel still dials
    /// the production host (there is no dev historical), and the auth wire
    /// marker stays production (no `authEnv` is carried — dev replay has no
    /// server-side auth env of its own, so a dev session authenticates
    /// exactly as a production session). Only the streaming hosts differ.
    Dev,
}

impl Environment {
    /// Stable string label for the environment, used both for
    /// diagnostics and as the cluster selector readback. Note this is the
    /// *cluster* label, not the auth wire marker: [`Environment::Dev`]
    /// reads back as `"DEV"` even though its auth request carries no
    /// `authEnv` (it authenticates as production).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Environment::Prod => "PROD",
            Environment::Stage => "STAGE",
            Environment::Dev => "DEV",
        }
    }

    /// Parse the stable string label (case-insensitive, surrounding
    /// whitespace ignored). `"PROD"` maps to [`Environment::Prod`],
    /// `"STAGE"` to [`Environment::Stage`], and `"DEV"` to
    /// [`Environment::Dev`]; any other input returns `None`. This is the
    /// round-trip inverse of [`Self::as_str`] and is the parser behind the
    /// `THETADATA_MDDS_TYPE` env var and any string-driven binding
    /// selector.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "PROD" => Some(Environment::Prod),
            "STAGE" => Some(Environment::Stage),
            "DEV" => Some(Environment::Dev),
            _ => None,
        }
    }

    /// Historical (gRPC) host for this environment's cluster.
    ///
    /// Every cluster serves historical over TLS on port 443; only the
    /// host differs, so this is the single place that maps environment
    /// to historical host. Dev has no historical cluster of its own — it
    /// is a streaming-only environment — so it dials the production host,
    /// matching the dev preset's long-standing "historical still uses
    /// production servers" contract.
    #[must_use]
    pub(crate) fn historical_host(self) -> &'static str {
        match self {
            // Production keeps the canonical historical default in
            // `HistoricalConfig::production_defaults`; the literal here
            // mirrors it so the two never drift (a unit test asserts the
            // equality).
            Environment::Prod => "mdds-01.thetadata.us",
            Environment::Stage => "mdds-stage.thetadata.us",
            // Dev historical hits production: there is no dev historical
            // cluster, so a dev config's historical channel is identical
            // to production's.
            Environment::Dev => "mdds-01.thetadata.us",
        }
    }

    /// Streaming hosts for this environment's cluster.
    ///
    /// Production spans two NJ machines with two ports each; staging uses
    /// its own host:port set; dev uses the replay cluster on port 20200.
    /// This is the single place that maps environment to streaming hosts.
    /// Production delegates to [`StreamingConfig::production_defaults`] so
    /// the host list is never duplicated.
    #[must_use]
    pub(crate) fn streaming_hosts(self) -> Vec<(String, u16)> {
        match self {
            Environment::Prod => super::StreamingConfig::production_defaults().hosts,
            // Source: config.toml fpss_stage_hosts
            Environment::Stage => vec![
                ("nj-a.thetadata.us".to_string(), 20100),
                ("test-server.thetadata.us".to_string(), 20100),
                ("test-server.thetadata.us".to_string(), 20101),
            ],
            // Source: config.toml fpss_dev_hosts. The dev replay cluster
            // replays a random historical trading day in an infinite loop
            // at maximum speed; see `DirectConfig::dev`.
            Environment::Dev => vec![
                ("nj-a.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20201),
            ],
        }
    }
}

impl std::str::FromStr for Environment {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| {
            crate::error::Error::config_invalid(
                "environment",
                format!("environment must be one of \"PROD\", \"STAGE\", \"DEV\"; got {s:?}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_prod() {
        assert_eq!(Environment::default(), Environment::Prod);
    }

    #[test]
    fn label_matches_cluster_selector() {
        assert_eq!(Environment::Prod.as_str(), "PROD");
        assert_eq!(Environment::Stage.as_str(), "STAGE");
        assert_eq!(Environment::Dev.as_str(), "DEV");
    }

    #[test]
    fn parse_round_trips_label_case_insensitively() {
        use std::str::FromStr;
        for env in [Environment::Prod, Environment::Stage, Environment::Dev] {
            assert_eq!(Environment::parse(env.as_str()), Some(env));
            assert_eq!(Environment::from_str(env.as_str()).unwrap(), env);
        }
        assert_eq!(Environment::parse("prod"), Some(Environment::Prod));
        assert_eq!(Environment::parse("  stage  "), Some(Environment::Stage));
        assert_eq!(Environment::parse("StAgE"), Some(Environment::Stage));
        assert_eq!(Environment::parse("dev"), Some(Environment::Dev));
        assert_eq!(Environment::parse("  DeV "), Some(Environment::Dev));
        assert_eq!(Environment::parse("bogus"), None);
        assert_eq!(Environment::parse(""), None);
        assert!(Environment::from_str("bogus").is_err());
    }

    #[test]
    fn prod_cluster_matches_canonical_defaults() {
        use crate::config::{HistoricalConfig, StreamingConfig};
        // The Prod cluster literal in `historical_host` must mirror the
        // canonical historical default so the two never drift, and the
        // streaming hosts must equal the production streaming defaults.
        assert_eq!(
            Environment::Prod.historical_host(),
            HistoricalConfig::production_defaults().host
        );
        assert_eq!(
            Environment::Prod.streaming_hosts(),
            StreamingConfig::production_defaults().hosts
        );
    }

    #[test]
    fn stage_cluster_uses_staging_hosts() {
        assert_eq!(
            Environment::Stage.historical_host(),
            "mdds-stage.thetadata.us"
        );
        assert_eq!(
            Environment::Stage.streaming_hosts(),
            vec![
                ("nj-a.thetadata.us".to_string(), 20100),
                ("test-server.thetadata.us".to_string(), 20100),
                ("test-server.thetadata.us".to_string(), 20101),
            ]
        );
    }

    #[test]
    fn dev_cluster_streams_on_replay_hosts_but_serves_historical_from_production() {
        // Dev is streaming-only: historical dials the production host (no
        // dev historical cluster exists), while streaming dials the dev
        // replay cluster on port 20200. Pin both as a regression guard —
        // these must stay byte-identical to the dev preset's host values.
        assert_eq!(Environment::Dev.historical_host(), "mdds-01.thetadata.us");
        assert_eq!(
            Environment::Dev.historical_host(),
            Environment::Prod.historical_host(),
            "dev historical must equal production's — there is no dev historical cluster"
        );
        assert_eq!(
            Environment::Dev.streaming_hosts(),
            vec![
                ("nj-a.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20201),
            ]
        );
    }
}
