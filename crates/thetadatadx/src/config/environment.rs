//! Per-channel target environment selectors for `ThetaData` server access.
//!
//! The SDK drives two independent server channels, and each has its own
//! environment set:
//!
//! * The historical (MDDS) channel runs in [`HistoricalEnvironment::Prod`] or
//!   [`HistoricalEnvironment::Stage`]. The historical environment also drives
//!   the auth wire marker (the `authEnv` object on the Nexus auth request):
//!   staging carries the staging marker, production carries none.
//! * The streaming (FPSS) channel runs in [`StreamingEnvironment::Prod`] or
//!   [`StreamingEnvironment::Dev`]. The streaming environment selects only the
//!   streaming hosts; it never affects auth.
//!
//! The two are chosen independently: a config can be historical-staging with
//! streaming-production, historical-production with streaming-dev, and so on.
//! There is no historical dev cluster and no streaming staging cluster; the
//! enums encode exactly the environments each channel supports.
//!
//! The selectors are set by the [`DirectConfig`] presets:
//! [`DirectConfig::production`] selects production on both channels;
//! [`DirectConfig::stage`] selects historical-staging while streaming stays on
//! production; [`DirectConfig::dev`] selects streaming-dev while historical
//! stays on production. They can also be chosen directly with
//! [`DirectConfig::with_historical_environment`] /
//! [`DirectConfig::with_streaming_environment`], or via the
//! `THETADATA_MDDS_TYPE` (`PROD` / `STAGE`) and `THETADATA_FPSS_TYPE`
//! (`PROD` / `DEV`) environment variables.
//!
//! [`DirectConfig`]: crate::config::DirectConfig
//! [`DirectConfig::production`]: crate::config::DirectConfig::production
//! [`DirectConfig::stage`]: crate::config::DirectConfig::stage
//! [`DirectConfig::dev`]: crate::config::DirectConfig::dev
//! [`DirectConfig::with_historical_environment`]: crate::config::DirectConfig::with_historical_environment
//! [`DirectConfig::with_streaming_environment`]: crate::config::DirectConfig::with_streaming_environment

/// Which `ThetaData` historical (MDDS) environment the SDK targets.
///
/// The historical channel runs in production or staging only. This value also
/// drives the auth wire marker the Nexus auth request carries: staging carries
/// the staging `authEnv`, production carries none. Defaults to
/// [`HistoricalEnvironment::Prod`].
///
/// Selected with [`DirectConfig::stage`](crate::config::DirectConfig::stage),
/// the [`with_historical_environment`](crate::config::DirectConfig::with_historical_environment)
/// builder, or `THETADATA_MDDS_TYPE=STAGE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum HistoricalEnvironment {
    /// Production historical cluster — the standard live `ThetaData` cluster.
    #[default]
    Prod,
    /// Staging historical cluster, used for validating against pre-release
    /// server changes. Less stable than production and subject to frequent
    /// reboots. Authenticates with the staging marker.
    Stage,
}

/// Which `ThetaData` streaming (FPSS) environment the SDK targets.
///
/// The streaming channel runs in production or dev only. This value selects
/// only the streaming hosts and has no effect on auth — a dev session
/// authenticates exactly as a production session. Defaults to
/// [`StreamingEnvironment::Prod`].
///
/// Selected with [`DirectConfig::dev`](crate::config::DirectConfig::dev), the
/// [`with_streaming_environment`](crate::config::DirectConfig::with_streaming_environment)
/// builder, or `THETADATA_FPSS_TYPE=DEV`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StreamingEnvironment {
    /// Production streaming cluster — the standard live `ThetaData` cluster.
    #[default]
    Prod,
    /// Dev streaming cluster, which replays a random historical trading day in
    /// an infinite loop at maximum speed for development and testing when
    /// markets are closed. It is a streaming-only offering; selecting it does
    /// not change the historical channel or the auth marker.
    Dev,
}

impl HistoricalEnvironment {
    /// Stable string label, used for diagnostics and as the selector readback.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            HistoricalEnvironment::Prod => "PROD",
            HistoricalEnvironment::Stage => "STAGE",
        }
    }

    /// Parse the stable string label (case-insensitive, surrounding whitespace
    /// ignored). `"PROD"` maps to [`Self::Prod`] and `"STAGE"` to
    /// [`Self::Stage`]; any other input (including `"DEV"`, which the
    /// historical channel does not support) returns `None`. The round-trip
    /// inverse of [`Self::as_str`] and the parser behind `THETADATA_MDDS_TYPE`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "PROD" => Some(HistoricalEnvironment::Prod),
            "STAGE" => Some(HistoricalEnvironment::Stage),
            _ => None,
        }
    }

    /// Historical (gRPC) host for this environment's cluster.
    ///
    /// Every cluster serves historical over TLS on port 443; only the host
    /// differs, so this is the single place that maps the historical
    /// environment to its host.
    #[must_use]
    pub(crate) fn host(self) -> &'static str {
        match self {
            // Production keeps the canonical historical default in
            // `HistoricalConfig::production_defaults`; the literal here mirrors
            // it so the two never drift (a unit test asserts the equality).
            HistoricalEnvironment::Prod => "mdds-01.thetadata.us",
            HistoricalEnvironment::Stage => "mdds-stage.thetadata.us",
        }
    }
}

