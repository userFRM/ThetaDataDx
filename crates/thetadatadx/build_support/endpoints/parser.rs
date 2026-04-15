//! Parse `endpoint_surface.toml` and join it to proto-derived wire metadata.
//!
//! Owns the surface-side flow only: TOML loading, template/param-group
//! resolution (with cycle detection and unused-config rejection),
//! cross-validation against the wire contract, and the final merge into
//! [`ParsedEndpoints`]. Proto parsing lives in [`super::proto_parser`].

use std::collections::{HashMap, HashSet};

use super::model::{
    GeneratedEndpoint, GeneratedParam, ParsedEndpoints, ResolvedSurfaceEndpoint, ResolvedTemplate,
    SurfaceEndpoint, SurfaceParam, SurfaceParamEntry, SurfaceSpec, SurfaceTestFixtures,
    TestFixtures,
};
use super::proto_parser::load_proto_endpoints;

/// Load the explicit endpoint surface spec and join it to proto-derived wire metadata.
pub(super) fn load_endpoint_specs() -> Result<ParsedEndpoints, Box<dyn std::error::Error>> {
    let wire = load_proto_endpoints()?;
    let spec_path = "endpoint_surface.toml";
    let spec_str = std::fs::read_to_string(spec_path)?;
    let spec: SurfaceSpec = toml::from_str(&spec_str)?;
    if spec.version != 2 {
        return Err(format!(
            "unsupported endpoint surface spec version {} in {spec_path}",
            spec.version
        )
        .into());
    }

    let resolved = resolve_surface_endpoints(&spec)?;

    let mut seen_names = HashSet::new();
    let mut wire_by_name = HashMap::new();
    for endpoint in wire.endpoints {
        wire_by_name.insert(endpoint.name.clone(), endpoint);
    }

    let mut endpoints = Vec::with_capacity(resolved.len());
    let mut consumed_wire_names = HashSet::new();
    for surface in resolved {
        if !seen_names.insert(surface.name.clone()) {
            return Err(format!("duplicate endpoint surface entry: {}", surface.name).into());
        }
        let wire_name = surface.wire_name.as_deref().unwrap_or(&surface.name);
        let wire_endpoint = wire_by_name.get(wire_name).ok_or_else(|| {
            format!(
                "endpoint surface '{}' references unknown wire endpoint '{}'",
                surface.name, wire_name
            )
        })?;
        consumed_wire_names.insert(wire_name.to_string());

        validate_surface_endpoint(&surface, wire_endpoint)?;
        endpoints.push(merge_surface_and_wire(surface, wire_endpoint));
    }

    // Detect proto RPCs not covered by endpoint_surface.toml. A new RPC added
    // to the proto should fail the build rather than being silently ignored.
    // Synthetic wire entries (cloned variants like stock_history_ohlc_range that
    // share an RPC with another endpoint) are excluded because they don't
    // correspond to a unique proto RPC.
    let synthetic = ["stock_history_ohlc_range"];
    for wire_name in wire_by_name.keys() {
        if !consumed_wire_names.contains(wire_name.as_str())
            && !synthetic.contains(&wire_name.as_str())
        {
            return Err(format!(
                "wire endpoint '{}' from external.proto has no entry in endpoint_surface.toml",
                wire_name
            )
            .into());
        }
    }

    println!("cargo:rerun-if-changed={spec_path}");

    let fixtures = into_test_fixtures(spec.test_fixtures);
    validate_test_fixtures(&fixtures, &endpoints)?;

    Ok(ParsedEndpoints {
        endpoints,
        fixtures,
    })
}

/// Drop the TOML shape and expose fixtures to the rest of the generator
/// without leaking `serde::Deserialize` plumbing.
fn into_test_fixtures(surface: SurfaceTestFixtures) -> TestFixtures {
    TestFixtures {
        category_symbol: surface.category_symbol,
        concrete_by_type: surface.concrete_by_type,
        concrete_overrides: surface.concrete_overrides,
        mode_overrides: surface.mode_overrides,
        optional_defaults: surface.optional_defaults,
    }
}

