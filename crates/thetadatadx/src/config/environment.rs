//! Target environment selector for `ThetaData` server access.
//!
//! Selects which `ThetaData` server cluster the SDK connects to. The
//! default is [`Environment::Prod`]; [`Environment::Stage`] points the
//! auth, historical, and streaming channels at the staging cluster.
//!
//! The selector is set as a unit by the [`DirectConfig`] presets:
//! [`DirectConfig::production`] selects [`Environment::Prod`] and
//! [`DirectConfig::stage`] selects [`Environment::Stage`] along with the
//! matching staging hosts. It can also be chosen directly with
//! [`DirectConfig::with_environment`], or via the `THETADATA_MDDS_TYPE`
//! environment variable (`PROD` / `STAGE`).
//!
//! [`DirectConfig`]: crate::config::DirectConfig
//! [`DirectConfig::production`]: crate::config::DirectConfig::production
//! [`DirectConfig::stage`]: crate::config::DirectConfig::stage
//! [`DirectConfig::with_environment`]: crate::config::DirectConfig::with_environment

/// Which `ThetaData` server environment the SDK targets.
///
/// Defaults to [`Environment::Prod`]. Selecting [`Environment::Stage`]
/// (via [`DirectConfig::stage`](crate::config::DirectConfig::stage), the
/// [`with_environment`](crate::config::DirectConfig::with_environment)
/// builder, or `THETADATA_MDDS_TYPE=STAGE`) points every channel at the
/// staging cluster: the auth request carries the staging marker, the
/// historical channel connects to the staging host, and the streaming
/// channel uses the staging hosts.
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
}

impl Environment {
    /// Stable string label for the environment, used both for
    /// diagnostics and as the value carried on the auth request.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Environment::Prod => "PROD",
            Environment::Stage => "STAGE",
        }
    }

    /// Parse the stable string label (case-insensitive, surrounding
    /// whitespace ignored). `"PROD"` maps to [`Environment::Prod`] and
    /// `"STAGE"` to [`Environment::Stage`]; any other input returns
    /// `None`. This is the round-trip inverse of [`Self::as_str`] and is
    /// the parser behind the `THETADATA_MDDS_TYPE` env var and any
    /// string-driven binding selector.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "PROD" => Some(Environment::Prod),
            "STAGE" => Some(Environment::Stage),
            _ => None,
        }
    }

    /// Historical (gRPC) host for this environment's cluster.
    ///
    /// Both clusters serve historical over TLS on port 443; only the
    /// host differs, so this is the single place that maps environment
    /// to historical host.
    #[must_use]
    pub(crate) fn historical_host(self) -> &'static str {
        match self {
            // Production keeps the canonical historical default in
            // `HistoricalConfig::production_defaults`; the literal here
            // mirrors it so the two never drift (a unit test asserts the
            // equality).
            Environment::Prod => "mdds-01.thetadata.us",
            Environment::Stage => "mdds-stage.thetadata.us",
        }
    }

    /// Streaming hosts for this environment's cluster.
    ///
    /// Production spans two NJ machines with two ports each; staging uses
    /// its own host:port set. This is the single place that maps
    /// environment to streaming hosts. Production delegates to
    /// [`StreamingConfig::production_defaults`] so the host list is never
    /// duplicated.
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
        }
    }
}

impl std::str::FromStr for Environment {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| {
            crate::error::Error::config_invalid(
                "environment",
                format!("environment must be one of \"PROD\", \"STAGE\"; got {s:?}"),
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
    fn label_matches_wire_value() {
        assert_eq!(Environment::Prod.as_str(), "PROD");
        assert_eq!(Environment::Stage.as_str(), "STAGE");
    }

    #[test]
    fn parse_round_trips_label_case_insensitively() {
        use std::str::FromStr;
        for env in [Environment::Prod, Environment::Stage] {
            assert_eq!(Environment::parse(env.as_str()), Some(env));
            assert_eq!(Environment::from_str(env.as_str()).unwrap(), env);
        }
        assert_eq!(Environment::parse("prod"), Some(Environment::Prod));
        assert_eq!(Environment::parse("  stage  "), Some(Environment::Stage));
        assert_eq!(Environment::parse("StAgE"), Some(Environment::Stage));
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
}
