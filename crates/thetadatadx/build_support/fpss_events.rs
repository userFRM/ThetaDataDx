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
    /// "data" (typed market-data tick), "simple" (diagnostic control /
    /// fallback), or "raw_data" (unrecognized wire frame). Drives how the
    /// generator places the variant in the `BufferedEvent` enum, maps it
    /// to the `FpssEvent` source variant, and whether its typed struct
    /// carries a `kind` discriminator getter.
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default)]
    doc: String,
    columns: Vec<ColumnDef>,
}

fn default_kind() -> String {
    "data".to_string()
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

/// Names of `[events.*]` entries whose `kind = "data"` — the
/// market-data tick variants. The TypeScript emitter uses this to
/// skip the per-variant `#[napi(object)]` struct emission for the
/// Simple / RawData variants, which have their own dedicated
/// `FpssSimplePayload` / `FpssRawDataPayload` payloads on the
/// `FpssEvent` wrapper.
fn sorted_data_event_names(schema: &Schema) -> Vec<&str> {
    let mut names: Vec<&str> = schema
        .events
        .iter()
        .filter(|(_, def)| def.kind == "data")
        .map(|(n, _)| n.as_str())
        .collect();
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
        "u8" => "u8",
        "f64" => "f64",
        "String" => "String",
        "Option<String>" => "Option<String>",
        "Option<i32>" => "Option<i32>",
        "Vec<u8>" => "Vec<u8>",
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
        // `u8` is widened to `u32` because napi-rs has no `u8` encoder
        // (JS numbers are f64-backed; sub-32-bit ints are all the same
        // on the wire).
        "u8" => "u32",
        "f64" => "f64",
        "String" => "String",
        "Option<String>" => "Option<String>",
        "Option<i32>" => "Option<i32>",
        "Vec<u8>" => "Vec<u8>",
        other => {
            panic!("unsupported FPSS event column type '{other}' in {event_name}.{column_name}")
        }
    }
}

/// Returns `true` when the column needs a `BigInt::from(...)` wrapper in
/// the TS typed-event converter (i.e. crosses to JS as `bigint`).
fn ts_needs_bigint(column_type: &str) -> bool {
    matches!(column_type, "i64" | "u64")
}

/// Returns `true` when the column is `Option<...>` on the Rust side and
/// therefore needs `Copy` handling in move/pattern bindings.
fn is_option(column_type: &str) -> bool {
    column_type.starts_with("Option<")
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
                "generated file out of date: {} (run: cargo run -p thetadatadx --bin generate_sdk_surfaces --features config-file)",
                file.relative_path
            )
            .into());
        }
    }
    Ok(())
}

