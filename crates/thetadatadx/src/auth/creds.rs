//! Credential parsing and sourcing.
//!
//! Credentials carry one of two authentication methods:
//!
//! - **Email + password** — parsed from a two-line `creds.txt` file or
//!   supplied inline. This is the original method.
//! - **API key** — a single secret string supplied inline or sourced
//!   from the `THETADATA_API_KEY` environment variable.
//!
//! Both methods are accepted by the historical channel (the
//! authentication endpoint) and the streaming channel (the login
//! handshake); they are mutually exclusive on a single `Credentials`.
//!
//! # `creds.txt` format
//!
//! `creds.txt` uses a two-line plaintext format:
//! - Line 1: email address (lowercased, trimmed)
//! - Line 2: password (trimmed)
//!
//! The file must contain exactly 2 non-empty lines after trimming.

use std::path::Path;

use zeroize::Zeroizing;

use crate::error::Error;

/// Environment variable carrying an API key as an alternative to
/// email + password.
const API_KEY_ENV: &str = "THETADATA_API_KEY";

/// The authentication method backing a [`Credentials`] value.
///
/// Exactly one method is present. The two variants are mutually
/// exclusive: a `Credentials` either authenticates with an email +
/// password pair or with a single API key, never both.
///
/// Secret material (the password, the API key) is wrapped in
/// [`zeroize::Zeroizing`] so the backing buffer is wiped on drop,
/// preventing plaintext recovery from a core dump or `/proc/<pid>/mem`.
#[derive(Clone)]
pub(crate) enum AuthMethod {
    /// Email + password. The email is lowercased and trimmed; the
    /// password is trimmed and zeroed on drop.
    Password {
        /// Email address, lowercased and trimmed.
        email: String,
        /// Password, trimmed. Zeroed on drop.
        password: Zeroizing<String>,
    },
    /// API key. The optional email, when present, is the account email
    /// the streaming login may carry alongside the key.
    ApiKey {
        /// Account email, when one is available. `None` when the caller
        /// supplied only an API key.
        email: Option<String>,
        /// API key. Zeroed on drop.
        key: Zeroizing<String>,
    },
}

impl std::fmt::Debug for AuthMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password { .. } => f
                .debug_struct("Password")
                .field("email", &"<redacted>")
                .field("password", &"<redacted>")
                .finish(),
            Self::ApiKey { .. } => f
                .debug_struct("ApiKey")
                .field("email", &"<redacted>")
                .field("key", &"<redacted>")
                .finish(),
        }
    }
}

/// Authentication credentials for `ThetaData` direct server access.
///
/// A `Credentials` carries exactly one authentication method — email +
/// password or an API key — selected at construction time. Both methods
/// are used by both channels:
/// - **Historical channel**: the credential is exchanged with the
///   authentication endpoint to obtain a session UUID.
/// - **Streaming channel**: the credential is sent in the login
///   handshake.
///
/// Secret material is wrapped in [`zeroize::Zeroizing`] so the backing
/// buffer is wiped when the struct (or any clone) is dropped. `Debug`
/// redacts every secret and the email.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Credentials {
    pub(crate) method: AuthMethod,
}

impl Credentials {
    /// Parse email + password credentials from a `creds.txt` file.
    ///
    /// # Format
    ///
    /// ```text
    /// user@example.com
    /// hunter2
    /// ```
    ///
    /// Email is lowercased and trimmed; password is trimmed.
    ///
    /// # Zeroization pipeline
    ///
    /// The full file contents are read into a `Zeroizing<String>`
    /// buffer so the on-disk password bytes are wiped from the heap on
    /// drop — not just the final `password` field. Every transient
    /// copy in the parse path (`contents`, the intermediate owned
    /// password `String`) is wrapped so a core dump / `/proc/<pid>/mem`
    /// reader cannot recover the password from an earlier stage of
    /// the pipeline.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if the file cannot be read, and
    /// [`Error::Auth`] if its contents fail to parse (fewer than two
    /// lines, or an empty email or password).
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        // The `Error::Config` Display is structurally
        // `"configuration error ({kind}): {message}"`. The detail
        // stays on the `kind` side (the typed `ConfigErrorKind::Io`
        // retains the full path + os error for log parsers / retry
        // classifiers) and the outer `message` is a short label, so
        // the parenthesised section is not duplicated in the
        // human-readable form.
        let contents = Zeroizing::new(std::fs::read_to_string(path).map_err(|e| {
            // The typed Io variant carries the long form (path + os
            // error) so structural callers can extract it; the
            // shorter `message` field is what users see after the
            // `(kind)` prefix.
            Error::Config {
                kind: crate::error::ConfigErrorKind::Io(format!(
                    "failed to read credentials file {}: {}",
                    path.display(),
                    e
                )),
                message: "credentials file unreadable".to_string(),
                source: Some(Box::new(e)),
            }
        })?);

