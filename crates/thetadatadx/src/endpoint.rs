//! Shared endpoint invocation runtime for `thetadatadx`.
//!
//! This module owns the typed argument model, validation helpers, and
//! generated endpoint dispatch used by registry-driven projections such as the
//! CLI, REST server, and MCP server.

use std::collections::BTreeMap;

use tdbe::types::tick::{
    CalendarDay, EodTick, GreeksTick, InterestRateTick, IvTick, MarketValueTick, OhlcTick,
    OpenInterestTick, OptionContract, PriceTick, QuoteTick, TradeQuoteTick, TradeTick,
};

use crate::registry::ParamType;
use crate::Error;

/// Validated scalar argument value accepted by the shared endpoint runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum EndpointArgValue {
    /// UTF-8 string value.
    Str(String),
    /// Signed integer value.
    Int(i64),
    /// Floating-point value.
    Float(f64),
    /// Boolean flag.
    Bool(bool),
}

/// Typed argument bag consumed by generated endpoint dispatch.
///
/// Callers insert normalized values, then generated adapters use typed
/// accessors on this type to enforce endpoint parameter semantics. A
/// per-call deadline can be attached via [`Self::with_timeout_ms`]; the
/// generated dispatch applies it as `with_deadline` on the matching
/// builder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EndpointArgs {
    args: BTreeMap<String, EndpointArgValue>,
    timeout_ms: Option<u64>,
}

impl EndpointArgs {
    /// Create an empty endpoint argument bag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a per-call deadline expressed in milliseconds.
    ///
    /// The generated dispatch hands this to the endpoint builder via
    /// `with_deadline(Duration::from_millis(ms))`. On expiry the in-flight
    /// gRPC call is cancelled (the future is dropped, freeing its
    /// `request_semaphore` permit and the tonic stream) and the call
    /// returns [`crate::Error::Timeout`].
    #[must_use]
    pub fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// In-place equivalent of [`Self::with_timeout_ms`] for `&mut self` callers
    /// (FFI dispatch shims that already hold a `&mut EndpointArgs`).
    pub fn set_timeout_ms(&mut self, ms: u64) {
        self.timeout_ms = Some(ms);
    }

    /// Configured per-call deadline in milliseconds, if any.
    #[must_use]
    pub fn timeout_ms(&self) -> Option<u64> {
        self.timeout_ms
    }

    /// Drop the per-call deadline.
    pub fn clear_timeout(&mut self) {
        self.timeout_ms = None;
    }

    /// Insert or replace a normalized endpoint argument value.
    pub fn insert(&mut self, key: String, value: EndpointArgValue) -> Option<EndpointArgValue> {
        self.args.insert(key, value)
    }

    /// Parse a raw string according to registry metadata and insert it.
    pub fn insert_raw(
        &mut self,
        key: &str,
        param_type: ParamType,
        raw: &str,
    ) -> Result<(), EndpointError> {
        let value = parse_raw_arg_value(param_type, key, raw)?;
        self.insert(key.to_string(), value);
        Ok(())
    }

    fn required_value(&self, key: &str) -> Result<&EndpointArgValue, EndpointError> {
        self.args.get(key).ok_or_else(|| {
            EndpointError::InvalidParams(format!("missing required argument: {key}"))
        })
    }

    fn optional_value(&self, key: &str) -> Option<&EndpointArgValue> {
        self.args.get(key)
    }

    /// Read a required string argument.
    pub fn required_str(&self, key: &str) -> Result<&str, EndpointError> {
        match self.required_value(key)? {
            EndpointArgValue::Str(value) => Ok(value),
            _ => Err(EndpointError::InvalidParams(format!(
                "required string argument '{key}' must be a string"
            ))),
        }
    }

    /// Read an optional string argument.
    pub fn optional_str(&self, key: &str) -> Result<Option<&str>, EndpointError> {
        match self.optional_value(key) {
            None => Ok(None),
            Some(EndpointArgValue::Str(value)) => Ok(Some(value)),
            Some(_) => Err(EndpointError::InvalidParams(format!(
                "optional string argument '{key}' must be a string"
            ))),
        }
    }

