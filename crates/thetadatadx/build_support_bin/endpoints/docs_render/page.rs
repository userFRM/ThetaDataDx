//! Endpoint reference page assembly and the reference sidebar manifest.

use std::fmt::Write as _;

use super::super::super::ticks::schema::Schema;
use super::super::model::{GeneratedEndpoint, GeneratedParam};
use super::super::sdk_helpers::{builder_params, method_params};
use super::{lang, response};

/// Sidebar metadata for one endpoint page: its category, subcategory,
/// title, and site link.
pub(super) struct PageMeta {
    pub(super) category: String,
    pub(super) subcategory: String,
    pub(super) title: String,
    pub(super) link: String,
}

/// Bare-noun page title from the REST-path suffix after the category.
pub(super) fn endpoint_title(endpoint: &GeneratedEndpoint) -> String {
    let rest = endpoint._rest_path.trim_start_matches("/v3/");
    let mut parts: Vec<&str> = rest.split('/').collect();
    // Drop the category segment; calendar paths have no subcategory.
    parts.remove(0);
    if parts.first() == Some(&"history")
        || parts.first() == Some(&"snapshot")
        || parts.first() == Some(&"list")
        || parts.first() == Some(&"at_time")
    {
        parts.remove(0);
    }
    let leaf = parts.join("/");
    match leaf.as_str() {
        "symbols" => "Symbols",
        "dates" => "Dates",
        "expirations" => "Expirations",
        "strikes" => "Strikes",
        "contracts" => "Contracts",
        "eod" => "EOD",
        "ohlc" => "OHLC",
        "ohlc_range" => "OHLC Range",
        "trade" => "Trade",
        "quote" => "Quote",
        "trade_quote" => "Trade Quote",
        "market_value" => "Market Value",
        "open_interest" => "Open Interest",
        "price" => "Price",
        "greeks/all" => "All Greeks",
        "greeks/first_order" => "First-Order Greeks",
        "greeks/second_order" => "Second-Order Greeks",
        "greeks/third_order" => "Third-Order Greeks",
        "greeks/implied_volatility" => "Implied Volatility",
        "greeks/eod" => "EOD Greeks",
        "trade_greeks/all" => "All Trade Greeks",
        "trade_greeks/first_order" => "First-Order Trade Greeks",
        "trade_greeks/second_order" => "Second-Order Trade Greeks",
        "trade_greeks/third_order" => "Third-Order Trade Greeks",
        "trade_greeks/implied_volatility" => "Trade Implied Volatility",
        "open_today" => "Open Today",
        "on_date" => "On Date",
        "year" => "Year",
        other => panic!("no docs title mapping for endpoint leaf {other}"),
    }
    .to_string()
}

/// Rewrite the upstream vendor docstring for the docs site: relative
/// vendor links become absolute links to the vendor's documentation
/// (or to our own article when one covers the topic), and RST-style
/// double-backtick code spans become Markdown single-backtick spans.
fn rewrite_vendor_docstring(text: &str) -> String {
    let mut out = text.replace("``", "`");
    // Articles we cover ourselves.
    for (vendor, ours) in [
        (
            "/Articles/Data-And-Requests/Option-Greeks.html",
            "/articles/option-greeks",
        ),
        (
            "/Articles/Errors-Exchanges-Conditions/Trade-Conditions.html",
            "/articles/conditions",
        ),
        (
            "/Articles/Errors-Exchanges-Conditions/Quote-Conditions.html",
            "/articles/conditions",
        ),
        (
            "/Articles/Errors-Exchanges-Conditions/Exchanges.html",
            "/articles/exchanges",
        ),
    ] {
        out = out.replace(&format!("]({vendor})"), &format!("]({ours})"));
    }
    // Everything else under the vendor's docs tree points at the vendor.
    out = out.replace("](/Articles/", "](https://docs.thetadata.us/Articles/");
    out = out.replace("](/operations/", "](https://docs.thetadata.us/operations/");
    out.trim().to_string()
}

fn docs_param_type(param_type: &str) -> &'static str {
    match param_type {
        "Int" => "int",
        "Float" => "float",
        "Bool" => "bool",
        "Date" | "Expiration" => "date",
        "Symbols" => "symbols",
        _ => "string",
    }
}

