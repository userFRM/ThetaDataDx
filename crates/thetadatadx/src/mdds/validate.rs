//! Canonical parameter validation for `thetadatadx`.
//!
//! Every runtime check (date format, symbol format, interval legality,
//! option right, year, strike) lives here as the single source of truth.
//! Both the shared endpoint runtime ([`crate::mdds::endpoint_args`]) and
//! the MDDS client macros ([`crate::mdds::macros`]) delegate to these
//! functions.
//!
//! Build-time validators in `build_support/endpoints/` operate on the
//! TOML surface spec and proto schema — a fundamentally different
//! domain — so they remain separate.
//!
//! Wave 3 merged the previous top-level `crate::validate` and the
//! single-arg `crate::mdds::validate` adapter into this module. The
//! two-argument canonical validators sit at the top; the single-arg
//! adapters used by the generated builder macros sit below.

use crate::error::Error;
use crate::mdds::endpoint_args::EndpointError;
use crate::mdds::wire_semantics::is_iso_date;

// -- Canonical validators (two-arg, take a parameter name for diagnostics) --

pub(crate) fn validate_date(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be exactly 8 digits (YYYYMMDD), got: '{value}'"
        )));
    }
    // Shape passed; now apply the calendar check. Rejects the
    // `00000000` sentinel and impossible dates like `20260230` or
    // `19990431` that the shape-only check used to silently accept.
    // The leap-year / month-length logic lives in `tdbe::time` so MDDS
    // and FPSS share one canonical Gregorian validator (H3 + H4).
    let yyyymmdd: i32 = value.parse().map_err(|_| {
        EndpointError::InvalidParams(format!(
            "'{param_name}' must be 8 digits (YYYYMMDD), got: '{value}'"
        ))
    })?;
    if !tdbe::time::is_valid_yyyymmdd(yyyymmdd) {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' is not a valid Gregorian date (YYYYMMDD with year 1900-2100, valid month, day-of-month including 4/100/400 leap rule), got: '{value}'"
        )));
    }
    Ok(())
}

/// Validate `expiration`: accepts `YYYY-MM-DD`, `YYYYMMDD`, `*`, or the
/// legacy `"0"` wildcard (translated to `*` in
/// [`crate::mdds::wire_semantics::normalize_expiration`]).
pub(crate) fn validate_expiration(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if matches!(value, "*" | "0") || is_iso_date(value) {
        return Ok(());
    }
    validate_date(value, param_name).map_err(|_| {
        EndpointError::InvalidParams(format!(
            "'{param_name}' must be '*' (wildcard), '0' (legacy wildcard), 'YYYYMMDD', or 'YYYY-MM-DD', got: '{value}'"
        ))
    })
}

/// Validate the `strike` parameter.
///
/// Upstream documents `strike` as a decimal price string (e.g. `"550"`,
/// `"17.5"`) or `*` for all strikes, with `*` as the documented default.
/// We additionally accept `"0"` and the empty string as ergonomic
/// wildcard forms. Wildcards become proto-unset in
/// [`crate::mdds::wire_semantics::wire_strike_opt`] so the server applies
/// its documented default.
pub(crate) fn validate_strike(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.is_empty() || matches!(value, "*" | "0") {
        return Ok(());
    }
    match value.parse::<f64>() {
        Ok(n) if n.is_finite() && n > 0.0 => Ok(()),
        _ => Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be '*' (wildcard), '0' (legacy wildcard), or a positive decimal (e.g. '550' or '17.5'), got: '{value}'"
        ))),
    }
}

pub(crate) fn validate_symbol(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.is_empty() {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be non-empty"
        )));
    }
    Ok(())
}

pub(crate) fn validate_interval(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be a non-empty alphanumeric string (e.g. '60000' or '1m'), got: '{value}'"
        )));
    }
    Ok(())
}

pub(crate) fn validate_right(value: &str, param_name: &str) -> Result<(), EndpointError> {
    // Delegate to the canonical parser so the accepted vocabulary stays
    // in one place. The endpoint layer does not distinguish
    // Call/Put/Both here -- per-endpoint logic in the MDDS client
    // decides whether `both` / `*` is meaningful -- so we only care
    // about "is this parseable at all".
    tdbe::right::parse_right(value).map(|_| ()).map_err(|_| {
        EndpointError::InvalidParams(format!(
            "'{param_name}' must be one of: 'call', 'put', 'both', 'C', 'P', '*' (case-insensitive), got: '{value}'"
        ))
    })
}

