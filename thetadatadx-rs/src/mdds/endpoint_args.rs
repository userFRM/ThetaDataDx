//! Shared endpoint invocation runtime for `thetadatadx`.
//!
//! Owns the typed argument model, validation helpers, and generated endpoint
//! dispatch used by registry-driven projections (CLI, MCP server, optional
//! local HTTP front-end).

// Items in this module are split into two groups:
//
// 1. Always-compiled (no feature gate): `EndpointError` and its trait impls.
//    Used internally by `mdds::validate` and the `From` conversions in
//    `error.rs` — needed in every build.
//
// 2. `#[cfg(feature = "__internal")]`: everything else. The argument bag
//    (`EndpointArgs`, `EndpointArgValue`), the output enum (`EndpointOutput`),
//    and the generated dispatch (`invoke_generated_endpoint`,
//    `invoke_endpoint`, `parse_raw_arg_value`) are only reachable from
//    workspace tools (`tools/server`, `tools/mcp`) and
//    bindings — never from crate-internal code.

use crate::Error;

#[cfg(feature = "__internal")]
use crate::columns::Ticks;
#[cfg(feature = "__internal")]
use crate::mdds::registry::ParamType;
#[cfg(feature = "__internal")]
use crate::tdbe::types::tick::{
    CalendarDay, EodTick, GreeksAllTick, GreeksEodTick, GreeksFirstOrderTick,
    GreeksSecondOrderTick, GreeksThirdOrderTick, IndexPriceAtTimeTick, InterestRateTick,
    IntervalQuoteTick, IvTick, MarketValueTick, OhlcTick, OpenInterestTick, OptionContract,
    PriceTick, QuoteTick, TradeGreeksAllTick, TradeGreeksFirstOrderTick,
    TradeGreeksImpliedVolatilityTick, TradeGreeksSecondOrderTick, TradeGreeksThirdOrderTick,
    TradeQuoteTick, TradeTick,
};
#[cfg(feature = "__internal")]
use std::collections::BTreeMap;

/// Validated scalar argument value accepted by the shared endpoint runtime.
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
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

/// Per-call deadline state carried by [`EndpointArgs`].
///
/// A two-state `Option<u64>` cannot distinguish "the caller never set a
/// deadline" from "the caller explicitly disabled the deadline" — both
/// collapse to `None`. That ambiguity makes `with_timeout_ms(0)` silently
/// fall back to the configured
/// [`crate::config::MarketDataConfig::request_timeout_secs`] default instead
/// of opting the call out of any deadline. This tri-state keeps the two
/// intents distinct so the generated dispatch can honour each:
///
/// | Variant      | Source                  | Effect on the dispatched call                                  |
/// |--------------|-------------------------|----------------------------------------------------------------|
/// | `Unset`      | no `with_timeout_ms`    | fall back to the configured `request_timeout_secs` default     |
/// | `Disabled`   | `with_timeout_ms(0)`    | no deadline — the call runs unbounded (the documented opt-out) |
/// | `Millis(n)`  | `with_timeout_ms(n>0)`  | a per-call deadline of `n` milliseconds                        |
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeadlineSetting {
    /// No per-call deadline was set; fall back to the configured default.
    #[default]
    Unset,
    /// The deadline was explicitly disabled (`with_timeout_ms(0)`); the
    /// call runs unbounded.
    Disabled,
    /// An explicit per-call deadline in milliseconds (`> 0`).
    Millis(u64),
}

#[cfg(feature = "__internal")]
impl DeadlineSetting {
    /// Translate the tri-state into the optional `Duration` the generated
    /// dispatch applies to an endpoint builder via `with_deadline` (or a
    /// list `_with_deadline` overload).
    ///
    /// - [`Self::Unset`] → `None`: the dispatch must NOT call `with_deadline`,
    ///   so the builder falls back to the configured `request_timeout_secs`
    ///   default.
    /// - [`Self::Disabled`] → `Some(Duration::ZERO)`: an explicit zero, which
    ///   `with_deadline` carries through to
    ///   [`crate::mdds::macros::effective_deadline`] where it disables the
    ///   deadline entirely (the call runs unbounded).
    /// - [`Self::Millis`] → `Some(Duration::from_millis(n))`: the explicit
    ///   per-call bound.
    #[must_use]
    pub fn builder_deadline(self) -> Option<std::time::Duration> {
        match self {
            DeadlineSetting::Unset => None,
            DeadlineSetting::Disabled => Some(std::time::Duration::ZERO),
            DeadlineSetting::Millis(ms) => Some(std::time::Duration::from_millis(ms)),
        }
    }
}

