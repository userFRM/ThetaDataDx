//! Checked-in generation for non-endpoint SDK/tool surfaces.
//!
//! `endpoint_surface.toml` remains the SSOT for validated request/response
//! endpoints. This module covers the remaining non-endpoint surfaces that still
//! need declarative checked-in generation: offline utilities, FPSS/unified
//! wrapper methods, and small public wrapper implementations.
//!
//! The TOML (`sdk_surface.toml`) declares the public method set, parameters,
//! target projections, and user-facing docs. This module is only the renderer:
//! it maps the semantic kinds declared in TOML onto language-specific code
//! templates.
//!
//! Layout:
//! * [`spec`] — TOML-backed data types + validation.
//! * [`common`] — shared renderer helpers (string/case helpers, type maps,
//!   generated-header, greek/param layouts).
//! * [`python`] / [`typescript`] / [`go`] / [`cpp`] / [`mcp`] / [`cli`] —
//!   one file per render target.

// Reason: shared between build.rs and generate_sdk_surfaces binary via #[path]; not all
// helpers are called from both entry points.
#![allow(dead_code, unused_imports)]

use std::path::Path;

mod cli;
mod common;
mod cpp;
mod go;
mod mcp;
mod python;
mod spec;
mod typescript;

use spec::{
    load_sdk_surface_spec, validate_spec, MethodSpec, MethodTarget, UtilitySpec, UtilityTarget,
};

struct GeneratedSourceFile {
    relative_path: &'static str,
    contents: String,
}

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

pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files()? {
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

fn render_sdk_generated_files() -> Result<Vec<GeneratedSourceFile>, Box<dyn std::error::Error>> {
    let spec = load_sdk_surface_spec()?;
    validate_spec(&spec)?;

    let python_unified_methods: Vec<&MethodSpec> = spec
        .methods
        .iter()
        .filter(|method| method.targets.contains(&MethodTarget::PythonUnified))
        .collect();
    let go_fpss_methods: Vec<&MethodSpec> = spec
        .methods
        .iter()
        .filter(|method| method.targets.contains(&MethodTarget::GoFpss))
        .collect();
    let cpp_fpss_methods: Vec<&MethodSpec> = spec
        .methods
        .iter()
        .filter(|method| method.targets.contains(&MethodTarget::CppFpss))
        .collect();
    let cpp_lifecycle_methods: Vec<&MethodSpec> = spec
        .methods
        .iter()
        .filter(|method| method.targets.contains(&MethodTarget::CppLifecycle))
        .collect();
    let ts_napi_methods: Vec<&MethodSpec> = spec
        .methods
        .iter()
        .filter(|method| method.targets.contains(&MethodTarget::TypescriptNapi))
        .collect();
    let python_utilities: Vec<&UtilitySpec> = spec
        .utilities
        .iter()
        .filter(|utility| utility.targets.contains(&UtilityTarget::Python))
        .collect();
    let go_utilities: Vec<&UtilitySpec> = spec
        .utilities
        .iter()
        .filter(|utility| utility.targets.contains(&UtilityTarget::Go))
        .collect();
    let cpp_utilities: Vec<&UtilitySpec> = spec
        .utilities
        .iter()
        .filter(|utility| utility.targets.contains(&UtilityTarget::Cpp))
        .collect();
    let mcp_utilities: Vec<&UtilitySpec> = spec
        .utilities
        .iter()
        .filter(|utility| utility.targets.contains(&UtilityTarget::Mcp))
        .collect();
    let cli_utilities: Vec<&UtilitySpec> = spec
        .utilities
        .iter()
        .filter(|utility| utility.targets.contains(&UtilityTarget::Cli))
        .collect();

    let go_fpss_methods_src =
        go::render_go_fpss_methods(&go_fpss_methods, &spec.go_ffi.tls_reader_markers);
    let go_utilities_src =
        go::render_go_utility_functions(&go_utilities, &spec.go_ffi.tls_reader_markers);

    Ok(vec![
        GeneratedSourceFile {
            relative_path: "sdks/python/src/streaming_methods.rs",
            contents: python::render_python_streaming_methods(&python_unified_methods),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/utility_functions.rs",
            contents: python::render_python_utility_functions(&python_utilities),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/streaming_methods.rs",
            contents: typescript::render_ts_streaming_methods(&ts_napi_methods),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/fpss_methods.go",
            contents: go_fpss_methods_src,
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/utilities.go",
            contents: go_utilities_src,
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/fpss.hpp.inc",
            contents: cpp::render_cpp_fpss_decls(&cpp_fpss_methods),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/fpss.cpp.inc",
            contents: cpp::render_cpp_fpss_defs(&cpp_fpss_methods),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/utilities.hpp.inc",
            contents: cpp::render_cpp_utility_decls(&cpp_utilities),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/utilities.cpp.inc",
            contents: cpp::render_cpp_utility_defs(&cpp_utilities),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/lifecycle.cpp.inc",
            contents: cpp::render_cpp_lifecycle_defs(&cpp_lifecycle_methods),
        },
        GeneratedSourceFile {
            relative_path: "tools/mcp/src/utilities.rs",
            contents: mcp::render_mcp_utilities(&mcp_utilities),
        },
        GeneratedSourceFile {
            relative_path: "tools/cli/src/utilities.rs",
            contents: cli::render_cli_utilities(&cli_utilities),
        },
    ])
}
