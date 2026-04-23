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
//! * [`go`] — Go FFI converters + public structs.
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

use schema::Schema;

mod cli_headers;
mod cpp;
mod go;
mod parser;
mod python_arrow;
mod python_classes;
mod rust_frames;
mod schema;
mod typescript;

pub(crate) use typescript::ts_tick_class_factory_name;

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
            relative_path: "sdks/go/tick_converters.go",
            contents: go::render_go_tick_converters(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/tick_ffi_sizes_generated.go",
            contents: go::render_go_tick_ffi_sizes(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/ffi_layout_generated_test.go",
            contents: go::render_go_tick_ffi_layout_tests(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/tick_layout_asserts.hpp.inc",
            contents: cpp::render_cpp_tick_layout_asserts(),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/tick_structs.go",
            contents: go::render_go_tick_structs(&schema),
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
pub(super) fn pyclass_name(type_name: &str) -> &'static str {
    match type_name {
        "EodTick" => "EodTick",
        "OhlcTick" => "OhlcTick",
        "TradeTick" => "TradeTick",
        "QuoteTick" => "QuoteTick",
        "TradeQuoteTick" => "TradeQuoteTick",
        "OpenInterestTick" => "OpenInterestTick",
        "MarketValueTick" => "MarketValueTick",
        "GreeksTick" => "GreeksTick",
        "IvTick" => "IvTick",
        "PriceTick" => "PriceTick",
        "CalendarDay" => "CalendarDay",
        "InterestRateTick" => "InterestRateTick",
        "OptionContract" => "OptionContract",
        other => panic!("unsupported Python pyclass type: {other}"),
    }
}
