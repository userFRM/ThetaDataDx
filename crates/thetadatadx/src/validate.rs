//! Canonical parameter validation functions for `thetadatadx`.
//!
//! Every runtime validation check (date format, symbol format, interval
//! legality, option right, year) lives here as the single source of truth.
//! Both the shared endpoint runtime ([`crate::endpoint`]) and the direct
//! client macros ([`crate::direct`]) delegate to these functions.
//!
//! Build-time validators in `build_support/endpoints.rs` operate on the TOML
//! surface spec and proto schema — a fundamentally different domain — so they
//! remain separate.

use crate::endpoint::EndpointError;

pub(crate) fn validate_date(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be exactly 8 digits (YYYYMMDD), got: '{value}'"
        )));
    }
    Ok(())
}

pub(crate) fn validate_expiration(value: &str, param_name: &str) -> Result<(), EndpointError> {
    // Upstream's v3 API (see the decompiled Java terminal + openapiv3.yaml) accepts:
    //   - `*`                canonical wildcard ("all expirations")
    //   - `YYYYMMDD`         explicit date, compact
    //   - `YYYY-MM-DD`       explicit date, ISO-dashed
    //
    // We additionally accept the legacy v3-terminal sentinel `0` and translate
    // it to `*` on the wire in `direct::normalize_expiration` (the server
    // itself rejects `0` for expiration with an `InvalidArgument` parse error).
    if value == "*" || value == "0" {
        return Ok(());
    }
    // ISO form: `YYYY-MM-DD`
    if value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7) || b.is_ascii_digit())
    {
        return Ok(());
    }
    // Compact form: `YYYYMMDD`
    validate_date(value, param_name).map_err(|_| {
        EndpointError::InvalidParams(format!(
            "'{param_name}' must be '*' (wildcard), '0' (legacy wildcard), 'YYYYMMDD', or 'YYYY-MM-DD', got: '{value}'"
        ))
    })
}

/// Validate the `strike` parameter.
///
/// Accepts:
/// - `*`           canonical wildcard ("all strikes", server default)
/// - `0`           legacy v3-terminal sentinel we translate to proto-unset
/// - `""`          empty string, treated as wildcard / proto-unset
/// - any value parseable as a positive decimal (e.g. `"550"`, `"17.5"`)
///
/// Wire-level translation happens in `direct::wire_strike_opt`: sentinels
/// become `None` on the `ContractSpec.strike` field, matching the Java
/// terminal's wire semantics (field unset -> server applies default).
pub(crate) fn validate_strike(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value == "*" || value == "0" || value.is_empty() {
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
    // -- per-endpoint logic in the direct client decides whether `both` /
    // `*` is meaningful -- so we only care about "is this parseable at all".
    crate::right::parse_right(value).map(|_| ()).map_err(|_| {
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
    fn expiration_accepts_canonical_wildcard() {
        assert!(validate_expiration("*", "expiration").is_ok());
    }

    #[test]
    fn expiration_accepts_legacy_zero_wildcard() {
        assert!(validate_expiration("0", "expiration").is_ok());
    }

    #[test]
    fn expiration_accepts_explicit_date() {
        assert!(validate_expiration("20260417", "expiration").is_ok());
    }

    #[test]
    fn expiration_accepts_iso_dashed() {
        assert!(validate_expiration("2026-04-17", "expiration").is_ok());
    }

    #[test]
    fn expiration_rejects_garbage() {
        for bad in ["", "abc", "1", "99", "202604175", "**", "2026/04/17"] {
            let err = validate_expiration(bad, "expiration").unwrap_err();
            let msg = format!("{err:?}");
            assert!(
                msg.contains("expiration"),
                "expected descriptive error for '{bad}', got: {msg}"
            );
        }
    }

    #[test]
    fn strike_accepts_canonical_wildcard() {
        assert!(validate_strike("*", "strike").is_ok());
    }

    #[test]
    fn strike_accepts_legacy_zero_wildcard() {
        assert!(validate_strike("0", "strike").is_ok());
    }

    #[test]
    fn strike_accepts_empty_as_wildcard() {
        assert!(validate_strike("", "strike").is_ok());
    }

    #[test]
    fn strike_accepts_decimal_values() {
        for good in ["550", "17.5", "0.5", "1000", "2.125"] {
            assert!(
                validate_strike(good, "strike").is_ok(),
                "unexpected rejection of '{good}'"
            );
        }
    }

    #[test]
    fn strike_rejects_garbage() {
        for bad in ["abc", "-10", "1.5.3", "$500", "500$"] {
            let err = validate_strike(bad, "strike").unwrap_err();
            let msg = format!("{err:?}");
            assert!(
                msg.contains("strike"),
                "expected descriptive error for '{bad}', got: {msg}"
            );
        }
    }
}
