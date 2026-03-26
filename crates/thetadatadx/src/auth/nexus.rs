//! HTTP authentication against the ThetaData Nexus API.
//!
//! # Protocol (from decompiled Java — `AuthenticationManager.authenticateViaCloud()`)
//!
//! The Java terminal authenticates by POSTing to the Nexus API:
//!
//! ```text
//! POST https://nexus-api.thetadata.us/identity/terminal/auth_user
//! Headers:
//!   TD-TERMINAL-KEY: cf58ada4-4175-11f0-860f-1e2e95c79e64
//!   Accept: application/json
//!   Content-Type: application/json
//! Body: {"email": "...", "password": "..."}
//! ```
//!
//! The `TD-TERMINAL-KEY` is a hardcoded UUID in the Java terminal that identifies
//! the terminal application itself (not the user). Found in `AuthenticationManager`
//! as a static final field. This is NOT a secret — it ships in every copy of the
//! terminal JAR.
//!
//! # Response
//!
//! ```json
//! {
//!   "sessionId": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
//!   "user": {
//!     "email": "...",
//!     "subscriptionLevel": "...",
//!     ...
//!   },
//!   "sessionCreated": "2024-01-01T00:00:00Z"
//! }
//! ```
//!
//! The `sessionId` UUID is then embedded in every MDDS gRPC request via
//! `QueryInfo.auth_token.session_uuid`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Credentials;
use crate::error::Error;

// ── Constants (from decompiled Java) ──

/// Nexus API authentication endpoint.
///
/// Source: `AuthenticationManager.CLOUD_AUTH_URL` in decompiled terminal.
const NEXUS_AUTH_URL: &str = "https://nexus-api.thetadata.us/identity/terminal/auth_user";

/// Terminal identification key sent in every Nexus API request.
///
/// Source: `AuthenticationManager.TERMINAL_KEY` — hardcoded UUID that identifies
/// the terminal application. Ships in every copy of the JAR; not a user secret.
const TERMINAL_KEY: &str = "cf58ada4-4175-11f0-860f-1e2e95c79e64";

/// Header name for the terminal key.
///
/// Source: `AuthenticationManager.authenticateViaCloud()` in decompiled terminal.
const TERMINAL_KEY_HEADER: &str = "TD-TERMINAL-KEY";

// ── Request / Response types ──

/// JSON body for the auth request.
///
/// Debug is intentionally NOT derived — `password` must never appear in logs.
#[derive(Serialize)]
struct AuthRequest<'a> {
    email: &'a str,
    password: &'a str,
}

/// Successful authentication response from Nexus API.
///
/// Only the fields we need are deserialized; unknown fields are ignored
/// via `#[serde(deny_unknown_fields)]` being absent.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    /// Session UUID — the primary auth token for MDDS gRPC requests.
    pub session_id: String,

    /// User details (subscription level, etc.).
    pub user: Option<AuthUser>,

    /// ISO 8601 timestamp of session creation.
    pub session_created: Option<String>,
}

/// User info returned by the Nexus auth endpoint.
///
/// The Nexus API returns per-asset subscription tiers. The Java terminal uses
/// these to compute concurrency limits: `2^tier` where FREE=0, VALUE=1,
/// STANDARD=2, PROFESSIONAL=3.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUser {
    pub email: Option<String>,
    /// Per-asset subscription tiers (integer: 0=FREE, 1=VALUE, 2=STANDARD, 3=PRO).
    #[serde(default)]
    pub stock_subscription: Option<i32>,
    #[serde(default)]
    pub options_subscription: Option<i32>,
    #[serde(default)]
    pub indices_subscription: Option<i32>,
    #[serde(default)]
    pub interest_rate_subscription: Option<i32>,
}

