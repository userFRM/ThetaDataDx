//! Orchestrator for checked-in SDK surface files.
//!
//! `write_sdk_generated_files` regenerates every projection from
//! `endpoint_surface.toml`; `check_sdk_generated_files` verifies the
//! checked-in tree matches what the current generator would emit. The list
//! returned by [`render_sdk_generated_files`] is the single place where each
//! per-language emitter is wired to its output path.

use std::path::Path;

use super::super::enum_projection::load_enum_projections;
use super::super::fixture_validation::validate_test_fixtures;
use super::super::parser::load_endpoint_specs;
use super::super::sdk_helpers::collect_builder_params;
use super::super::test_fixtures::load_test_fixtures;
use super::{
    cli_validate, config_accessors, cpp, cpp_validate, enums, ffi, python, python_stub,
    python_validate, typescript,
};

struct GeneratedSourceFile {
    relative_path: &'static str,
    contents: String,
}

/// Fully checked-in file that carries one generated region between two
/// marker comments while the rest of the file stays hand-maintained.
struct SplicedSourceFile {
    relative_path: &'static str,
    begin_marker: &'static str,
    end_marker: &'static str,
    /// The freshly generated region (markers included).
    region: String,
}

/// Write the checked-in SDK surface artifacts generated from `endpoint_surface.toml`.
pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files()? {
        let path = repo_root.join(file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, file.contents)?;
    }
    for file in render_spliced_files()? {
        let path = repo_root.join(file.relative_path);
        let current = std::fs::read_to_string(&path)?;
        let spliced = splice_region(&current, &file)?;
        std::fs::write(path, spliced)?;
    }
    Ok(())
}

/// Verify the checked-in SDK surface artifacts match the generated output.
pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files()? {
        let path = repo_root.join(file.relative_path);
        let actual = std::fs::read_to_string(&path)?;
        // Normalize \r\n → \n so Windows checkouts don't false-positive.
        let actual_normalized = actual.replace("\r\n", "\n");
        if actual_normalized != file.contents {
            return Err(format!(
                "generated SDK surface '{}' is stale; run `cargo run -p thetadatadx --bin generate_sdk_surfaces` to refresh",
                file.relative_path
            )
            .into());
        }
    }
    for file in render_spliced_files()? {
        let path = repo_root.join(file.relative_path);
        let actual = std::fs::read_to_string(&path)?.replace("\r\n", "\n");
        let expected = splice_region(&actual, &file)?;
        if actual != expected {
            return Err(format!(
                "generated region of '{}' is stale; run `cargo run -p thetadatadx --bin generate_sdk_surfaces` to refresh",
                file.relative_path
            )
            .into());
        }
    }
    Ok(())
}

/// Replace the marked region of `current` with the freshly generated one,
/// preserving the file's leading and trailing hand-written content and the
/// exact indentation in front of each marker.
fn splice_region(
    current: &str,
    file: &SplicedSourceFile,
) -> Result<String, Box<dyn std::error::Error>> {
    let normalized = current.replace("\r\n", "\n");
    let begin = normalized.find(file.begin_marker).ok_or_else(|| {
        format!(
            "begin marker not found in '{}'; expected line: {}",
            file.relative_path, file.begin_marker
        )
    })?;
    let after_end_marker = normalized.find(file.end_marker).ok_or_else(|| {
        format!(
            "end marker not found in '{}'; expected line: {}",
            file.relative_path, file.end_marker
        )
    })? + file.end_marker.len();
    let mut out = String::with_capacity(normalized.len());
    out.push_str(&normalized[..begin]);
    out.push_str(&file.region);
    out.push_str(&normalized[after_end_marker..]);
    Ok(out)
}

