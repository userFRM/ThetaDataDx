//! MCP (Model Context Protocol) server tool definitions + execute arms.

use std::fmt::Write as _;

use super::common::{
    find_utility_param, generated_header, greek_result_fields, mcp_json_type,
    mcp_param_description, mcp_param_name, rust_string_array_literal, rust_string_literal,
};
use super::spec::{UtilityKind, UtilitySpec};

pub(super) fn render_mcp_utilities(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("fn push_generated_utility_tool_definitions(tools: &mut Vec<Value>) {\n");
    for utility in utilities {
        out.push_str(&mcp_tool_definition(utility));
    }
    out.push_str("}\n\n");
    out.push_str(
        "async fn try_execute_generated_utility(\n    client: &Option<ThetaDataDx>,\n    name: &str,\n    args: &Value,\n    start_time: std::time::Instant,\n) -> Option<Result<Value, ToolError>> {\n    macro_rules! param_or_return {\n        ($expr:expr) => {\n            match $expr {\n                Ok(value) => value,\n                Err(error) => return Some(Err(ToolError::InvalidParams(error))),\n            }\n        };\n    }\n    match name {\n",
    );
    for utility in utilities {
        out.push_str(&mcp_execute_arm(utility));
    }
    out.push_str("        _ => None,\n    }\n}\n");
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
            out.push_str("                \"server\": \"thetadatadx-mcp\",\n");
            out.push_str("                \"version\": VERSION,\n");
            out.push_str("                \"uptime_secs\": uptime.as_secs(),\n");
            out.push_str("                \"connected\": client.is_some(),\n");
            out.push_str("            })))\n");
        }
        UtilityKind::AllGreeks => {
            let spot_key = mcp_param_name(find_utility_param(utility, "spot"));
            let strike_key = mcp_param_name(find_utility_param(utility, "strike"));
            let rate_key = mcp_param_name(find_utility_param(utility, "rate"));
            let div_key = mcp_param_name(find_utility_param(utility, "div_yield"));
            let tte_key = mcp_param_name(find_utility_param(utility, "tte"));
            let option_price_key = mcp_param_name(find_utility_param(utility, "option_price"));
            let right_key = mcp_param_name(find_utility_param(utility, "right"));
            writeln!(
                out,
                "            let spot = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(spot_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let strike = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(strike_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let rate = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(rate_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let div_yield = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(div_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let tte = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(tte_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let option_price = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(option_price_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let right = param_or_return!(arg_str(args, {}));",
                rust_string_literal(right_key)
            )
            .unwrap();
            out.push_str("            param_or_return!(thetadatadx::parse_right_strict(&right).map_err(|e| e.to_string()));\n");
            out.push_str("            let g = tdbe::greeks::all_greeks(spot, strike, rate, div_yield, tte, option_price, &right);\n");
            out.push_str("            Some(Ok(json!({\n");
            for (field, rust_field) in greek_result_fields() {
                writeln!(
                    out,
                    "                {}: g.{rust_field},",
                    rust_string_literal(field)
                )
                .unwrap();
            }
            out.push_str("            })))\n");
        }
        UtilityKind::ImpliedVolatility => {
            let spot_key = mcp_param_name(find_utility_param(utility, "spot"));
            let strike_key = mcp_param_name(find_utility_param(utility, "strike"));
            let rate_key = mcp_param_name(find_utility_param(utility, "rate"));
            let div_key = mcp_param_name(find_utility_param(utility, "div_yield"));
            let tte_key = mcp_param_name(find_utility_param(utility, "tte"));
            let option_price_key = mcp_param_name(find_utility_param(utility, "option_price"));
            let right_key = mcp_param_name(find_utility_param(utility, "right"));
            writeln!(
                out,
                "            let spot = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(spot_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let strike = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(strike_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let rate = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(rate_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let div_yield = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(div_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let tte = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(tte_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let option_price = param_or_return!(arg_f64(args, {}));",
                rust_string_literal(option_price_key)
            )
            .unwrap();
            writeln!(
                out,
                "            let right = param_or_return!(arg_str(args, {}));",
                rust_string_literal(right_key)
            )
            .unwrap();
            out.push_str("            param_or_return!(thetadatadx::parse_right_strict(&right).map_err(|e| e.to_string()));\n");
            out.push_str("            let (iv, err) = tdbe::greeks::implied_volatility(spot, strike, rate, div_yield, tte, option_price, &right);\n");
            out.push_str("            Some(Ok(json!({\n");
            out.push_str("                \"implied_volatility\": iv,\n");
            out.push_str("                \"error\": err,\n");
            out.push_str("            })))\n");
        }
        UtilityKind::Auth => panic!("auth is CLI-only"),
    }
    out.push_str("        }\n");
    out
}
