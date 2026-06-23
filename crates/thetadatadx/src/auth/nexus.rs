//! HTTP authentication against the `ThetaData` Nexus API.
//!
//! # Protocol
//!
//! Authentication issues a POST to the Nexus API carrying either the
//! caller's email and password or an API key, plus a static
//! terminal-identification header:
//!
//! ```text
//! POST https://nexus-api.thetadata.us/identity/terminal/auth_user
//! Headers:
//!   TD-TERMINAL-KEY: cf58ada4-4175-11f0-860f-1e2e95c79e64
//!   Accept: application/json
//!   Content-Type: application/json
//! Body (email + password): {"email": "...", "password": "..."}
//! Body (API key):          {"apiKey": "..."}
//! ```
//!
//! The endpoint accepts either credential form; the two are mutually
//! exclusive on a single request. Targeting the staging cluster adds an
//! `authEnv` object (`{"envType": "STAGE"}`); production omits it and the
//! server routes the session as `PROD` by default.
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
use crate::config::HistoricalEnvironment;
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

/// Target environment marker carried on the auth request.
///
/// Serializes as the UPPERCASE string the server expects. Only the
/// staging environment is ever carried on the wire (`"STAGE"`);
/// production omits the marker entirely (the server treats an absent
/// `authEnv` as `PROD`), so the `Prod` variant exists only to keep the
/// mapping total.
#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "UPPERCASE")]
enum EnvType {
    Stage,
}

/// Nested `authEnv` object on the auth request body.
///
/// Serializes as `{"envType": "STAGE"}` and routes the session to the
/// staging cluster.
#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
struct AuthEnv {
    env_type: EnvType,
}

impl AuthEnv {
    /// Build the `authEnv` marker for the target HISTORICAL environment.
    ///
    /// The auth marker is driven by the historical environment only;
    /// the streaming environment never reaches auth. Production carries no
    /// marker: the server treats an absent `authEnv` as `PROD`, so omitting it
    /// keeps the production request body byte-identical to the long-validated
    /// shape. Only the staging historical environment serializes an explicit
    /// marker. A streaming-dev session therefore authenticates byte-identically
    /// to production, since its historical environment is production.
    fn for_environment(env: HistoricalEnvironment) -> Option<Self> {
        match env {
            HistoricalEnvironment::Prod => None,
            HistoricalEnvironment::Stage => Some(Self {
                env_type: EnvType::Stage,
            }),
        }
    }
}

/// JSON body for the auth request.
///
/// Carries either email + password or an API key; the unused credential
/// fields are skipped so the serialized body holds exactly one credential
/// form. The staging environment additionally carries an `authEnv`
/// object; production omits it so the body stays byte-identical to the
/// validated production shape.
///
/// Debug is intentionally NOT derived — `password` and `api_key` must
/// never appear in logs.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<&'a str>,
    /// Target server environment. Present only for staging; absent for
    /// production, which the server routes as `PROD` by default.
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_env: Option<AuthEnv>,
}

impl<'a> AuthRequest<'a> {
    /// Build the request body from the credential's authentication
    /// method and the target environment. An API key serializes as
    /// `{"apiKey": ...}`; an email + password serializes as
    /// `{"email": ..., "password": ...}`. The staging environment adds
    /// `"authEnv": {"envType": "STAGE"}`; production omits it.
    fn from_credentials(creds: &'a Credentials, environment: HistoricalEnvironment) -> Self {
        let auth_env = AuthEnv::for_environment(environment);
        if let Some(key) = creds.api_key_secret() {
            Self {
                email: None,
                password: None,
                api_key: Some(key),
                auth_env,
            }
        } else {
            Self {
                email: creds.email(),
                password: creds.password(),
                api_key: None,
                auth_env,
            }
        }
    }
}

/// Serialize the auth request body for `environment` using a fixed
/// credential, returning the on-the-wire JSON.
///
/// Test-only seam so callers outside this module (notably the config-layer
/// tests) can assert the auth wire body a given [`HistoricalEnvironment`] produces —
/// in particular that a dev config's body is byte-identical to production's
/// — without `AuthRequest` leaving this module's private surface. The
/// credential is irrelevant to the `authEnv` marker under test, so a fixed
/// email/password pair is used.
#[cfg(test)]
pub(crate) fn auth_request_json_for_test(environment: HistoricalEnvironment) -> serde_json::Value {
    let creds = Credentials::new("user@example.com", "hunter2");
    serde_json::to_value(AuthRequest::from_credentials(&creds, environment))
        .expect("auth request serializes")
}