fn render_sdk_generated_files() -> Result<Vec<GeneratedSourceFile>, Box<dyn std::error::Error>> {
    let parsed = load_endpoint_specs()?;
    let enum_projections = load_enum_projections()?;
    // Fixtures power the live-validator matrix only — kept out of the
    // shared `load_endpoint_specs` path so the build script doesn't pay
    // the upstream OpenAPI snapshot dependency (not in the Python sdist).
    // Every fixture consumer flows through here, so validating at this
    // seam catches every drift case with full per-endpoint blast radius.
    let fixtures = load_test_fixtures()?;
    validate_test_fixtures(&fixtures, &parsed.endpoints)?;
    let builder_params = collect_builder_params(&parsed.endpoints);

    Ok(vec![
        GeneratedSourceFile {
            relative_path: "crates/thetadatadx/src/tdbe/types/generated/enums_endpoint.rs",
            contents: enums::render_tdbe_enums(&enum_projections),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/_generated/enums_generated.rs",
            contents: enums::render_python_enums(&enum_projections),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/_generated/enums_generated.rs",
            contents: enums::render_typescript_enums(&enum_projections),
        },
        GeneratedSourceFile {
            relative_path: "ffi/src/endpoint_request_options.rs",
            contents: ffi::render_ffi_endpoint_request_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "ffi/src/endpoint_with_options.rs",
            contents: ffi::render_ffi_with_options(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "ffi/src/endpoint_stream.rs",
            contents: ffi::render_ffi_stream_endpoints(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "ffi/src/config_accessors.rs",
            contents: config_accessors::render_ffi_config_accessors()?,
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/config_accessors.hpp.inc",
            contents: config_accessors::render_cpp_config_accessors()?,
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/_generated/config_accessors.rs",
            contents: config_accessors::render_python_config_accessors()?,
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/_generated/config_accessors.rs",
            contents: config_accessors::render_typescript_config_accessors()?,
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/endpoint_request_options.h.inc",
            contents: ffi::render_c_endpoint_request_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/historical_stream.h.inc",
            contents: ffi::render_c_stream_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/endpoint_options.hpp.inc",
            contents: cpp::render_cpp_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/historical.hpp.inc",
            contents: cpp::render_cpp_historical_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/historical_stream.hpp.inc",
            contents: cpp::render_cpp_stream_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/endpoint_with_options.h.inc",
            contents: cpp::render_c_endpoint_with_options_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/historical.cpp.inc",
            contents: cpp::render_cpp_historical_defs(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/historical_stream.cpp.inc",
            contents: cpp::render_cpp_stream_defs(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/_generated/historical_methods.rs",
            contents: python::render_python_historical_methods(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/_generated/decode_bench.rs",
            contents: python::render_python_decode_bench(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/_generated/historical_methods.rs",
            contents: typescript::render_typescript_historical_methods(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "scripts/ci/check_cli.py",
            contents: cli_validate::render_cli_validate(&parsed.endpoints, &fixtures),
        },
        GeneratedSourceFile {
            relative_path: "scripts/ci/check_python.py",
            contents: python_validate::render_python_validate(&parsed.endpoints, &fixtures),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/examples/validate.cpp",
            contents: cpp_validate::render_cpp_validate(&parsed.endpoints, &fixtures),
        },
    ])
}

/// Files whose generated region is spliced between marker comments while
/// the rest of the file stays hand-maintained. Today this carries only the
/// Python type stub's `HistoricalView` endpoint surface, projected from the
/// same `endpoint_surface.toml` that drives the runtime `#[pymethods]`.
fn render_spliced_files() -> Result<Vec<SplicedSourceFile>, Box<dyn std::error::Error>> {
    let parsed = load_endpoint_specs()?;
    Ok(vec![SplicedSourceFile {
        relative_path: "sdks/python/python/thetadatadx/__init__.pyi",
        begin_marker: python_stub::STUB_BEGIN_MARKER,
        end_marker: python_stub::STUB_END_MARKER,
        region: python_stub::render_python_historical_view_stub(&parsed.endpoints),
    }])
}