/// Typed argument bag consumed by generated endpoint dispatch.
///
/// Callers insert normalized values, then generated adapters use typed
/// accessors on this type to enforce endpoint parameter semantics. A
/// per-call deadline can be attached via [`Self::with_timeout_ms`]; the
/// generated dispatch applies it as `with_deadline` on the matching
/// builder.
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EndpointArgs {
    args: BTreeMap<String, EndpointArgValue>,
    deadline: DeadlineSetting,
}

#[cfg(feature = "__internal")]
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
    ///
    /// `ms == 0` is the deadline opt-out: it records
    /// [`DeadlineSetting::Disabled`] so the dispatched call runs unbounded.
    /// This is distinct from never calling `with_timeout_ms` at all
    /// ([`DeadlineSetting::Unset`]), which falls back to the configured
    /// [`crate::config::MarketDataConfig::request_timeout_secs`] default. A
    /// positive `ms` records [`DeadlineSetting::Millis`].
    #[must_use]
    pub fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.set_timeout_ms(ms);
        self
    }

    /// In-place equivalent of [`Self::with_timeout_ms`] for `&mut self` callers
    /// (FFI dispatch shims that already hold a `&mut EndpointArgs`).
    ///
    /// `ms == 0` records [`DeadlineSetting::Disabled`] (the deadline opt-out)
    /// — see [`Self::with_timeout_ms`].
    pub fn set_timeout_ms(&mut self, ms: u64) {
        self.deadline = if ms == 0 {
            DeadlineSetting::Disabled
        } else {
            DeadlineSetting::Millis(ms)
        };
    }

    /// Per-call deadline state, distinguishing "unset" (fall back to the
    /// configured default) from "explicitly disabled" (run unbounded). The
    /// generated dispatch reads this to honour `with_timeout_ms(0)` as the
    /// documented opt-out rather than collapsing it into the default.
    #[must_use]
    pub fn deadline_setting(&self) -> DeadlineSetting {
        self.deadline
    }

    /// Drop the per-call deadline, returning to [`DeadlineSetting::Unset`]
    /// (the configured default applies on the next dispatch).
    pub fn clear_timeout(&mut self) {
        self.deadline = DeadlineSetting::Unset;
    }

    /// Insert or replace a normalized endpoint argument value.
    pub fn insert(&mut self, key: String, value: EndpointArgValue) -> Option<EndpointArgValue> {
        self.args.insert(key, value)
    }

    /// Parse a raw string according to registry metadata and insert it.
    ///
    /// # Errors
    ///
    /// Returns [`EndpointError::InvalidParams`] when `raw` does not parse as
    /// the type `param_type` requires.
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
    /// Accepts `*` (wildcard), `YYYYMMDD`, or `YYYY-MM-DD`.
    /// Wire-level canonicalization happens in `crate::mdds::wire_semantics::normalize_expiration`.
    pub fn required_expiration(&self, key: &str) -> Result<&str, EndpointError> {
        let value = self.required_str(key)?;
        validate_expiration(value, key)?;
        Ok(value)
    }

    /// Read an optional expiration argument.
    ///
    /// Accepts `*` (wildcard), `YYYYMMDD`, or `YYYY-MM-DD`.
    pub fn optional_expiration(&self, key: &str) -> Result<Option<&str>, EndpointError> {
        let Some(value) = self.optional_str(key)? else {
            return Ok(None);
        };
        validate_expiration(value, key)?;
        Ok(Some(value))
    }

    /// Read a required strike argument.
    ///
    /// Accepts `*` (wildcard), empty (unset), or a positive decimal
    /// (e.g. `"550"`, `"17.5"`). Both wildcard forms are sent as a literal `*`
    /// via `crate::mdds::wire_semantics::wire_strike_opt`.
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

    /// Read an optional interval argument.
    pub fn optional_interval(&self, key: &str) -> Result<Option<&str>, EndpointError> {
        let Some(value) = self.optional_str(key)? else {
            return Ok(None);
        };
        validate_interval(value, key)?;
        Ok(Some(value))
    }

    /// Read a required option right argument.
    pub fn required_right(&self, key: &str) -> Result<&str, EndpointError> {
        let value = self.required_str(key)?;
        validate_right(value, key)?;
        Ok(value)
    }

    /// Read an optional option right argument.
    pub fn optional_right(&self, key: &str) -> Result<Option<&str>, EndpointError> {
        let Some(value) = self.optional_str(key)? else {
            return Ok(None);
        };
        validate_right(value, key)?;
        Ok(Some(value))
    }

    /// Read a required `YYYY` year argument.
    pub fn required_year(&self, key: &str) -> Result<&str, EndpointError> {
        let value = self.required_str(key)?;
        validate_year(value, key)?;
        Ok(value)
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
                )));
            }
        };
        let narrowed = i32::try_from(raw).map_err(|_| {
            EndpointError::InvalidParams(format!(
                "optional integer argument '{key}' is out of range for i32: {raw}"
            ))
        })?;
        Ok(Some(narrowed))
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

