//! Per-language signature and runnable-sample renderers for the
//! endpoint reference pages.
//!
//! One renderer per tab: Rust, Python, TypeScript, C++, HTTP. Each
//! derives from the same `GeneratedEndpoint` model the SDK projection
//! emitters consume, so a registry change reshapes every tab on the
//! next generator run.

use std::fmt::Write as _;

use super::super::helpers::to_pascal_case;
use super::super::model::{GeneratedEndpoint, GeneratedParam};
use super::super::sdk_helpers::{
    builder_params, is_snapshot_endpoint, method_params, sdk_method_arg_name, to_camel_case,
};
use super::response::{display_fields, schema_type_name};

// ───────────────────────── Sample fixture values ────────────────────────────
//
// Mirrors `[test_fixtures]` in `endpoint_surface.toml`: the same values the
// generated live validators exercise, so every sample is a request the
// production server is known to answer.

fn sample_value(param: &GeneratedParam, category: &str) -> &'static str {
    match param.param_type.as_str() {
        "Symbol" | "Symbols" => match category {
            "stock" => "AAPL",
            "option" => "SPY",
            "index" => "SPX",
            "rate" => "SOFR",
            other => panic!("no sample symbol for category {other}"),
        },
        "Date" => match param.name.as_str() {
            "start_date" => "20250303",
            "end_date" => "20250306",
            _ => "20250303",
        },
        "Expiration" => "20250321",
        "Strike" => "570",
        "Right" => "C",
        "Interval" => "1m",
        "RequestType" => "trade",
        "Year" => "2025",
        "Str" if param.name == "time_of_day" => "10:30:00.000",
        "Str" => "10:30:00",
        other => panic!("no sample value for param type {other}"),
    }
}

/// Builder params showcased in the runnable samples. Pinning `strike` +
/// `right` turns the wildcard default into the canonical single-contract
/// request; `interval` is the one tuning knob most requests set.
fn showcased_builder_params(endpoint: &GeneratedEndpoint) -> Vec<&GeneratedParam> {
    builder_params(endpoint)
        .into_iter()
        .filter(|p| matches!(p.name.as_str(), "strike" | "right" | "interval"))
        .collect()
}

// ───────────────────────── Shared return-shape helpers ──────────────────────

fn rust_item_type(endpoint: &GeneratedEndpoint) -> String {
    match endpoint.return_type.as_str() {
        "StringList" => "String".into(),
        "OptionContracts" => "OptionContract".into(),
        "CalendarDays" => "CalendarDay".into(),
        other => schema_type_name(other),
    }
}

fn python_return_type(endpoint: &GeneratedEndpoint) -> String {
    let item = rust_item_type(endpoint);
    if endpoint.return_type == "StringList" {
        "StringList".into()
    } else if is_snapshot_endpoint(endpoint) {
        format!("list[{item}]")
    } else {
        format!("{item}List")
    }
}

// ───────────────────────── Rust ─────────────────────────────────────────────

/// Renders the Rust signature block for an endpoint: the fenced method
/// signature plus prose describing the builder setters and execution.
pub(super) fn rust_signature(endpoint: &GeneratedEndpoint) -> String {
    let item = rust_item_type(endpoint);
    let required = method_params(endpoint);
    let args: Vec<String> = required
        .iter()
        .map(|p| {
            let name = sdk_method_arg_name(p);
            if p.param_type == "Symbols" {
                format!("{name}: &[&str]")
            } else {
                format!("{name}: &str")
            }
        })
        .collect();

    let builder = format!("{}Builder", to_pascal_case(&endpoint.name));
    let mut sig = String::from("```rust\n");
    if args.len() <= 2 {
        let _ = write!(
            sig,
            "pub fn {}(&self{}{}) -> {builder}<'_>",
            endpoint.name,
            if args.is_empty() { "" } else { ", " },
            args.join(", ")
        );
    } else {
        let _ = writeln!(sig, "pub fn {}(", endpoint.name);
        sig.push_str("    &self,\n");
        for arg in &args {
            let _ = writeln!(sig, "    {arg},");
        }
        let _ = write!(sig, ") -> {builder}<'_>");
    }
    sig.push_str("\n```\n");

    let opts = builder_params(endpoint);
    let mut prose = String::new();
    if opts.is_empty() {
        let _ = write!(
            prose,
            "Execute with `.await` → `Result<Vec<{item}>, Error>`"
        );
    } else {
        let setters: Vec<String> = opts
            .iter()
            .map(|p| format!("`.{}({})`", p.name, rust_setter_arg(p)))
            .collect();
        let _ = write!(
            prose,
            "Optional parameters chain on the builder: {}. Execute with `.await` → `Result<Vec<{item}>, Error>`",
            setters.join(", ")
        );
    }
    if endpoint.kind == "parsed" && endpoint.return_type != "StringList" {
        prose.push_str(", or decode chunk-by-chunk with `.stream(handler)`.");
    } else {
        prose.push('.');
    }
    format!("{sig}\n{prose}\n")
}

