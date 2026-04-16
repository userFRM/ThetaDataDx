//! Checked-in generation for non-endpoint SDK/tool surfaces.
//!
//! `endpoint_surface.toml` remains the SSOT for validated request/response
//! endpoints. This module covers the remaining non-endpoint surfaces that still
//! need declarative checked-in generation: offline utilities, FPSS/unified
//! wrapper methods, and small public wrapper implementations.
//!
//! The TOML (`sdk_surface.toml`) declares the public method set, parameters,
//! target projections, and user-facing docs. This file is only the renderer:
//! it maps the semantic kinds declared in TOML onto language-specific code
//! templates.

use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SdkSurfaceSpec {
    version: u32,
    #[serde(default)]
    methods: Vec<MethodSpec>,
    #[serde(default)]
    utilities: Vec<UtilitySpec>,
    /// Go-side FFI configuration. Holds the TLS-reader marker SSOT that
    /// drives both `inject_os_thread_pin` (build-time body rewriter) and
    /// the generated `tlsReaderMarkers` list consumed by the static-audit
    /// test in `sdks/go/timeout_pin_test.go`.
    #[serde(default)]
    go_ffi: GoFfiSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct MethodSpec {
    name: String,
    kind: MethodKind,
    doc: String,
    targets: Vec<MethodTarget>,
    #[serde(default)]
    params: Vec<ParamSpec>,
    #[serde(default)]
    runtime_call: Option<String>,
    #[serde(default)]
    ffi_call: Option<String>,
    #[serde(default)]
    config_variant: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MethodKind {
    StartStreaming,
    IsStreaming,
    StockContractCall,
    OptionContractCall,
    FullCall,
    ContractMap,
    ContractLookup,
    ActiveSubscriptions,
    NextEvent,
    Reconnect,
    StopStreaming,
    Shutdown,
    IsAuthenticated,
    FpssConnect,
    CredentialsFromFile,
    CredentialsFromEmail,
    ConfigConstructor,
    ClientConnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MethodTarget {
    PythonUnified,
    GoFpss,
    CppFpss,
    CppLifecycle,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct UtilitySpec {
    name: String,
    kind: UtilityKind,
    doc: String,
    targets: Vec<UtilityTarget>,
    #[serde(default)]
    params: Vec<ParamSpec>,
    #[serde(default)]
    cli_name: Option<String>,
    #[serde(default)]
    cli_about: Option<String>,
    #[serde(default)]
    mcp_description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UtilityKind {
    Auth,
    Ping,
    AllGreeks,
    ImpliedVolatility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UtilityTarget {
    Python,
    Go,
    Cpp,
    Mcp,
    Cli,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParamSpec {
    name: String,
    #[serde(rename = "type")]
    param_type: ParamType,
    doc: String,
    #[serde(default)]
    cli_name: Option<String>,
    #[serde(default)]
    mcp_name: Option<String>,
    #[serde(default)]
    mcp_description: Option<String>,
    #[serde(default)]
    enum_values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ParamType {
    String,
    F64,
    I32,
    U64,
    CredentialsRef,
    ConfigRef,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoFfiSpec {
    #[serde(default)]
    tls_reader_markers: Vec<TlsReaderMarker>,
}

/// A substring that, when present on a Go source line, identifies an FFI
/// thread-local error read. The enclosing function must have executed
/// `runtime.LockOSThread()` + `defer runtime.UnlockOSThread()` before
/// reaching such a line. See `sdk_surface.toml` for the authoritative
/// description of each marker.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TlsReaderMarker {
    substring: String,
    description: String,
}

struct GeneratedSourceFile {
    relative_path: &'static str,
    contents: String,
}

pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files(repo_root)? {
        let path = repo_root.join(file.relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, file.contents)?;
    }
    Ok(())
}

pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files(repo_root)? {
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
    repo_root: &Path,
) -> Result<Vec<GeneratedSourceFile>, Box<dyn std::error::Error>> {
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
        render_go_fpss_methods(&go_fpss_methods, &spec.go_ffi.tls_reader_markers);
    let go_utilities_src =
        render_go_utility_functions(&go_utilities, &spec.go_ffi.tls_reader_markers);

    Ok(vec![
        GeneratedSourceFile {
            relative_path: "sdks/python/src/streaming_methods.rs",
            contents: render_python_streaming_methods(&python_unified_methods),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/utility_functions.rs",
            contents: render_python_utility_functions(&python_utilities),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/fpss_methods.go",
            contents: go_fpss_methods_src.clone(),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/utilities.go",
            contents: go_utilities_src.clone(),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/timeout_pin_generated_test.go",
            contents: render_go_timeout_pin_generated_test(
                repo_root,
                &spec.go_ffi.tls_reader_markers,
                &[
                    ("fpss_methods.go", go_fpss_methods_src.as_str()),
                    ("utilities.go", go_utilities_src.as_str()),
                ],
            )?,
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/fpss.hpp.inc",
            contents: render_cpp_fpss_decls(&cpp_fpss_methods),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/fpss.cpp.inc",
            contents: render_cpp_fpss_defs(&cpp_fpss_methods),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/utilities.hpp.inc",
            contents: render_cpp_utility_decls(&cpp_utilities),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/utilities.cpp.inc",
            contents: render_cpp_utility_defs(&cpp_utilities),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/lifecycle.cpp.inc",
            contents: render_cpp_lifecycle_defs(&cpp_lifecycle_methods),
        },
        GeneratedSourceFile {
            relative_path: "tools/mcp/src/utilities.rs",
            contents: render_mcp_utilities(&mcp_utilities),
        },
        GeneratedSourceFile {
            relative_path: "tools/cli/src/utilities.rs",
            contents: render_cli_utilities(&cli_utilities),
        },
    ])
}

fn load_sdk_surface_spec() -> Result<SdkSurfaceSpec, Box<dyn std::error::Error>> {
    let spec_path = "sdk_surface.toml";
    let spec_str = std::fs::read_to_string(spec_path)?;
    let spec: SdkSurfaceSpec = toml::from_str(&spec_str)?;
    println!("cargo:rerun-if-changed={spec_path}");
    Ok(spec)
}

fn validate_spec(spec: &SdkSurfaceSpec) -> Result<(), Box<dyn std::error::Error>> {
    if spec.version != 2 {
        return Err(format!("unsupported sdk_surface.toml version: {}", spec.version).into());
    }

    let mut seen_methods = std::collections::HashSet::new();
    for method in &spec.methods {
        if !seen_methods.insert(method.name.as_str()) {
            return Err(format!("duplicate method '{}'", method.name).into());
        }
        validate_method_spec(method)?;
    }

    let mut seen_utilities = std::collections::HashSet::new();
    for utility in &spec.utilities {
        if !seen_utilities.insert(utility.name.as_str()) {
            return Err(format!("duplicate utility '{}'", utility.name).into());
        }
        validate_utility_spec(utility)?;
    }

    let mut seen_tls_markers = std::collections::HashSet::new();
    for marker in &spec.go_ffi.tls_reader_markers {
        if marker.substring.trim().is_empty() {
            return Err(
                "sdk_surface.toml go_ffi.tls_reader_markers contains an empty substring".into(),
            );
        }
        if !seen_tls_markers.insert(marker.substring.as_str()) {
            return Err(format!(
                "sdk_surface.toml go_ffi.tls_reader_markers contains duplicate substring '{}'",
                marker.substring
            )
            .into());
        }
    }

    Ok(())
}

fn validate_method_spec(method: &MethodSpec) -> Result<(), Box<dyn std::error::Error>> {
    if method.targets.is_empty() {
        return Err(format!("method '{}' must declare at least one target", method.name).into());
    }
    ensure_unique_strings(
        &format!("method '{}' targets", method.name),
        method.targets.iter().map(|target| format!("{target:?}")),
    )?;
    ensure_unique_strings(
        &format!("method '{}' params", method.name),
        method.params.iter().map(|param| param.name.clone()),
    )?;

    match method.kind {
        MethodKind::StartStreaming => {
            expect_method_name(method, "start_streaming")?;
            expect_exact_method_targets(method, &[MethodTarget::PythonUnified])?;
            expect_param_layout(method, &[])?;
            expect_none(&method.runtime_call, &method.name, "runtime_call")?;
            expect_none(&method.ffi_call, &method.name, "ffi_call")?;
            expect_none(&method.config_variant, &method.name, "config_variant")?;
        }
        MethodKind::IsStreaming => {
            expect_method_name(method, "is_streaming")?;
            expect_exact_method_targets(method, &[MethodTarget::PythonUnified])?;
            expect_param_layout(method, &[])?;
            expect_none(&method.runtime_call, &method.name, "runtime_call")?;
            expect_none(&method.ffi_call, &method.name, "ffi_call")?;
            expect_none(&method.config_variant, &method.name, "config_variant")?;
        }
        MethodKind::StockContractCall => {
            expect_method_targets_subset(
                method,
                &[
                    MethodTarget::PythonUnified,
                    MethodTarget::GoFpss,
                    MethodTarget::CppFpss,
                ],
            )?;
            expect_param_layout(method, &[("symbol", ParamType::String)])?;
            expect_some(&method.runtime_call, &method.name, "runtime_call")?;
            expect_some(&method.ffi_call, &method.name, "ffi_call")?;
            expect_none(&method.config_variant, &method.name, "config_variant")?;
        }
        MethodKind::OptionContractCall => {
            expect_method_targets_subset(
                method,
                &[
                    MethodTarget::PythonUnified,
                    MethodTarget::GoFpss,
                    MethodTarget::CppFpss,
                ],
            )?;
            expect_param_layout(
                method,
                &[
                    ("symbol", ParamType::String),
                    ("expiration", ParamType::String),
                    ("strike", ParamType::String),
                    ("right", ParamType::String),
                ],
            )?;
            expect_some(&method.runtime_call, &method.name, "runtime_call")?;
            expect_some(&method.ffi_call, &method.name, "ffi_call")?;
            expect_none(&method.config_variant, &method.name, "config_variant")?;
        }
        MethodKind::FullCall => {
            expect_method_targets_subset(
                method,
                &[
                    MethodTarget::PythonUnified,
                    MethodTarget::GoFpss,
                    MethodTarget::CppFpss,
                ],
            )?;
            expect_param_layout(method, &[("sec_type", ParamType::String)])?;
            expect_some(&method.runtime_call, &method.name, "runtime_call")?;
            expect_some(&method.ffi_call, &method.name, "ffi_call")?;
            expect_none(&method.config_variant, &method.name, "config_variant")?;
        }
        MethodKind::ContractMap => {
            expect_method_name(method, "contract_map")?;
            expect_method_targets_subset(
                method,
                &[
                    MethodTarget::PythonUnified,
                    MethodTarget::GoFpss,
                    MethodTarget::CppFpss,
                ],
            )?;
            expect_param_layout(method, &[])?;
        }
        MethodKind::ContractLookup => {
            expect_method_name(method, "contract_lookup")?;
            expect_method_targets_subset(
                method,
                &[
                    MethodTarget::PythonUnified,
                    MethodTarget::GoFpss,
                    MethodTarget::CppFpss,
                ],
            )?;
            expect_param_layout(method, &[("id", ParamType::I32)])?;
        }
        MethodKind::ActiveSubscriptions => {
            expect_method_name(method, "active_subscriptions")?;
            expect_method_targets_subset(
                method,
                &[
                    MethodTarget::PythonUnified,
                    MethodTarget::GoFpss,
                    MethodTarget::CppFpss,
                ],
            )?;
            expect_param_layout(method, &[])?;
        }
        MethodKind::NextEvent => {
            expect_method_name(method, "next_event")?;
            expect_method_targets_subset(
                method,
                &[
                    MethodTarget::PythonUnified,
                    MethodTarget::GoFpss,
                    MethodTarget::CppFpss,
                ],
            )?;
            expect_param_layout(method, &[("timeout_ms", ParamType::U64)])?;
        }
        MethodKind::Reconnect => {
            expect_method_name(method, "reconnect")?;
            expect_method_targets_subset(
                method,
                &[
                    MethodTarget::PythonUnified,
                    MethodTarget::GoFpss,
                    MethodTarget::CppFpss,
                ],
            )?;
            expect_param_layout(method, &[])?;
        }
        MethodKind::StopStreaming => {
            expect_method_name(method, "stop_streaming")?;
            expect_exact_method_targets(method, &[MethodTarget::PythonUnified])?;
            expect_param_layout(method, &[])?;
        }
        MethodKind::Shutdown => {
            expect_method_name(method, "shutdown")?;
            expect_method_targets_subset(
                method,
                &[
                    MethodTarget::PythonUnified,
                    MethodTarget::GoFpss,
                    MethodTarget::CppFpss,
                ],
            )?;
            expect_param_layout(method, &[])?;
        }
        MethodKind::IsAuthenticated => {
            expect_method_name(method, "is_authenticated")?;
            expect_method_targets_subset(method, &[MethodTarget::GoFpss, MethodTarget::CppFpss])?;
            expect_param_layout(method, &[])?;
        }
        MethodKind::FpssConnect => {
            expect_method_name(method, "connect")?;
            expect_exact_method_targets(method, &[MethodTarget::CppFpss])?;
            expect_param_layout(
                method,
                &[
                    ("creds", ParamType::CredentialsRef),
                    ("config", ParamType::ConfigRef),
                ],
            )?;
        }
        MethodKind::CredentialsFromFile => {
            expect_method_name(method, "credentials_from_file")?;
            expect_exact_method_targets(method, &[MethodTarget::CppLifecycle])?;
            expect_param_layout(method, &[("path", ParamType::String)])?;
        }
        MethodKind::CredentialsFromEmail => {
            expect_method_name(method, "credentials_from_email")?;
            expect_exact_method_targets(method, &[MethodTarget::CppLifecycle])?;
            expect_param_layout(
                method,
                &[
                    ("email", ParamType::String),
                    ("password", ParamType::String),
                ],
            )?;
        }
        MethodKind::ConfigConstructor => {
            let variant = expect_some(&method.config_variant, &method.name, "config_variant")?;
            expect_method_name(method, &format!("config_{variant}"))?;
            expect_exact_method_targets(method, &[MethodTarget::CppLifecycle])?;
            expect_param_layout(method, &[])?;
            if !matches!(variant, "production" | "dev" | "stage") {
                return Err(format!(
                    "method '{}' has unsupported config_variant '{}'",
                    method.name, variant
                )
                .into());
            }
        }
        MethodKind::ClientConnect => {
            expect_method_name(method, "client_connect")?;
            expect_exact_method_targets(method, &[MethodTarget::CppLifecycle])?;
            expect_param_layout(
                method,
                &[
                    ("creds", ParamType::CredentialsRef),
                    ("config", ParamType::ConfigRef),
                ],
            )?;
        }
    }

    for param in &method.params {
        if !param.enum_values.is_empty() && param.param_type != ParamType::String {
            return Err(format!(
                "method '{}' param '{}' declares enum_values but is not a string",
                method.name, param.name
            )
            .into());
        }
    }

    Ok(())
}

fn validate_utility_spec(utility: &UtilitySpec) -> Result<(), Box<dyn std::error::Error>> {
    if utility.targets.is_empty() {
        return Err(format!(
            "utility '{}' must declare at least one target",
            utility.name
        )
        .into());
    }
    ensure_unique_strings(
        &format!("utility '{}' targets", utility.name),
        utility.targets.iter().map(|target| format!("{target:?}")),
    )?;
    ensure_unique_strings(
        &format!("utility '{}' params", utility.name),
        utility.params.iter().map(|param| param.name.clone()),
    )?;
    for param in &utility.params {
        if !param.enum_values.is_empty() && param.param_type != ParamType::String {
            return Err(format!(
                "utility '{}' param '{}' declares enum_values but is not a string",
                utility.name, param.name
            )
            .into());
        }
    }

    match utility.kind {
        UtilityKind::Auth => {
            if utility.name != "auth" {
                return Err("utility kind Auth must use name 'auth'".into());
            }
            expect_exact_utility_targets(utility, &[UtilityTarget::Cli])?;
            expect_utility_param_layout(utility, &[])?;
        }
        UtilityKind::Ping => {
            if utility.name != "ping" {
                return Err("utility kind Ping must use name 'ping'".into());
            }
            expect_exact_utility_targets(utility, &[UtilityTarget::Mcp])?;
            expect_utility_param_layout(utility, &[])?;
        }
        UtilityKind::AllGreeks => {
            if utility.name != "all_greeks" {
                return Err("utility kind AllGreeks must use name 'all_greeks'".into());
            }
            expect_utility_targets_subset(
                utility,
                &[
                    UtilityTarget::Python,
                    UtilityTarget::Go,
                    UtilityTarget::Cpp,
                    UtilityTarget::Mcp,
                    UtilityTarget::Cli,
                ],
            )?;
            expect_utility_param_layout(utility, &offline_greeks_param_layout())?;
        }
        UtilityKind::ImpliedVolatility => {
            if utility.name != "implied_volatility" {
                return Err(
                    "utility kind ImpliedVolatility must use name 'implied_volatility'".into(),
                );
            }
            expect_utility_targets_subset(
                utility,
                &[
                    UtilityTarget::Python,
                    UtilityTarget::Go,
                    UtilityTarget::Cpp,
                    UtilityTarget::Mcp,
                    UtilityTarget::Cli,
                ],
            )?;
            expect_utility_param_layout(utility, &offline_greeks_param_layout())?;
        }
    }

    Ok(())
}

fn expect_method_name(
    method: &MethodSpec,
    expected: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if method.name != expected {
        return Err(format!(
            "method kind {:?} must use name '{}', got '{}'",
            method.kind, expected, method.name
        )
        .into());
    }
    Ok(())
}

fn expect_method_targets_subset(
    method: &MethodSpec,
    allowed: &[MethodTarget],
) -> Result<(), Box<dyn std::error::Error>> {
    for target in &method.targets {
        if !allowed.contains(target) {
            return Err(format!(
                "method '{}' declares unsupported target {:?}",
                method.name, target
            )
            .into());
        }
    }
    Ok(())
}

fn expect_exact_method_targets(
    method: &MethodSpec,
    expected: &[MethodTarget],
) -> Result<(), Box<dyn std::error::Error>> {
    expect_method_targets_subset(method, expected)?;
    if method.targets.len() != expected.len() {
        return Err(format!(
            "method '{}' must target exactly {:?}, got {:?}",
            method.name, expected, method.targets
        )
        .into());
    }
    Ok(())
}

fn expect_utility_targets_subset(
    utility: &UtilitySpec,
    allowed: &[UtilityTarget],
) -> Result<(), Box<dyn std::error::Error>> {
    for target in &utility.targets {
        if !allowed.contains(target) {
            return Err(format!(
                "utility '{}' declares unsupported target {:?}",
                utility.name, target
            )
            .into());
        }
    }
    Ok(())
}

fn expect_exact_utility_targets(
    utility: &UtilitySpec,
    expected: &[UtilityTarget],
) -> Result<(), Box<dyn std::error::Error>> {
    expect_utility_targets_subset(utility, expected)?;
    if utility.targets.len() != expected.len() {
        return Err(format!(
            "utility '{}' must target exactly {:?}, got {:?}",
            utility.name, expected, utility.targets
        )
        .into());
    }
    Ok(())
}

fn expect_param_layout(
    method: &MethodSpec,
    expected: &[(&str, ParamType)],
) -> Result<(), Box<dyn std::error::Error>> {
    if method.params.len() != expected.len() {
        return Err(format!(
            "method '{}' expected {} params but found {}",
            method.name,
            expected.len(),
            method.params.len()
        )
        .into());
    }
    for (param, (name, kind)) in method.params.iter().zip(expected.iter()) {
        if param.name != *name || param.param_type != *kind {
            return Err(format!(
                "method '{}' expected param ({name}, {kind:?}) but found ({}, {:?})",
                method.name, param.name, param.param_type
            )
            .into());
        }
    }
    Ok(())
}

fn expect_utility_param_layout(
    utility: &UtilitySpec,
    expected: &[(&str, ParamType)],
) -> Result<(), Box<dyn std::error::Error>> {
    if utility.params.len() != expected.len() {
        return Err(format!(
            "utility '{}' expected {} params but found {}",
            utility.name,
            expected.len(),
            utility.params.len()
        )
        .into());
    }
    for (param, (name, kind)) in utility.params.iter().zip(expected.iter()) {
        if param.name != *name || param.param_type != *kind {
            return Err(format!(
                "utility '{}' expected param ({name}, {kind:?}) but found ({}, {:?})",
                utility.name, param.name, param.param_type
            )
            .into());
        }
    }
    Ok(())
}

fn expect_some<'a>(
    value: &'a Option<String>,
    owner: &str,
    field: &str,
) -> Result<&'a str, Box<dyn std::error::Error>> {
    value
        .as_deref()
        .ok_or_else(|| format!("'{owner}' must declare {field}").into())
}

fn expect_none(
    value: &Option<String>,
    owner: &str,
    field: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if value.is_some() {
        return Err(format!("'{owner}' must not declare {field}").into());
    }
    Ok(())
}

fn ensure_unique_strings<I>(label: &str, values: I) -> Result<(), Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = std::collections::HashSet::new();
    for value in values {
        if !seen.insert(value.clone()) {
            return Err(format!("duplicate {label} entry '{value}'").into());
        }
    }
    Ok(())
}

fn generated_header() -> &'static str {
    "// @generated DO NOT EDIT — regenerated by build.rs from sdk_surface.toml\n\n"
}

fn offline_greeks_param_layout() -> [(&'static str, ParamType); 7] {
    [
        ("spot", ParamType::F64),
        ("strike", ParamType::F64),
        ("rate", ParamType::F64),
        ("div_yield", ParamType::F64),
        ("tte", ParamType::F64),
        ("option_price", ParamType::F64),
        ("right", ParamType::String),
    ]
}

fn greek_result_fields() -> [(&'static str, &'static str); 22] {
    [
        ("value", "value"),
        ("iv", "iv"),
        ("iv_error", "iv_error"),
        ("delta", "delta"),
        ("gamma", "gamma"),
        ("theta", "theta"),
        ("vega", "vega"),
        ("rho", "rho"),
        ("vanna", "vanna"),
        ("charm", "charm"),
        ("vomma", "vomma"),
        ("veta", "veta"),
        ("speed", "speed"),
        ("zomma", "zomma"),
        ("color", "color"),
        ("ultima", "ultima"),
        ("d1", "d1"),
        ("d2", "d2"),
        ("dual_delta", "dual_delta"),
        ("dual_gamma", "dual_gamma"),
        ("epsilon", "epsilon"),
        ("lambda", "lambda"),
    ]
}

fn rust_string_literal(value: &str) -> String {
    format!("{value:?}")
}

fn rust_string_array_literal(values: &[String]) -> String {
    let mut out = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&rust_string_literal(value));
    }
    out.push(']');
    out
}

fn pascal_case(value: &str) -> String {
    let mut out = String::new();
    for part in value.split('_').filter(|part| !part.is_empty()) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.extend(chars);
        }
    }
    out
}

