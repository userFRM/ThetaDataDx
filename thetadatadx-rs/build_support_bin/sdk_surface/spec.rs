//! TOML-backed spec types + validation for `sdk_surface.toml`.
//!
//! Parsing lives here because the semantic shape of each method/utility kind
//! (expected name, allowed targets, required param layout) is intrinsic to
//! the spec, not to any particular render target.

use serde::Deserialize;

use super::common::offline_greeks_param_layout;

/// Parsed `sdk_surface.toml`: the schema version plus the declared methods and utilities.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SdkSurfaceSpec {
    pub(super) version: u32,
    #[serde(default)]
    pub(super) methods: Vec<MethodSpec>,
    #[serde(default)]
    pub(super) utilities: Vec<UtilitySpec>,
}

/// One declared SDK method: its name, semantic kind, doc text, target projections, params, and optional call/config bindings.
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
    pub(super) config_variant: Option<String>,
}

/// Semantic kind of an SDK method, driving its expected name, allowed targets, param layout, and emitted code template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MethodKind {
    StartStreaming,
    IsStreaming,
    Batches,
    ActiveSubscriptions,
    Reconnect,
    StopStreaming,
    Shutdown,
    AwaitDrain,
    IsAuthenticated,
    FpssConnect,
    FpssConnectFromFile,
    CredentialsFromFile,
    CredentialsFromEmail,
    CredentialsFromApiKey,
    CredentialsFromApiKeyWithEmail,
    CredentialsFromEnv,
    CredentialsFromEnvOrFile,
    CredentialsFromDotenv,
    ConfigConstructor,
    ConfigFromDotenv,
    ClientConnect,
    ClientConnectFromFile,
}

/// Render target a method projects to (Python unified, C++ FPSS, C++ lifecycle, or TypeScript napi).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MethodTarget {
    PythonUnified,
    CppFpss,
    CppLifecycle,
    TypescriptNapi,
}

/// One declared offline utility: its name, semantic kind, doc text, target projections, params, and optional MCP overrides.
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
    pub(super) mcp_description: Option<String>,
    /// Core function path forwarded to by a `Forwarder` utility, e.g.
    /// `thetadatadx::utils::conditions::condition_name`. Required for the
    /// `Forwarder` kind and forbidden on every other kind.
    #[serde(default)]
    pub(super) forward_call: Option<String>,
    /// Scalar return type of a `Forwarder` utility. Required for the
    /// `Forwarder` kind and forbidden on every other kind.
    #[serde(default)]
    pub(super) forward_return: Option<ForwardReturn>,
}

/// Semantic kind of an offline utility, driving its expected name, allowed targets, and emitted code template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum UtilityKind {
    Ping,
    AllGreeks,
    ImpliedVolatility,
    /// Thin one-line forward into a `thetadatadx::utils::*` lookup table.
    /// Body, params, and return type come from `forward_call` /
    /// `forward_return` plus the declared `params`, so the 10 lookup
    /// helpers share one emitter arm instead of one per name.
    Forwarder,
    /// `calendar_status_name` — `CalendarStatus::from_code(...).map_or`.
    CalendarStatusName,
    /// `timestamp_ms` — `(date, ms-of-day)` to epoch ms (Python `i64`,
    /// TypeScript `BigInt`).
    TimestampMs,
    /// `sequence_signed_to_unsigned` — wire-range-checked re-encoding.
    SequenceSignedToUnsigned,
    /// `sequence_unsigned_to_signed` — wire-range-checked re-encoding.
    SequenceUnsignedToSigned,
}

/// Scalar return type of a `Forwarder` utility, projected per binding
/// (Python `&'static str` / `bool`; TypeScript `String` / `bool`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ForwardReturn {
    Str,
    Bool,
}

/// Render target a utility projects to (Python, TypeScript, C++, or MCP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum UtilityTarget {
    Python,
    Typescript,
    Cpp,
    Mcp,
}

/// One method or utility parameter: its name, type, doc text, optional MCP name and description overrides, and allowed enum values.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ParamSpec {
    pub(super) name: String,
    #[serde(rename = "type")]
    pub(super) param_type: ParamType,
    pub(super) doc: String,
    #[serde(default)]
    pub(super) mcp_name: Option<String>,
    #[serde(default)]
    pub(super) mcp_description: Option<String>,
    #[serde(default)]
    pub(super) enum_values: Vec<String>,
}

/// Type of a parameter, projected per language into the concrete Rust, C++, and JSON Schema types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ParamType {
    String,
    F64,
    I32,
    I64,
    U64,
    CredentialsRef,
    ConfigRef,
}

