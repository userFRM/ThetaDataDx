//! Shared endpoint invocation bridge for `thetadatadx`.
//!
//! This module provides the typed argument model, validation helpers, and
//! generated endpoint dispatch first introduced for the standalone MCP server.
//! The same runtime is also reused by other endpoint projections such as the
//! CLI and REST server so they do not maintain their own handwritten endpoint
//! matches.

use std::collections::BTreeMap;

use tdbe::types::tick::{
    CalendarDay, EodTick, GreeksTick, InterestRateTick, IvTick, MarketValueTick, OhlcTick,
    OpenInterestTick, OptionContract, PriceTick, QuoteTick, TradeQuoteTick, TradeTick,
};

use crate::registry::ParamType;
use crate::{Error, ThetaDataDx};

/// Validated scalar argument value accepted by the MCP bridge.
///
/// The MCP transport normalizes JSON values into this small set so endpoint
/// validation and dispatch can run without depending on a particular JSON
/// library.
#[derive(Debug, Clone, PartialEq)]
pub enum McpArgValue {
    /// UTF-8 string value.
    Str(String),
    /// Signed integer value.
    Int(i64),
    /// Floating-point value.
    Float(f64),
    /// Boolean flag.
    Bool(bool),
}

/// Typed argument bag consumed by generated MCP endpoint dispatch.
///
/// Callers insert raw validated values, then endpoint adapters use the
/// typed accessors on this type to enforce parameter semantics such as
/// symbol format, date format, and integer range checks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct McpArgs(BTreeMap<String, McpArgValue>);

impl McpArgs {
    /// Create an empty MCP argument bag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a normalized MCP argument value.
    pub fn insert(&mut self, key: String, value: McpArgValue) -> Option<McpArgValue> {
        self.0.insert(key, value)
    }

    /// Parse a raw string according to registry metadata and insert it.
    pub fn insert_raw(
        &mut self,
        key: &str,
        param_type: ParamType,
        raw: &str,
    ) -> Result<(), McpError> {
        let value = parse_raw_arg_value(param_type, key, raw)?;
        self.insert(key.to_string(), value);
        Ok(())
    }

    fn required_value(&self, key: &str) -> Result<&McpArgValue, McpError> {
        self.0
            .get(key)
            .ok_or_else(|| McpError::InvalidParams(format!("missing required argument: {key}")))
    }

    fn optional_value(&self, key: &str) -> Option<&McpArgValue> {
        self.0.get(key)
    }

    /// Read a required string argument.
    pub fn required_str(&self, key: &str) -> Result<&str, McpError> {
        match self.required_value(key)? {
            McpArgValue::Str(value) => Ok(value),
            _ => Err(McpError::InvalidParams(format!(
                "required string argument '{key}' must be a string"
            ))),
        }
    }

    /// Read an optional string argument.
    pub fn optional_str(&self, key: &str) -> Result<Option<&str>, McpError> {
        match self.optional_value(key) {
            None => Ok(None),
            Some(McpArgValue::Str(value)) => Ok(Some(value)),
            Some(_) => Err(McpError::InvalidParams(format!(
                "optional string argument '{key}' must be a string"
            ))),
        }
    }

    /// Read a required single symbol argument.
    pub fn required_symbol(&self, key: &str) -> Result<&str, McpError> {
        let value = self.required_str(key)?;
        validate_symbol(value, key)?;
        Ok(value)
    }

    /// Read a required comma-separated symbol list argument.
    pub fn required_symbols(&self, key: &str) -> Result<Vec<String>, McpError> {
        let value = self.required_str(key)?;
        let symbols = parse_symbols(value);
        if symbols.is_empty() {
            return Err(McpError::InvalidParams(format!(
                "'{key}' must contain at least one non-empty ticker symbol"
            )));
        }
        Ok(symbols)
    }

    /// Read a required `YYYYMMDD` date argument.
    pub fn required_date(&self, key: &str) -> Result<&str, McpError> {
        let value = self.required_str(key)?;
        validate_date(value, key)?;
        Ok(value)
    }

    /// Read an optional `YYYYMMDD` date argument.
    pub fn optional_date(&self, key: &str) -> Result<Option<&str>, McpError> {
        let Some(value) = self.optional_str(key)? else {
            return Ok(None);
        };
        validate_date(value, key)?;
        Ok(Some(value))
    }

    /// Read a required interval argument.
    pub fn required_interval(&self, key: &str) -> Result<&str, McpError> {
        let value = self.required_str(key)?;
        validate_interval(value, key)?;
        Ok(value)
    }

    /// Read a required option right argument.
    pub fn required_right(&self, key: &str) -> Result<&str, McpError> {
        let value = self.required_str(key)?;
        validate_right(value, key)?;
        Ok(value)
    }

    /// Read a required `YYYY` year argument.
    pub fn required_year(&self, key: &str) -> Result<&str, McpError> {
        let value = self.required_str(key)?;
        validate_year(value, key)?;
        Ok(value)
    }