fn rust_setter_arg(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "i32",
        "Float" => "f64",
        "Bool" => "bool",
        _ => "&str",
    }
}

/// Renders a runnable Rust sample for an endpoint, calling the method
/// with fixture argument values and printing each returned row.
pub(super) fn rust_example(endpoint: &GeneratedEndpoint) -> String {
    let required = method_params(endpoint);
    let showcased = showcased_builder_params(endpoint);
    let args: Vec<String> = required
        .iter()
        .map(|p| {
            let value = sample_value(p, &endpoint.category);
            if p.param_type == "Symbols" {
                format!("&[\"{value}\"]")
            } else {
                format!("\"{value}\"")
            }
        })
        .collect();

    let mut code = String::new();
    if showcased.is_empty() {
        let _ = writeln!(
            code,
            "let rows = tdx.{}({}).await?;",
            endpoint.name,
            args.join(", ")
        );
    } else {
        let _ = writeln!(code, "let rows = tdx");
        let _ = writeln!(code, "    .{}({})", endpoint.name, args.join(", "));
        for p in &showcased {
            let _ = writeln!(
                code,
                "    .{}(\"{}\")",
                p.name,
                sample_value(p, &endpoint.category)
            );
        }
        code.push_str("    .await?;\n");
    }

    if endpoint.return_type == "StringList" {
        code.push_str("for value in &rows {\n    println!(\"{value}\");\n}");
    } else {
        let fields = display_fields(&endpoint.return_type);
        let fmt: Vec<String> = fields.iter().map(|f| format!("{f}={{}}")).collect();
        let vals: Vec<String> = fields.iter().map(|f| format!("t.{f}")).collect();
        let _ = write!(
            code,
            "for t in &rows {{\n    println!(\"{}\", {});\n}}",
            fmt.join(" "),
            vals.join(", ")
        );
    }
    format!("```rust\n{code}\n```\n")
}

// ───────────────────────── Python ───────────────────────────────────────────

/// Renders the Python signature block for an endpoint: the fenced
/// method signature plus prose covering the async variant and DataFrame
/// converters.
pub(super) fn python_signature(endpoint: &GeneratedEndpoint) -> String {
    let required = method_params(endpoint);
    let opts = builder_params(endpoint);
    let ret = python_return_type(endpoint);

    let req: Vec<String> = required.iter().map(|p| sdk_method_arg_name(p)).collect();
    let mut kw: Vec<String> = opts.iter().map(|p| format!("{}=None", p.name)).collect();
    kw.push("timeout_ms=None".into());

    let mut sig = String::from("```python\n");
    let req_prefix = if req.is_empty() {
        String::new()
    } else {
        format!("{}, ", req.join(", "))
    };
    let one_line = format!(
        "Client.{}({req_prefix}*, {}) -> {ret}",
        endpoint.name,
        kw.join(", ")
    );
    if one_line.len() <= 88 {
        sig.push_str(&one_line);
    } else {
        let _ = writeln!(sig, "Client.{}(", endpoint.name);
        if !req.is_empty() {
            let _ = writeln!(sig, "    {},", req.join(", "));
        }
        sig.push_str("    *,\n");
        let mut line = String::from("    ");
        for (i, item) in kw.iter().enumerate() {
            if line.len() + item.len() > 80 {
                let _ = writeln!(sig, "{line}");
                line = String::from("    ");
            }
            line.push_str(item);
            if i + 1 < kw.len() {
                line.push_str(", ");
            } else {
                line.push(',');
            }
        }
        let _ = writeln!(sig, "{line}");
        let _ = write!(sig, ") -> {ret}");
    }
    sig.push_str("\n```\n");

    let mut prose = format!("`{}_async(...)` awaits the same call shape.", endpoint.name);
    if !is_snapshot_endpoint(endpoint) && endpoint.return_type != "StringList" {
        prose.push_str(
            " Chain `.to_pandas()` / `.to_polars()` / `.to_arrow()` / `.to_list()` on the result.",
        );
    }
    format!("{sig}\n{prose}\n")
}

