//! Orchestrator for checked-in SDK surface files.
//!
//! `write_sdk_generated_files` regenerates every projection from the
//! endpoint table; `check_sdk_generated_files` verifies the
//! checked-in tree matches what the current generator would emit. The list
//! returned by [`render_sdk_generated_files`] is the single place where each
//! per-language emitter is wired to its output path.

use std::path::Path;

use super::super::table::{self, REQUEST_OPTIONS};
use super::{config_accessors, cpp, cpp_validate, ffi, python_stub, python_validate};

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

/// Write the checked-in SDK surface artifacts generated from the endpoint table.
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
                "generated SDK surface '{}' is stale; run `cargo run -p thetadatadx-rs --bin generate_sdk_surfaces` to refresh",
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
                "generated region of '{}' is stale; run `cargo run -p thetadatadx-rs --bin generate_sdk_surfaces` to refresh",
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
    let endpoints = table::endpoints();
    table::check_proto_coverage(&endpoints)?;
    table::check_entries(&endpoints)?;
    super::super::test_fixtures::check_fixtures(&endpoints)?;

    Ok(vec![
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/include/config_accessors.hpp.inc",
            contents: config_accessors::render_cpp_config_accessors(),
        },
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/include/endpoint_request_options.h.inc",
            contents: ffi::render_c_endpoint_request_options(REQUEST_OPTIONS),
        },
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/include/market_data_stream.h.inc",
            contents: ffi::render_c_stream_decls(&endpoints),
        },
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/include/market_data_to_path.h.inc",
            contents: ffi::render_c_to_path_decls(&endpoints),
        },
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/include/endpoint_options.hpp.inc",
            contents: cpp::render_cpp_options(REQUEST_OPTIONS),
        },
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/include/market_data.hpp.inc",
            contents: cpp::render_cpp_market_data_decls(&endpoints),
        },
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/include/market_data_stream.hpp.inc",
            contents: cpp::render_cpp_stream_decls(&endpoints),
        },
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/include/endpoint_with_options.h.inc",
            contents: cpp::render_c_endpoint_with_options_decls(&endpoints),
        },
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/src/market_data.cpp.inc",
            contents: cpp::render_cpp_market_data_defs(&endpoints),
        },
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/src/market_data_stream.cpp.inc",
            contents: cpp::render_cpp_stream_defs(&endpoints),
        },
        GeneratedSourceFile {
            relative_path: "scripts/ci/check_python.py",
            contents: python_validate::render_python_validate(&endpoints),
        },
        GeneratedSourceFile {
            relative_path: "thetadatadx-cpp/examples/validate.cpp",
            contents: cpp_validate::render_cpp_validate(&endpoints),
        },
    ])
}

/// Files whose generated region is spliced between marker comments while
/// the rest of the file stays hand-maintained. Today this carries only the
/// Python type stub's `MarketDataView` endpoint surface, projected from the
/// endpoint table.
fn render_spliced_files() -> Result<Vec<SplicedSourceFile>, Box<dyn std::error::Error>> {
    let endpoints = table::endpoints();
    Ok(vec![SplicedSourceFile {
        relative_path: "thetadatadx-py/python/thetadatadx/__init__.pyi",
        begin_marker: python_stub::STUB_BEGIN_MARKER,
        end_marker: python_stub::STUB_END_MARKER,
        region: python_stub::render_python_market_data_view_stub(&endpoints),
    }])
}
