//! Per-channel target environment selectors for `ThetaData` server access.
//!
//! The SDK drives two independent server channels, and each has its own
//! environment set:
//!
//! * The market-data channel runs in [`MarketDataEnvironment::Prod`] or
//!   [`MarketDataEnvironment::Stage`]. The market-data environment also drives
//!   the auth wire marker (the `authEnv` object on the Nexus auth request):
//!   staging carries the staging marker, production carries none.
//! * The streaming channel runs in [`StreamingEnvironment::Prod`] or
//!   [`StreamingEnvironment::Dev`]. The streaming environment selects only the
//!   streaming hosts; it never affects auth.
//!
//! The two are chosen independently: a config can be market-data-staging with
//! streaming-production, market-data-production with streaming-dev, and so on.
//! There is no market-data dev cluster and no streaming staging cluster; the
//! enums encode exactly the environments each channel supports.
//!
//! The selectors are set by the [`DirectConfig`] presets:
//! [`DirectConfig::production`] selects production on both channels;
//! [`DirectConfig::stage`] selects market-data-staging while streaming stays on
//! production; [`DirectConfig::dev`] selects streaming-dev while market-data
//! stays on production. They can also be chosen directly with
//! [`DirectConfig::with_market_data_environment`] /
//! [`DirectConfig::with_streaming_environment`], or via the
//! `THETADATA_MARKET_DATA_TYPE` (`PROD` / `STAGE`) and `THETADATA_STREAMING_TYPE`
//! (`PROD` / `DEV`) environment variables.
//!
//! [`DirectConfig`]: crate::config::DirectConfig
//! [`DirectConfig::production`]: crate::config::DirectConfig::production
//! [`DirectConfig::stage`]: crate::config::DirectConfig::stage
//! [`DirectConfig::dev`]: crate::config::DirectConfig::dev
//! [`DirectConfig::with_market_data_environment`]: crate::config::DirectConfig::with_market_data_environment
//! [`DirectConfig::with_streaming_environment`]: crate::config::DirectConfig::with_streaming_environment

/// Which `ThetaData` market-data environment the SDK targets.
///
/// The market-data channel runs in production or staging only. This value also
/// drives the auth wire marker the Nexus auth request carries: staging carries
/// the staging `authEnv`, production carries none. Defaults to
/// [`MarketDataEnvironment::Prod`].
///
/// Selected with [`DirectConfig::stage`](crate::config::DirectConfig::stage),
/// the [`with_market_data_environment`](crate::config::DirectConfig::with_market_data_environment)
/// builder, or `THETADATA_MARKET_DATA_TYPE=STAGE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MarketDataEnvironment {
    /// Production market-data cluster — the standard live `ThetaData` cluster.
    #[default]
    Prod,
    /// Staging market-data cluster, used for validating against pre-release
    /// server changes. Less stable than production and subject to frequent
    /// reboots. Authenticates with the staging marker.
    Stage,
}

/// Which `ThetaData` streaming environment the SDK targets.
///
/// The streaming channel runs in production or dev only. This value selects
/// only the streaming hosts and has no effect on auth — a dev session
/// authenticates exactly as a production session. Defaults to
/// [`StreamingEnvironment::Prod`].
///
/// Selected with [`DirectConfig::dev`](crate::config::DirectConfig::dev), the
/// [`with_streaming_environment`](crate::config::DirectConfig::with_streaming_environment)
/// builder, or `THETADATA_STREAMING_TYPE=DEV`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum StreamingEnvironment {
    /// Production streaming cluster — the standard live `ThetaData` cluster.
    #[default]
    Prod,
    /// Dev streaming cluster, which replays a random historical trading day in
    /// an infinite loop at maximum speed for development and testing when
    /// markets are closed. It is a streaming-only offering; selecting it does
    /// not change the market-data channel or the auth marker.
    Dev,
}

