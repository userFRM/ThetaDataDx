//! HTTP authentication against the `ThetaData` Nexus API.
//!
//! # Protocol
//!
//! Authentication issues a POST to the Nexus API carrying the caller's
//! email and password plus a static terminal-identification header:
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
//! The `TD-TERMINAL-KEY` is a static UUID that identifies the terminal
//! application (not the user). It is not a user secret.
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
//! The `sessionId` UUID is attached to every subsequent historical data
//! request via `QueryInfo.auth_token.session_uuid`.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::Credentials;
use crate::error::Error;
use crate::util::random_id::validate_uuid_format;

// -- Constants --

/// Nexus API authentication endpoint.
///
/// Only used by `authenticate()` which is gated on `__internal`.
#[cfg(feature = "__internal")]
const NEXUS_AUTH_URL: &str = "https://nexus-api.thetadata.us/identity/terminal/auth_user";

/// Static terminal-identification key sent in every Nexus API request.
///
/// Identifies the terminal application (not the user). Not a user secret.
const TERMINAL_KEY: &str = "cf58ada4-4175-11f0-860f-1e2e95c79e64";

/// Header name for the terminal identification key.
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
#[non_exhaustive]
pub struct AuthResponse {
    /// Session UUID — the primary auth token for MDDS requests.
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
/// The Nexus API returns per-asset subscription tiers encoded as integers:
/// FREE=0, VALUE=1, STANDARD=2, PROFESSIONAL=3. These drive concurrency
/// limits: `2^tier` concurrent historical requests per asset class.
///
/// `Debug` is implemented manually so `email` never lands in panic
/// output / tracing diagnostics / FFI `repr()`. The subscription
/// tiers are safe to print (integers with no PII).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct AuthUser {
    // Only materialized when the `__internal` feature is enabled — external
    // consumers of `AuthUser` (workspace tools) are the only callers that
    // read this field. The crate itself only uses `max_concurrent_requests`.
    /// Account email associated with the authenticated session.
    #[cfg(feature = "__internal")]
    pub email: Option<String>,
    /// Equities subscription tier (integer: 0=FREE, 1=VALUE, 2=STANDARD, 3=PRO).
    #[serde(default)]
    pub stock_subscription: Option<i32>,
    /// Options subscription tier (integer: 0=FREE, 1=VALUE, 2=STANDARD, 3=PRO).
    #[serde(default)]
    pub options_subscription: Option<i32>,
    /// Indices subscription tier (integer: 0=FREE, 1=VALUE, 2=STANDARD, 3=PRO).
    #[serde(default)]
    pub indices_subscription: Option<i32>,
    /// Interest-rate subscription tier (integer: 0=FREE, 1=VALUE, 2=STANDARD, 3=PRO).
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
    /// Compute the maximum concurrent historical requests based on subscription tier.
    ///
    /// Returns `2^tier` where the tier is the highest across all asset classes:
    /// - FREE = 0 -> 1 concurrent request
    /// - VALUE = 1 -> 2 concurrent requests
    /// - STANDARD = 2 -> 4 concurrent requests
    /// - PROFESSIONAL/PRO = 3 -> 8 concurrent requests
    ///
    /// Out-of-range wire bytes are folded to the conservative Free=1 default.
    /// Unknown values emit a `warn` so operators can spot upstream-tier drift
    /// without crashing the auth path.
    #[must_use]
    pub fn max_concurrent_requests(&self) -> usize {
        use crate::mdds::SubscriptionTier;

        // Resolve each asset's subscription byte to a tier independently,
        // then take the highest valid tier. Mapping through `from_wire`
        // BEFORE the max is what keeps a single unrecognized byte on one
        // asset class from poisoning a legitimate tier on another and
        // silently collapsing a paid account to Free: an out-of-range
        // byte is dropped, not promoted past every real tier.
        let best = [
            self.stock_subscription,
            self.options_subscription,
            self.indices_subscription,
            self.interest_rate_subscription,
        ]
        .into_iter()
        .flatten()
        .filter_map(SubscriptionTier::from_wire)
        .max_by_key(|tier| tier.max_concurrent_requests());

        best.map_or_else(
            || {
                tracing::warn!(
                    "Nexus auth reported no recognized subscription tier; \
                     defaulting to Free=1 concurrent request",
                );
                SubscriptionTier::Free.max_concurrent_requests()
            },
            |tier| tier.max_concurrent_requests(),
        )
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
/// Propagates [`authenticate_at`]'s errors: [`Error::Auth`] for
/// rejected credentials, network failure, timeout, server error, or an
/// unparseable / malformed-UUID response.
///
/// Only available when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
pub async fn authenticate(creds: &Credentials) -> Result<AuthResponse, Error> {
    authenticate_at(NEXUS_AUTH_URL, creds).await
}

/// Build the rustls config for the auth HTTP client with an explicit ring
/// provider so the handshake needs no process-global default.
///
/// reqwest is pinned with `rustls-no-provider`, so it has no provider baked in
/// and would otherwise read `CryptoProvider::get_default()` — panicking when no
/// global default is installed. Building the config here and handing it to
/// reqwest via `use_preconfigured_tls` mirrors reqwest's own default path
/// (ring provider + platform-verifier trust roots) without depending on global
/// state. ALPN is set explicitly because a preconfigured config bypasses
/// reqwest's ALPN defaulting; `h2` + `http/1.1` matches reqwest's auto mode.
fn auth_tls_config() -> Result<rustls::ClientConfig, Error> {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let verifier = std::sync::Arc::new(
        rustls_platform_verifier::Verifier::new(provider.clone()).map_err(|e| Error::Auth {
            kind: crate::error::AuthErrorKind::NetworkError,
            message: format!("failed to build TLS trust verifier: {e}"),
        })?,
    );
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Auth {
            kind: crate::error::AuthErrorKind::NetworkError,
            message: format!("failed to build TLS config: {e}"),
        })?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// Maximum number of characters from an upstream response body carried into
/// a surfaced error. Bounds the untrusted excerpt so it cannot flood logs
/// or smuggle a large payload through the error chain.
const MAX_BODY_EXCERPT_CHARS: usize = 256;

/// Render a non-reversible marker for a session id.
///
/// The session id is a bearer token and must never appear verbatim in a
/// surfaced or logged error, even when malformed. Returns a fixed marker
/// carrying no token material.
fn redacted_session_marker() -> &'static str {
    "redacted"
}

/// Produce a bounded excerpt of an untrusted upstream body.
///
/// Keeps at most [`MAX_BODY_EXCERPT_CHARS`] characters and appends an
/// ellipsis marker when truncated. Operates on a `char` boundary so the
/// result is always valid UTF-8.
fn bound_body_excerpt(body: &str) -> String {
    match body.char_indices().nth(MAX_BODY_EXCERPT_CHARS) {
        Some((cut, _)) => format!("{}...", &body[..cut]),
        None => body.to_string(),
    }
}

/// Authenticate against the Nexus API and return the session info.
///
/// Identical to [`authenticate`] but accepts a caller-supplied URL.
/// Used by auto-refresh and by deployments that redirect auth to a
/// staging cluster via the `THETADATA_NEXUS_URL` env variable.
///
/// The returned `AuthResponse.session_id` is a UUID string that must be
/// embedded in every historical request as `QueryInfo.auth_token.session_uuid`.
///
/// Transient network errors (connection refused, timeout, DNS failure) are
/// retried up to 3 times with 2-second delays. Auth failures (wrong password,
/// invalid credentials) are NOT retried.
///
/// # Errors
///
/// Returns [`Error::Auth`] with [`AuthErrorKind::InvalidCredentials`]
/// when Nexus returns 401/404, [`AuthErrorKind::Timeout`] or
/// [`AuthErrorKind::NetworkError`] on transient connection failure after
/// retries are exhausted, and [`AuthErrorKind::ServerError`] for any
/// other non-success status, an unparseable body, or a malformed session
/// UUID.
///
/// [`AuthErrorKind::InvalidCredentials`]: crate::error::AuthErrorKind::InvalidCredentials
/// [`AuthErrorKind::Timeout`]: crate::error::AuthErrorKind::Timeout
/// [`AuthErrorKind::NetworkError`]: crate::error::AuthErrorKind::NetworkError
/// [`AuthErrorKind::ServerError`]: crate::error::AuthErrorKind::ServerError
pub async fn authenticate_at(url: &str, creds: &Credentials) -> Result<AuthResponse, Error> {
    metrics::counter!("thetadatadx.auth.requests").increment(1);
    let auth_start = std::time::Instant::now();

    let client = reqwest::Client::builder()
        .use_preconfigured_tls(auth_tls_config()?)
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
    // address. The Nexus URL itself includes routing topology that
    // operators rarely need at `debug` — keep that field at `trace`
    // verbosity so production deployments do not record it by default.
    tracing::debug!(
        email_prefix = %redacted_email_prefix(&creds.email),
        "authenticating against Nexus API"
    );
    tracing::trace!(url = url, "Nexus auth URL");

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
    // Nexus returns 401 (rejected) and 404 (unknown account) for the
    // same caller-facing condition: the supplied email/password pair is
    // not valid. Both collapse to `InvalidCredentials` so callers do not
    // retry — a bad password will not become good on a second attempt.
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
        // The upstream body is untrusted and unbounded — it may carry
        // arbitrary (or hostile) text. Carry only a bounded excerpt into
        // the surfaced error so it cannot flood logs or smuggle a large
        // payload through the error chain.
        return Err(Error::Auth {
            kind: crate::error::AuthErrorKind::ServerError,
            message: format!(
                "Nexus API returned HTTP {status}: {}",
                bound_body_excerpt(&body_text)
            ),
        });
    }

