//! Auth (Nexus) sub-configuration.

/// Default Nexus auth URL (matches the upstream production endpoint).
pub const DEFAULT_NEXUS_URL: &str = "https://nexus-api.thetadata.us/identity/terminal/auth_user";

/// Default `QueryInfo.client_type`.
pub const DEFAULT_CLIENT_TYPE: &str = "rust-thetadatadx";

/// Nexus authentication endpoint + client identifier.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Nexus auth URL. Default matches the upstream production endpoint; set
    /// [`crate::config::ENV_NEXUS_URL`] to redirect at a staging cluster.
    pub nexus_url: String,

    /// Value used for `QueryInfo.client_type`. Defaults to `"rust-thetadatadx"`;
    /// override via [`crate::config::ENV_CLIENT_TYPE`] to identify a deployment fleet
    /// in server-side dashboards.
    pub client_type: String,
}

impl AuthConfig {
    /// Production defaults — upstream Nexus URL + canonical `rust-thetadatadx` client type.
    #[must_use]
    pub fn production_defaults() -> Self {
        Self {
            nexus_url: DEFAULT_NEXUS_URL.to_string(),
            client_type: DEFAULT_CLIENT_TYPE.to_string(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self::production_defaults()
    }
}
