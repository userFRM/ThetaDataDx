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

    #[test]
    fn validate_date_valid() {
        assert!(validate_date("20240101").is_ok());
        assert!(validate_date("20231231").is_ok());
        assert!(validate_date("00000000").is_ok());
    }

    #[test]
    fn validate_date_invalid() {
        // Too short
        assert!(validate_date("2024010").is_err());
        // Too long
        assert!(validate_date("202401011").is_err());
        // Contains non-digit
        assert!(validate_date("2024-101").is_err());
        assert!(validate_date("2024Jan1").is_err());
        // Empty
        assert!(validate_date("").is_err());
        // Whitespace
        assert!(validate_date("2024 101").is_err());
    }
}
