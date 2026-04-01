//! MCP (Model Context Protocol) server for ThetaDataDx.
//!
//! Gives any MCP-compatible LLM (Claude, Codex, Gemini, Cursor) instant access
//! to ThetaData market data via structured tool calls over stdio JSON-RPC 2.0.
//!
//! Architecture:
//! ```text
//! LLM (Claude/Codex/Gemini)
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

use serde::Serialize;
use sonic_rs::{json, JsonContainerTrait, JsonValueTrait, Value};
use tokio::io::{AsyncBufReadExt, BufReader};

use thetadatadx::registry::{self, ENDPOINTS};
use thetadatadx::{Credentials, DirectConfig, ThetaDataDx};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

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
//  Tool argument validation
// ═══════════════════════════════════════════════════════════════════════════

fn validate_date(value: &str, param_name: &str) -> Result<(), String> {
    if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "'{param_name}' must be exactly 8 digits (YYYYMMDD), got: '{value}'"
        ));
    }
    Ok(())
}

fn validate_symbol(value: &str, param_name: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("'{param_name}' must be non-empty"));
    }
    Ok(())
}

fn validate_interval(value: &str, param_name: &str) -> Result<(), String> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "'{param_name}' must be a non-empty string of digits, got: '{value}'"
        ));
    }
    Ok(())
}

