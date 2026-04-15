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

/// Match `YYYY-MM-DD` (ISO-dashed date). Shared by the validator and the
/// wire-level canonicalizer in `direct::normalize_expiration`.
pub(crate) fn is_iso_date(value: &str) -> bool {
    let mut parts = value.splitn(3, '-');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(y), Some(m), Some(d), None)
            if y.len() == 4
                && m.len() == 2
                && d.len() == 2
                && y.bytes().all(|b| b.is_ascii_digit())
                && m.bytes().all(|b| b.is_ascii_digit())
                && d.bytes().all(|b| b.is_ascii_digit())
    )
}

/// Validate the `expiration` parameter.
///
/// Upstream's `openapiv3.yaml` documents the accepted vocabulary for
/// `expiration` on option endpoints as:
///
/// - `YYYY-MM-DD` — ISO-dashed date
/// - `YYYYMMDD`   — compact date
/// - `*`          — all expirations (wildcard)
///
/// We additionally accept `"0"` as a legacy wildcard sentinel and translate
/// it to `*` on the wire in [`crate::direct::normalize_expiration`]. The
/// server itself rejects a literal `"0"` with `InvalidArgument -- Error
/// parsing expiration Cannot parse date string: 0`, so the client-side
/// translation is what makes that form functional.
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
/// forms. Wildcards become proto-unset in [`crate::direct::wire_strike_opt`]
/// so the server applies its documented default.
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
    fn expiration_accepts_compact_date() {
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
