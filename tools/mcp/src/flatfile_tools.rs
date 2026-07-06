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
//! with case-insensitive enum strings constrained to the served matrix; an
//! unserved `(sec_type, req_type)` pair is rejected with a typed
//! invalid-parameter error before any network round-trip. A missing
//! `output_path` writes to a deterministic temp path and surfaces it in
//! the response.

use sonic_rs::{json, JsonValueTrait, Value};
use thetadatadx::flatfiles::{flat_file_serves, FlatFileFormat, ReqType, SecType, SERVED_DATASETS};
use thetadatadx::Client;

/// Tool name of the multi-asset flat-file dispatcher. Shared so the tool
/// registration and the `tools/list` subscription gate agree on one literal.
pub(crate) const FLATFILE_DISPATCHER_TOOL: &str = "thetadatadx_flatfile_request";

use crate::ToolError;

/// Both case spellings of a token, in matrix-stable order: the upper-case
/// form (`OPTION`) first, then the lower-case form (`option`), matching the
/// case-insensitive parser. Duplicates (a token already collected) are
/// skipped so a token that is its own lower-case never appears twice.
fn case_variants(upper: &str, out: &mut Vec<String>) {
    for token in [upper.to_string(), upper.to_ascii_lowercase()] {
        if !out.contains(&token) {
            out.push(token);
        }
    }
}

/// Distinct, matrix-ordered `sec_type` tokens (both case spellings) for the
/// served datasets. Derived from the served matrix so the advertised choices
/// can never drift from what the flat-file service actually serves.
fn served_sec_type_tokens() -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for (sec, _) in SERVED_DATASETS {
        case_variants(&sec.to_string(), &mut tokens);
    }
    tokens
}

/// Distinct, matrix-ordered `req_type` tokens (both case spellings) across all
/// served datasets, for the standalone `req_type` enum.
fn served_req_type_tokens() -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for (_, req) in SERVED_DATASETS {
        case_variants(&req.as_str().to_ascii_uppercase(), &mut tokens);
    }
    tokens
}

/// The served `req_type` tokens (both case spellings) for one `sec_type`, in
/// matrix order. Empty when the security type has no served flat-file dataset.
fn served_req_type_tokens_for(sec: SecType) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for (served_sec, req) in SERVED_DATASETS {
        if *served_sec == sec {
            case_variants(&req.as_str().to_ascii_uppercase(), &mut tokens);
        }
    }
    tokens
}

/// A `oneOf` branch list that pairs each served `sec_type` with exactly the
/// `req_type` tokens served for it, derived from `SERVED_DATASETS`. This makes
/// the generic-tool schema pair-aware: a JSON-schema validator accepts a
/// `(sec_type, req_type)` document only when it matches a served pair, so an
/// individually-valid-but-unserved combination (e.g. stock `open_interest`)
/// fails the schema rather than parsing and being rejected only at runtime.
fn served_pair_branches() -> Vec<Value> {
    let mut branches: Vec<Value> = Vec::new();
    let mut seen_secs: Vec<String> = Vec::new();
    for (sec, _) in SERVED_DATASETS {
        let sec_upper = sec.to_string();
        if seen_secs.contains(&sec_upper) {
            continue;
        }
        seen_secs.push(sec_upper.clone());

        let sec_tokens: Vec<Value> = {
            let mut v = Vec::new();
            case_variants(&sec_upper, &mut v);
            v.into_iter().map(|t| Value::from(t.as_str())).collect()
        };
        let req_tokens: Vec<Value> = served_req_type_tokens_for(*sec)
            .into_iter()
            .map(|t| Value::from(t.as_str()))
            .collect();

        branches.push(json!({
            "properties": {
                "sec_type": { "enum": sec_tokens },
                "req_type": { "enum": req_tokens },
            }
        }));
    }
    branches
}

/// `sec_type` / `req_type` enum tokens as JSON `Value`s, derived from the
/// served matrix.
fn token_values(tokens: Vec<String>) -> Vec<Value> {
    tokens.iter().map(|t| Value::from(t.as_str())).collect()
}

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
        "name": FLATFILE_DISPATCHER_TOOL,
        "description": "Generic flat-file request. Pull a whole-universe daily blob for a \
                        served (sec_type, req_type) combination. The flat-file service serves \
                        option trade_quote / open_interest / eod and stock trade_quote / eod; \
                        any other pair is rejected. Returns the written file path.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "sec_type": {
                    "type": "string",
                    "description": "Security type. Only security types with a served flat-file \
                                    dataset are accepted.",
                    "enum": token_values(served_sec_type_tokens())
                },
                "req_type": {
                    "type": "string",
                    "description": "Request type. Only request types served as a flat file are \
                                    accepted; the valid set depends on the security type.",
                    "enum": token_values(served_req_type_tokens())
                },
                "date": date_prop,
                "output_path": output_prop,
                "format": format_prop,
            },
            "required": ["sec_type", "req_type", "date"],
            // Pair-aware constraint: the (sec_type, req_type) document must match
            // one served branch. The per-property enums above narrow each field to
            // a served token; this oneOf rejects an unserved combination of two
            // individually-valid tokens (e.g. stock open_interest) at the schema
            // level, before the handler runs. Derived from SERVED_DATASETS.
            "oneOf": served_pair_branches()
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