fn validate_right(value: &str, param_name: &str) -> Result<(), String> {
    match value.to_uppercase().as_str() {
        "C" | "P" | "CALL" | "PUT" => Ok(()),
        _ => Err(format!(
            "'{param_name}' must be C, P, call, or put, got: '{value}'"
        )),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tool definitions — generated from endpoint registry + hand-written offline tools
// ═══════════════════════════════════════════════════════════════════════════

fn tool_definitions() -> Vec<Value> {
    let mut tools = Vec::with_capacity(ENDPOINTS.len() + 3);

    // Hand-written: ping
    tools.push(json!({
        "name": "ping",
        "description": "Check MCP server status. Returns uptime and connection info without hitting ThetaData servers.",
        "inputSchema": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));

    // Registry-driven: all 61 DirectClient endpoints
    for ep in ENDPOINTS {
        let mut props = sonic_rs::Object::new();
        let mut required = Vec::new();
        for p in ep.params {
            props.insert(
                &p.name,
                json!({
                    "type": registry::param_type_to_json_type(p.param_type),
                    "description": p.description,
                }),
            );
            if p.required {
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

    // Hand-written: offline Greeks
    tools.push(json!({
        "name": "all_greeks",
        "description": "Compute all 22 Black-Scholes Greeks OFFLINE (no ThetaData server needed). Returns value, delta, gamma, theta, vega, rho, IV, vanna, charm, vomma, veta, speed, zomma, color, ultima, d1, d2, dual_delta, dual_gamma, epsilon, lambda.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "spot": { "type": "number", "description": "Spot price (underlying)" },
                "strike": { "type": "number", "description": "Strike price" },
                "rate": { "type": "number", "description": "Risk-free rate (e.g. 0.05 for 5%)" },
                "dividend_yield": { "type": "number", "description": "Dividend yield (e.g. 0.02 for 2%)" },
                "time_to_expiry": { "type": "number", "description": "Time to expiration in years (e.g. 0.25 for 3 months)" },
                "option_price": { "type": "number", "description": "Market price of the option" },
                "is_call": { "type": "boolean", "description": "true for call, false for put" }
            },
            "required": ["spot", "strike", "rate", "dividend_yield", "time_to_expiry", "option_price", "is_call"]
        }
    }));

    tools.push(json!({
        "name": "implied_volatility",
        "description": "Compute implied volatility OFFLINE using bisection (no ThetaData server needed). Returns IV and error.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "spot": { "type": "number", "description": "Spot price (underlying)" },
                "strike": { "type": "number", "description": "Strike price" },
                "rate": { "type": "number", "description": "Risk-free rate (e.g. 0.05)" },
                "dividend_yield": { "type": "number", "description": "Dividend yield (e.g. 0.02)" },
                "time_to_expiry": { "type": "number", "description": "Time to expiration in years" },
                "option_price": { "type": "number", "description": "Market price of the option" },
                "is_call": { "type": "boolean", "description": "true for call, false for put" }
            },
            "required": ["spot", "strike", "rate", "dividend_yield", "time_to_expiry", "option_price", "is_call"]
        }
    }));

    tools
}

// ═══════════════════════════════════════════════════════════════════════════
//  Serialization helpers
// ═══════════════════════════════════════════════════════════════════════════

#[allow(dead_code)]
fn serialize_data_table(table: &thetadatadx::proto::DataTable) -> Value {
    let headers: Vec<&str> = table.headers.iter().map(|s| s.as_str()).collect();
    let rows: Vec<Value> = table
        .data_table
        .iter()
        .map(|row| {
            let cells: Vec<Value> = row
                .values
                .iter()
                .map(|dv| match &dv.data_type {
                    Some(thetadatadx::proto::data_value::DataType::Number(n)) => json!(n),
                    Some(thetadatadx::proto::data_value::DataType::Price(p)) => {
                        // Use Price::from_proto().to_f64() to avoid duplicating
                        // the scaling logic that already lives in types/price.rs.
                        let price = tdbe::Price::new(p.value, p.r#type);
                        json!(price.to_f64())
                    }
                    Some(thetadatadx::proto::data_value::DataType::Text(s)) => json!(s),
                    Some(thetadatadx::proto::data_value::DataType::NullValue(_)) => {
                        Value::new_null()
                    }
                    Some(thetadatadx::proto::data_value::DataType::Timestamp(ts)) => {
                        json!(format!("{ts:?}"))
                    }
                    None => Value::new_null(),
                })
                .collect();
            cells.into_iter().collect::<Value>()
        })
        .collect();

    json!({
        "headers": headers,
        "rows": rows,
        "row_count": rows.len(),
    })
}

fn serialize_eod_ticks(ticks: &[tdbe::types::tick::EodTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            json!({
                "date": t.date,
                "ms_of_day": t.ms_of_day,
                "open": t.open_price().to_f64(),
                "high": t.high_price().to_f64(),
                "low": t.low_price().to_f64(),
                "close": t.close_price().to_f64(),
                "volume": t.volume,
                "count": t.count,
                "bid": t.bid_price().to_f64(),
                "ask": t.ask_price().to_f64(),
                "bid_size": t.bid_size,
                "ask_size": t.ask_size,
            })
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_ohlc_ticks(ticks: &[tdbe::types::tick::OhlcTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            json!({
                "date": t.date,
                "ms_of_day": t.ms_of_day,
                "open": t.open_price().to_f64(),
                "high": t.high_price().to_f64(),
                "low": t.low_price().to_f64(),
                "close": t.close_price().to_f64(),
                "volume": t.volume,
                "count": t.count,
            })
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_trade_ticks(ticks: &[tdbe::types::tick::TradeTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            json!({
                "date": t.date,
                "ms_of_day": t.ms_of_day,
                "price": t.get_price().to_f64(),
                "size": t.size,
                "exchange": t.exchange,
                "condition": t.condition,
                "sequence": t.sequence,
            })
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_quote_ticks(ticks: &[tdbe::types::tick::QuoteTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            json!({
                "date": t.date,
                "ms_of_day": t.ms_of_day,
                "bid": t.bid_price().to_f64(),
                "bid_size": t.bid_size,
                "bid_exchange": t.bid_exchange,
                "ask": t.ask_price().to_f64(),
                "ask_size": t.ask_size,
                "ask_exchange": t.ask_exchange,
            })
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_trade_quote_ticks(ticks: &[tdbe::types::tick::TradeQuoteTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            json!({
                "date": t.date,
                "ms_of_day": t.ms_of_day,
                "price": t.trade_price().to_f64(),
                "size": t.size,
                "exchange": t.exchange,
                "condition": t.condition,
                "sequence": t.sequence,
                "bid": t.bid_price().to_f64(),
                "bid_size": t.bid_size,
                "ask": t.ask_price().to_f64(),
                "ask_size": t.ask_size,
            })
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_open_interest_ticks(ticks: &[tdbe::types::tick::OpenInterestTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(
            |t| json!({"date": t.date, "ms_of_day": t.ms_of_day, "open_interest": t.open_interest}),
        )
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_market_value_ticks(ticks: &[tdbe::types::tick::MarketValueTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            json!({
                "date": t.date, "ms_of_day": t.ms_of_day,
                "market_cap": t.market_cap, "shares_outstanding": t.shares_outstanding,
                "enterprise_value": t.enterprise_value, "book_value": t.book_value,
                "free_float": t.free_float,
            })
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_greeks_ticks(ticks: &[tdbe::types::tick::GreeksTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            json!({
                "date": t.date, "ms_of_day": t.ms_of_day,
                "implied_volatility": t.implied_volatility, "delta": t.delta,
                "gamma": t.gamma, "theta": t.theta, "vega": t.vega, "rho": t.rho,
                "iv_error": t.iv_error,
            })
        })
        .collect();
    json!({ "ticks": rows, "count": rows.len() })
}

fn serialize_iv_ticks(ticks: &[tdbe::types::tick::IvTick]) -> Value {
    let rows: Vec<Value> = ticks
        .iter()
        .map(|t| {
            json!({
                "date": t.date, "ms_of_day": t.ms_of_day,
                "implied_volatility": t.implied_volatility, "iv_error": t.iv_error,
            })
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
                "price": t.get_price().to_f64(),
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

// ═══════════════════════════════════════════════════════════════════════════
//  Argument extraction helpers
// ═══════════════════════════════════════════════════════════════════════════

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v: &Value| v.as_str())
        .ok_or_else(|| format!("missing required string argument: {key}"))
}

fn arg_f64(args: &Value, key: &str) -> Result<f64, String> {
    args.get(key)
        .and_then(|v: &Value| v.as_f64())
        .ok_or_else(|| format!("missing required number argument: {key}"))
}

fn arg_bool(args: &Value, key: &str) -> Result<bool, String> {
    args.get(key)
        .and_then(|v: &Value| v.as_bool())
        .ok_or_else(|| format!("missing required boolean argument: {key}"))
}

fn arg_date<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    let val = arg_str(args, key)?;
    validate_date(val, key)?;
    Ok(val)
}

fn arg_symbol<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    let val = arg_str(args, key)?;
    validate_symbol(val, key)?;
    Ok(val)
}

fn arg_interval<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    let val = arg_str(args, key)?;
    validate_interval(val, key)?;
    Ok(val)
}

fn arg_right<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    let val = arg_str(args, key)?;
    validate_right(val, key)?;
    Ok(val)
}

fn parse_symbols(s: &str) -> Vec<&str> {
    s.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect()
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

macro_rules! api {
    ($expr:expr) => {
        ($expr).map_err(|e| ToolError::ServerError(sanitize_error(&e.to_string())))?
    };
}

async fn execute_tool(
    client: &Option<ThetaDataDx>,
    name: &str,
    args: &Value,
    start_time: std::time::Instant,
) -> Result<Value, ToolError> {
    // ── Offline tools (no client needed) ────────────────────────────
    match name {
        "ping" => {
            let uptime = start_time.elapsed();
            return Ok(json!({
                "status": "ok",
                "server": "thetadatadx-mcp",
                "version": VERSION,
                "uptime_secs": uptime.as_secs(),
                "connected": client.is_some(),
            }));
        }

        "all_greeks" => {
            let s = param!(arg_f64(args, "spot"));
            let x = param!(arg_f64(args, "strike"));
            let r = param!(arg_f64(args, "rate"));
            let q = param!(arg_f64(args, "dividend_yield"));
            let t = param!(arg_f64(args, "time_to_expiry"));
            let price = param!(arg_f64(args, "option_price"));
            let is_call = param!(arg_bool(args, "is_call"));

            let g = tdbe::greeks::all_greeks(s, x, r, q, t, price, is_call);
            return Ok(json!({
                "value": g.value,
                "iv": g.iv,
                "iv_error": g.iv_error,
                "delta": g.delta,
                "gamma": g.gamma,
                "theta": g.theta,
                "vega": g.vega,
                "rho": g.rho,
                "vanna": g.vanna,
                "charm": g.charm,
                "vomma": g.vomma,
                "veta": g.veta,
                "speed": g.speed,
                "zomma": g.zomma,
                "color": g.color,
                "ultima": g.ultima,
                "d1": g.d1,
                "d2": g.d2,
                "dual_delta": g.dual_delta,
                "dual_gamma": g.dual_gamma,
                "epsilon": g.epsilon,
                "lambda": g.lambda,
            }));
        }

        "implied_volatility" => {
            let s = param!(arg_f64(args, "spot"));
            let x = param!(arg_f64(args, "strike"));
            let r = param!(arg_f64(args, "rate"));
            let q = param!(arg_f64(args, "dividend_yield"));
            let t = param!(arg_f64(args, "time_to_expiry"));
            let price = param!(arg_f64(args, "option_price"));
            let is_call = param!(arg_bool(args, "is_call"));

            let (iv, err) = tdbe::greeks::implied_volatility(s, x, r, q, t, price, is_call);
            return Ok(json!({
                "implied_volatility": iv,
                "error": err,
            }));
        }

        _ => {}
    }

    // ── Online tools (require connected client) ─────────────────────
    let client = client.as_ref().ok_or_else(|| {
        ToolError::ServerError(
            "ThetaData client not connected. Set THETA_EMAIL + THETA_PASSWORD env vars or use --creds flag.".to_string(),
        )
    })?;

    match name {
        // ── Stock List ──────────────────────────────────────────────
        "stock_list_symbols" => {
            let symbols = api!(client.stock_list_symbols().await);
            Ok(json!({ "symbols": symbols, "count": symbols.len() }))
        }
        "stock_list_dates" => {
            let rt = param!(arg_str(args, "request_type"));
            let sym = param!(arg_symbol(args, "symbol"));
            let dates = api!(client.stock_list_dates(rt, sym).await);
            Ok(json!({ "dates": dates, "count": dates.len() }))
        }

        // ── Stock Snapshot ──────────────────────────────────────────
        "stock_snapshot_ohlc" => {
            let syms_str = param!(arg_symbol(args, "symbols"));
            let syms = parse_symbols(syms_str);
            let ticks = api!(client.stock_snapshot_ohlc(&syms).await);
            Ok(serialize_ohlc_ticks(&ticks))
        }
        "stock_snapshot_trade" => {
            let syms_str = param!(arg_symbol(args, "symbols"));
            let syms = parse_symbols(syms_str);
            let ticks = api!(client.stock_snapshot_trade(&syms).await);
            Ok(serialize_trade_ticks(&ticks))
        }
        "stock_snapshot_quote" => {
            let syms_str = param!(arg_symbol(args, "symbols"));
            let syms = parse_symbols(syms_str);
            let ticks = api!(client.stock_snapshot_quote(&syms).await);
            Ok(serialize_quote_ticks(&ticks))
        }
        "stock_snapshot_market_value" => {
            let syms_str = param!(arg_symbol(args, "symbols"));
            let syms = parse_symbols(syms_str);
            let ticks = api!(client.stock_snapshot_market_value(&syms).await);
            Ok(serialize_market_value_ticks(&ticks))
        }

        // ── Stock History ───────────────────────────────────────────
        "stock_history_eod" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let ticks = api!(client.stock_history_eod(sym, start, end).await);
            Ok(serialize_eod_ticks(&ticks))
        }
        "stock_history_ohlc" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let date = param!(arg_date(args, "date"));
            let interval = param!(arg_interval(args, "interval"));
            let ticks = api!(client.stock_history_ohlc(sym, date, interval).await);
            Ok(serialize_ohlc_ticks(&ticks))
        }
        "stock_history_ohlc_range" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let interval = param!(arg_interval(args, "interval"));
            let ticks = api!(
                client
                    .stock_history_ohlc_range(sym, start, end, interval)
                    .await
            );
            Ok(serialize_ohlc_ticks(&ticks))
        }
        "stock_history_trade" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let date = param!(arg_date(args, "date"));
            let ticks = api!(client.stock_history_trade(sym, date).await);
            Ok(serialize_trade_ticks(&ticks))
        }
        "stock_history_quote" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let date = param!(arg_date(args, "date"));
            let interval = param!(arg_interval(args, "interval"));
            let ticks = api!(client.stock_history_quote(sym, date, interval).await);
            Ok(serialize_quote_ticks(&ticks))
        }
        "stock_history_trade_quote" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let date = param!(arg_date(args, "date"));
            let ticks = api!(client.stock_history_trade_quote(sym, date).await);
            Ok(serialize_trade_quote_ticks(&ticks))
        }

        // ── Stock At-Time ───────────────────────────────────────────
        "stock_at_time_trade" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let tod = param!(arg_str(args, "time_of_day"));
            let ticks = api!(client.stock_at_time_trade(sym, start, end, tod).await);
            Ok(serialize_trade_ticks(&ticks))
        }
        "stock_at_time_quote" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let tod = param!(arg_str(args, "time_of_day"));
            let ticks = api!(client.stock_at_time_quote(sym, start, end, tod).await);
            Ok(serialize_quote_ticks(&ticks))
        }

        // ── Option List ─────────────────────────────────────────────
        "option_list_symbols" => {
            let symbols = api!(client.option_list_symbols().await);
            Ok(json!({ "symbols": symbols, "count": symbols.len() }))
        }
        "option_list_dates" => {
            let rt = param!(arg_str(args, "request_type"));
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let dates = api!(client.option_list_dates(rt, sym, exp, strike, right).await);
            Ok(json!({ "dates": dates, "count": dates.len() }))
        }
        "option_list_expirations" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exps = api!(client.option_list_expirations(sym).await);
            Ok(json!({ "expirations": exps, "count": exps.len() }))
        }
        "option_list_strikes" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strikes = api!(client.option_list_strikes(sym, exp).await);
            Ok(json!({ "strikes": strikes, "count": strikes.len() }))
        }
        "option_list_contracts" => {
            let rt = param!(arg_str(args, "request_type"));
            let sym = param!(arg_symbol(args, "symbol"));
            let date = param!(arg_date(args, "date"));
            let ticks = api!(client.option_list_contracts(rt, sym, date).await);
            Ok(serialize_option_contracts(&ticks))
        }

        // ── Option Snapshot ─────────────────────────────────────────
        "option_snapshot_ohlc"
        | "option_snapshot_trade"
        | "option_snapshot_quote"
        | "option_snapshot_open_interest"
        | "option_snapshot_market_value"
        | "option_snapshot_greeks_implied_volatility"
        | "option_snapshot_greeks_all"
        | "option_snapshot_greeks_first_order"
        | "option_snapshot_greeks_second_order"
        | "option_snapshot_greeks_third_order" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            match name {
                "option_snapshot_ohlc" => {
                    let ticks = api!(client.option_snapshot_ohlc(sym, exp, strike, right).await);
                    Ok(serialize_ohlc_ticks(&ticks))
                }
                "option_snapshot_trade" => {
                    let ticks = api!(client.option_snapshot_trade(sym, exp, strike, right).await);
                    Ok(serialize_trade_ticks(&ticks))
                }
                "option_snapshot_quote" => {
                    let ticks = api!(client.option_snapshot_quote(sym, exp, strike, right).await);
                    Ok(serialize_quote_ticks(&ticks))
                }
                "option_snapshot_open_interest" => {
                    let ticks = api!(
                        client
                            .option_snapshot_open_interest(sym, exp, strike, right)
                            .await
                    );
                    Ok(serialize_open_interest_ticks(&ticks))
                }
                "option_snapshot_market_value" => {
                    let ticks = api!(
                        client
                            .option_snapshot_market_value(sym, exp, strike, right)
                            .await
                    );
                    Ok(serialize_market_value_ticks(&ticks))
                }
                "option_snapshot_greeks_implied_volatility" => {
                    let ticks = api!(
                        client
                            .option_snapshot_greeks_implied_volatility(sym, exp, strike, right)
                            .await
                    );
                    Ok(serialize_iv_ticks(&ticks))
                }
                "option_snapshot_greeks_all" => {
                    let ticks = api!(
                        client
                            .option_snapshot_greeks_all(sym, exp, strike, right)
                            .await
                    );
                    Ok(serialize_greeks_ticks(&ticks))
                }
                "option_snapshot_greeks_first_order" => {
                    let ticks = api!(
                        client
                            .option_snapshot_greeks_first_order(sym, exp, strike, right)
                            .await
                    );
                    Ok(serialize_greeks_ticks(&ticks))
                }
                "option_snapshot_greeks_second_order" => {
                    let ticks = api!(
                        client
                            .option_snapshot_greeks_second_order(sym, exp, strike, right)
                            .await
                    );
                    Ok(serialize_greeks_ticks(&ticks))
                }
                "option_snapshot_greeks_third_order" => {
                    let ticks = api!(
                        client
                            .option_snapshot_greeks_third_order(sym, exp, strike, right)
                            .await
                    );
                    Ok(serialize_greeks_ticks(&ticks))
                }
                _ => unreachable!(),
            }
        }

        // ── Option History ──────────────────────────────────────────
        "option_history_eod" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let ticks = api!(
                client
                    .option_history_eod(sym, exp, strike, right, start, end)
                    .await
            );
            Ok(serialize_eod_ticks(&ticks))
        }
        "option_history_ohlc" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let date = param!(arg_date(args, "date"));
            let interval = param!(arg_interval(args, "interval"));
            let ticks = api!(
                client
                    .option_history_ohlc(sym, exp, strike, right, date, interval)
                    .await
            );
            Ok(serialize_ohlc_ticks(&ticks))
        }
        "option_history_trade" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let date = param!(arg_date(args, "date"));
            let ticks = api!(
                client
                    .option_history_trade(sym, exp, strike, right, date)
                    .await
            );
            Ok(serialize_trade_ticks(&ticks))
        }
        "option_history_quote" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let date = param!(arg_date(args, "date"));
            let interval = param!(arg_interval(args, "interval"));
            let ticks = api!(
                client
                    .option_history_quote(sym, exp, strike, right, date, interval)
                    .await
            );
            Ok(serialize_quote_ticks(&ticks))
        }
        "option_history_trade_quote" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let date = param!(arg_date(args, "date"));
            let ticks = api!(
                client
                    .option_history_trade_quote(sym, exp, strike, right, date)
                    .await
            );
            Ok(serialize_trade_quote_ticks(&ticks))
        }
        "option_history_open_interest" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let date = param!(arg_date(args, "date"));
            let ticks = api!(
                client
                    .option_history_open_interest(sym, exp, strike, right, date)
                    .await
            );
            Ok(serialize_open_interest_ticks(&ticks))
        }

        // ── Option History Greeks ───────────────────────────────────
        "option_history_greeks_eod" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let ticks = api!(
                client
                    .option_history_greeks_eod(sym, exp, strike, right, start, end)
                    .await
            );
            Ok(serialize_greeks_ticks(&ticks))
        }
        // Greeks with interval (4 endpoints)
        "option_history_greeks_all"
        | "option_history_greeks_first_order"
        | "option_history_greeks_second_order"
        | "option_history_greeks_third_order" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let date = param!(arg_date(args, "date"));
            let interval = param!(arg_interval(args, "interval"));
            let ticks = match name {
                "option_history_greeks_all" => api!(
                    client
                        .option_history_greeks_all(sym, exp, strike, right, date, interval)
                        .await
                ),
                "option_history_greeks_first_order" => api!(
                    client
                        .option_history_greeks_first_order(sym, exp, strike, right, date, interval)
                        .await
                ),
                "option_history_greeks_second_order" => api!(
                    client
                        .option_history_greeks_second_order(
                            sym, exp, strike, right, date, interval,
                        )
                        .await
                ),
                "option_history_greeks_third_order" => api!(
                    client
                        .option_history_greeks_third_order(sym, exp, strike, right, date, interval)
                        .await
                ),
                _ => unreachable!(),
            };
            Ok(serialize_greeks_ticks(&ticks))
        }
        "option_history_greeks_implied_volatility" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let date = param!(arg_date(args, "date"));
            let interval = param!(arg_interval(args, "interval"));
            let ticks = api!(
                client
                    .option_history_greeks_implied_volatility(
                        sym, exp, strike, right, date, interval,
                    )
                    .await
            );
            Ok(serialize_iv_ticks(&ticks))
        }
        // Trade Greeks (4 endpoints, no interval)
        "option_history_trade_greeks_all"
        | "option_history_trade_greeks_first_order"
        | "option_history_trade_greeks_second_order"
        | "option_history_trade_greeks_third_order" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let date = param!(arg_date(args, "date"));
            let ticks = match name {
                "option_history_trade_greeks_all" => api!(
                    client
                        .option_history_trade_greeks_all(sym, exp, strike, right, date)
                        .await
                ),
                "option_history_trade_greeks_first_order" => api!(
                    client
                        .option_history_trade_greeks_first_order(sym, exp, strike, right, date)
                        .await
                ),
                "option_history_trade_greeks_second_order" => api!(
                    client
                        .option_history_trade_greeks_second_order(sym, exp, strike, right, date)
                        .await
                ),
                "option_history_trade_greeks_third_order" => api!(
                    client
                        .option_history_trade_greeks_third_order(sym, exp, strike, right, date)
                        .await
                ),
                _ => unreachable!(),
            };
            Ok(serialize_greeks_ticks(&ticks))
        }
        "option_history_trade_greeks_implied_volatility" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let date = param!(arg_date(args, "date"));
            let ticks = api!(
                client
                    .option_history_trade_greeks_implied_volatility(sym, exp, strike, right, date)
                    .await
            );
            Ok(serialize_iv_ticks(&ticks))
        }

        // ── Option At-Time ──────────────────────────────────────────
        "option_at_time_trade" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let tod = param!(arg_str(args, "time_of_day"));
            let ticks = api!(
                client
                    .option_at_time_trade(sym, exp, strike, right, start, end, tod)
                    .await
            );
            Ok(serialize_trade_ticks(&ticks))
        }
        "option_at_time_quote" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let exp = param!(arg_date(args, "expiration"));
            let strike = param!(arg_str(args, "strike"));
            let right = param!(arg_right(args, "right"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let tod = param!(arg_str(args, "time_of_day"));
            let ticks = api!(
                client
                    .option_at_time_quote(sym, exp, strike, right, start, end, tod)
                    .await
            );
            Ok(serialize_quote_ticks(&ticks))
        }

        // ── Index List ──────────────────────────────────────────────
        "index_list_symbols" => {
            let symbols = api!(client.index_list_symbols().await);
            Ok(json!({ "symbols": symbols, "count": symbols.len() }))
        }
        "index_list_dates" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let dates = api!(client.index_list_dates(sym).await);
            Ok(json!({ "dates": dates, "count": dates.len() }))
        }

        // ── Index Snapshot ──────────────────────────────────────────
        "index_snapshot_ohlc" => {
            let syms_str = param!(arg_symbol(args, "symbols"));
            let syms = parse_symbols(syms_str);
            let ticks = api!(client.index_snapshot_ohlc(&syms).await);
            Ok(serialize_ohlc_ticks(&ticks))
        }
        "index_snapshot_price" => {
            let syms_str = param!(arg_symbol(args, "symbols"));
            let syms = parse_symbols(syms_str);
            let ticks = api!(client.index_snapshot_price(&syms).await);
            Ok(serialize_price_ticks(&ticks))
        }
        "index_snapshot_market_value" => {
            let syms_str = param!(arg_symbol(args, "symbols"));
            let syms = parse_symbols(syms_str);
            let ticks = api!(client.index_snapshot_market_value(&syms).await);
            Ok(serialize_market_value_ticks(&ticks))
        }

        // ── Index History ───────────────────────────────────────────
        "index_history_eod" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let ticks = api!(client.index_history_eod(sym, start, end).await);
            Ok(serialize_eod_ticks(&ticks))
        }
        "index_history_ohlc" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let interval = param!(arg_interval(args, "interval"));
            let ticks = api!(client.index_history_ohlc(sym, start, end, interval).await);
            Ok(serialize_ohlc_ticks(&ticks))
        }
        "index_history_price" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let date = param!(arg_date(args, "date"));
            let interval = param!(arg_interval(args, "interval"));
            let ticks = api!(client.index_history_price(sym, date, interval).await);
            Ok(serialize_price_ticks(&ticks))
        }

        // ── Index At-Time ───────────────────────────────────────────
        "index_at_time_price" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let tod = param!(arg_str(args, "time_of_day"));
            let ticks = api!(client.index_at_time_price(sym, start, end, tod).await);
            Ok(serialize_price_ticks(&ticks))
        }

        // ── Calendar ────────────────────────────────────────────────
        "calendar_open_today" => {
            let ticks = api!(client.calendar_open_today().await);
            Ok(serialize_calendar_days(&ticks))
        }
        "calendar_on_date" => {
            let date = param!(arg_date(args, "date"));
            let ticks = api!(client.calendar_on_date(date).await);
            Ok(serialize_calendar_days(&ticks))
        }
        "calendar_year" => {
            let year = param!(arg_str(args, "year"));
            let ticks = api!(client.calendar_year(year).await);
            Ok(serialize_calendar_days(&ticks))
        }

        // ── Interest Rate ───────────────────────────────────────────
        "interest_rate_history_eod" => {
            let sym = param!(arg_symbol(args, "symbol"));
            let start = param!(arg_date(args, "start_date"));
            let end = param!(arg_date(args, "end_date"));
            let ticks = api!(client.interest_rate_history_eod(sym, start, end).await);
            Ok(serialize_interest_rate_ticks(&ticks))
        }

        _ => Err(ToolError::InvalidParams(format!("unknown tool: {name}"))),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Request handling
