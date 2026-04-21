//! Parameter validators invoked by generated endpoint macros.
//!
//! The canonical (two-argument) validators live in [`crate::validate`]; this
//! module wraps them in the single-argument signatures that the
//! `parsed_endpoint!` / streaming builder macros expect, so the generated code
//! can call `validate_date(&date)?` without repeating the param-name literal
//! at every call site.

use crate::error::Error;

/// Validate a date string via the canonical [`crate::validate`] module.
///
/// This wrapper adapts the two-arg canonical signature to the single-arg
/// convention used by the builder macros (where the param name is implicit).
pub(super) fn validate_date(date: &str) -> Result<(), Error> {
    crate::validate::validate_date(date, "date").map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn validate_date_valid() {
        assert_eq!(validate_date("20240101").unwrap(), ());
        assert_eq!(validate_date("20231231").unwrap(), ());
        assert_eq!(validate_date("00000000").unwrap(), ());
    }

    fn assert_validation_err(input: &str) {
        match validate_date(input) {
            // `validate::validate_date` returns `EndpointError::InvalidParams`,
            // which converts to `Error::Config` (see `impl From<EndpointError>
            // for Error` in `endpoint.rs`).
            Err(Error::Config(message)) => {
                assert!(
                    message.contains("date"),
                    "error message should name the 'date' param, got {message:?}"
                );
            }
            other => panic!("expected Error::Config(..) for input {input:?}, got {other:?}"),
        }
    }

    #[test]
    fn validate_date_invalid() {
        // Too short
        assert_validation_err("2024010");
        // Too long
        assert_validation_err("202401011");
        // Contains non-digit
        assert_validation_err("2024-101");
        assert_validation_err("2024Jan1");
        // Empty
        assert_validation_err("");
        // Whitespace
        assert_validation_err("2024 101");
    }
}
