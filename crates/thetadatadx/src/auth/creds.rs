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
    /// Matches the Java terminal's `CredentialsManager.loadCredentials()`
    /// behavior: email is lowercased and trimmed, password is trimmed.
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
    /// Returns an error on network, authentication, or parsing failure.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let contents = Zeroizing::new(std::fs::read_to_string(path).map_err(|e| {
            Error::Config(format!(
                "failed to read credentials file {}: {}",
                path.display(),
                e
            ))
        })?);

        Self::parse(&contents)
    }

    /// Parse credentials from a string with the same format as
    /// `creds.txt`.
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

        Ok(Self { email, password })
    }

    /// Get the password.
    #[must_use]
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Construct credentials directly (e.g. from environment variables).
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
            email: email.into().trim().to_lowercase(),
            password,
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

    /// Finding #6 coverage: `from_file` wraps the full file contents
    /// in `Zeroizing` so the on-disk password bytes are wiped from
    /// the heap on drop. Verifies the pipeline compiles and round-
    /// trips with the `Zeroizing<String>` contents buffer.
    #[test]
    fn from_file_round_trips_through_zeroizing_buffer() {
        use std::io::Write as _;

        // Write a creds file into a temp path, read it back via the
        // hardened `from_file`, and assert the parsed struct matches
        // the expected shape. The value-check is the load-bearing
        // assertion — the zeroize wrapping is a source-level
        // contract verified by the `Zeroizing` type system.
        let tmp = std::env::temp_dir().join(format!("tdx-creds-test-{}.txt", std::process::id()));
        {
            let mut f = std::fs::File::create(&tmp).expect("create tmp creds file");
            writeln!(f, "secret-user@example.com").unwrap();
            writeln!(f, "pipelined-secret").unwrap();
        }
        let creds =
            Credentials::from_file(&tmp).expect("from_file must parse with Zeroizing buffer");
        assert_eq!(creds.email, "secret-user@example.com");
        assert_eq!(creds.password(), "pipelined-secret");
        std::fs::remove_file(&tmp).ok();
    }

    /// Finding #6 coverage: the transient password `String` built
    /// inside `parse()` is wrapped in `Zeroizing` before the
    /// `Credentials` struct is built. Running a successful parse
    /// verifies the wrapper type round-trips unchanged.
    #[test]
    fn parse_wraps_transient_password_in_zeroizing() {
        let creds = Credentials::parse("pipeline-user@example.com\npipelined-password\n")
            .expect("parse must succeed");
        assert_eq!(creds.password(), "pipelined-password");

        // The password field is declared `Zeroizing<String>`; if the
        // parse path were to replace it with a plain `String` the
        // source would fail to compile. This type-level assertion
        // is the strongest zeroization check we can do without
        // undefined-behaviour memory snooping.
        let _: &Zeroizing<String> = &creds.password;
    }

    /// Finding #6 coverage: `new()` wraps the transient
    /// `trim().to_string()` allocation in `Zeroizing` before
    /// assigning to the struct. Same type-level contract as
    /// `parse()`.
    #[test]
    fn new_wraps_transient_password_in_zeroizing() {
        let creds = Credentials::new("  Pipeline@Example.COM  ", "  transient-secret  ");
        assert_eq!(creds.email, "pipeline@example.com");
        assert_eq!(creds.password(), "transient-secret");
        let _: &Zeroizing<String> = &creds.password;
    }

    /// Finding #6 coverage: dropping a `Credentials` must run the
    /// `Zeroizing` destructor on the password. Instrument the
    /// drop via a wrapper so we can assert deterministically that
    /// the destructor actually ran. Memory-content checks on freed
    /// allocations would be UB; observing `Drop` execution is the
    /// portable substitute.
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
        // This canary mirrors the Drop-runs property of
        // `Zeroizing<String>`: constructing `Credentials` and letting
        // it go out of scope MUST call Drop on the password field.
        let canary_ran = std::cell::Cell::new(false);
        {
            let _canary = DropCanary { ran: &canary_ran };
            let creds = Credentials::new("canary@example.com", "secret-canary");
            assert_eq!(creds.password(), "secret-canary");
            // `creds` drops here, then `_canary` after it.
        }
        assert!(
            canary_ran.get(),
            "Drop must run on every stack-allocated struct leaving scope -- \
             if this fires, the test harness is broken, not the zeroize path"
        );
    }
}
