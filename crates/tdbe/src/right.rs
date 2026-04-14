//! Canonical parser for the option `right` parameter.
//!
//! Every user-facing input boundary (MDDS endpoints, FPSS contracts, CLI,
//! SDK surfaces, the Greeks utilities) funnels `right` strings through
//! [`parse_right`] so that the accepted vocabulary and validation rules
//! live in exactly one place.
//!
//! Lives in `tdbe` (the pure-data crate) so that the Greeks utilities can
//! reuse the same parser without `tdbe` reverse-depending on `thetadatadx`.
//! The `thetadatadx::right` module re-exports these items for back-compat.
//!
//! # Accepted input
//!
//! The parser is intentionally permissive at the input boundary to match
//! the ergonomics we expose across SDKs:
//!
//! - `"call"`, `"CALL"`, `"Call"` (any case)
//! - `"put"`, `"PUT"`, `"Put"` (any case)
//! - `"C"`, `"c"` (short-form call, our convention)
//! - `"P"`, `"p"` (short-form put, our convention)
//! - `"both"`, `"BOTH"`, `"*"` — wildcard; only valid where the endpoint
//!   supports it (e.g. snapshot / history endpoints taking `strike="0"`)
//!
//! Anything else returns [`Error::Config`](crate::error::Error::Config) with
//! a descriptive message. No silent defaults.
//!
//! # Upstream vs ours
//!
//! ThetaData's own OpenAPI spec (`https://docs.thetadata.us/openapiv3.yaml`)
//! defines request query `right` as `enum: [call, put, both]` with default
//! `both`. We extend the accepted set with short-form `C`/`P` for SDK
//! ergonomics — a strict superset, so any upstream client continues to work.

use crate::error::Error;

/// Parsed representation of the option `right` parameter.
///
/// Carries every representation downstream consumers need so that the
/// parsing logic runs exactly once per user input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedRight {
    /// Call option.
    Call,
    /// Put option.
    Put,
    /// Wildcard — both calls and puts. Only valid where the endpoint
    /// supports it (historical / snapshot endpoints that also accept
    /// `strike = "0"` as a wildcard).
    Both,
}

impl ParsedRight {
    /// Lowercase string expected by the MDDS gRPC server
    /// (`"call"` / `"put"` / `"both"`).
    #[must_use]
    pub fn as_mdds_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Put => "put",
            Self::Both => "both",
        }
    }

    /// Short-form string used in our tick dicts and drop-in REST/WS
    /// responses (`"C"` / `"P"`). `Both` has no short form — callers
    /// must reject it if they only accept a single contract.
    ///
    /// Returns `None` for `Both` so the caller can surface a proper
    /// error instead of silently picking one side.
    #[must_use]
    pub fn as_short_str(self) -> Option<&'static str> {
        match self {
            Self::Call => Some("C"),
            Self::Put => Some("P"),
            Self::Both => None,
        }
    }

    /// Boolean used by the FPSS wire protocol (`true` = call, `false` = put).
    /// `Both` is not representable on the FPSS wire and returns `None`.
    #[must_use]
    pub fn as_is_call(self) -> Option<bool> {
        match self {
            Self::Call => Some(true),
            Self::Put => Some(false),
            Self::Both => None,
        }
    }

    /// Raw FPSS wire-format byte (`67` = ASCII `'C'`, `80` = ASCII `'P'`).
    /// `Both` is not representable on the FPSS wire and returns `None`.
    #[must_use]
    pub fn as_wire_byte(self) -> Option<i32> {
        match self {
            Self::Call => Some(67),
            Self::Put => Some(80),
            Self::Both => None,
        }
    }
}

/// Parse a user-supplied `right` string.
///
/// Accepts `call`/`put`/`both`/`C`/`P`/`*` in any case. Returns
/// [`Error::Config`](crate::error::Error::Config) for anything else.
///
/// # Errors
///
/// Returns [`Error::Config`](crate::error::Error::Config) if the input does
/// not match any of the accepted forms.
///
/// # Examples
///
/// ```
/// use tdbe::right::{parse_right, ParsedRight};
///
/// assert_eq!(parse_right("C").unwrap(), ParsedRight::Call);
/// assert_eq!(parse_right("put").unwrap(), ParsedRight::Put);
/// assert_eq!(parse_right("BOTH").unwrap(), ParsedRight::Both);
/// assert_eq!(parse_right("*").unwrap(), ParsedRight::Both);
/// assert!(parse_right("xyz").is_err());
/// ```
pub fn parse_right(input: &str) -> Result<ParsedRight, Error> {
    // `*` is punctuation — handle before the lowercase dance.
    if input == "*" {
        return Ok(ParsedRight::Both);
    }

    // Lower-case once so we match `C`/`c`/`CALL`/`Call`/etc. uniformly.
    match input.to_ascii_lowercase().as_str() {
        "c" | "call" => Ok(ParsedRight::Call),
        "p" | "put" => Ok(ParsedRight::Put),
        "both" => Ok(ParsedRight::Both),
        _ => Err(Error::Config(format!(
            "invalid option right: '{input}' (expected one of: 'call', 'put', 'both', 'C', 'P', '*' -- case-insensitive)"
        ))),
    }
}