#[doc(hidden)]
impl From<EndpointError> for Error {
    fn from(value: EndpointError) -> Self {
        match value {
            EndpointError::InvalidParams(message) => {
                Error::config_invalid("endpoint.params", message)
            }
            EndpointError::UnknownEndpoint(message) => {
                Error::config_invalid("endpoint.name", message)
            }
            EndpointError::Server(error) => error,
        }
    }
}

/// Typed result variants emitted by generated endpoint adapters.
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
#[derive(Debug)]
pub enum EndpointOutput {
    /// `Vec<String>` list result.
    StringList(Vec<String>),
    /// `Ticks<EodTick>` result.
    EodTicks(Ticks<EodTick>),
    /// `Ticks<OhlcTick>` result.
    OhlcTicks(Ticks<OhlcTick>),
    /// `Ticks<TradeTick>` result.
    TradeTicks(Ticks<TradeTick>),
    /// `Ticks<QuoteTick>` result.
    QuoteTicks(Ticks<QuoteTick>),
    /// `Ticks<TradeQuoteTick>` result.
    TradeQuoteTicks(Ticks<TradeQuoteTick>),
    /// `Ticks<OpenInterestTick>` result.
    OpenInterestTicks(Ticks<OpenInterestTick>),
    /// `Ticks<IntervalQuoteTick>` result. Returned by the quote flat files
    /// (`*_flat_file_quote`): one NBBO quote per interval, carrying both the
    /// interval time and the time the quote was reported.
    IntervalQuoteTicks(Ticks<IntervalQuoteTick>),
    /// `Ticks<MarketValueTick>` result.
    MarketValueTicks(Ticks<MarketValueTick>),
    /// `Ticks<GreeksAllTick>` result. Returned by `option_*_greeks_all`
    /// endpoints (interval-sampled full-union Greeks paired with the
    /// bid/ask quote pair).
    GreeksAllTicks(Ticks<GreeksAllTick>),
    /// `Ticks<GreeksEodTick>` result. Returned by
    /// `option_history_greeks_eod` -- end-of-day full-union Greeks
    /// calculation fused with the twelve EOD trade/quote context
    /// columns (`open`, `high`, `low`, `close`, `volume`, `count`,
    /// `bid_size`, `bid_exchange`, `bid_condition`, `ask_size`,
    /// `ask_exchange`, `ask_condition`) the bare `GreeksAllTick`
    /// silently dropped.
    GreeksEodTicks(Ticks<GreeksEodTick>),
    /// `Ticks<GreeksFirstOrderTick>` result. Returned by
    /// `option_*_greeks_first_order` endpoints.
    GreeksFirstOrderTicks(Ticks<GreeksFirstOrderTick>),
    /// `Ticks<GreeksSecondOrderTick>` result. Returned by
    /// `option_*_greeks_second_order` endpoints.
    GreeksSecondOrderTicks(Ticks<GreeksSecondOrderTick>),
    /// `Ticks<GreeksThirdOrderTick>` result. Returned by
    /// `option_*_greeks_third_order` endpoints.
    GreeksThirdOrderTicks(Ticks<GreeksThirdOrderTick>),
    /// `Ticks<TradeGreeksAllTick>` result. Returned by
    /// `option_history_trade_greeks_all` -- per-OPRA-trade Greeks calculation
    /// carrying both the trade-side execution columns and every Greek the
    /// server publishes for the all-union endpoint.
    TradeGreeksAllTicks(Ticks<TradeGreeksAllTick>),
    /// `Ticks<TradeGreeksFirstOrderTick>` result. Returned by
    /// `option_history_trade_greeks_first_order` -- per-OPRA-trade first-order
    /// Greeks calculation with trade execution columns.
    TradeGreeksFirstOrderTicks(Ticks<TradeGreeksFirstOrderTick>),
    /// `Ticks<TradeGreeksSecondOrderTick>` result. Returned by
    /// `option_history_trade_greeks_second_order` -- per-OPRA-trade
    /// second-order Greeks calculation with trade execution columns.
    TradeGreeksSecondOrderTicks(Ticks<TradeGreeksSecondOrderTick>),
    /// `Ticks<TradeGreeksThirdOrderTick>` result. Returned by
    /// `option_history_trade_greeks_third_order` -- per-OPRA-trade third-order
    /// Greeks calculation with trade execution columns.
    TradeGreeksThirdOrderTicks(Ticks<TradeGreeksThirdOrderTick>),
    /// `Ticks<TradeGreeksImpliedVolatilityTick>` result. Returned by
    /// `option_history_trade_greeks_implied_volatility` -- per-OPRA-trade IV
    /// calculation with trade execution columns. Carries only the single
    /// `implied_volatility` + `iv_error` pair (not the bid/mid/ask IV triple
    /// of the interval-sampled `IvTick`).
    TradeGreeksImpliedVolatilityTicks(Ticks<TradeGreeksImpliedVolatilityTick>),
    /// `Ticks<IvTick>` result.
    IvTicks(Ticks<IvTick>),
    /// `Ticks<PriceTick>` result.
    PriceTicks(Ticks<PriceTick>),
    /// `Ticks<IndexPriceAtTimeTick>` result. Returned by
    /// `index_at_time_price` -- trade-shaped row (10 columns:
    /// `timestamp`, `sequence`, `ext_condition1..4`, `condition`,
    /// `size`, `exchange`, `price`) carrying the seven trade-side
    /// execution columns the bare `PriceTick` (3 columns) silently
    /// dropped, including the SIP-exchange attribution field.
    IndexPriceAtTimeTicks(Ticks<IndexPriceAtTimeTick>),
    /// `Vec<CalendarDay>` result.
    CalendarDays(Ticks<CalendarDay>),
    /// `Ticks<InterestRateTick>` result.
    InterestRateTicks(Ticks<InterestRateTick>),
    /// `Vec<OptionContract>` result.
    OptionContracts(Ticks<OptionContract>),
}

