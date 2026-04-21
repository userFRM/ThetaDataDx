//! TOML-backed spec types + validation for `sdk_surface.toml`.
//!
//! Parsing lives here because the semantic shape of each method/utility kind
//! (expected name, allowed targets, required param layout) is intrinsic to
//! the spec, not to any particular render target.

use serde::Deserialize;

use super::common::offline_greeks_param_layout;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SdkSurfaceSpec {
    pub(super) version: u32,
    #[serde(default)]
    pub(super) methods: Vec<MethodSpec>,
    #[serde(default)]
    pub(super) utilities: Vec<UtilitySpec>,
    /// Go-side FFI configuration. Holds the TLS-reader marker SSOT that
    /// drives both `inject_os_thread_pin` (build-time body rewriter) and
    /// the generated `tlsReaderMarkers` list consumed by the static-audit
    /// test in `sdks/go/timeout_pin_test.go`.
    #[serde(default)]
    pub(super) go_ffi: GoFfiSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MethodSpec {
    pub(super) name: String,
    pub(super) kind: MethodKind,
    pub(super) doc: String,
    pub(super) targets: Vec<MethodTarget>,
    #[serde(default)]
    pub(super) params: Vec<ParamSpec>,
    #[serde(default)]
    pub(super) runtime_call: Option<String>,
    #[serde(default)]
    pub(super) ffi_call: Option<String>,
    #[serde(default)]
    pub(super) config_variant: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MethodKind {
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
pub(super) enum MethodTarget {
    PythonUnified,
    GoFpss,
    CppFpss,
    CppLifecycle,
    TypescriptNapi,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UtilitySpec {
    pub(super) name: String,
    pub(super) kind: UtilityKind,
    pub(super) doc: String,
    pub(super) targets: Vec<UtilityTarget>,
    #[serde(default)]
    pub(super) params: Vec<ParamSpec>,
    #[serde(default)]
    pub(super) cli_name: Option<String>,
    #[serde(default)]
    pub(super) cli_about: Option<String>,
    #[serde(default)]
    pub(super) mcp_description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum UtilityKind {
    Auth,
    Ping,
    AllGreeks,
    ImpliedVolatility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum UtilityTarget {
    Python,
    Go,
    Cpp,
    Mcp,
    Cli,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ParamSpec {
    pub(super) name: String,
    #[serde(rename = "type")]
    pub(super) param_type: ParamType,
    pub(super) doc: String,
    #[serde(default)]
    pub(super) cli_name: Option<String>,
    #[serde(default)]
    pub(super) mcp_name: Option<String>,
    #[serde(default)]
    pub(super) mcp_description: Option<String>,
    #[serde(default)]
    pub(super) enum_values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ParamType {
    String,
    F64,
    I32,
    U64,
    CredentialsRef,
    ConfigRef,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GoFfiSpec {
    #[serde(default)]
    pub(super) tls_reader_markers: Vec<TlsReaderMarker>,
}

/// A substring that, when present on a Go source line, identifies an FFI
/// thread-local error read. The enclosing function must have executed
/// `runtime.LockOSThread()` + `defer runtime.UnlockOSThread()` before
/// reaching such a line.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TlsReaderMarker {
    pub(super) substring: String,
}

/// Rendering shape shared by the `MethodKind` match arms in
/// `validate_method_spec`. Name extracted to keep
/// `clippy::type_complexity` happy and document the structure:
///
/// * `0` — optional fixed method name (`None` when the name varies per
///   endpoint inside the kind, e.g. `StockContractCall`);
/// * `1` — targets this kind is allowed to project to;
/// * `2` — `true` when the targets list must be exact (no omissions);
/// * `3` — required parameter layout `(name, type)` pairs.
type MethodShape<'a> = (
    Option<&'a str>,
    &'a [MethodTarget],
    bool,
    &'a [(&'static str, ParamType)],
);

pub(super) fn load_sdk_surface_spec() -> Result<SdkSurfaceSpec, Box<dyn std::error::Error>> {
    let spec_path = "sdk_surface.toml";
    let spec_str = std::fs::read_to_string(spec_path)?;
    let spec: SdkSurfaceSpec = toml::from_str(&spec_str)?;
    println!("cargo:rerun-if-changed={spec_path}");
    Ok(spec)
}

pub(super) fn validate_spec(spec: &SdkSurfaceSpec) -> Result<(), Box<dyn std::error::Error>> {
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
    for param in &method.params {
        if !param.enum_values.is_empty() && param.param_type != ParamType::String {
            return Err(format!(
                "method '{}' param '{}' declares enum_values but is not a string",
                method.name, param.name
            )
            .into());
        }
    }

    // Per-kind shape: (expected_name, allowed_targets, exact_targets, params).
    // `expected_name = None` means the name varies per endpoint inside the
    // kind (StockContractCall, OptionContractCall, FullCall). The
    // runtime_call/ffi_call/config_variant fields aren't validated here —
    // the code generator fails loudly when it tries to use a missing one.
    const PY: MethodTarget = MethodTarget::PythonUnified;
    const GO: MethodTarget = MethodTarget::GoFpss;
    const CPP: MethodTarget = MethodTarget::CppFpss;
    const LIFE: MethodTarget = MethodTarget::CppLifecycle;
    const TS: MethodTarget = MethodTarget::TypescriptNapi;
    let shape: MethodShape<'_> = match method.kind {
        MethodKind::StartStreaming => (Some("start_streaming"), &[PY, TS], true, &[]),
        MethodKind::IsStreaming => (Some("is_streaming"), &[PY, TS], true, &[]),
        MethodKind::StockContractCall => (
            None,
            &[PY, TS, GO, CPP],
            false,
            &[("symbol", ParamType::String)],
        ),
        MethodKind::OptionContractCall => (
            None,
            &[PY, TS, GO, CPP],
            false,
            &[
                ("symbol", ParamType::String),
                ("expiration", ParamType::String),
                ("strike", ParamType::String),
                ("right", ParamType::String),
            ],
        ),
        MethodKind::FullCall => (
            None,
            &[PY, TS, GO, CPP],
            false,
            &[("sec_type", ParamType::String)],
        ),
        MethodKind::ContractMap => (Some("contract_map"), &[PY, TS, GO, CPP], false, &[]),
        MethodKind::ContractLookup => (
            Some("contract_lookup"),
            &[PY, TS, GO, CPP],
            false,
            &[("id", ParamType::I32)],
        ),
        MethodKind::ActiveSubscriptions => {
            (Some("active_subscriptions"), &[PY, TS, GO, CPP], false, &[])
        }
        MethodKind::NextEvent => (
            Some("next_event"),
            &[PY, TS, GO, CPP],
            false,
            &[("timeout_ms", ParamType::U64)],
        ),
        MethodKind::Reconnect => (Some("reconnect"), &[PY, TS, GO, CPP], false, &[]),
        MethodKind::StopStreaming => (Some("stop_streaming"), &[PY, TS], true, &[]),
        MethodKind::Shutdown => (Some("shutdown"), &[PY, TS, GO, CPP], false, &[]),
        MethodKind::IsAuthenticated => (Some("is_authenticated"), &[GO, CPP], false, &[]),
        MethodKind::FpssConnect => (
            Some("connect"),
            &[CPP],
            true,
            &[
                ("creds", ParamType::CredentialsRef),
                ("config", ParamType::ConfigRef),
            ],
        ),
        MethodKind::CredentialsFromFile => (
            Some("credentials_from_file"),
            &[LIFE],
            true,
            &[("path", ParamType::String)],
        ),
        MethodKind::CredentialsFromEmail => (
            Some("credentials_from_email"),
            &[LIFE],
            true,
            &[
                ("email", ParamType::String),
                ("password", ParamType::String),
            ],
        ),
        MethodKind::ConfigConstructor => {
            // Name is data-dependent (`config_<variant>`); check it here so
            // the shared name check below can be skipped for this arm.
            let variant = method
                .config_variant
                .as_deref()
                .ok_or_else(|| format!("method '{}' must declare config_variant", method.name))?;
            if !matches!(variant, "production" | "dev" | "stage") {
                return Err(format!(
                    "method '{}' has unsupported config_variant '{}'",
                    method.name, variant
                )
                .into());
            }
            let expected_name = format!("config_{variant}");
            if method.name != expected_name {
                return Err(format!(
                    "method kind ConfigConstructor must use name '{expected_name}', got '{}'",
                    method.name
                )
                .into());
            }
            (None, &[LIFE], true, &[])
        }
        MethodKind::ClientConnect => (
            Some("client_connect"),
            &[LIFE],
            true,
            &[
                ("creds", ParamType::CredentialsRef),
                ("config", ParamType::ConfigRef),
            ],
        ),
    };
    let (expected_name, allowed_targets, exact_targets, params) = shape;

    if let Some(name) = expected_name {
        if method.name != name {
            return Err(format!(
                "method kind {:?} must use name '{name}', got '{}'",
                method.kind, method.name
            )
            .into());
        }
    }
    check_targets(
        &method.name,
        "method",
        &method.targets,
        allowed_targets,
        exact_targets,
    )?;
    check_param_layout(&method.name, "method", &method.params, params)?;

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

    use UtilityTarget::{Cli, Cpp, Go, Mcp, Python};
    let greeks_params = offline_greeks_param_layout();
    let (expected_name, allowed_targets, exact_targets, params): (
        &str,
        &[UtilityTarget],
        bool,
        &[(&str, ParamType)],
    ) = match utility.kind {
        UtilityKind::Auth => ("auth", &[Cli], true, &[]),
        UtilityKind::Ping => ("ping", &[Mcp], true, &[]),
        UtilityKind::AllGreeks => (
            "all_greeks",
            &[Python, Go, Cpp, Mcp, Cli],
            false,
            &greeks_params,
        ),
        UtilityKind::ImpliedVolatility => (
            "implied_volatility",
            &[Python, Go, Cpp, Mcp, Cli],
            false,
            &greeks_params,
        ),
    };

    if utility.name != expected_name {
        return Err(format!(
            "utility kind {:?} must use name '{expected_name}', got '{}'",
            utility.kind, utility.name
        )
        .into());
    }
    check_utility_targets(utility, allowed_targets, exact_targets)?;
    check_param_layout(&utility.name, "utility", &utility.params, params)?;

    Ok(())
}

fn check_targets(
    owner: &str,
    label: &str,
    actual: &[MethodTarget],
    allowed: &[MethodTarget],
    exact: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    for target in actual {
        if !allowed.contains(target) {
            return Err(format!("{label} '{owner}' declares unsupported target {target:?}").into());
        }
    }
    if exact && actual.len() != allowed.len() {
        return Err(
            format!("{label} '{owner}' must target exactly {allowed:?}, got {actual:?}").into(),
        );
    }
    Ok(())
}

fn check_utility_targets(
    utility: &UtilitySpec,
    allowed: &[UtilityTarget],
    exact: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    for target in &utility.targets {
        if !allowed.contains(target) {
            return Err(format!(
                "utility '{}' declares unsupported target {target:?}",
                utility.name
            )
            .into());
        }
    }
    if exact && utility.targets.len() != allowed.len() {
        return Err(format!(
            "utility '{}' must target exactly {allowed:?}, got {:?}",
            utility.name, utility.targets
        )
        .into());
    }
    Ok(())
}

fn check_param_layout(
    owner: &str,
    label: &str,
    actual: &[ParamSpec],
    expected: &[(&str, ParamType)],
) -> Result<(), Box<dyn std::error::Error>> {
    if actual.len() != expected.len() {
        return Err(format!(
            "{label} '{owner}' expected {} params but found {}",
            expected.len(),
            actual.len()
        )
        .into());
    }
    for (param, (name, kind)) in actual.iter().zip(expected.iter()) {
        if param.name != *name || param.param_type != *kind {
            return Err(format!(
                "{label} '{owner}' expected param ({name}, {kind:?}) but found ({}, {:?})",
                param.name, param.param_type
            )
            .into());
        }
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