fn arg_str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(sonic_rs::JsonValueTrait::as_str)
        .map(ToString::to_string)
}

/// Map a tool-name suffix to a `(SecType, ReqType)` pair, e.g.
/// `"thetadatadx_flatfile_option_eod"` -> `Some((Option, Eod))`. Only
/// the datasets the flat-file distribution serves have a convenience
/// tool; every other request type is reachable via the market-data
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
        let sec_str = match crate::arg_str(args, "sec_type") {
            Ok(s) => s,
            Err(e) => return Some(Err(ToolError::InvalidParams(e))),
        };
        let req_str = match crate::arg_str(args, "req_type") {
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

    // Reject an unserved (sec_type, req_type) pair at the surface, before any
    // path or network work. The advertised enums already exclude pairs the
    // service never serves; this guards the generic tool against a combination
    // that parses to valid variants but isn't a served flat-file dataset
    // (e.g. stock open_interest). The served matrix is the single source.
    if !flat_file_serves(sec_type, req_type) {
        return Some(Err(ToolError::InvalidParams(format!(
            "flat-file service does not serve {sec_type} {}",
            req_type.as_str()
        ))));
    }

    let date = match crate::arg_str(args, "date") {
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
                "ThetaData client not connected. Set THETADATA_API_KEY, or THETADATA_EMAIL + \
                 THETADATA_PASSWORD, or pass --api-key / --creds."
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
            "req_type": req_type.as_str(),
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
/// `400` and C-ABI `THETADATADX_ERR_INVALID_PARAMETER` mappings: any core error
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
    use sonic_rs::JsonContainerTrait;

    use super::*;

    /// Collect the advertised `enum` tokens for a property on the generic
    /// `thetadatadx_flatfile_request` tool schema.
    fn generic_request_enum(property: &str) -> Vec<String> {
        let mut tools: Vec<Value> = Vec::new();
        push_flatfile_tool_definitions(&mut tools);
        let generic = tools
            .iter()
            .find(|t| t.get("name").as_str() == Some("thetadatadx_flatfile_request"))
            .expect("generic flatfile tool must be advertised");
        generic
            .pointer(["inputSchema", "properties", property, "enum"])
            .and_then(|v| v.as_array())
            .expect("property must carry an enum")
            .iter()
            .filter_map(|v| v.as_str().map(ToString::to_string))
            .collect()
    }

    /// The advertised `sec_type` / `req_type` enums must cover exactly the
    /// tokens the served matrix yields (each in both cases) and must not offer
    /// a request type the flat-file service never serves — no per-tick `QUOTE`
    /// / `TRADE` / `OHLC`, and no `INDEX` security (not a flat-file dataset).
    #[test]
    fn generic_tool_enums_match_the_served_matrix() {
        let sec_tokens = generic_request_enum("sec_type");
        for served in ["OPTION", "STOCK", "option", "stock"] {
            assert!(
                sec_tokens.iter().any(|t| t == served),
                "sec_type enum must advertise `{served}`; got {sec_tokens:?}"
            );
        }
        for unserved in ["INDEX", "index"] {
            assert!(
                !sec_tokens.iter().any(|t| t == unserved),
                "sec_type enum must not advertise unserved `{unserved}`; got {sec_tokens:?}"
            );
        }

        let req_tokens = generic_request_enum("req_type");
        for served in ["TRADE_QUOTE", "OPEN_INTEREST", "EOD"] {
            assert!(
                req_tokens.iter().any(|t| t == served),
                "req_type enum must advertise `{served}`; got {req_tokens:?}"
            );
        }
        for unserved in ["QUOTE", "TRADE", "OHLC", "quote", "trade", "ohlc"] {
            assert!(
                !req_tokens.iter().any(|t| t == unserved),
                "req_type enum must not advertise unserved `{unserved}`; got {req_tokens:?}"
            );
        }
    }

    /// A pair that parses to valid variants but is not a served flat-file
    /// dataset — stock `open_interest` — must be rejected at the surface with
    /// `-32602` Invalid params, before any client work, even though both the
    /// security type and the request type are individually served.
    #[tokio::test]
    async fn unserved_but_parseable_pair_is_rejected_at_the_surface() {
        let args = json!({
            "sec_type": "STOCK",
            "req_type": "OPEN_INTEREST",
            "date": "20260428",
        });
        let result = try_execute_flatfile_tool(None, "thetadatadx_flatfile_request", &args)
            .await
            .expect("the generic flatfile tool must handle this name");
        match result {
            Err(ToolError::InvalidParams(msg)) => {
                assert!(
                    msg.contains("does not serve") && msg.contains("open_interest"),
                    "rejection must name the unserved dataset; got {msg:?}"
                );
            }
            other => panic!("unserved pair must map to InvalidParams; got {other:?}"),
        }
    }

    /// A served pair (stock `eod`) must pass the surface pair check and
    /// reach the client stage, where with no connected client it surfaces the
    /// not-connected server error rather than an invalid-parameter rejection.
    /// This proves the pair gate accepts a genuinely served combination.
    #[tokio::test]
    async fn served_pair_passes_the_surface_check() {
        let args = json!({
            "sec_type": "STOCK",
            "req_type": "EOD",
            "date": "20260428",
        });
        let result = try_execute_flatfile_tool(None, "thetadatadx_flatfile_request", &args)
            .await
            .expect("the generic flatfile tool must handle this name");
        match result {
            Err(ToolError::ServerError(msg)) => {
                assert!(
                    msg.contains("not connected"),
                    "a served pair with no client must reach the not-connected error; got {msg:?}"
                );
            }
            other => panic!("a served pair must not be rejected as invalid; got {other:?}"),
        }
    }

    /// Collect the generic tool's `oneOf` pair branches as
    /// `(sec_tokens, req_tokens)` pairs.
    fn generic_request_pair_branches() -> Vec<(Vec<String>, Vec<String>)> {
        let mut tools: Vec<Value> = Vec::new();
        push_flatfile_tool_definitions(&mut tools);
        let generic = tools
            .iter()
            .find(|t| t.get("name").as_str() == Some("thetadatadx_flatfile_request"))
            .expect("generic flatfile tool must be advertised");
        generic
            .pointer(["inputSchema", "oneOf"])
            .and_then(|v| v.as_array())
            .expect("the generic schema must carry a oneOf of served pairs")
            .iter()
            .map(|branch| {
                let pick = |prop: &str| {
                    branch
                        .pointer(["properties", prop, "enum"])
                        .and_then(|v| v.as_array())
                        .expect("each branch must enumerate sec_type and req_type")
                        .iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect::<Vec<_>>()
                };
                (pick("sec_type"), pick("req_type"))
            })
            .collect()
    }

    /// The schema's pair-aware `oneOf` must encode exactly the served matrix:
    /// the option branch offers `eod`, `open_interest`, and `trade_quote`; the
    /// stock branch offers `eod` and `trade_quote` but never `open_interest`.
    /// This is what makes a validator reject stock `open_interest` before the
    /// handler runs. There is no `index` branch — index is not a flat-file
    /// dataset.
    #[test]
    fn generic_tool_oneof_encodes_served_pairs() {
        let branches = generic_request_pair_branches();

        let option_branch = branches
            .iter()
            .find(|(sec, _)| sec.iter().any(|t| t == "OPTION"))
            .expect("a oneOf branch must cover the option security type");
        for served in ["EOD", "OPEN_INTEREST", "TRADE_QUOTE"] {
            assert!(
                option_branch.1.iter().any(|t| t == served),
                "option branch must serve `{served}`; got {:?}",
                option_branch.1
            );
        }

        let stock_branch = branches
            .iter()
            .find(|(sec, _)| sec.iter().any(|t| t == "STOCK"))
            .expect("a oneOf branch must cover the stock security type");
        for served in ["EOD", "TRADE_QUOTE"] {
            assert!(
                stock_branch.1.iter().any(|t| t == served),
                "stock branch must serve `{served}`; got {:?}",
                stock_branch.1
            );
        }
        for unserved in ["OPEN_INTEREST", "open_interest"] {
            assert!(
                !stock_branch.1.iter().any(|t| t == unserved),
                "stock branch must not serve `{unserved}`; got {:?}",
                stock_branch.1
            );
        }

        // Index is not a flat-file dataset: no branch may cover it.
        assert!(
            !branches
                .iter()
                .any(|(sec, _)| sec.iter().any(|t| t == "INDEX" || t == "index")),
            "no oneOf branch may cover the index security type; got {branches:?}"
        );

        // One branch per served security type (option, stock).
        assert_eq!(
            branches.len(),
            2,
            "the served matrix yields exactly two security-type branches"
        );
    }

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