/// Successful authentication response from Nexus API.
///
/// Only the fields we need are deserialized; unknown fields are ignored
/// via `#[serde(deny_unknown_fields)]` being absent.
///
/// `Debug` is implemented manually so `session_id` (a bearer token used in
/// every historical request) is never written to logs.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct AuthResponse {
    /// Session UUID, the primary auth token for historical requests.
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
pub async fn authenticate(
    creds: &Credentials,
    environment: HistoricalEnvironment,
) -> Result<AuthResponse, Error> {
    authenticate_at(NEXUS_AUTH_URL, creds, environment).await
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

/// Render a non-reversible marker for a session id.
///
/// The session id is a bearer token and must never appear verbatim in a
/// surfaced or logged error, even when malformed. Returns a fixed marker
/// carrying no token material.
fn redacted_session_marker() -> &'static str {
    "redacted"
}

/// Build the surfaced message for a non-401/404 auth failure.
///
/// The message is a pure function of the HTTP status — it never takes the
/// upstream response body as input, so it cannot mirror a credential that a
/// proxy or Nexus error reflected back from the submitted auth request. This
/// is the boundary that keeps the secret out of the error chain by
/// construction.
fn server_error_message(status: reqwest::StatusCode) -> String {
    format!(
        "authentication failed (server returned HTTP {})",
        status.as_u16()
    )
}

/// Fixed, body-free message for a malformed success (HTTP 200) response body.
///
/// A `serde_json` decode error can embed fragments of the input it failed to
/// parse — including response-body token text — into its `Display`. On the 200
/// path that body is the upstream auth payload, which can carry a session
/// token, so interpolating the decoder error would reflect that material into
/// the surfaced error and any caller log. The message is a constant with no
/// decoder text, keeping the secret out of the error chain by construction
/// (the same boundary [`server_error_message`] enforces for the non-200 path).
fn malformed_success_body_message() -> &'static str {
    "authentication response was malformed"
}