    /// Read a required integer argument and narrow it to `i32`.
    pub fn required_int32(&self, key: &str) -> Result<i32, McpError> {
        let raw = match self.required_value(key)? {
            McpArgValue::Int(value) => *value,
            _ => {
                return Err(McpError::InvalidParams(format!(
                    "required integer argument '{key}' must be an integer"
                )))
            }
        };
        i32::try_from(raw).map_err(|_| {
            McpError::InvalidParams(format!(
                "required integer argument '{key}' is out of range for i32: {raw}"
            ))
        })
    }

    /// Read an optional integer argument and narrow it to `i32`.
    pub fn optional_int32(&self, key: &str) -> Result<Option<i32>, McpError> {
        let Some(value) = self.optional_value(key) else {
            return Ok(None);
        };
        let raw = match value {
            McpArgValue::Int(value) => *value,
            _ => {
                return Err(McpError::InvalidParams(format!(
                    "optional integer argument '{key}' must be an integer"
                )))
            }
        };
        let narrowed = i32::try_from(raw).map_err(|_| {
            McpError::InvalidParams(format!(
                "optional integer argument '{key}' is out of range for i32: {raw}"
            ))
        })?;
        Ok(Some(narrowed))
    }

    /// Read a required floating-point argument.
    pub fn required_float64(&self, key: &str) -> Result<f64, McpError> {
        match self.required_value(key)? {
            McpArgValue::Float(value) => Ok(*value),
            McpArgValue::Int(value) => Ok(*value as f64),
            _ => Err(McpError::InvalidParams(format!(
                "required number argument '{key}' must be a number"
            ))),
        }
    }

    /// Read an optional floating-point argument.
    pub fn optional_float64(&self, key: &str) -> Result<Option<f64>, McpError> {
        match self.optional_value(key) {
            None => Ok(None),
            Some(McpArgValue::Float(value)) => Ok(Some(*value)),
            Some(McpArgValue::Int(value)) => Ok(Some(*value as f64)),
            Some(_) => Err(McpError::InvalidParams(format!(
                "optional number argument '{key}' must be a number"
            ))),
        }
    }

    /// Read a required boolean argument.
    pub fn required_bool(&self, key: &str) -> Result<bool, McpError> {
        match self.required_value(key)? {
            McpArgValue::Bool(value) => Ok(*value),
            _ => Err(McpError::InvalidParams(format!(
                "required boolean argument '{key}' must be a boolean"
            ))),
        }
    }

    /// Read an optional boolean argument.
    pub fn optional_bool(&self, key: &str) -> Result<Option<bool>, McpError> {
        match self.optional_value(key) {
            None => Ok(None),
            Some(McpArgValue::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(McpError::InvalidParams(format!(
                "optional boolean argument '{key}' must be a boolean"
            ))),
        }
    }
}

/// Error surface for the shared MCP bridge.
#[derive(Debug)]
pub enum McpError {
    /// The caller supplied invalid or missing endpoint arguments.
    InvalidParams(String),
    /// The underlying SDK call failed.
    Server(Error),
    /// No generated endpoint adapter matches the requested tool name.
    UnknownEndpoint(String),
}

impl From<Error> for McpError {
    fn from(value: Error) -> Self {
        Self::Server(value)
    }
}

impl From<McpError> for Error {
    fn from(value: McpError) -> Self {
        match value {
            McpError::InvalidParams(message) | McpError::UnknownEndpoint(message) => {
                Error::Config(message)
            }
            McpError::Server(error) => error,
        }
    }
}

/// Typed result variants emitted by generated MCP endpoint adapters.
#[derive(Debug)]
pub enum McpOutput {
    /// `Vec<String>` list result.
    StringList(Vec<String>),
    /// `Vec<EodTick>` result.
    EodTicks(Vec<EodTick>),
    /// `Vec<OhlcTick>` result.
    OhlcTicks(Vec<OhlcTick>),
    /// `Vec<TradeTick>` result.
    TradeTicks(Vec<TradeTick>),
    /// `Vec<QuoteTick>` result.
    QuoteTicks(Vec<QuoteTick>),
    /// `Vec<TradeQuoteTick>` result.
    TradeQuoteTicks(Vec<TradeQuoteTick>),
    /// `Vec<OpenInterestTick>` result.
    OpenInterestTicks(Vec<OpenInterestTick>),
    /// `Vec<MarketValueTick>` result.
    MarketValueTicks(Vec<MarketValueTick>),
    /// `Vec<GreeksTick>` result.
    GreeksTicks(Vec<GreeksTick>),
    /// `Vec<IvTick>` result.
    IvTicks(Vec<IvTick>),
    /// `Vec<PriceTick>` result.
    PriceTicks(Vec<PriceTick>),
    /// `Vec<CalendarDay>` result.
    CalendarDays(Vec<CalendarDay>),
    /// `Vec<InterestRateTick>` result.
    InterestRateTicks(Vec<InterestRateTick>),
    /// `Vec<OptionContract>` result.
    OptionContracts(Vec<OptionContract>),
}

/// Invoke a generated MCP adapter by endpoint name.
///
/// This is the shared execution entrypoint used by the standalone MCP server.
pub async fn invoke_endpoint(
    client: &ThetaDataDx,
    name: &str,
    args: &McpArgs,
) -> Result<McpOutput, McpError> {
    invoke_generated_endpoint(client, name, args).await
}

/// Parse a raw string value according to registry metadata into a typed endpoint arg.
pub fn parse_raw_arg_value(
    param_type: ParamType,
    param_name: &str,
    raw: &str,
) -> Result<McpArgValue, McpError> {
    match param_type {
        ParamType::Float => raw.parse::<f64>().map(McpArgValue::Float).map_err(|error| {
            McpError::InvalidParams(format!(
                "'{param_name}' must be a number, got '{raw}': {error}"
            ))
        }),
        ParamType::Int => raw.parse::<i64>().map(McpArgValue::Int).map_err(|error| {
            McpError::InvalidParams(format!(
                "'{param_name}' must be an integer, got '{raw}': {error}"
            ))
        }),
        ParamType::Bool => parse_bool(raw).map(McpArgValue::Bool).map_err(|message| {
            McpError::InvalidParams(format!(
                "'{param_name}' must be true/false or 1/0, got '{raw}': {message}"
            ))
        }),
        _ => Ok(McpArgValue::Str(raw.to_string())),
    }
}

fn validate_date(value: &str, param_name: &str) -> Result<(), McpError> {
    if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(McpError::InvalidParams(format!(
            "'{param_name}' must be exactly 8 digits (YYYYMMDD), got: '{value}'"
        )));
    }
    Ok(())
}