/// Public entry point for transport-neutral endpoint dispatch.
///
/// Delegates to the generated dispatch function. This indirection exists
/// as a hook point for cross-cutting concerns (auth retry, metrics,
/// rate limiting) without modifying generated code.
///
/// The per-call deadline is resolved inside the generated dispatch, which
/// threads [`EndpointArgs::deadline_setting`] onto the endpoint builder via
/// `with_deadline` (or the list `_with_deadline` overload). The builder is
/// the single deadline authority, so the three deadline intents are honoured
/// exactly: [`DeadlineSetting::Unset`] falls back to the configured
/// [`crate::config::MarketDataConfig::request_timeout_secs`] default,
/// [`DeadlineSetting::Disabled`] (`with_timeout_ms(0)`) runs unbounded, and
/// [`DeadlineSetting::Millis`] applies the explicit per-call bound. On expiry
/// the in-flight future is dropped — its locals (`_permit`,
/// `tonic::Streaming`) drop with it, releasing the request semaphore and
/// cancelling the in-flight gRPC stream — and the call returns
/// `EndpointError::Server(Error::Timeout { duration_ms })`. Subsequent calls
/// on the same `MarketDataClient` succeed.
///
/// `price_types`, when given, receives the price type of every cell of a
/// buffered response's rows, row for row, which is the precision the vendor
/// wrote each price at. A list response leaves it untouched.
///
/// # Errors
///
/// Returns [`EndpointError::UnknownEndpoint`] for an unrecognized `name`,
/// [`EndpointError::InvalidParams`] for malformed `args`, and
/// [`EndpointError::Server`] for transport, auth, or deadline failures
/// (including `Error::Timeout` when an applied per-call deadline expires).
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
pub async fn invoke_endpoint(
    client: &crate::mdds::MarketDataClient,
    name: &str,
    args: &EndpointArgs,
    price_types: Option<&mut crate::mdds::decode::extract::PriceTypes>,
) -> Result<EndpointOutput, EndpointError> {
    invoke_generated_endpoint(client, name, args, price_types).await
}

