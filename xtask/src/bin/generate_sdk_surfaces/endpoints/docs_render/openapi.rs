//! The routes of the published OpenAPI document.
//!
//! Every registry endpoint, and every second route the server mounts to one,
//! is rendered from the endpoint table: its tier and its `expiration` rule
//! from the vendor's pinned spec, its documentation, its parameters as the
//! document's parameter anchors and its examples as the call the endpoint's
//! signature takes. The rest of the document (the parameter and response
//! anchors, the terminal's system routes and the components) is written by
//! hand around the generated region.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use heck::{ToLowerCamelCase, ToUpperCamelCase};

use super::super::super::upstream_openapi::UpstreamOpenApi;
use super::super::modes::endpoint_supports_expiration_wildcard;
use super::super::sdk_helpers::{cpp_arg_literal, method_params, python_arg_literal};
use super::super::table::{GeneratedEndpoint, GeneratedParam};
use super::super::test_fixtures::{category_symbol, type_value};
use super::lang::showcased_builder_params;
use super::page::rewrite_vendor_docstring;

pub(super) const OPENAPI_PATH: &str = "docs-site/docs/public/thetadatadx.yaml";
const BEGIN_MARKER: &str = "  # --- BEGIN GENERATED ROUTES (endpoint table) ---\n";
const END_MARKER: &str = "  # --- END GENERATED ROUTES ---\n";

/// A second route the server mounts to an endpoint (`register_v3_path_routes`
/// in `tools/server/src/router.rs`), with the operation id it is published
/// under.
struct RouteAlias {
    path: &'static str,
    endpoint: &'static str,
    operation_id: &'static str,
}

const ROUTE_ALIASES: &[RouteAlias] = &[
    RouteAlias {
        path: "/v3/stock/list/dates/{request_type}",
        endpoint: "stock_list_dates",
        operation_id: "stockListDatesByRequestType",
    },
    RouteAlias {
        path: "/v3/option/list/dates/{request_type}",
        endpoint: "option_list_dates",
        operation_id: "optionListDatesByRequestType",
    },
    RouteAlias {
        path: "/v3/option/list/contracts/{request_type}",
        endpoint: "option_list_contracts",
        operation_id: "optionListContractsByRequestType",
    },
    RouteAlias {
        path: "/v3/calendar/today",
        endpoint: "calendar_open_today",
        operation_id: "calendarToday",
    },
    RouteAlias {
        path: "/v3/calendar/year_holidays",
        endpoint: "calendar_year",
        operation_id: "calendarYearHolidays",
    },
    RouteAlias {
        path: "/v3/interest_rate/history/eod",
        endpoint: "interest_rate_history_eod",
        operation_id: "interestRateHistoryEodPath",
    },
];

/// What a parameter anchor of the document's `x-anchors` publishes.
#[derive(Default)]
struct Anchor<'a> {
    name: &'a str,
    location: &'a str,
    required: bool,
    default: Option<&'a str>,
}

/// The OpenAPI document on disk with its generated region rendered: every
/// endpoint's route, each followed by the server's second routes to it.
pub(super) fn render_openapi_document(
    repo_root: &Path,
    endpoints: &[&GeneratedEndpoint],
    upstream: &UpstreamOpenApi,
) -> Result<String, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(repo_root.join(OPENAPI_PATH))?;
    let missing = || format!("{OPENAPI_PATH} lacks the generated-routes markers");
    let (head, rest) = text.split_once(BEGIN_MARKER).ok_or_else(missing)?;
    let (_, tail) = rest.split_once(END_MARKER).ok_or_else(missing)?;
    let anchors = parameter_anchors(head);

    let mut out = format!("{head}{BEGIN_MARKER}");
    for alias in ROUTE_ALIASES {
        if !endpoints
            .iter()
            .any(|endpoint| endpoint.name == alias.endpoint)
        {
            return Err(format!(
                "route alias {} names {}, which is not an endpoint",
                alias.path, alias.endpoint
            )
            .into());
        }
    }
    for endpoint in endpoints {
        let tier = super::endpoint_tier(endpoint, upstream);
        render_route(&mut out, endpoint, None, &tier, &anchors)?;
        for alias in ROUTE_ALIASES
            .iter()
            .filter(|alias| alias.endpoint == endpoint.name)
        {
            render_route(&mut out, endpoint, Some(alias), &tier, &anchors)?;
        }
    }
    out.push('\n');
    out.push_str(END_MARKER);
    out.push_str(tail);
    Ok(out)
}

/// The parameter anchors the hand-written head declares, by anchor name.
fn parameter_anchors(head: &str) -> HashMap<&str, Anchor<'_>> {
    let mut anchors: HashMap<&str, Anchor<'_>> = HashMap::new();
    let mut current = None;
    for line in head.lines() {
        if let Some((_, anchor)) = line
            .strip_prefix("  ")
            .filter(|key| !key.starts_with(' '))
            .and_then(|key| key.split_once(": &"))
        {
            current = Some(anchor);
            continue;
        }
        let Some(anchor) = current else { continue };
        let entry = anchors.entry(anchor).or_default();
        let value = |prefix: &str| line.strip_prefix(prefix).map(str::trim);
        if let Some(name) = value("    name: ") {
            entry.name = name;
        } else if let Some(location) = value("    in: ") {
            entry.location = location;
        } else if let Some(required) = value("    required: ") {
            entry.required = required == "true";
        } else if let Some(default) = value("      default: ") {
            entry.default = Some(default.trim_matches('\''));
        }
    }
    anchors
}