fn validate_symbol(value: &str, param_name: &str) -> Result<(), McpError> {
    if value.is_empty() {
        return Err(McpError::InvalidParams(format!(
            "'{param_name}' must be non-empty"
        )));
    }
    Ok(())
}

fn validate_interval(value: &str, param_name: &str) -> Result<(), McpError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(McpError::InvalidParams(format!(
            "'{param_name}' must be a non-empty alphanumeric string (e.g. '60000' or '1m'), got: '{value}'"
        )));
    }
    Ok(())
}

fn validate_right(value: &str, param_name: &str) -> Result<(), McpError> {
    match value.to_uppercase().as_str() {
        "C" | "P" | "CALL" | "PUT" => Ok(()),
        _ => Err(McpError::InvalidParams(format!(
            "'{param_name}' must be C, P, call, or put, got: '{value}'"
        ))),
    }
}

fn validate_year(value: &str, param_name: &str) -> Result<(), McpError> {
    if value.len() != 4 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(McpError::InvalidParams(format!(
            "'{param_name}' must be exactly 4 digits (YYYY), got: '{value}'"
        )));
    }
    Ok(())
}

fn parse_symbols(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_bool(value: &str) -> Result<bool, &'static str> {
    if value.eq_ignore_ascii_case("true") || value == "1" {
        Ok(true)
    } else if value.eq_ignore_ascii_case("false") || value == "0" {
        Ok(false)
    } else {
        Err("accepted values are true, false, 1, or 0")
    }
}

include!(concat!(env!("OUT_DIR"), "/mcp_generated.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_symbols_trim_and_reject_empty_entries() {
        let mut args = McpArgs::new();
        args.insert("symbol".into(), McpArgValue::Str(" AAPL, MSFT ,, ".into()));

        assert_eq!(
            args.required_symbols("symbol").unwrap(),
            vec!["AAPL".to_string(), "MSFT".to_string()]
        );
    }

    #[test]
    fn optional_i32_rejects_out_of_range_values() {
        let mut args = McpArgs::new();
        args.insert(
            "strike_range".into(),
            McpArgValue::Int(i64::from(i32::MAX) + 1),
        );

        let err = args.optional_int32("strike_range").unwrap_err();
        assert!(
            matches!(err, McpError::InvalidParams(message) if message.contains("out of range for i32"))
        );
    }

    #[test]
    fn required_date_enforces_yyyymmdd() {
        let mut args = McpArgs::new();
        args.insert("date".into(), McpArgValue::Str("2026-04-09".into()));

        let err = args.required_date("date").unwrap_err();
        assert!(
            matches!(err, McpError::InvalidParams(message) if message.contains("exactly 8 digits"))
        );
    }

    #[test]
    fn parse_raw_bool_accepts_terminal_style_values() {
        assert_eq!(
            parse_raw_arg_value(ParamType::Bool, "exclusive", "true").unwrap(),
            McpArgValue::Bool(true)
        );
        assert_eq!(
            parse_raw_arg_value(ParamType::Bool, "exclusive", "0").unwrap(),
            McpArgValue::Bool(false)
        );
    }
}
