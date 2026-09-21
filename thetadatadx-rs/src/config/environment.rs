//! Per-channel target environment selectors for `ThetaData` server access.
//!
//! The SDK drives two independent server channels, and each has its own
//! environment set:
//!
//! * The market-data channel runs in [`MarketDataEnvironment::Prod`] or
//!   [`MarketDataEnvironment::Stage`]. The market-data environment also drives
//!   the auth wire marker (the `authEnv` object on the Nexus auth request):
//!   staging carries the staging marker, production carries none.
//! * The streaming channel runs in [`StreamingEnvironment::Prod`],
//!   [`StreamingEnvironment::Stage`] or [`StreamingEnvironment::Dev`]. The
//!   streaming environment selects only the streaming hosts; it never affects
//!   auth.
//!
//! The two are chosen independently: a config can be market-data-staging with
//! streaming-production, market-data-production with streaming-dev, and so on.
//! There is no market-data dev cluster; the enums encode exactly the
//! environments each channel supports.
//!
//! The selectors are set by the [`DirectConfig`] presets:
//! [`DirectConfig::production`] selects production on both channels;
//! [`DirectConfig::stage`] selects market-data-staging while streaming stays on
//! production; [`DirectConfig::dev`] selects streaming-dev while market-data
//! stays on production. The streaming channel has a staging cluster of its own,
//! which no preset selects because it is the live feed on a build that reboots
//! often; it is chosen explicitly. They can also be chosen directly with
//! [`DirectConfig::with_market_data_environment`] /
//! [`DirectConfig::with_streaming_environment`], or via the
//! `THETADATA_MARKET_DATA_TYPE` (`PROD` / `STAGE`) and `THETADATA_STREAMING_TYPE`
//! (`PROD` / `STAGE` / `DEV`) environment variables.
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
/// This value selects only the streaming hosts and has no effect on auth — a
/// session on any of them authenticates exactly as a production one. Defaults
/// to [`StreamingEnvironment::Prod`].
///
/// Selected with [`DirectConfig::dev`](crate::config::DirectConfig::dev) or
/// [`DirectConfig::stage`](crate::config::DirectConfig::stage), the
/// [`with_streaming_environment`](crate::config::DirectConfig::with_streaming_environment)
/// builder, or `THETADATA_STREAMING_TYPE=DEV` / `=STAGE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum StreamingEnvironment {
    /// Production streaming cluster — the standard live `ThetaData` cluster.
    #[default]
    Prod,
    /// Staging streaming cluster, which runs the server build bound for
    /// production. It carries the live feed rather than a replay, and is
    /// rebooted often, so it is for validating against pre-release server
    /// changes and not for data a caller depends on.
    Stage,
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
            StreamingEnvironment::Stage => "STAGE",
            StreamingEnvironment::Dev => "DEV",
        }
    }

    /// Every streaming environment the SDK ships, so a caller enumerating
    /// them cannot silently miss one.
    ///
    /// The TLS hostname allowlist is checked against this list, and a host
    /// missing from the allowlist fails the handshake with `NotValidForName`,
    /// which costs that environment its failover. A variant added here without
    /// its hosts being allowlisted trips that check rather than shipping a
    /// dead environment.
    ///
    /// Test-only, like [`DirectConfig::dev_streaming_hosts`]: production code
    /// selects one environment rather than walking them all, and the
    /// enumeration exists so the checks cannot be written against a list that
    /// quietly falls behind the enum.
    ///
    /// [`DirectConfig::dev_streaming_hosts`]: crate::config::DirectConfig
    #[cfg(test)]
    pub(crate) const ALL: &'static [Self] = &[Self::Prod, Self::Stage, Self::Dev];

    /// Parse the stable string label (case-insensitive, surrounding whitespace
    /// ignored). `"PROD"`, `"STAGE"` and `"DEV"` map to their variants; any
    /// other input returns `None`. The round-trip inverse of [`Self::as_str`]
    /// and the parser behind `THETADATA_STREAMING_TYPE`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "PROD" => Some(StreamingEnvironment::Prod),
            "STAGE" => Some(StreamingEnvironment::Stage),
            "DEV" => Some(StreamingEnvironment::Dev),
            _ => None,
        }
    }

    /// Streaming hosts for this environment's cluster.
    ///
    /// Production spans two machines with two ports each; staging is on port
    /// 20100 and dev on 20200. This is the single place that maps the
    /// streaming environment to its hosts. Production delegates to
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
            // The staging cluster runs the build bound for production and
            // carries the live feed. Its host list has the same shape as dev's
            // in the terminal's own config, with `test-server` failovers that
            // resolve only inside ThetaData's network, so the same single
            // publicly reachable host is all an external caller can dial.
            StreamingEnvironment::Stage => vec![("nj-a.thetadata.us".to_string(), 20100)],
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
                format!(
                    "streaming environment must be one of \"PROD\", \"STAGE\", \"DEV\"; \
                     got {s:?}"
                ),
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
        for env in StreamingEnvironment::ALL {
            assert_eq!(StreamingEnvironment::parse(env.as_str()), Some(*env));
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
    fn the_market_data_channel_rejects_the_streaming_only_env() {
        use std::str::FromStr;
        // There is no market-data dev cluster. A streaming-only value must NOT
        // silently fall back — it parses to None and `from_str` yields a typed
        // error naming the valid set.
        assert_eq!(MarketDataEnvironment::parse("DEV"), None);
        assert!(MarketDataEnvironment::from_str("DEV").is_err());
        assert!(MarketDataEnvironment::from_str("bogus").is_err());
        assert!(StreamingEnvironment::from_str("bogus").is_err());
        assert!(StreamingEnvironment::from_str("").is_err());
    }

    #[test]
    fn all_holds_every_streaming_environment_exactly_once() {
        // `ALL` is what the TLS hostname-allowlist coverage test enumerates,
        // so an environment missing from it ships without its hosts ever
        // being checked against the allowlist. The labels come from the
        // exhaustive `as_str` match, which a new variant does not compile
        // without, so a variant that never reached `ALL` is short a label
        // here.
        let labels: Vec<&str> = StreamingEnvironment::ALL
            .iter()
            .map(|e| e.as_str())
            .collect();
        assert_eq!(
            labels,
            vec!["PROD", "STAGE", "DEV"],
            "every streaming environment is listed once, in a stable order"
        );
        for env in StreamingEnvironment::ALL {
            assert!(
                !env.hosts().is_empty(),
                "{env:?} has no hosts to dial, so selecting it cannot connect"
            );
        }
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
    fn stage_streaming_uses_the_staging_host() {
        assert_eq!(
            StreamingEnvironment::Stage.hosts(),
            vec![("nj-a.thetadata.us".to_string(), 20100)]
        );
    }

    #[test]
    fn dev_streaming_uses_replay_hosts() {
        assert_eq!(
            StreamingEnvironment::Dev.hosts(),
            vec![("nj-a.thetadata.us".to_string(), 20200),]
        );
    }
}