/// Transport-neutral endpoint dispatch that drains the response
/// chunk-by-chunk instead of buffering the full `EndpointOutput`.
///
/// The streaming sibling of [`invoke_endpoint`]: same registry-driven
/// dispatch, same typed builder construction and `EndpointArgs`-sourced
/// optional setters, but the response is delivered through `handler` one
/// decoded gRPC chunk at a time so peak memory tracks a single chunk rather
/// than the whole result. `handler` receives each chunk's rows as a
/// type-erased `(*const c_void, usize)` pointing at a contiguous run of the
/// endpoint's `#[repr(C)]` tick, the boundary the FFI layer rebuilds the
/// typed pointer from without re-marshaling, and the chunk's
/// [`crate::columns::ColumnPresence`], which carries each row's root. A
/// `Break` from `handler` ends the call, every band of a fan-out included,
/// and it returns `Ok(())` without calling `handler` again.
///
/// The per-call deadline is resolved inside the generated dispatch — the
/// `.stream()` builder threads [`EndpointArgs::deadline_setting`] through
/// `with_deadline` exactly as the buffered path does, so the same three
/// intents ([`DeadlineSetting::Unset`] → configured default,
/// [`DeadlineSetting::Disabled`] → unbounded, [`DeadlineSetting::Millis`] →
/// explicit bound) are honoured. On expiry the future is dropped, its locals
/// (`_permit`, `tonic::Streaming`) drop with it, the in-flight gRPC stream is
/// cancelled, and the call returns
/// `EndpointError::Server(Error::Timeout { duration_ms })`. The handler is
/// guaranteed not to be invoked again once the deadline fires.
///
/// # Errors
///
/// Same error surface as [`invoke_endpoint`]:
/// [`EndpointError::UnknownEndpoint`] for an unrecognized `name`,
/// [`EndpointError::InvalidParams`] for malformed `args`, and
/// [`EndpointError::Server`] for transport, auth, or deadline failures.
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
pub async fn invoke_endpoint_stream(
    client: &crate::mdds::MarketDataClient,
    name: &str,
    args: &EndpointArgs,
    handler: impl FnMut(
        *const core::ffi::c_void,
        usize,
        &crate::columns::ColumnPresence,
    ) -> core::ops::ControlFlow<()>
    + Send,
) -> Result<(), EndpointError> {
    invoke_generated_endpoint_stream(client, name, args, handler).await
}

/// Parse a raw string value according to registry metadata into a typed endpoint arg.
///
/// # Errors
///
/// Returns [`EndpointError::InvalidParams`] when `raw` cannot be parsed as the
/// numeric or boolean shape `param_type` demands; string-typed params never error.
///
/// Only present when the `__internal` feature is enabled.
#[cfg(feature = "__internal")]
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

// Canonical validation — delegates to the shared `mdds::validate` module.
// Gated on `__internal` because these are only called from within the gated
// `EndpointArgs` impl block and `parse_raw_arg_value` above.
#[cfg(feature = "__internal")]
use crate::mdds::validate::{
    parse_bool, parse_symbols, validate_date, validate_expiration, validate_interval,
    validate_right, validate_strike, validate_symbol, validate_year,
};

/// Full-schema presence for a header-less collected result. The streaming
/// buffered dispatch drains per-chunk slices without a response header list,
/// so it reports every schema column present (the [`crate::columns::WireColumns::all_columns`]
/// default) rather than a wire-projected set. Called by the generated dispatch.
#[cfg(feature = "__internal")]
fn collected_columns<T: crate::columns::WireColumns>(
    _rows: &[T],
) -> crate::columns::ColumnPresence {
    <T as crate::columns::WireColumns>::all_columns()
}