fn render_sdk_generated_files() -> Result<Vec<GeneratedSourceFile>, Box<dyn std::error::Error>> {
    let schema = load_schema()?;
    // Single shared `BufferedEvent` definition + `fpss_event_to_buffered`
    // converter — identical for Python and TypeScript. Emitting the same
    // content to two files rather than factoring into a shared crate so
    // each SDK stays a single `include!` away from the source of truth
    // with no extra crate dependency. Field types are native Rust
    // primitives (String, i32, u64, Vec<u8>, Option<...>); the FFI
    // conversion happens in the per-language `buffered_event_to_typed`
    // dispatcher which knows the napi/pyclass surface.
    let buffered = render_buffered_event_file(&schema);
    Ok(vec![
        GeneratedSourceFile {
            relative_path: "sdks/python/src/buffered_event.rs",
            contents: buffered.clone(),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/buffered_event.rs",
            contents: buffered,
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/fpss_event_classes.rs",
            contents: render_python_fpss_event_classes(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/fpss_event_classes.rs",
            contents: render_ts_fpss_event_classes(&schema),
        },
        // Rust FFI `#[repr(C)]` structs + ZERO consts. Included from
        // `ffi/src/lib.rs` before `FfiBufferedEvent` so the hand-written
        // backing-memory wrapper can name the generated event struct.
        GeneratedSourceFile {
            relative_path: "ffi/src/fpss_event_structs.rs",
            contents: render_ffi_fpss_event_structs(&schema),
        },
        // Rust FFI `fpss_event_to_ffi` converter. Included from
        // `ffi/src/lib.rs` after `FfiBufferedEvent` so it can name the
        // hand-written backing-memory wrapper.
        GeneratedSourceFile {
            relative_path: "ffi/src/fpss_event_converter.rs",
            contents: render_ffi_fpss_event_converter(&schema),
        },
        // C header mirror of the FFI event structs. `#include`'d from
        // `sdks/go/ffi_bridge.h` for Go cgo consumption.
        GeneratedSourceFile {
            relative_path: "sdks/go/fpss_event_structs.h.inc",
            contents: render_go_fpss_event_header(&schema),
        },
        // Go-idiomatic struct definitions + kind enum + control constants
        // + `FpssEvent` wrapper. Standalone file in the `thetadatadx`
        // package, drop-in replacement for the hand-written block that
        // used to live inside `sdks/go/fpss.go`.
        GeneratedSourceFile {
            relative_path: "sdks/go/fpss_event_structs.go",
            contents: render_go_fpss_event_structs(&schema),
        },
    ])
}

// ── Shared `BufferedEvent` + converter (Python + TypeScript) ─────────────

/// Emit the `BufferedEvent` enum + the `fpss_event_to_buffered` converter
/// in a form both the Python (`pyo3`) and TypeScript (`napi-rs`) SDKs can
/// `include!`. This is the SSOT for the intermediate event shape between
/// the FPSS Disruptor callback and the per-language typed dispatcher.
///
/// Data variants + field names come from `fpss_event_schema.toml`
/// directly. Control / raw-data variants are stable and documented
/// inline; `FpssControl` itself is `#[non_exhaustive]`, so the converter
/// also has a wildcard arm that routes unknown control variants through
/// the `Simple` variant with `event_type = "unknown_control"`.
fn render_buffered_event_file(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from fpss_event_schema.toml\n",
    );
    out.push_str("// Intermediate flat event type that crosses the Rust mpsc channel\n");
    out.push_str("// between the FPSS callback and the SDK-specific typed dispatcher.\n");
    out.push_str("// Identical in Python and TypeScript crates — do not hand-edit either\n");
    out.push_str("// copy; change `fpss_event_schema.toml` and regenerate.\n\n");

    // ── enum BufferedEvent ──
    out.push_str("#[derive(Clone, Debug)]\n");
    out.push_str("pub(crate) enum BufferedEvent {\n");
    let names = sorted_event_names(schema);
    for event_name in &names {
        let def = &schema.events[*event_name];
        if !def.doc.is_empty() {
            for line in def.doc.lines() {
                writeln!(out, "    /// {line}").unwrap();
            }
        }
        writeln!(out, "    {event_name} {{").unwrap();
        for column in &def.columns {
            let ty = rust_field_type(&column.r#type, event_name, &column.name);
            writeln!(out, "        {}: {},", column.name, ty).unwrap();
        }
        out.push_str("    },\n");
    }
    out.push_str("}\n\n");

    // ── converter ──
    out.push_str(
        "pub(crate) fn fpss_event_to_buffered(event: &fpss::FpssEvent) -> BufferedEvent {\n",
    );
    out.push_str("    match event {\n");
    out.push_str("        fpss::FpssEvent::Data(data) => match data {\n");
    for event_name in &names {
        let def = &schema.events[*event_name];
        if def.kind != "data" {
            continue;
        }
        // Data variants mirror `FpssData` one-for-one by schema name.
        out.push_str(&render_data_match_arm(event_name, def));
    }
    // `FpssData` is `#[non_exhaustive]`; route unknown data variants
    // through `Simple` so downstream consumers observe them instead of
    // panicking.
    out.push_str("            _ => BufferedEvent::Simple {\n");
    out.push_str("                event_type: \"unknown_data\".to_string(),\n");
    out.push_str("                detail: None,\n");
    out.push_str("                id: None,\n");
    out.push_str("            },\n");
    out.push_str("        },\n");

    // Control variants → BufferedEvent::Simple.
    out.push_str(&render_control_match_arms());

    // Raw-data → BufferedEvent::RawData.
    out.push_str(
        "        fpss::FpssEvent::RawData { code, payload } => BufferedEvent::RawData {\n",
    );
    out.push_str("            code: *code,\n");
    out.push_str("            payload: payload.clone(),\n");
    out.push_str("        },\n");

    // `FpssEvent` itself is `#[non_exhaustive]`; same unknown-event route.
    out.push_str("        _ => BufferedEvent::Simple {\n");
    out.push_str("            event_type: \"unknown\".to_string(),\n");
    out.push_str("            detail: None,\n");
    out.push_str("            id: None,\n");
    out.push_str("        },\n");

    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

/// Field type emitted in the shared `BufferedEvent` enum. Native Rust
/// types — the FFI widening (BigInt for i64/u64, etc.) happens in the
/// per-language typed dispatcher.
fn rust_field_type(column_type: &str, event_name: &str, column_name: &str) -> &'static str {
    match column_type {
        "i32" => "i32",
        "i64" => "i64",
        "u64" => "u64",
        "u8" => "u8",
        "f64" => "f64",
        "String" => "String",
        "Option<String>" => "Option<String>",
        "Option<i32>" => "Option<i32>",
        "Vec<u8>" => "Vec<u8>",
        other => {
            panic!("unsupported FPSS event column type '{other}' in {event_name}.{column_name}")
        }
    }
}

fn render_data_match_arm(event_name: &str, def: &EventDef) -> String {
    let mut out = String::new();
    writeln!(out, "            fpss::FpssData::{event_name} {{").unwrap();
    for column in &def.columns {
        writeln!(out, "                {},", column.name).unwrap();
    }
    // `FpssData::*` variants are `#[non_exhaustive]` at the top-level
    // enum but each struct-variant has named fields. The `..` rest
    // pattern lets the core crate add auxiliary fields (e.g. an internal
    // decode cursor) without breaking the SDK conversion.
    out.push_str("                ..\n            } => BufferedEvent::");
    writeln!(out, "{event_name} {{").unwrap();
    for column in &def.columns {
        let rhs = match column.r#type.as_str() {
            "String" | "Vec<u8>" => format!("{field}.clone()", field = column.name),
            t if is_option(t) => format!("{field}.clone()", field = column.name),
            _ => format!("*{field}", field = column.name),
        };
        writeln!(out, "                {}: {rhs},", column.name).unwrap();
    }
    out.push_str("            },\n");
    out
}