impl StreamingEnvironment {
    /// Stable string label, used for diagnostics and as the selector readback.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            StreamingEnvironment::Prod => "PROD",
            StreamingEnvironment::Dev => "DEV",
        }
    }

    /// Parse the stable string label (case-insensitive, surrounding whitespace
    /// ignored). `"PROD"` maps to [`Self::Prod`] and `"DEV"` to [`Self::Dev`];
    /// any other input (including `"STAGE"`, which the streaming channel does
    /// not support) returns `None`. The round-trip inverse of [`Self::as_str`]
    /// and the parser behind `THETADATA_FPSS_TYPE`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "PROD" => Some(StreamingEnvironment::Prod),
            "DEV" => Some(StreamingEnvironment::Dev),
            _ => None,
        }
    }

    /// Streaming hosts for this environment's cluster.
    ///
    /// Production spans two machines with two ports each; dev uses the replay
    /// cluster on port 20200. This is the single place that maps the streaming
    /// environment to its hosts. Production delegates to
    /// [`StreamingConfig::production_defaults`](super::StreamingConfig::production_defaults)
    /// so the host list is never duplicated.
    #[must_use]
    pub(crate) fn hosts(self) -> Vec<(String, u16)> {
        match self {
            StreamingEnvironment::Prod => super::StreamingConfig::production_defaults().hosts,
            // The dev replay cluster replays a random historical trading day in
            // an infinite loop at maximum speed; see `DirectConfig::dev`.
            StreamingEnvironment::Dev => vec![
                ("nj-a.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20201),
            ],
        }
    }
}

impl std::str::FromStr for HistoricalEnvironment {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| {
            crate::error::Error::config_invalid(
                "historical environment",
                format!("historical environment must be one of \"PROD\", \"STAGE\"; got {s:?}"),
            )
        })
    }
}

impl std::str::FromStr for StreamingEnvironment {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| {
            crate::error::Error::config_invalid(
                "streaming environment",
                format!("streaming environment must be one of \"PROD\", \"DEV\"; got {s:?}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_prod() {
        assert_eq!(
            HistoricalEnvironment::default(),
            HistoricalEnvironment::Prod
        );
        assert_eq!(StreamingEnvironment::default(), StreamingEnvironment::Prod);
    }

    #[test]
    fn labels_round_trip_case_insensitively() {
        for env in [HistoricalEnvironment::Prod, HistoricalEnvironment::Stage] {
            assert_eq!(HistoricalEnvironment::parse(env.as_str()), Some(env));
        }
        for env in [StreamingEnvironment::Prod, StreamingEnvironment::Dev] {
            assert_eq!(StreamingEnvironment::parse(env.as_str()), Some(env));
        }
        assert_eq!(
            HistoricalEnvironment::parse("  stage  "),
            Some(HistoricalEnvironment::Stage)
        );
        assert_eq!(
            StreamingEnvironment::parse("DeV"),
            Some(StreamingEnvironment::Dev)
        );
    }

    #[test]
    fn each_channel_rejects_the_other_channels_env() {
        use std::str::FromStr;
        // The historical channel has no dev; the streaming channel has no
        // stage. A cross-channel value must NOT silently fall back — it parses
        // to None and `from_str` yields a typed error naming the valid set.
        assert_eq!(HistoricalEnvironment::parse("DEV"), None);
        assert_eq!(StreamingEnvironment::parse("STAGE"), None);
        assert!(HistoricalEnvironment::from_str("DEV").is_err());
        assert!(StreamingEnvironment::from_str("STAGE").is_err());
        assert!(HistoricalEnvironment::from_str("bogus").is_err());
        assert!(StreamingEnvironment::from_str("").is_err());
    }

    #[test]
    fn prod_cluster_matches_canonical_defaults() {
        use crate::config::{HistoricalConfig, StreamingConfig};
        // The Prod literals must mirror the canonical defaults so the two never
        // drift.
        assert_eq!(
            HistoricalEnvironment::Prod.host(),
            HistoricalConfig::production_defaults().host
        );
        assert_eq!(
            StreamingEnvironment::Prod.hosts(),
            StreamingConfig::production_defaults().hosts
        );
    }

    #[test]
    fn stage_historical_uses_staging_host() {
        assert_eq!(
            HistoricalEnvironment::Stage.host(),
            "mdds-stage.thetadata.us"
        );
    }

    #[test]
    fn dev_streaming_uses_replay_hosts() {
        assert_eq!(
            StreamingEnvironment::Dev.hosts(),
            vec![
                ("nj-a.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20200),
                ("test-server.thetadata.us".to_string(), 20201),
            ]
        );
    }
}