/// Parse a `right` that must resolve to a single side (call or put).
///
/// Returns [`Error::Config`](crate::error::Error::Config) if the input parses
/// to [`ParsedRight::Both`]. Use this for endpoints where the wildcard is not
/// meaningful (e.g. FPSS per-contract subscriptions, Greeks utilities).
///
/// # Errors
///
/// Returns [`Error::Config`](crate::error::Error::Config) for invalid inputs
/// and for the `both` / `*` wildcards.
pub fn parse_right_strict(input: &str) -> Result<ParsedRight, Error> {
    let parsed = parse_right(input)?;
    if matches!(parsed, ParsedRight::Both) {
        return Err(Error::Config(format!(
            "option right '{input}' resolves to 'both' but this endpoint requires a single side (call or put)"
        )));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_call_all_cases() {
        for form in ["call", "CALL", "Call", "CaLl", "C", "c"] {
            assert_eq!(
                parse_right(form).unwrap(),
                ParsedRight::Call,
                "failed on {form}"
            );
        }
    }

    #[test]
    fn accepts_put_all_cases() {
        for form in ["put", "PUT", "Put", "PuT", "P", "p"] {
            assert_eq!(
                parse_right(form).unwrap(),
                ParsedRight::Put,
                "failed on {form}"
            );
        }
    }

    #[test]
    fn accepts_both_and_wildcard() {
        assert_eq!(parse_right("both").unwrap(), ParsedRight::Both);
        assert_eq!(parse_right("BOTH").unwrap(), ParsedRight::Both);
        assert_eq!(parse_right("Both").unwrap(), ParsedRight::Both);
        assert_eq!(parse_right("*").unwrap(), ParsedRight::Both);
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["xyz", "", " ", "calls", "p ", "0", "67", "**"] {
            let err = parse_right(bad).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains("invalid option right"),
                "expected a descriptive error for '{bad}', got: {msg}"
            );
        }
    }

    #[test]
    fn mdds_projection_matches_upstream_vocabulary() {
        assert_eq!(parse_right("C").unwrap().as_mdds_str(), "call");
        assert_eq!(parse_right("p").unwrap().as_mdds_str(), "put");
        assert_eq!(parse_right("*").unwrap().as_mdds_str(), "both");
    }

    #[test]
    fn short_form_projection() {
        assert_eq!(parse_right("call").unwrap().as_short_str(), Some("C"));
        assert_eq!(parse_right("PUT").unwrap().as_short_str(), Some("P"));
        assert_eq!(parse_right("both").unwrap().as_short_str(), None);
    }

    #[test]
    fn fpss_bool_projection() {
        assert_eq!(parse_right("C").unwrap().as_is_call(), Some(true));
        assert_eq!(parse_right("P").unwrap().as_is_call(), Some(false));
        assert_eq!(parse_right("both").unwrap().as_is_call(), None);
    }

    #[test]
    fn fpss_wire_byte_projection() {
        // 'C' = 67, 'P' = 80 -- ASCII codes for the FPSS wire format.
        assert_eq!(parse_right("call").unwrap().as_wire_byte(), Some(67));
        assert_eq!(parse_right("put").unwrap().as_wire_byte(), Some(80));
        assert_eq!(parse_right("*").unwrap().as_wire_byte(), None);
    }

    #[test]
    fn strict_rejects_both() {
        assert_eq!(parse_right_strict("C").unwrap(), ParsedRight::Call);
        assert_eq!(parse_right_strict("put").unwrap(), ParsedRight::Put);

        let err = parse_right_strict("both").unwrap_err();
        assert!(format!("{err}").contains("resolves to 'both'"));

        let err = parse_right_strict("*").unwrap_err();
        assert!(format!("{err}").contains("resolves to 'both'"));

        // Still surfaces the baseline invalid-input error for garbage.
        let err = parse_right_strict("xyz").unwrap_err();
        assert!(format!("{err}").contains("invalid option right"));
    }
}
