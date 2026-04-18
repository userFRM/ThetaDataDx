//! FPSS streaming event schema → per-language typed struct generator.
//!
//! Reads `fpss_event_schema.toml` and emits:
//!
//! - `sdks/python/src/fpss_event_classes.rs` — `#[pyclass]` per Data variant
//!   + `buffered_event_to_typed` dispatcher + `register_fpss_event_classes`.
//! - `sdks/typescript/src/fpss_event_classes.rs` — `#[napi(object)]` per
//!   Data variant + a flat `FpssEvent` wrapper struct with optional typed
//!   payload fields + a `buffered_event_to_typed` dispatcher that converts
//!   an internal `BufferedEvent` into one. Control/simple/raw-data events
//!   fall through to string-tagged variants so TypeScript consumers can
//!   discriminate without losing the diagnostic payload.
//!
//! Same pattern the tick generator uses for MDDS (`build_support/ticks.rs`).
//! The TS target deliberately produces a flat discriminated struct (one
//! `kind` tag plus optional per-variant payloads) rather than a serde-tagged
//! napi enum: napi-rs 3.x's enum-over-object support is still rough around
//! union lowering, while `#[napi(object)]` structs with `Option<T>` payloads
//! lower to a clean TypeScript union with full IntelliSense.

#![allow(dead_code, unused_imports)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

use super::ticks::GeneratedSourceFile;

#[derive(Debug, Deserialize)]
struct Schema {
    #[serde(default)]
    version: u32,
    events: HashMap<String, EventDef>,
}

#[derive(Debug, Deserialize)]
struct EventDef {
    #[serde(default)]
    doc: String,
    columns: Vec<ColumnDef>,
}

#[derive(Debug, Deserialize)]
struct ColumnDef {
    name: String,
    r#type: String,
}

fn load_schema() -> Result<Schema, Box<dyn std::error::Error>> {
    let schema_str = std::fs::read_to_string("fpss_event_schema.toml")?;
    let schema: Schema = toml::from_str(&schema_str)?;
    Ok(schema)
}

fn sorted_event_names(schema: &Schema) -> Vec<&str> {
    let mut names = schema.events.keys().map(String::as_str).collect::<Vec<_>>();
    names.sort();
    names
}

/// Convert a PascalCase event name to snake_case ("OpenInterest" → "open_interest")
/// so the `kind` discriminator exposed to Python matches the wire tag the
/// existing dict-based `next_event` emits.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn snake_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = false;
    for ch in s.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.push(ch.to_ascii_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn python_rust_field_type(column_type: &str, event_name: &str, column_name: &str) -> &'static str {
    match column_type {
        "i32" => "i32",
        "i64" => "i64",
        "u64" => "u64",
        "f64" => "f64",
        other => {
            panic!("unsupported FPSS event column type '{other}' in {event_name}.{column_name}")
        }
    }
}

fn ts_rust_field_type(column_type: &str, event_name: &str, column_name: &str) -> &'static str {
    match column_type {
        "i32" => "i32",
        // Epoch nanoseconds today are ~1.8e18, well past
        // `Number.MAX_SAFE_INTEGER` (2^53 - 1 ≈ 9e15). `i64`/`u64` fields
        // must cross to JS as `bigint` to avoid silent precision loss.
        // `napi::bindgen_prelude::BigInt` is how napi-rs 3.x exposes that.
        "u64" | "i64" => "BigInt",
        "f64" => "f64",
        other => {
            panic!("unsupported FPSS event column type '{other}' in {event_name}.{column_name}")
        }
    }
}

pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files()? {
        let abs = repo_root.join(file.relative_path);
        std::fs::write(&abs, &file.contents)?;
    }
    Ok(())
}

pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files()? {
        let abs = repo_root.join(file.relative_path);
        let current = std::fs::read_to_string(&abs).unwrap_or_default();
        if current.replace("\r\n", "\n") != file.contents {
            return Err(format!(
                "generated file out of date: {} (run: cargo run -p thetadatadx --bin generate_sdk_surfaces)",
                file.relative_path
            )
            .into());
        }
    }
    Ok(())
}