/// Closed sets the live-validator matrix in `modes.rs` knows how to consume.
/// Anything outside these tables is a typo in the TOML and silently dropped
/// coverage, so we reject it at build time. Keep these in lockstep with
/// `modes.rs::test_modes_for` and the wire-type tables in `helpers.rs`.
const KNOWN_CATEGORIES_WITH_SYMBOL: &[&str] = &["stock", "option", "index", "rate"];
const KNOWN_WIRE_TYPES: &[&str] = &[
    "Date",
    "Expiration",
    "Strike",
    "Right",
    "Interval",
    "RequestType",
    "Year",
    "Str",
];
const KNOWN_MODE_OVERRIDES: &[&str] = &[
    "concrete_iso",
    "all_strikes_one_exp",
    "all_exps_one_strike",
    "bulk_chain",
    "legacy_zero_wildcard",
];

/// Cross-check the `[test_fixtures]` block against the resolved endpoint set.
///
/// Two failure modes Codex round-1 caught:
///   1. A missing entry in `optional_defaults` silently dropped a `with_<name>`
///      cell and excluded the key from `all_optionals`, erasing coverage with
///      no signal.
///   2. A typo in any fixture map (`expiratoin = "2025-03-21"`) silently
///      fell through to the default — the cell still ran, just with the
///      wrong (unintended) value.
///
/// This pass collects every wrong row and returns one combined error so a
/// dev fixing their TOML sees the full picture in one rebuild.
fn validate_test_fixtures(
    fixtures: &TestFixtures,
    endpoints: &[GeneratedEndpoint],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut errors: Vec<String> = Vec::new();

    // ── Vocabulary derived from the resolved endpoint set ──────────────────
    let known_method_param_names: HashSet<&str> = endpoints
        .iter()
        .flat_map(|ep| ep.params.iter())
        .filter(|p| p.binding == "method")
        .map(|p| p.name.as_str())
        .collect();
    let known_builder_param_names: HashSet<&str> = endpoints
        .iter()
        .flat_map(|ep| ep.params.iter())
        .filter(|p| p.binding == "builder")
        .map(|p| p.name.as_str())
        .collect();

    // ── Required-keys: every builder-bound optional needs a row in
    //    `optional_defaults`. Without one, `with_<name>` is silently dropped
    //    and the param is excluded from `all_optionals` — pure coverage loss.
    let mut missing_optional_defaults: Vec<String> = Vec::new();
    for endpoint in endpoints {
        for param in &endpoint.params {
            if param.binding != "builder" {
                continue;
            }
            if !fixtures.optional_defaults.contains_key(&param.name) {
                missing_optional_defaults.push(format!(
                    "  '{}' (needed for {}.with_{})",
                    param.name, endpoint.name, param.name
                ));
            }
        }
    }
    if !missing_optional_defaults.is_empty() {
        missing_optional_defaults.sort();
        missing_optional_defaults.dedup();
        errors.push(format!(
            "[test_fixtures.optional_defaults] is missing required entries:\n{}",
            missing_optional_defaults.join("\n")
        ));
    }

    // ── Unknown-keys: every map's keys must be in its expected vocabulary.

    // category_symbol → known-with-symbol categories.
    let unknown_categories: Vec<&str> = fixtures
        .category_symbol
        .keys()
        .map(String::as_str)
        .filter(|name| !KNOWN_CATEGORIES_WITH_SYMBOL.contains(name))
        .collect();
    if !unknown_categories.is_empty() {
        let mut sorted = unknown_categories;
        sorted.sort();
        errors.push(format!(
            "[test_fixtures.category_symbol] has unknown categories: {} (expected one of: {})",
            sorted.join(", "),
            KNOWN_CATEGORIES_WITH_SYMBOL.join(", ")
        ));
    }

    // concrete_by_type → known wire types.
    let unknown_wire_types: Vec<&str> = fixtures
        .concrete_by_type
        .keys()
        .map(String::as_str)
        .filter(|name| !KNOWN_WIRE_TYPES.contains(name))
        .collect();
    if !unknown_wire_types.is_empty() {
        let mut sorted = unknown_wire_types;
        sorted.sort();
        errors.push(format!(
            "[test_fixtures.concrete_by_type] has unknown wire types: {} (expected one of: {})",
            sorted.join(", "),
            KNOWN_WIRE_TYPES.join(", ")
        ));
    }

    // concrete_overrides → real method-call param names.
    let unknown_concrete_overrides: Vec<&str> = fixtures
        .concrete_overrides
        .keys()
        .map(String::as_str)
        .filter(|name| !known_method_param_names.contains(name))
        .collect();
    if !unknown_concrete_overrides.is_empty() {
        let mut sorted = unknown_concrete_overrides;
        sorted.sort();
        errors.push(format!(
            "[test_fixtures.concrete_overrides] references param names that are not method-bound \
             on any endpoint: {}",
            sorted.join(", ")
        ));
    }

    // mode_overrides.<mode> → known mode names; inner keys → real method-call params.
    let mut unknown_mode_names: Vec<&str> = Vec::new();
    let mut unknown_mode_param_keys: Vec<String> = Vec::new();
    for (mode_name, overrides) in &fixtures.mode_overrides {
        if !KNOWN_MODE_OVERRIDES.contains(&mode_name.as_str()) {
            unknown_mode_names.push(mode_name.as_str());
            continue;
        }
        for key in overrides.keys() {
            if !known_method_param_names.contains(key.as_str()) {
                unknown_mode_param_keys.push(format!("{mode_name}.{key}"));
            }
        }
    }
    if !unknown_mode_names.is_empty() {
        unknown_mode_names.sort();
        errors.push(format!(
            "[test_fixtures.mode_overrides] has unknown mode names: {} (expected one of: {})",
            unknown_mode_names.join(", "),
            KNOWN_MODE_OVERRIDES.join(", ")
        ));
    }
    if !unknown_mode_param_keys.is_empty() {
        unknown_mode_param_keys.sort();
        errors.push(format!(
            "[test_fixtures.mode_overrides] references param names that are not method-bound on \
             any endpoint: {}",
            unknown_mode_param_keys.join(", ")
        ));
    }

    // optional_defaults → real builder-bound param names.
    let unknown_optional_defaults: Vec<&str> = fixtures
        .optional_defaults
        .keys()
        .map(String::as_str)
        .filter(|name| !known_builder_param_names.contains(name))
        .collect();
    if !unknown_optional_defaults.is_empty() {
        let mut sorted = unknown_optional_defaults;
        sorted.sort();
        errors.push(format!(
            "[test_fixtures.optional_defaults] references param names that are not builder-bound \
             on any endpoint: {}",
            sorted.join(", ")
        ));
    }

    if !errors.is_empty() {
        return Err(format!(
            "endpoint_surface.toml [test_fixtures] validation failed:\n\n{}",
            errors.join("\n\n")
        )
        .into());
    }
    Ok(())
}