// ═══════════════════════════════════════════════════════════════════════════

async fn handle_request(
    req: &JsonRpcRequest,
    client: &Option<ThetaDataDx>,
    start_time: std::time::Instant,
) -> JsonRpcResponse {
    let id = req.id.clone().unwrap_or(Value::new_null());

    match req.method.as_str() {
        "initialize" => {
            let client_version = req
                .params
                .get("protocolVersion")
                .and_then(|v: &Value| v.as_str())
                .unwrap_or("");

            if !client_version.is_empty() && client_version != PROTOCOL_VERSION {
                return JsonRpcResponse::error(
                    id,
                    -32602,
                    format!(
                        "Unsupported protocol version '{}'. This server implements '{}'.",
                        client_version, PROTOCOL_VERSION
                    ),
                );
            }

            JsonRpcResponse::success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
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
                Ok(result) => {
                    let text = sonic_rs::to_string(&result).unwrap_or_default();
                    JsonRpcResponse::success(
                        id,
                        json!({
                            "content": [{
                                "type": "text",
                                "text": text,
                            }]
                        }),
                    )
                }
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

    // ── Connect to ThetaData (if credentials available) ─────────────
    let client: Option<ThetaDataDx> = if let Some(creds) = creds {
        match ThetaDataDx::connect(&creds, DirectConfig::production()).await {
            Ok(c) => {
                tracing::info!("connected to ThetaData MDDS");
                Some(c)
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to connect to ThetaData, starting in offline mode");
                None
            }
        }
    } else {
        None
    };

    // ── Main JSON-RPC loop over stdin ───────────────────────────────
    tracing::info!(
        version = VERSION,
        connected = client.is_some(),
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
