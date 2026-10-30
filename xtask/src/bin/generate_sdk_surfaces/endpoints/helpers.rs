//! Cross-cutting helpers shared by every renderer.
//!
//! Holds param classification (`is_*` filters) and docstring composition.
//! Per-language SDK projection helpers (arg/literal renderers, FFI option
//! mappers, CLI command derivations, etc.) live in `sdk_helpers.rs`.
//!
//! Anything that emits a multi-line chunk of target-language code belongs
//! in `sdk_render/` or `docs_render/`, not here.

use heck::ToUpperCamelCase;

use super::table::GeneratedEndpoint;

// ───────────────────────── Param classification ────────────────────────────

/// True when the endpoint returns a flat list of strings.
pub(super) fn is_simple_list_endpoint(endpoint: &GeneratedEndpoint) -> bool {
    endpoint.shape == "list"
}

/// True for the raw server streams.
pub(super) fn is_streaming_endpoint(endpoint: &GeneratedEndpoint) -> bool {
    endpoint.shape == "stream"
}

/// True for the snapshot and calendar endpoints, which return a bounded
/// handful of rows in one response, so there is nothing to drain chunk by
/// chunk.
pub(super) fn is_snapshot_endpoint(endpoint: &GeneratedEndpoint) -> bool {
    endpoint.shape == "buffered"
}

/// True when an endpoint gets a server-stream terminal (`request.stream`
/// surfaced per binding): the multi-day and full-universe history pulls.
pub(super) fn endpoint_streams(endpoint: &GeneratedEndpoint) -> bool {
    endpoint.shape == "chunked"
}

/// True when an endpoint gets a server-stream terminal on the C ABI and the
/// C++ wrapper.
///
/// Narrows [`endpoint_streams`] by the constraint the zero-copy chunk
/// callback imposes: the per-chunk pointer is the SAME flat `#[repr(C)]` tick
/// the buffered tick array exposes, so the C side reinterprets it with no
/// re-marshaling. `OptionContracts` is the lone streaming return type whose
/// core row carries an owned `String` (the contract symbol) — its FFI
/// representation (`ThetaDataDxOptionContract`, a `*const c_char` symbol) is a
/// distinct layout the buffered path marshals row-by-row. That per-row
/// `CString` allocation is exactly the heap traffic the streaming path exists
/// to avoid, and it has no stable lifetime inside a borrowed-slice callback,
/// so the contract-discovery endpoint keeps its buffered C-ABI form only. The
/// managed bindings (Python / TypeScript) marshal per row in GC'd memory and
/// stream it regardless.
pub(super) fn endpoint_streams_repr_c_ticks(endpoint: &GeneratedEndpoint) -> bool {
    endpoint_streams(endpoint) && endpoint.return_type != "OptionContracts"
}

// ───────────────────────── Docstring composition ───────────────────────────
//
// The first doc line is the short description that heads every method's
// docs; the lines after it are the vendor's richer prose (feed-source notes,
// subscription tier behavior, parameter defaults). Both sync and async
// methods, and the fluent builder's `arrow()` / `list()` / `polars()` /
// `pandas()` terminals, pull from the same composed string so no variant can
// drift.

/// Compose the full doc body for an endpoint: its doc lines, then the
/// defaults block.
pub(super) fn compose_endpoint_doc(endpoint: &GeneratedEndpoint) -> String {
    let mut body = endpoint.doc.join("\n");
    // A doc of several lines ends with a line break.
    if endpoint.doc.len() > 1 {
        body.push('\n');
    }
    let defaults_block = render_param_defaults_block(endpoint);
    if !defaults_block.is_empty() {
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.push('\n');
        body.push_str(&defaults_block);
    }
    body
}

/// Render the "Defaults (upstream)" block surfacing every param whose
/// `default` is set in the endpoint table. Single SSOT origin so
/// `help()` (Python), JSDoc hover (TypeScript), and `cargo doc` (Rust)
/// all agree. String defaults render with quotes; numeric / bool
/// defaults render bare. Empty output when no param has a default.
fn render_param_defaults_block(endpoint: &GeneratedEndpoint) -> String {
    use std::fmt::Write as _;
    let mut rows: Vec<(&str, String)> = Vec::new();
    for param in endpoint.params {
        let Some(default) = param.default else {
            continue;
        };
        let value = match param.param_type {
            "Bool" | "Int" | "Float" => default.to_string(),
            _ => format!("\"{default}\""),
        };
        rows.push((param.name, value));
    }
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from("Defaults (upstream):\n");
    for (name, value) in rows {
        writeln!(out, "- `{name}`: `{value}`").unwrap();
    }
    out
}

// ───────────────────────── Casing ────────────────────────────────────────────

/// Converts a `snake_case` identifier to `PascalCase`.
pub(super) fn to_pascal_case(value: &str) -> String {
    value.to_upper_camel_case()
}

// ───────────────────────── Per-language type tables ─────────────────────────

// ───────────────────────── Builder / FFI option tables ─────────────────────

// ───────────────────────── SDK method arg declarations ─────────────────────

// ───────────────────────── Validator arg literals ──────────────────────────

// ───────────────────────── CLI / validator scaffolding ─────────────────────
