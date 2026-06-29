//! OpenAPI-example samples for the endpoint reference pages.
//!
//! Each endpoint's example-response rows are lifted from the upstream
//! OpenAPI spec's `application/json` example, so every page shows a real,
//! vendor-documented sample. Endpoints whose spec has no JSON example
//! (e.g. list endpoints) render the schema only — sample data is never
//! fabricated.

use std::sync::OnceLock;

use serde_json::Value;

const SAMPLE_ROWS: usize = 3;

/// Vendored upstream OpenAPI spec, relative to the generator's package-root
/// cwd. Read once and cached for the example lookups below.
fn openapi_text() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        std::fs::read_to_string("../scripts/ci/data/upstream_openapi.yaml").unwrap_or_default()
    })
}

/// Example response rows for `rest_path` (e.g. "/v3/option/history/ohlc"),
/// lifted from the upstream OpenAPI spec's `application/json` example so every
/// endpoint shows a real, vendor-documented sample. Flat (`response: [..]`)
/// and option (`response: [{contract, data: [..]}]`) shapes both reduce to the
/// record list; the first `SAMPLE_ROWS` are returned. Empty when the spec has
/// no JSON example (e.g. list endpoints), leaving the page sample-less.
pub(super) fn td_example_rows(rest_path: &str) -> Vec<Value> {
    let yaml_path = rest_path.strip_prefix("/v3").unwrap_or(rest_path);
    let Some(example) = extract_json_example(openapi_text(), yaml_path) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&example) else {
        return Vec::new();
    };
    let Some(rows) = parsed.get("response").and_then(Value::as_array) else {
        return Vec::new();
    };
    let records = rows
        .first()
        .and_then(|r| r.get("data"))
        .and_then(Value::as_array)
        .unwrap_or(rows);
    records.iter().take(SAMPLE_ROWS).cloned().collect()
}

/// Pull the first `application/json` `example: |` block under `path` from the
/// raw OpenAPI text. Line/indent-based to avoid a YAML dependency, mirroring
/// the existing upstream-spec parser. JSON ignores the residual indentation.
fn extract_json_example(text: &str, path: &str) -> Option<String> {
    let path_key = format!("{path}:");
    let lines: Vec<&str> = text.lines().collect();
    let indent_of = |l: &str| l.len() - l.trim_start().len();

    let start = lines
        .iter()
        .position(|l| indent_of(l) == 2 && l.trim_start() == path_key)?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| indent_of(l) == 2 && l.trim_start().starts_with('/'))
        .map_or(lines.len(), |p| start + 1 + p);
    let block = &lines[start..end];

    let json_idx = block.iter().position(|l| l.trim() == "application/json:")?;
    let ex_idx = json_idx
        + block[json_idx..]
            .iter()
            .position(|l| l.trim_start().starts_with("example:"))?;
    let ex_indent = indent_of(block[ex_idx]);

    let mut out = String::new();
    for line in &block[ex_idx + 1..] {
        if !line.trim().is_empty() && indent_of(line) <= ex_indent {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    (!out.trim().is_empty()).then_some(out)
}