    /// Read a required single symbol argument.
    pub fn required_symbol(&self, key: &str) -> Result<&str, EndpointError> {
        let value = self.required_str(key)?;
        validate_symbol(value, key)?;
        Ok(value)
    }

    /// Read a required comma-separated symbol list argument.
    pub fn required_symbols(&self, key: &str) -> Result<Vec<String>, EndpointError> {
        let value = self.required_str(key)?;
        let symbols = parse_symbols(value);
        if symbols.is_empty() {
            return Err(EndpointError::InvalidParams(format!(
                "'{key}' must contain at least one non-empty ticker symbol"
            )));
        }
        Ok(symbols)
    }

    /// Read a required `YYYYMMDD` date argument.
    pub fn required_date(&self, key: &str) -> Result<&str, EndpointError> {
        let value = self.required_str(key)?;
        validate_date(value, key)?;
        Ok(value)
    }

    /// Read an optional `YYYYMMDD` date argument.
    pub fn optional_date(&self, key: &str) -> Result<Option<&str>, EndpointError> {
        let Some(value) = self.optional_str(key)? else {
            return Ok(None);
        };
        validate_date(value, key)?;
        Ok(Some(value))
    }

    /// Read a required expiration argument.
    ///
    /// Accepts `*` / `0` (wildcard sentinels), `YYYYMMDD`, or `YYYY-MM-DD`.
    /// Wire-level canonicalization happens in `direct::normalize_expiration`.
    pub fn required_expiration(&self, key: &str) -> Result<&str, EndpointError> {
        let value = self.required_str(key)?;
        validate_expiration(value, key)?;
        Ok(value)
    }

    /// Read an optional expiration argument.
    ///
    /// Accepts `*` / `0` (wildcard sentinels), `YYYYMMDD`, or `YYYY-MM-DD`.
    pub fn optional_expiration(&self, key: &str) -> Result<Option<&str>, EndpointError> {
        let Some(value) = self.optional_str(key)? else {
            return Ok(None);
        };
        validate_expiration(value, key)?;
        Ok(Some(value))
    }

    /// Read a required strike argument.
    ///
    /// Accepts `*` / `0` / empty (wildcard sentinels) or a positive decimal
    /// (e.g. `"550"`, `"17.5"`). Wildcards become proto-unset on the wire
    /// via `direct::wire_strike_opt` so the server applies its default.
    pub fn required_strike(&self, key: &str) -> Result<&str, EndpointError> {
        let value = self.required_str(key)?;
        validate_strike(value, key)?;
        Ok(value)
    }

    /// Read an optional strike argument.
    pub fn optional_strike(&self, key: &str) -> Result<Option<&str>, EndpointError> {
        let Some(value) = self.optional_str(key)? else {
            return Ok(None);
        };
        validate_strike(value, key)?;
        Ok(Some(value))
    }

    /// Read a required interval argument.
    pub fn required_interval(&self, key: &str) -> Result<&str, EndpointError> {
        let value = self.required_str(key)?;
        validate_interval(value, key)?;
        Ok(value)
    }

    /// Read a required option right argument.
    pub fn required_right(&self, key: &str) -> Result<&str, EndpointError> {
        let value = self.required_str(key)?;
        validate_right(value, key)?;
        Ok(value)
    }

    /// Read a required `YYYY` year argument.
    pub fn required_year(&self, key: &str) -> Result<&str, EndpointError> {
        let value = self.required_str(key)?;
        validate_year(value, key)?;
        Ok(value)
    }

    /// Read a required integer argument and narrow it to `i32`.
    pub fn required_int32(&self, key: &str) -> Result<i32, EndpointError> {
        let raw = match self.required_value(key)? {
            EndpointArgValue::Int(value) => *value,
            _ => {
                return Err(EndpointError::InvalidParams(format!(
                    "required integer argument '{key}' must be an integer"
                )))
            }
        };
        i32::try_from(raw).map_err(|_| {
            EndpointError::InvalidParams(format!(
                "required integer argument '{key}' is out of range for i32: {raw}"
            ))
        })
    }

