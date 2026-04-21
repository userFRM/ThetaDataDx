//! HTTP authentication against the `ThetaData` Nexus API.
//!
//! # Protocol (from decompiled Java — `AuthenticationManager.authenticateViaCloud()`)
//!
//! The Java terminal authenticates by `POSTing` to the Nexus API:
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

// -- Constants (from decompiled Java) --

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

// -- Request / Response types --

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
///
/// `Debug` is implemented manually so `session_id` (a bearer token used in
/// every MDDS request) is never written to logs.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    /// Session UUID — the primary auth token for MDDS gRPC requests.
    pub session_id: String,

    /// User details (subscription level, etc.).
    pub user: Option<AuthUser>,

    /// ISO 8601 timestamp of session creation.
    pub session_created: Option<String>,
}

impl std::fmt::Debug for AuthResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `user` carries the caller's email; keep it behind the same
        // `<redacted>` marker the `Credentials` Debug impl uses so
        // panics / `tracing::error!("{:?}", resp)` / crash dumps cannot
        // leak the email.
        f.debug_struct("AuthResponse")
            .field("session_id", &"***")
            .field("user", &"<redacted>")
            .field("session_created", &self.session_created)
            .finish()
    }
}

/// User info returned by the Nexus auth endpoint.
///
/// The Nexus API returns per-asset subscription tiers. The Java terminal uses
/// these to compute concurrency limits: `2^tier` where FREE=0, VALUE=1,
/// STANDARD=2, PROFESSIONAL=3.
///
/// `Debug` is implemented manually so `email` never lands in panic
/// output / tracing diagnostics / FFI `repr()`. The subscription
/// tiers are safe to print (integers with no PII).
#[derive(Deserialize)]
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

impl std::fmt::Debug for AuthUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthUser")
            .field("email", &"<redacted>")
            .field("stock_subscription", &self.stock_subscription)
            .field("options_subscription", &self.options_subscription)
            .field("indices_subscription", &self.indices_subscription)
            .field(
                "interest_rate_subscription",
                &self.interest_rate_subscription,
            )
            .finish()
    }
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
    #[must_use]
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
        .unwrap_or(0);
        let tier = usize::try_from(tier).unwrap_or(0);
        1usize << tier // 2^tier: 1, 2, 4, 8
    }
}

// -- Redaction helpers --

/// Compute a diagnostic-safe prefix of `email` for log output.
///
/// Returns up to the first 3 characters of the local part followed by
/// `...@<domain>`. Empty or malformed addresses yield `<redacted>`.
///
/// The prefix is enough for operators to correlate a request with a
/// tenant without exposing the full address in structured logs /
/// crash reports.
fn redacted_email_prefix(email: &str) -> String {
    let Some((local, domain)) = email.split_once('@') else {
        return "<redacted>".to_string();
    };
    if local.is_empty() || domain.is_empty() {
        return "<redacted>".to_string();
    }
    let prefix: String = local.chars().take(3).collect();
    format!("{prefix}...@{domain}")
}

// -- Public API --

/// Maximum number of retry attempts for transient network errors during auth.
const AUTH_MAX_RETRIES: u32 = 3;

/// Delay between auth retry attempts.
const AUTH_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Check whether a `reqwest` error is a transient network error worth retrying.
///
/// Returns `true` for connection refused, timeouts, and DNS failures.
/// Returns `false` for auth failures (wrong password) and server-side rejections.
fn is_transient_network_error(err: &reqwest::Error) -> bool {
    err.is_connect() || err.is_timeout()
}

/// Authenticate against the default Nexus URL. Delegates to
/// [`authenticate_at`] with the hardcoded URL constant.
///
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub async fn authenticate(creds: &Credentials) -> Result<AuthResponse, Error> {
    authenticate_at(NEXUS_AUTH_URL, creds).await
}

