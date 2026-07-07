//! MCP (Model Context Protocol) server tool definitions + execute arms.

use std::fmt::Write as _;

use super::common::{
    generated_header, mcp_json_type, mcp_param_description, mcp_param_name,
    rust_string_array_literal, rust_string_literal,
};
use super::spec::{UtilityKind, UtilitySpec};

/// Renders the MCP utilities source: the tool definitions and the execute arms that run each utility.
pub(super) fn render_mcp_utilities(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("fn push_generated_utility_tool_definitions(tools: &mut Vec<Value>) {\n");
    for utility in utilities {
        out.push_str(&mcp_tool_definition(utility));
    }
    out.push_str("}\n\n");
    out.push_str(&render_try_execute_preamble(utilities));
    for utility in utilities {
        out.push_str(&mcp_execute_arm(utility));
    }
    out.push_str("        _ => None,\n    }\n}\n");
    out
}

/// Whether any execute arm reads the JSON `args` map (and so needs the
/// `param_or_return!` argument-fetch helper). `ping` reads only the
/// connection state and the server clock, so a roster of arg-free
/// utilities emits neither the `args` binding nor the macro.
fn any_utility_reads_args(utilities: &[&UtilitySpec]) -> bool {
    utilities
        .iter()
        .any(|u| !matches!(u.kind, UtilityKind::Ping))
}

/// Emit the `try_execute_generated_utility` signature + `match name`
/// opener. The `args` parameter and the `param_or_return!` argument-fetch
/// macro are emitted only when an arm reads them, so an arg-free roster
/// does not carry an unused binding or macro.
fn render_try_execute_preamble(utilities: &[&UtilitySpec]) -> String {
    let reads_args = any_utility_reads_args(utilities);
    let args_binding = if reads_args { "args" } else { "_args" };
    let mut out = String::new();
    out.push_str("async fn try_execute_generated_utility(\n");
    out.push_str("    client: Option<&Client>,\n");
    out.push_str("    name: &str,\n");
    writeln!(out, "    {args_binding}: &Value,").unwrap();
    out.push_str("    start_time: std::time::Instant,\n");
    out.push_str(") -> Option<Result<Value, ToolError>> {\n");
    if reads_args {
        out.push_str("    macro_rules! param_or_return {\n");
        out.push_str("        ($expr:expr) => {\n");
        out.push_str("            match $expr {\n");
        out.push_str("                Ok(value) => value,\n");
        out.push_str(
            "                Err(error) => return Some(Err(ToolError::InvalidParams(error))),\n",
        );
        out.push_str("            }\n");
        out.push_str("        };\n");
        out.push_str("    }\n");
    }
    out.push_str("    match name {\n");
    out
}

fn mcp_tool_definition(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    out.push_str("    tools.push(json!({\n");
    writeln!(
        out,
        "        \"name\": {},",
        rust_string_literal(&utility.name)
    )
    .unwrap();
    writeln!(
        out,
        "        \"description\": {},",
        rust_string_literal(utility.mcp_description.as_deref().unwrap_or(&utility.doc))
    )
    .unwrap();
    out.push_str("        \"inputSchema\": {\n");
    out.push_str("            \"type\": \"object\",\n");
    out.push_str("            \"properties\": {\n");
    for (index, param) in utility.params.iter().enumerate() {
        let suffix = if index + 1 == utility.params.len() {
            ""
        } else {
            ","
        };
        write!(
            out,
            "                {}: {{ \"type\": {}, \"description\": {}",
            rust_string_literal(mcp_param_name(param)),
            rust_string_literal(mcp_json_type(param.param_type)),
            rust_string_literal(mcp_param_description(param))
        )
        .unwrap();
        if !param.enum_values.is_empty() {
            write!(
                out,
                ", \"enum\": {}",
                rust_string_array_literal(&param.enum_values)
            )
            .unwrap();
        }
        writeln!(out, " }}{suffix}").unwrap();
    }
    out.push_str("            },\n");
    out.push_str("            \"required\": [");
    for (index, param) in utility.params.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&rust_string_literal(mcp_param_name(param)));
    }
    out.push_str("]\n");
    out.push_str("        }\n");
    out.push_str("    }));\n");
    out
}

fn mcp_execute_arm(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    writeln!(out, "        {} => {{", rust_string_literal(&utility.name)).unwrap();
    match utility.kind {
        UtilityKind::Ping => {
            out.push_str("            let uptime = start_time.elapsed();\n");
            out.push_str("            Some(Ok(json!({\n");
            out.push_str("                \"status\": \"ok\",\n");
            out.push_str("                \"server\": \"thetadatadx-mcp-server\",\n");
            out.push_str("                \"version\": VERSION,\n");
            out.push_str("                \"uptime_secs\": uptime.as_secs(),\n");
            out.push_str("                \"connected\": client.is_some(),\n");
            out.push_str("            })))\n");
        }
        UtilityKind::Forwarder
        | UtilityKind::CalendarStatusName
        | UtilityKind::TimestampMs
        | UtilityKind::SequenceSignedToUnsigned
        | UtilityKind::SequenceUnsignedToSigned => {
            panic!("lookup-table helpers target python/typescript only, not MCP")
        }
    }
    out.push_str("        }\n");
    out
}