/// Authenticate against the Nexus API and return the session info.
///
/// Identical to [`authenticate`] but accepts a caller-supplied URL.
/// Used by auto-refresh and by deployments that redirect auth to a
/// staging cluster via the `THETADATA_NEXUS_URL` env variable. The
/// `environment` selects the target cluster carried on the request body.
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
pub async fn authenticate_at(
    url: &str,
    creds: &Credentials,
    environment: HistoricalEnvironment,
) -> Result<AuthResponse, Error> {
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

    let body = AuthRequest::from_credentials(creds, environment);

    // Log the email prefix rather than the full address: full emails in
    // structured logs / crash reports are recoverable PII even when the
    // password is zeroized. The prefix is enough for operators to
    // correlate a request with a tenant without exposing the full
    // address. API-key credentials may carry no email — fall back to a
    // method label so the log line still records that an auth attempt
    // ran. The Nexus URL itself includes routing topology that operators
    // rarely need at `debug` — keep that field at `trace` verbosity so
    // production deployments do not record it by default.
    let email_label = creds
        .email()
        .map_or_else(|| "<api-key>".to_string(), redacted_email_prefix);
    tracing::debug!(
        email_prefix = %email_label,
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
        // The auth REQUEST body carries the credential (password / apiKey).
        // A proxy or Nexus error that reflects the submitted JSON would
        // otherwise mirror that secret into the surfaced error and any
        // caller log. Carry only the HTTP status — never the upstream
        // response body — so the error can never interpolate untrusted text
        // that might echo a credential. (`resp` is dropped unread.)
        return Err(Error::Auth {
            kind: crate::error::AuthErrorKind::ServerError,
            message: server_error_message(status),
        });
    }

    let auth: AuthResponse = resp.json().await.map_err(|_| Error::Auth {
        kind: crate::error::AuthErrorKind::ServerError,
        // A 200 body that fails to decode must not echo the decoder error: it
        // can embed body-token text (a session token rides this payload). Carry
        // a fixed, body-free message — see `malformed_success_body_message`.
        message: malformed_success_body_message().to_string(),
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

    /// Email + password credentials serialize as `{"email", "password"}`
    /// with no `apiKey` field present.
    #[test]
    fn auth_request_serializes_email_password() {
        let creds = Credentials::new("user@example.com", "hunter2");
        let body = AuthRequest::from_credentials(&creds, HistoricalEnvironment::Prod);
        let json: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(json["email"], "user@example.com");
        assert_eq!(json["password"], "hunter2");
        assert!(
            json.get("apiKey").is_none(),
            "password body must omit apiKey: {json}"
        );
    }

    /// API-key credentials serialize as `{"apiKey"}` only, with email and
    /// password omitted entirely.
    #[test]
    fn auth_request_serializes_api_key_only() {
        let creds = Credentials::api_key("secret-key-xyz");
        let body = AuthRequest::from_credentials(&creds, HistoricalEnvironment::Prod);
        let json: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(json["apiKey"], "secret-key-xyz");
        assert!(
            json.get("email").is_none(),
            "api-key body must omit email: {json}"
        );
        assert!(
            json.get("password").is_none(),
            "api-key body must omit password: {json}"
        );
    }

    /// An API key paired with an email still serializes apiKey-only: the
    /// endpoint treats the two credential forms as mutually exclusive, so
    /// the email never rides alongside the key on the auth request.
    #[test]
    fn auth_request_api_key_with_email_stays_api_key_only() {
        let creds = Credentials::api_key_with_email("user@example.com", "secret-key-xyz");
        let body = AuthRequest::from_credentials(&creds, HistoricalEnvironment::Prod);
        let json: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(json["apiKey"], "secret-key-xyz");
        assert!(json.get("email").is_none());
        assert!(json.get("password").is_none());
    }

    /// The production environment carries NO `authEnv` marker: the server
    /// treats an absent `authEnv` as `PROD`, so omitting it keeps the
    /// production request body byte-identical to the validated shape.
    #[test]
    fn auth_request_omits_auth_env_for_prod() {
        let creds = Credentials::new("user@example.com", "hunter2");
        let body = AuthRequest::from_credentials(&creds, HistoricalEnvironment::Prod);
        let json: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert!(
            json.get("authEnv").is_none(),
            "production body must omit authEnv: {json}"
        );
    }

    /// The staging environment serializes `authEnv` as
    /// `{"envType": "STAGE"}` so the server routes the session to the
    /// staging cluster.
    #[test]
    fn auth_request_serializes_auth_env_stage() {
        let creds = Credentials::new("user@example.com", "hunter2");
        let body = AuthRequest::from_credentials(&creds, HistoricalEnvironment::Stage);
        let json: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(json["authEnv"], serde_json::json!({ "envType": "STAGE" }));
    }

    /// The auth marker is total over the historical environment, and only
    /// staging carries one: the streaming environment (including streaming
    /// dev) never reaches auth, so it cannot produce an `authEnv`. The
    /// "streaming-dev authenticates as production" invariant is therefore
    /// structural here — a dev config's historical environment is production,
    /// which omits the marker — and is pinned at the config layer where
    /// `DirectConfig::dev()` exists. This locks the production body as the
    /// only no-marker shape across both credential forms.
    #[test]
    fn only_staging_carries_an_auth_marker() {
        for creds in [
            Credentials::new("user@example.com", "hunter2"),
            Credentials::api_key("secret-key-xyz"),
        ] {
            let prod: serde_json::Value = serde_json::to_value(AuthRequest::from_credentials(
                &creds,
                HistoricalEnvironment::Prod,
            ))
            .unwrap();
            let stage: serde_json::Value = serde_json::to_value(AuthRequest::from_credentials(
                &creds,
                HistoricalEnvironment::Stage,
            ))
            .unwrap();
            assert!(
                prod.get("authEnv").is_none(),
                "production body must omit authEnv: {prod}"
            );
            assert_eq!(
                stage["authEnv"],
                serde_json::json!({ "envType": "STAGE" }),
                "staging body must carry the staging marker"
            );
        }
    }

    /// The full production body shape, pinned end to end: exactly the
    /// credential fields and nothing else — byte-identical to the
    /// validated production auth body, with no `authEnv` key.
    #[test]
    fn auth_request_full_body_shape_prod() {
        let creds = Credentials::new("user@example.com", "hunter2");
        let body = AuthRequest::from_credentials(&creds, HistoricalEnvironment::Prod);
        let json: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "email": "user@example.com",
                "password": "hunter2"
            })
        );
    }

    /// The full staging body shape: the same credential fields plus the
    /// explicit `authEnv` staging marker.
    #[test]
    fn auth_request_full_body_shape_stage() {
        let creds = Credentials::new("user@example.com", "hunter2");
        let body = AuthRequest::from_credentials(&creds, HistoricalEnvironment::Stage);
        let json: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "email": "user@example.com",
                "password": "hunter2",
                "authEnv": { "envType": "STAGE" }
            })
        );
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
    fn server_error_message_carries_status_not_body() {
        // A non-401/404 auth failure must surface only the HTTP status. The
        // upstream body can mirror the submitted auth request (which carries
        // the password / apiKey); the message is a pure function of the
        // status and never accepts that body, so a secret can never reach
        // the error chain on this path.
        let msg = server_error_message(reqwest::StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            msg.contains("500"),
            "status code missing from message: {msg}"
        );
        // Stand-ins for a reflected credential or any upstream body text.
        for leaked in ["hunter2", "td_secret_apikey", "password", "apiKey", "email"] {
            assert!(
                !msg.contains(leaked),
                "message must not carry body-derived text {leaked:?}: {msg}"
            );
        }
    }

    #[test]
    fn malformed_success_body_message_is_fixed_and_body_free() {
        // The 200-path parse-failure message must be a constant with no
        // decoder/body text. A `serde_json` error can embed fragments of the
        // body it failed to parse — and the success body carries a session
        // token — so the surfaced error must never interpolate it.
        let msg = malformed_success_body_message();
        assert_eq!(msg, "authentication response was malformed");
        // Stand-ins for a reflected token or any upstream body text that a
        // decoder error might otherwise embed.
        for leaked in [
            "11111111-2222-3333-4444-555555555555",
            "sessionId",
            "session_id",
            "td_secret_apikey",
            "expected",
            "column",
            "line",
        ] {
            assert!(
                !msg.contains(leaked),
                "fixed message must not carry body-derived text {leaked:?}: {msg}"
            );
        }
    }

    #[test]
    fn malformed_200_body_maps_to_fixed_message_without_body_content() {
        // Exercise the exact mapping the 200 path applies: a malformed body
        // that EMBEDS a session token fails to decode into `AuthResponse`, and
        // the `map_err` must collapse it to the fixed body-free message — never
        // the `serde_json` error text, which can quote the offending body.
        let token = "11111111-2222-3333-4444-555555555555";
        // Valid JSON shape-wise but wrong type for `session_id` (number, not
        // string), so the decoder error references the token-bearing field.
        let body = format!(r#"{{"sessionId": 12345, "leaked_token": "{token}"}}"#);
        let parse_result: Result<AuthResponse, _> = serde_json::from_str(&body);
        let raw_decoder_text = parse_result
            .as_ref()
            .err()
            .map(std::string::ToString::to_string)
            .unwrap_or_default();
        let err = parse_result.map_err(|_| Error::Auth {
            kind: crate::error::AuthErrorKind::ServerError,
            message: malformed_success_body_message().to_string(),
        });
        let surfaced = err.expect_err("a malformed 200 body must fail to decode");
        let surfaced_msg = surfaced.to_string();
        assert!(
            surfaced_msg.contains("authentication response was malformed"),
            "surfaced error must carry the fixed message: {surfaced_msg}"
        );
        assert!(
            !surfaced_msg.contains(token),
            "surfaced error must not reflect the body token: {surfaced_msg}"
        );
        // Sanity: the discarded raw decoder text COULD have carried body
        // material, which is exactly why the fixed message is used. (Skip the
        // assertion if a given serde version happens not to quote the field —
        // the load-bearing guarantee is the surfaced message above.)
        let _ = raw_decoder_text;
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
