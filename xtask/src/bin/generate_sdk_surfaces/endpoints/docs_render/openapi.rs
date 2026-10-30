//! The `x-code-examples` of the published OpenAPI document.
//!
//! The rest of the document is written by hand. Each route's examples are the
//! call its endpoint's signature takes, rendered here: kept by hand, they
//! passed optional parameters positionally into signatures that have no such
//! positions, so a reader who copied one got a call that does not compile.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use super::super::sdk_helpers::{cpp_arg_literal, method_params, python_arg_literal};
use super::super::table::{GeneratedEndpoint, GeneratedParam};
use super::super::test_fixtures::{category_symbol, type_value};
use super::lang::showcased_builder_params;

pub(super) const OPENAPI_PATH: &str = "docs-site/docs/public/thetadatadx.yaml";

/// The server's second routes to an endpoint (`register_v3_path_routes` in
/// `tools/server/src/router.rs`), each documented with its endpoint's
/// examples.
const ROUTE_ALIASES: &[(&str, &str)] = &[
    ("/v3/stock/list/dates/{request_type}", "stock_list_dates"),
    ("/v3/option/list/dates/{request_type}", "option_list_dates"),
    (
        "/v3/option/list/contracts/{request_type}",
        "option_list_contracts",
    ),
    ("/v3/calendar/today", "calendar_open_today"),
    ("/v3/calendar/year_holidays", "calendar_year"),
    ("/v3/interest_rate/history/eod", "interest_rate_history_eod"),
];

/// The OpenAPI document on disk with every route's `x-code-examples` block
/// rendered from its endpoint. A route with no block is an error: the block
/// is where the examples go, and a route without one publishes no call.
pub(super) fn render_openapi_document(
    repo_root: &Path,
    endpoints: &[&GeneratedEndpoint],
) -> Result<String, Box<dyn std::error::Error>> {
    let mut blocks: BTreeMap<&str, String> = endpoints
        .iter()
        .map(|endpoint| (endpoint.rest_path, example_block(endpoint)))
        .collect();
    for &(route, name) in ROUTE_ALIASES {
        let endpoint = endpoints
            .iter()
            .find(|endpoint| endpoint.name == name)
            .ok_or_else(|| format!("route alias {route} names {name}, which is not an endpoint"))?;
        blocks.insert(route, example_block(endpoint));
    }

    let text = std::fs::read_to_string(repo_root.join(OPENAPI_PATH))?;
    let lines: Vec<&str> = text.lines().collect();
    let indent = |line: &str| line.len() - line.trim_start().len();
    let mut out = String::with_capacity(text.len());
    let mut route = None;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if !line.trim().is_empty() && indent(line) <= 2 {
            route = line
                .strip_prefix("  ")
                .and_then(|key| key.strip_suffix(':'))
                .filter(|key| key.starts_with('/'));
        }
        if line == "      x-code-examples:"
            && let Some(block) = route.and_then(|route| blocks.remove(route))
        {
            out.push_str(&block);
            // The old block runs to the next line at its own depth; the blank
            // lines before that line separate routes and stay.
            let mut end = i + 1;
            while end < lines.len() && (lines[end].trim().is_empty() || indent(lines[end]) > 6) {
                end += 1;
            }
            while end > i + 1 && lines[end - 1].trim().is_empty() {
                end -= 1;
            }
            i = end;
            continue;
        }
        out.push_str(line);
        out.push('\n');
        i += 1;
    }
    if let Some(route) = blocks.keys().next() {
        return Err(format!(
            "{OPENAPI_PATH} {route} carries no `x-code-examples`, so the route publishes no \
             runnable call; add the key to the route"
        )
        .into());
    }
    Ok(out)
}

/// One route's block: the same call in Rust, Python and C++.
fn example_block(endpoint: &GeneratedEndpoint) -> String {
    let name = endpoint.name;
    let variable = example_variable(endpoint);
    let required = method_params(endpoint);
    let optional = showcased_builder_params(endpoint);
    let value = |param: &GeneratedParam| example_value(endpoint, param);
    let args = |literal: fn(&GeneratedParam, &str) -> String| {
        required
            .iter()
            .map(|param| literal(param, value(param)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let set = |form: fn(&str, &str) -> String| -> String {
        optional
            .iter()
            .map(|param| form(param.name, value(param)))
            .collect()
    };

    let rust = format!(
        "let {variable} = client.market_data().{name}({}){}.await?;",
        args(rust_arg_literal),
        set(|name, value| format!(".{name}(\"{value}\")")),
    );
    let python = format!(
        "{variable} = client.market_data.{name}({}{})",
        args(python_arg_literal),
        set(|name, value| format!(", {name}=\"{value}\"")),
    );
    let setters = set(|name, value| format!(".with_{name}(\"{value}\")"));
    let options = if setters.is_empty() {
        String::new()
    } else {
        format!(", thetadatadx::EndpointRequestOptions{{}}{setters}")
    };
    let cpp = format!(
        "auto {variable} = client.market_data().{name}({}{options});",
        args(cpp_arg_literal),
    );

    let mut out = String::from("      x-code-examples:\n");
    for (lang, label, call) in [
        ("rust", "Rust", rust),
        ("python", "Python", python),
        ("cpp", "C++", cpp),
    ] {
        let _ = write!(
            out,
            "        - lang: {lang}\n          label: {label}\n          source: |\n            {call}\n"
        );
    }
    out
}

fn rust_arg_literal(param: &GeneratedParam, value: &str) -> String {
    match param.param_type {
        "Symbols" => format!("&[\"{value}\"]"),
        _ => format!("\"{value}\""),
    }
}

/// The value an example passes: the fixture the live validators send.
fn example_value(endpoint: &GeneratedEndpoint, param: &GeneratedParam) -> &'static str {
    match param.param_type {
        "Symbol" | "Symbols" => category_symbol(endpoint.category),
        // A flat file keeps only the vendor's most recent days, so a fixed
        // date goes stale; its examples show the form instead.
        "Date" if endpoint.subcategory == "flat_file" => "YYYYMMDD",
        other => type_value(other),
    }
}

/// The variable an example binds its result to: a list is named for the rows
/// it lists (`symbols`), anything else for its return type without the
/// `Ticks` suffix (`TradeGreeksAllTicks` -> `trade_greeks_all`).
fn example_variable(endpoint: &GeneratedEndpoint) -> String {
    if let Some(column) = endpoint.list_column {
        return format!("{column}s");
    }
    let stem = endpoint
        .return_type
        .strip_suffix("Ticks")
        .unwrap_or(endpoint.return_type);
    let mut out = String::new();
    for (i, ch) in stem.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}