pub(crate) fn validate_year(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.len() != 4 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be exactly 4 digits (YYYY), got: '{value}'"
        )));
    }
    Ok(())
}

pub(crate) fn parse_symbols(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn parse_bool(value: &str) -> Result<bool, &'static str> {
    if value.eq_ignore_ascii_case("true") || value == "1" {
        Ok(true)
    } else if value.eq_ignore_ascii_case("false") || value == "0" {
        Ok(false)
    } else {
        Err("accepted values are true, false, 1, or 0")
    }
}

// -- Single-arg adapter used by the generated builder macros --
//
// The two-arg canonical [`validate_date`] above takes a parameter name
// for diagnostics; the generated MDDS endpoint code emits
// `validate_date(&arg)?` and wants a one-arg form. The generated
// `mdds/endpoints.rs` calls into a `validate_date_required` helper
// re-exported below — same code path, single-arg shape.

/// Validate a date string for the generated builder macros.
///
/// Wraps the two-arg canonical [`validate_date`] in the single-arg
/// signature the `parsed_endpoint!` and streaming-builder macros
/// expect (the param name is implicit at the call site).
pub(super) fn validate_date_required(date: &str) -> Result<(), Error> {
    validate_date(date, "date").map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn validate_date_required_valid() {
        assert!(validate_date_required("20240101").is_ok());
        assert!(validate_date_required("20231231").is_ok());
        // Leap-year boundaries — the calendar check accepts these.
        assert!(validate_date_required("20240229").is_ok());
        assert!(validate_date_required("20000229").is_ok());
    }

    #[test]
    fn validate_date_required_rejects_impossible_calendar_dates() {
        // The exact garbage shapes the H3 codex finding called out:
        // shape-only validation used to silently accept these.
        assert!(validate_date_required("00000000").is_err());
        assert!(validate_date_required("20260230").is_err()); // Feb 30
        assert!(validate_date_required("19990431").is_err()); // Apr 31
        assert!(validate_date_required("20231300").is_err()); // month 13
        assert!(validate_date_required("19000229").is_err()); // /100 non-leap
        assert!(validate_date_required("18991231").is_err()); // year < 1900
        assert!(validate_date_required("21010101").is_err()); // year > 2100
    }

    fn assert_validate_date_required_err(input: &str) {
        match validate_date_required(input) {
            // `validate_date` returns `EndpointError::InvalidParams`,
            // which converts to `Error::Config` (see `impl
            // From<EndpointError> for Error` in `mdds::endpoint_args`).
            Err(Error::Config { message, .. }) => {
                assert!(
                    message.contains("date"),
                    "error message should name the 'date' param, got {message:?}"
                );
            }
            other => panic!("expected Error::Config {{ .. }} for input {input:?}, got {other:?}"),
        }
    }

    #[test]
    fn validate_date_required_invalid() {
        // Too short
        assert_validate_date_required_err("2024010");
        // Too long
        assert_validate_date_required_err("202401011");
        // Contains non-digit
        assert_validate_date_required_err("2024-101");
        assert_validate_date_required_err("2024Jan1");
        // Empty
        assert_validate_date_required_err("");
        // Whitespace
        assert_validate_date_required_err("2024 101");
    }

    #[test]
    fn expiration_accepts_documented_vocab_and_rejects_garbage() {
        for good in ["*", "0", "20260417", "2026-04-17"] {
            assert!(validate_expiration(good, "expiration").is_ok(), "{good}");
        }
        for bad in [
            "",
            "abc",
            "202604175",
            "2026/04/17",
            // H3: shape-only validation used to accept these. The
            // Gregorian check now rejects them on every public input.
            "20260230",
            "19990431",
            "00000000",
        ] {
            assert!(validate_expiration(bad, "expiration").is_err(), "{bad}");
        }
    }

    #[test]
    fn strike_accepts_wildcards_and_positive_decimals_and_rejects_garbage() {
        for good in ["*", "0", "", "550", "17.5", "0.5"] {
            assert!(validate_strike(good, "strike").is_ok(), "{good}");
        }
        for bad in ["abc", "-10", "1.5.3", "$500"] {
            assert!(validate_strike(bad, "strike").is_err(), "{bad}");
        }
    }
}
