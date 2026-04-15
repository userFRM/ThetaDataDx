//! Orchestrator for checked-in SDK surface files.
//!
//! `write_sdk_generated_files` regenerates every projection from
//! `endpoint_surface.toml`; `check_sdk_generated_files` verifies the
//! checked-in tree matches what the current generator would emit. The list
//! returned by [`render_sdk_generated_files`] is the single place where each
//! per-language emitter is wired to its output path.

use std::path::Path;

use super::super::helpers::collect_builder_params;
use super::super::parser::load_endpoint_specs;
use super::{cli_validate, cpp, cpp_validate, ffi, go, go_validate, python, python_validate};

struct GeneratedSourceFile {
    relative_path: &'static str,
    contents: String,
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
    Ok(())
}

fn render_sdk_generated_files() -> Result<Vec<GeneratedSourceFile>, Box<dyn std::error::Error>> {
    let parsed = load_endpoint_specs()?;
    let builder_params = collect_builder_params(&parsed.endpoints);

    Ok(vec![
        GeneratedSourceFile {
            relative_path: "ffi/src/endpoint_request_options.rs",
            contents: ffi::render_ffi_endpoint_request_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "ffi/src/endpoint_with_options.rs",
            contents: ffi::render_ffi_with_options(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/endpoint_request_options.h.inc",
            contents: ffi::render_c_endpoint_request_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/endpoint_options.go",
            contents: go::render_go_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/historical.go",
            contents: go::render_go_historical(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/endpoint_with_options.h.inc",
            contents: go::render_go_endpoint_with_options_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/endpoint_request_options.h.inc",
            contents: ffi::render_c_endpoint_request_options(&builder_params),
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
            relative_path: "sdks/cpp/include/endpoint_with_options.h.inc",
            contents: cpp::render_c_endpoint_with_options_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/historical.cpp.inc",
            contents: cpp::render_cpp_historical_defs(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/historical_methods.rs",
            contents: python::render_python_historical_methods(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "scripts/validate_cli.py",
            contents: cli_validate::render_cli_validate(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "scripts/validate_python.py",
            contents: python_validate::render_python_validate(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/validate.go",
            contents: go_validate::render_go_validate(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/examples/validate.cpp",
            contents: cpp_validate::render_cpp_validate(&parsed.endpoints),
        },
    ])
}