fn render_control_match_arms() -> String {
    // FpssControl variant → BufferedEvent::Simple event_type tag mapping.
    // Payload extraction uses `Some(...).clone()` for diagnostic strings.
    // Stable — last changed by #368 when Reconnecting/Reconnected were
    // added. `FpssControl` is `#[non_exhaustive]`, so the trailing `_ =>`
    // catches any future variant the core crate adds.
    let mut out = String::new();
    out.push_str("        fpss::FpssEvent::Control(ctrl) => match ctrl {\n");
    out.push_str(
        "            fpss::FpssControl::LoginSuccess { permissions } => BufferedEvent::Simple {\n",
    );
    out.push_str("                event_type: \"login_success\".to_string(),\n");
    out.push_str("                detail: Some(permissions.clone()),\n");
    out.push_str("                id: None,\n");
    out.push_str("            },\n");
    out.push_str("            fpss::FpssControl::ContractAssigned { id, contract } => BufferedEvent::Simple {\n");
    out.push_str("                event_type: \"contract_assigned\".to_string(),\n");
    out.push_str("                detail: Some(format!(\"{contract}\")),\n");
    out.push_str("                id: Some(*id),\n");
    out.push_str("            },\n");
    out.push_str("            fpss::FpssControl::ReqResponse { req_id, result } => BufferedEvent::Simple {\n");
    out.push_str("                event_type: \"req_response\".to_string(),\n");
    out.push_str("                detail: Some(format!(\"{result:?}\")),\n");
    out.push_str("                id: Some(*req_id),\n");
    out.push_str("            },\n");
    out.push_str("            fpss::FpssControl::MarketOpen => BufferedEvent::Simple {\n");
    out.push_str("                event_type: \"market_open\".to_string(),\n");
    out.push_str("                detail: None,\n");
    out.push_str("                id: None,\n");
    out.push_str("            },\n");
    out.push_str("            fpss::FpssControl::MarketClose => BufferedEvent::Simple {\n");
    out.push_str("                event_type: \"market_close\".to_string(),\n");
    out.push_str("                detail: None,\n");
    out.push_str("                id: None,\n");
    out.push_str("            },\n");
    out.push_str(
        "            fpss::FpssControl::ServerError { message } => BufferedEvent::Simple {\n",
    );
    out.push_str("                event_type: \"server_error\".to_string(),\n");
    out.push_str("                detail: Some(message.clone()),\n");
    out.push_str("                id: None,\n");
    out.push_str("            },\n");
    out.push_str(
        "            fpss::FpssControl::Disconnected { reason } => BufferedEvent::Simple {\n",
    );
    out.push_str("                event_type: \"disconnected\".to_string(),\n");
    out.push_str("                detail: Some(format!(\"{reason:?}\")),\n");
    out.push_str("                id: None,\n");
    out.push_str("            },\n");
    out.push_str("            fpss::FpssControl::Reconnecting {\n");
    out.push_str("                reason,\n");
    out.push_str("                attempt,\n");
    out.push_str("                delay_ms,\n");
    out.push_str("            } => BufferedEvent::Simple {\n");
    out.push_str("                event_type: \"reconnecting\".to_string(),\n");
    out.push_str("                detail: Some(format!(\n");
    out.push_str(
        "                    \"reason={reason:?} attempt={attempt} delay_ms={delay_ms}\"\n",
    );
    out.push_str("                )),\n");
    out.push_str("                id: Some(*attempt as i32),\n");
    out.push_str("            },\n");
    out.push_str("            fpss::FpssControl::Reconnected => BufferedEvent::Simple {\n");
    out.push_str("                event_type: \"reconnected\".to_string(),\n");
    out.push_str("                detail: None,\n");
    out.push_str("                id: None,\n");
    out.push_str("            },\n");
    out.push_str("            fpss::FpssControl::Error { message } => BufferedEvent::Simple {\n");
    out.push_str("                event_type: \"error\".to_string(),\n");
    out.push_str("                detail: Some(message.clone()),\n");
    out.push_str("                id: None,\n");
    out.push_str("            },\n");
    out.push_str("            fpss::FpssControl::UnknownFrame { code, payload } => BufferedEvent::Simple {\n");
    out.push_str("                event_type: \"unknown_frame\".to_string(),\n");
    out.push_str("                detail: Some(format!(\n");
    out.push_str("                    \"code={code} payload_hex={}\",\n");
    out.push_str("                    payload\n");
    out.push_str("                        .iter()\n");
    out.push_str("                        .map(|b| format!(\"{b:02x}\"))\n");
    out.push_str("                        .collect::<String>()\n");
    out.push_str("                )),\n");
    out.push_str("                id: Some(*code as i32),\n");
    out.push_str("            },\n");
    out.push_str("            _ => BufferedEvent::Simple {\n");
    out.push_str("                event_type: \"unknown_control\".to_string(),\n");
    out.push_str("                detail: None,\n");
    out.push_str("                id: None,\n");
    out.push_str("            },\n");
    out.push_str("        },\n");
    out
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

    // Dispatcher: BufferedEvent -> typed pyclass for EVERY variant.
    // No PyDict fallback — every `[events.*]` entry in the schema has a
    // typed `#[pyclass]` representation, so the match is exhaustive and
    // the dict path is obliterated from the public Python surface.
    out.push_str("pub(crate) fn buffered_event_to_typed(\n");
    out.push_str("    py: Python<'_>,\n");
    out.push_str("    event: &BufferedEvent,\n");
    out.push_str(") -> PyResult<Py<PyAny>> {\n");
    out.push_str("    match event {\n");
    for event_name in &names {
        let def = &schema.events[*event_name];
        out.push_str(&render_python_buffered_match_arm(event_name, def));
    }
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
    // No `..` rest pattern: `BufferedEvent` itself is generator-emitted
    // from this same schema, so the variant arms are exhaustive by
    // construction. Any drift is a compile error, not silent data loss.
    out.push_str("        } => Py::new(\n");
    out.push_str("            py,\n");
    writeln!(out, "            {event_name} {{").unwrap();
    for column in &def.columns {
        let rhs =
            if is_option(&column.r#type) || column.r#type == "String" || column.r#type == "Vec<u8>"
            {
                // Non-`Copy` types clone through the deref of the pattern binding.
                format!("{field}.clone()", field = column.name)
            } else {
                // `Copy` primitives — explicit deref of the reference binding.
                format!("*{field}", field = column.name)
            };
        writeln!(
            out,
            "                {field}: {rhs},",
            field = column.name,
            rhs = rhs
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
    out.push_str("// payload field. Simple / control / raw events expose their diagnostic\n");
    out.push_str("// strings via `simple` and `raw_data` payloads. The `simple` kind tag\n");
    out.push_str("// matches the dict-path serde rename on `BufferedEvent::Simple`.\n");
    out.push_str("//\n");
    out.push_str("// `BigInt` is pulled in via `tick_classes.rs`'s `use` line — both\n");
    out.push_str("// files `include!` into the same TS-SDK module scope (see\n");
    out.push_str("// `sdks/typescript/src/lib.rs`), so importing it here would collide.\n\n");

    // Per-variant `#[napi(object)]` structs are emitted only for `data`
    // kinds. Simple + RawData have their own dedicated
    // `FpssSimplePayload` / `FpssRawDataPayload` hand-shaped below to
    // keep the public TS discriminator aligned with
    // `#[serde(rename = "simple" / "raw_data")]` in `BufferedEvent`.
    let data_names = sorted_data_event_names(schema);
    let names = sorted_event_names(schema);

    for event_name in &data_names {
        let def = &schema.events[*event_name];
        out.push_str(&render_ts_event_class_struct(event_name, def));
        out.push('\n');
    }

    // Simple / diagnostic payload + raw payload.
    out.push_str("/// FPSS simple / diagnostic payload (login, disconnect, market open,\n");
    out.push_str("/// unknown-data fallback, ...). Mirrors `BufferedEvent::Simple`.\n");
    out.push_str("#[napi(object)]\n");
    out.push_str("#[derive(Clone)]\n");
    out.push_str("pub struct FpssSimplePayload {\n");
    out.push_str("    /// Concrete event kind (e.g. \"login_success\", \"disconnected\",\n");
    out.push_str("    /// \"unknown_data\", \"unknown_control\").\n");
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
    // Kind tags: one per data variant + the fixed `simple` / `raw_data`
    // tags (emitted via the `FpssSimplePayload` / `FpssRawDataPayload`
    // payload fields below). Schema Simple / RawData variants produce
    // the same two tags, so `names` here picks up both from the schema.
    let kind_tags: Vec<String> = names
        .iter()
        .map(|n| format!("'{}'", snake_case(n)))
        .collect();
    let kind_union = kind_tags.join(" | ");
    out.push_str("    /// Discriminator matching one of the typed payload fields below.\n");
    out.push_str("    /// Narrowed to a literal union in TS so `switch (event.kind)`\n");
    out.push_str("    /// correctly narrows the optional payload fields.\n");
    writeln!(out, "    #[napi(ts_type = \"{kind_union}\")]").unwrap();
    out.push_str("    pub kind: String,\n");
    // One optional typed payload per data variant.
    for event_name in &data_names {
        let field = snake_case(event_name);
        writeln!(
            out,
            "    pub {field}: Option<{event_name}>,",
            event_name = event_name
        )
        .unwrap();
    }
    // Plus the two fixed payloads for non-data variants. These field
    // names align with the `kind` tag produced by the schema Simple /
    // RawData variants (`snake_case("Simple") == "simple"`,
    // `snake_case("RawData") == "raw_data"`).
    out.push_str("    pub simple: Option<FpssSimplePayload>,\n");
    out.push_str("    pub raw_data: Option<FpssRawDataPayload>,\n");
    out.push_str("}\n\n");

    // Dispatcher.
    out.push_str("pub(crate) fn buffered_event_to_typed(event: BufferedEvent) -> FpssEvent {\n");
    out.push_str("    let mut out = FpssEvent {\n");
    out.push_str("        kind: String::new(),\n");
    for event_name in &data_names {
        let field = snake_case(event_name);
        writeln!(out, "        {field}: None,").unwrap();
    }
    out.push_str("        simple: None,\n");
    out.push_str("        raw_data: None,\n");
    out.push_str("    };\n");
    out.push_str("    match event {\n");
    for event_name in &data_names {
        let def = &schema.events[*event_name];
        let field = snake_case(event_name);
        let kind_tag = snake_case(event_name);
        writeln!(out, "        BufferedEvent::{event_name} {{").unwrap();
        for column in &def.columns {
            writeln!(out, "            {},", column.name).unwrap();
        }
        out.push_str("        } => {\n");
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
    out.push_str("            out.kind = \"simple\".to_string();\n");
    out.push_str("            out.simple = Some(FpssSimplePayload {\n");
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
    for event_name in sorted_data_event_names(&schema) {
        let kind_tag = snake_case(event_name);
        let field_camel = snake_to_camel(&kind_tag);
        parts.push(format!(
            "{{ kind: '{kind_tag}'; {field_camel}: {event_name} }}"
        ));
    }
    parts.push("{ kind: 'simple'; simple: FpssSimplePayload }".to_string());
    parts.push("{ kind: 'raw_data'; rawData: FpssRawDataPayload }".to_string());
    format!("({}) | null", parts.join(" | "))
}

// ── Shared helpers for the Rust-FFI / C header / Go emitters ────────────

/// Iterate schema variants in a stable order, yielding only `kind = "data"`
/// entries. The Rust-FFI, C-header, and Go emitters all share this ordering
/// so the tagged-struct `TdxFpssEvent` / `FpssEvent` layouts line up across
/// languages without manual coordination.
fn sorted_data_events(schema: &Schema) -> Vec<(&str, &EventDef)> {
    let mut out: Vec<(&str, &EventDef)> = schema
        .events
        .iter()
        .filter(|(_, def)| def.kind == "data")
        .map(|(n, d)| (n.as_str(), d))
        .collect();
    out.sort_by_key(|(n, _)| *n);
    out
}

/// Schema primitive → Rust `#[repr(C)]` scalar. The Rust-FFI struct names
/// each scalar exactly as the schema does; the C header mirror below uses
/// the matching `<cstdint>` alias so both sides have the same layout.
fn rust_ffi_scalar(column_type: &str, event_name: &str, column_name: &str) -> &'static str {
    match column_type {
        "i32" => "i32",
        "i64" => "i64",
        "u64" => "u64",
        "u8" => "u8",
        "f64" => "f64",
        other => panic!(
            "unsupported Rust FFI column type '{other}' in {event_name}.{column_name} \
             (data variants must be pure scalars; strings/bytes belong on control/raw variants)"
        ),
    }
}

/// Zero literal for a schema primitive in the `ZERO_*` const body.
fn rust_ffi_zero_literal(column_type: &str) -> &'static str {
    match column_type {
        "i32" | "i64" | "u64" | "u8" => "0",
        "f64" => "0.0",
        other => panic!("no FFI zero literal for column type '{other}'"),
    }
}

/// Schema primitive → C `<stdint.h>` alias used by the cgo-facing header.
fn c_ffi_scalar(column_type: &str, event_name: &str, column_name: &str) -> &'static str {
    match column_type {
        "i32" => "int32_t",
        "i64" => "int64_t",
        "u64" => "uint64_t",
        "u8" => "uint8_t",
        "f64" => "double",
        other => panic!("unsupported C FFI column type '{other}' in {event_name}.{column_name}"),
    }
}

/// Schema primitive → Go scalar (match the `C.` cgo type promotions the
/// generated converter uses — `int32`, `int64`, `uint64`, `uint8`,
/// `float64`).
fn go_scalar(column_type: &str, event_name: &str, column_name: &str) -> &'static str {
    match column_type {
        "i32" => "int32",
        "i64" => "int64",
        "u64" => "uint64",
        "u8" => "uint8",
        "f64" => "float64",
        other => panic!("unsupported Go column type '{other}' in {event_name}.{column_name}"),
    }
}