/// Note appended to the `expiration` parameter row on the endpoints whose
/// upstream binding accepts the chain-wide wildcard. Single sentence so the
/// per-endpoint `expiration=*` capability is documented once, from the same
/// capability the mode taxonomy reads, instead of being hand-scattered across
/// vendor docstrings where it silently drifted out of the parameters table.
const EXPIRATION_WILDCARD_NOTE: &str =
    "Pass `*` to select all expirations for the underlying (chain-wide; query one date at a time).";

/// Build the parameters-table Description cell for one parameter row, folding
/// newlines to spaces. The `expiration` row also carries the chain-wide
/// wildcard note when this endpoint supports it; endpoints upstream binds to
/// `expiration_no_star` reject `*`, so they render the plain description.
fn param_description_cell(param: &GeneratedParam, supports_expiration_wildcard: bool) -> String {
    let base = param.description.replace('\n', " ");
    if param.name == "expiration" && supports_expiration_wildcard {
        format!("{base} {EXPIRATION_WILDCARD_NOTE}")
    } else {
        base
    }
}

fn render_params_section(endpoint: &GeneratedEndpoint) -> String {
    // Single source of truth for `expiration=*` support: the same pinned
    // upstream snapshot the mode taxonomy reads, so the rendered note and the
    // emitted wildcard test modes can never disagree.
    let supports_expiration_wildcard =
        super::super::modes::endpoint_supports_expiration_wildcard(&endpoint.name);

    let mut out = String::from("## Parameters\n\n");
    out.push_str("| Name | Type | Required | Default | Description |\n|---|---|---|---|---|\n");
    for param in method_params(endpoint)
        .into_iter()
        .chain(builder_params(endpoint))
    {
        let default = param
            .default
            .as_deref()
            .map(|d| format!("`{d}`"))
            .unwrap_or_else(|| "—".to_string());
        let required = if param.required { "yes" } else { "no" };
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} |",
            param.name,
            docs_param_type(&param.param_type),
            required,
            default,
            param_description_cell(param, supports_expiration_wildcard),
        );
    }
    out.push_str(
        "| `timeout_ms` | int | no | — | Per-request deadline in milliseconds. 0 means no deadline. |\n\n",
    );
    out
}

fn tab(slot: &str, signature: String, example: String) -> String {
    format!("<template #{slot}>\n\n{signature}\n**Example**\n\n{example}\n</template>\n")
}

/// Renders a full endpoint reference page (frontmatter, tier badge,
/// description, language tabs, parameters, response schema, and sample)
/// and returns it alongside the page's sidebar metadata.
pub(super) fn render_endpoint_page(
    endpoint: &GeneratedEndpoint,
    tier: &str,
    page_path: &str,
    tick_schema: &Schema,
) -> Result<(String, PageMeta), Box<dyn std::error::Error>> {
    let title = endpoint_title(endpoint);
    let description = endpoint.description.replace('\n', " ");

    let mut out = String::new();
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "title: {title}");
    let _ = writeln!(out, "description: \"{}\"", description.replace('"', "\\\""));
    let _ = writeln!(out, "---");
    out.push_str(
        "\n<!-- @generated by `generate_docs_site` from endpoint_surface.toml + tick_schema.toml. Do not edit by hand. -->\n\n",
    );
    let _ = writeln!(out, "# {title}\n");
    let _ = writeln!(out, "<TierBadge tier=\"{tier}\" />\n");
    let _ = writeln!(out, "{description}");

    if let Some(vendor) = endpoint.vendor_docstring.as_deref() {
        let rewritten = rewrite_vendor_docstring(vendor);
        if !rewritten.is_empty() {
            let _ = writeln!(out, "\n{rewritten}");
        }
    }

    out.push_str("\n<SdkTabs>\n\n");
    out.push_str(&tab(
        "rust",
        lang::rust_signature(endpoint),
        lang::rust_example(endpoint),
    ));
    out.push('\n');
    out.push_str(&tab(
        "python",
        lang::python_signature(endpoint),
        lang::python_example(endpoint),
    ));
    out.push('\n');
    out.push_str(&tab(
        "typescript",
        lang::typescript_signature(endpoint),
        lang::typescript_example(endpoint),
    ));
    out.push('\n');
    out.push_str(&tab(
        "cpp",
        lang::cpp_signature(endpoint),
        lang::cpp_example(endpoint),
    ));
    out.push('\n');
    out.push_str(&tab(
        "http",
        lang::http_signature(endpoint),
        lang::http_example(endpoint),
    ));
    out.push_str("\n</SdkTabs>\n\n");

    out.push_str(&render_params_section(endpoint));
    out.push_str(&response::render_response_section(endpoint, tick_schema)?);
    if let Some(sample) = response::render_sample_section(endpoint, tick_schema)? {
        out.push_str(&sample);
    }

    let meta = PageMeta {
        category: endpoint.category.clone(),
        subcategory: endpoint.subcategory.clone(),
        title,
        link: format!("/{page_path}"),
    };
    Ok((out, meta))
}