impl AuthUser {
    /// Compute the maximum concurrent gRPC requests based on subscription tier.
    ///
    /// Returns `2^tier` where the tier is the highest across all asset classes:
    /// - FREE = 0 -> 1 concurrent request
    /// - VALUE = 1 -> 2 concurrent requests
    /// - STANDARD = 2 -> 4 concurrent requests
    /// - PROFESSIONAL/PRO = 3 -> 8 concurrent requests
    ///
    /// Source: Java terminal `MddsConnectionManager` — `2^subscription_tier`.
    pub fn max_concurrent_requests(&self) -> usize {
        let tier = [
            self.stock_subscription,
            self.options_subscription,
            self.indices_subscription,
            self.interest_rate_subscription,
        ]
        .iter()
        .filter_map(|s| *s)
        .max()
        .unwrap_or(0) as usize;
        1usize << tier // 2^tier: 1, 2, 4, 8
    }
}

// ── Public API ──

/// Authenticate against the Nexus API and return the session info.
///
/// This performs the same HTTP POST as the Java terminal's
/// `AuthenticationManager.authenticateViaCloud()`.
///
/// The returned `AuthResponse.session_id` is a UUID string that must be
/// embedded in every MDDS gRPC request as `QueryInfo.auth_token.session_uuid`.
pub async fn authenticate(creds: &Credentials) -> Result<AuthResponse, Error> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| Error::Auth(format!("failed to build HTTP client: {e}")))?;

    let body = AuthRequest {
        email: &creds.email,
        password: &creds.password,
    };

    tracing::debug!(
        email = %creds.email,
        url = NEXUS_AUTH_URL,
        "authenticating against Nexus API"
    );

    let resp = client
        .post(NEXUS_AUTH_URL)
        .header(TERMINAL_KEY_HEADER, TERMINAL_KEY)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Auth(format!("Nexus API request failed: {e}")))?;

    let status = resp.status();
    // Java special-cases 401 and 404 as "invalid credentials".
    // Source: AuthenticationManager.authenticateViaCloud() in decompiled terminal.
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::NOT_FOUND {
        return Err(Error::Auth(
            "invalid credentials (server returned 401/404)".into(),
        ));
    }
    if !status.is_success() {
        let body_text = resp
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable>".to_string());
        return Err(Error::Auth(format!(
            "Nexus API returned HTTP {status}: {body_text}"
        )));
    }

    let auth: AuthResponse = resp
        .json()
        .await
        .map_err(|e| Error::Auth(format!("failed to parse Nexus API response: {e}")))?;

    // Validate the session UUID is well-formed.
    let _uuid = Uuid::parse_str(&auth.session_id).map_err(|e| {
        Error::Auth(format!(
            "Nexus API returned invalid session UUID '{}': {e}",
            auth.session_id
        ))
    })?;

    tracing::debug!(
        session_id_prefix = %&auth.session_id[..8.min(auth.session_id.len())],
        "authenticated successfully (session_id redacted)"
    );

    Ok(auth)
}

/// The session UUID parsed from an `AuthResponse`.
///
/// Thin wrapper to ensure the UUID was validated at parse time.
#[derive(Debug, Clone)]
pub struct SessionToken {
    /// The raw UUID string, as returned by the Nexus API.
    pub session_uuid: String,
}

impl SessionToken {
    /// Extract and validate the session token from an auth response.
    pub fn from_response(resp: &AuthResponse) -> Result<Self, Error> {
        let _uuid = Uuid::parse_str(&resp.session_id)
            .map_err(|e| Error::Auth(format!("invalid session UUID '{}': {e}", resp.session_id)))?;

        Ok(Self {
            session_uuid: resp.session_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_key_is_valid_uuid() {
        // Sanity check: the hardcoded terminal key should be a valid UUID.
        Uuid::parse_str(TERMINAL_KEY).expect("TERMINAL_KEY must be a valid UUID");
    }

    #[test]
    fn session_token_from_valid_response() {
        let resp = AuthResponse {
            session_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            user: None,
            session_created: None,
        };
        let token = SessionToken::from_response(&resp).unwrap();
        assert_eq!(token.session_uuid, resp.session_id);
    }

    #[test]
    fn session_token_rejects_garbage() {
        let resp = AuthResponse {
            session_id: "not-a-uuid".to_string(),
            user: None,
            session_created: None,
        };
        let err = SessionToken::from_response(&resp).unwrap_err();
        assert!(err.to_string().contains("invalid session UUID"));
    }
}