fn render_sdk_generated_files() -> Result<Vec<GeneratedSourceFile>, Box<dyn std::error::Error>> {
    let schema = load_schema()?;
    Ok(vec![
        GeneratedSourceFile {
            relative_path: "sdks/python/src/fpss_event_classes.rs",
            contents: render_python_fpss_event_classes(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/fpss_event_classes.rs",
            contents: render_ts_fpss_event_classes(&schema),
        },
    ])
}

// ── Python emitter ──────────────────────────────────────────────────────────

fn render_python_fpss_event_classes(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from fpss_event_schema.toml\n",
    );
    out.push_str("// Typed #[pyclass] structs for every `FpssData` variant + the dispatch\n");
    out.push_str("// helper that converts a `BufferedEvent` into one of them. The single\n");
    out.push_str("// source of truth is `crates/thetadatadx/fpss_event_schema.toml`.\n\n");

    let names = sorted_event_names(schema);

    // Structs.
    for event_name in &names {
        let def = &schema.events[*event_name];
        out.push_str(&render_python_event_class_struct(event_name, def));
        out.push('\n');
    }

    // Dispatcher: BufferedEvent -> typed pyclass (non-tick variants fall back to dict).
    out.push_str("pub(crate) fn buffered_event_to_typed(\n");
    out.push_str("    py: Python<'_>,\n");
    out.push_str("    event: &BufferedEvent,\n");
    out.push_str(") -> PyResult<Py<PyAny>> {\n");
    out.push_str("    match event {\n");
    for event_name in &names {
        let def = &schema.events[*event_name];
        out.push_str(&render_python_buffered_match_arm(event_name, def));
    }
    out.push_str("        _ => Ok(buffered_event_to_py(py, event)),\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    // Module registration helper.
    out.push_str(
        "pub(crate) fn register_fpss_event_classes(m: &Bound<'_, PyModule>) -> PyResult<()> {\n",
    );
    for event_name in &names {
        writeln!(out, "    m.add_class::<{}>()?;", event_name).unwrap();
    }
    out.push_str("    Ok(())\n");
    out.push_str("}\n");

    out
}

fn render_python_event_class_struct(event_name: &str, def: &EventDef) -> String {
    let mut out = String::new();
    let doc_text = if def.doc.is_empty() {
        format!("FPSS {event_name} event.")
    } else {
        def.doc.clone()
    };
    for line in doc_text.lines() {
        writeln!(out, "/// {line}").unwrap();
    }
    out.push_str("#[pyclass(module = \"thetadatadx\", frozen)]\n");
    out.push_str("#[derive(Clone)]\n");
    writeln!(out, "pub(crate) struct {event_name} {{").unwrap();
    for column in &def.columns {
        let ty = python_rust_field_type(&column.r#type, event_name, &column.name);
        writeln!(out, "    #[pyo3(get)] pub {}: {},", column.name, ty).unwrap();
    }
    out.push_str("}\n");
    writeln!(out, "#[pymethods]").unwrap();
    writeln!(out, "impl {event_name} {{").unwrap();
    writeln!(
        out,
        "    fn __repr__(&self) -> String {{ format!(\"{event_name}(...)\") }}\n"
    )
    .unwrap();
    // Use the same lowercase/snake_case wire tag the existing dict-based
    // `next_event` emits ("quote", "trade", "open_interest", "ohlcvc") so
    // `next_event_typed` is a true drop-in — consumer `event.kind` checks
    // don't need to branch on which API they called.
    let kind_tag = snake_case(event_name);
    writeln!(out, "    #[getter]").unwrap();
    writeln!(
        out,
        "    fn kind(&self) -> &'static str {{ \"{kind_tag}\" }}"
    )
    .unwrap();
    out.push_str("}\n");
    out
}

fn render_python_buffered_match_arm(event_name: &str, def: &EventDef) -> String {
    let mut out = String::new();
    // Pattern binds each schema field to a same-named local.
    writeln!(out, "        BufferedEvent::{event_name} {{").unwrap();
    for column in &def.columns {
        writeln!(out, "            {},", column.name).unwrap();
    }
    out.push_str("            ..\n        } => Py::new(\n");
    out.push_str("            py,\n");
    writeln!(out, "            {event_name} {{").unwrap();
    for column in &def.columns {
        writeln!(
            out,
            "                {field}: *{field},",
            field = column.name
        )
        .unwrap();
    }
    out.push_str("            },\n        )\n        .map(|p| p.into_any()),\n");
    out
}

// ── TypeScript emitter ──────────────────────────────────────────────────────

fn render_ts_fpss_event_classes(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from fpss_event_schema.toml\n",
    );
    out.push_str("// Typed #[napi(object)] structs for every `FpssData` variant + a flat\n");
    out.push_str("// `FpssEvent` wrapper and the `buffered_event_to_typed` dispatcher.\n");
    out.push_str("// Consumers discriminate on `event.kind` and pull the matching optional\n");
    out.push_str("// payload field. Control / raw / simple events expose their diagnostic\n");
    out.push_str("// strings via `control`, `raw_data`, and `simple` payloads.\n\n");
    out.push_str("use napi::bindgen_prelude::BigInt;\n\n");

    let names = sorted_event_names(schema);

    // Typed data payload structs.
    for event_name in &names {
        let def = &schema.events[*event_name];
        out.push_str(&render_ts_event_class_struct(event_name, def));
        out.push('\n');
    }

    // Simple / diagnostic payloads (control, raw_data, simple).
    out.push_str("/// FPSS control / diagnostic payload (login, disconnect, market open, ...).\n");
    out.push_str("#[napi(object)]\n");
    out.push_str("#[derive(Clone)]\n");
    out.push_str("pub struct FpssControlPayload {\n");
    out.push_str(
        "    /// Concrete control event kind (e.g. \"login_success\", \"disconnected\").\n",
    );
    out.push_str("    pub event_type: String,\n");
    out.push_str("    /// Free-form diagnostic detail; empty when the event carries no payload.\n");
    out.push_str("    pub detail: Option<String>,\n");
    out.push_str(
        "    /// Optional event id (req_id for ReqResponse, contract id for ContractAssigned).\n",
    );
    out.push_str("    pub id: Option<i32>,\n");
    out.push_str("}\n\n");

    out.push_str("/// FPSS raw-bytes payload for frames the decoder did not recognise.\n");
    out.push_str("#[napi(object)]\n");
    out.push_str("#[derive(Clone)]\n");
    out.push_str("pub struct FpssRawDataPayload {\n");
    out.push_str("    pub code: u32,\n");
    out.push_str("    pub payload: Vec<u8>,\n");
    out.push_str("}\n\n");

    // Flat wrapper struct with `kind` tag + optional payloads.
    out.push_str("/// A single FPSS event surfaced to JS/TS.\n");
    out.push_str("///\n");
    out.push_str("/// `kind` is the discriminator — switch on it and read the matching\n");
    out.push_str("/// payload field. The shape is stable and every payload is typed, so\n");
    out.push_str("/// consumers never fall back to untyped `any`.\n");
    out.push_str("#[napi(object)]\n");
    out.push_str("#[derive(Clone)]\n");
    out.push_str("pub struct FpssEvent {\n");
    // Emit the `kind` literal-union discriminator from the same schema
    // path as the method-level `ts_return_type` union, so the struct
    // definition and `nextEvent()`'s narrowing shape never diverge when
    // a new variant is added to `fpss_event_schema.toml`.
    let mut kind_tags: Vec<String> = names
        .iter()
        .map(|n| format!("'{}'", snake_case(n)))
        .collect();
    kind_tags.push("'control'".to_string());
    kind_tags.push("'raw_data'".to_string());
    let kind_union = kind_tags.join(" | ");
    out.push_str("    /// Discriminator matching one of the typed payload fields below.\n");
    out.push_str("    /// Narrowed to a literal union in TS so `switch (event.kind)`\n");
    out.push_str("    /// correctly narrows the optional payload fields.\n");
    writeln!(out, "    #[napi(ts_type = \"{kind_union}\")]").unwrap();
    out.push_str("    pub kind: String,\n");
    for event_name in &names {
        let field = snake_case(event_name);
        writeln!(
            out,
            "    pub {field}: Option<{event_name}>,",
            event_name = event_name
        )
        .unwrap();
    }
    out.push_str("    pub control: Option<FpssControlPayload>,\n");
    out.push_str("    pub raw_data: Option<FpssRawDataPayload>,\n");
    out.push_str("}\n\n");

    // Dispatcher.
    out.push_str("pub(crate) fn buffered_event_to_typed(event: BufferedEvent) -> FpssEvent {\n");
    out.push_str("    let mut out = FpssEvent {\n");
    out.push_str("        kind: String::new(),\n");
    for event_name in &names {
        let field = snake_case(event_name);
        writeln!(out, "        {field}: None,").unwrap();
    }
    out.push_str("        control: None,\n");
    out.push_str("        raw_data: None,\n");
    out.push_str("    };\n");
    out.push_str("    match event {\n");
    for event_name in &names {
        let def = &schema.events[*event_name];
        let field = snake_case(event_name);
        let kind_tag = snake_case(event_name);
        writeln!(out, "        BufferedEvent::{event_name} {{").unwrap();
        for column in &def.columns {
            writeln!(out, "            {},", column.name).unwrap();
        }
        out.push_str("            ..\n        } => {\n");
        writeln!(out, "            out.kind = \"{kind_tag}\".to_string();").unwrap();
        writeln!(out, "            out.{field} = Some({event_name} {{").unwrap();
        for column in &def.columns {
            // u64/i64 wire columns cross to JS as `bigint` to preserve
            // full precision. Nanosecond timestamps today already exceed
            // `Number.MAX_SAFE_INTEGER` by three orders of magnitude, so
            // `number` is not an option.
            let rhs = match column.r#type.as_str() {
                "u64" | "i64" => format!("BigInt::from({name})", name = column.name),
                _ => column.name.clone(),
            };
            writeln!(
                out,
                "                {name}: {rhs},",
                name = column.name,
                rhs = rhs
            )
            .unwrap();
        }
        out.push_str("            });\n        }\n");
    }
    out.push_str("        BufferedEvent::RawData { code, payload } => {\n");
    out.push_str("            out.kind = \"raw_data\".to_string();\n");
    out.push_str("            out.raw_data = Some(FpssRawDataPayload {\n");
    out.push_str("                code: code as u32,\n");
    out.push_str("                payload,\n");
    out.push_str("            });\n");
    out.push_str("        }\n");
    out.push_str("        BufferedEvent::Simple { event_type, detail, id } => {\n");
    out.push_str("            out.kind = \"control\".to_string();\n");
    out.push_str("            out.control = Some(FpssControlPayload {\n");
    out.push_str("                event_type,\n");
    out.push_str("                detail,\n");
    out.push_str("                id,\n");
    out.push_str("            });\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("    out\n");
    out.push_str("}\n");

    out
}

fn render_ts_event_class_struct(event_name: &str, def: &EventDef) -> String {
    let mut out = String::new();
    let doc_text = if def.doc.is_empty() {
        format!("FPSS {event_name} event.")
    } else {
        def.doc.clone()
    };
    for line in doc_text.lines() {
        writeln!(out, "/// {line}").unwrap();
    }
    out.push_str("#[napi(object)]\n");
    out.push_str("#[derive(Clone)]\n");
    writeln!(out, "pub struct {event_name} {{").unwrap();
    for column in &def.columns {
        let ty = ts_rust_field_type(&column.r#type, event_name, &column.name);
        writeln!(out, "    pub {}: {},", column.name, ty).unwrap();
    }
    out.push_str("}\n");
    out
}

/// Render the discriminated-union TypeScript type literal used by the
/// `next_event` napi method's `#[napi(ts_return_type = ...)]` override.
///
/// The flat `FpssEvent` interface napi-rs emits does not narrow payloads
/// inside `switch (ev.kind)`, so we re-emit the shape as a true TS union
/// and pin that onto `next_event`'s return type. Derived from the same
/// `fpss_event_schema.toml` SSOT — adding a new data variant updates
/// both the struct and the union in lockstep.
pub fn ts_next_event_union_type() -> String {
    let schema = load_schema().expect("fpss_event_schema.toml must parse");
    let mut parts: Vec<String> = Vec::new();
    for event_name in sorted_event_names(&schema) {
        let kind_tag = snake_case(event_name);
        let field_camel = snake_to_camel(&kind_tag);
        parts.push(format!(
            "{{ kind: '{kind_tag}'; {field_camel}: {event_name} }}"
        ));
    }
    parts.push("{ kind: 'control'; control: FpssControlPayload }".to_string());
    parts.push("{ kind: 'raw_data'; rawData: FpssRawDataPayload }".to_string());
    format!("({}) | null", parts.join(" | "))
}