// ───────────────────────── Sidebar manifest ─────────────────────────────────

fn category_label(category: &str) -> &'static str {
    match category {
        "stock" => "Stock",
        "option" => "Option",
        "index" => "Index",
        "calendar" => "Calendar",
        "rate" => "Interest Rate",
        other => panic!("no sidebar label for category {other}"),
    }
}

fn subcategory_label(subcategory: &str) -> &'static str {
    match subcategory {
        "list" => "List",
        "snapshot" | "snapshot_greeks" => "Snapshot",
        "history" | "history_greeks" | "history_trade_greeks" => "History",
        "at_time" => "At-Time",
        other => panic!("no sidebar label for subcategory {other}"),
    }
}

/// VitePress sidebar items for the reference tree, emitted as JSON and
/// imported by `config.ts`. Categories follow registry order; within a
/// category, subgroups follow first-appearance order.
pub(super) fn render_reference_sidebar(pages: &[PageMeta]) -> String {
    let mut json = String::from("[\n");
    let mut categories: Vec<&str> = Vec::new();
    for page in pages {
        if !categories.contains(&page.category.as_str()) {
            categories.push(&page.category);
        }
    }

    for (ci, category) in categories.iter().enumerate() {
        let in_category: Vec<&PageMeta> =
            pages.iter().filter(|p| &p.category == category).collect();
        let _ = writeln!(
            json,
            "  {{ \"text\": \"{}\", \"collapsed\": true, \"items\": [",
            category_label(category)
        );

        // Calendar / rate trees are flat — no subcategory grouping.
        let flat = matches!(*category, "calendar" | "rate");
        if flat {
            for (i, page) in in_category.iter().enumerate() {
                let comma = if i + 1 < in_category.len() { "," } else { "" };
                let _ = writeln!(
                    json,
                    "    {{ \"text\": \"{}\", \"link\": \"{}\" }}{comma}",
                    page.title, page.link
                );
            }
        } else {
            let mut groups: Vec<&str> = Vec::new();
            for page in &in_category {
                let label = subcategory_label(&page.subcategory);
                if !groups.contains(&label) {
                    groups.push(label);
                }
            }
            for (gi, group) in groups.iter().enumerate() {
                let in_group: Vec<&&PageMeta> = in_category
                    .iter()
                    .filter(|p| subcategory_label(&p.subcategory) == *group)
                    .collect();
                let _ = writeln!(
                    json,
                    "    {{ \"text\": \"{group}\", \"collapsed\": true, \"items\": ["
                );
                for (i, page) in in_group.iter().enumerate() {
                    let comma = if i + 1 < in_group.len() { "," } else { "" };
                    let _ = writeln!(
                        json,
                        "      {{ \"text\": \"{}\", \"link\": \"{}\" }}{comma}",
                        page.title, page.link
                    );
                }
                let comma = if gi + 1 < groups.len() { "," } else { "" };
                let _ = writeln!(json, "    ]}}{comma}");
            }
        }

        let comma = if ci + 1 < categories.len() { "," } else { "" };
        let _ = writeln!(json, "  ]}}{comma}");
    }
    json.push_str("]\n");
    json
}
