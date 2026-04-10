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
    if value == "0" {
        return Ok(());
    }
    validate_date(value, param_name)
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
    match value.to_uppercase().as_str() {
        "C" | "P" | "CALL" | "PUT" => Ok(()),
        _ => Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be C, P, call, or put, got: '{value}'"
        ))),
    }
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
