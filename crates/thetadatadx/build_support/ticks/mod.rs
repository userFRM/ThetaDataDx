// Reason: shared between build.rs (which calls `generate`) and the
// `generate_sdk_surfaces` binary (which calls only the `*_sdk_generated_files`
// entry points) via `#[path]`. Not every helper is reachable from both
// entry points, so mark the whole module as allowed-dead once rather than
// annotating each helper individually.
#![allow(dead_code, unused_imports)]

//! Code-generate tick structs + parsers from `tick_schema.toml`.
//!
//! Layout:
//! * [`schema`] — TOML-backed types + `Schema` loader.
//! * [`parser`] — `decode_generated.rs` (wire → `Vec<Tick>`) emitter.
//! * [`python_arrow`] / [`python_classes`] — Python `#[pyclass]` struct + Arrow
//!   columnar pipeline.
//! * [`typescript`] — napi-rs `#[napi(object)]` tick types.
//! * [`layout`] — schema-driven `#[repr(C)]` size/align/offset math shared
//!   by the C++ static-assert emitter and the `tdbe` layout-guard emitter.
//! * [`cli_headers`] — `tools/cli/src/raw_headers_generated.rs` constants.
//!
//! # Removed surfaces (audit trail)
//!
//! * `sdks/python/src/tick_columnar.rs` — dict-of-lists `*_ticks_to_columnar`
//!   helpers returning `Py<PyDict>`. Replaced by typed pyclass lists + the
//!   Arrow columnar pipeline. (PR #365 dropped PyDict returns on historical
//!   endpoints; PR #379 finished the Arrow cutover.)
//! * Per-type `*_ticks_to_arrow_batch(&[tick::T])` fast-path helpers — only
//!   reached by the `stock_history_*_df` convenience wrappers, which were
//!   themselves dropped for SSOT purity. The sole public DataFrame path is
//!   now the `pyclass_list_to_arrow_table` trait dispatcher. (PR #379.)
//! * `sdks/typescript/src/*_to_columnar` serde_json converters — superseded
//!   by the typed `#[napi(object)]` class-vec path (`render_ts_tick_classes`).
//!   (PR #366.)
//! * `sdks/typescript/src/types.ts` — hand-imported counterpart to the
//!   deleted `*_to_columnar` helpers. Typed shapes now come from `index.d.ts`
//!   emitted by napi-rs from the `#[napi(object)]` tick classes. (PR #368.)

use std::path::Path;

pub(super) use schema::render_for_type;
use schema::Schema;

mod cli_headers;
mod cpp;
mod layout;
mod parser;
mod python_arrow;
mod python_classes;
mod rust_frames;
mod schema;
mod tdbe_structs;
mod typescript;

pub struct GeneratedSourceFile {
    pub relative_path: &'static str,
    pub contents: String,
}

pub fn generate() -> Result<(), Box<dyn std::error::Error>> {
    parser::generate()
}

pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let snapshot_return_types = super::endpoints::snapshot_return_types()?;
    for file in render_sdk_generated_files(&snapshot_return_types)? {
        let path = repo_root.join(file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, file.contents)?;
    }
    Ok(())
}

pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let snapshot_return_types = super::endpoints::snapshot_return_types()?;
    for file in render_sdk_generated_files(&snapshot_return_types)? {
        let path = repo_root.join(file.relative_path);
        let actual = std::fs::read_to_string(&path)?;
        if actual.replace("\r\n", "\n") != file.contents {
            return Err(format!(
                "generated SDK surface '{}' is stale; run `cargo run -p thetadatadx --bin generate_sdk_surfaces` to refresh",
                file.relative_path
            )
            .into());
        }
    }
    Ok(())
}