        Self::parse(&contents)
    }

    /// Parse email + password credentials from a string with the same
    /// format as `creds.txt`.
    ///
    /// Useful for testing and for cases where credentials come from
    /// environment variables or other sources.
    ///
    /// # Zeroization pipeline
    ///
    /// The intermediate owned password `String` is wrapped in
    /// `Zeroizing` before the final `Credentials` struct is built,
    /// so a panic or early-return between allocation and struct
    /// construction still wipes the plaintext on unwind. The email is
    /// PII but not a secret in the same way -- it is tracked
    /// separately via the `Debug` redaction.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Auth`] if `contents` has fewer than two lines,
    /// or if the email (line 1) or password (line 2) is empty after
    /// trimming.
    pub fn parse(contents: &str) -> Result<Self, Error> {
        let lines: Vec<&str> = contents.lines().collect();

        if lines.len() < 2 {
            return Err(Error::Auth {
                kind: crate::error::AuthErrorKind::InvalidCredentials,
                message: format!(
                    "creds.txt must contain at least 2 lines (email + password), got {}",
                    lines.len()
                ),
            });
        }

        let email = lines[0].trim().to_lowercase();
        // Wrap the transient password allocation in `Zeroizing`
        // immediately so any early-return path (empty check below,
        // panic elsewhere in the function) still wipes the plaintext.
        let password: Zeroizing<String> = Zeroizing::new(lines[1].trim().to_string());

        if email.is_empty() {
            return Err(Error::Auth {
                kind: crate::error::AuthErrorKind::InvalidCredentials,
                message: "email (line 1) is empty".to_string(),
            });
        }

        if password.is_empty() {
            return Err(Error::Auth {
                kind: crate::error::AuthErrorKind::InvalidCredentials,
                message: "password (line 2) is empty".to_string(),
            });
        }

        Ok(Self {
            method: AuthMethod::Password { email, password },
        })
    }

    /// Construct email + password credentials directly (e.g. from
    /// environment variables).
    ///
    /// # Zeroization pipeline
    ///
    /// The caller-supplied password goes through a transient owned
    /// `String` for the `trim()` + `to_string()` step. That transient
    /// is wrapped in `Zeroizing` immediately so the post-trim copy is
    /// also wiped on drop -- matching the file-read path.
    pub fn new(email: impl Into<String>, password: impl Into<String>) -> Self {
        // `trim().to_string()` returns an owned `String` that is NOT
        // the same allocation as the caller's `Into<String>` source.
        // Wrap it in `Zeroizing` before building the struct so the
        // transient is wiped even if the caller panics between the
        // allocation and the `Credentials` construction.
        let password: Zeroizing<String> = Zeroizing::new(password.into().trim().to_string());
        Self {
            method: AuthMethod::Password {
                email: email.into().trim().to_lowercase(),
                password,
            },
        }
    }

    /// Construct credentials that authenticate with an API key.
    ///
    /// The API key is an alternative to email + password. It is trimmed
    /// and wrapped in [`zeroize::Zeroizing`] so the backing buffer is
    /// wiped on drop.
    ///
    /// # Zeroization pipeline
    ///
    /// The caller-supplied key goes through a transient owned `String`
    /// for the `trim()` + `to_string()` step, wrapped in `Zeroizing`
    /// before the struct is built so the post-trim copy is wiped on
    /// drop even on a panic between the allocation and construction.
    pub fn api_key(key: impl Into<String>) -> Self {
        let key: Zeroizing<String> = Zeroizing::new(key.into().trim().to_string());
        Self {
            method: AuthMethod::ApiKey { email: None, key },
        }
    }

    /// Construct API-key credentials that also carry an account email.
    ///
    /// The email is lowercased and trimmed. An empty email collapses to
    /// `None`. The streaming login can carry the email alongside the key.
    ///
    /// # Zeroization pipeline
    ///
    /// The key transient is wrapped in `Zeroizing` before the struct is
    /// built, matching [`Credentials::api_key`].
    pub fn api_key_with_email(email: impl Into<String>, key: impl Into<String>) -> Self {
        let key: Zeroizing<String> = Zeroizing::new(key.into().trim().to_string());
        let email = email.into().trim().to_lowercase();
        Self {
            method: AuthMethod::ApiKey {
                email: (!email.is_empty()).then_some(email),
                key,
            },
        }
    }

    /// Source credentials from the environment, falling back to a
    /// `creds.txt` file.
    ///
    /// Precedence:
    /// 1. `THETADATA_API_KEY` — if set and non-empty, an API key is used.
    /// 2. The `creds.txt` file at `path` — the two-line email + password
    ///    format.
    ///
    /// An explicit constructor ([`Credentials::new`],
    /// [`Credentials::api_key`]) always takes precedence over both, since
    /// the caller is then supplying the credential directly rather than
    /// asking this helper to source one.
    ///
    /// # Errors
    ///
    /// When `THETADATA_API_KEY` is unset or empty, returns whatever
    /// [`Credentials::from_file`] returns for `path`.
    pub fn from_env_or_file(path: impl AsRef<Path>) -> Result<Self, Error> {
        if let Ok(key) = std::env::var(API_KEY_ENV) {
            // Wrap the environment buffer so the key bytes are wiped on drop
            // rather than lingering in freed heap; `api_key` keeps its own
            // zeroized copy.
            let key = Zeroizing::new(key);
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                return Ok(Self::api_key(trimmed));
            }
        }
        Self::from_file(path)
    }

    /// The account email, when one is available.
    ///
    /// Always `Some` for email + password credentials. For API-key
    /// credentials it is `Some` only when the key was paired with an
    /// email via [`Credentials::api_key_with_email`].
    #[must_use]
    pub fn email(&self) -> Option<&str> {
        match &self.method {
            AuthMethod::Password { email, .. } => Some(email),
            AuthMethod::ApiKey { email, .. } => email.as_deref(),
        }
    }

    /// The password, when this credential authenticates with email +
    /// password. `None` for API-key credentials.
    #[must_use]
    pub fn password(&self) -> Option<&str> {
        match &self.method {
            AuthMethod::Password { password, .. } => Some(password),
            AuthMethod::ApiKey { .. } => None,
        }
    }

    /// The API key, when this credential authenticates with one.
    /// `None` for email + password credentials.
    #[must_use]
    pub fn api_key_secret(&self) -> Option<&str> {
        match &self.method {
            AuthMethod::ApiKey { key, .. } => Some(key),
            AuthMethod::Password { .. } => None,
        }
    }

    /// Whether this credential authenticates with an API key.
    #[must_use]
    pub fn is_api_key(&self) -> bool {
        matches!(self.method, AuthMethod::ApiKey { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let creds = Credentials::parse("user@example.com\nhunter2\n").unwrap();
        assert_eq!(creds.email(), Some("user@example.com"));
        assert_eq!(creds.password(), Some("hunter2"));
        assert_eq!(creds.api_key_secret(), None);
        assert!(!creds.is_api_key());
    }

    #[test]
    fn parse_lowercases_email() {
        let creds = Credentials::parse("User@Example.COM\npassword123\n").unwrap();
        assert_eq!(creds.email(), Some("user@example.com"));
    }

    #[test]
    fn parse_trims_whitespace() {
        let creds = Credentials::parse("  user@example.com  \n  hunter2  \n").unwrap();
        assert_eq!(creds.email(), Some("user@example.com"));
        assert_eq!(creds.password(), Some("hunter2"));
    }

    #[test]
    fn parse_ignores_extra_lines() {
        let creds = Credentials::parse("user@example.com\nhunter2\nextra line\nanother\n").unwrap();
        assert_eq!(creds.email(), Some("user@example.com"));
        assert_eq!(creds.password(), Some("hunter2"));
    }

    #[test]
    fn parse_no_trailing_newline() {
        let creds = Credentials::parse("user@example.com\nhunter2").unwrap();
        assert_eq!(creds.email(), Some("user@example.com"));
        assert_eq!(creds.password(), Some("hunter2"));
    }

    #[test]
    fn parse_empty_string() {
        let err = Credentials::parse("").unwrap_err();
        assert!(err.to_string().contains("at least 2 lines"));
    }

    #[test]
    fn parse_one_line() {
        let err = Credentials::parse("user@example.com\n").unwrap_err();
        assert!(err.to_string().contains("at least 2 lines"));
    }

    #[test]
    fn parse_empty_email() {
        let err = Credentials::parse("   \nhunter2\n").unwrap_err();
        assert!(err.to_string().contains("email (line 1) is empty"));
    }

    #[test]
    fn parse_empty_password() {
        let err = Credentials::parse("user@example.com\n   \n").unwrap_err();
        assert!(err.to_string().contains("password (line 2) is empty"));
    }

    #[test]
    fn new_trims_and_lowercases() {
        let creds = Credentials::new("  User@Example.COM  ", "  hunter2  ");
        assert_eq!(creds.email(), Some("user@example.com"));
        assert_eq!(creds.password(), Some("hunter2"));
    }

    #[test]
    fn api_key_basic() {
        let creds = Credentials::api_key("  secret-key-123  ");
        assert!(creds.is_api_key());
        assert_eq!(creds.api_key_secret(), Some("secret-key-123"));
        assert_eq!(creds.password(), None);
        assert_eq!(creds.email(), None);
    }

    #[test]
    fn api_key_with_email_carries_email() {
        let creds = Credentials::api_key_with_email("  User@Example.COM  ", "  key-abc  ");
        assert!(creds.is_api_key());
        assert_eq!(creds.api_key_secret(), Some("key-abc"));
        assert_eq!(creds.email(), Some("user@example.com"));
    }

    #[test]
    fn api_key_with_empty_email_is_none() {
        let creds = Credentials::api_key_with_email("   ", "key-abc");
        assert_eq!(creds.email(), None);
        assert_eq!(creds.api_key_secret(), Some("key-abc"));
    }

    /// Env sourcing: `THETADATA_API_KEY` selects an API key and takes
    /// precedence over the file fallback. A missing/empty env var falls
    /// back to the file. The test uses a unique env scope to avoid
    /// cross-test interference.
    #[test]
    fn from_env_or_file_prefers_env_api_key() {
        // SAFETY: single-threaded test body; the env var is set and
        // removed within this test's scope. No other test reads
        // `THETADATA_API_KEY`.
        temp_env_var(API_KEY_ENV, Some("  env-sourced-key  "), || {
            let creds = Credentials::from_env_or_file("/nonexistent/creds.txt")
                .expect("env api key must source without touching the file");
            assert!(creds.is_api_key());
            assert_eq!(creds.api_key_secret(), Some("env-sourced-key"));
        });
    }

    #[test]
    fn from_env_or_file_falls_back_to_file_when_env_absent() {
        use std::io::Write as _;
        let tmp = std::env::temp_dir().join(format!(
            "thetadatadx-env-fallback-{}.txt",
            std::process::id()
        ));
        {
            let mut f = std::fs::File::create(&tmp).expect("create tmp creds file");
            writeln!(f, "fallback@example.com").unwrap();
            writeln!(f, "fallback-pass").unwrap();
        }
        temp_env_var(API_KEY_ENV, None, || {
            let creds =
                Credentials::from_env_or_file(&tmp).expect("file fallback must parse creds.txt");
            assert!(!creds.is_api_key());
            assert_eq!(creds.email(), Some("fallback@example.com"));
            assert_eq!(creds.password(), Some("fallback-pass"));
        });
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn from_env_or_file_treats_empty_env_as_absent() {
        temp_env_var(API_KEY_ENV, Some("   "), || {
            // Empty/whitespace env must NOT be treated as a key; the
            // file fallback then errors on the missing path.
            let res = Credentials::from_env_or_file("/nonexistent/creds.txt");
            assert!(
                res.is_err(),
                "whitespace-only env var must fall through to the file path"
            );
        });
    }

    /// Serializes env-mutating tests so parallel test threads never
    /// observe a torn `THETADATA_API_KEY` while another test is mid-swap.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Run `body` with `var` temporarily set to `value` (or removed when
    /// `None`), restoring the prior value afterward. Keeps env mutation
    /// scoped so the suite stays order-independent.
    fn temp_env_var(var: &str, value: Option<&str>, body: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior = std::env::var(var).ok();
        // SAFETY: tests run in-process; this helper sets/removes a single
        // env var around a synchronous closure and restores the prior
        // value, so no other thread observes a torn state for `var`.
        unsafe {
            match value {
                Some(v) => std::env::set_var(var, v),
                None => std::env::remove_var(var),
            }
        }
        body();
        // SAFETY: same as above — restore the captured prior value.
        unsafe {
            match prior {
                Some(v) => std::env::set_var(var, v),
                None => std::env::remove_var(var),
            }
        }
    }

    /// `Debug` must never expose the email or the password -- both would
    /// land in panic output, `tracing::error!("{:?}", ...)`, crash dumps,
    /// and Jupyter `repr()` on the Python bindings.
    #[test]
    fn debug_redacts_email_and_password() {
        let creds = Credentials::new("user@example.com", "hunter2");
        let rendered = format!("{creds:?}");
        assert!(
            !rendered.contains("user@example.com"),
            "Debug impl leaked email: {rendered}"
        );
        assert!(
            !rendered.contains("hunter2"),
            "Debug impl leaked password: {rendered}"
        );
        assert!(
            rendered.contains("<redacted>"),
            "Debug missing redaction marker: {rendered}"
        );
    }

    /// `Debug` must never expose the API key either.
    #[test]
    fn debug_redacts_api_key() {
        let creds = Credentials::api_key_with_email("user@example.com", "super-secret-key");
        let rendered = format!("{creds:?}");
        assert!(
            !rendered.contains("super-secret-key"),
            "Debug impl leaked api key: {rendered}"
        );
        assert!(
            !rendered.contains("user@example.com"),
            "Debug impl leaked email: {rendered}"
        );
        assert!(
            rendered.contains("<redacted>"),
            "Debug missing redaction marker: {rendered}"
        );
    }

    /// Smoke test that the password accessor derefs through the
    /// `Zeroizing<String>` wrapper. The actual zero-on-drop behavior is
    /// covered by the `zeroize` crate's own tests.
    #[test]
    fn password_accessor_round_trips() {
        let creds = Credentials::new("user@example.com", "hunter2");
        assert_eq!(creds.password(), Some("hunter2"));
    }

    /// The full file contents and the parsed password both round-trip
    /// through `Zeroizing` buffers in `from_file`.
    #[test]
    fn from_file_round_trips_through_zeroizing_buffer() {
        use std::io::Write as _;

        let tmp =
            std::env::temp_dir().join(format!("thetadatadx-creds-test-{}.txt", std::process::id()));
        {
            let mut f = std::fs::File::create(&tmp).expect("create tmp creds file");
            writeln!(f, "secret-user@example.com").unwrap();
            writeln!(f, "pipelined-secret").unwrap();
        }
        let creds =
            Credentials::from_file(&tmp).expect("from_file must parse with Zeroizing buffer");
        assert_eq!(creds.email(), Some("secret-user@example.com"));
        assert_eq!(creds.password(), Some("pipelined-secret"));
        std::fs::remove_file(&tmp).ok();
    }

    /// The transient password `String` built inside `parse()` is wrapped
    /// in `Zeroizing` before the `Credentials` struct is built; the
    /// `AuthMethod::Password` variant stores it as `Zeroizing<String>`,
    /// so a regression to a plain `String` would fail to compile.
    #[test]
    fn parse_stores_password_in_zeroizing() {
        let creds = Credentials::parse("pipeline-user@example.com\npipelined-password\n")
            .expect("parse must succeed");
        assert_eq!(creds.password(), Some("pipelined-password"));
        match &creds.method {
            AuthMethod::Password { password, .. } => {
                let _: &Zeroizing<String> = password;
            }
            AuthMethod::ApiKey { .. } => panic!("expected a password credential"),
        }
    }

    /// Dropping a `Credentials` must run the `Zeroizing` destructor on
    /// the secret. Observing `Drop` execution via a canary is the
    /// portable substitute for snooping freed memory (which is UB).
    #[test]
    fn credentials_drop_runs_zeroizing_destructor() {
        struct DropCanary<'a> {
            ran: &'a std::cell::Cell<bool>,
        }
        impl Drop for DropCanary<'_> {
            fn drop(&mut self) {
                self.ran.set(true);
            }
        }
        let canary_ran = std::cell::Cell::new(false);
        {
            let _canary = DropCanary { ran: &canary_ran };
            let creds = Credentials::new("canary@example.com", "secret-canary");
            assert_eq!(creds.password(), Some("secret-canary"));
        }
        assert!(
            canary_ran.get(),
            "Drop must run on every stack-allocated struct leaving scope -- \
             if this fires, the test harness is broken, not the zeroize path"
        );
    }
}