/// Expands both endpoint tables into the two dispatch functions. A match
/// cannot take its arms from separate macro calls, so the entries are first
/// sorted into buffered and chunk-streaming arms, then emitted together. The
/// first arm names the functions' parameters once and carries those tokens
/// down, so every arm refers to the same bindings.
#[cfg(feature = "__internal")]
macro_rules! endpoint_dispatch {
    (@collect $($entries:tt)*) => {
        crate::__stream_endpoints! { endpoint_dispatch { @sort { client args price_types handler } [] [] $($entries)* } }
    };
    // Option contract rows own a string, so they cannot cross the
    // type-erased chunk callback: buffered dispatch only.
    (@sort $c:tt [$($b:tt)*] $s:tt $(#[$attr:ident])* chunked $name:ident {
        group: $g:ident / $sg:ident, rest: $r:tt, returns: OptionContract, params: $keys:tt, doc: $d:tt,
    } $($rest:tt)*) => {
        endpoint_dispatch! { @sort $c [$($b)* { chunked $name OptionContract $keys }] $s $($rest)* }
    };
    (@sort $c:tt [$($b:tt)*] [$($s:tt)*] $(#[$attr:ident])* chunked $name:ident {
        group: $g:ident / $sg:ident, rest: $r:tt, returns: $ret:ident, params: $keys:tt, doc: $d:tt,
    } $($rest:tt)*) => {
        endpoint_dispatch! { @sort $c [$($b)* { chunked $name $ret $keys }] [$($s)* { $name $ret $keys }] $($rest)* }
    };
    (@sort $c:tt [$($b:tt)*] $s:tt $(#[$attr:ident $(($w:ident))?])* $shape:ident $name:ident {
        group: $g:ident / $sg:ident, rest: $r:tt, returns: $ret:ident $(($col:tt))?, params: $keys:tt, doc: $d:tt,
    } $($rest:tt)*) => {
        endpoint_dispatch! { @sort $c [$($b)* { $shape $name $ret $keys }] $s $($rest)* }
    };
    (@sort { $client:ident $args:ident $pt:ident $handler:ident }
        [$({ $shape:ident $name:ident $ret:ident [$($key:tt)*] })*]
        [$({ $sname:ident $sret:ident [$($skey:tt)*] })*]) => {
        /// Dispatch a validated endpoint call into the typed builder it names.
        ///
        /// `price_types`, when given, receives the price type of every cell of a
        /// buffered response's rows, row for row; list and streamed responses
        /// leave it untouched.
        pub async fn invoke_generated_endpoint(
            $client: &crate::mdds::MarketDataClient,
            name: &str,
            $args: &EndpointArgs,
            $pt: Option<&mut crate::mdds::decode::extract::PriceTypes>,
        ) -> Result<EndpointOutput, EndpointError> {
            match name {
                $(stringify!($name) => crate::__resolve! { [$($key)*] => dispatch_arm { { $client $args $pt } $shape $name $ret } },)*
                _ => Err(EndpointError::UnknownEndpoint(name.to_string())),
            }
        }

        /// Dispatch a validated endpoint call, draining the response chunk by
        /// chunk instead of buffering it.
        ///
        /// The per-chunk rows are type-erased to `(*const c_void, usize)` so this
        /// one signature serves every tick shape; the caller reconstructs the
        /// typed pointer (the same `#[repr(C)]` tick the buffered array exposes).
        ///
        /// `handler` is invoked once per decoded gRPC chunk on the dispatching
        /// task, with the chunk's rows and the columns its wire carried, each
        /// row's root included. Both are valid only for the duration of the
        /// call: the chunk is freed before the next is fetched, bounding peak
        /// memory to a single chunk regardless of the full result size. A
        /// `Break` from `handler` ends the call, every band of a fan-out
        /// included, and it returns `Ok(())`.
        pub async fn invoke_generated_endpoint_stream(
            $client: &crate::mdds::MarketDataClient,
            name: &str,
            $args: &EndpointArgs,
            mut $handler: impl FnMut(*const core::ffi::c_void, usize, &crate::columns::ColumnPresence) -> core::ops::ControlFlow<()> + Send,
        ) -> Result<(), EndpointError> {
            match name {
                $(stringify!($sname) => crate::__resolve! { [$($skey)*] => dispatch_arm { { $client $args $handler } chunk_stream $sname $sret } },)*
                _ => Err(EndpointError::UnknownEndpoint(name.to_string())),
            }
        }
    };
}

