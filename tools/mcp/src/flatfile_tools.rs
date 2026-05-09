// Hand-written FLATFILES tool surface for the MCP server (issue #431).
//
// Mirrors the Python / TypeScript flatfile API. Tools return the
// on-disk path of the written CSV / JSONL blob — MCP transports
// JSON-RPC, not raw bytes, and whole-universe flat files routinely
// exceed 100 MB. Returning a path is the same convention the existing
// MCP utility tools use for any file-producing operation.
//
// Tool naming (matches the Rust `flatfile_*` helper methods on
// `ThetaDataDxClient`):
//
//   tdx_flatfile_request                     (generic)
//   tdx_flatfile_option_quote
//   tdx_flatfile_option_trade
//   tdx_flatfile_option_trade_quote
//   tdx_flatfile_option_ohlc
//   tdx_flatfile_option_open_interest
//   tdx_flatfile_option_eod
//   tdx_flatfile_stock_quote
//   tdx_flatfile_stock_trade
//   tdx_flatfile_stock_trade_quote
//   tdx_flatfile_stock_eod
//
// All ten convenience tools take `(date, output_path?, format?)`. The
// generic tool takes `(sec_type, req_type, date, output_path, format)`
// with case-insensitive enum strings. A missing `output_path` writes
// to a deterministic temp path and surfaces it in the response.

use sonic_rs::{json, JsonValueTrait, Value};
use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};
use thetadatadx::ThetaDataDxClient;

use crate::ToolError;

/// Append flatfile tool definitions to the `tools/list` array.
pub(crate) fn push_flatfile_tool_definitions(tools: &mut Vec<Value>) {
    let format_prop = json!({
        "type": "string",
        "description": "On-disk format. csv = vendor byte-format CSV, jsonl = JSON Lines.",
        "enum": ["csv", "jsonl"]
    });
    let date_prop = json!({
        "type": "string",
        "description": "Trading date in YYYYMMDD form, e.g. \"20260428\"."
    });
    let output_prop = json!({
        "type": "string",
        "description": "Output file path. If omitted, the tool writes to a deterministic temp path \
                        and returns it in the response."
    });

    let convenience = [
        (
            "tdx_flatfile_option_quote",
            "Whole-universe option-quote flat file for a single date. Returns the written file path.",
        ),
        (
            "tdx_flatfile_option_trade",
            "Whole-universe option-trade flat file for a single date. Returns the written file path.",
        ),
        (
            "tdx_flatfile_option_trade_quote",
            "Whole-universe option trade-quote flat file for a single date. Returns the written file path.",
        ),
        (
            "tdx_flatfile_option_ohlc",
            "Whole-universe option-OHLC flat file for a single date. Returns the written file path.",
        ),
        (
            "tdx_flatfile_option_open_interest",
            "Whole-universe option open-interest flat file for a single date. Returns the written file path.",
        ),
        (
            "tdx_flatfile_option_eod",
            "Whole-universe option end-of-day flat file for a single date. Returns the written file path.",
        ),
        (
            "tdx_flatfile_stock_quote",
            "Whole-universe stock-quote flat file for a single date. Returns the written file path.",
        ),
        (
            "tdx_flatfile_stock_trade",
            "Whole-universe stock-trade flat file for a single date. Returns the written file path.",
        ),
        (
            "tdx_flatfile_stock_trade_quote",
            "Whole-universe stock trade-quote flat file for a single date. Returns the written file path.",
        ),
        (
            "tdx_flatfile_stock_eod",
            "Whole-universe stock end-of-day flat file for a single date. Returns the written file path.",
        ),
    ];

    for (name, description) in convenience {
        tools.push(json!({
            "name": name,
            "description": description,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "date": date_prop,
                    "output_path": output_prop,
                    "format": format_prop,
                },
                "required": ["date"]
            }
        }));
    }

    tools.push(json!({
        "name": "tdx_flatfile_request",
        "description": "Generic flat-file request. Pull a whole-universe daily blob for any \
                        (sec_type, req_type) combination supported by ThetaData. Returns the \
                        written file path.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "sec_type": {
                    "type": "string",
                    "description": "Security type.",
                    "enum": ["OPTION", "STOCK", "INDEX", "option", "stock", "index"]
                },
                "req_type": {
                    "type": "string",
                    "description": "Request type.",
                    "enum": [
                        "EOD", "QUOTE", "OPEN_INTEREST", "OHLC", "TRADE", "TRADE_QUOTE",
                        "eod", "quote", "open_interest", "ohlc", "trade", "trade_quote"
                    ]
                },
                "date": date_prop,
                "output_path": output_prop,
                "format": format_prop,
            },
            "required": ["sec_type", "req_type", "date"]
        }
    }));
}

fn parse_sec_type(s: &str) -> Result<SecType, String> {
    match s.to_ascii_uppercase().as_str() {
        "OPTION" => Ok(SecType::Option),
        "STOCK" => Ok(SecType::Stock),
        "INDEX" => Ok(SecType::Index),
        other => Err(format!("unknown sec_type: {other}")),
    }
}

