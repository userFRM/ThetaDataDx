//! Canonical parameter validation functions for `thetadatadx`.
//!
//! Every runtime validation check (date format, symbol format, interval
//! legality, option right, year) lives here as the single source of truth.
//! Both the shared endpoint runtime ([`crate::endpoint`]) and the MDDS
//! client macros ([`crate::mdds`]) delegate to these functions.
//!
//! Build-time validators in `build_support/endpoints/` operate on the TOML
//! surface spec and proto schema — a fundamentally different domain — so they
//! remain separate.

use crate::endpoint::EndpointError;
use crate::wire_semantics::is_iso_date;

pub(crate) fn validate_date(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be exactly 8 digits (YYYYMMDD), got: '{value}'"
        )));
    }
    Ok(())
}

/// Validate `expiration`: accepts `YYYY-MM-DD`, `YYYYMMDD`, `*`, or the
/// legacy `"0"` wildcard (translated to `*` in
/// [`crate::wire_semantics::normalize_expiration`]).
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
/// We additionally accept `"0"` and the empty string as ergonomic wildcard
/// forms. Wildcards become proto-unset in
/// [`crate::wire_semantics::wire_strike_opt`] so the server applies its
/// documented default.
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
    // Delegate to the canonical parser so the accepted vocabulary stays in
    // one place. The endpoint layer does not distinguish Call/Put/Both here
    // -- per-endpoint logic in the MDDS client decides whether `both` /
    // `*` is meaningful -- so we only care about "is this parseable at all".
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiration_accepts_documented_vocab_and_rejects_garbage() {
        for good in ["*", "0", "20260417", "2026-04-17"] {
            assert!(validate_expiration(good, "expiration").is_ok(), "{good}");
        }
        for bad in ["", "abc", "202604175", "2026/04/17"] {
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