/// One dispatch arm: validate the arguments, build the call, run it.
#[cfg(feature = "__internal")]
macro_rules! dispatch_arm {
    // A list endpoint without optionals: the plain method or its deadline twin.
    ({ $client:ident $args:ident $tail:ident } list $name:ident $ret:ident params {
        all $all:tt req [$({ $p:ident $a:ident req $t:ident [] $d:tt })*] opt [] $($more:tt)*
    }) => {{
        $(dispatch_arm!(@bind $args $t $p refs);)*
        let values = match $args.deadline_setting().builder_deadline() {
            Some(deadline) => pastey::paste!($client.[<$name _with_deadline>]($(dispatch_arm!(@arg $t $p refs),)* deadline)).await?,
            None => $client.$name($(dispatch_arm!(@arg $t $p refs)),*).await?,
        };
        Ok(EndpointOutput::StringList(values))
    }};
    // Every other endpoint is a builder: required arguments construct it,
    // optional ones set it.
    ({ $client:ident $args:ident $tail:ident } $mode:ident $name:ident $ret:ident params {
        all $all:tt
        req [$({ $p:ident $a:ident req $t:ident [] $d:tt })*]
        opt [$({ $op:ident $oa:ident opt $ot:ident [$($ov:tt)?] $od:tt })*]
        $($more:tt)*
    }) => {{
        $(dispatch_arm!(@bind $args $t $p refs);)*
        let mut builder = $client.$name($(dispatch_arm!(@arg $t $p refs)),*);
        $(
            if let Some(value) = dispatch_arm!(@opt $args $ot $op) {
                builder = builder.$op(value);
            }
        )*
        if let Some(deadline) = $args.deadline_setting().builder_deadline() {
            builder = builder.with_deadline(deadline);
        }
        dispatch_arm!(@finish $mode $ret builder $tail)
    }};

    (@finish list $ret:ident $b:ident $tail:ident) => {{
        let values = $b.await?;
        Ok(EndpointOutput::StringList(values))
    }};
    // A raw server stream is collected; its rows carry no header, so every
    // schema column is reported present.
    (@finish stream $ret:ident $b:ident $tail:ident) => {{
        let mut result = Vec::new();
        $b.stream(|chunk| result.extend_from_slice(chunk)).await?;
        let columns = collected_columns(&result);
        pastey::paste!(Ok(EndpointOutput::[<$ret s>](Ticks::new(result, columns))))
    }};
    (@finish chunk_stream $ret:ident $b:ident $handler:ident) => {
        $b.stream_ticks(move |chunk| $handler(chunk.as_ptr().cast::<core::ffi::c_void>(), chunk.len(), chunk.columns()))
            .await
            .map_err(EndpointError::from)
    };
    (@finish $mode:ident $ret:ident $b:ident $pt:ident) => {{
        let result = $b.fetch($pt).await?;
        pastey::paste!(Ok(EndpointOutput::[<$ret s>](result)))
    }};

    (@bind $args:ident Symbols $p:ident $refs:ident) => {
        let $p = $args.required_symbols(stringify!($p))?;
        let $refs: Vec<&str> = $p.iter().map(String::as_str).collect();
    };
    (@bind $args:ident $t:ident $p:ident $refs:ident) => {
        let $p = dispatch_arm!(@req $args $t $p);
    };
    (@req $args:ident Symbol $p:ident) => { $args.required_symbol(stringify!($p))? };
    (@req $args:ident Date $p:ident) => { $args.required_date(stringify!($p))? };
    (@req $args:ident Expiration $p:ident) => { $args.required_expiration(stringify!($p))? };
    (@req $args:ident Year $p:ident) => { $args.required_year(stringify!($p))? };
    // A request type and a time of day are validated by the builder.
    (@req $args:ident $t:ident $p:ident) => { $args.required_str(stringify!($p))? };
    (@arg Symbols $p:ident $refs:ident) => { &$refs };
    (@arg $t:ident $p:ident $refs:ident) => { $p };
    (@opt $args:ident Bool $p:ident) => { $args.optional_bool(stringify!($p))? };
    (@opt $args:ident Int $p:ident) => { $args.optional_int32(stringify!($p))? };
    (@opt $args:ident Float $p:ident) => { $args.optional_float64(stringify!($p))? };
    (@opt $args:ident Date $p:ident) => { $args.optional_date(stringify!($p))? };
    (@opt $args:ident Interval $p:ident) => { $args.optional_interval(stringify!($p))? };
    (@opt $args:ident Strike $p:ident) => { $args.optional_strike(stringify!($p))? };
    (@opt $args:ident Right $p:ident) => { $args.optional_right(stringify!($p))? };
    (@opt $args:ident $t:ident $p:ident) => { $args.optional_str(stringify!($p))? };
}
// The dispatch is only compiled with `__internal`: its arms reference
// `EndpointArgs`, `EndpointOutput` and `EndpointError`, all gated above.
#[cfg(feature = "__internal")]
crate::__endpoints! { endpoint_dispatch { @collect } }

#[cfg(all(test, feature = "__internal"))]
mod tests {
    use super::*;

    #[test]
    fn endpoint_args_default_has_no_timeout() {
        let args = EndpointArgs::new();
        assert_eq!(args.deadline_setting(), DeadlineSetting::Unset);
    }

    #[test]
    fn with_timeout_ms_attaches_deadline() {
        let args = EndpointArgs::new().with_timeout_ms(60_000);
        assert_eq!(args.deadline_setting(), DeadlineSetting::Millis(60_000));
    }

    #[test]
    fn clear_timeout_drops_deadline() {
        let mut args = EndpointArgs::new().with_timeout_ms(60_000);
        args.clear_timeout();
        // Clearing returns to "unset" — the configured default applies on
        // the next dispatch, NOT the disabled (unbounded) state.
        assert_eq!(args.deadline_setting(), DeadlineSetting::Unset);
    }

    /// `timeout_ms == 0` is the deadline opt-out: it records `Disabled`, a
    /// state distinct from `Unset`. The generated dispatch reads
    /// `deadline_setting()` to tell them apart — `Disabled` runs the call
    /// unbounded while `Unset` falls back to the configured
    /// `request_timeout_secs` default.
    #[test]
    fn with_timeout_ms_zero_records_disabled() {
        let args = EndpointArgs::new().with_timeout_ms(0);
        assert_eq!(args.deadline_setting(), DeadlineSetting::Disabled);
    }

    /// `set_timeout_ms(0)` matches the same normalization as `with_timeout_ms`.
    #[test]
    fn set_timeout_ms_zero_records_disabled() {
        let mut args = EndpointArgs::new().with_timeout_ms(60_000);
        args.set_timeout_ms(0);
        assert_eq!(args.deadline_setting(), DeadlineSetting::Disabled);
    }