/// Renders a runnable Python sample for an endpoint, calling the method
/// with fixture argument values and printing each returned row.
pub(super) fn python_example(endpoint: &GeneratedEndpoint) -> String {
    let required = method_params(endpoint);
    let showcased = showcased_builder_params(endpoint);
    let mut args: Vec<String> = required
        .iter()
        .map(|p| {
            let value = sample_value(p, &endpoint.category);
            if p.param_type == "Symbols" {
                format!("[\"{value}\"]")
            } else {
                format!("\"{value}\"")
            }
        })
        .collect();
    for p in &showcased {
        args.push(format!(
            "{}=\"{}\"",
            p.name,
            sample_value(p, &endpoint.category)
        ));
    }

    let mut code = format!("rows = tdx.{}({})\n", endpoint.name, args.join(", "));
    if code.len() > 90 {
        code = format!(
            "rows = tdx.{}(\n    {},\n)\n",
            endpoint.name,
            args.join(",\n    ")
        );
    }
    if endpoint.return_type == "StringList" {
        code.push_str("for value in rows:\n    print(value)");
    } else {
        let fields = display_fields(&endpoint.return_type);
        let vals: Vec<String> = fields.iter().map(|f| format!("t.{f}")).collect();
        let _ = write!(code, "for t in rows:\n    print({})", vals.join(", "));
    }
    format!("```python\n{code}\n```\n")
}

// ───────────────────────── TypeScript ───────────────────────────────────────

fn ts_param_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "number",
        "Float" => "number",
        "Bool" => "boolean",
        "Date" | "Expiration" => "string | Date",
        _ if matches!(
            param.name.as_str(),
            "start_time" | "end_time" | "min_time" | "time_of_day"
        ) =>
        {
            "string | Date"
        }
        _ => "string",
    }
}

fn ts_return_type(endpoint: &GeneratedEndpoint) -> String {
    // Every endpoint method resolves off the runtime's execution thread,
    // so the surface is a Promise; the element type is unchanged.
    if endpoint.return_type == "StringList" {
        "Promise<Array<string>>".into()
    } else {
        format!("Promise<Array<{}>>", rust_item_type(endpoint))
    }
}

/// Renders the TypeScript signature block for an endpoint: the fenced
/// method signature plus the trailing options-object key list.
pub(super) fn typescript_signature(endpoint: &GeneratedEndpoint) -> String {
    let required = method_params(endpoint);
    let opts = builder_params(endpoint);
    let method = to_camel_case(&endpoint.name);
    let ret = ts_return_type(endpoint);

    let mut args: Vec<String> = required
        .iter()
        .map(|p| {
            let name = to_camel_case(&sdk_method_arg_name(p));
            if p.param_type == "Symbols" {
                format!("{name}: Array<string>")
            } else {
                format!("{name}: {}", ts_param_type(p))
            }
        })
        .collect();
    args.push("options?: { ... }".into());

    let mut sig = String::from("```typescript\n");
    let one_line = format!("{method}({}): {ret}", args.join(", "));
    if one_line.len() <= 88 {
        sig.push_str(&one_line);
    } else {
        let _ = writeln!(sig, "{method}(");
        let mut line = String::from("  ");
        for (i, item) in args.iter().enumerate() {
            if line.len() + item.len() > 80 {
                let _ = writeln!(sig, "{line}");
                line = String::from("  ");
            }
            line.push_str(item);
            if i + 1 < args.len() {
                line.push_str(", ");
            } else {
                line.push(',');
            }
        }
        let _ = writeln!(sig, "{line}");
        let _ = write!(sig, "): {ret}");
    }
    sig.push_str("\n```\n");

    // Options-object key list: the camelCase optional parameter names
    // plus the universal timeoutMs deadline key.
    let mut keys: Vec<String> = opts
        .iter()
        .map(|p| format!("`{}?: {}`", to_camel_case(&p.name), ts_param_type(p)))
        .collect();
    keys.push("`timeoutMs?: number`".into());
    format!(
        "{sig}\nOptional parameters ride in a single trailing options object: {}.\n",
        keys.join(", ")
    )
}