/// One route: the endpoint's own, or `alias`'s second route to it.
fn render_route(
    out: &mut String,
    endpoint: &GeneratedEndpoint,
    alias: Option<&RouteAlias>,
    tier: &str,
    anchors: &HashMap<&str, Anchor<'_>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = alias.map_or(endpoint.rest_path, |alias| alias.path);
    let operation_id = alias.map_or_else(
        || endpoint.name.to_lower_camel_case(),
        |alias| alias.operation_id.to_string(),
    );
    let _ = writeln!(
        out,
        "\n  {path}:\n    x-min-subscription: {tier}\n    get:\n      \
         operationId: {operation_id}\n      summary: {}\n      description: |",
        serde_json::to_string(endpoint.description)?
    );
    let mut description = Vec::new();
    if alias.is_some() {
        description.push(if path.contains("{request_type}") {
            format!(
                "The path form of `{}`: the request type is the last path segment \
                 rather than a query parameter. Same endpoint, same response.",
                endpoint.rest_path
            )
        } else {
            format!(
                "Another path to `{}`. Same endpoint, same response.",
                endpoint.rest_path
            )
        });
        description.push(String::new());
    }
    let docs = rewrite_vendor_docstring(&endpoint.doc[1..].join("\n"));
    if !docs.is_empty() {
        description.extend(docs.lines().map(str::to_string));
        description.push(String::new());
    }
    description.push(format!("gRPC: Get{}", endpoint.name.to_upper_camel_case()));
    for line in &description {
        if line.is_empty() {
            out.push('\n');
        } else {
            let _ = writeln!(out, "        {}", line.trim_end());
        }
    }
    let _ = writeln!(out, "      tags: [{}]\n      parameters:", tag(endpoint));
    let wildcard = endpoint_supports_expiration_wildcard(endpoint.name);
    for param in endpoint.params {
        let in_path = path.contains(&format!("{{{}}}", param.name));
        let key = anchor_key(param, in_path, wildcard, anchors);
        let anchor = anchors
            .get(key.as_str())
            .filter(|anchor| !anchor.name.is_empty());
        let Some(anchor) = anchor else {
            return Err(format!(
                "{OPENAPI_PATH} declares no parameter anchor `{key}` for `{}` of {path}",
                param.name
            )
            .into());
        };
        let location = if in_path { "path" } else { "query" };
        if anchor.name != param.name
            || anchor.location != location
            || anchor.required != param.required
            || anchor.default != param.default
        {
            return Err(format!(
                "{OPENAPI_PATH} anchor `{key}` publishes `{}` in {} with required={} and \
                 default {}, but {path} takes `{}` in {location} with required={} and \
                 default {}",
                anchor.name,
                anchor.location,
                anchor.required,
                anchor.default.unwrap_or("none"),
                param.name,
                param.required,
                param.default.unwrap_or("none"),
            )
            .into());
        }
        let _ = writeln!(out, "        - *{key}");
    }
    let response = if endpoint.subcategory == "flat_file" {
        "flat-file-response"
    } else {
        "v3-response"
    };
    let _ = writeln!(
        out,
        "        - *format-param\n      responses:\n        '200': *{response}"
    );
    out.push_str(&example_block(endpoint));
    Ok(())
}

/// The anchor a parameter aliases: named after the parameter, with the
/// optional form where the document declares one, the array form for a
/// symbol list, the path form for a path segment and the form that rejects
/// `*` where the vendor's spec does.
fn anchor_key(
    param: &GeneratedParam,
    in_path: bool,
    wildcard: bool,
    anchors: &HashMap<&str, Anchor<'_>>,
) -> String {
    let name = param.name;
    if in_path {
        return format!("{name}-path-param");
    }
    if name == "expiration" && !wildcard {
        return "expiration-no-star-param".to_string();
    }
    if param.param_type == "Symbols" {
        return format!("{name}s-param");
    }
    let optional = format!("{name}-optional-param");
    if !param.required && anchors.contains_key(optional.as_str()) {
        return optional;
    }
    format!("{name}-param")
}

/// The tag a route is listed under.
fn tag(endpoint: &GeneratedEndpoint) -> String {
    let category = match endpoint.category {
        "calendar" => return "Calendar".to_string(),
        "rate" => return "Interest Rate".to_string(),
        "stock" => "Stock",
        "option" => "Option",
        "index" => "Index",
        other => panic!("no OpenAPI tag for category {other}"),
    };
    let group = match endpoint.subcategory {
        "flat_file" => return "Flat Files".to_string(),
        "list" => "List",
        "snapshot" => "Snapshot",
        "snapshot_greeks" => "Snapshot Greeks",
        "history" => "History",
        "history_greeks" => "History Greeks",
        "history_trade_greeks" => "History Trade Greeks",
        "at_time" => "AtTime",
        other => panic!("no OpenAPI tag for subcategory {other}"),
    };
    format!("{category} / {group}")
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
