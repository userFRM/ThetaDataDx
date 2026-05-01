//! MCP (Model Context Protocol) server for ThetaDataDx.
//!
//! Gives any MCP-compatible LLM client instant access to ThetaData market
//! data via structured tool calls over stdio JSON-RPC 2.0.
//!
//! Architecture:
//! ```text
//! MCP-compatible LLM client
//!     |  JSON-RPC 2.0 over stdio
//!     v
//! thetadatadx-mcp (long-running process)
//!     |  Single ThetaDataDx client, authenticated once
//!     v
//! ThetaData servers (MDDS gRPC + FPSS TCP)
//! ```
//!
//! The server authenticates ONCE at startup, keeps the ThetaDataDx client alive,
//! and serves tool calls instantly with no per-request auth overhead.
//!
//! Tool definitions and dispatch are driven by the shared endpoint registry
//! (`thetadatadx::registry`). When ThetaData adds a new RPC, add
//! one entry to the registry and this server picks it up automatically.

use std::io::Write as _;
use std::sync::Arc;

use serde::Serialize;
use sonic_rs::{json, JsonContainerTrait, JsonValueMutTrait, JsonValueTrait, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::OnceCell;

use thetadatadx::endpoint::{self, EndpointArgValue, EndpointArgs, EndpointError, EndpointOutput};
use thetadatadx::{
    param_type_to_json_type, Credentials, DirectConfig, EndpointMeta, ParamMeta, ThetaDataDx,
    ENDPOINTS,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2024-11-05"];

// ═══════════════════════════════════════════════════════════════════════════
//  JSON-RPC types
// ═══════════════════════════════════════════════════════════════════════════

/// Validated JSON-RPC 2.0 request (built from raw JSON, not via Deserialize).
struct JsonRpcRequest {
    /// Already validated to be "2.0" during parsing.
    _jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message }),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  JSON-RPC parsing with proper error codes
// ═══════════════════════════════════════════════════════════════════════════

/// Parse a raw JSON line into a validated `JsonRpcRequest`.
fn parse_jsonrpc_request(line: &str) -> Result<JsonRpcRequest, JsonRpcResponse> {
    let val: Value = sonic_rs::from_str(line).map_err(|e| {
        JsonRpcResponse::error(Value::new_null(), -32700, format!("Parse error: {e}"))
    })?;

    let obj = val.as_object().ok_or_else(|| {
        JsonRpcResponse::error(
            Value::new_null(),
            -32600,
            "Invalid request: expected JSON object".into(),
        )
    })?;

    let id = obj.get(&"id").cloned();
    let id_for_error = id.clone().unwrap_or_else(Value::new_null);

    // Validate `id` type: JSON-RPC 2.0 spec allows number, string, or null.
    // Reject arrays, objects, and booleans.
    if let Some(ref id_val) = id {
        if !(id_val.is_number() || id_val.is_str() || id_val.is_null()) {
            let type_name = if id_val.is_boolean() {
                "boolean"
            } else if id_val.is_array() {
                "array"
            } else if id_val.is_object() {
                "object"
            } else {
                "unknown"
            };
            return Err(JsonRpcResponse::error(
                Value::new_null(),
                -32600,
                format!(
                    "Invalid request: 'id' must be a number, string, or null, got: {type_name}"
                ),
            ));
        }
    }

    let jsonrpc = obj
        .get(&"jsonrpc")
        .and_then(|v: &Value| v.as_str())
        .ok_or_else(|| {
            JsonRpcResponse::error(
                id_for_error.clone(),
                -32600,
                "Invalid request: missing or non-string 'jsonrpc' field".into(),
            )
        })?
        .to_string();

    if jsonrpc != "2.0" {
        return Err(JsonRpcResponse::error(
            id_for_error.clone(),
            -32600,
            format!(
                "Invalid request: unsupported JSON-RPC version '{}', expected '2.0'",
                jsonrpc
            ),
        ));
    }

    let method = obj
        .get(&"method")
        .and_then(|v: &Value| v.as_str())
        .ok_or_else(|| {
            JsonRpcResponse::error(
                id_for_error.clone(),
                -32600,
                "Invalid request: missing or non-string 'method' field".into(),
            )
        })?
        .to_string();

    let params = obj.get(&"params").cloned().unwrap_or(json!({}));

    // Validate `params` type: JSON-RPC 2.0 spec requires a structured value
    // (object or array), but MCP only uses objects. Reject non-object types.
    if !params.is_object() {
        let type_name = if params.is_array() {
            "array"
        } else if params.is_str() {
            "string"
        } else if params.is_number() {
            "number"
        } else if params.is_boolean() {
            "boolean"
        } else if params.is_null() {
            "null"
        } else {
            "unknown"
        };
        return Err(JsonRpcResponse::error(
            id_for_error.clone(),
            -32600,
            format!("Invalid request: 'params' must be an object or absent, got: {type_name}"),
        ));
    }

    Ok(JsonRpcRequest {
        _jsonrpc: jsonrpc,
        id,
        method,
        params,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
//  Error sanitization
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum length for sanitized error messages exposed to MCP clients.
const MAX_ERROR_LEN: usize = 200;

/// Strip potential session UUIDs, email addresses, hex tokens, and other
/// sensitive data from error messages before sending them to MCP clients.
///
/// Also truncates to [`MAX_ERROR_LEN`] chars to avoid leaking verbose
/// backtraces or internal state.
fn sanitize_error(msg: &str) -> String {
    let mut result = String::with_capacity(msg.len().min(MAX_ERROR_LEN + 16));
    let bytes = msg.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        // UUID pattern: 8-4-4-4-12 hex chars
        if i + 36 <= len && is_uuid_at(bytes, i) {
            result.push_str("[REDACTED]");
            i += 36;
        // Email pattern: contains @ with word chars on both sides
        } else if bytes[i] == b'@' && i > 0 && is_email_boundary(&result, bytes, i, len) {
            // Walk back to erase the local part we already pushed
            while result.ends_with(|c: char| {
                c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' || c == '+'
            }) {
                result.pop();
            }
            // Skip forward past the domain part
            i += 1; // skip @
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'-')
            {
                i += 1;
            }
            result.push_str("[REDACTED]");
        // Long hex token: 32+ consecutive hex chars (API keys, session tokens)
        } else if bytes[i].is_ascii_hexdigit() && is_hex_token_at(bytes, i) {
            result.push_str("[REDACTED]");
            let start = i;
            while i < len && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            let _ = start; // consumed
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }

        // Truncate early if we've already exceeded the limit.
        if result.len() >= MAX_ERROR_LEN {
            result.truncate(MAX_ERROR_LEN);
            result.push_str("...");
            return result;
        }
    }
    result
}

fn is_uuid_at(bytes: &[u8], pos: usize) -> bool {
    let groups = [8, 4, 4, 4, 12];
    let mut offset = pos;
    for (gi, &count) in groups.iter().enumerate() {
        if gi > 0 {
            if offset >= bytes.len() || bytes[offset] != b'-' {
                return false;
            }
            offset += 1;
        }
        for _ in 0..count {
            if offset >= bytes.len() || !bytes[offset].is_ascii_hexdigit() {
                return false;
            }
            offset += 1;
        }
    }
    true
}

/// Check if we're at an `@` that looks like part of an email address.
fn is_email_boundary(result: &str, bytes: &[u8], at_pos: usize, len: usize) -> bool {
    // Must have a word char before @
    let has_local = result
        .as_bytes()
        .last()
        .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_');
    // Must have a word char after @
    let has_domain = at_pos + 1 < len && bytes[at_pos + 1].is_ascii_alphanumeric();
    has_local && has_domain
}

/// Check if position starts a run of 32+ hex characters (likely an API key or token).
fn is_hex_token_at(bytes: &[u8], pos: usize) -> bool {
    let mut count = 0;
    let mut i = pos;
    while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
        count += 1;
        i += 1;
        if count >= 32 {
            return true;
        }
    }
    false
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tool definitions — generated from endpoint registry + generated utilities
// ═══════════════════════════════════════════════════════════════════════════

fn tool_definitions() -> Vec<Value> {
    let mut tools = Vec::with_capacity(ENDPOINTS.len() + 3);

    // Registry-driven: every MddsClient endpoint
    for ep in ENDPOINTS {
        let mut props = sonic_rs::Object::new();
        let mut required = Vec::new();
        for p in ep.params {
            props.insert(
                &p.name,
                json!({
                    "type": param_type_to_json_type(p.param_type),
                    "description": mcp_param_description(ep, p),
                }),
            );
            if p.required && !required.contains(&p.name) {
                required.push(p.name);
            }
        }
        tools.push(json!({
            "name": ep.name,
            "description": ep.description,
            "inputSchema": {
                "type": "object",
                "properties": props,
                "required": required,
            }
        }));
    }

    push_generated_utility_tool_definitions(&mut tools);

    tools
}

/// Return the LLM-facing MCP parameter description for a registry endpoint.
///
/// Most parameters can use the shared registry wording directly. A small set of
/// option bulk-query parameters benefit from MCP-specific clarification because
/// the MCP transport uses `"0"` as the wildcard sentinel instead of REST's
/// `*`, and `strike_range` only filters an already-bulk selection.
fn mcp_param_description(ep: &EndpointMeta, param: &ParamMeta) -> String {
    if ep.category == "option" {
        match param.name {
            "strike" => {
                return format!(
                    "{}. Use \"0\" for wildcard/bulk strike selection on endpoints that support bulk option queries.",
                    param.description
                );
            }
            "expiration" => {
                return format!(
                    "{}. Use \"0\" for wildcard/bulk expiration selection on endpoints that support bulk option queries.",
                    param.description
                );
            }
            "strike_range" => {
                return format!(
                    "{}. Filters a wildcard/bulk option selection around spot/ATM; it does not expand a pinned strike.",
                    param.description
                );
            }
            _ => {}
        }
    }

    param.description.to_string()
}

fn negotiate_protocol_version(client_version: Option<&str>) -> &'static str {
    client_version
        .and_then(|version| {
            SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .find(|supported| version == *supported)
        })
        .unwrap_or(PROTOCOL_VERSION)
}

// ═══════════════════════════════════════════════════════════════════════════
//  Serialization helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Render ThetaData's option right code as a human-readable MCP field value.
fn option_right_value(right: i32) -> Value {
    match right {
        67 => "C".into(),
        80 => "P".into(),
        _ => right.into(),
    }
}

/// Attach wildcard option contract identifiers to a serialized tick row.
///
/// ThetaData only populates these fields on wildcard/bulk queries, where
/// callers request `expiration = "0"` and/or `strike = "0"`. Single-contract
/// queries leave them as zero, so MCP omits them to keep those payloads lean.
fn insert_contract_id_fields(row: &mut Value, expiration: i32, strike: f64, right: i32) {
    if expiration == 0 {
        return;
    }

    let object = row
        .as_object_mut()
        .expect("serialized tick rows must always be JSON objects");
    object.insert(
        "expiration",
        sonic_rs::to_value(&expiration).expect("i32 contract expiration should serialize"),
    );
    object.insert(
        "strike",
        sonic_rs::to_value(&strike).expect("f64 contract strike should serialize"),
    );
    object.insert("right", option_right_value(right));
}

fn serialize_eod_ticks(ticks: &[tdbe::types::tick::EodTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            let mut row = json!({
                "date": t.date,
                "ms_of_day": t.ms_of_day,
                "ms_of_day2": t.ms_of_day2,
                "open": t.open,
                "high": t.high,
                "low": t.low,
                "close": t.close,
                "volume": t.volume,
                "count": t.count,
                "bid_exchange": t.bid_exchange,
                "bid": t.bid,
                "bid_condition": t.bid_condition,
                "ask_exchange": t.ask_exchange,
                "ask": t.ask,
                "ask_condition": t.ask_condition,
                "bid_size": t.bid_size,
                "ask_size": t.ask_size,
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_ohlc_ticks(ticks: &[tdbe::types::tick::OhlcTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            let mut row = json!({
                "date": t.date,
                "ms_of_day": t.ms_of_day,
                "open": t.open,
                "high": t.high,
                "low": t.low,
                "close": t.close,
                "volume": t.volume,
                "count": t.count,
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_trade_ticks(ticks: &[tdbe::types::tick::TradeTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            let mut row = json!({
                "date": t.date,
                "ms_of_day": t.ms_of_day,
                "price": t.price,
                "size": t.size,
                "exchange": t.exchange,
                "condition": t.condition,
                "sequence": t.sequence,
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_quote_ticks(ticks: &[tdbe::types::tick::QuoteTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            let mut row = json!({
                "date": t.date,
                "ms_of_day": t.ms_of_day,
                "bid": t.bid,
                "bid_size": t.bid_size,
                "bid_exchange": t.bid_exchange,
                "bid_condition": t.bid_condition,
                "ask": t.ask,
                "ask_size": t.ask_size,
                "ask_exchange": t.ask_exchange,
                "ask_condition": t.ask_condition,
                "midpoint": t.midpoint,
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_trade_quote_ticks(ticks: &[tdbe::types::tick::TradeQuoteTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            let mut row = json!({
                "date": t.date,
                "ms_of_day": t.ms_of_day,
                "price": t.price,
                "size": t.size,
                "exchange": t.exchange,
                "condition": t.condition,
                "sequence": t.sequence,
                "ext_condition1": t.ext_condition1,
                "ext_condition2": t.ext_condition2,
                "ext_condition3": t.ext_condition3,
                "ext_condition4": t.ext_condition4,
                "condition_flags": t.condition_flags,
                "price_flags": t.price_flags,
                "volume_type": t.volume_type,
                "records_back": t.records_back,
                "quote_ms_of_day": t.quote_ms_of_day,
                "bid": t.bid,
                "bid_size": t.bid_size,
                "bid_exchange": t.bid_exchange,
                "bid_condition": t.bid_condition,
                "ask": t.ask,
                "ask_size": t.ask_size,
                "ask_exchange": t.ask_exchange,
                "ask_condition": t.ask_condition,
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_open_interest_ticks(ticks: &[tdbe::types::tick::OpenInterestTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            let mut row =
                json!({"date": t.date, "ms_of_day": t.ms_of_day, "open_interest": t.open_interest});
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_market_value_ticks(ticks: &[tdbe::types::tick::MarketValueTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            let mut row = json!({
                "date": t.date, "ms_of_day": t.ms_of_day,
                "market_bid": t.market_bid, "market_ask": t.market_ask,
                "market_price": t.market_price,
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_greeks_ticks(ticks: &[tdbe::types::tick::GreeksTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            let mut row = json!({
                "date": t.date, "ms_of_day": t.ms_of_day,
                "implied_volatility": t.implied_volatility,
                "delta": t.delta, "gamma": t.gamma, "theta": t.theta,
                "vega": t.vega, "rho": t.rho, "iv_error": t.iv_error,
                "vanna": t.vanna, "charm": t.charm, "vomma": t.vomma,
                "veta": t.veta, "speed": t.speed, "zomma": t.zomma,
                "color": t.color, "ultima": t.ultima,
                "d1": t.d1, "d2": t.d2,
                "dual_delta": t.dual_delta, "dual_gamma": t.dual_gamma,
                "epsilon": t.epsilon, "lambda": t.lambda, "vera": t.vera,
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_iv_ticks(ticks: &[tdbe::types::tick::IvTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            let mut row = json!({
                "date": t.date, "ms_of_day": t.ms_of_day,
                "implied_volatility": t.implied_volatility, "iv_error": t.iv_error,
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_price_ticks(ticks: &[tdbe::types::tick::PriceTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            json!({
                "date": t.date, "ms_of_day": t.ms_of_day,
                "price": t.price,
            })
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_calendar_days(days: &[tdbe::types::tick::CalendarDay]) -> Value {
    let rows: Vec<Value> = days
        .iter()
        .map(|d| {
            json!({
                "date": d.date, "is_open": d.is_open,
                "open_time": d.open_time, "close_time": d.close_time,
                "status": d.status,
            })
        })
        .collect();
    json!({ "days": rows, "count": rows.len() })
}

fn serialize_interest_rate_ticks(ticks: &[tdbe::types::tick::InterestRateTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| json!({"date": t.date, "ms_of_day": t.ms_of_day, "rate": t.rate}))
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_option_contracts(contracts: &[tdbe::types::tick::OptionContract]) -> Value {
    let rows: Vec<Value> = contracts
        .iter()
        .map(|c| {
            json!({
                "root": c.root, "expiration": c.expiration,
                "strike": c.strike, "right": c.right,
            })
        })
        .collect();
    json!({ "contracts": rows, "count": rows.len() })
}

fn serialize_string_list(name: &str, values: &[String]) -> Value {
    let key = if name.ends_with("_symbols") {
        "symbols"
    } else if name.ends_with("_dates") {
        "dates"
    } else if name.ends_with("_expirations") {
        "expirations"
    } else if name.ends_with("_strikes") {
        "strikes"
    } else {
        "values"
    };
    json!({ key: values, "count": values.len() })
}

fn serialize_endpoint_output(name: &str, output: &EndpointOutput) -> Value {
    match output {
        EndpointOutput::StringList(values) => serialize_string_list(name, values),
        EndpointOutput::EodTicks(ticks) => serialize_eod_ticks(ticks),
        EndpointOutput::OhlcTicks(ticks) => serialize_ohlc_ticks(ticks),
        EndpointOutput::TradeTicks(ticks) => serialize_trade_ticks(ticks),
        EndpointOutput::QuoteTicks(ticks) => serialize_quote_ticks(ticks),
        EndpointOutput::TradeQuoteTicks(ticks) => serialize_trade_quote_ticks(ticks),
        EndpointOutput::OpenInterestTicks(ticks) => serialize_open_interest_ticks(ticks),
        EndpointOutput::MarketValueTicks(ticks) => serialize_market_value_ticks(ticks),
        EndpointOutput::GreeksTicks(ticks) => serialize_greeks_ticks(ticks),
        EndpointOutput::IvTicks(ticks) => serialize_iv_ticks(ticks),
        EndpointOutput::PriceTicks(ticks) => serialize_price_ticks(ticks),
        EndpointOutput::CalendarDays(days) => serialize_calendar_days(days),
        EndpointOutput::InterestRateTicks(ticks) => serialize_interest_rate_ticks(ticks),
        EndpointOutput::OptionContracts(contracts) => serialize_option_contracts(contracts),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Argument extraction helpers
// ═══════════════════════════════════════════════════════════════════════════

fn arg_f64(args: &Value, key: &str) -> Result<f64, String> {
    args.get(key)
        .and_then(|v: &Value| v.as_f64())
        .ok_or_else(|| format!("missing required number argument: {key}"))
}

fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v: &Value| v.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| format!("missing required string argument: {key}"))
}

fn convert_endpoint_args(args: &Value) -> Result<EndpointArgs, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "tool arguments must be a JSON object".to_string())?;
    let mut converted = EndpointArgs::new();
    for (key, value) in obj.iter() {
        let arg_value = if let Some(v) = value.as_str() {
            EndpointArgValue::Str(v.to_string())
        } else if let Some(v) = value.as_i64() {
            EndpointArgValue::Int(v)
        } else if let Some(v) = value.as_f64() {
            EndpointArgValue::Float(v)
        } else if let Some(v) = value.as_bool() {
            EndpointArgValue::Bool(v)
        } else {
            return Err(format!(
                "argument '{}' must be a string, integer, number, or boolean",
                key
            ));
        };
        converted.insert(key.to_string(), arg_value);
    }
    Ok(converted)
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tool execution — registry-driven dispatch
// ═══════════════════════════════════════════════════════════════════════════

enum ToolError {
    /// -32602: Invalid params
    InvalidParams(String),
    /// -32000: Server error
    ServerError(String),
}

macro_rules! param {
    ($expr:expr) => {
        ($expr).map_err(ToolError::InvalidParams)?
    };
}

include!("utilities.rs");

async fn execute_tool(
    client: Option<&ThetaDataDx>,
    name: &str,
    args: &Value,
    start_time: std::time::Instant,
) -> Result<Value, ToolError> {
    if let Some(result) = try_execute_generated_utility(client, name, args, start_time).await {
        return result;
    }

    // ── Online tools (require connected client) ─────────────────────
    let client = client.ok_or_else(|| {
        ToolError::ServerError(
            "ThetaData client not connected. Set THETA_EMAIL + THETA_PASSWORD env vars or use --creds flag.".to_string(),
        )
    })?;

    let converted_args = param!(convert_endpoint_args(args));
    let output = match endpoint::invoke_endpoint(client, name, &converted_args).await {
        Ok(output) => output,
        Err(EndpointError::InvalidParams(message)) => {
            return Err(ToolError::InvalidParams(message));
        }
        Err(EndpointError::UnknownEndpoint(_)) => {
            return Err(ToolError::InvalidParams(format!("unknown tool: {name}")));
        }
        Err(EndpointError::Server(error)) => {
            return Err(ToolError::ServerError(sanitize_error(&error.to_string())));
        }
    };

    Ok(serialize_endpoint_output(name, &output))
}

// ═══════════════════════════════════════════════════════════════════════════
//  Request handling
// ═══════════════════════════════════════════════════════════════════════════

async fn handle_request(
    req: &JsonRpcRequest,
    client: &Arc<OnceCell<ThetaDataDx>>,
    start_time: std::time::Instant,
) -> JsonRpcResponse {
    // OnceCell::get is lock-free; no guard is held across the awaits below.
    let client = client.get();
    let id = req.id.clone().unwrap_or(Value::new_null());

    match req.method.as_str() {
        "initialize" => {
            let client_version = req
                .params
                .get("protocolVersion")
                .and_then(|v: &Value| v.as_str())
                .filter(|version| !version.is_empty());
            let protocol_version = negotiate_protocol_version(client_version);

            JsonRpcResponse::success(
                id,
                json!({
                    "protocolVersion": protocol_version,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "thetadatadx-mcp",
                        "version": VERSION,
                    }
                }),
            )
        }

        "notifications/initialized" => JsonRpcResponse::success(id, Value::new_null()),

        "tools/list" => {
            let tools = tool_definitions();
            JsonRpcResponse::success(id, json!({ "tools": tools }))
        }

        "tools/call" => {
            let tool_name = req
                .params
                .get("name")
                .and_then(|v: &Value| v.as_str())
                .unwrap_or("");
            let arguments = req.params.get("arguments").cloned().unwrap_or(json!({}));

            match execute_tool(client, tool_name, &arguments, start_time).await {
                Ok(mut result) => build_tool_call_response(id, &mut result),
                Err(ToolError::InvalidParams(msg)) => {
                    JsonRpcResponse::error(id, -32602, format!("Invalid params: {msg}"))
                }
                Err(ToolError::ServerError(msg)) => {
                    JsonRpcResponse::error(id, -32000, format!("Server error: {msg}"))
                }
            }
        }

        _ => JsonRpcResponse::error(id, -32601, format!("Method not found: {}", req.method)),
    }
}

/// Build the JSON-RPC response for a successful `tools/call` invocation.
///
/// Canonicalises non-finite f64 leaves to JSON `null` (cross-language SDK
/// agreement, see `json_canon`) and surfaces any residual serialisation
/// failure as a JSON-RPC `-32603` Internal Error so the LLM client never
/// receives a successful but empty `tools/call` result. Lifted out of the
/// `tools/call` arm so the regression test for issue #459 can exercise the
/// canonicalisation path without spinning up a live `ThetaDataDx` client.
fn build_tool_call_response(id: Value, result: &mut Value) -> JsonRpcResponse {
    match json_canon::canonicalize_and_serialize(result) {
        Ok(text) => JsonRpcResponse::success(
            id,
            json!({
                "content": [{
                    "type": "text",
                    "text": text,
                }]
            }),
        ),
        Err(err) => JsonRpcResponse::error(
            id,
            -32603,
            format!("Internal error: failed to serialise tool result: {err}"),
        ),
    }
}

fn emit_response(stdout: &std::io::Stdout, resp: &JsonRpcResponse) {
    match sonic_rs::to_string(resp) {
        Ok(out) => {
            let mut lock = stdout.lock();
            let _ = writeln!(lock, "{out}");
            let _ = lock.flush();
        }
        Err(e) => {
            eprintln!("FATAL: failed to serialize JSON-RPC response: {e}");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  CLI argument parsing (minimal, no clap dependency)
// ═══════════════════════════════════════════════════════════════════════════

struct Args {
    creds_path: Option<String>,
}

fn parse_args() -> Args {
    let mut args = Args { creds_path: None };
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--creds" => {
                args.creds_path = argv.next();
            }
            "--help" | "-h" => {
                eprintln!("thetadatadx-mcp v{VERSION}");
                eprintln!("MCP server for ThetaData market data");
                eprintln!();
                eprintln!("USAGE:");
                eprintln!("  thetadatadx-mcp [OPTIONS]");
                eprintln!();
                eprintln!("OPTIONS:");
                eprintln!("  --creds <PATH>  Path to creds.txt (email + password)");
                eprintln!("  -h, --help      Print help");
                eprintln!();
                eprintln!("ENVIRONMENT:");
                eprintln!("  THETA_EMAIL     ThetaData account email");
                eprintln!("  THETA_PASSWORD  ThetaData account password");
                eprintln!("  RUST_LOG        Log level (default: info)");
                eprintln!();
                eprintln!("Credentials are read from env vars first, then --creds file.");
                eprintln!("If neither is provided, the server starts in offline mode");
                eprintln!("(only ping, all_greeks, and implied_volatility tools work).");
                std::process::exit(0);
            }
            _ => {
                eprintln!("unknown argument: {arg}");
                std::process::exit(1);
            }
        }
    }
    args
}

// ═══════════════════════════════════════════════════════════════════════════
//  Main
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli_args = parse_args();
    let start_time = std::time::Instant::now();

    // ── Resolve credentials ─────────────────────────────────────────
    let creds = if let (Ok(email), Ok(password)) = (
        std::env::var("THETA_EMAIL"),
        std::env::var("THETA_PASSWORD"),
    ) {
        tracing::info!("using credentials from THETA_EMAIL/THETA_PASSWORD env vars");
        Some(Credentials::new(email, password))
    } else if let Some(path) = &cli_args.creds_path {
        match Credentials::from_file(path) {
            Ok(c) => {
                tracing::info!(path = %path, "loaded credentials from file");
                Some(c)
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "failed to load credentials, starting in offline mode");
                None
            }
        }
    } else {
        tracing::info!("no credentials provided, starting in offline mode (ping, all_greeks, implied_volatility only)");
        None
    };

    // ── Connect to ThetaData in the background ──────────────────────
    // We must NOT block here: MCP clients (e.g. Claude Code) send `initialize`
    // immediately after spawning and time out waiting for a response if we block
    // on the ThetaData gRPC handshake (~800 ms).  Wrap the client in an
    // Arc<RwLock> so the background task can populate it while the stdin loop
    // is already running.
    let client: Arc<OnceCell<ThetaDataDx>> = Arc::new(OnceCell::new());

    if let Some(creds) = creds {
        let client_bg = Arc::clone(&client);
        tokio::spawn(async move {
            match ThetaDataDx::connect(&creds, DirectConfig::production()).await {
                Ok(c) => {
                    tracing::info!("connected to ThetaData MDDS");
                    if client_bg.set(c).is_err() {
                        tracing::warn!("client already initialised; dropping duplicate connect");
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "failed to connect to ThetaData, running in offline mode");
                }
            }
        });
    }

    // ── Main JSON-RPC loop over stdin ───────────────────────────────
    tracing::info!(
        version = VERSION,
        "thetadatadx-mcp ready, reading JSON-RPC from stdin"
    );

    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    let stdout = std::io::stdout();

    while let Ok(Some(raw_line)) = lines.next_line().await {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let req = match parse_jsonrpc_request(line) {
            Ok(r) => r,
            Err(resp) => {
                tracing::warn!("invalid JSON-RPC request");
                emit_response(&stdout, &resp);
                continue;
            }
        };

        let is_notification = req.id.is_none();

        let resp = handle_request(&req, &client, start_time).await;

        if !is_notification {
            emit_response(&stdout, &resp);
        }
    }

    tracing::info!("stdin closed, shutting down");
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use tdbe::types::tick::{EodTick, GreeksTick, QuoteTick, TradeQuoteTick};

    fn sample_eod_tick(expiration: i32, strike: f64, right: i32) -> EodTick {
        EodTick {
            ms_of_day: 34_200_000,
            ms_of_day2: 57_600_000,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 10,
            count: 1,
            bid_size: 2,
            bid_exchange: 11,
            bid: 0.9,
            bid_condition: 22,
            ask_size: 3,
            ask_exchange: 33,
            ask: 1.1,
            ask_condition: 44,
            date: 20221219,
            expiration,
            strike,
            right,
        }
    }

    fn sample_greeks_tick(expiration: i32, strike: f64, right: i32) -> GreeksTick {
        GreeksTick {
            ms_of_day: 0,
            implied_volatility: 0.25,
            delta: 0.5,
            gamma: 0.1,
            theta: -0.01,
            vega: 0.2,
            rho: 0.05,
            iv_error: 0.0,
            vanna: 0.0,
            charm: 0.0,
            vomma: 0.0,
            veta: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
            ultima: 0.0,
            d1: 0.0,
            d2: 0.0,
            dual_delta: 0.0,
            dual_gamma: 0.0,
            epsilon: 0.0,
            lambda: 0.0,
            vera: 0.0,
            date: 20221219,
            expiration,
            strike,
            right,
        }
    }

    #[test]
    fn negotiate_protocol_version_uses_requested_supported_version() {
        assert_eq!(negotiate_protocol_version(Some("2025-11-25")), "2025-11-25");
        assert_eq!(negotiate_protocol_version(Some("2024-11-05")), "2024-11-05");
    }

    #[test]
    fn negotiate_protocol_version_falls_back_to_latest_supported_version() {
        assert_eq!(negotiate_protocol_version(None), PROTOCOL_VERSION);
        assert_eq!(negotiate_protocol_version(Some("")), PROTOCOL_VERSION);
        assert_eq!(
            negotiate_protocol_version(Some("2099-01-01")),
            PROTOCOL_VERSION
        );
    }

    #[test]
    fn tool_schemas_do_not_emit_duplicate_required_parameters() {
        for tool in tool_definitions() {
            let name = tool
                .get("name")
                .and_then(|value: &Value| value.as_str())
                .unwrap_or("<unnamed>");
            let Some(required) = tool
                .pointer(["inputSchema", "required"])
                .and_then(|value| value.as_array())
            else {
                continue;
            };

            let mut seen = HashSet::new();
            for param in required.as_slice() {
                let param = param
                    .as_str()
                    .expect("tool required parameters must be strings");
                assert!(
                    seen.insert(param),
                    "tool {name} emits duplicate required parameter {param}"
                );
            }
        }
    }

    #[test]
    fn tool_schemas_clarify_option_bulk_wildcards_for_llm_consumers() {
        let tool = tool_definitions()
            .into_iter()
            .find(|tool| {
                tool.get("name").and_then(|value: &Value| value.as_str())
                    == Some("option_history_greeks_eod")
            })
            .expect("option_history_greeks_eod tool should exist");

        let strike = tool
            .pointer(["inputSchema", "properties", "strike", "description"])
            .and_then(|value| value.as_str())
            .expect("strike description should exist");
        let expiration = tool
            .pointer(["inputSchema", "properties", "expiration", "description"])
            .and_then(|value| value.as_str())
            .expect("expiration description should exist");
        let strike_range = tool
            .pointer(["inputSchema", "properties", "strike_range", "description"])
            .and_then(|value| value.as_str())
            .expect("strike_range description should exist");

        assert!(
            strike.contains("\"0\" for wildcard/bulk strike selection"),
            "strike description should explain MCP wildcard strike semantics: {strike}"
        );
        assert!(
            expiration.contains("\"0\" for wildcard/bulk expiration selection"),
            "expiration description should explain MCP wildcard expiration semantics: {expiration}"
        );
        assert!(
            strike_range.contains("does not expand a pinned strike"),
            "strike_range description should explain wildcard-only filtering semantics: {strike_range}"
        );
    }

    #[test]
    fn optional_i32_args_reject_out_of_range_values() {
        let args = convert_endpoint_args(&sonic_rs::json!({
            "strike_range": i64::from(i32::MAX) + 1
        }))
        .expect("arguments should convert");

        assert!(
            matches!(
                args.optional_int32("strike_range").unwrap_err(),
                EndpointError::InvalidParams(message) if message.contains("out of range for i32")
            ),
            "expected i32 overflow validation error"
        );
    }

    #[test]
    fn optional_args_reject_type_mismatches() {
        let args = convert_endpoint_args(&sonic_rs::json!({
            "exclusive": "false",
            "rate_value": "3.5",
            "venue": 42
        }))
        .expect("arguments should convert");

        assert!(
            matches!(
                args.optional_bool("exclusive").unwrap_err(),
                EndpointError::InvalidParams(message)
                    if message == "optional boolean argument 'exclusive' must be a boolean"
            ),
            "expected boolean type validation error"
        );
        assert!(
            matches!(
                args.optional_float64("rate_value").unwrap_err(),
                EndpointError::InvalidParams(message)
                    if message == "optional number argument 'rate_value' must be a number"
            ),
            "expected number type validation error"
        );
        assert!(
            matches!(
                args.optional_str("venue").unwrap_err(),
                EndpointError::InvalidParams(message)
                    if message == "optional string argument 'venue' must be a string"
            ),
            "expected string type validation error"
        );
    }

    #[test]
    fn optional_date_args_validate_format() {
        let args = convert_endpoint_args(&sonic_rs::json!({
            "start_date": "2026-04-09",
            "end_date": "20260409"
        }))
        .expect("arguments should convert");

        assert!(
            matches!(
                args.optional_date("start_date").unwrap_err(),
                EndpointError::InvalidParams(message)
                    if message
                        == "'start_date' must be exactly 8 digits (YYYYMMDD), got: '2026-04-09'"
            ),
            "expected date format validation error"
        );
        assert_eq!(
            args.optional_date("end_date")
                .expect("end_date should validate"),
            Some("20260409")
        );
    }

    #[test]
    fn serialize_option_history_eod_preserves_bulk_contract_identifiers() {
        let payload = serialize_eod_ticks(&[sample_eod_tick(20230120, 385.0, 67)]);
        let tick = payload
            .get("ticks")
            .and_then(|value: &Value| value.as_array())
            .and_then(|rows| rows.first())
            .expect("serialized tick row should exist");

        assert_eq!(
            tick.get("expiration")
                .and_then(|value: &Value| value.as_i64()),
            Some(20230120)
        );
        assert_eq!(
            tick.get("strike").and_then(|value: &Value| value.as_f64()),
            Some(385.0)
        );
        assert_eq!(
            tick.get("right").and_then(|value: &Value| value.as_str()),
            Some("C")
        );
    }

    #[test]
    fn serialize_eod_ticks_preserves_full_eod_fields() {
        let payload = serialize_eod_ticks(&[sample_eod_tick(0, 0.0, 0)]);
        let tick = payload
            .get("ticks")
            .and_then(|value: &Value| value.as_array())
            .and_then(|rows| rows.first())
            .expect("serialized tick row should exist");

        assert_eq!(
            tick.get("ms_of_day")
                .and_then(|value: &Value| value.as_i64()),
            Some(34_200_000)
        );
        assert_eq!(
            tick.get("ms_of_day2")
                .and_then(|value: &Value| value.as_i64()),
            Some(57_600_000)
        );
        assert_eq!(
            tick.get("bid_exchange")
                .and_then(|value: &Value| value.as_i64()),
            Some(11)
        );
        assert_eq!(
            tick.get("bid_condition")
                .and_then(|value: &Value| value.as_i64()),
            Some(22)
        );
        assert_eq!(
            tick.get("ask_exchange")
                .and_then(|value: &Value| value.as_i64()),
            Some(33)
        );
        assert_eq!(
            tick.get("ask_condition")
                .and_then(|value: &Value| value.as_i64()),
            Some(44)
        );
    }

    #[test]
    fn serialize_option_history_greeks_eod_omits_contract_identifiers_for_single_contract_rows() {
        let payload = serialize_greeks_ticks(&[sample_greeks_tick(0, 0.0, 0)]);
        let tick = payload
            .get("ticks")
            .and_then(|value: &Value| value.as_array())
            .and_then(|rows| rows.first())
            .expect("serialized tick row should exist");

        assert!(
            tick.get("expiration").is_none(),
            "single-contract rows should not emit wildcard-only expiration metadata"
        );
        assert!(
            tick.get("strike").is_none(),
            "single-contract rows should not emit wildcard-only strike metadata"
        );
        assert!(
            tick.get("right").is_none(),
            "single-contract rows should not emit wildcard-only right metadata"
        );
    }

    // ── Serializer field-parity regression tests ──────────────────────
    // These catch future field loss by asserting specific keys exist in
    // the serialized JSON output.

    #[test]
    fn serialize_quote_ticks_includes_condition_and_midpoint_fields() {
        let tick = QuoteTick {
            ms_of_day: 0,
            bid_size: 100,
            bid_exchange: 11,
            bid: 150.0,
            bid_condition: 1,
            ask_size: 200,
            ask_exchange: 12,
            ask: 151.0,
            ask_condition: 2,
            date: 20260410,
            expiration: 0,
            strike: 0.0,
            right: 0,
            midpoint: 150.5,
        };
        let payload = serialize_quote_ticks(&[tick]);
        let row = payload["ticks"].as_array().unwrap().first().unwrap();
        for key in [
            "bid_condition",
            "ask_condition",
            "midpoint",
            "bid_exchange",
            "ask_exchange",
        ] {
            assert!(row.get(key).is_some(), "missing key: {key}");
        }
    }

    #[test]
    fn serialize_trade_quote_ticks_includes_extended_fields() {
        let tick = TradeQuoteTick {
            ms_of_day: 0,
            sequence: 1,
            ext_condition1: 10,
            ext_condition2: 20,
            ext_condition3: 30,
            ext_condition4: 40,
            condition: 1,
            size: 100,
            exchange: 11,
            price: 150.0,
            condition_flags: 0,
            price_flags: 0,
            volume_type: 1,
            records_back: 0,
            quote_ms_of_day: 34_200_000,
            bid_size: 100,
            bid_exchange: 11,
            bid: 149.0,
            bid_condition: 1,
            ask_size: 200,
            ask_exchange: 12,
            ask: 151.0,
            ask_condition: 2,
            date: 20260410,
            expiration: 0,
            strike: 0.0,
            right: 0,
        };
        let payload = serialize_trade_quote_ticks(&[tick]);
        let row = payload["ticks"].as_array().unwrap().first().unwrap();
        for key in [
            "quote_ms_of_day",
            "bid_exchange",
            "ask_exchange",
            "bid_condition",
            "ask_condition",
            "ext_condition1",
            "ext_condition2",
            "ext_condition3",
            "ext_condition4",
            "condition_flags",
            "price_flags",
            "volume_type",
            "records_back",
        ] {
            assert!(row.get(key).is_some(), "missing key: {key}");
        }
    }

    #[test]
    fn serialize_greeks_ticks_includes_all_22_greeks() {
        let tick = sample_greeks_tick(0, 0.0, 0);
        let payload = serialize_greeks_ticks(&[tick]);
        let row = payload["ticks"].as_array().unwrap().first().unwrap();
        for key in [
            "implied_volatility",
            "delta",
            "gamma",
            "theta",
            "vega",
            "rho",
            "iv_error",
            "vanna",
            "charm",
            "vomma",
            "veta",
            "speed",
            "zomma",
            "color",
            "ultima",
            "d1",
            "d2",
            "dual_delta",
            "dual_gamma",
            "epsilon",
            "lambda",
            "vera",
        ] {
            assert!(row.get(key).is_some(), "missing Greek: {key}");
        }
    }

    // -----------------------------------------------------------------------
    //  Issue #459 regression — tools/call must not return a successful but
    //  empty result when the tool output carries a non-finite f64 cell.
    // -----------------------------------------------------------------------

    #[test]
    fn tools_call_response_serialises_nan_cell_as_null_not_empty() {
        // Mimic an `option_snapshot_greeks_*` row that came back with a
        // non-finite vega from the upstream solver. Before the fix, the
        // outer `sonic_rs::to_string(...).unwrap_or_default()` would have
        // collapsed the entire serialise result to an empty string and the
        // MCP client would have seen `result.content[0].text = ""`.
        let mut tool_result = sonic_rs::json!({
            "ticks": [{
                "symbol": "AAPL",
                "delta": 0.5_f64,
                "vega": Value::new_null(),
            }]
        });
        if let Some(ticks) = tool_result.get_mut("ticks").and_then(|v| v.as_array_mut()) {
            if let Some(row) = ticks.first_mut().and_then(|v| v.as_object_mut()) {
                row.insert("vega", json_canon::finite_or_null(f64::NAN));
            }
        }

        let id = sonic_rs::json!(42);
        let resp = build_tool_call_response(id, &mut tool_result);

        // Success path — `result` must be Some, `error` must be None.
        assert!(
            resp.error.is_none(),
            "expected success, got error: {:?}",
            resp.error.as_ref().map(|e| &e.message)
        );
        let result = resp.result.expect("success branch must populate result");

        // Drill into `result.content[0].text` and assert the embedded JSON
        // string is non-empty AND contains the canonicalised null.
        let content = result.get("content").expect("content field");
        let arr = content.as_array().expect("content is an array");
        assert_eq!(arr.len(), 1, "exactly one content block expected");
        let text = arr
            .first()
            .unwrap()
            .get("text")
            .and_then(|v: &Value| v.as_str())
            .expect("text field");
        assert!(
            !text.is_empty(),
            "issue #459: NaN cell must not collapse the tool result text to empty"
        );
        assert!(
            text.contains("\"vega\":null"),
            "vega must canonicalise to null, got {text}"
        );
        assert!(
            text.contains("\"delta\":0.5"),
            "delta must round-trip unchanged, got {text}"
        );
        assert!(
            text.contains("\"symbol\":\"AAPL\""),
            "symbol must round-trip unchanged, got {text}"
        );
    }

    #[test]
    fn tools_call_response_finite_only_round_trips_exact_values() {
        // Sanity check: a finite-only result tree must round-trip every cell
        // byte-for-byte. Object key order is not part of the JSON-RPC
        // contract (sonic_rs's hash table iteration order is the source of
        // truth on the wire), so this test re-parses the emitted text and
        // asserts the resulting tree is `==` to the input — exact value
        // agreement, not a containment check.
        let original = sonic_rs::json!({
            "ticks": [{ "symbol": "AAPL", "delta": 0.5_f64 }]
        });
        let mut tool_result = original.clone();
        let id = sonic_rs::json!("call-1");
        let resp = build_tool_call_response(id, &mut tool_result);
        assert!(resp.error.is_none());
        let text = resp
            .result
            .expect("success branch must populate result")
            .get("content")
            .and_then(|v| v.as_array().and_then(|a| a.first().cloned()))
            .and_then(|v| v.get("text").map(|s| s.as_str().map(str::to_owned)))
            .flatten()
            .expect("text field");
        let reparsed: Value = sonic_rs::from_str(&text).expect("emitted text must reparse");
        assert_eq!(
            reparsed, original,
            "finite-only payload must round-trip with all values preserved"
        );
    }
}
