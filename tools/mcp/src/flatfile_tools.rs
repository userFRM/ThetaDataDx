//! Hand-written flat-file tool surface for the MCP server.
//!
//! Mirrors the Python / TypeScript flatfile API. Tools return the
//! on-disk path of the written CSV / JSONL blob — MCP transports
//! JSON-RPC, not raw bytes, and whole-universe flat files are large.
//! Returning a path is the same convention the other MCP utility tools
//! use for any file-producing operation.
//!
//! Tool naming (matches the Rust `flatfile_*` helper methods on
//! `Client`):
//!
//!   thetadatadx_flatfile_request                     (generic)
//!   thetadatadx_flatfile_option_trade_quote
//!   thetadatadx_flatfile_option_open_interest
//!   thetadatadx_flatfile_option_eod
//!   thetadatadx_flatfile_stock_trade_quote
//!   thetadatadx_flatfile_stock_eod
//!
//! The convenience tools cover exactly the datasets the flat-file
//! distribution serves, each taking `(date, output_path?, format?)`. The
//! generic tool takes `(sec_type, req_type, date, output_path, format)`
//! with case-insensitive enum strings; an unserved `(sec_type, req_type)`
//! pair surfaces a typed invalid-parameter error before any network
//! round-trip. A missing `output_path` writes to a deterministic temp
//! path and surfaces it in the response.

use sonic_rs::{json, JsonValueTrait, Value};
use thetadatadx::flatfiles::{FlatFileFormat, ReqType, SecType};
use thetadatadx::Client;

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
            "thetadatadx_flatfile_option_trade_quote",
            "Whole-universe option trade-quote flat file for a single date. Returns the written file path.",
        ),
        (
            "thetadatadx_flatfile_option_open_interest",
            "Whole-universe option open-interest flat file for a single date. Returns the written file path.",
        ),
        (
            "thetadatadx_flatfile_option_eod",
            "Whole-universe option end-of-day flat file for a single date. Returns the written file path.",
        ),
        (
            "thetadatadx_flatfile_stock_trade_quote",
            "Whole-universe stock trade-quote flat file for a single date. Returns the written file path.",
        ),
        (
            "thetadatadx_flatfile_stock_eod",
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
        "name": "thetadatadx_flatfile_request",
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
/// `"thetadatadx_flatfile_option_eod"` -> `Some((Option, Eod))`. Only
/// the datasets the flat-file distribution serves have a convenience
/// tool; every other request type is reachable via the historical
/// endpoints, not as a flat file.
fn convenience_pair(tool_name: &str) -> Option<(SecType, ReqType)> {
    match tool_name {
        "thetadatadx_flatfile_option_trade_quote" => Some((SecType::Option, ReqType::TradeQuote)),
        "thetadatadx_flatfile_option_open_interest" => {
            Some((SecType::Option, ReqType::OpenInterest))
        }
        "thetadatadx_flatfile_option_eod" => Some((SecType::Option, ReqType::Eod)),
        "thetadatadx_flatfile_stock_trade_quote" => Some((SecType::Stock, ReqType::TradeQuote)),
        "thetadatadx_flatfile_stock_eod" => Some((SecType::Stock, ReqType::Eod)),
        _ => None,
    }
}

/// Try to execute a flatfile tool. Returns `Some(result)` if `name`
/// matches a flatfile tool; `None` otherwise (caller falls through
/// to the registry-driven dispatch).
///
/// # Errors
/// The inner `Result` is `Err` when a required argument is missing, when an
/// enum string (`sec_type`, `req_type`, `format`) fails to parse, when no
/// `Client` is connected, or when the underlying flat-file request
/// fails.
pub(crate) async fn try_execute_flatfile_tool(
    client: Option<&Client>,
    name: &str,
    args: &Value,
) -> Option<Result<Value, ToolError>> {
    let (sec_type, req_type) = if name == "thetadatadx_flatfile_request" {
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
                "thetadatadx_flatfile_{sec_type}_{}_{date}.{}",
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
        Err(e) => Err(classify_core_error(&e)),
    })
}

/// Map a core [`thetadatadx::Error`] onto the JSON-RPC error taxonomy.
///
/// An unserved `(sec_type, req_type)` pair fails the SDK's local dataset
/// gate with a typed invalid-parameter error before any upstream call —
/// that is a client request fault (`-32602` Invalid params), not a
/// server-side outage (`-32000` Server error). This mirrors the REST
/// `400` and C-ABI `TDX_ERR_INVALID_PARAMETER` mappings: any core error
/// whose kind reports [`is_invalid_parameter`](thetadatadx::ConfigErrorKind::is_invalid_parameter)
/// routes to Invalid params, generically — never keyed on the tool name.
fn classify_core_error(e: &thetadatadx::Error) -> ToolError {
    let message = crate::sanitize_error(&e.to_string());
    if matches!(
        e,
        thetadatadx::Error::Config { kind, .. } if kind.is_invalid_parameter()
    ) {
        ToolError::InvalidParams(message)
    } else {
        ToolError::ServerError(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unserved (sec_type, req_type) flat-file pair — e.g. option
    /// quote — is rejected by the core dataset gate with the same typed
    /// invalid-parameter error reproduced here. It must surface through
    /// the MCP tool dispatch as `-32602` Invalid params (via
    /// `ToolError::InvalidParams`), mirroring the REST 400, not `-32000`
    /// Server error.
    #[test]
    fn unserved_pair_maps_to_invalid_params() {
        // Byte-for-byte the error the core dataset gate raises for an
        // unserved pair (see flatfiles::request::validate_dataset).
        let err = thetadatadx::Error::config_invalid(
            "flatfiles.dataset",
            "flat-file service does not serve option quote",
        );
        let thetadatadx::Error::Config { kind, .. } = &err else {
            panic!("config_invalid must build an Error::Config");
        };
        assert!(
            kind.is_invalid_parameter(),
            "the unserved-pair error must classify as invalid-parameter"
        );
        assert!(
            matches!(classify_core_error(&err), ToolError::InvalidParams(_)),
            "MCP dispatch must route an invalid-parameter core error to -32602 Invalid params"
        );
    }

    /// A genuine upstream/server fault carries no invalid-parameter
    /// classification and must stay on `-32000` Server error.
    #[test]
    fn server_fault_maps_to_server_error() {
        let err = thetadatadx::Error::config_internal("flatfiles: decode task panicked");
        let thetadatadx::Error::Config { kind, .. } = &err else {
            panic!("config_internal must build an Error::Config");
        };
        assert!(!kind.is_invalid_parameter());
        assert!(matches!(
            classify_core_error(&err),
            ToolError::ServerError(_)
        ));
    }
}
