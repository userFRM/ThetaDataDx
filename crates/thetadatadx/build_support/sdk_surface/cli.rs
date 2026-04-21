//! CLI command builders + dispatch arms for `tools/cli/src/utilities.rs`.

use std::fmt::Write as _;

use super::common::{
    cli_param_name, emit_cli_f64_arg, find_utility_param, generated_header, greek_result_fields,
    rust_string_literal,
};
use super::spec::{UtilityKind, UtilitySpec};

pub(super) fn render_cli_utilities(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("fn add_generated_utility_commands(mut app: Command) -> Command {\n");
    for utility in utilities {
        out.push_str(&cli_command_builder(utility));
    }
    out.push_str("    app\n}\n\n");
    out.push_str(
        "async fn try_run_generated_utility(\n    subcommand: Option<(&str, &ArgMatches)>,\n    fmt: &OutputFormat,\n    creds_path: &str,\n) -> Result<bool, thetadatadx::Error> {\n    match subcommand {\n",
    );
    for utility in utilities {
        out.push_str(&cli_dispatch_arm(utility));
    }
    out.push_str("        _ => Ok(false),\n    }\n}\n");
    out
}

fn cli_command_builder(utility: &UtilitySpec) -> String {
    let cli_name = utility.cli_name.as_deref().unwrap_or(&utility.name);
    let cli_about = utility.cli_about.as_deref().unwrap_or(&utility.doc);
    let mut out = String::new();
    if utility.kind == UtilityKind::Auth {
        writeln!(
            out,
            "    app = app.subcommand(Command::new({}).about({}));",
            rust_string_literal(cli_name),
            rust_string_literal(cli_about)
        )
        .unwrap();
        return out;
    }

    out.push_str("    app = app.subcommand(\n");
    writeln!(
        out,
        "        Command::new({})",
        rust_string_literal(cli_name)
    )
    .unwrap();
    writeln!(
        out,
        "            .about({})",
        rust_string_literal(cli_about)
    )
    .unwrap();
    for param in &utility.params {
        out.push_str("            .arg(\n");
        writeln!(
            out,
            "                Arg::new({})",
            rust_string_literal(cli_param_name(param))
        )
        .unwrap();
        out.push_str("                    .required(true)\n");
        writeln!(
            out,
            "                    .help({}),",
            rust_string_literal(&param.doc)
        )
        .unwrap();
        out.push_str("            )\n");
    }
    out.push_str("    );\n");
    out
}