fn lower_camel_case(value: &str) -> String {
    let pascal = pascal_case(value);
    let mut chars = pascal.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::new();
    out.push(first.to_ascii_lowercase());
    out.extend(chars);
    out
}

fn go_exported_name(name: &str) -> String {
    pascal_case(name)
}

fn go_param_name(name: &str) -> String {
    lower_camel_case(name)
}

fn go_c_var(name: &str) -> String {
    format!("c{}", pascal_case(name))
}

fn push_rust_doc_comment(out: &mut String, indent: &str, doc: &str) {
    for line in doc.lines() {
        writeln!(out, "{indent}/// {line}").unwrap();
    }
}

fn push_cpp_doc_comment(out: &mut String, indent: &str, doc: &str) {
    if doc.contains('\n') {
        writeln!(out, "{indent}/**").unwrap();
        for line in doc.lines() {
            writeln!(out, "{indent} * {line}").unwrap();
        }
        writeln!(out, "{indent} */").unwrap();
    } else {
        writeln!(out, "{indent}/** {doc} */").unwrap();
    }
}

fn python_type(param_type: ParamType) -> &'static str {
    match param_type {
        ParamType::String => "&str",
        ParamType::F64 => "f64",
        ParamType::I32 => "i32",
        ParamType::U64 => "u64",
        ParamType::CredentialsRef | ParamType::ConfigRef => {
            panic!("credentials/config refs are not valid for Python emitters")
        }
    }
}

