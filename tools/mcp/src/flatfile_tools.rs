//! The flat-file tools: one `<endpoint>_to_path` tool per flat file.
//!
//! A flat file is the whole universe for one date, most of them far past the
//! rows one tool call returns, and MCP carries JSON-RPC rather than files. So
//! each flat file's tool writes the file to disk as the terminal writes it and
//! answers with the path. A missing `output_path` writes to a deterministic
//! temp path.

use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value, json};
use thetadatadx::flatfiles::FlatFileFormat;
use thetadatadx::{Client, ENDPOINTS};

use crate::ToolError;

/// What names a flat file's tool after its endpoint.
pub(crate) const TO_PATH_SUFFIX: &str = "_to_path";

/// The arguments a flat file's tool takes besides the endpoint's own.
const WRITE_ARGS: [&str; 2] = ["output_path", "format"];

/// Append one `<endpoint>_to_path` tool per flat file to the `tools/list`
/// array.
pub(crate) fn push_flatfile_tool_definitions(tools: &mut Vec<Value>) {
    for ep in ENDPOINTS.iter().filter(|ep| ep.subcategory == "flat_file") {
        let mut props = sonic_rs::Object::new();
        let mut required = Vec::new();
        for p in ep.params {
            props.insert(
                &p.name,
                json!({
                    "type": thetadatadx::param_type_to_json_type(p.param_type),
                    "description": crate::mcp_param_description(ep, p),
                }),
            );
            if p.required {
                required.push(p.name);
            }
        }
        props.insert(
            &"output_path",
            json!({
                "type": "string",
                "description": "Output file path. If omitted, the tool writes to a deterministic \
                                temp path and returns it in the response.",
            }),
        );
        props.insert(
            &"format",
            json!({
                "type": "string",
                "description": "The terminal's layout to write: csv (the default), json, \
                                json_legacy, ndjson (or its alias jsonl) or html.",
                "enum": ["csv", "json", "json_legacy", "ndjson", "jsonl", "html"],
            }),
        );
        let subscription = crate::AssetClass::from_category(ep.category)
            .map(|class| format!(" Requires a {} subscription.", class.subscription_label()))
            .unwrap_or_default();
        tools.push(json!({
            "name": format!("{}{TO_PATH_SUFFIX}", ep.name),
            "description": format!(
                "{} Writes the whole file to disk as the terminal writes it, however many \
                 rows it has, and returns the path written.{subscription}",
                ep.description
            ),
            "inputSchema": {
                "type": "object",
                "properties": props,
                "required": required,
            }
        }));
    }
}

/// Run the flat-file tool `name`, or return `None` when `name` is not one.
pub(crate) async fn try_execute_flatfile_tool(
    client: Option<&Client>,
    name: &str,
    args: &Value,
) -> Option<Result<Value, ToolError>> {
    let endpoint = name.strip_suffix(TO_PATH_SUFFIX)?;
    let ep = ENDPOINTS
        .iter()
        .find(|ep| ep.name == endpoint && ep.subcategory == "flat_file")?;
    Some(write_flat_file(client, ep, args).await)
}

async fn write_flat_file(
    client: Option<&Client>,
    ep: &thetadatadx::EndpointMeta,
    args: &Value,
) -> Result<Value, ToolError> {
    // A key outside the schema, such as a filter, cannot narrow a flat file;
    // refused by name rather than answered with the whole universe.
    if let Some(obj) = args.as_object()
        && let Some((key, _)) = obj
            .iter()
            .find(|(key, _)| !WRITE_ARGS.contains(key) && !ep.params.iter().any(|p| p.name == *key))
    {
        return Err(ToolError::InvalidParams(format!(
            "unknown argument '{key}' for {}{TO_PATH_SUFFIX}; accepted: date, output_path, format",
            ep.name
        )));
    }
    let date = crate::arg_str(args, "date").map_err(ToolError::InvalidParams)?;
    let format = match args.get("format").and_then(|v: &Value| v.as_str()) {
        Some(name) => name
            .parse::<FlatFileFormat>()
            .map_err(ToolError::InvalidParams)?,
        None => FlatFileFormat::Csv,
    };
    let output_path = args
        .get("output_path")
        .and_then(|v: &Value| v.as_str())
        .map_or_else(
            || {
                std::env::temp_dir().join(format!(
                    "thetadatadx_{}_{date}.{}",
                    ep.name,
                    format.extension()
                ))
            },
            std::path::PathBuf::from,
        );
    let client = client.ok_or_else(|| {
        ToolError::ServerError(
            "ThetaData client not connected. Set THETADATA_API_KEY, or THETADATA_EMAIL + \
             THETADATA_PASSWORD, or pass --api-key / --creds."
                .to_string(),
        )
    })?;
    let written = client
        .market_data()
        .flat_file_to_path(ep.name, &date, &output_path, format)
        .await
        .map_err(|e| crate::classify_core_error(&e))?;
    Ok(json!({
        "status": "ok",
        "path": written.to_string_lossy(),
        "endpoint": ep.name,
        "format": format.to_string(),
        "date": date,
    }))
}
