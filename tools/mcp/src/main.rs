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
use std::sync::Arc;

use serde::Serialize;
use sonic_rs::{json, JsonContainerTrait, JsonValueMutTrait, JsonValueTrait, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::RwLock;

use thetadatadx::endpoint::{self, EndpointArgValue, EndpointArgs, EndpointError, EndpointOutput};
use thetadatadx::registry::{self, ENDPOINTS};
use thetadatadx::{Credentials, DirectConfig, ThetaDataDx};

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
                "open": t.open,
                "high": t.high,
                "low": t.low,
                "close": t.close,
                "volume": t.volume,
                "count": t.count,
                "bid": t.bid,
                "ask": t.ask,
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
                "ask": t.ask,
                "ask_size": t.ask_size,
                "ask_exchange": t.ask_exchange,
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
                "bid": t.bid,
                "bid_size": t.bid_size,
                "ask": t.ask,
                "ask_size": t.ask_size,
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
                "market_cap": t.market_cap, "shares_outstanding": t.shares_outstanding,
                "enterprise_value": t.enterprise_value, "book_value": t.book_value,
                "free_float": t.free_float,
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
                "implied_volatility": t.implied_volatility, "delta": t.delta,
                "gamma": t.gamma, "theta": t.theta, "vega": t.vega, "rho": t.rho,
                "iv_error": t.iv_error,
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

fn arg_bool(args: &Value, key: &str) -> Result<bool, String> {
    args.get(key)
        .and_then(|v: &Value| v.as_bool())
        .ok_or_else(|| format!("missing required boolean argument: {key}"))
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
        // (client is already dereferenced above via the RwLock read guard)

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
    client: &Arc<RwLock<Option<ThetaDataDx>>>,
    start_time: std::time::Instant,
) -> JsonRpcResponse {
    let client = client.read().await;
    let client = &*client;
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

    // ── Connect to ThetaData in the background ──────────────────────
    // We must NOT block here: MCP clients (e.g. Claude Code) send `initialize`
    // immediately after spawning and time out waiting for a response if we block
    // on the ThetaData gRPC handshake (~800 ms).  Wrap the client in an
    // Arc<RwLock> so the background task can populate it while the stdin loop
    // is already running.
    let client: Arc<RwLock<Option<ThetaDataDx>>> = Arc::new(RwLock::new(None));

    if let Some(creds) = creds {
        let client_bg = Arc::clone(&client);
        tokio::spawn(async move {
            match ThetaDataDx::connect(&creds, DirectConfig::production()).await {
                Ok(c) => {
                    tracing::info!("connected to ThetaData MDDS");
                    *client_bg.write().await = Some(c);
                }
                Err(e) => {
                    tracing::error!(error = %e, "failed to connect to ThetaData, running in offline mode");
                }
            }
        });
    }

    // ── Main JSON-RPC loop over stdin ───────────────────────────────
    tracing::info!(version = VERSION, "thetadatadx-mcp ready, reading JSON-RPC from stdin");

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
    use tdbe::types::tick::{EodTick, GreeksTick};

    fn sample_eod_tick(expiration: i32, strike: f64, right: i32) -> EodTick {
        EodTick {
            ms_of_day: 0,
            ms_of_day2: 0,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 10,
            count: 1,
            bid_size: 2,
            bid_exchange: 0,
            bid: 0.9,
            bid_condition: 0,
            ask_size: 3,
            ask_exchange: 0,
            ask: 1.1,
            ask_condition: 0,
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
        assert_eq!(
            negotiate_protocol_version(Some("2025-11-25")),
            "2025-11-25"
        );
        assert_eq!(
            negotiate_protocol_version(Some("2024-11-05")),
            "2024-11-05"
        );
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
            args.optional_date("end_date").expect("end_date should validate"),
            Some("20260409")
        );
    }

    #[test]
    fn serialize_option_history_eod_preserves_bulk_contract_identifiers() {
        let payload = serialize_eod_ticks(&[sample_eod_tick(20230120, 385.0, 67)]);
        let tick = payload
            .get("ticks")
            .and_then(|value: &Value| value.as_array())
            .and_then(|rows| rows.get(0))
            .expect("serialized tick row should exist");

        assert_eq!(
            tick.get("expiration").and_then(|value: &Value| value.as_i64()),
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
    fn serialize_option_history_greeks_eod_omits_contract_identifiers_for_single_contract_rows() {
        let payload = serialize_greeks_ticks(&[sample_greeks_tick(0, 0.0, 0)]);
        let tick = payload
            .get("ticks")
            .and_then(|value: &Value| value.as_array())
            .and_then(|rows| rows.get(0))
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
}