    /// Positive timeout values pass through unchanged.
    #[test]
    fn with_timeout_ms_positive_value_stored() {
        let args = EndpointArgs::new().with_timeout_ms(1);
        assert_eq!(args.deadline_setting(), DeadlineSetting::Millis(1));
    }

    /// The tri-state maps to the builder deadline the generated dispatch
    /// applies. This is the crux of the registry/FFI opt-out fix: `Unset`
    /// must NOT apply a deadline (the builder keeps the configured default),
    /// `Disabled` must apply an explicit `Duration::ZERO` (which
    /// `with_deadline` / `effective_deadline` resolve to "no deadline"), and
    /// `Millis(n)` must apply that exact bound.
    #[test]
    fn deadline_setting_maps_to_builder_deadline() {
        use std::time::Duration;
        assert_eq!(DeadlineSetting::Unset.builder_deadline(), None);
        assert_eq!(
            DeadlineSetting::Disabled.builder_deadline(),
            Some(Duration::ZERO)
        );
        assert_eq!(
            DeadlineSetting::Millis(250).builder_deadline(),
            Some(Duration::from_millis(250))
        );
        // The disabled builder deadline must route through effective_deadline
        // to "no deadline" even against a positive configured default, so the
        // opt-out actually holds end-to-end.
        let disabled = DeadlineSetting::Disabled.builder_deadline();
        assert_eq!(
            crate::mdds::macros::effective_deadline(disabled, 300),
            None,
            "with_timeout_ms(0) must disable the deadline, not fall back to the default"
        );
        // Unset routes to the configured default.
        let unset = DeadlineSetting::Unset.builder_deadline();
        assert_eq!(
            crate::mdds::macros::effective_deadline(unset, 300),
            Some(Duration::from_secs(300)),
            "an unset deadline must fall back to the configured default"
        );
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
    fn required_date_accepts_both_compact_and_iso_forms() {
        // `date` accepts the same two textual forms `expiration` always
        // accepted; ISO input is canonicalized to the compact wire form
        // at request construction by `wire_semantics::normalize_date`.
        let mut args = EndpointArgs::new();
        args.insert("date".into(), EndpointArgValue::Str("2026-04-09".into()));
        assert_eq!(args.required_date("date").unwrap(), "2026-04-09");

        let mut args = EndpointArgs::new();
        args.insert("date".into(), EndpointArgValue::Str("20260409".into()));
        assert_eq!(args.required_date("date").unwrap(), "20260409");
    }

    #[test]
    fn required_date_rejects_malformed_shapes() {
        for bad in ["2026-4-09", "garbage", "202604", "2026/04/09"] {
            let mut args = EndpointArgs::new();
            args.insert("date".into(), EndpointArgValue::Str(bad.into()));
            let err = args.required_date("date").unwrap_err();
            assert!(
                matches!(err, EndpointError::InvalidParams(message) if message.contains("YYYYMMDD")),
                "expected shape rejection for {bad}"
            );
        }
    }

    #[test]
    fn required_expiration_refuses_zero() {
        let mut args = EndpointArgs::new();
        args.insert("expiration".into(), EndpointArgValue::Str("0".into()));
        assert!(args.required_expiration("expiration").is_err());
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
        // ISO-dashed `YYYY-MM-DD` is accepted; assert on an actually-invalid form.
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
    fn optional_expiration_refuses_zero() {
        let mut args = EndpointArgs::new();
        args.insert("expiration".into(), EndpointArgValue::Str("0".into()));
        assert!(args.optional_expiration("expiration").is_err());
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

    /// Verify registry metadata integrity: no duplicates, all categories
    /// present, and no empty descriptions or rest_paths.
    ///
    /// Note: dispatch-registry alignment (every registry name has a
    /// generated match arm) is guaranteed at build time — both are
    /// generated from the same `ParsedEndpoints` vec in
    /// `build_support/endpoints/`. A name mismatch is structurally
    /// impossible without a build failure. This test validates the
    /// registry's content, not dispatch coverage.
    #[test]
    fn registry_metadata_integrity() {
        use crate::mdds::registry::ENDPOINTS;

        let registry_names: std::collections::HashSet<&str> =
            ENDPOINTS.iter().map(|e| e.name).collect();
        assert_eq!(
            registry_names.len(),
            ENDPOINTS.len(),
            "duplicate names in ENDPOINTS"
        );
        assert!(
            !ENDPOINTS.is_empty(),
            "generated registry unexpectedly contains no endpoints"
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