/// Rendering shape shared by the `MethodKind` match arms in
/// `validate_method_spec`. Name extracted to keep
/// `clippy::type_complexity` happy and document the structure:
///
/// * `0` — optional fixed method name (`None` when the name is
///   data-dependent, e.g. `ConfigConstructor`'s `config_<variant>`);
/// * `1` — targets this kind is allowed to project to;
/// * `2` — `true` when the targets list must be exact (no omissions);
/// * `3` — required parameter layout `(name, type)` pairs.
type MethodShape<'a> = (
    Option<&'a str>,
    &'a [MethodTarget],
    bool,
    &'a [(&'static str, ParamType)],
);

/// Loads and deserializes `sdk_surface.toml` into an [`SdkSurfaceSpec`] and registers it as a build rerun trigger.
pub(super) fn load_sdk_surface_spec() -> Result<SdkSurfaceSpec, Box<dyn std::error::Error>> {
    let spec_path = "sdk_surface.toml";
    let spec_str = std::fs::read_to_string(spec_path)?;
    let spec: SdkSurfaceSpec = toml::from_str(&spec_str)?;
    println!("cargo:rerun-if-changed={spec_path}");
    Ok(spec)
}

/// Validates the spec's version, rejects duplicate method or utility names, and checks each entry's per-kind shape.
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
    // `expected_name = None` means the name is data-dependent (e.g.
    // `ConfigConstructor`'s `config_<variant>`). The `config_variant`
    // field isn't validated here — the code generator fails loudly when it
    // tries to use a missing one.
    const PY: MethodTarget = MethodTarget::PythonUnified;
    const CPP: MethodTarget = MethodTarget::CppFpss;
    const LIFE: MethodTarget = MethodTarget::CppLifecycle;
    const TS: MethodTarget = MethodTarget::TypescriptNapi;
    let shape: MethodShape<'_> = match method.kind {
        MethodKind::StartStreaming => (Some("start_streaming"), &[PY, TS], true, &[]),
        MethodKind::IsStreaming => (Some("is_streaming"), &[PY, TS], true, &[]),
        // The pull-based Arrow `RecordBatch` reader, a sibling to the
        // per-event callback. The Python + TypeScript entries are generated
        // (PY, TS). The C++ entry is hand-written on the `Stream` class —
        // the same hand-written class that carries `set_callback`, distinct
        // from the generated `StreamingClient` the `cpp_fpss` target feeds —
        // so C++ is intentionally NOT in this generated set; it is tracked
        // for parity directly. The tuning knobs (batch_size / linger /
        // backpressure / capacity) are language-native optional parameters
        // wired in each per-language surface, not positional spec params, so
        // the layout here is empty.
        MethodKind::Batches => (Some("batches"), &[PY, TS], true, &[]),
        MethodKind::ActiveSubscriptions => {
            (Some("active_subscriptions"), &[PY, TS, CPP], false, &[])
        }
        MethodKind::Reconnect => (Some("reconnect"), &[PY, TS, CPP], false, &[]),
        MethodKind::StopStreaming => (Some("stop_streaming"), &[PY, TS], true, &[]),
        MethodKind::Shutdown => (Some("shutdown"), &[PY, TS, CPP], false, &[]),
        MethodKind::AwaitDrain => (
            Some("await_drain"),
            &[PY, TS],
            true,
            &[("timeout_ms", ParamType::U64)],
        ),
        MethodKind::IsAuthenticated => (Some("is_authenticated"), &[CPP], false, &[]),
        MethodKind::FpssConnect => (
            Some("connect"),
            &[CPP],
            true,
            &[
                ("creds", ParamType::CredentialsRef),
                ("config", ParamType::ConfigRef),
            ],
        ),
        MethodKind::FpssConnectFromFile => (
            Some("from_file"),
            &[CPP],
            true,
            &[("path", ParamType::String)],
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
        MethodKind::CredentialsFromApiKey => (
            Some("credentials_from_api_key"),
            &[LIFE],
            true,
            &[("api_key", ParamType::String)],
        ),
        MethodKind::CredentialsFromApiKeyWithEmail => (
            Some("credentials_from_api_key_with_email"),
            &[LIFE],
            true,
            &[("email", ParamType::String), ("api_key", ParamType::String)],
        ),
        MethodKind::CredentialsFromEnv => (Some("credentials_from_env"), &[LIFE], true, &[]),
        MethodKind::CredentialsFromEnvOrFile => (
            Some("credentials_from_env_or_file"),
            &[LIFE],
            true,
            &[("path", ParamType::String)],
        ),
        MethodKind::CredentialsFromDotenv => (
            Some("credentials_from_dotenv"),
            &[LIFE],
            true,
            &[("path", ParamType::String)],
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
        MethodKind::ConfigFromDotenv => (
            Some("config_from_dotenv"),
            &[LIFE],
            true,
            &[("path", ParamType::String)],
        ),
        MethodKind::ClientConnect => (
            Some("client_connect"),
            &[LIFE],
            true,
            &[
                ("creds", ParamType::CredentialsRef),
                ("config", ParamType::ConfigRef),
            ],
        ),
        MethodKind::ClientConnectFromFile => (
            Some("mdds_client_from_file"),
            &[LIFE],
            true,
            &[("path", ParamType::String)],
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

/// The single `(code, i32)` parameter shape that every lookup-table
/// `Forwarder` (and the `CalendarStatusName` helper) takes. Both the
/// spec validator and the per-language emitters key on this: `validate`
/// pins a forwarder's declared params to it, and the emitters hardcode
/// `(code: i32)` in the generated body, so any other shape must fail the
/// build rather than silently mis-emit.
pub(super) const FORWARDER_CODE_PARAMS: &[(&str, ParamType)] = &[("code", ParamType::I32)];

/// Assert at emit time that a `Forwarder` utility's params are exactly the
/// `(code, i32)` shape the emitters hardcode. `validate_spec` already
/// enforces this, but the emitters bake `(code: i32)` in directly; this
/// guard makes that assumption self-failing so a future differently-shaped
/// forwarder panics here instead of emitting a body that ignores its spec.
pub(super) fn assert_forwarder_code_params(utility: &UtilitySpec) {
    let ok = utility.params.len() == FORWARDER_CODE_PARAMS.len()
        && utility
            .params
            .iter()
            .zip(FORWARDER_CODE_PARAMS)
            .all(|(param, (name, kind))| param.name == *name && param.param_type == *kind);
    assert!(
        ok,
        "forwarder '{}' emits a hardcoded (code: i32) body but declares params {:?}; \
         only the (code, i32) shape is supported",
        utility.name, utility.params
    );
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

    // `forward_call` / `forward_return` belong only to the Forwarder kind.
    let is_forwarder = utility.kind == UtilityKind::Forwarder;
    if is_forwarder {
        if utility.forward_call.is_none() || utility.forward_return.is_none() {
            return Err(format!(
                "forwarder utility '{}' must declare forward_call and forward_return",
                utility.name
            )
            .into());
        }
    } else if utility.forward_call.is_some() || utility.forward_return.is_some() {
        return Err(format!(
            "utility '{}' declares forward_call/forward_return but is not a forwarder",
            utility.name
        )
        .into());
    }

    use UtilityTarget::{Cpp, Mcp, Python, Typescript};
    let greeks_params = offline_greeks_param_layout();
    // Forwarders are name-agnostic (the name is the forwarded helper's
    // name); `expected_name = None` skips the fixed-name check below.
    let (expected_name, allowed_targets, exact_targets, params): (
        Option<&str>,
        &[UtilityTarget],
        bool,
        &[(&str, ParamType)],
    ) = match utility.kind {
        UtilityKind::Ping => (Some("ping"), &[Mcp], true, &[]),
        UtilityKind::AllGreeks => (
            Some("all_greeks"),
            &[Python, Typescript, Cpp, Mcp],
            false,
            &greeks_params,
        ),
        UtilityKind::ImpliedVolatility => (
            Some("implied_volatility"),
            &[Python, Typescript, Cpp, Mcp],
            false,
            &greeks_params,
        ),
        UtilityKind::Forwarder => (None, &[Python, Typescript], true, FORWARDER_CODE_PARAMS),
        UtilityKind::CalendarStatusName => (
            Some("calendar_status_name"),
            &[Python, Typescript],
            true,
            FORWARDER_CODE_PARAMS,
        ),
        UtilityKind::TimestampMs => (
            Some("timestamp_ms"),
            &[Python, Typescript],
            true,
            &[("date", ParamType::I32), ("ms_of_day", ParamType::I32)],
        ),
        UtilityKind::SequenceSignedToUnsigned => (
            Some("sequence_signed_to_unsigned"),
            &[Python, Typescript],
            true,
            &[("signed_value", ParamType::I64)],
        ),
        UtilityKind::SequenceUnsignedToSigned => (
            Some("sequence_unsigned_to_signed"),
            &[Python, Typescript],
            true,
            &[("unsigned_value", ParamType::U64)],
        ),
    };

    if let Some(expected) = expected_name {
        if utility.name != expected {
            return Err(format!(
                "utility kind {:?} must use name '{expected}', got '{}'",
                utility.kind, utility.name
            )
            .into());
        }
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