/// Authenticate against the Nexus API and return the session info.
///
/// This performs the same HTTP POST as the Java terminal's
/// `AuthenticationManager.authenticateViaCloud()`, but against a caller-
/// supplied URL. Used by auto-refresh and by deployments that redirect
/// auth to a staging cluster via [`crate::config::ENV_NEXUS_URL`].
///
/// The returned `AuthResponse.session_id` is a UUID string that must be
/// embedded in every MDDS gRPC request as `QueryInfo.auth_token.session_uuid`.
///
/// Transient network errors (connection refused, timeout, DNS failure) are
/// retried up to 3 times with 2-second delays. Auth failures (wrong password,
/// invalid credentials) are NOT retried.
///
/// # Errors
///
/// Returns an error on network, authentication, or parsing failure.
pub async fn authenticate_at(url: &str, creds: &Credentials) -> Result<AuthResponse, Error> {
    metrics::counter!("thetadatadx.auth.requests").increment(1);
    let auth_start = std::time::Instant::now();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| Error::Auth {
            kind: crate::error::AuthErrorKind::NetworkError,
            message: format!("failed to build HTTP client: {e}"),
        })?;

    let body = AuthRequest {
        email: &creds.email,
        password: &creds.password,
    };

    // Log the email prefix rather than the full address: full emails in
    // structured logs / crash reports are recoverable PII even when the
    // password is zeroized. The prefix is enough for operators to
    // correlate a request with a tenant without exposing the full
    // address.
    tracing::debug!(
        email_prefix = %redacted_email_prefix(&creds.email),
        url = url,
        "authenticating against Nexus API"
    );

    // Retry loop for transient network errors (connection refused, timeout, DNS).
    // Auth failures (wrong password, 401/404) are NOT retried.
    let mut last_err: Option<reqwest::Error> = None;
    let resp = 'retry: {
        for attempt in 1..=AUTH_MAX_RETRIES {
            match client
                .post(url)
                .header(TERMINAL_KEY_HEADER, TERMINAL_KEY)
                .header("Accept", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => break 'retry r,
                Err(e) if is_transient_network_error(&e) && attempt < AUTH_MAX_RETRIES => {
                    tracing::warn!(
                        attempt,
                        max = AUTH_MAX_RETRIES,
                        error = %e,
                        delay_secs = AUTH_RETRY_DELAY.as_secs(),
                        "Nexus auth request failed (transient), retrying"
                    );
                    last_err = Some(e);
                    tokio::time::sleep(AUTH_RETRY_DELAY).await;
                }
                Err(e) => {
                    let kind = if e.is_timeout() {
                        crate::error::AuthErrorKind::Timeout
                    } else if e.is_connect() {
                        crate::error::AuthErrorKind::NetworkError
                    } else {
                        crate::error::AuthErrorKind::ServerError
                    };
                    return Err(Error::Auth {
                        kind,
                        message: format!("Nexus API request failed: {e}"),
                    });
                }
            }
        }
        // All retries exhausted (should not reach here, but handle as a safety net).
        return Err(Error::Auth {
            kind: crate::error::AuthErrorKind::NetworkError,
            message: format!(
                "Nexus API request failed after {AUTH_MAX_RETRIES} retries: {}",
                last_err.map_or_else(|| "unknown".to_string(), |e| e.to_string())
            ),
        });
    };

    let status = resp.status();
    // Java special-cases 401 and 404 as "invalid credentials".
    // Source: AuthenticationManager.authenticateViaCloud() in decompiled terminal.
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::NOT_FOUND {
        return Err(Error::Auth {
            kind: crate::error::AuthErrorKind::InvalidCredentials,
            message: "invalid credentials (server returned 401/404)".into(),
        });
    }
    if !status.is_success() {
        let body_text = resp
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable>".to_string());
        return Err(Error::Auth {
            kind: crate::error::AuthErrorKind::ServerError,
            message: format!("Nexus API returned HTTP {status}: {body_text}"),
        });
    }

    let auth: AuthResponse = resp.json().await.map_err(|e| Error::Auth {
        kind: crate::error::AuthErrorKind::ServerError,
        message: format!("failed to parse Nexus API response: {e}"),
    })?;

    // Validate the session UUID is well-formed.
    let _uuid = Uuid::parse_str(&auth.session_id).map_err(|e| Error::Auth {
        kind: crate::error::AuthErrorKind::ServerError,
        message: format!(
            "Nexus API returned invalid session UUID '{}': {e}",
            auth.session_id
        ),
    })?;

    tracing::debug!(
        session_id_prefix = %&auth.session_id[..8.min(auth.session_id.len())],
        "authenticated successfully (session_id redacted)"
    );

    metrics::histogram!("thetadatadx.auth.latency_ms")
        .record(auth_start.elapsed().as_secs_f64() * 1_000.0);

    Ok(auth)
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
    fn auth_response_debug_redacts_session_id() {
        let resp = AuthResponse {
            session_id: "11111111-2222-3333-4444-555555555555".to_string(),
            user: None,
            session_created: None,
        };
        let dbg = format!("{resp:?}");
        assert!(!dbg.contains("11111111"), "session_id leaked: {dbg}");
        assert!(dbg.contains("***"), "session_id not redacted: {dbg}");
    }

    /// Finding #5 coverage: the `AuthResponse` Debug impl must NOT leak
    /// the caller's email even when a populated `user` is carried.
    /// Before this fix the nested `AuthUser.email` field rendered
    /// in-place, dumping the address into panic output / crash logs /
    /// tracing diagnostics whenever `Debug::fmt(&resp)` ran.
    #[test]
    fn auth_response_debug_redacts_user_email() {
        let resp = AuthResponse {
            session_id: "22222222-3333-4444-5555-666666666666".to_string(),
            user: Some(AuthUser {
                email: Some("user@example.com".to_string()),
                stock_subscription: Some(3),
                options_subscription: Some(2),
                indices_subscription: None,
                interest_rate_subscription: None,
            }),
            session_created: None,
        };
        let dbg = format!("{resp:?}");
        assert!(
            !dbg.contains("user@example.com"),
            "AuthResponse Debug leaked email: {dbg}"
        );
        assert!(
            dbg.contains("<redacted>"),
            "AuthResponse Debug missing redaction marker: {dbg}"
        );
    }

    /// Finding #5 coverage: the `AuthUser` Debug impl must NOT leak
    /// the caller's email when rendered standalone (e.g. when the
    /// struct is placed inside a `#[derive(Debug)]` parent elsewhere
    /// in the crate).
    #[test]
    fn auth_user_debug_redacts_email() {
        let user = AuthUser {
            email: Some("sensitive@test.com".to_string()),
            stock_subscription: Some(1),
            options_subscription: None,
            indices_subscription: None,
            interest_rate_subscription: None,
        };
        let dbg = format!("{user:?}");
        assert!(
            !dbg.contains("sensitive@test.com"),
            "AuthUser Debug leaked email: {dbg}"
        );
        assert!(
            dbg.contains("<redacted>"),
            "AuthUser Debug missing redaction marker: {dbg}"
        );
        // Subscription tiers must still render (they're safe operator
        // diagnostics and not PII).
        assert!(
            dbg.contains("stock_subscription"),
            "AuthUser Debug must still expose subscription tiers: {dbg}"
        );
    }

    /// Finding #5 coverage for the tracing path: `redacted_email_prefix`
    /// is the helper that replaces the raw `email = %creds.email`
    /// field. Returns at most 3 chars of the local part, then
    /// `...@<domain>`. The local part is never rendered in full.
    #[test]
    fn redacted_email_prefix_truncates_local_part() {
        let p = redacted_email_prefix("alice@example.com");
        assert_eq!(p, "ali...@example.com");
        assert!(
            !p.contains("alice"),
            "redacted prefix must not contain the full local part: {p}"
        );
    }

    #[test]
    fn redacted_email_prefix_handles_short_local_part() {
        // Local parts shorter than 3 chars render in full; they carry
        // so little identifying information that masking is wasted,
        // but the domain is the load-bearing part for tenant
        // correlation anyway.
        let p = redacted_email_prefix("ab@test.com");
        assert_eq!(p, "ab...@test.com");
    }

    #[test]
    fn redacted_email_prefix_rejects_malformed_input() {
        // No `@` -> not a recoverable address shape -> fall back to
        // the same `<redacted>` marker the Debug impls use.
        assert_eq!(redacted_email_prefix("no-at-sign"), "<redacted>");
        assert_eq!(redacted_email_prefix("@missing-local"), "<redacted>");
        assert_eq!(redacted_email_prefix("missing-domain@"), "<redacted>");
        assert_eq!(redacted_email_prefix(""), "<redacted>");
    }
}
