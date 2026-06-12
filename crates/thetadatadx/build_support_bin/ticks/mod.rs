//! Checked-in tick projections emitted by `generate_sdk_surfaces`.
//!
//! Shared schema loader + wire decoder live in `build_support/ticks/`.
//! This tree adds the per-language SDK projections (Python pyclass +
//! Arrow, TypeScript napi, C++ layout asserts, tdbe repr structs, CLI
//! raw headers) the build script never compiles.

pub(super) mod schema;

mod cli_headers;
mod cpp;
pub(super) mod idents;
mod layout;
mod python_arrow;
pub(super) mod python_classes;
mod rust_frames;
mod tdbe_structs;
mod typescript;

use std::path::Path;

use schema::Schema;

pub(super) struct GeneratedSourceFile {
    pub(super) relative_path: &'static str,
    pub(super) contents: String,
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
    // `python_classes` module doc-comment for the audit trail.
    Ok(vec![
        GeneratedSourceFile {
            // Arrow columnar pipeline: `pyclass_list_to_arrow_table` dispatcher
            // + typed Arrow schema map. Single alloc per column, zero-copy to
            // pyarrow via the Arrow C Data Interface.
            relative_path: "sdks/python/src/_generated/tick_arrow.rs",
            contents: python_arrow::render_python_tick_arrow(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/_generated/tick_classes.rs",
            contents: python_classes::render_python_tick_classes(&schema, snapshot_return_types),
        },
        GeneratedSourceFile {
            // Rust `frames/generated.rs` — per-tick-type `TicksPolarsExt` /
            // `TicksArrowExt` impls. Feature-gated (`polars` / `arrow`),
            // `include!`-ed from the hand-written `src/frames.rs` so the
            // trait definitions and the per-type impls compile in the
            // same unit.
            relative_path: "crates/thetadatadx/src/frames/generated.rs",
            contents: rust_frames::render_rust_frames(&schema),
        },
        GeneratedSourceFile {
            // tdbe tick structs -- `#[repr(C, align(N))]` definitions
            // emitted from the schema. Hand-written `tick.rs` `pub use`s
            // them and adds the macro applications + custom impls the
            // schema cannot express.
            relative_path: "crates/tdbe/src/types/generated/tick.rs",
            contents: tdbe_structs::render_tdbe_tick_structs(&schema),
        },
        GeneratedSourceFile {
            // tdbe layout asserts -- per-tick `size_of` / `align_of` /
            // `offset_of!` asserts emitted from the schema. Hand-written
            // `tick.rs` `include!`s this file inside `#[cfg(test)]`.
            // Adding a tick type to `tick_schema.toml` therefore picks up
            // ABI guard coverage automatically.
            relative_path: "crates/tdbe/src/types/generated/tick_layout_asserts.rs",
            contents: tdbe_structs::render_tdbe_layout_asserts(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/tick_layout_asserts.hpp.inc",
            contents: cpp::render_cpp_tick_layout_asserts(&schema),
        },
        GeneratedSourceFile {
            // C++ free-function flag-word accessors (`tdx::is_cancelled`,
            // ...). Included from `thetadx.hpp`; mirrors the Python
            // computed properties and TypeScript precomputed fields from
            // the same schema flag_accessors rows.
            relative_path: "sdks/cpp/include/tick_flag_accessors.hpp.inc",
            contents: cpp::render_cpp_tick_flag_accessors(&schema),
        },
        GeneratedSourceFile {
            relative_path: "tools/cli/src/raw_headers_generated.rs",
            contents: cli_headers::render_cli_raw_headers(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/_generated/tick_classes.rs",
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