    /// Read an optional integer argument and narrow it to `i32`.
    pub fn optional_int32(&self, key: &str) -> Result<Option<i32>, EndpointError> {
        let Some(value) = self.optional_value(key) else {
            return Ok(None);
        };
        let raw = match value {
            EndpointArgValue::Int(value) => *value,
            _ => {
                return Err(EndpointError::InvalidParams(format!(
                    "optional integer argument '{key}' must be an integer"
                )))
            }
        };
        let narrowed = i32::try_from(raw).map_err(|_| {
            EndpointError::InvalidParams(format!(
                "optional integer argument '{key}' is out of range for i32: {raw}"
            ))
        })?;
        Ok(Some(narrowed))
    }

    /// Read a required floating-point argument.
    ///
    /// Accepts both `Float` and `Int` values. `Int` is widened to `f64`.
    pub fn required_float64(&self, key: &str) -> Result<f64, EndpointError> {
        match self.required_value(key)? {
            EndpointArgValue::Float(value) => Ok(*value),
            EndpointArgValue::Int(value) => Ok(*value as f64),
            _ => Err(EndpointError::InvalidParams(format!(
                "required number argument '{key}' must be a number"
            ))),
        }
    }

    /// Read an optional floating-point argument.
    ///
    /// Accepts both `Float` and `Int` values. `Int` is widened to `f64`.
    pub fn optional_float64(&self, key: &str) -> Result<Option<f64>, EndpointError> {
        match self.optional_value(key) {
            None => Ok(None),
            Some(EndpointArgValue::Float(value)) => Ok(Some(*value)),
            Some(EndpointArgValue::Int(value)) => Ok(Some(*value as f64)),
            Some(_) => Err(EndpointError::InvalidParams(format!(
                "optional number argument '{key}' must be a number"
            ))),
        }
    }

    /// Read a required boolean argument.
    pub fn required_bool(&self, key: &str) -> Result<bool, EndpointError> {
        match self.required_value(key)? {
            EndpointArgValue::Bool(value) => Ok(*value),
            _ => Err(EndpointError::InvalidParams(format!(
                "required boolean argument '{key}' must be a boolean"
            ))),
        }
    }

    /// Read an optional boolean argument.
    pub fn optional_bool(&self, key: &str) -> Result<Option<bool>, EndpointError> {
        match self.optional_value(key) {
            None => Ok(None),
            Some(EndpointArgValue::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(EndpointError::InvalidParams(format!(
                "optional boolean argument '{key}' must be a boolean"
            ))),
        }
    }
}

/// Error surface for the shared endpoint runtime.
#[derive(Debug)]
pub enum EndpointError {
    /// The caller supplied invalid or missing endpoint arguments.
    InvalidParams(String),
    /// The underlying SDK call failed.
    Server(Error),
    /// No generated endpoint adapter matches the requested endpoint name.
    UnknownEndpoint(String),
}

impl std::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParams(msg) => write!(f, "invalid params: {msg}"),
            Self::Server(err) => write!(f, "server error: {err}"),
            Self::UnknownEndpoint(name) => write!(f, "unknown endpoint: {name}"),
        }
    }
}

impl std::error::Error for EndpointError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Server(err) => Some(err),
            _ => None,
        }
    }
}

impl From<Error> for EndpointError {
    fn from(value: Error) -> Self {
        Self::Server(value)
    }
}

impl From<EndpointError> for Error {
    fn from(value: EndpointError) -> Self {
        match value {
            EndpointError::InvalidParams(message) | EndpointError::UnknownEndpoint(message) => {
                Error::Config(message)
            }
            EndpointError::Server(error) => error,
        }
    }
}