fn cli_dispatch_arm(utility: &UtilitySpec) -> String {
    let cli_name = utility.cli_name.as_deref().unwrap_or(&utility.name);
    let mut out = String::new();
    match utility.kind {
        UtilityKind::Auth => {
            writeln!(
                out,
                "        Some(({}, _)) => {{",
                rust_string_literal(cli_name)
            )
            .unwrap();
            out.push_str(
                "            let creds = thetadatadx::Credentials::from_file(creds_path)?;\n",
            );
            out.push_str(
                "            let resp = thetadatadx::auth::authenticate(&creds).await?;\n",
            );
            out.push_str("            let mut td = TabularData::new(vec![\n");
            out.push_str("                \"session_id\",\n                \"email\",\n                \"stock_tier\",\n                \"options_tier\",\n                \"indices_tier\",\n                \"rate_tier\",\n                \"created\",\n            ]);\n");
            out.push_str("            let user = resp.user.as_ref();\n");
            out.push_str("            let redacted_session = if resp.session_id.len() >= 8 {\n");
            out.push_str("                format!(\"{}...\", &resp.session_id[..8])\n");
            out.push_str("            } else {\n");
            out.push_str("                resp.session_id.clone()\n");
            out.push_str("            };\n");
            out.push_str("            td.push(vec![\n");
            out.push_str("                redacted_session,\n");
            out.push_str(
                "                user.and_then(|u| u.email.clone()).unwrap_or_default(),\n",
            );
            out.push_str("                user.and_then(|u| u.stock_subscription)\n                    .map(|t| format!(\"{t}\"))\n                    .unwrap_or_default(),\n");
            out.push_str("                user.and_then(|u| u.options_subscription)\n                    .map(|t| format!(\"{t}\"))\n                    .unwrap_or_default(),\n");
            out.push_str("                user.and_then(|u| u.indices_subscription)\n                    .map(|t| format!(\"{t}\"))\n                    .unwrap_or_default(),\n");
            out.push_str("                user.and_then(|u| u.interest_rate_subscription)\n                    .map(|t| format!(\"{t}\"))\n                    .unwrap_or_default(),\n");
            out.push_str("                resp.session_created.unwrap_or_default(),\n");
            out.push_str("            ]);\n");
            out.push_str("            td.render(fmt);\n");
            out.push_str("            Ok(true)\n");
            out.push_str("        }\n");
        }
        UtilityKind::AllGreeks => {
            writeln!(
                out,
                "        Some(({}, sub_m)) => {{",
                rust_string_literal(cli_name)
            )
            .unwrap();
            emit_cli_f64_arg(&mut out, utility, "spot", "spot");
            emit_cli_f64_arg(&mut out, utility, "strike", "strike");
            emit_cli_f64_arg(&mut out, utility, "rate", "rate");
            emit_cli_f64_arg(&mut out, utility, "div_yield", "div_yield");
            emit_cli_f64_arg(&mut out, utility, "tte", "tte");
            emit_cli_f64_arg(&mut out, utility, "option_price", "option_price");
            let right_key = cli_param_name(find_utility_param(utility, "right"));
            writeln!(
                out,
                "            let right = get_arg(sub_m, {});",
                rust_string_literal(right_key)
            )
            .unwrap();
            out.push_str("            thetadatadx::parse_right_strict(right)?;\n");
            out.push_str("            let g = tdbe::greeks::all_greeks(spot, strike, rate, div_yield, tte, option_price, right);\n");
            out.push_str(
                "            let mut td = TabularData::new(vec![\"greek\", \"value\"]);\n",
            );
            out.push_str("            let rows = [\n");
            for (field, rust_field) in greek_result_fields() {
                writeln!(
                    out,
                    "                ({}, g.{rust_field}),",
                    rust_string_literal(field)
                )
                .unwrap();
            }
            out.push_str("            ];\n");
            out.push_str("            for (name, val) in rows {\n");
            out.push_str(
                "                td.push(vec![name.to_string(), format!(\"{val:.8}\")]);\n",
            );
            out.push_str("            }\n");
            out.push_str("            td.render(fmt);\n");
            out.push_str("            Ok(true)\n");
            out.push_str("        }\n");
        }
        UtilityKind::ImpliedVolatility => {
            writeln!(
                out,
                "        Some(({}, sub_m)) => {{",
                rust_string_literal(cli_name)
            )
            .unwrap();
            emit_cli_f64_arg(&mut out, utility, "spot", "spot");
            emit_cli_f64_arg(&mut out, utility, "strike", "strike");
            emit_cli_f64_arg(&mut out, utility, "rate", "rate");
            emit_cli_f64_arg(&mut out, utility, "div_yield", "div_yield");
            emit_cli_f64_arg(&mut out, utility, "tte", "tte");
            emit_cli_f64_arg(&mut out, utility, "option_price", "option_price");
            let right_key = cli_param_name(find_utility_param(utility, "right"));
            writeln!(
                out,
                "            let right = get_arg(sub_m, {});",
                rust_string_literal(right_key)
            )
            .unwrap();
            out.push_str("            thetadatadx::parse_right_strict(right)?;\n");
            out.push_str("            let (iv, iv_error) = tdbe::greeks::implied_volatility(spot, strike, rate, div_yield, tte, option_price, right);\n");
            out.push_str(
                "            let mut td = TabularData::new(vec![\"iv\", \"iv_error\"]);\n",
            );
            out.push_str(
                "            td.push(vec![format!(\"{iv:.8}\"), format!(\"{iv_error:.8}\")]);\n",
            );
            out.push_str("            td.render(fmt);\n");
            out.push_str("            Ok(true)\n");
            out.push_str("        }\n");
        }
        UtilityKind::Ping => panic!("ping is MCP-only"),
    }
    out
}