fn go_type(param_type: ParamType) -> &'static str {
    match param_type {
        ParamType::String => "string",
        ParamType::F64 => "float64",
        ParamType::I32 => "int",
        ParamType::U64 => "uint64",
        ParamType::CredentialsRef => "*Credentials",
        ParamType::ConfigRef => "*Config",
    }
}

fn cpp_type(param_type: ParamType) -> &'static str {
    match param_type {
        ParamType::String => "const std::string&",
        ParamType::F64 => "double",
        ParamType::I32 => "int",
        ParamType::U64 => "uint64_t",
        ParamType::CredentialsRef => "const Credentials&",
        ParamType::ConfigRef => "const Config&",
    }
}

fn cli_param_name(param: &ParamSpec) -> &str {
    param.cli_name.as_deref().unwrap_or(&param.name)
}

fn mcp_param_name(param: &ParamSpec) -> &str {
    param.mcp_name.as_deref().unwrap_or(&param.name)
}

fn mcp_param_description(param: &ParamSpec) -> &str {
    param.mcp_description.as_deref().unwrap_or(&param.doc)
}

fn mcp_json_type(param_type: ParamType) -> &'static str {
    match param_type {
        ParamType::String => "string",
        ParamType::F64 => "number",
        ParamType::I32 | ParamType::U64 => "integer",
        ParamType::CredentialsRef | ParamType::ConfigRef => {
            panic!("credentials/config refs are not valid for MCP emitters")
        }
    }
}

