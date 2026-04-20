//! Credential parsing from `creds.txt`.
//!
//! # Format (from decompiled Java — `CredentialsManager.loadCredentials()`)
//!
//! The Java terminal reads `creds.txt` from the working directory:
//! - Line 1: email address (lowercased, trimmed)
//! - Line 2: password (trimmed)
//!
//! The file must contain exactly 2 non-empty lines after trimming.

use std::path::Path;

use zeroize::Zeroizing;

use crate::error::Error;

/// Raw credentials parsed from `creds.txt`.
///
/// These are used for both auth flows:
/// - **MDDS (gRPC)**: email + password are sent to Nexus API to obtain a session UUID
/// - **FPSS (TCP)**: email + password are sent directly over the TCP connection
///
/// The `password` is wrapped in [`zeroize::Zeroizing`] so the backing buffer is
/// wiped when the struct (or any clone) is dropped, preventing plaintext
/// recovery from a core dump or `/proc/<pid>/mem`.
#[derive(Clone)]
pub struct Credentials {
    /// Email address, lowercased and trimmed (matches Java `toLowerCase().trim()`).
    pub email: String,
    /// Password, trimmed. Zeroed on drop.
    pub(crate) password: Zeroizing<String>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("email", &"<redacted>")
            .field("password", &"<redacted>")
            .finish()
    }
}

impl Credentials {
    /// Parse credentials from a `creds.txt` file.
    ///
    /// # Format
    ///
    /// ```text
    /// user@example.com
    /// hunter2
    /// ```
    ///
    /// Matches the Java terminal's `CredentialsManager.loadCredentials()` behavior:
    /// email is lowercased and trimmed, password is trimmed.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path).map_err(|e| {
            Error::Config(format!(
                "failed to read credentials file {}: {}",
                path.display(),
                e
            ))
        })?;

        Self::parse(&contents)
    }

    /// Parse credentials from a string with the same format as `creds.txt`.
    ///
    /// Useful for testing and for cases where credentials come from environment
    /// variables or other sources.
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
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
        let password = lines[1].trim().to_string();

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
            email,
            password: Zeroizing::new(password),
        })
    }

    /// Get the password.
    #[must_use]
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Construct credentials directly (e.g. from environment variables).
    pub fn new(email: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            email: email.into().trim().to_lowercase(),
            password: Zeroizing::new(password.into().trim().to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let creds = Credentials::parse("user@example.com\nhunter2\n").unwrap();
        assert_eq!(creds.email, "user@example.com");
        assert_eq!(creds.password(), "hunter2");
    }

    #[test]
    fn parse_lowercases_email() {
        let creds = Credentials::parse("User@Example.COM\npassword123\n").unwrap();
        assert_eq!(creds.email, "user@example.com");
    }

    #[test]
    fn parse_trims_whitespace() {
        let creds = Credentials::parse("  user@example.com  \n  hunter2  \n").unwrap();
        assert_eq!(creds.email, "user@example.com");
        assert_eq!(creds.password(), "hunter2");
    }

    #[test]
    fn parse_ignores_extra_lines() {
        let creds = Credentials::parse("user@example.com\nhunter2\nextra line\nanother\n").unwrap();
        assert_eq!(creds.email, "user@example.com");
        assert_eq!(creds.password(), "hunter2");
    }

    #[test]
    fn parse_no_trailing_newline() {
        let creds = Credentials::parse("user@example.com\nhunter2").unwrap();
        assert_eq!(creds.email, "user@example.com");
        assert_eq!(creds.password(), "hunter2");
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
        assert_eq!(creds.email, "user@example.com");
        assert_eq!(creds.password(), "hunter2");
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

    /// Smoke test that the `Zeroizing<String>` wrapper derefs to `&str` so
    /// every existing `&creds.password` call site keeps compiling. The
    /// actual zero-on-drop behavior is covered by the `zeroize` crate's
    /// own tests; asserting on freed memory here would be UB.
    #[test]
    fn password_derefs_to_str() {
        let creds = Credentials::new("user@example.com", "hunter2");
        let borrowed: &str = &creds.password;
        assert_eq!(borrowed, "hunter2");
    }
}