/// Typed result variants emitted by generated endpoint adapters.
#[derive(Debug)]
pub enum EndpointOutput {
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

/// Public entry point for transport-neutral endpoint dispatch.
///
/// Delegates to the generated dispatch function. This indirection exists
/// as a hook point for cross-cutting concerns (auth retry, metrics,
/// rate limiting) without modifying generated code.
///
/// When `args.timeout_ms()` is set, the entire dispatch future (and all
/// futures it spawns: builder, gRPC call, `collect_stream`) is wrapped in
/// [`tokio::time::timeout`]. On expiry the future is dropped — its locals
/// (`_permit`, `tonic::Streaming`) drop with it, releasing the request
/// semaphore and cancelling the in-flight gRPC stream — and the call
/// returns `EndpointError::Server(Error::Timeout { duration_ms })`.
/// Subsequent calls on the same `DirectClient` succeed.
pub async fn invoke_endpoint(
    client: &crate::direct::DirectClient,
    name: &str,
    args: &EndpointArgs,
) -> Result<EndpointOutput, EndpointError> {
    let dispatch = invoke_generated_endpoint(client, name, args);
    match args.timeout_ms() {
        None => dispatch.await,
        Some(ms) => {
            match tokio::time::timeout(std::time::Duration::from_millis(ms), dispatch).await {
                Ok(inner) => inner,
                Err(_) => Err(EndpointError::Server(Error::Timeout { duration_ms: ms })),
            }
        }
    }
}

/// Parse a raw string value according to registry metadata into a typed endpoint arg.
pub fn parse_raw_arg_value(
    param_type: ParamType,
    param_name: &str,
    raw: &str,
) -> Result<EndpointArgValue, EndpointError> {
    match param_type {
        ParamType::Float => raw
            .parse::<f64>()
            .map(EndpointArgValue::Float)
            .map_err(|error| {
                EndpointError::InvalidParams(format!(
                    "'{param_name}' must be a number, got '{raw}': {error}"
                ))
            }),
        ParamType::Int => raw
            .parse::<i64>()
            .map(EndpointArgValue::Int)
            .map_err(|error| {
                EndpointError::InvalidParams(format!(
                    "'{param_name}' must be an integer, got '{raw}': {error}"
                ))
            }),
        ParamType::Bool => parse_bool(raw)
            .map(EndpointArgValue::Bool)
            .map_err(|message| {
                EndpointError::InvalidParams(format!(
                    "'{param_name}' must be true/false or 1/0, got '{raw}': {message}"
                ))
            }),
        _ => Ok(EndpointArgValue::Str(raw.to_string())),
    }
}

// Canonical validation — delegates to the shared `validate` module.
use crate::validate::{
    parse_bool, parse_symbols, validate_date, validate_expiration, validate_interval,
    validate_right, validate_strike, validate_symbol, validate_year,
};

include!(concat!(env!("OUT_DIR"), "/endpoint_generated.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_args_default_has_no_timeout() {
        let args = EndpointArgs::new();
        assert_eq!(args.timeout_ms(), None);
    }

    #[test]
    fn with_timeout_ms_attaches_deadline() {
        let args = EndpointArgs::new().with_timeout_ms(60_000);
        assert_eq!(args.timeout_ms(), Some(60_000));
    }

    #[test]
    fn clear_timeout_drops_deadline() {
        let mut args = EndpointArgs::new().with_timeout_ms(60_000);
        args.clear_timeout();
        assert_eq!(args.timeout_ms(), None);
    }

    #[test]
    fn required_symbols_trim_and_reject_empty_entries() {
        let mut args = EndpointArgs::new();
        args.insert(
            "symbol".into(),
            EndpointArgValue::Str(" AAPL, MSFT ,, ".into()),
        );

        assert_eq!(
            args.required_symbols("symbol").unwrap(),
            vec!["AAPL".to_string(), "MSFT".to_string()]
        );
    }

    #[test]
    fn optional_i32_rejects_out_of_range_values() {
        let mut args = EndpointArgs::new();
        args.insert(
            "strike_range".into(),
            EndpointArgValue::Int(i64::from(i32::MAX) + 1),
        );

        let err = args.optional_int32("strike_range").unwrap_err();
        assert!(
            matches!(err, EndpointError::InvalidParams(message) if message.contains("out of range for i32"))
        );
    }

    #[test]
    fn required_date_enforces_yyyymmdd() {
        let mut args = EndpointArgs::new();
        args.insert("date".into(), EndpointArgValue::Str("2026-04-09".into()));

        let err = args.required_date("date").unwrap_err();
        assert!(
            matches!(err, EndpointError::InvalidParams(message) if message.contains("exactly 8 digits"))
        );
    }

    #[test]
    fn required_expiration_accepts_wildcard_zero() {
        let mut args = EndpointArgs::new();
        args.insert("expiration".into(), EndpointArgValue::Str("0".into()));
        assert_eq!(args.required_expiration("expiration").unwrap(), "0");
    }

    #[test]
    fn required_expiration_accepts_yyyymmdd() {
        let mut args = EndpointArgs::new();
        args.insert(
            "expiration".into(),
            EndpointArgValue::Str("20260410".into()),
        );
        assert_eq!(args.required_expiration("expiration").unwrap(), "20260410");
    }

    #[test]
    fn required_expiration_rejects_invalid_formats() {
        // ISO-dashed `YYYY-MM-DD` is now valid; assert on an actually-invalid form.
        let mut args = EndpointArgs::new();
        args.insert(
            "expiration".into(),
            EndpointArgValue::Str("2026/04/10".into()),
        );
        let err = args.required_expiration("expiration").unwrap_err();
        assert!(
            matches!(err, EndpointError::InvalidParams(message) if message.contains("expiration"))
        );
    }

    #[test]
    fn required_expiration_accepts_iso_dashed() {
        let mut args = EndpointArgs::new();
        args.insert(
            "expiration".into(),
            EndpointArgValue::Str("2026-04-17".into()),
        );
        assert_eq!(
            args.required_expiration("expiration").unwrap(),
            "2026-04-17"
        );
    }

    #[test]
    fn required_expiration_accepts_star_wildcard() {
        let mut args = EndpointArgs::new();
        args.insert("expiration".into(), EndpointArgValue::Str("*".into()));
        assert_eq!(args.required_expiration("expiration").unwrap(), "*");
    }

    #[test]
    fn optional_expiration_accepts_wildcard_zero() {
        let mut args = EndpointArgs::new();
        args.insert("expiration".into(), EndpointArgValue::Str("0".into()));
        assert_eq!(args.optional_expiration("expiration").unwrap(), Some("0"));
    }

    #[test]
    fn optional_expiration_returns_none_when_absent() {
        let args = EndpointArgs::new();
        assert_eq!(args.optional_expiration("expiration").unwrap(), None);
    }

    #[test]
    fn parse_raw_bool_accepts_terminal_style_values() {
        assert_eq!(
            parse_raw_arg_value(ParamType::Bool, "exclusive", "true").unwrap(),
            EndpointArgValue::Bool(true)
        );
        assert_eq!(
            parse_raw_arg_value(ParamType::Bool, "exclusive", "0").unwrap(),
            EndpointArgValue::Bool(false)
        );
    }

    /// Verify registry metadata integrity: 61 endpoints, no duplicates,
    /// all categories present, no empty descriptions or rest_paths.
    ///
    /// Note: dispatch-registry alignment (every registry name has a
    /// generated match arm) is guaranteed at build time — both are
    /// generated from the same `ParsedEndpoints` vec in
    /// `build_support/endpoints.rs`. A name mismatch is structurally
    /// impossible without a build failure. This test validates the
    /// registry's content, not dispatch coverage.
    #[test]
    fn registry_metadata_integrity() {
        use crate::registry::ENDPOINTS;

        let registry_names: std::collections::HashSet<&str> =
            ENDPOINTS.iter().map(|e| e.name).collect();
        assert_eq!(
            registry_names.len(),
            ENDPOINTS.len(),
            "duplicate names in ENDPOINTS"
        );
        assert_eq!(
            ENDPOINTS.len(),
            61,
            "expected 61 endpoints, got {}",
            ENDPOINTS.len()
        );
        let categories: std::collections::HashSet<&str> =
            ENDPOINTS.iter().map(|e| e.category).collect();
        for expected in ["stock", "option", "index", "calendar", "rate"] {
            assert!(
                categories.contains(expected),
                "missing category: {expected}"
            );
        }
        for ep in ENDPOINTS {
            assert!(
                !ep.description.is_empty(),
                "endpoint {} has empty description",
                ep.name
            );
            assert!(
                !ep.rest_path.is_empty(),
                "endpoint {} has empty rest_path",
                ep.name
            );
            // Every endpoint must have at least a name and return type
            assert!(!ep.name.is_empty(), "found endpoint with empty name");
        }
    }
}