fn parse_req_type(s: &str) -> Result<ReqType, String> {
    match s.to_ascii_uppercase().as_str() {
        "EOD" => Ok(ReqType::Eod),
        "QUOTE" => Ok(ReqType::Quote),
        "OPEN_INTEREST" | "OPENINTEREST" => Ok(ReqType::OpenInterest),
        "OHLC" => Ok(ReqType::Ohlc),
        "TRADE" => Ok(ReqType::Trade),
        "TRADE_QUOTE" | "TRADEQUOTE" => Ok(ReqType::TradeQuote),
        other => Err(format!("unknown req_type: {other}")),
    }
}

fn parse_format(value: Option<&str>) -> Result<FlatFileFormat, String> {
    match value.unwrap_or("csv").to_ascii_lowercase().as_str() {
        "csv" => Ok(FlatFileFormat::Csv),
        "jsonl" | "json" => Ok(FlatFileFormat::Jsonl),
        other => Err(format!(
            "unknown flat-file format: {other:?} (expected csv or jsonl)"
        )),
    }
}

fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(sonic_rs::JsonValueTrait::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| format!("missing required string argument: {key}"))
}

fn arg_str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(sonic_rs::JsonValueTrait::as_str)
        .map(ToString::to_string)
}

/// Map a tool-name suffix to a `(SecType, ReqType)` pair, e.g.
/// `"tdx_flatfile_option_quote"` -> `Some((Option, Quote))`.
fn convenience_pair(tool_name: &str) -> Option<(SecType, ReqType)> {
    match tool_name {
        "tdx_flatfile_option_quote" => Some((SecType::Option, ReqType::Quote)),
        "tdx_flatfile_option_trade" => Some((SecType::Option, ReqType::Trade)),
        "tdx_flatfile_option_trade_quote" => Some((SecType::Option, ReqType::TradeQuote)),
        "tdx_flatfile_option_ohlc" => Some((SecType::Option, ReqType::Ohlc)),
        "tdx_flatfile_option_open_interest" => Some((SecType::Option, ReqType::OpenInterest)),
        "tdx_flatfile_option_eod" => Some((SecType::Option, ReqType::Eod)),
        "tdx_flatfile_stock_quote" => Some((SecType::Stock, ReqType::Quote)),
        "tdx_flatfile_stock_trade" => Some((SecType::Stock, ReqType::Trade)),
        "tdx_flatfile_stock_trade_quote" => Some((SecType::Stock, ReqType::TradeQuote)),
        "tdx_flatfile_stock_eod" => Some((SecType::Stock, ReqType::Eod)),
        _ => None,
    }
}

/// Try to execute a flatfile tool. Returns `Some(result)` if `name`
/// matches a flatfile tool; `None` otherwise (caller falls through
/// to the registry-driven dispatch).
pub(crate) async fn try_execute_flatfile_tool(
    client: Option<&ThetaDataDxClient>,
    name: &str,
    args: &Value,
) -> Option<Result<Value, ToolError>> {
    let (sec_type, req_type) = if name == "tdx_flatfile_request" {
        let sec_str = match arg_str(args, "sec_type") {
            Ok(s) => s,
            Err(e) => return Some(Err(ToolError::InvalidParams(e))),
        };
        let req_str = match arg_str(args, "req_type") {
            Ok(s) => s,
            Err(e) => return Some(Err(ToolError::InvalidParams(e))),
        };
        let sec = match parse_sec_type(&sec_str) {
            Ok(v) => v,
            Err(e) => return Some(Err(ToolError::InvalidParams(e))),
        };
        let req = match parse_req_type(&req_str) {
            Ok(v) => v,
            Err(e) => return Some(Err(ToolError::InvalidParams(e))),
        };
        (sec, req)
    } else {
        convenience_pair(name)?
    };

    let date = match arg_str(args, "date") {
        Ok(d) => d,
        Err(e) => return Some(Err(ToolError::InvalidParams(e))),
    };
    let format_str = arg_str_opt(args, "format");
    let format = match parse_format(format_str.as_deref()) {
        Ok(f) => f,
        Err(e) => return Some(Err(ToolError::InvalidParams(e))),
    };
    let output_path = arg_str_opt(args, "output_path").map_or_else(
        || {
            std::env::temp_dir().join(format!(
                "tdx_flatfile_{sec_type}_{}_{date}.{}",
                req_type as u32,
                format.extension(),
            ))
        },
        std::path::PathBuf::from,
    );

    let client = match client {
        Some(c) => c,
        None => {
            return Some(Err(ToolError::ServerError(
                "ThetaData client not connected. Set THETA_EMAIL + THETA_PASSWORD env vars or use \
                 --creds flag."
                    .to_string(),
            )));
        }
    };

    let result = client
        .flatfile_request(sec_type, req_type, &date, &output_path, format)
        .await;

    Some(match result {
        Ok(written) => Ok(json!({
            "status": "ok",
            "path": written.to_string_lossy(),
            "sec_type": sec_type.to_string(),
            "req_type": format!("{:?}", req_type),
            "format": format.extension(),
            "date": date,
        })),
        Err(e) => Err(ToolError::ServerError(crate::sanitize_error(&e.to_string()))),
    })
}