/// Renders a runnable TypeScript sample for an endpoint, awaiting the
/// method with fixture argument values and logging each returned row.
pub(super) fn typescript_example(endpoint: &GeneratedEndpoint) -> String {
    let required = method_params(endpoint);
    let showcased = showcased_builder_params(endpoint);
    let method = to_camel_case(&endpoint.name);

    let mut args: Vec<String> = required
        .iter()
        .map(|p| {
            let value = sample_value(p, &endpoint.category);
            if p.param_type == "Symbols" {
                format!("['{value}']")
            } else {
                format!("'{value}'")
            }
        })
        .collect();

    // Showcased optionals ride in the trailing options object under
    // their camelCase keys.
    if !showcased.is_empty() {
        let entries = showcased
            .iter()
            .map(|p| {
                format!(
                    "{}: '{}'",
                    to_camel_case(&p.name),
                    sample_value(p, &endpoint.category)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        args.push(format!("{{ {entries} }}"));
    }

    // The method returns a Promise resolved off the execution thread, so
    // the sample awaits it — the idiomatic JavaScript shape for a fetch.
    let mut code = format!("const rows = await tdx.{method}({});\n", args.join(", "));
    if endpoint.return_type == "StringList" {
        code.push_str("for (const value of rows) {\n  console.log(value);\n}");
    } else {
        let fields = display_fields(&endpoint.return_type);
        let vals: Vec<String> = fields
            .iter()
            .map(|f| format!("t.{}", to_camel_case(f)))
            .collect();
        let _ = write!(
            code,
            "for (const t of rows) {{\n  console.log({});\n}}",
            vals.join(", ")
        );
    }
    format!("```typescript\n{code}\n```\n")
}

// ───────────────────────── C++ ──────────────────────────────────────────────

fn cpp_return_type(endpoint: &GeneratedEndpoint) -> String {
    if endpoint.return_type == "StringList" {
        "std::vector<std::string>".into()
    } else {
        format!("std::vector<{}>", rust_item_type(endpoint))
    }
}

/// Renders the C++ signature block for an endpoint: the fenced method
/// declaration plus prose covering the option setters and error model.
pub(super) fn cpp_signature(endpoint: &GeneratedEndpoint) -> String {
    let required = method_params(endpoint);
    let ret = cpp_return_type(endpoint);

    let mut args: Vec<String> = required
        .iter()
        .map(|p| {
            let name = sdk_method_arg_name(p);
            if p.param_type == "Symbols" {
                format!("const std::vector<std::string>& {name}")
            } else {
                format!("const std::string& {name}")
            }
        })
        .collect();
    args.push("const EndpointRequestOptions& options = {}".into());

    let mut sig = String::from("```cpp\n");
    let one_line = format!("{ret} {}({}) const;", endpoint.name, args.join(", "));
    if one_line.len() <= 88 {
        sig.push_str(&one_line);
    } else {
        let _ = writeln!(sig, "{ret} {}(", endpoint.name);
        for (i, arg) in args.iter().enumerate() {
            let sep = if i + 1 < args.len() { "," } else { ") const;" };
            let _ = writeln!(sig, "    {arg}{sep}");
        }
        sig.pop();
    }
    sig.push_str("\n```\n");

    let opts = builder_params(endpoint);
    let prose = if opts.is_empty() {
        "Throws `thetadatadx::Error` on failure.".to_string()
    } else {
        let setters: Vec<String> = opts
            .iter()
            .map(|p| format!("`.with_{}(...)`", p.name))
            .collect();
        format!(
            "Optional parameters chain on `EndpointRequestOptions`: {}. Throws `thetadatadx::Error` on failure.",
            setters.join(", ")
        )
    };
    format!("{sig}\n{prose}\n")
}

/// Renders a runnable C++ sample for an endpoint, calling the method
/// with fixture argument values and streaming each returned row.
pub(super) fn cpp_example(endpoint: &GeneratedEndpoint) -> String {
    let required = method_params(endpoint);
    let showcased = showcased_builder_params(endpoint);

    let mut args: Vec<String> = required
        .iter()
        .map(|p| {
            let value = sample_value(p, &endpoint.category);
            if p.param_type == "Symbols" {
                format!("{{\"{value}\"}}")
            } else {
                format!("\"{value}\"")
            }
        })
        .collect();
    if !showcased.is_empty() {
        let setters: Vec<String> = showcased
            .iter()
            .map(|p| {
                format!(
                    ".with_{}(\"{}\")",
                    p.name,
                    sample_value(p, &endpoint.category)
                )
            })
            .collect();
        args.push(format!(
            "\n    thetadatadx::EndpointRequestOptions{{}}{}",
            setters.join("")
        ));
    }

    let mut code = format!(
        "auto rows = client.{}({});\n",
        endpoint.name,
        args.join(", ")
    );
    if endpoint.return_type == "StringList" {
        code.push_str("for (const auto& value : rows) {\n    std::cout << value << \"\\n\";\n}");
    } else {
        let fields = display_fields(&endpoint.return_type);
        let stream: Vec<String> = fields.iter().map(|f| format!("t.{f}")).collect();
        let _ = write!(
            code,
            "for (const auto& t : rows) {{\n    std::cout << {} << \"\\n\";\n}}",
            stream.join(" << ' ' << ")
        );
    }
    format!("```cpp\n{code}\n```\n")
}

// ───────────────────────── HTTP ─────────────────────────────────────────────

/// ISO form of a YYYYMMDD fixture date for HTTP samples.
fn iso_date(value: &str) -> String {
    if value.len() == 8 && value.chars().all(|c| c.is_ascii_digit()) {
        format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..8])
    } else {
        value.to_string()
    }
}

fn http_sample_value(param: &GeneratedParam, category: &str) -> String {
    let raw = sample_value(param, category);
    match param.param_type.as_str() {
        "Date" | "Expiration" => iso_date(raw),
        _ => raw.to_string(),
    }
}

/// Renders the HTTP signature block for an endpoint: the `GET` line for
/// the REST path plus a note on the server binary and response formats.
pub(super) fn http_signature(endpoint: &GeneratedEndpoint) -> String {
    format!(
        "```http\nGET http://127.0.0.1:25503{}\n```\n\nServed by the bundled [server binary](/server/); responses stream as JSON, CSV, or NDJSON via the `format` parameter.\n",
        endpoint._rest_path
    )
}

/// Renders a runnable `curl` sample for an endpoint, building the query
/// string from fixture argument values.
pub(super) fn http_example(endpoint: &GeneratedEndpoint) -> String {
    let required = method_params(endpoint);
    let showcased = showcased_builder_params(endpoint);

    let mut params: Vec<(String, String)> = required
        .iter()
        .map(|p| (p.name.clone(), http_sample_value(p, &endpoint.category)))
        .collect();
    for p in &showcased {
        params.push((p.name.clone(), http_sample_value(p, &endpoint.category)));
    }

    let url = format!("http://127.0.0.1:25503{}", endpoint._rest_path);
    let code = if params.len() <= 3 {
        let query: Vec<String> = params.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if query.is_empty() {
            format!("curl '{url}'")
        } else {
            format!("curl '{url}?{}'", query.join("&"))
        }
    } else {
        let mut lines = vec![format!("curl -G '{url}' \\")];
        for (i, (k, v)) in params.iter().enumerate() {
            let cont = if i + 1 < params.len() { " \\" } else { "" };
            lines.push(format!("    --data-urlencode '{k}={v}'{cont}"));
        }
        lines.join("\n")
    };
    format!("```bash\n{code}\n```\n")
}