impl MarketDataEnvironment {
    /// Stable string label, used for diagnostics and as the selector readback.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MarketDataEnvironment::Prod => "PROD",
            MarketDataEnvironment::Stage => "STAGE",
        }
    }

    /// Parse the stable string label (case-insensitive, surrounding whitespace
    /// ignored). `"PROD"` maps to [`Self::Prod`] and `"STAGE"` to
    /// [`Self::Stage`]; any other input (including `"DEV"`, which the
    /// market-data channel does not support) returns `None`. The round-trip
    /// inverse of [`Self::as_str`] and the parser behind `THETADATA_MARKET_DATA_TYPE`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "PROD" => Some(MarketDataEnvironment::Prod),
            "STAGE" => Some(MarketDataEnvironment::Stage),
            _ => None,
        }
    }

    /// Market-data (gRPC) host for this environment's cluster.
    ///
    /// Every cluster serves market-data over TLS on port 443; only the host
    /// differs, so this is the single place that maps the market-data
    /// environment to its host.
    #[must_use]
    pub(crate) fn host(self) -> &'static str {
        match self {
            // Production keeps the canonical market-data default in
            // `MarketDataConfig::production_defaults`; the literal here mirrors
            // it so the two never drift (a unit test asserts the equality).
            MarketDataEnvironment::Prod => "mdds-01.thetadata.us",
            MarketDataEnvironment::Stage => "mdds-stage.thetadata.us",
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
    /// and the parser behind `THETADATA_STREAMING_TYPE`.
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
            // an infinite loop at maximum speed; see `DirectConfig::dev`. The
            // terminal's dev list also carries `test-server.thetadata.us`
            // failover hosts, but those resolve only inside ThetaData's own
            // network — an external SDK caller cannot reach them, so a shuffled
            // connect that landed on one only logged a DNS failure before
            // failing over. `nj-a.thetadata.us:20200` is the publicly
            // reachable dev host.
            StreamingEnvironment::Dev => vec![("nj-a.thetadata.us".to_string(), 20200)],
        }
    }
}

impl std::str::FromStr for MarketDataEnvironment {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| {
            crate::error::Error::config_invalid(
                "market-data environment",
                format!("market-data environment must be one of \"PROD\", \"STAGE\"; got {s:?}"),
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
            MarketDataEnvironment::default(),
            MarketDataEnvironment::Prod
        );
        assert_eq!(StreamingEnvironment::default(), StreamingEnvironment::Prod);
    }

    #[test]
    fn labels_round_trip_case_insensitively() {
        for env in [MarketDataEnvironment::Prod, MarketDataEnvironment::Stage] {
            assert_eq!(MarketDataEnvironment::parse(env.as_str()), Some(env));
        }
        for env in [StreamingEnvironment::Prod, StreamingEnvironment::Dev] {
            assert_eq!(StreamingEnvironment::parse(env.as_str()), Some(env));
        }
        assert_eq!(
            MarketDataEnvironment::parse("  stage  "),
            Some(MarketDataEnvironment::Stage)
        );
        assert_eq!(
            StreamingEnvironment::parse("DeV"),
            Some(StreamingEnvironment::Dev)
        );
    }

    #[test]
    fn each_channel_rejects_the_other_channels_env() {
        use std::str::FromStr;
        // The market-data channel has no dev; the streaming channel has no
        // stage. A cross-channel value must NOT silently fall back — it parses
        // to None and `from_str` yields a typed error naming the valid set.
        assert_eq!(MarketDataEnvironment::parse("DEV"), None);
        assert_eq!(StreamingEnvironment::parse("STAGE"), None);
        assert!(MarketDataEnvironment::from_str("DEV").is_err());
        assert!(StreamingEnvironment::from_str("STAGE").is_err());
        assert!(MarketDataEnvironment::from_str("bogus").is_err());
        assert!(StreamingEnvironment::from_str("").is_err());
    }

    #[test]
    fn prod_cluster_matches_canonical_defaults() {
        use crate::config::{MarketDataConfig, StreamingConfig};
        // The Prod literals must mirror the canonical defaults so the two never
        // drift.
        assert_eq!(
            MarketDataEnvironment::Prod.host(),
            MarketDataConfig::production_defaults().host
        );
        assert_eq!(
            StreamingEnvironment::Prod.hosts(),
            StreamingConfig::production_defaults().hosts
        );
    }

    #[test]
    fn stage_market_data_uses_staging_host() {
        assert_eq!(
            MarketDataEnvironment::Stage.host(),
            "mdds-stage.thetadata.us"
        );
    }

    #[test]
    fn dev_streaming_uses_replay_hosts() {
        assert_eq!(
            StreamingEnvironment::Dev.hosts(),
            vec![
                ("nj-a.thetadata.us".to_string(), 20200),
            ]
        );
    }
}