/// Resolve the reusable spec language in `endpoint_surface.toml` into concrete endpoints.
///
/// This expands parameter groups, resolves template inheritance, detects
/// cycles, and rejects dead configuration such as unused groups or templates.
fn resolve_surface_endpoints(
    spec: &SurfaceSpec,
) -> Result<Vec<ResolvedSurfaceEndpoint>, Box<dyn std::error::Error>> {
    let mut template_cache = HashMap::new();
    let mut param_group_cache = HashMap::new();
    let mut template_stack = Vec::new();
    let mut param_group_stack = Vec::new();
    let mut used_templates = HashSet::new();
    let mut used_param_groups = HashSet::new();
    let mut endpoints = Vec::with_capacity(spec.endpoints.len());

    for endpoint in &spec.endpoints {
        endpoints.push(resolve_surface_endpoint(
            endpoint,
            spec,
            &mut template_cache,
            &mut param_group_cache,
            &mut template_stack,
            &mut param_group_stack,
            &mut used_templates,
            &mut used_param_groups,
        )?);
    }

    let mut unused_templates = spec
        .templates
        .keys()
        .filter(|name| !used_templates.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    unused_templates.sort();
    if !unused_templates.is_empty() {
        return Err(format!(
            "unused endpoint templates in endpoint_surface.toml: {}",
            unused_templates.join(", ")
        )
        .into());
    }

    let mut unused_param_groups = spec
        .param_groups
        .keys()
        .filter(|name| !used_param_groups.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    unused_param_groups.sort();
    if !unused_param_groups.is_empty() {
        return Err(format!(
            "unused parameter groups in endpoint_surface.toml: {}",
            unused_param_groups.join(", ")
        )
        .into());
    }

    Ok(endpoints)
}

/// Resolve a single concrete endpoint, applying any referenced template first.
#[allow(clippy::too_many_arguments)] // Reason: endpoint resolution needs spec, proto schema, param groups, and shared state in one call.
fn resolve_surface_endpoint(
    endpoint: &SurfaceEndpoint,
    spec: &SurfaceSpec,
    template_cache: &mut HashMap<String, ResolvedTemplate>,
    param_group_cache: &mut HashMap<String, Vec<SurfaceParam>>,
    template_stack: &mut Vec<String>,
    param_group_stack: &mut Vec<String>,
    used_templates: &mut HashSet<String>,
    used_param_groups: &mut HashSet<String>,
) -> Result<ResolvedSurfaceEndpoint, Box<dyn std::error::Error>> {
    let template = if let Some(template_name) = endpoint.template.as_deref() {
        used_templates.insert(template_name.to_string());
        resolve_surface_template(
            template_name,
            spec,
            template_cache,
            param_group_cache,
            template_stack,
            param_group_stack,
            used_templates,
            used_param_groups,
        )?
    } else {
        ResolvedTemplate::default()
    };

    let mut params = template.params;
    params.extend(resolve_param_entries(
        &endpoint.params,
        spec,
        param_group_cache,
        param_group_stack,
        used_param_groups,
    )?);

    Ok(ResolvedSurfaceEndpoint {
        name: endpoint.name.clone(),
        wire_name: endpoint.wire_name.clone().or(template.wire_name),
        description: resolve_required_surface_field(
            endpoint.description.clone().or(template.description),
            &endpoint.name,
            "description",
        )?,
        category: resolve_required_surface_field(
            endpoint.category.clone().or(template.category),
            &endpoint.name,
            "category",
        )?,
        subcategory: resolve_required_surface_field(
            endpoint.subcategory.clone().or(template.subcategory),
            &endpoint.name,
            "subcategory",
        )?,
        rest_path: resolve_required_surface_field(
            endpoint.rest_path.clone().or(template.rest_path),
            &endpoint.name,
            "rest_path",
        )?,
        kind: resolve_required_surface_field(
            endpoint.kind.clone().or(template.kind),
            &endpoint.name,
            "kind",
        )?,
        returns: resolve_required_surface_field(
            endpoint.returns.clone().or(template.returns),
            &endpoint.name,
            "returns",
        )?,
        list_column: endpoint.list_column.clone().or(template.list_column),
        params,
    })
}

/// Resolve a template, including any inherited parent template chain.
#[allow(clippy::too_many_arguments)] // Reason: template resolution needs spec, proto schema, param groups, and shared state in one call.
fn resolve_surface_template(
    name: &str,
    spec: &SurfaceSpec,
    template_cache: &mut HashMap<String, ResolvedTemplate>,
    param_group_cache: &mut HashMap<String, Vec<SurfaceParam>>,
    template_stack: &mut Vec<String>,
    param_group_stack: &mut Vec<String>,
    used_templates: &mut HashSet<String>,
    used_param_groups: &mut HashSet<String>,
) -> Result<ResolvedTemplate, Box<dyn std::error::Error>> {
    if let Some(cached) = template_cache.get(name) {
        return Ok(cached.clone());
    }
    if template_stack.iter().any(|entry| entry == name) {
        let mut cycle = template_stack.clone();
        cycle.push(name.to_string());
        return Err(format!("template inheritance cycle: {}", cycle.join(" -> ")).into());
    }

    let template = spec
        .templates
        .get(name)
        .ok_or_else(|| format!("unknown endpoint template '{}'", name))?;
    template_stack.push(name.to_string());

    let mut resolved = if let Some(parent) = template.extends.as_deref() {
        used_templates.insert(parent.to_string());
        resolve_surface_template(
            parent,
            spec,
            template_cache,
            param_group_cache,
            template_stack,
            param_group_stack,
            used_templates,
            used_param_groups,
        )?
    } else {
        ResolvedTemplate::default()
    };

    if let Some(value) = &template.wire_name {
        resolved.wire_name = Some(value.clone());
    }
    if let Some(value) = &template.description {
        resolved.description = Some(value.clone());
    }
    if let Some(value) = &template.category {
        resolved.category = Some(value.clone());
    }
    if let Some(value) = &template.subcategory {
        resolved.subcategory = Some(value.clone());
    }
    if let Some(value) = &template.rest_path {
        resolved.rest_path = Some(value.clone());
    }
    if let Some(value) = &template.kind {
        resolved.kind = Some(value.clone());
    }
    if let Some(value) = &template.returns {
        resolved.returns = Some(value.clone());
    }
    if let Some(value) = &template.list_column {
        resolved.list_column = Some(value.clone());
    }
    resolved.params.extend(resolve_param_entries(
        &template.params,
        spec,
        param_group_cache,
        param_group_stack,
        used_param_groups,
    )?);

    template_stack.pop();
    template_cache.insert(name.to_string(), resolved.clone());
    Ok(resolved)
}

/// Expand a sequence of parameter entries, recursively resolving group references.
fn resolve_param_entries(
    entries: &[SurfaceParamEntry],
    spec: &SurfaceSpec,
    param_group_cache: &mut HashMap<String, Vec<SurfaceParam>>,
    param_group_stack: &mut Vec<String>,
    used_param_groups: &mut HashSet<String>,
) -> Result<Vec<SurfaceParam>, Box<dyn std::error::Error>> {
    let mut params = Vec::new();
    for entry in entries {
        match entry {
            SurfaceParamEntry::Param(param) => params.push(param.clone()),
            SurfaceParamEntry::Use(param_use) => {
                used_param_groups.insert(param_use.group.clone());
                params.extend(resolve_param_group(
                    &param_use.group,
                    spec,
                    param_group_cache,
                    param_group_stack,
                    used_param_groups,
                )?);
            }
        }
    }
    Ok(params)
}

/// Resolve a reusable parameter group with cycle detection and memoization.
fn resolve_param_group(
    name: &str,
    spec: &SurfaceSpec,
    param_group_cache: &mut HashMap<String, Vec<SurfaceParam>>,
    param_group_stack: &mut Vec<String>,
    used_param_groups: &mut HashSet<String>,
) -> Result<Vec<SurfaceParam>, Box<dyn std::error::Error>> {
    if let Some(cached) = param_group_cache.get(name) {
        return Ok(cached.clone());
    }
    if param_group_stack.iter().any(|entry| entry == name) {
        let mut cycle = param_group_stack.clone();
        cycle.push(name.to_string());
        return Err(format!("parameter group cycle: {}", cycle.join(" -> ")).into());
    }

    let group = spec
        .param_groups
        .get(name)
        .ok_or_else(|| format!("unknown parameter group '{}'", name))?;
    param_group_stack.push(name.to_string());
    let params = resolve_param_entries(
        &group.params,
        spec,
        param_group_cache,
        param_group_stack,
        used_param_groups,
    )?;
    param_group_stack.pop();
    param_group_cache.insert(name.to_string(), params.clone());
    Ok(params)
}

/// Require a fully-resolved endpoint field after template inheritance has been applied.
fn resolve_required_surface_field(
    value: Option<String>,
    endpoint_name: &str,
    field_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    value.ok_or_else(|| {
        format!(
            "endpoint '{}' is missing required field '{}'",
            endpoint_name, field_name
        )
        .into()
    })
}

fn validate_surface_endpoint(
    surface: &ResolvedSurfaceEndpoint,
    wire: &GeneratedEndpoint,
) -> Result<(), Box<dyn std::error::Error>> {
    match surface.kind.as_str() {
        "list" | "parsed" | "stream" => {}
        other => {
            return Err(format!(
                "endpoint '{}' has unsupported kind '{}'",
                surface.name, other
            )
            .into())
        }
    }

    if surface.kind == "list" && surface.returns != "StringList" {
        return Err(format!(
            "list endpoint '{}' must return StringList, got {}",
            surface.name, surface.returns
        )
        .into());
    }
    if surface.kind != "list" && surface.list_column.is_some() {
        return Err(format!(
            "non-list endpoint '{}' cannot define list_column",
            surface.name
        )
        .into());
    }
    if surface.kind == "list" && surface.list_column.is_none() {
        return Err(format!("list endpoint '{}' must define list_column", surface.name).into());
    }
    if surface.returns != wire.return_type {
        return Err(format!(
            "endpoint '{}' declares return type {} but wire-derived model uses {}",
            surface.name, surface.returns, wire.return_type
        )
        .into());
    }

    let wire_params = wire
        .params
        .iter()
        .map(|param| (param.name.as_str(), param))
        .collect::<HashMap<_, _>>();
    let mut surface_names = HashSet::new();
    for param in &surface.params {
        if !surface_names.insert(param.name.clone()) {
            return Err(format!(
                "endpoint '{}' defines duplicate param '{}'",
                surface.name, param.name
            )
            .into());
        }
        let wire_param = wire_params.get(param.name.as_str()).ok_or_else(|| {
            format!(
                "endpoint '{}' declares param '{}' not present in wire endpoint '{}'",
                surface.name, param.name, wire.name
            )
        })?;
        if param.param_type != wire_param.param_type {
            return Err(format!(
                "endpoint '{}.{}' declares type {} but wire-derived model uses {}",
                surface.name, param.name, param.param_type, wire_param.param_type
            )
            .into());
        }
        if wire_param.required && !param.required {
            return Err(format!(
                "endpoint '{}.{}' relaxes a required wire parameter",
                surface.name, param.name
            )
            .into());
        }
        match param.binding.as_str() {
            "method" | "builder" => {}
            other => {
                return Err(format!(
                    "endpoint '{}.{}' has unsupported binding '{}'",
                    surface.name, param.name, other
                )
                .into())
            }
        }
        if param.required && param.default.is_some() {
            return Err(format!(
                "endpoint '{}.{}' cannot define a default for a required parameter",
                surface.name, param.name
            )
            .into());
        }
        if param.binding == "method" && !param.required {
            return Err(format!(
                "endpoint '{}.{}' cannot declare an optional method-bound parameter",
                surface.name, param.name
            )
            .into());
        }
        if param.default.is_some() && param.binding != "builder" {
            return Err(format!(
                "endpoint '{}.{}' can only define defaults for builder-bound parameters",
                surface.name, param.name
            )
            .into());
        }
        if let Some(ref default_val) = param.default {
            validate_default_type(&surface.name, &param.name, &param.param_type, default_val)?;
        }
    }

    for wire_param in &wire.params {
        let missing_from_surface = !surface_names.contains(&wire_param.name);
        let must_be_present = surface.wire_name.is_none() || wire_param.required;
        if missing_from_surface && must_be_present {
            return Err(format!(
                "endpoint '{}' is missing wire parameter '{}' in endpoint_surface.toml",
                surface.name, wire_param.name
            )
            .into());
        }
    }

    Ok(())
}

/// Verify a TOML default value is compatible with its declared param_type.
fn validate_default_type(
    endpoint: &str,
    param: &str,
    param_type: &str,
    default_val: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let ok = match param_type {
        "Int" => default_val.parse::<i32>().is_ok(),
        "Float" => default_val.parse::<f64>().is_ok(),
        "Bool" => default_val == "true" || default_val == "false",
        "Date" => default_val.len() == 8 && default_val.chars().all(|c| c.is_ascii_digit()),
        "Year" => default_val.len() == 4 && default_val.chars().all(|c| c.is_ascii_digit()),
        // String-like types accept any value
        "Symbol" | "Symbols" | "Interval" | "Right" | "Strike" | "Expiration" | "RequestType"
        | "Str" => true,
        _ => true, // unknown types pass (caught elsewhere)
    };
    if !ok {
        return Err(format!(
            "endpoint '{endpoint}.{param}' has default '{default_val}' incompatible with type {param_type}"
        )
        .into());
    }
    Ok(())
}

fn merge_surface_and_wire(
    surface: ResolvedSurfaceEndpoint,
    wire: &GeneratedEndpoint,
) -> GeneratedEndpoint {
    GeneratedEndpoint {
        name: surface.name,
        description: surface.description,
        category: surface.category,
        subcategory: surface.subcategory,
        rest_path: surface.rest_path,
        grpc_name: wire.grpc_name.clone(),
        request_type: wire.request_type.clone(),
        query_type: wire.query_type.clone(),
        fields: wire.fields.clone(),
        params: surface
            .params
            .into_iter()
            .map(|param| GeneratedParam {
                name: param.name,
                description: param.description,
                param_type: param.param_type,
                required: param.required,
                binding: param.binding,
                arg_name: param.arg_name,
                default: param.default,
            })
            .collect(),
        return_type: surface.returns,
        kind: surface.kind,
        list_column: surface.list_column,
    }
}
