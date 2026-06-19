//! Target environment selector for `ThetaData` server access.
//!
//! Selects which `ThetaData` server cluster the SDK connects to. The
//! default is [`Environment::Prod`]; [`Environment::Stage`] points the
//! auth, historical, and streaming channels at the staging cluster.
//!
//! The selector is set as a unit by the [`DirectConfig`] presets:
//! [`DirectConfig::production`] selects [`Environment::Prod`] and
//! [`DirectConfig::stage`] selects [`Environment::Stage`] along with the
//! matching staging hosts.
//!
//! [`DirectConfig`]: crate::config::DirectConfig
//! [`DirectConfig::production`]: crate::config::DirectConfig::production
//! [`DirectConfig::stage`]: crate::config::DirectConfig::stage

/// Which `ThetaData` server environment the SDK targets.
///
/// Defaults to [`Environment::Prod`]. Selecting [`Environment::Stage`]
/// (via [`DirectConfig::stage`](crate::config::DirectConfig::stage))
/// points every channel at the staging cluster: the auth request carries
/// the staging marker, the historical channel connects to the staging
/// host, and the streaming channel uses the staging hosts.
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
}