/// snake_case column name → Go PascalCase field identifier.
///
/// Special-case: a bare `id` word maps to `ID` so `contract_id` →
/// `ContractID` (matching existing Go convention). Every other word is
/// simple `capitalize-first-letter-only`, so `ms_of_day` → `MsOfDay`,
/// `ext_condition1` → `ExtCondition1`, etc. Trailing digits stay attached
/// to the word they follow.
fn snake_to_go_pascal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for part in s.split('_') {
        if part.is_empty() {
            continue;
        }
        if part == "id" {
            out.push_str("ID");
            continue;
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            for ch in chars {
                out.push(ch);
            }
        }
    }
    out
}

// ── Rust FFI struct emitter (ffi/src/fpss_event_structs.rs) ─────────────

/// Emit the `#[repr(C)]` FPSS event structs + `ZERO_*` consts + tagged
/// `TdxFpssEvent` for the Rust FFI crate.
///
/// The file is `include!`'d from `ffi/src/lib.rs` BEFORE `FfiBufferedEvent`
/// so the hand-written backing-memory wrapper can name the generated
/// tagged struct. Required imports (`std::ffi::CString`,
/// `std::os::raw::c_char`, `std::ptr`) must already be in scope at the
/// include site — this file does not re-declare them.
fn render_ffi_fpss_event_structs(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from fpss_event_schema.toml\n",
    );
    out.push_str("// Rust FFI `#[repr(C)]` FPSS event structs + ZERO_* consts + tagged\n");
    out.push_str("// `TdxFpssEvent`. `include!`'d from `ffi/src/lib.rs`; do not hand-edit.\n");
    out.push_str("// Expects `std::os::raw::c_char`, `std::ptr` already in scope at the\n");
    out.push_str("// include site.\n\n");

    // Kind enum — order matches the C header + Go enum below.
    out.push_str("/// FPSS event kind tag. Check this to determine which field of\n");
    out.push_str("/// `TdxFpssEvent` is valid.\n");
    out.push_str("#[repr(C)]\n");
    out.push_str("pub enum TdxFpssEventKind {\n");
    out.push_str("    Quote = 0,\n");
    out.push_str("    Trade = 1,\n");
    out.push_str("    OpenInterest = 2,\n");
    out.push_str("    Ohlcvc = 3,\n");
    out.push_str("    Control = 4,\n");
    out.push_str("    RawData = 5,\n");
    out.push_str("}\n\n");

    // One #[repr(C)] struct per data variant.
    for (event_name, def) in sorted_data_events(schema) {
        let doc_text = if def.doc.is_empty() {
            format!("`#[repr(C)]` FPSS {event_name} event.")
        } else {
            def.doc.clone()
        };
        for line in doc_text.lines() {
            writeln!(out, "/// {line}").unwrap();
        }
        out.push_str("#[repr(C)]\n");
        writeln!(out, "pub struct TdxFpss{event_name} {{").unwrap();
        for column in &def.columns {
            let ty = rust_ffi_scalar(&column.r#type, event_name, &column.name);
            writeln!(out, "    pub {}: {},", column.name, ty).unwrap();
        }
        out.push_str("}\n\n");
    }

    // Control + RawData are fixed shapes — the schema-defined Simple /
    // RawData variants model them on the Python/TS side, but the FFI
    // surface keeps the Control-kind-integer + optional-`detail` pattern
    // that every downstream SDK already speaks. Documented inline to keep
    // the kind-integer mapping next to the struct definition.
    out.push_str("/// `#[repr(C)]` FPSS control event.\n");
    out.push_str("///\n");
    out.push_str("/// `kind` encodes the control sub-type:\n");
    out.push_str("///   `0=login_success`, `1=contract_assigned`, `2=req_response`,\n");
    out.push_str("///   `3=market_open`, `4=market_close`, `5=server_error`,\n");
    out.push_str("///   `6=disconnected`, `8=reconnecting`, `9=reconnected`,\n");
    out.push_str("///   `10=error`, `11=unknown_frame`, `12=unknown_event` (non-Data /\n");
    out.push_str("///   non-Control / non-RawData fallback; carries no payload).\n");
    out.push_str("///   Value `7` is reserved for future use. `99` is an internal sentinel\n");
    out.push_str("///   for \"unknown control-variant\" — kept for backward compat; new\n");
    out.push_str("///   consumers should treat `12` as the canonical unknown marker.\n");
    out.push_str("///\n");
    out.push_str("/// `id` carries the `contract_id`, `req_id`, reconnect attempt number,\n");
    out.push_str("/// or unknown-frame code where applicable (0 otherwise).\n");
    out.push_str("/// `detail` is a NUL-terminated C string (may be null).\n");
    out.push_str("#[repr(C)]\n");
    out.push_str("pub struct TdxFpssControl {\n");
    out.push_str("    pub kind: i32,\n");
    out.push_str("    pub id: i32,\n");
    out.push_str("    pub detail: *const c_char,\n");
    out.push_str("}\n\n");

    out.push_str("/// `#[repr(C)]` FPSS raw/undecoded data event.\n");
    out.push_str("///\n");
    out.push_str("/// `code` is the wire message code. `payload` is a pointer to the raw\n");
    out.push_str("/// bytes and `payload_len` is the number of bytes.\n");
    out.push_str("#[repr(C)]\n");
    out.push_str("pub struct TdxFpssRawData {\n");
    out.push_str("    pub code: u8,\n");
    out.push_str("    pub payload: *const u8,\n");
    out.push_str("    pub payload_len: usize,\n");
    out.push_str("}\n\n");

    // Tagged union-style wrapper. Flat struct (not a C union) so the
    // layout is trivially FFI-safe.
    out.push_str("/// Tagged FPSS event for FFI. Check `kind` then read the corresponding\n");
    out.push_str("/// field. Only the field matching `kind` contains valid data — this is\n");
    out.push_str("/// a flat struct (not a C union) for simplicity and safety.\n");
    out.push_str("#[repr(C)]\n");
    out.push_str("pub struct TdxFpssEvent {\n");
    out.push_str("    pub kind: TdxFpssEventKind,\n");
    for (event_name, _) in sorted_data_events(schema) {
        let field = snake_case(event_name);
        writeln!(out, "    pub {field}: TdxFpss{event_name},").unwrap();
    }
    out.push_str("    pub control: TdxFpssControl,\n");
    out.push_str("    pub raw_data: TdxFpssRawData,\n");
    out.push_str("}\n\n");

    // Zero-initialised defaults for inactive fields.
    out.push_str("// Zero-initialized defaults for inactive union-style fields.\n");
    for (event_name, def) in sorted_data_events(schema) {
        let const_name = zero_const_name(event_name);
        writeln!(
            out,
            "pub(crate) const {const_name}: TdxFpss{event_name} = TdxFpss{event_name} {{"
        )
        .unwrap();
        for column in &def.columns {
            let lit = rust_ffi_zero_literal(&column.r#type);
            writeln!(out, "    {}: {lit},", column.name).unwrap();
        }
        out.push_str("};\n");
    }
    out.push_str("pub(crate) const ZERO_CONTROL: TdxFpssControl = TdxFpssControl {\n");
    out.push_str("    kind: 0,\n");
    out.push_str("    id: 0,\n");
    out.push_str("    detail: ptr::null(),\n");
    out.push_str("};\n");
    out.push_str("pub(crate) const ZERO_RAW: TdxFpssRawData = TdxFpssRawData {\n");
    out.push_str("    code: 0,\n");
    out.push_str("    payload: ptr::null(),\n");
    out.push_str("    payload_len: 0,\n");
    out.push_str("};\n");

    out
}

/// `Quote` → `ZERO_QUOTE`, `OpenInterest` → `ZERO_OI`, `Ohlcvc` →
/// `ZERO_OHLCVC`. Matches the hand-written names the converter used so
/// diffs against the old code stay readable.
fn zero_const_name(event_name: &str) -> String {
    match event_name {
        "OpenInterest" => "ZERO_OI".to_string(),
        _ => format!("ZERO_{}", snake_case(event_name).to_uppercase()),
    }
}

// ── Rust FFI converter emitter (ffi/src/fpss_event_converter.rs) ────────

/// Emit `fpss_event_to_ffi`, which converts a `thetadatadx::fpss::FpssEvent`
/// into an `FfiBufferedEvent` ready to box + cast across the FFI boundary.
///
/// `include!`'d from `ffi/src/lib.rs` AFTER `FfiBufferedEvent` is defined
/// so it can name the backing-memory wrapper. The Control-variant → (kind,
/// id, detail) table is kept in a single `match` so the mapping lives next
/// to the doc-comment on `TdxFpssControl`.
fn render_ffi_fpss_event_converter(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from fpss_event_schema.toml\n",
    );
    out.push_str("// FPSS event → FFI buffered-event converter. `include!`'d from\n");
    out.push_str("// `ffi/src/lib.rs` after `FfiBufferedEvent` is in scope.\n\n");

    out.push_str("pub(crate) fn fpss_event_to_ffi(event: &thetadatadx::fpss::FpssEvent) -> FfiBufferedEvent {\n");
    out.push_str("    use thetadatadx::fpss::{FpssControl, FpssData, FpssEvent};\n\n");
    out.push_str("    match event {\n");

    let data_events = sorted_data_events(schema);

    // One match arm per `FpssData::*` variant. Each arm fills the matching
    // TdxFpss{Variant} field and zero-fills the rest. Non-`Copy` columns
    // would need `.clone()` — all `data` variants use pure scalars so
    // dereference is always sufficient.
    for (event_name, def) in &data_events {
        writeln!(out, "        FpssEvent::Data(FpssData::{event_name} {{").unwrap();
        for column in &def.columns {
            writeln!(out, "            {},", column.name).unwrap();
        }
        // `..` rest pattern so the core crate can add internal decode
        // cursors / auxiliary fields without breaking the FFI conversion.
        out.push_str("            ..\n");
        out.push_str("        }) => FfiBufferedEvent {\n");
        out.push_str("            event: TdxFpssEvent {\n");
        writeln!(out, "                kind: TdxFpssEventKind::{event_name},").unwrap();
        // The variant-specific typed payload.
        let field = snake_case(event_name);
        writeln!(out, "                {field}: TdxFpss{event_name} {{").unwrap();
        for column in &def.columns {
            writeln!(
                out,
                "                    {name}: *{name},",
                name = column.name
            )
            .unwrap();
        }
        out.push_str("                },\n");
        // Zero-fill every sibling data field + control + raw_data.
        for (other_name, _) in &data_events {
            if other_name == event_name {
                continue;
            }
            let other_field = snake_case(other_name);
            let other_zero = zero_const_name(other_name);
            writeln!(out, "                {other_field}: {other_zero},").unwrap();
        }
        out.push_str("                control: ZERO_CONTROL,\n");
        out.push_str("                raw_data: ZERO_RAW,\n");
        out.push_str("            },\n");
        out.push_str("            _detail_string: None,\n");
        out.push_str("            _raw_payload: None,\n");
        out.push_str("        },\n\n");
    }

    // Raw-data arm. Clones the payload into owned storage so the `*const u8`
    // pointer stays valid for the lifetime of `FfiBufferedEvent`.
    out.push_str("        FpssEvent::RawData { code, payload } => {\n");
    out.push_str("            let owned = payload.clone();\n");
    out.push_str("            let ptr = owned.as_ptr();\n");
    out.push_str("            let len = owned.len();\n");
    out.push_str("            FfiBufferedEvent {\n");
    out.push_str("                event: TdxFpssEvent {\n");
    out.push_str("                    kind: TdxFpssEventKind::RawData,\n");
    out.push_str("                    raw_data: TdxFpssRawData {\n");
    out.push_str("                        code: *code,\n");
    out.push_str("                        payload: ptr,\n");
    out.push_str("                        payload_len: len,\n");
    out.push_str("                    },\n");
    for (event_name, _) in &data_events {
        let field = snake_case(event_name);
        let zero = zero_const_name(event_name);
        writeln!(out, "                    {field}: {zero},").unwrap();
    }
    out.push_str("                    control: ZERO_CONTROL,\n");
    out.push_str("                },\n");
    out.push_str("                _detail_string: None,\n");
    out.push_str("                _raw_payload: Some(owned),\n");
    out.push_str("            }\n");
    out.push_str("        }\n\n");

    // Control arm. The (kind, id, detail) mapping is kept here — the doc
    // comment on `TdxFpssControl` documents the same integer tags. Any
    // divergence between the two would be a review finding.
    out.push_str("        FpssEvent::Control(ctrl) => {\n");
    out.push_str("            let (kind, id, detail_str) = match ctrl {\n");
    out.push_str(
        "                FpssControl::LoginSuccess { permissions } => (0, 0, Some(permissions.clone())),\n",
    );
    out.push_str(
        "                FpssControl::ContractAssigned { id, contract } => (1, *id, Some(format!(\"{contract}\"))),\n",
    );
    out.push_str(
        "                FpssControl::ReqResponse { req_id, result } => (2, *req_id, Some(format!(\"{result:?}\"))),\n",
    );
    out.push_str("                FpssControl::MarketOpen => (3, 0, None),\n");
    out.push_str("                FpssControl::MarketClose => (4, 0, None),\n");
    out.push_str(
        "                FpssControl::ServerError { message } => (5, 0, Some(message.clone())),\n",
    );
    out.push_str(
        "                FpssControl::Disconnected { reason } => (6, 0, Some(format!(\"{reason:?}\"))),\n",
    );
    out.push_str("                FpssControl::Reconnecting {\n");
    out.push_str("                    reason,\n");
    out.push_str("                    attempt,\n");
    out.push_str("                    delay_ms,\n");
    out.push_str(
        "                } => (8, *attempt as i32, Some(format!(\"{reason:?} delay={delay_ms}ms\"))),\n",
    );
    out.push_str("                FpssControl::Reconnected => (9, 0, None),\n");
    out.push_str(
        "                FpssControl::Error { message } => (10, 0, Some(message.clone())),\n",
    );
    out.push_str("                FpssControl::UnknownFrame { code, payload } => (\n");
    out.push_str("                    11,\n");
    out.push_str("                    *code as i32,\n");
    out.push_str("                    Some(\n");
    out.push_str("                        payload\n");
    out.push_str("                            .iter()\n");
    out.push_str("                            .map(|b| format!(\"{b:02x}\"))\n");
    out.push_str("                            .collect::<String>(),\n");
    out.push_str("                    ),\n");
    out.push_str("                ),\n");
    // `FpssControl` is `#[non_exhaustive]`.
    out.push_str("                _ => (99, 0, None), // unknown control\n");
    out.push_str("            };\n");
    out.push_str(
        "            let cstring = detail_str.and_then(|s| std::ffi::CString::new(s).ok());\n",
    );
    out.push_str(
        "            let detail_ptr = cstring.as_ref().map_or(ptr::null(), |cs| cs.as_ptr());\n",
    );
    out.push_str("            FfiBufferedEvent {\n");
    out.push_str("                event: TdxFpssEvent {\n");
    out.push_str("                    kind: TdxFpssEventKind::Control,\n");
    out.push_str("                    control: TdxFpssControl {\n");
    out.push_str("                        kind,\n");
    out.push_str("                        id,\n");
    out.push_str("                        detail: detail_ptr,\n");
    out.push_str("                    },\n");
    for (event_name, _) in &data_events {
        let field = snake_case(event_name);
        let zero = zero_const_name(event_name);
        writeln!(out, "                    {field}: {zero},").unwrap();
    }
    out.push_str("                    raw_data: ZERO_RAW,\n");
    out.push_str("                },\n");
    out.push_str("                _detail_string: cstring,\n");
    out.push_str("                _raw_payload: None,\n");
    out.push_str("            }\n");
    out.push_str("        }\n\n");

    // Non-Data, non-Control, non-RawData fallback. `FpssEvent` itself is
    // `#[non_exhaustive]`, so a future variant lands here; kind=12 is the
    // canonical unknown-event sentinel (NOT 8 — 8 is Reconnecting).
    out.push_str("        _ => {\n");
    out.push_str("            // Empty / unknown event — surface as a control with kind=12.\n");
    out.push_str("            // kind=12 is the canonical unknown-event sentinel; see doc\n");
    out.push_str("            // comment on `TdxFpssControl` for the full mapping.\n");
    out.push_str("            FfiBufferedEvent {\n");
    out.push_str("                event: TdxFpssEvent {\n");
    out.push_str("                    kind: TdxFpssEventKind::Control,\n");
    out.push_str("                    control: TdxFpssControl {\n");
    out.push_str("                        kind: 12,\n");
    out.push_str("                        id: 0,\n");
    out.push_str("                        detail: ptr::null(),\n");
    out.push_str("                    },\n");
    for (event_name, _) in &data_events {
        let field = snake_case(event_name);
        let zero = zero_const_name(event_name);
        writeln!(out, "                    {field}: {zero},").unwrap();
    }
    out.push_str("                    raw_data: ZERO_RAW,\n");
    out.push_str("                },\n");
    out.push_str("                _detail_string: None,\n");
    out.push_str("                _raw_payload: None,\n");
    out.push_str("            }\n");
    out.push_str("        }\n");

    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

// ── C header emitter (sdks/go/fpss_event_structs.h.inc) ─────────────────

/// Emit the C mirror of the Rust FFI event structs. `#include`'d from
/// `sdks/go/ffi_bridge.h` for Go's cgo consumption. Keeps field order and
/// padding identical to the Rust `#[repr(C)]` layout by using the same
/// schema column ordering on both sides.
fn render_go_fpss_event_header(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str(
        "/* @generated DO NOT EDIT — regenerated by build.rs from fpss_event_schema.toml */\n",
    );
    out.push_str("/* C mirror of the Rust FFI FPSS event structs. #include'd from\n");
    out.push_str(" * sdks/go/ffi_bridge.h for Go cgo consumption; do not hand-edit. */\n\n");

    out.push_str("typedef enum {\n");
    out.push_str("    TDX_FPSS_QUOTE = 0,\n");
    out.push_str("    TDX_FPSS_TRADE = 1,\n");
    out.push_str("    TDX_FPSS_OPEN_INTEREST = 2,\n");
    out.push_str("    TDX_FPSS_OHLCVC = 3,\n");
    out.push_str("    TDX_FPSS_CONTROL = 4,\n");
    out.push_str("    TDX_FPSS_RAW_DATA = 5,\n");
    out.push_str("} TdxFpssEventKind;\n\n");

    for (event_name, def) in sorted_data_events(schema) {
        out.push_str("typedef struct {\n");
        for column in &def.columns {
            let ty = c_ffi_scalar(&column.r#type, event_name, &column.name);
            writeln!(out, "    {ty} {};", column.name).unwrap();
        }
        writeln!(out, "}} TdxFpss{event_name};\n").unwrap();
    }

    out.push_str("typedef struct {\n");
    out.push_str("    int32_t kind;\n");
    out.push_str("    int32_t id;\n");
    out.push_str("    const char* detail;\n");
    out.push_str("} TdxFpssControl;\n\n");

    out.push_str("typedef struct {\n");
    out.push_str("    uint8_t code;\n");
    out.push_str("    const uint8_t* payload;\n");
    out.push_str("    size_t payload_len;\n");
    out.push_str("} TdxFpssRawData;\n\n");

    out.push_str("typedef struct {\n");
    out.push_str("    TdxFpssEventKind kind;\n");
    for (event_name, _) in sorted_data_events(schema) {
        let field = snake_case(event_name);
        writeln!(out, "    TdxFpss{event_name} {field};").unwrap();
    }
    out.push_str("    TdxFpssControl control;\n");
    out.push_str("    TdxFpssRawData raw_data;\n");
    out.push_str("} TdxFpssEvent;\n");

    out
}

// ── Go struct emitter (sdks/go/fpss_event_structs.go) ───────────────────

/// Emit the Go-idiomatic FPSS event types, kind enum, control constants,
/// and `FpssEvent` wrapper. Lives next to `fpss.go` in the `thetadatadx`
/// package; replaces the hand-written block that used to live there.
///
/// Breaking rename: `FpssOpenInterest` (not `FpssOpenInterestData`) and
/// `FpssControl` (not `FpssControlData`). The old suffix was a legacy of
/// the first Go prototype and has no active consumers per user direction.
fn render_go_fpss_event_structs(schema: &Schema) -> String {
    let mut out = String::new();
    out.push_str("// Code generated by build.rs from fpss_event_schema.toml. DO NOT EDIT.\n");
    out.push_str("// Go-idiomatic mirror of the Rust FFI FPSS event types. Lives in\n");
    out.push_str("// the `thetadatadx` package alongside the hand-written client glue.\n\n");
    out.push_str("package thetadatadx\n\n");

    out.push_str("// FpssEventKind identifies the type of an FPSS streaming event.\n");
    out.push_str("type FpssEventKind int\n\n");
    out.push_str("const (\n");
    out.push_str("\tFpssQuoteEvent        FpssEventKind = 0\n");
    out.push_str("\tFpssTradeEvent        FpssEventKind = 1\n");
    out.push_str("\tFpssOpenInterestEvent FpssEventKind = 2\n");
    out.push_str("\tFpssOhlcvcEvent       FpssEventKind = 3\n");
    out.push_str("\tFpssControlEvent      FpssEventKind = 4\n");
    out.push_str("\tFpssRawDataEvent      FpssEventKind = 5\n");
    out.push_str(")\n\n");

    out.push_str("// FpssControlKind identifies the sub-type of a control event.\n");
    out.push_str("// Use with FpssControl.Kind.\n");
    out.push_str("type FpssControlKind = int32\n\n");
    out.push_str("const (\n");
    out.push_str("\tFpssCtrlLoginSuccess     FpssControlKind = 0\n");
    out.push_str("\tFpssCtrlContractAssigned FpssControlKind = 1\n");
    out.push_str("\tFpssCtrlReqResponse      FpssControlKind = 2\n");
    out.push_str("\tFpssCtrlMarketOpen       FpssControlKind = 3\n");
    out.push_str("\tFpssCtrlMarketClose      FpssControlKind = 4\n");
    out.push_str("\tFpssCtrlServerError      FpssControlKind = 5\n");
    out.push_str("\tFpssCtrlDisconnected     FpssControlKind = 6\n");
    out.push_str("\tFpssCtrlReconnecting     FpssControlKind = 8\n");
    out.push_str("\tFpssCtrlReconnected      FpssControlKind = 9\n");
    out.push_str("\tFpssCtrlError            FpssControlKind = 10\n");
    out.push_str("\tFpssCtrlUnknownFrame     FpssControlKind = 11 // ID = frame code, Detail = hex payload\n");
    out.push_str("\tFpssCtrlUnknownEvent     FpssControlKind = 12 // non-Data / non-Control / non-RawData fallback\n");
    out.push_str("\t// Value 7 is reserved for future use.\n");
    out.push_str(")\n\n");

    for (event_name, def) in sorted_data_events(schema) {
        let doc_text = if def.doc.is_empty() {
            format!("Fpss{event_name} is a real-time {event_name} event from FPSS.")
        } else {
            def.doc.clone()
        };
        for line in doc_text.lines() {
            writeln!(out, "// {line}").unwrap();
        }
        writeln!(out, "type Fpss{event_name} struct {{").unwrap();
        // Pre-compute Go field names so we can align them in columns.
        let go_fields: Vec<(String, &'static str)> = def
            .columns
            .iter()
            .map(|c| {
                (
                    snake_to_go_pascal(&c.name),
                    go_scalar(&c.r#type, event_name, &c.name),
                )
            })
            .collect();
        let name_width = go_fields.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        for (name, ty) in &go_fields {
            writeln!(out, "\t{name:<name_width$} {ty}").unwrap();
        }
        out.push_str("}\n\n");
    }

    out.push_str("// FpssControl is a control/lifecycle event from FPSS.\n");
    out.push_str("//\n");
    out.push_str("// Kind encodes the sub-type; see the FpssCtrl* constants above for the\n");
    out.push_str("// canonical numeric mapping (0..=6, 8..=12). Value 7 is currently\n");
    out.push_str("// unassigned — use the named constants, not literals.\n");
    out.push_str("//\n");
    out.push_str("// ID carries the contract_id, req_id, or reconnect attempt number where\n");
    out.push_str("// applicable (0 otherwise). Detail is a human-readable string (may be\n");
    out.push_str("// empty).\n");
    out.push_str("type FpssControl struct {\n");
    out.push_str("\tKind   int32\n");
    out.push_str("\tID     int32\n");
    out.push_str("\tDetail string\n");
    out.push_str("}\n\n");

    out.push_str("// FpssRawData is an undecoded frame from FPSS. Surfaced when the wire\n");
    out.push_str("// message code does not match any known decoder.\n");
    out.push_str("type FpssRawData struct {\n");
    out.push_str("\tCode    uint8\n");
    out.push_str("\tPayload []byte\n");
    out.push_str("}\n\n");

    out.push_str("// FpssEvent is a tagged streaming event from FPSS.\n");
    out.push_str("// Check Kind to determine which field is non-nil.\n");
    out.push_str("type FpssEvent struct {\n");
    out.push_str("\tKind FpssEventKind\n");
    for (event_name, _) in sorted_data_events(schema) {
        // Field name matches the kind: `Quote`, `Trade`, `OpenInterest`,
        // `Ohlcvc`. Consumers do `event.Quote` / `event.Ohlcvc`.
        writeln!(out, "\t{event_name} *Fpss{event_name}").unwrap();
    }
    out.push_str("\tControl *FpssControl\n");
    out.push_str("\tRawData *FpssRawData\n");
    out.push_str("}\n");

    out
}