fn render_sdk_generated_files(
    snapshot_return_types: &std::collections::HashSet<String>,
) -> Result<Vec<GeneratedSourceFile>, Box<dyn std::error::Error>> {
    let schema = schema::load_schema()?;
    // Note: `tick_columnar.rs` is intentionally NOT emitted — see the
    // "Removed surfaces" block in the module doc-comment for the audit trail.
    Ok(vec![
        GeneratedSourceFile {
            // Arrow columnar pipeline: `pyclass_list_to_arrow_table` dispatcher
            // + typed Arrow schema map. Single alloc per column, zero-copy to
            // pyarrow via the Arrow C Data Interface.
            relative_path: "sdks/python/src/tick_arrow.rs",
            contents: python_arrow::render_python_tick_arrow(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/tick_classes.rs",
            contents: python_classes::render_python_tick_classes(&schema, snapshot_return_types),
        },
        GeneratedSourceFile {
            // Rust `frames_generated.rs` — per-tick-type `TicksPolarsExt` /
            // `TicksArrowExt` impls. Feature-gated (`polars` / `arrow`),
            // `include!`-ed from the hand-written `src/frames.rs` so the
            // trait definitions and the per-type impls compile in the
            // same unit.
            relative_path: "crates/thetadatadx/src/frames_generated.rs",
            contents: rust_frames::render_rust_frames(&schema),
        },
        GeneratedSourceFile {
            // tdbe tick structs -- `#[repr(C, align(N))]` definitions
            // emitted from the schema. Hand-written `tick.rs` `pub use`s
            // them and adds the macro applications + custom impls the
            // schema cannot express.
            relative_path: "crates/tdbe/src/types/tick_generated.rs",
            contents: tdbe_structs::render_tdbe_tick_structs(&schema),
        },
        GeneratedSourceFile {
            // tdbe layout asserts -- per-tick `size_of` / `align_of` /
            // `offset_of!` asserts emitted from the schema. Hand-written
            // `tick.rs` `include!`s this file inside `#[cfg(test)]`.
            // Adding a tick type to `tick_schema.toml` therefore picks up
            // ABI guard coverage automatically.
            relative_path: "crates/tdbe/src/types/tick_layout_asserts_generated.rs",
            contents: tdbe_structs::render_tdbe_layout_asserts(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/tick_layout_asserts.hpp.inc",
            contents: cpp::render_cpp_tick_layout_asserts(&schema),
        },
        GeneratedSourceFile {
            relative_path: "tools/cli/src/raw_headers_generated.rs",
            contents: cli_headers::render_cli_raw_headers(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/tick_classes.rs",
            contents: typescript::render_ts_tick_classes(&schema),
        },
    ])
}

/// Stable sorted list of tick type names — shared by every renderer.
pub(super) fn sorted_type_names(schema: &Schema) -> Vec<&str> {
    let mut names = schema.types.keys().map(String::as_str).collect::<Vec<_>>();
    names.sort();
    names
}

/// Stable pyclass / typed-struct name for a schema type. Shared across
/// the Python emitters (struct, Arrow reader, and list converter) so all
/// surfaces see the same class identifier.
///
/// TOML-driven via `[types.X.render].pyclass` -- adding a tick type means
/// adding the schema row, not editing this helper. The OnceLock makes the
/// lookup `'static` because the schema is closed at build time.
pub(super) fn pyclass_name(type_name: &str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        let schema = schema::load_schema()
            .unwrap_or_else(|e| panic!("failed to load tick_schema.toml for pyclass_name: {e}"));
        // Leak the strings -- build-time only, bounded by the closed set of
        // tick types in tick_schema.toml. Avoids returning borrowed refs out
        // of the closure.
        schema
            .types
            .into_iter()
            .map(|(name, def)| {
                let key: &'static str = Box::leak(name.into_boxed_str());
                let value: &'static str = Box::leak(def.render.pyclass.into_boxed_str());
                (key, value)
            })
            .collect()
    });
    map.get(type_name).copied().unwrap_or_else(|| {
        let mut keys: Vec<&str> = map.keys().copied().collect();
        keys.sort();
        panic!(
            "unsupported Python pyclass type '{type_name}'; available: {}",
            keys.join(", ")
        )
    })
}