    let auth: AuthResponse = resp.json().await.map_err(|e| Error::Auth {
        kind: crate::error::AuthErrorKind::ServerError,
        message: format!("failed to parse Nexus API response: {e}"),
    })?;

    // Validate the session UUID is well-formed. The session id is a bearer
    // token; even a malformed value can be structurally near-valid, so it
    // must never be echoed verbatim into a surfaced or logged message.
    // Carry only a non-reversible redaction marker, matching the redacted
    // success path below.
    validate_uuid_format(&auth.session_id).map_err(|e| Error::Auth {
        kind: crate::error::AuthErrorKind::ServerError,
        message: format!(
            "Nexus API returned invalid session UUID ({}): {e}",
            redacted_session_marker()
        ),
    })?;

    tracing::debug!("authenticated successfully (session_id redacted)");

    metrics::histogram!("thetadatadx.auth.latency_ms")
        .record(auth_start.elapsed().as_secs_f64() * 1_000.0);

    Ok(auth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_key_is_valid_uuid() {
        validate_uuid_format(TERMINAL_KEY).expect("TERMINAL_KEY must be a valid UUID");
    }

    #[test]
    fn redacted_session_marker_carries_no_token_material() {
        // A structurally near-valid session id must not survive into the
        // marker used by the surfaced error.
        let near_valid = "11111111-2222-3333-4444-55555555555X";
        let marker = redacted_session_marker();
        assert!(
            !near_valid.contains(marker),
            "marker overlaps token material: {marker}"
        );
        assert_eq!(marker, "redacted");
    }

    #[test]
    fn bound_body_excerpt_truncates_oversized_body() {
        let body = "x".repeat(MAX_BODY_EXCERPT_CHARS * 4);
        let excerpt = bound_body_excerpt(&body);
        assert!(excerpt.ends_with("..."), "missing truncation marker");
        assert!(
            excerpt.chars().count() <= MAX_BODY_EXCERPT_CHARS + 3,
            "excerpt exceeds bound: {} chars",
            excerpt.chars().count()
        );
    }

    #[test]
    fn bound_body_excerpt_passes_through_short_body() {
        let body = "upstream is down";
        assert_eq!(bound_body_excerpt(body), body);
    }

    #[test]
    fn bound_body_excerpt_respects_char_boundaries() {
        // Multi-byte characters must not be split mid-codepoint.
        let body = "é".repeat(MAX_BODY_EXCERPT_CHARS * 2);
        let excerpt = bound_body_excerpt(&body);
        assert!(excerpt.ends_with("..."), "missing truncation marker");
        // If a byte boundary had been used this would have panicked above.
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
    #[cfg(feature = "__internal")]
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
    #[cfg(feature = "__internal")]
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

    /// Hostile wire bytes for the subscription tier must NOT panic the
    /// `max_concurrent_requests` arithmetic. The old `1usize << tier`
    /// path panicked in debug (and was UB in release) for `tier > 63`;
    /// the typed `SubscriptionTier::from_wire` fold caps it at Free=1
    /// for any unknown value.
    #[cfg(feature = "__internal")]
    #[test]
    fn max_concurrent_requests_clamps_hostile_wire_byte() {
        for bad in [-1, 4, 99, i32::MAX, i32::MIN] {
            let user = AuthUser {
                email: None,
                stock_subscription: Some(bad),
                options_subscription: None,
                indices_subscription: None,
                interest_rate_subscription: None,
            };
            // Must not panic and must fall back to Free=1.
            assert_eq!(
                user.max_concurrent_requests(),
                1,
                "hostile tier {bad} must fall back to Free=1"
            );
        }
    }

    /// Valid wire bytes (0..=3) round-trip through the typed
    /// `SubscriptionTier::max_concurrent_requests` ladder: 1, 2, 4, 8.
    #[cfg(feature = "__internal")]
    #[test]
    fn max_concurrent_requests_matches_tier_ladder() {
        for (wire, expected) in [(0, 1), (1, 2), (2, 4), (3, 8)] {
            let user = AuthUser {
                email: None,
                stock_subscription: Some(wire),
                options_subscription: None,
                indices_subscription: None,
                interest_rate_subscription: None,
            };
            assert_eq!(
                user.max_concurrent_requests(),
                expected,
                "tier wire {wire} must map to {expected} concurrent requests"
            );
        }
    }

    /// The highest tier across all asset classes wins — pin the
    /// behaviour so a regression that walks only `stock_subscription`
    /// is caught.
    #[cfg(feature = "__internal")]
    #[test]
    fn max_concurrent_requests_picks_highest_tier() {
        let user = AuthUser {
            email: None,
            stock_subscription: Some(0),
            options_subscription: Some(3),
            indices_subscription: Some(1),
            interest_rate_subscription: None,
        };
        assert_eq!(user.max_concurrent_requests(), 8);
    }

    /// A legitimate paid tier on one asset class must survive an
    /// out-of-range byte on another. The earlier `max()`-then-`from_wire`
    /// order let a hostile byte (`4`) win the raw max, fold to `None`,
    /// and collapse a Pro account (`3` → 8) to Free=1. Resolving each
    /// byte through `from_wire` first keeps the valid Pro tier.
    #[cfg(feature = "__internal")]
    #[test]
    fn max_concurrent_requests_keeps_valid_tier_despite_hostile_byte() {
        let user = AuthUser {
            email: None,
            stock_subscription: Some(4), // out of range — must be dropped
            options_subscription: Some(3), // Pro — must win
            indices_subscription: Some(99), // out of range — must be dropped
            interest_rate_subscription: None,
        };
        assert_eq!(
            user.max_concurrent_requests(),
            8,
            "a valid Pro tier must not be downgraded by an unrecognized byte"
        );
    }
}