fn render_python_streaming_methods(methods: &[&MethodSpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("#[pymethods]\n");
    out.push_str("impl ThetaDataDx {\n");
    for method in methods {
        out.push_str(&python_streaming_method(method));
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

fn render_python_utility_functions(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    for utility in utilities {
        out.push_str(&python_utility_function(utility));
        out.push('\n');
    }
    out.push_str(
        "fn register_generated_utility_functions(m: &Bound<'_, PyModule>) -> PyResult<()> {\n",
    );
    for utility in utilities {
        writeln!(
            out,
            "    m.add_function(wrap_pyfunction!({}, m)?)?;",
            utility.name
        )
        .unwrap();
    }
    out.push_str("    Ok(())\n");
    out.push_str("}\n");
    out
}

fn render_go_fpss_methods(
    methods: &[&MethodSpec],
    tls_reader_markers: &[TlsReaderMarker],
) -> String {
    let mut out = String::new();
    out.push_str("// Code generated by build.rs from sdk_surface.toml; DO NOT EDIT.\n\n");
    out.push_str("package thetadatadx\n\n");
    out.push_str("/*\n#include \"ffi_bridge.h\"\n*/\nimport \"C\"\n\n");
    out.push_str("import (\n\t\"fmt\"\n\t\"runtime\"\n\t\"unsafe\"\n)\n\n");
    for method in methods {
        out.push_str(&inject_os_thread_pin(
            &go_fpss_method(method),
            tls_reader_markers,
        ));
        out.push('\n');
    }
    out
}

fn render_go_utility_functions(
    utilities: &[&UtilitySpec],
    tls_reader_markers: &[TlsReaderMarker],
) -> String {
    let mut out = String::new();
    out.push_str("// Code generated by build.rs from sdk_surface.toml; DO NOT EDIT.\n\n");
    out.push_str("package thetadatadx\n\n");
    out.push_str("/*\n#include <stdlib.h>\n#include \"ffi_bridge.h\"\n*/\nimport \"C\"\n\n");
    out.push_str("import (\n\t\"fmt\"\n\t\"runtime\"\n\t\"unsafe\"\n)\n\n");
    for utility in utilities {
        out.push_str(&inject_os_thread_pin(
            &go_utility_function(utility),
            tls_reader_markers,
        ));
        out.push('\n');
    }
    out
}

fn render_go_timeout_pin_generated_test(
    repo_root: &Path,
    tls_reader_markers: &[TlsReaderMarker],
    generated_overrides: &[(&str, &str)],
) -> Result<String, Box<dyn std::error::Error>> {
    let expected_pinned_methods =
        count_go_tls_reader_methods(repo_root, tls_reader_markers, generated_overrides)?;
    let mut out = String::new();
    out.push_str("// Code generated by build.rs from sdk_surface.toml; DO NOT EDIT.\n\n");
    out.push_str("package thetadatadx\n\n");
    out.push_str("// tlsReaderMarkers is the single source of truth for the static\n");
    out.push_str("// Go TLS-reader audit in timeout_pin_test.go.\n");
    out.push_str("var tlsReaderMarkers = []string{\n");
    for marker in tls_reader_markers {
        writeln!(out, "\t{:?}, // {}", marker.substring, marker.description)?;
    }
    out.push_str("}\n\n");
    out.push_str("// expectedPinnedMethods is derived from the current non-test Go\n");
    out.push_str("// source tree: every function body that reads the FFI thread-local\n");
    out.push_str("// error slot must pin its goroutine to one OS thread.\n");
    writeln!(
        out,
        "const expectedPinnedMethods = {expected_pinned_methods}"
    )?;
    Ok(out)
}

fn render_cpp_fpss_decls(methods: &[&MethodSpec]) -> String {
    let mut out = String::new();
    out.push_str(
        "    // @generated DO NOT EDIT — regenerated by build.rs from sdk_surface.toml\n\n",
    );
    for method in methods {
        out.push_str(&cpp_fpss_decl(method));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn render_cpp_fpss_defs(methods: &[&MethodSpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    for method in methods {
        out.push_str(&cpp_fpss_def(method));
        out.push('\n');
    }
    out
}

fn render_cpp_utility_decls(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str("// @generated DO NOT EDIT — regenerated by build.rs from sdk_surface.toml\n\n");
    for utility in utilities {
        out.push_str(&cpp_utility_decl(utility));
        out.push('\n');
    }
    out
}

fn render_cpp_utility_defs(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    for utility in utilities {
        out.push_str(&cpp_utility_def(utility));
        out.push('\n');
    }
    out
}

fn render_cpp_lifecycle_defs(methods: &[&MethodSpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    for method in methods {
        out.push_str(&cpp_lifecycle_def(method));
        out.push('\n');
    }
    out
}

fn render_mcp_utilities(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("fn push_generated_utility_tool_definitions(tools: &mut Vec<Value>) {\n");
    for utility in utilities {
        out.push_str(&mcp_tool_definition(utility));
    }
    out.push_str("}\n\n");
    out.push_str(
        "async fn try_execute_generated_utility(\n    client: &Option<ThetaDataDx>,\n    name: &str,\n    args: &Value,\n    start_time: std::time::Instant,\n) -> Option<Result<Value, ToolError>> {\n    macro_rules! param_or_return {\n        ($expr:expr) => {\n            match $expr {\n                Ok(value) => value,\n                Err(error) => return Some(Err(ToolError::InvalidParams(error))),\n            }\n        };\n    }\n    match name {\n",
    );
    for utility in utilities {
        out.push_str(&mcp_execute_arm(utility));
    }
    out.push_str("        _ => None,\n    }\n}\n");
    out
}

fn render_cli_utilities(utilities: &[&UtilitySpec]) -> String {
    let mut out = String::new();
    out.push_str(generated_header());
    out.push_str("fn add_generated_utility_commands(mut app: Command) -> Command {\n");
    for utility in utilities {
        out.push_str(&cli_command_builder(utility));
    }
    out.push_str("    app\n}\n\n");
    out.push_str(
        "async fn try_run_generated_utility(\n    subcommand: Option<(&str, &ArgMatches)>,\n    fmt: &OutputFormat,\n    creds_path: &str,\n) -> Result<bool, thetadatadx::Error> {\n    match subcommand {\n",
    );
    for utility in utilities {
        out.push_str(&cli_dispatch_arm(utility));
    }
    out.push_str("        _ => Ok(false),\n    }\n}\n");
    out
}

fn python_streaming_method(method: &MethodSpec) -> String {
    let mut out = String::new();
    push_rust_doc_comment(&mut out, "    ", &method.doc);
    match method.kind {
        MethodKind::StartStreaming => {
            writeln!(out, "    fn {}(&self) -> PyResult<()> {{", method.name).unwrap();
            out.push_str("        let (tx, rx) = std::sync::mpsc::channel::<BufferedEvent>();\n\n");
            out.push_str("        self.tdx\n");
            out.push_str("            .start_streaming(move |event: &fpss::FpssEvent| {\n");
            out.push_str("                let buffered = fpss_event_to_buffered(event);\n");
            out.push_str("                let _ = tx.send(buffered);\n");
            out.push_str("            })\n");
            out.push_str("            .map_err(to_py_err)?;\n\n");
            out.push_str("        if let Ok(mut guard) = self.rx.lock() {\n");
            out.push_str("            *guard = Some(Arc::new(Mutex::new(rx)));\n");
            out.push_str("        }\n");
            out.push_str("        Ok(())\n");
            out.push_str("    }\n");
        }
        MethodKind::IsStreaming => {
            writeln!(out, "    fn {}(&self) -> bool {{", method.name).unwrap();
            out.push_str("        self.tdx.is_streaming()\n");
            out.push_str("    }\n");
        }
        MethodKind::StockContractCall => {
            let param = &method.params[0];
            writeln!(
                out,
                "    fn {}(&self, {}: {}) -> PyResult<()> {{",
                method.name,
                param.name,
                python_type(param.param_type)
            )
            .unwrap();
            writeln!(
                out,
                "        let contract = fpss::protocol::Contract::stock({});",
                param.name
            )
            .unwrap();
            writeln!(
                out,
                "        self.tdx.{}(&contract).map_err(to_py_err)",
                method.runtime_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("    }\n");
        }
        MethodKind::OptionContractCall => {
            writeln!(out, "    fn {}(", method.name).unwrap();
            out.push_str("        &self,\n");
            for param in &method.params {
                writeln!(
                    out,
                    "        {}: {},",
                    param.name,
                    python_type(param.param_type)
                )
                .unwrap();
            }
            out.push_str("    ) -> PyResult<()> {\n");
            writeln!(
                out,
                "        let contract = fpss::protocol::Contract::option({}, {}, {}, {});",
                method.params[0].name,
                method.params[1].name,
                method.params[2].name,
                method.params[3].name
            )
            .unwrap();
            writeln!(
                out,
                "        self.tdx.{}(&contract).map_err(to_py_err)",
                method.runtime_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("    }\n");
        }
        MethodKind::FullCall => {
            let param = &method.params[0];
            writeln!(
                out,
                "    fn {}(&self, {}: {}) -> PyResult<()> {{",
                method.name,
                param.name,
                python_type(param.param_type)
            )
            .unwrap();
            writeln!(out, "        let st = parse_sec_type({})?;", param.name).unwrap();
            writeln!(
                out,
                "        self.tdx.{}(st).map_err(to_py_err)",
                method.runtime_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("    }\n");
        }
        MethodKind::ContractMap => {
            writeln!(
                out,
                "    fn {}(&self) -> PyResult<std::collections::HashMap<i32, String>> {{",
                method.name
            )
            .unwrap();
            out.push_str("        self.tdx\n");
            out.push_str("            .contract_map()\n");
            out.push_str("            .map(|m| m.into_iter().map(|(id, c)| (id, format!(\"{c}\"))).collect())\n");
            out.push_str("            .map_err(to_py_err)\n");
            out.push_str("    }\n");
        }
        MethodKind::ContractLookup => {
            let param = &method.params[0];
            writeln!(
                out,
                "    fn {}(&self, {}: {}) -> PyResult<Option<String>> {{",
                method.name,
                param.name,
                python_type(param.param_type)
            )
            .unwrap();
            writeln!(out, "        self.tdx.contract_lookup({})", param.name).unwrap();
            out.push_str("            .map(|opt| opt.map(|c| format!(\"{c}\")))\n");
            out.push_str("            .map_err(to_py_err)\n");
            out.push_str("    }\n");
        }
        MethodKind::ActiveSubscriptions => {
            writeln!(
                out,
                "    fn {}(&self) -> PyResult<Vec<std::collections::HashMap<String, String>>> {{",
                method.name
            )
            .unwrap();
            out.push_str("        self.tdx\n");
            out.push_str("            .active_subscriptions()\n");
            out.push_str("            .map(|subs| {\n");
            out.push_str("                subs.into_iter()\n");
            out.push_str("                    .map(|(kind, contract)| {\n");
            out.push_str("                        let mut m = std::collections::HashMap::new();\n");
            out.push_str(
                "                        m.insert(\"kind\".to_string(), format!(\"{kind:?}\"));\n",
            );
            out.push_str("                        m.insert(\"contract\".to_string(), format!(\"{contract}\"));\n");
            out.push_str("                        m\n");
            out.push_str("                    })\n");
            out.push_str("                    .collect()\n");
            out.push_str("            })\n");
            out.push_str("            .map_err(to_py_err)\n");
            out.push_str("    }\n");
        }
        MethodKind::NextEvent => {
            let param = &method.params[0];
            writeln!(
                out,
                "    fn {}(&self, py: Python<'_>, {}: {}) -> PyResult<Option<Py<PyAny>>> {{",
                method.name,
                param.name,
                python_type(param.param_type)
            )
            .unwrap();
            out.push_str(
                "        let rx_outer = self.rx.lock().unwrap_or_else(|e| e.into_inner());\n",
            );
            out.push_str("        let rx_arc = match rx_outer.as_ref() {\n");
            out.push_str("            Some(arc) => Arc::clone(arc),\n");
            out.push_str("            None => {\n");
            out.push_str("                return Err(PyRuntimeError::new_err(\n");
            out.push_str(
                "                    \"streaming not started -- call start_streaming() first\",\n",
            );
            out.push_str("                ))\n");
            out.push_str("            }\n");
            out.push_str("        };\n");
            out.push_str("        drop(rx_outer);\n");
            writeln!(
                out,
                "        let timeout = std::time::Duration::from_millis({});",
                param.name
            )
            .unwrap();
            out.push_str("        let result = py.detach(move || {\n");
            out.push_str(
                "            let rx = rx_arc.lock().unwrap_or_else(|e| e.into_inner());\n",
            );
            out.push_str("            rx.recv_timeout(timeout).ok()\n");
            out.push_str("        });\n");
            out.push_str("        match result {\n");
            out.push_str(
                "            Some(event) => Ok(Some(buffered_event_to_py(py, &event))),\n",
            );
            out.push_str("            None => Ok(None),\n");
            out.push_str("        }\n");
            out.push_str("    }\n");
        }
        MethodKind::Reconnect => {
            writeln!(out, "    fn {}(&self) -> PyResult<()> {{", method.name).unwrap();
            out.push_str("        let (tx, rx) = std::sync::mpsc::channel::<BufferedEvent>();\n");
            out.push_str("        self.tdx\n");
            out.push_str("            .reconnect_streaming(move |event: &fpss::FpssEvent| {\n");
            out.push_str("                let _ = tx.send(fpss_event_to_buffered(event));\n");
            out.push_str("            })\n");
            out.push_str("            .map_err(to_py_err)?;\n");
            out.push_str("        if let Ok(mut guard) = self.rx.lock() {\n");
            out.push_str("            *guard = Some(Arc::new(Mutex::new(rx)));\n");
            out.push_str("        }\n");
            out.push_str("        Ok(())\n");
            out.push_str("    }\n");
        }
        MethodKind::StopStreaming | MethodKind::Shutdown => {
            writeln!(out, "    fn {}(&self) {{", method.name).unwrap();
            out.push_str("        self.tdx.stop_streaming();\n");
            out.push_str("        if let Ok(mut guard) = self.rx.lock() {\n");
            out.push_str("            *guard = None;\n");
            out.push_str("        }\n");
            out.push_str("    }\n");
        }
        other => panic!("unsupported Python method kind: {other:?}"),
    }
    out
}

fn python_utility_function(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    push_rust_doc_comment(&mut out, "", &utility.doc);
    out.push_str("#[pyfunction]\n");
    if utility.params.len() > 6 {
        out.push_str(
            "#[allow(clippy::too_many_arguments)] // Reason: mirrors Black-Scholes parameter set expected by SDK callers\n",
        );
    }
    match utility.kind {
        UtilityKind::AllGreeks => {
            writeln!(out, "fn {}(", utility.name).unwrap();
            out.push_str("    py: Python<'_>,\n");
            for param in &utility.params {
                writeln!(
                    out,
                    "    {}: {},",
                    param.name,
                    python_type(param.param_type)
                )
                .unwrap();
            }
            out.push_str(") -> Py<PyAny> {\n");
            writeln!(
                out,
                "    let g = tdbe::greeks::all_greeks({});",
                utility
                    .params
                    .iter()
                    .map(|param| param.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
            out.push_str("    let dict = PyDict::new(py);\n");
            out.push_str("    // PyO3: set_item is infallible for primitive types\n");
            for (field, rust_field) in greek_result_fields() {
                writeln!(
                    out,
                    "    dict.set_item({}, g.{rust_field}).unwrap();",
                    rust_string_literal(field)
                )
                .unwrap();
            }
            out.push_str("    dict.into_any().unbind()\n");
            out.push_str("}\n");
        }
        UtilityKind::ImpliedVolatility => {
            writeln!(out, "fn {}(", utility.name).unwrap();
            for param in &utility.params {
                writeln!(
                    out,
                    "    {}: {},",
                    param.name,
                    python_type(param.param_type)
                )
                .unwrap();
            }
            out.push_str(") -> (f64, f64) {\n");
            writeln!(
                out,
                "    tdbe::greeks::implied_volatility({})",
                utility
                    .params
                    .iter()
                    .map(|param| param.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
            out.push_str("}\n");
        }
        other => panic!("unsupported Python utility kind: {other:?}"),
    }
    out
}

fn go_fpss_method(method: &MethodSpec) -> String {
    let mut out = String::new();
    let exported_name = go_exported_name(&method.name);
    writeln!(out, "// {exported_name} {}", method.doc).unwrap();
    match method.kind {
        MethodKind::StockContractCall | MethodKind::FullCall => {
            let param = &method.params[0];
            let go_name = go_param_name(&param.name);
            let c_var = go_c_var(&param.name);
            writeln!(
                out,
                "func (f *FpssClient) {exported_name}({go_name} {}) (int, error) {{",
                go_type(param.param_type)
            )
            .unwrap();
            writeln!(out, "    {c_var} := C.CString({go_name})").unwrap();
            writeln!(out, "    defer C.free(unsafe.Pointer({c_var}))").unwrap();
            writeln!(
                out,
                "    return f.fpssCall(C.tdx_fpss_{}(f.handle, {c_var}))",
                method.ffi_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("}\n");
        }
        MethodKind::OptionContractCall => {
            let params = method
                .params
                .iter()
                .map(|param| {
                    format!(
                        "{} {}",
                        go_param_name(&param.name),
                        go_type(param.param_type)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                out,
                "func (f *FpssClient) {exported_name}({params}) (int, error) {{"
            )
            .unwrap();
            for param in &method.params {
                writeln!(
                    out,
                    "    {} := C.CString({})",
                    go_c_var(&param.name),
                    go_param_name(&param.name)
                )
                .unwrap();
            }
            for param in &method.params {
                writeln!(
                    out,
                    "    defer C.free(unsafe.Pointer({}))",
                    go_c_var(&param.name)
                )
                .unwrap();
            }
            let ffi_args = method
                .params
                .iter()
                .map(|param| go_c_var(&param.name))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                out,
                "    return f.fpssCall(C.tdx_fpss_{}(f.handle, {ffi_args}))",
                method.ffi_call.as_deref().unwrap()
            )
            .unwrap();
            out.push_str("}\n");
        }
        MethodKind::IsAuthenticated => {
            writeln!(out, "func (f *FpssClient) {exported_name}() bool {{").unwrap();
            out.push_str("    return C.tdx_fpss_is_authenticated(f.handle) != 0\n");
            out.push_str("}\n");
        }
        MethodKind::ContractLookup => {
            let param = &method.params[0];
            let name = go_param_name(&param.name);
            writeln!(
                out,
                "func (f *FpssClient) {exported_name}({name} {}) (string, error) {{",
                go_type(param.param_type)
            )
            .unwrap();
            writeln!(
                out,
                "    cstr := C.tdx_fpss_contract_lookup(f.handle, C.int({name}))"
            )
            .unwrap();
            out.push_str("    if cstr == nil {\n");
            out.push_str("        if msg := lastError(); msg != \"\" {\n");
            out.push_str("            return \"\", fmt.Errorf(\"thetadatadx: %s\", msg)\n");
            out.push_str("        }\n");
            out.push_str("        return \"\", nil\n");
            out.push_str("    }\n");
            out.push_str("    goStr := C.GoString(cstr)\n");
            out.push_str("    C.tdx_string_free(cstr)\n");
            out.push_str("    return goStr, nil\n");
            out.push_str("}\n");
        }
        MethodKind::ContractMap => {
            writeln!(
                out,
                "func (f *FpssClient) {exported_name}() (map[int32]string, error) {{"
            )
            .unwrap();
            out.push_str("    arr := C.tdx_fpss_contract_map(f.handle)\n");
            out.push_str("    if arr == nil {\n");
            out.push_str("        return nil, fmt.Errorf(\"thetadatadx: %s\", lastError())\n");
            out.push_str("    }\n");
            out.push_str("    defer C.tdx_contract_map_array_free(arr)\n");
            out.push_str("    result := make(map[int32]string, int(arr.len))\n");
            out.push_str("    if arr.data == nil || arr.len == 0 {\n");
            out.push_str("        return result, nil\n");
            out.push_str("    }\n");
            out.push_str("    entries := unsafe.Slice(arr.data, int(arr.len))\n");
            out.push_str("    for _, entry := range entries {\n");
            out.push_str("        value := \"\"\n");
            out.push_str("        if entry.contract != nil {\n");
            out.push_str("            value = C.GoString(entry.contract)\n");
            out.push_str("        }\n");
            out.push_str("        result[int32(entry.id)] = value\n");
            out.push_str("    }\n");
            out.push_str("    return result, nil\n");
            out.push_str("}\n");
        }
        MethodKind::ActiveSubscriptions => {
            writeln!(
                out,
                "func (f *FpssClient) {exported_name}() ([]Subscription, error) {{"
            )
            .unwrap();
            out.push_str("    arr := C.tdx_fpss_active_subscriptions(f.handle)\n");
            out.push_str("    if arr == nil {\n");
            out.push_str("        return nil, fmt.Errorf(\"thetadatadx: %s\", lastError())\n");
            out.push_str("    }\n");
            out.push_str("    defer C.tdx_subscription_array_free(arr)\n");
            out.push_str("    n := int(arr.len)\n");
            out.push_str("    if n == 0 || arr.data == nil {\n");
            out.push_str("        return nil, nil\n");
            out.push_str("    }\n");
            out.push_str("    subs := unsafe.Slice(arr.data, n)\n");
            out.push_str("    result := make([]Subscription, n)\n");
            out.push_str("    for i := 0; i < n; i++ {\n");
            out.push_str("        if subs[i].kind != nil {\n");
            out.push_str("            result[i].Kind = C.GoString(subs[i].kind)\n");
            out.push_str("        }\n");
            out.push_str("        if subs[i].contract != nil {\n");
            out.push_str("            result[i].Contract = C.GoString(subs[i].contract)\n");
            out.push_str("        }\n");
            out.push_str("    }\n");
            out.push_str("    return result, nil\n");
            out.push_str("}\n");
        }
        MethodKind::NextEvent => {
            let param = &method.params[0];
            let name = go_param_name(&param.name);
            writeln!(
                out,
                "func (f *FpssClient) {exported_name}({name} {}) (*FpssEvent, error) {{",
                go_type(param.param_type)
            )
            .unwrap();
            writeln!(
                out,
                "    raw := C.tdx_fpss_next_event(f.handle, C.uint64_t({name}))"
            )
            .unwrap();
            out.push_str("    if raw == nil {\n");
            out.push_str("        return nil, nil\n");
            out.push_str("    }\n");
            out.push_str("    defer C.tdx_fpss_event_free(raw)\n\n");
            out.push_str("    event := &FpssEvent{\n");
            out.push_str("        Kind: FpssEventKind(raw.kind),\n");
            out.push_str("    }\n\n");
            out.push_str("    switch event.Kind {\n");
            out.push_str("    case FpssQuoteEvent:\n");
            out.push_str("        q := raw.quote\n");
            out.push_str("        event.Quote = &FpssQuote{\n");
            out.push_str("            ContractID:   int32(q.contract_id),\n");
            out.push_str("            MsOfDay:      int32(q.ms_of_day),\n");
            out.push_str("            BidSize:      int32(q.bid_size),\n");
            out.push_str("            BidExchange:  int32(q.bid_exchange),\n");
            out.push_str("            Bid:          float64(q.bid),\n");
            out.push_str("            BidCondition: int32(q.bid_condition),\n");
            out.push_str("            AskSize:      int32(q.ask_size),\n");
            out.push_str("            AskExchange:  int32(q.ask_exchange),\n");
            out.push_str("            Ask:          float64(q.ask),\n");
            out.push_str("            AskCondition: int32(q.ask_condition),\n");
            out.push_str("            Date:         int32(q.date),\n");
            out.push_str("            ReceivedAtNs: uint64(q.received_at_ns),\n");
            out.push_str("        }\n");
            out.push_str("    case FpssTradeEvent:\n");
            out.push_str("        t := raw.trade\n");
            out.push_str("        event.Trade = &FpssTrade{\n");
            out.push_str("            ContractID:     int32(t.contract_id),\n");
            out.push_str("            MsOfDay:        int32(t.ms_of_day),\n");
            out.push_str("            Sequence:       int32(t.sequence),\n");
            out.push_str("            ExtCondition1:  int32(t.ext_condition1),\n");
            out.push_str("            ExtCondition2:  int32(t.ext_condition2),\n");
            out.push_str("            ExtCondition3:  int32(t.ext_condition3),\n");
            out.push_str("            ExtCondition4:  int32(t.ext_condition4),\n");
            out.push_str("            Condition:      int32(t.condition),\n");
            out.push_str("            Size:           int32(t.size),\n");
            out.push_str("            Exchange:       int32(t.exchange),\n");
            out.push_str("            Price:          float64(t.price),\n");
            out.push_str("            ConditionFlags: int32(t.condition_flags),\n");
            out.push_str("            PriceFlags:     int32(t.price_flags),\n");
            out.push_str("            VolumeType:     int32(t.volume_type),\n");
            out.push_str("            RecordsBack:    int32(t.records_back),\n");
            out.push_str("            Date:           int32(t.date),\n");
            out.push_str("            ReceivedAtNs:   uint64(t.received_at_ns),\n");
            out.push_str("        }\n");
            out.push_str("    case FpssOpenInterestEvent:\n");
            out.push_str("        oi := raw.open_interest\n");
            out.push_str("        event.OpenInterest = &FpssOpenInterestData{\n");
            out.push_str("            ContractID:   int32(oi.contract_id),\n");
            out.push_str("            MsOfDay:      int32(oi.ms_of_day),\n");
            out.push_str("            OpenInterest: int32(oi.open_interest),\n");
            out.push_str("            Date:         int32(oi.date),\n");
            out.push_str("            ReceivedAtNs: uint64(oi.received_at_ns),\n");
            out.push_str("        }\n");
            out.push_str("    case FpssOhlcvcEvent:\n");
            out.push_str("        o := raw.ohlcvc\n");
            out.push_str("        event.Ohlcvc = &FpssOhlcvc{\n");
            out.push_str("            ContractID:   int32(o.contract_id),\n");
            out.push_str("            MsOfDay:      int32(o.ms_of_day),\n");
            out.push_str("            Open:         float64(o.open),\n");
            out.push_str("            High:         float64(o.high),\n");
            out.push_str("            Low:          float64(o.low),\n");
            out.push_str("            Close:        float64(o.close),\n");
            out.push_str("            Volume:       int64(o.volume),\n");
            out.push_str("            Count:        int64(o.count),\n");
            out.push_str("            Date:         int32(o.date),\n");
            out.push_str("            ReceivedAtNs: uint64(o.received_at_ns),\n");
            out.push_str("        }\n");
            out.push_str("    case FpssControlEvent:\n");
            out.push_str("        ctrl := raw.control\n");
            out.push_str("        detail := \"\"\n");
            out.push_str("        if ctrl.detail != nil {\n");
            out.push_str("            detail = C.GoString(ctrl.detail)\n");
            out.push_str("        }\n");
            out.push_str("        event.Control = &FpssControlData{\n");
            out.push_str("            Kind:   int32(ctrl.kind),\n");
            out.push_str("            ID:     int32(ctrl.id),\n");
            out.push_str("            Detail: detail,\n");
            out.push_str("        }\n");
            out.push_str("    case FpssRawDataEvent:\n");
            out.push_str("        rd := raw.raw_data\n");
            out.push_str("        event.RawCode = uint8(rd.code)\n");
            out.push_str("        if rd.payload != nil && rd.payload_len > 0 {\n");
            out.push_str("            event.RawPayload = C.GoBytes(unsafe.Pointer(rd.payload), C.int(rd.payload_len))\n");
            out.push_str("        }\n");
            out.push_str("    }\n\n");
            out.push_str("    return event, nil\n");
            out.push_str("}\n");
        }
        MethodKind::Reconnect => {
            writeln!(out, "func (f *FpssClient) {exported_name}() error {{").unwrap();
            out.push_str("    rc := C.tdx_fpss_reconnect(f.handle)\n");
            out.push_str("    if rc < 0 {\n");
            out.push_str("        return fmt.Errorf(\"thetadatadx: %s\", lastError())\n");
            out.push_str("    }\n");
            out.push_str("    return nil\n");
            out.push_str("}\n");
        }
        MethodKind::Shutdown => {
            writeln!(out, "func (f *FpssClient) {exported_name}() {{").unwrap();
            out.push_str("    if f.handle != nil {\n");
            out.push_str("        C.tdx_fpss_shutdown(f.handle)\n");
            out.push_str("    }\n");
            out.push_str("}\n");
        }
        other => panic!("unsupported Go method kind: {other:?}"),
    }
    out
}

fn go_greeks_field_name(field: &str) -> &'static str {
    match field {
        "value" => "Value",
        "iv" => "IV",
        "iv_error" => "IVError",
        "delta" => "Delta",
        "gamma" => "Gamma",
        "theta" => "Theta",
        "vega" => "Vega",
        "rho" => "Rho",
        "vanna" => "Vanna",
        "charm" => "Charm",
        "vomma" => "Vomma",
        "veta" => "Veta",
        "speed" => "Speed",
        "zomma" => "Zomma",
        "color" => "Color",
        "ultima" => "Ultima",
        "d1" => "D1",
        "d2" => "D2",
        "dual_delta" => "DualDelta",
        "dual_gamma" => "DualGamma",
        "epsilon" => "Epsilon",
        "lambda" => "Lambda",
        other => panic!("unknown Greeks field: {other}"),
    }
}

fn go_utility_function(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    let exported_name = go_exported_name(&utility.name);
    writeln!(out, "// {exported_name} {}", utility.doc).unwrap();
    match utility.kind {
        UtilityKind::AllGreeks => {
            let params = utility
                .params
                .iter()
                .map(|param| {
                    format!(
                        "{} {}",
                        go_param_name(&param.name),
                        go_type(param.param_type)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(out, "func {exported_name}({params}) (*Greeks, error) {{").unwrap();
            out.push_str("    cRight := C.CString(right)\n");
            out.push_str("    defer C.free(unsafe.Pointer(cRight))\n");
            out.push_str("    ptr := C.tdx_all_greeks(C.double(spot), C.double(strike), C.double(rate), C.double(divYield), C.double(tte), C.double(optionPrice), cRight)\n");
            out.push_str("    if ptr == nil {\n");
            out.push_str("        return nil, fmt.Errorf(\"thetadatadx: %s\", lastError())\n");
            out.push_str("    }\n");
            out.push_str("    defer C.tdx_greeks_result_free(ptr)\n");
            out.push_str("    return &Greeks{\n");
            for (field, _) in greek_result_fields() {
                writeln!(
                    out,
                    "        {}: float64(ptr.{field}),",
                    go_greeks_field_name(field)
                )
                .unwrap();
            }
            out.push_str("    }, nil\n");
            out.push_str("}\n");
        }
        UtilityKind::ImpliedVolatility => {
            let params = utility
                .params
                .iter()
                .map(|param| {
                    format!(
                        "{} {}",
                        go_param_name(&param.name),
                        go_type(param.param_type)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                out,
                "func {exported_name}({params}) (float64, float64, error) {{"
            )
            .unwrap();
            out.push_str("    cRight := C.CString(right)\n");
            out.push_str("    defer C.free(unsafe.Pointer(cRight))\n");
            out.push_str("    var iv, ivErr C.double\n");
            out.push_str("    rc := C.tdx_implied_volatility(C.double(spot), C.double(strike), C.double(rate), C.double(divYield), C.double(tte), C.double(optionPrice), cRight, &iv, &ivErr)\n");
            out.push_str("    if rc != 0 {\n");
            out.push_str("        return 0, 0, fmt.Errorf(\"thetadatadx: %s\", lastError())\n");
            out.push_str("    }\n");
            out.push_str("    return float64(iv), float64(ivErr), nil\n");
            out.push_str("}\n");
        }
        other => panic!("unsupported Go utility kind: {other:?}"),
    }
    out
}

fn cpp_fpss_decl(method: &MethodSpec) -> String {
    let mut out = String::new();
    push_cpp_doc_comment(&mut out, "    ", &method.doc);
    match method.kind {
        MethodKind::FpssConnect => {
            out.push_str("    FpssClient(const Credentials& creds, const Config& config);\n");
        }
        MethodKind::StockContractCall | MethodKind::FullCall => {
            writeln!(
                out,
                "    int {}({} {});",
                method.name,
                cpp_type(method.params[0].param_type),
                method.params[0].name
            )
            .unwrap();
        }
        MethodKind::OptionContractCall => {
            let params = method
                .params
                .iter()
                .map(|param| format!("{} {}", cpp_type(param.param_type), param.name))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(out, "    int {}({params});", method.name).unwrap();
        }
        MethodKind::IsAuthenticated => out.push_str("    bool is_authenticated() const;\n"),
        MethodKind::ContractLookup => {
            out.push_str("    std::optional<std::string> contract_lookup(int id) const;\n");
        }
        MethodKind::ContractMap => {
            out.push_str("    std::map<int32_t, std::string> contract_map() const;\n");
        }
        MethodKind::ActiveSubscriptions => {
            out.push_str("    std::vector<Subscription> active_subscriptions() const;\n");
        }
        MethodKind::NextEvent => {
            out.push_str("    FpssEventPtr next_event(uint64_t timeout_ms);\n");
        }
        MethodKind::Reconnect => out.push_str("    void reconnect();\n"),
        MethodKind::Shutdown => out.push_str("    void shutdown();\n"),
        other => panic!("unsupported C++ FPSS decl kind: {other:?}"),
    }
    out
}

fn cpp_fpss_def(method: &MethodSpec) -> String {
    match method.kind {
        MethodKind::FpssConnect => {
            r#"FpssClient::FpssClient(const Credentials& creds, const Config& config) {
    auto h = tdx_fpss_connect(creds.get(), config.get());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    handle_.reset(h);
}
"#
            .to_string()
        }
        MethodKind::StockContractCall | MethodKind::FullCall => format!(
            "int FpssClient::{}({} {}) {{ return tdx_fpss_{}(handle_.get(), {}.c_str()); }}\n",
            method.name,
            cpp_type(method.params[0].param_type),
            method.params[0].name,
            method.ffi_call.as_deref().unwrap(),
            method.params[0].name
        ),
        MethodKind::OptionContractCall => {
            let params = method
                .params
                .iter()
                .map(|param| format!("{} {}", cpp_type(param.param_type), param.name))
                .collect::<Vec<_>>()
                .join(", ");
            let ffi_args = method
                .params
                .iter()
                .map(|param| format!("{}.c_str()", param.name))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "int FpssClient::{}({params}) {{ return tdx_fpss_{}(handle_.get(), {ffi_args}); }}\n",
                method.name,
                method.ffi_call.as_deref().unwrap()
            )
        }
        MethodKind::IsAuthenticated => {
            "bool FpssClient::is_authenticated() const { return tdx_fpss_is_authenticated(handle_.get()) != 0; }\n"
                .to_string()
        }
        MethodKind::ContractLookup => {
            r#"std::optional<std::string> FpssClient::contract_lookup(int id) const {
    detail::FfiString result(tdx_fpss_contract_lookup(handle_.get(), id));
    if (!result.ok()) {
        std::string err = detail::last_ffi_error();
        if (!err.empty()) throw std::runtime_error("thetadatadx: " + err);
        return std::nullopt;
    }
    return result.str();
}
"#
            .to_string()
        }
        MethodKind::ContractMap => {
            r#"std::map<int32_t, std::string> FpssClient::contract_map() const {
    auto* arr = tdx_fpss_contract_map(handle_.get());
    if (!arr) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    std::map<int32_t, std::string> result;
    if (arr->data != nullptr && arr->len > 0) {
        for (size_t i = 0; i < arr->len; ++i) {
            result.emplace(arr->data[i].id, arr->data[i].contract ? std::string(arr->data[i].contract) : std::string());
        }
    }
    tdx_contract_map_array_free(arr);
    return result;
}
"#
            .to_string()
        }
        MethodKind::ActiveSubscriptions => {
            r#"std::vector<Subscription> FpssClient::active_subscriptions() const {
    return detail::subscription_array_to_vector(tdx_fpss_active_subscriptions(handle_.get()));
}
"#
            .to_string()
        }
        MethodKind::NextEvent => {
            r#"FpssEventPtr FpssClient::next_event(uint64_t timeout_ms) {
    auto* raw = tdx_fpss_next_event(handle_.get(), timeout_ms);
    return FpssEventPtr(raw);
}
"#
            .to_string()
        }
        MethodKind::Reconnect => {
            r#"void FpssClient::reconnect() {
    int rc = tdx_fpss_reconnect(handle_.get());
    if (rc < 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
}
"#
            .to_string()
        }
        MethodKind::Shutdown => {
            "void FpssClient::shutdown() { tdx_fpss_shutdown(handle_.get()); }\n".to_string()
        }
        other => panic!("unsupported C++ FPSS def kind: {other:?}"),
    }
}

fn cpp_utility_decl(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    push_cpp_doc_comment(&mut out, "", &utility.doc);
    match utility.kind {
        UtilityKind::AllGreeks => {
            out.push_str(
                "Greeks all_greeks(double spot, double strike, double rate, double div_yield,\n",
            );
            out.push_str(
                "                  double tte, double option_price, const std::string& right);\n",
            );
        }
        UtilityKind::ImpliedVolatility => {
            out.push_str(
                "std::pair<double, double> implied_volatility(double spot, double strike,\n",
            );
            out.push_str(
                "                                              double rate, double div_yield,\n",
            );
            out.push_str(
                "                                              double tte, double option_price,\n",
            );
            out.push_str(
                "                                              const std::string& right);\n",
            );
        }
        other => panic!("unsupported C++ utility kind: {other:?}"),
    }
    out
}

fn cpp_utility_def(utility: &UtilitySpec) -> String {
    match utility.kind {
        UtilityKind::AllGreeks => {
            r#"Greeks all_greeks(double spot, double strike, double rate, double div_yield,
                  double tte, double option_price, const std::string& right) {
    TdxGreeksResult* raw = tdx_all_greeks(
        spot,
        strike,
        rate,
        div_yield,
        tte,
        option_price,
        right.c_str()
    );
    if (raw == nullptr) {
        throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    }

    Greeks result{
        raw->value,
        raw->delta,
        raw->gamma,
        raw->theta,
        raw->vega,
        raw->rho,
        raw->iv,
        raw->iv_error,
        raw->vanna,
        raw->charm,
        raw->vomma,
        raw->veta,
        raw->speed,
        raw->zomma,
        raw->color,
        raw->ultima,
        raw->d1,
        raw->d2,
        raw->dual_delta,
        raw->dual_gamma,
        raw->epsilon,
        raw->lambda,
    };
    tdx_greeks_result_free(raw);
    return result;
}
"#
            .to_string()
        }
        UtilityKind::ImpliedVolatility => {
            r#"std::pair<double, double> implied_volatility(double spot, double strike,
                                              double rate, double div_yield,
                                              double tte, double option_price,
                                              const std::string& right) {
    double iv = 0.0, err = 0.0;
    int rc = tdx_implied_volatility(spot, strike, rate, div_yield, tte, option_price, right.c_str(), &iv, &err);
    if (rc != 0) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return {iv, err};
}
"#
            .to_string()
        }
        other => panic!("unsupported C++ utility kind: {other:?}"),
    }
}

fn cpp_lifecycle_def(method: &MethodSpec) -> String {
    match method.kind {
        MethodKind::CredentialsFromFile => {
            r#"Credentials Credentials::from_file(const std::string& path) {
    auto h = tdx_credentials_from_file(path.c_str());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Credentials(h);
}
"#
            .to_string()
        }
        MethodKind::CredentialsFromEmail => {
            r#"Credentials Credentials::from_email(const std::string& email, const std::string& password) {
    auto h = tdx_credentials_new(email.c_str(), password.c_str());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Credentials(h);
}
"#
            .to_string()
        }
        MethodKind::ConfigConstructor => {
            let variant = method.config_variant.as_deref().unwrap();
            format!("Config Config::{variant}() {{ return Config(tdx_config_{variant}()); }}\n")
        }
        MethodKind::ClientConnect => {
            r#"Client Client::connect(const Credentials& creds, const Config& config) {
    auto h = tdx_client_connect(creds.get(), config.get());
    if (!h) throw std::runtime_error("thetadatadx: " + detail::last_ffi_error());
    return Client(h);
}
"#
            .to_string()
        }
        other => panic!("unsupported C++ lifecycle kind: {other:?}"),
    }
}

fn mcp_tool_definition(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    out.push_str("    tools.push(json!({\n");
    writeln!(
        out,
        "        \"name\": {},",
        rust_string_literal(&utility.name)
    )
    .unwrap();
    writeln!(
        out,
        "        \"description\": {},",
        rust_string_literal(utility.mcp_description.as_deref().unwrap_or(&utility.doc))
    )
    .unwrap();
    out.push_str("        \"inputSchema\": {\n");
    out.push_str("            \"type\": \"object\",\n");
    out.push_str("            \"properties\": {\n");
    for (index, param) in utility.params.iter().enumerate() {
        let suffix = if index + 1 == utility.params.len() {
            ""
        } else {
            ","
        };
        write!(
            out,
            "                {}: {{ \"type\": {}, \"description\": {}",
            rust_string_literal(mcp_param_name(param)),
            rust_string_literal(mcp_json_type(param.param_type)),
            rust_string_literal(mcp_param_description(param))
        )
        .unwrap();
        if !param.enum_values.is_empty() {
            write!(
                out,
                ", \"enum\": {}",
                rust_string_array_literal(&param.enum_values)
            )
            .unwrap();
        }
        writeln!(out, " }}{suffix}").unwrap();
    }
    out.push_str("            },\n");
    out.push_str("            \"required\": [");
    for (index, param) in utility.params.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&rust_string_literal(mcp_param_name(param)));
    }
    out.push_str("]\n");
    out.push_str("        }\n");
    out.push_str("    }));\n");
    out
}

fn mcp_execute_arm(utility: &UtilitySpec) -> String {
    let mut out = String::new();
    writeln!(out, "        {} => {{", rust_string_literal(&utility.name)).unwrap();
    match utility.kind {
        UtilityKind::Ping => {
            out.push_str("            let uptime = start_time.elapsed();\n");
            out.push_str("            Some(Ok(json!({\n");
            out.push_str("                \"status\": \"ok\",\n");
            out.push_str("                \"server\": \"thetadatadx-mcp\",\n");
            out.push_str("                \"version\": VERSION,\n");
            out.push_str("                \"uptime_secs\": uptime.as_secs(),\n");
            out.push_str("                \"connected\": client.is_some(),\n");
            out.push_str("            })))\n");
        }
        UtilityKind::AllGreeks => {
            out.push_str("            let spot = param_or_return!(arg_f64(args, \"spot\"));\n");
            out.push_str("            let strike = param_or_return!(arg_f64(args, \"strike\"));\n");
            out.push_str("            let rate = param_or_return!(arg_f64(args, \"rate\"));\n");
            out.push_str("            let div_yield = param_or_return!(arg_f64(args, \"dividend_yield\"));\n");
            out.push_str(
                "            let tte = param_or_return!(arg_f64(args, \"time_to_expiry\"));\n",
            );
            out.push_str("            let option_price = param_or_return!(arg_f64(args, \"option_price\"));\n");
            out.push_str("            let right = param_or_return!(arg_str(args, \"right\"));\n");
            out.push_str("            param_or_return!(thetadatadx::parse_right_strict(&right).map_err(|e| e.to_string()));\n");
            out.push_str("            let g = tdbe::greeks::all_greeks(spot, strike, rate, div_yield, tte, option_price, &right);\n");
            out.push_str("            Some(Ok(json!({\n");
            for (field, rust_field) in greek_result_fields() {
                writeln!(
                    out,
                    "                {}: g.{rust_field},",
                    rust_string_literal(field)
                )
                .unwrap();
            }
            out.push_str("            })))\n");
        }
        UtilityKind::ImpliedVolatility => {
            out.push_str("            let spot = param_or_return!(arg_f64(args, \"spot\"));\n");
            out.push_str("            let strike = param_or_return!(arg_f64(args, \"strike\"));\n");
            out.push_str("            let rate = param_or_return!(arg_f64(args, \"rate\"));\n");
            out.push_str("            let div_yield = param_or_return!(arg_f64(args, \"dividend_yield\"));\n");
            out.push_str(
                "            let tte = param_or_return!(arg_f64(args, \"time_to_expiry\"));\n",
            );
            out.push_str("            let option_price = param_or_return!(arg_f64(args, \"option_price\"));\n");
            out.push_str("            let right = param_or_return!(arg_str(args, \"right\"));\n");
            out.push_str("            param_or_return!(thetadatadx::parse_right_strict(&right).map_err(|e| e.to_string()));\n");
            out.push_str("            let (iv, err) = tdbe::greeks::implied_volatility(spot, strike, rate, div_yield, tte, option_price, &right);\n");
            out.push_str("            Some(Ok(json!({\n");
            out.push_str("                \"implied_volatility\": iv,\n");
            out.push_str("                \"error\": err,\n");
            out.push_str("            })))\n");
        }
        UtilityKind::Auth => panic!("auth is CLI-only"),
    }
    out.push_str("        }\n");
    out
}

fn cli_command_builder(utility: &UtilitySpec) -> String {
    let cli_name = utility.cli_name.as_deref().unwrap_or(&utility.name);
    let cli_about = utility.cli_about.as_deref().unwrap_or(&utility.doc);
    let mut out = String::new();
    if utility.kind == UtilityKind::Auth {
        writeln!(
            out,
            "    app = app.subcommand(Command::new({}).about({}));",
            rust_string_literal(cli_name),
            rust_string_literal(cli_about)
        )
        .unwrap();
        return out;
    }

    out.push_str("    app = app.subcommand(\n");
    writeln!(
        out,
        "        Command::new({})",
        rust_string_literal(cli_name)
    )
    .unwrap();
    writeln!(
        out,
        "            .about({})",
        rust_string_literal(cli_about)
    )
    .unwrap();
    for param in &utility.params {
        out.push_str("            .arg(\n");
        writeln!(
            out,
            "                Arg::new({})",
            rust_string_literal(cli_param_name(param))
        )
        .unwrap();
        out.push_str("                    .required(true)\n");
        writeln!(
            out,
            "                    .help({}),",
            rust_string_literal(&param.doc)
        )
        .unwrap();
        out.push_str("            )\n");
    }
    out.push_str("    );\n");
    out
}

fn cli_dispatch_arm(utility: &UtilitySpec) -> String {
    let cli_name = utility.cli_name.as_deref().unwrap_or(&utility.name);
    let mut out = String::new();
    match utility.kind {
        UtilityKind::Auth => {
            writeln!(
                out,
                "        Some(({}, _)) => {{",
                rust_string_literal(cli_name)
            )
            .unwrap();
            out.push_str(
                "            let creds = thetadatadx::Credentials::from_file(creds_path)?;\n",
            );
            out.push_str(
                "            let resp = thetadatadx::auth::authenticate(&creds).await?;\n",
            );
            out.push_str("            let mut td = TabularData::new(vec![\n");
            out.push_str("                \"session_id\",\n                \"email\",\n                \"stock_tier\",\n                \"options_tier\",\n                \"indices_tier\",\n                \"rate_tier\",\n                \"created\",\n            ]);\n");
            out.push_str("            let user = resp.user.as_ref();\n");
            out.push_str("            let redacted_session = if resp.session_id.len() >= 8 {\n");
            out.push_str("                format!(\"{}...\", &resp.session_id[..8])\n");
            out.push_str("            } else {\n");
            out.push_str("                resp.session_id.clone()\n");
            out.push_str("            };\n");
            out.push_str("            td.push(vec![\n");
            out.push_str("                redacted_session,\n");
            out.push_str(
                "                user.and_then(|u| u.email.clone()).unwrap_or_default(),\n",
            );
            out.push_str("                user.and_then(|u| u.stock_subscription)\n                    .map(|t| format!(\"{t}\"))\n                    .unwrap_or_default(),\n");
            out.push_str("                user.and_then(|u| u.options_subscription)\n                    .map(|t| format!(\"{t}\"))\n                    .unwrap_or_default(),\n");
            out.push_str("                user.and_then(|u| u.indices_subscription)\n                    .map(|t| format!(\"{t}\"))\n                    .unwrap_or_default(),\n");
            out.push_str("                user.and_then(|u| u.interest_rate_subscription)\n                    .map(|t| format!(\"{t}\"))\n                    .unwrap_or_default(),\n");
            out.push_str("                resp.session_created.unwrap_or_default(),\n");
            out.push_str("            ]);\n");
            out.push_str("            td.render(fmt);\n");
            out.push_str("            Ok(true)\n");
            out.push_str("        }\n");
        }
        UtilityKind::AllGreeks => {
            writeln!(
                out,
                "        Some(({}, sub_m)) => {{",
                rust_string_literal(cli_name)
            )
            .unwrap();
            out.push_str("            let spot: f64 = get_arg(sub_m, \"spot\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid spot: {e}\")))?;\n");
            out.push_str("            let strike: f64 = get_arg(sub_m, \"strike\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid strike: {e}\")))?;\n");
            out.push_str("            let rate: f64 = get_arg(sub_m, \"rate\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid rate: {e}\")))?;\n");
            out.push_str("            let div_yield: f64 = get_arg(sub_m, \"dividend\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid dividend: {e}\")))?;\n");
            out.push_str("            let tte: f64 = get_arg(sub_m, \"time\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid time: {e}\")))?;\n");
            out.push_str("            let option_price: f64 = get_arg(sub_m, \"option_price\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid option_price: {e}\")))?;\n");
            out.push_str("            let right = get_arg(sub_m, \"right\");\n");
            out.push_str("            thetadatadx::parse_right_strict(right)?;\n");
            out.push_str("            let g = tdbe::greeks::all_greeks(spot, strike, rate, div_yield, tte, option_price, right);\n");
            out.push_str(
                "            let mut td = TabularData::new(vec![\"greek\", \"value\"]);\n",
            );
            out.push_str("            let rows = [\n");
            for (field, rust_field) in greek_result_fields() {
                writeln!(
                    out,
                    "                ({}, g.{rust_field}),",
                    rust_string_literal(field)
                )
                .unwrap();
            }
            out.push_str("            ];\n");
            out.push_str("            for (name, val) in rows {\n");
            out.push_str(
                "                td.push(vec![name.to_string(), format!(\"{val:.8}\")]);\n",
            );
            out.push_str("            }\n");
            out.push_str("            td.render(fmt);\n");
            out.push_str("            Ok(true)\n");
            out.push_str("        }\n");
        }
        UtilityKind::ImpliedVolatility => {
            writeln!(
                out,
                "        Some(({}, sub_m)) => {{",
                rust_string_literal(cli_name)
            )
            .unwrap();
            out.push_str("            let spot: f64 = get_arg(sub_m, \"spot\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid spot: {e}\")))?;\n");
            out.push_str("            let strike: f64 = get_arg(sub_m, \"strike\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid strike: {e}\")))?;\n");
            out.push_str("            let rate: f64 = get_arg(sub_m, \"rate\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid rate: {e}\")))?;\n");
            out.push_str("            let div_yield: f64 = get_arg(sub_m, \"dividend\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid dividend: {e}\")))?;\n");
            out.push_str("            let tte: f64 = get_arg(sub_m, \"time\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid time: {e}\")))?;\n");
            out.push_str("            let option_price: f64 = get_arg(sub_m, \"option_price\")\n");
            out.push_str("                .parse()\n");
            out.push_str("                .map_err(|e| thetadatadx::Error::Config(format!(\"invalid option_price: {e}\")))?;\n");
            out.push_str("            let right = get_arg(sub_m, \"right\");\n");
            out.push_str("            thetadatadx::parse_right_strict(right)?;\n");
            out.push_str("            let (iv, iv_error) = tdbe::greeks::implied_volatility(spot, strike, rate, div_yield, tte, option_price, right);\n");
            out.push_str(
                "            let mut td = TabularData::new(vec![\"iv\", \"iv_error\"]);\n",
            );
            out.push_str(
                "            td.push(vec![format!(\"{iv:.8}\"), format!(\"{iv_error:.8}\")]);\n",
            );
            out.push_str("            td.render(fmt);\n");
            out.push_str("            Ok(true)\n");
            out.push_str("        }\n");
        }
        UtilityKind::Ping => panic!("ping is MCP-only"),
    }
    out
}

/// Insert `runtime.LockOSThread()` + deferred unlock at the top of every
/// Go method body in `src` whose body reads the FFI's thread-local error
/// slot (any substring in `sdk_surface.toml`'s
/// `go_ffi.tls_reader_markers`). Methods without TLS reads pass through
/// unchanged.
fn inject_os_thread_pin(src: &str, tls_reader_markers: &[TlsReaderMarker]) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("func ") && line.trim_end().ends_with('{') {
            let mut j = i + 1;
            let mut touches_tls = false;
            while j < lines.len() {
                let l = lines[j];
                if l.starts_with('}') {
                    break;
                }
                if tls_reader_markers
                    .iter()
                    .any(|marker| l.contains(&marker.substring))
                {
                    touches_tls = true;
                }
                j += 1;
            }
            out.push_str(line);
            if touches_tls {
                out.push_str("    runtime.LockOSThread()\n");
                out.push_str("    defer runtime.UnlockOSThread()\n");
            }
            for k in (i + 1)..=j.min(lines.len().saturating_sub(1)) {
                out.push_str(lines[k]);
            }
            i = j + 1;
            continue;
        }
        out.push_str(line);
        i += 1;
    }
    out
}

fn count_go_tls_reader_methods(
    repo_root: &Path,
    tls_reader_markers: &[TlsReaderMarker],
    generated_overrides: &[(&str, &str)],
) -> Result<usize, Box<dyn std::error::Error>> {
    let go_dir = repo_root.join("sdks/go");
    let mut files: Vec<_> = std::fs::read_dir(&go_dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let file_name = path.file_name()?.to_str()?;
            if path.extension().and_then(|ext| ext.to_str()) == Some("go")
                && !file_name.ends_with("_test.go")
            {
                Some((file_name.to_string(), path))
            } else {
                None
            }
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let generated_overrides: std::collections::HashMap<&str, &str> =
        generated_overrides.iter().copied().collect();

    let mut count = 0usize;
    for (file_name, path) in files {
        let contents = if let Some(generated) = generated_overrides.get(file_name.as_str()) {
            (*generated).to_string()
        } else {
            std::fs::read_to_string(path)?
        };
        let lines: Vec<&str> = contents
            .split('\n')
            .map(|line| line.trim_end_matches('\r'))
            .collect();
        for (idx, line) in lines.iter().enumerate() {
            if !line.starts_with("func ") || !line.ends_with('{') {
                continue;
            }
            let Some(method_name) = extract_go_method_name(line) else {
                continue;
            };
            if is_go_tls_helper(method_name) {
                continue;
            }
            let body_end = find_go_method_body_end(&lines, idx + 1);
            let body = &lines[idx + 1..body_end];
            if body.iter().any(|body_line| {
                tls_reader_markers
                    .iter()
                    .any(|marker| body_line.contains(&marker.substring))
            }) {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn extract_go_method_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("func ")?;
    let rest = if rest.starts_with('(') {
        let end = rest.find(") ")?;
        &rest[end + 2..]
    } else {
        rest
    };
    let end = rest.find('(')?;
    Some(rest[..end].trim())
}

fn find_go_method_body_end(lines: &[&str], from: usize) -> usize {
    for (idx, line) in lines.iter().enumerate().skip(from) {
        if line.starts_with('}') {
            return idx;
        }
    }
    lines.len()
}

fn is_go_tls_helper(method_name: &str) -> bool {
    matches!(method_name, "lastError" | "lastErrorRaw" | "fpssCall")
}
