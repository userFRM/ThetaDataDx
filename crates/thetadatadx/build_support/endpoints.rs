//! Endpoint surface generation and validation.
//!
//! This module treats `endpoint_surface.toml` as the checked-in source of truth
//! for the normalized SDK surface, while still validating each declared
//! endpoint against the upstream gRPC wire contract in `proto/external.proto`.
//! The resulting joined model drives generated registry metadata, the shared
//! endpoint runtime, and all `DirectClient` methods (list, parsed, and streaming).
//!
//! Note: runtime parameter validation (date format, symbol format, interval,
//! right, year) lives in `crate::validate`. The validators here operate at
//! *build time* on the TOML surface spec and proto schema — a fundamentally
//! different domain — so they are intentionally separate.

// Reason: shared between build.rs and generate_sdk_surfaces binary via #[path]; not all
// functions are used from both entry points.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

/// A checked-in endpoint surface specification file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SurfaceSpec {
    version: u32,
    #[serde(default)]
    param_groups: HashMap<String, SurfaceParamGroup>,
    #[serde(default)]
    templates: HashMap<String, SurfaceTemplate>,
    endpoints: Vec<SurfaceEndpoint>,
}

/// A reusable parameter group declared in `endpoint_surface.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SurfaceParamGroup {
    #[serde(default)]
    params: Vec<SurfaceParamEntry>,
}

/// A reusable endpoint template declared in `endpoint_surface.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SurfaceTemplate {
    #[serde(default)]
    extends: Option<String>,
    #[serde(default)]
    wire_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    subcategory: Option<String>,
    #[serde(default)]
    rest_path: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    returns: Option<String>,
    #[serde(default)]
    list_column: Option<String>,
    #[serde(default)]
    params: Vec<SurfaceParamEntry>,
}

/// A normalized endpoint surface entry loaded from `endpoint_surface.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SurfaceEndpoint {
    name: String,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    wire_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    subcategory: Option<String>,
    #[serde(default)]
    rest_path: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    returns: Option<String>,
    #[serde(default)]
    list_column: Option<String>,
    #[serde(default)]
    params: Vec<SurfaceParamEntry>,
}

/// A normalized endpoint parameter entry loaded from `endpoint_surface.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SurfaceParam {
    name: String,
    description: String,
    param_type: String,
    required: bool,
    binding: String,
    #[serde(default)]
    arg_name: Option<String>,
    #[serde(default)]
    default: Option<String>,
}

/// A single parameter entry or reference inside a parameter group, template, or endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum SurfaceParamEntry {
    Use(SurfaceParamUse),
    Param(SurfaceParam),
}

/// A reference to a reusable parameter group.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SurfaceParamUse {
    #[serde(rename = "use")]
    group: String,
}

#[derive(Debug, Clone, Default)]
struct ResolvedTemplate {
    wire_name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    subcategory: Option<String>,
    rest_path: Option<String>,
    kind: Option<String>,
    returns: Option<String>,
    list_column: Option<String>,
    params: Vec<SurfaceParam>,
}

#[derive(Debug, Clone)]
struct ResolvedSurfaceEndpoint {
    name: String,
    wire_name: Option<String>,
    description: String,
    category: String,
    subcategory: String,
    rest_path: String,
    kind: String,
    returns: String,
    list_column: Option<String>,
    params: Vec<SurfaceParam>,
}

/// A parsed proto field.
#[derive(Debug, Clone)]
struct ProtoField {
    name: String,
    proto_type: String, // "string", "int32", "double", "bool", or "ContractSpec"
    is_optional: bool,
    is_repeated: bool,
}

/// A parsed RPC entry.
#[derive(Debug)]
struct Rpc {
    rpc_name: String,     // e.g. "GetStockHistoryEod"
    request_type: String, // e.g. "StockHistoryEodRequest"
}

#[derive(Debug, Clone)]
struct GeneratedParam {
    name: String,
    description: String,
    param_type: String,
    required: bool,
    binding: String,
    arg_name: Option<String>,
    default: Option<String>,
}

#[derive(Debug, Clone)]
struct GeneratedEndpoint {
    name: String,
    description: String,
    category: String,
    subcategory: String,
    rest_path: String,
    grpc_name: String,
    request_type: String,
    query_type: String,
    fields: Vec<ProtoField>,
    params: Vec<GeneratedParam>,
    return_type: String,
    kind: String,
    list_column: Option<String>,
}

#[derive(Debug, Clone)]
struct ParsedEndpoints {
    endpoints: Vec<GeneratedEndpoint>,
}

/// Parse endpoint metadata from `external.proto` into a reusable intermediate form.
///
/// This build-time parser performs several tightly-coupled passes over the same
/// proto source: RPC discovery, request-query extraction, field expansion,
/// endpoint normalization, and a small set of SDK-specific augmentations. It is
/// intentionally kept in one place so the generated registry, shared endpoint
/// runtime, and SDK surface stay aligned while the explicit endpoint surface
/// spec is validated against the wire contract.
#[allow(clippy::too_many_lines)] // Reason: build-time endpoint parser coordinates multiple passes over one proto source.
fn load_proto_endpoints() -> Result<ParsedEndpoints, Box<dyn std::error::Error>> {
    let proto = std::fs::read_to_string("proto/external.proto")?;

    // ── Parse RPCs ──────────────────────────────────────────────────────────
    let rpc_re = regex::Regex::new(r"rpc\s+(Get\w+)\s*\((\w+)\)\s*returns")?;
    let rpcs: Vec<Rpc> = rpc_re
        .captures_iter(&proto)
        .map(|c| Rpc {
            rpc_name: c[1].to_string(),
            request_type: c[2].to_string(),
        })
        .collect();

    // ── Parse query messages ────────────────────────────────────────────────
    // Everything lives in one package, so ContractSpec is referenced
    // unqualified instead of `endpoints.ContractSpec`.
    let msg_re = regex::Regex::new(r"message\s+(\w+RequestQuery)\s*\{([^}]*)}")?;
    let field_re = regex::Regex::new(
        r"(optional\s+|repeated\s+)?(string|int32|double|bool|ContractSpec)\s+(\w+)\s*=\s*\d+",
    )?;

    let mut query_messages: HashMap<String, Vec<ProtoField>> = HashMap::new();
    for cap in msg_re.captures_iter(&proto) {
        let msg_name = cap[1].to_string();
        let body = &cap[2];
        let fields: Vec<ProtoField> = field_re
            .captures_iter(body)
            .map(|f| ProtoField {
                name: f[3].to_string(),
                proto_type: f[2].to_string(),
                is_optional: f.get(1).is_some_and(|m| m.as_str().starts_with("optional")),
                is_repeated: f.get(1).is_some_and(|m| m.as_str().starts_with("repeated")),
            })
            .collect();
        query_messages.insert(msg_name, fields);
    }

    let mut endpoints = Vec::new();

    for rpc in &rpcs {
        // Derive snake_case method name: GetStockHistoryEod → stock_history_eod
        let method = rpc_to_method(&rpc.rpc_name);

        // Find the query message: StockHistoryEodRequest → StockHistoryEodRequestQuery
        let query_msg_name = format!("{}Query", rpc.request_type);
        let fields = if let Some(f) = query_messages.get(&query_msg_name) {
            f.clone()
        } else {
            eprintln!(
                "warning: no query message '{}' found, skipping {}",
                query_msg_name, rpc.rpc_name
            );
            continue;
        };

        // Expand fields (contract_spec → symbol, expiration, strike, right)
        let params = expand_fields(&fields);

        // Only return_type is cross-validated against the surface spec (line ~804).
        // Category, subcategory, rest_path, description come entirely from the TOML.
        let return_type = derive_return_type(&method);
        let mut params = params
            .into_iter()
            .map(|(name, description, param_type, required)| GeneratedParam {
                name,
                description,
                param_type,
                required,
                binding: String::new(),
                arg_name: None,
                default: None,
            })
            .collect::<Vec<_>>();
        normalize_method_params(&method, &mut params);

        endpoints.push(GeneratedEndpoint {
            name: method,
            description: String::new(),
            category: String::new(),
            subcategory: String::new(),
            rest_path: String::new(),
            grpc_name: format!("get_{}", rpc_to_method(&rpc.rpc_name)),
            request_type: rpc.request_type.clone(),
            query_type: query_msg_name,
            fields,
            params,
            return_type,
            kind: String::new(),
            list_column: None,
        });
    }

    // ── Synthetic extra: stock_history_ohlc_range ──────────────────────────
    // Second SDK-level method on top of the same GetStockHistoryOhlc RPC.
    // The proto supports both shapes via the optional `date` vs
    // `start_date`/`end_date` fields; the SDK exposes them as two distinct
    // methods for nicer ergonomics.  Clone the wire model from the base RPC
    // and rename; the TOML surface spec carries the parameter differences.
    if let Some(ohlc) = endpoints.iter().find(|e| e.name == "stock_history_ohlc") {
        let mut range = ohlc.clone();
        range.name = "stock_history_ohlc_range".into();
        endpoints.push(range);
    }

    Ok(ParsedEndpoints { endpoints })
}

/// Load the explicit endpoint surface spec and join it to proto-derived wire metadata.
fn load_endpoint_specs() -> Result<ParsedEndpoints, Box<dyn std::error::Error>> {
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

    Ok(ParsedEndpoints { endpoints })
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

pub fn generate_all() -> Result<(), Box<dyn std::error::Error>> {
    let parsed = load_endpoint_specs()?;
    generate_endpoint_registry(&parsed)?;
    generate_endpoint_runtime(&parsed)?;
    generate_direct_endpoints(&parsed)?;
    println!("cargo:rerun-if-changed=endpoint_surface.toml");
    println!("cargo:rerun-if-changed=proto/external.proto");
    Ok(())
}

fn generate_endpoint_registry(parsed: &ParsedEndpoints) -> Result<(), Box<dyn std::error::Error>> {
    // ── Generate Rust code ──────────────────────────────────────────────────
    let mut code = String::new();
    code.push_str(
        "// Auto-generated by build.rs from endpoint_surface.toml validated against external.proto.\n",
    );
    code.push_str("// Do not edit manually.\n\n");
    code.push_str("pub static ENDPOINTS: &[EndpointMeta] = &[\n");

    for endpoint in &parsed.endpoints {
        // Streaming endpoints use chunk-by-chunk callback semantics and are not
        // part of the collect-then-return registry.
        if is_streaming_endpoint(endpoint) {
            continue;
        }
        code.push_str("    EndpointMeta {\n");
        writeln!(code, "        name: \"{}\",", endpoint.name).unwrap();
        writeln!(code, "        description: \"{}\",", endpoint.description).unwrap();
        writeln!(code, "        category: \"{}\",", endpoint.category).unwrap();
        writeln!(code, "        subcategory: \"{}\",", endpoint.subcategory).unwrap();
        writeln!(code, "        rest_path: \"{}\",", endpoint.rest_path).unwrap();

        if endpoint.params.is_empty() {
            code.push_str("        params: &[],\n");
        } else {
            code.push_str("        params: &[\n");
            for param in &endpoint.params {
                code.push_str("            ParamMeta {\n");
                writeln!(code, "                name: \"{}\",", param.name).unwrap();
                writeln!(
                    code,
                    "                description: \"{}\",",
                    param.description
                )
                .unwrap();
                writeln!(
                    code,
                    "                param_type: ParamType::{},",
                    param.param_type
                )
                .unwrap();
                writeln!(code, "                required: {},", param.required).unwrap();
                code.push_str("            },\n");
            }
            code.push_str("        ],\n");
        }

        writeln!(
            code,
            "        returns: ReturnType::{},",
            endpoint.return_type
        )
        .unwrap();
        code.push_str("    },\n");
    }

    code.push_str("];\n");

    let out_dir = std::env::var("OUT_DIR")?;
    let dest = Path::new(&out_dir).join("registry_generated.rs");
    std::fs::write(&dest, &code)?;

    Ok(())
}

fn generate_endpoint_runtime(parsed: &ParsedEndpoints) -> Result<(), Box<dyn std::error::Error>> {
    let mut code = String::new();
    code.push_str(
        "// Auto-generated by build.rs from endpoint_surface.toml validated against external.proto.\n",
    );
    code.push_str("// Do not edit manually.\n\n");
    code.push_str("/// Dispatch a validated endpoint call into the generated SDK adapter set.\n");
    code.push_str("///\n");
    code.push_str(
        "/// The build script emits one match arm per endpoint from the shared endpoint\n",
    );
    code.push_str(
        "/// metadata so registry-driven projections stay aligned with the SDK surface.\n",
    );
    code.push_str("pub async fn invoke_generated_endpoint(\n");
    code.push_str("    client: &crate::direct::DirectClient,\n");
    code.push_str("    name: &str,\n");
    code.push_str("    args: &EndpointArgs,\n");
    code.push_str(") -> Result<EndpointOutput, EndpointError> {\n");
    code.push_str("    match name {\n");

    for endpoint in &parsed.endpoints {
        generate_endpoint_dispatch_arm(&mut code, endpoint);
    }

    code.push_str("        _ => Err(EndpointError::UnknownEndpoint(name.to_string())),\n");
    code.push_str("    }\n");
    code.push_str("}\n");

    let out_dir = std::env::var("OUT_DIR")?;
    let dest = Path::new(&out_dir).join("endpoint_generated.rs");
    std::fs::write(&dest, &code)?;

    Ok(())
}

fn generate_direct_endpoints(parsed: &ParsedEndpoints) -> Result<(), Box<dyn std::error::Error>> {
    let mut list_code = String::new();
    let mut parsed_code = String::new();
    let mut stream_code = String::new();
    let header =
        "// Auto-generated by build.rs from endpoint_surface.toml validated against external.proto.\n// Do not edit manually.\n\n";
    list_code.push_str(header);
    list_code.push_str("impl DirectClient {\n");
    parsed_code.push_str(header);
    stream_code.push_str(header);

    for endpoint in &parsed.endpoints {
        if is_simple_list_endpoint(endpoint) {
            generate_direct_list_endpoint(&mut list_code, endpoint);
        } else if is_streaming_endpoint(endpoint) {
            generate_direct_streaming_endpoint(&mut stream_code, endpoint);
        } else {
            generate_direct_parsed_endpoint(&mut parsed_code, endpoint);
        }
    }

    let out_dir = std::env::var("OUT_DIR")?;
    std::fs::write(
        Path::new(&out_dir).join("direct_list_endpoints_generated.rs"),
        format!("{list_code}}}\n"),
    )?;
    std::fs::write(
        Path::new(&out_dir).join("direct_parsed_endpoints_generated.rs"),
        parsed_code,
    )?;
    std::fs::write(
        Path::new(&out_dir).join("direct_streaming_endpoints_generated.rs"),
        stream_code,
    )?;

    Ok(())
}

fn generate_direct_list_endpoint(out: &mut String, endpoint: &GeneratedEndpoint) {
    writeln!(out, "list_endpoint! {{").unwrap();
    writeln!(out, "    #[doc = {:?}]", endpoint.description).unwrap();
    writeln!(
        out,
        "    #[doc = {:?}]",
        format!("gRPC stub: `{}`", endpoint.grpc_name)
    )
    .unwrap();

    let signature = endpoint
        .params
        .iter()
        .map(|param| format!("{}: &str", direct_method_arg_name(endpoint, param)))
        .collect::<Vec<_>>()
        .join(", ");
    if signature.is_empty() {
        writeln!(
            out,
            "    fn {}() -> {:?};",
            endpoint.name,
            endpoint
                .list_column
                .as_deref()
                .expect("list endpoint must declare list_column")
        )
        .unwrap();
    } else {
        writeln!(
            out,
            "    fn {}({signature}) -> {:?};",
            endpoint.name,
            endpoint
                .list_column
                .as_deref()
                .expect("list endpoint must declare list_column")
        )
        .unwrap();
    }

    writeln!(out, "    grpc: {};", endpoint.grpc_name).unwrap();
    writeln!(out, "    request: {};", endpoint.request_type).unwrap();
    if endpoint.fields.is_empty() {
        writeln!(out, "    query: {} {{}};", endpoint.query_type).unwrap();
    } else {
        writeln!(out, "    query: {} {{", endpoint.query_type).unwrap();
        for field in &endpoint.fields {
            let expr = direct_query_field_expr(endpoint, field, true);
            writeln!(out, "        {}: {expr},", field.name).unwrap();
        }
        out.push_str("    };\n");
    }
    out.push_str("}\n\n");
}

fn generate_direct_parsed_endpoint(out: &mut String, endpoint: &GeneratedEndpoint) {
    writeln!(out, "parsed_endpoint! {{").unwrap();
    writeln!(out, "    #[doc = {:?}]", endpoint.description).unwrap();
    writeln!(
        out,
        "    #[doc = {:?}]",
        format!("gRPC stub: `{}`", endpoint.grpc_name)
    )
    .unwrap();
    writeln!(
        out,
        "    builder {}Builder;",
        to_pascal_case(&endpoint.name)
    )
    .unwrap();

    let method_params = endpoint
        .params
        .iter()
        .filter(|param| is_method_call_param(param))
        .collect::<Vec<_>>();
    let signature = method_params
        .iter()
        .map(|param| {
            format!(
                "{}: {}",
                direct_method_arg_name(endpoint, param),
                direct_required_kind(param)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(
        out,
        "    fn {}({signature}) -> {};",
        endpoint.name,
        direct_return_type(&endpoint.return_type)
    )
    .unwrap();

    writeln!(out, "    grpc: {};", endpoint.grpc_name).unwrap();
    writeln!(out, "    request: {};", endpoint.request_type).unwrap();
    if endpoint.fields.is_empty() {
        writeln!(out, "    query: {} {{}};", endpoint.query_type).unwrap();
    } else {
        writeln!(out, "    query: {} {{", endpoint.query_type).unwrap();
        for field in &endpoint.fields {
            let expr = direct_query_field_expr(endpoint, field, false);
            writeln!(out, "        {}: {expr},", field.name).unwrap();
        }
        out.push_str("    };\n");
    }
    writeln!(
        out,
        "    parse: {};",
        direct_parser_name(&endpoint.return_type)
    )
    .unwrap();

    let date_args = method_params
        .iter()
        .filter_map(|param| direct_date_arg_name(endpoint, param))
        .collect::<Vec<_>>();
    if !date_args.is_empty() {
        writeln!(out, "    dates: {};", date_args.join(", ")).unwrap();
    }

    let optional_params = endpoint
        .params
        .iter()
        .filter(|param| !is_method_call_param(param))
        .collect::<Vec<_>>();
    if optional_params.is_empty() {
        out.push_str("    optional {}\n");
    } else {
        out.push_str("    optional {\n");
        for param in optional_params {
            let (kind, default) = direct_optional_kind_and_default(param);
            writeln!(out, "        {}: {} = {},", param.name, kind, default).unwrap();
        }
        out.push_str("    }\n");
    }
    out.push_str("}\n\n");
}

fn generate_direct_streaming_endpoint(out: &mut String, endpoint: &GeneratedEndpoint) {
    let method_params = endpoint
        .params
        .iter()
        .filter(|param| is_method_call_param(param))
        .collect::<Vec<_>>();
    let optional_params = endpoint
        .params
        .iter()
        .filter(|param| !is_method_call_param(param))
        .collect::<Vec<_>>();
    let builder_name = format!("{}Builder", to_pascal_case(&endpoint.name));
    let tick_type = direct_stream_tick_type(&endpoint.return_type);
    let parser_name = direct_parser_name(&endpoint.return_type);

    writeln!(
        out,
        "/// Builder for the [`DirectClient::{}`] streaming endpoint.",
        endpoint.name
    )
    .unwrap();
    writeln!(out, "pub struct {builder_name}<'a> {{").unwrap();
    out.push_str("    client: &'a DirectClient,\n");
    for param in &method_params {
        writeln!(
            out,
            "    pub(crate) {}: {},",
            direct_method_arg_name(endpoint, param),
            direct_required_field_type(param)
        )
        .unwrap();
    }
    for param in &optional_params {
        writeln!(
            out,
            "    pub(crate) {}: {},",
            param.name,
            direct_optional_rust_type(param)
        )
        .unwrap();
    }
    out.push_str("}\n\n");

    writeln!(out, "impl<'a> {builder_name}<'a> {{").unwrap();
    for param in &optional_params {
        writeln!(out, "    #[must_use]").unwrap();
        writeln!(
            out,
            "    pub fn {}(mut self, v: {}) -> Self {{",
            param.name,
            direct_optional_setter_arg_type(param)
        )
        .unwrap();
        writeln!(
            out,
            "        self.{0} = {1};",
            param.name,
            direct_optional_setter_assign_expr(param)
        )
        .unwrap();
        out.push_str("        self\n");
        out.push_str("    }\n");
    }
    out.push_str("\n    /// Execute the streaming request, calling `handler` for each chunk.\n");
    out.push_str("    ///\n");
    out.push_str("    /// # Errors\n");
    out.push_str("    ///\n");
    out.push_str("    /// Returns [`Error`] if the gRPC call fails or response parsing fails.\n");
    out.push_str("    pub async fn stream<F>(self, mut handler: F) -> Result<(), Error>\n");
    out.push_str("    where\n");
    writeln!(out, "        F: FnMut(&[{tick_type}]),").unwrap();
    out.push_str("    {\n");
    writeln!(out, "        let {builder_name} {{").unwrap();
    out.push_str("            client,\n");
    for param in &method_params {
        writeln!(
            out,
            "            {},",
            direct_method_arg_name(endpoint, param)
        )
        .unwrap();
    }
    for param in &optional_params {
        writeln!(out, "            {},", param.name).unwrap();
    }
    out.push_str("        } = self;\n");
    out.push_str("        let _ = &client;\n");
    for arg in method_params
        .iter()
        .filter_map(|param| direct_date_arg_name(endpoint, param))
    {
        writeln!(out, "        validate_date(&{arg})?;").unwrap();
    }
    writeln!(
        out,
        "        tracing::debug!(endpoint = {:?}, \"gRPC streaming request\");",
        endpoint.name
    )
    .unwrap();
    writeln!(
        out,
        "        metrics::counter!(\"thetadatadx.grpc.requests\", \"endpoint\" => {:?}).increment(1);",
        endpoint.name
    )
    .unwrap();
    out.push_str("        let _metrics_start = std::time::Instant::now();\n");
    out.push_str("        let _permit = client.request_semaphore.acquire().await\n");
    out.push_str(
        "            .map_err(|_| Error::Config(\"request semaphore closed\".into()))?;\n",
    );
    writeln!(
        out,
        "        let request = proto::{} {{",
        endpoint.request_type
    )
    .unwrap();
    out.push_str("            query_info: Some(client.query_info()),\n");
    if endpoint.fields.is_empty() {
        writeln!(
            out,
            "            params: Some(proto::{} {{}}),",
            endpoint.query_type
        )
        .unwrap();
    } else {
        writeln!(
            out,
            "            params: Some(proto::{} {{",
            endpoint.query_type
        )
        .unwrap();
        for field in &endpoint.fields {
            let expr = direct_query_field_expr(endpoint, field, false);
            if expr == field.name {
                writeln!(out, "                {expr},").unwrap();
            } else {
                writeln!(out, "                {}: {expr},", field.name).unwrap();
            }
        }
        out.push_str("            }),\n");
    }
    out.push_str("        };\n");
    writeln!(
        out,
        "        let stream = match client.stub().{}(request).await {{",
        endpoint.grpc_name
    )
    .unwrap();
    out.push_str("            Ok(resp) => resp.into_inner(),\n");
    out.push_str("            Err(e) => {\n");
    writeln!(
        out,
        "                metrics::counter!(\"thetadatadx.grpc.errors\", \"endpoint\" => {:?}).increment(1);",
        endpoint.name
    )
    .unwrap();
    out.push_str("                return Err(e.into());\n");
    out.push_str("            }\n");
    out.push_str("        };\n");
    out.push_str("        let result = client.for_each_chunk(stream, |_headers, rows| {\n");
    out.push_str("            let table = proto::DataTable {\n");
    out.push_str("                headers: _headers.to_vec(),\n");
    out.push_str("                data_table: rows.to_vec(),\n");
    out.push_str("            };\n");
    writeln!(out, "            let ticks = {parser_name}(&table);").unwrap();
    out.push_str("            handler(&ticks);\n");
    out.push_str("        }).await;\n");
    out.push_str("        match &result {\n");
    out.push_str("            Ok(()) => {\n");
    writeln!(
        out,
        "                metrics::histogram!(\"thetadatadx.grpc.latency_ms\", \"endpoint\" => {:?})",
        endpoint.name
    )
    .unwrap();
    out.push_str(
        "                    .record(_metrics_start.elapsed().as_secs_f64() * 1_000.0);\n",
    );
    out.push_str("            }\n");
    out.push_str("            Err(_) => {\n");
    writeln!(
        out,
        "                metrics::counter!(\"thetadatadx.grpc.errors\", \"endpoint\" => {:?}).increment(1);",
        endpoint.name
    )
    .unwrap();
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        result\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    writeln!(out, "impl DirectClient {{").unwrap();
    write!(out, "    pub fn {}(&self", endpoint.name).unwrap();
    for param in &method_params {
        write!(
            out,
            ", {}: {}",
            direct_method_arg_name(endpoint, param),
            direct_required_param_type(param)
        )
        .unwrap();
    }
    writeln!(out, ") -> {builder_name}<'_> {{").unwrap();
    writeln!(out, "        {builder_name} {{").unwrap();
    out.push_str("            client: self,\n");
    for param in &method_params {
        writeln!(
            out,
            "            {}: {},",
            direct_method_arg_name(endpoint, param),
            direct_required_store_expr(endpoint, param)
        )
        .unwrap();
    }
    for param in &optional_params {
        let (_, default) = direct_optional_kind_and_default(param);
        writeln!(out, "            {}: {},", param.name, default).unwrap();
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
}

fn direct_method_arg_name(endpoint: &GeneratedEndpoint, param: &GeneratedParam) -> String {
    let _ = endpoint;
    param.arg_name.clone().unwrap_or_else(|| param.name.clone())
}

fn direct_date_arg_name(endpoint: &GeneratedEndpoint, param: &GeneratedParam) -> Option<String> {
    match param.name.as_str() {
        "date" | "start_date" | "end_date" => Some(direct_method_arg_name(endpoint, param)),
        _ => None,
    }
}

fn direct_required_kind(param: &GeneratedParam) -> &'static str {
    if param.param_type == "Symbols" {
        "str_vec"
    } else {
        "str"
    }
}

fn direct_optional_kind_and_default(param: &GeneratedParam) -> (&'static str, String) {
    if let Some(default) = param.default.as_deref() {
        return match param.param_type.as_str() {
            "Str" => ("string", format!("{default:?}.to_string()")),
            "Int" => {
                let value = default.parse::<i32>().unwrap_or_else(|_| {
                    panic!(
                        "invalid int default '{}' for parameter '{}'",
                        default, param.name
                    )
                });
                ("opt_i32", format!("Some({value})"))
            }
            "Float" => {
                let value = default.parse::<f64>().unwrap_or_else(|_| {
                    panic!(
                        "invalid float default '{}' for parameter '{}'",
                        default, param.name
                    )
                });
                ("opt_f64", format!("Some({value:?})"))
            }
            "Bool" => {
                let value = default.parse::<bool>().unwrap_or_else(|_| {
                    panic!(
                        "invalid bool default '{}' for parameter '{}'",
                        default, param.name
                    )
                });
                ("opt_bool", format!("Some({value})"))
            }
            other => panic!(
                "unsupported default for parameter '{}' with type '{}'",
                param.name, other
            ),
        };
    }
    match param.param_type.as_str() {
        "Int" => ("opt_i32", "None".into()),
        "Float" => ("opt_f64", "None".into()),
        "Bool" => ("opt_bool", "None".into()),
        _ => ("opt_str", "None".into()),
    }
}

fn direct_optional_rust_type(param: &GeneratedParam) -> &'static str {
    match direct_optional_kind_and_default(param).0 {
        "opt_i32" => "Option<i32>",
        "opt_f64" => "Option<f64>",
        "opt_bool" => "Option<bool>",
        "string" => "String",
        _ => "Option<String>",
    }
}

fn direct_optional_setter_arg_type(param: &GeneratedParam) -> &'static str {
    match direct_optional_kind_and_default(param).0 {
        "opt_i32" => "i32",
        "opt_f64" => "f64",
        "opt_bool" => "bool",
        "string" => "&str",
        _ => "&str",
    }
}

fn direct_optional_setter_assign_expr(param: &GeneratedParam) -> &'static str {
    match direct_optional_kind_and_default(param).0 {
        "opt_i32" | "opt_f64" | "opt_bool" => "Some(v)",
        "string" => "v.to_string()",
        _ => "Some(v.to_string())",
    }
}

fn direct_required_field_type(param: &GeneratedParam) -> &'static str {
    if param.param_type == "Symbols" {
        "Vec<String>"
    } else {
        "String"
    }
}

fn direct_required_param_type(param: &GeneratedParam) -> &'static str {
    if param.param_type == "Symbols" {
        "&[&str]"
    } else {
        "&str"
    }
}

fn direct_required_store_expr(endpoint: &GeneratedEndpoint, param: &GeneratedParam) -> String {
    let arg_name = direct_method_arg_name(endpoint, param);
    if param.param_type == "Symbols" {
        format!("{arg_name}.iter().map(|s| s.to_string()).collect()")
    } else {
        format!("{arg_name}.to_string()")
    }
}

fn is_simple_list_endpoint(endpoint: &GeneratedEndpoint) -> bool {
    endpoint.kind == "list"
}

fn is_streaming_endpoint(endpoint: &GeneratedEndpoint) -> bool {
    endpoint.kind == "stream"
}

fn is_method_call_param(param: &GeneratedParam) -> bool {
    param.binding == "method"
}

fn required_getter_name(param_type: &str) -> &'static str {
    match param_type {
        "Symbol" => "required_symbol",
        "Symbols" => "required_symbols",
        "Date" => "required_date",
        "Expiration" => "required_expiration",
        "Strike" => "required_strike",
        "Interval" => "required_interval",
        "Right" => "required_right",
        "Int" => "required_int32",
        "Float" => "required_float64",
        "Bool" => "required_bool",
        "Year" => "required_year",
        _ => "required_str",
    }
}

fn optional_getter_name(param_type: &str) -> &'static str {
    match param_type {
        "Date" => "optional_date",
        "Expiration" => "optional_expiration",
        "Strike" => "optional_strike",
        "Int" => "optional_int32",
        "Float" => "optional_float64",
        "Bool" => "optional_bool",
        _ => "optional_str",
    }
}

fn is_symbols_param(param: &GeneratedParam) -> bool {
    param.param_type == "Symbols"
}

fn call_arg_name(param: &GeneratedParam) -> String {
    if is_symbols_param(param) {
        "&symbol_refs".into()
    } else {
        param.name.clone()
    }
}

/// Map a collection return type (e.g. `TradeTicks`) to the per-chunk tick type
/// (e.g. `TradeTick`) used by generated direct streaming builders.
fn direct_stream_tick_type(return_type: &str) -> &'static str {
    match return_type {
        "TradeTicks" => "TradeTick",
        "QuoteTicks" => "QuoteTick",
        other => panic!("unsupported streaming tick type: {other}"),
    }
}

fn direct_return_type(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "Vec<EodTick>",
        "OhlcTicks" => "Vec<OhlcTick>",
        "TradeTicks" => "Vec<TradeTick>",
        "QuoteTicks" => "Vec<QuoteTick>",
        "TradeQuoteTicks" => "Vec<TradeQuoteTick>",
        "OpenInterestTicks" => "Vec<OpenInterestTick>",
        "MarketValueTicks" => "Vec<MarketValueTick>",
        "GreeksTicks" => "Vec<GreeksTick>",
        "IvTicks" => "Vec<IvTick>",
        "PriceTicks" => "Vec<PriceTick>",
        "CalendarDays" => "Vec<CalendarDay>",
        "InterestRateTicks" => "Vec<InterestRateTick>",
        "OptionContracts" => "Vec<OptionContract>",
        other => panic!("unsupported direct return type: {other}"),
    }
}

fn direct_parser_name(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "decode::parse_eod_ticks",
        "OhlcTicks" => "decode::parse_ohlc_ticks",
        "TradeTicks" => "decode::parse_trade_ticks",
        "QuoteTicks" => "decode::parse_quote_ticks",
        "TradeQuoteTicks" => "decode::parse_trade_quote_ticks",
        "OpenInterestTicks" => "decode::parse_open_interest_ticks",
        "MarketValueTicks" => "decode::parse_market_value_ticks",
        "GreeksTicks" => "decode::parse_greeks_ticks",
        "IvTicks" => "decode::parse_iv_ticks",
        "PriceTicks" => "decode::parse_price_ticks",
        "CalendarDays" => "decode::parse_calendar_days_v3",
        "InterestRateTicks" => "decode::parse_interest_rate_ticks",
        "OptionContracts" => "decode::parse_option_contracts_v3",
        other => panic!("unsupported parser return type: {other}"),
    }
}

fn direct_query_field_expr(
    endpoint: &GeneratedEndpoint,
    field: &ProtoField,
    list_context: bool,
) -> String {
    if field.proto_type == "ContractSpec" {
        return "contract_spec!(symbol, expiration, strike, right)".into();
    }
    if field.name == "date" && endpoint.name == "stock_history_ohlc_range" {
        return "None".into();
    }

    let param = endpoint
        .params
        .iter()
        .find(|param| param.name == field.name)
        .unwrap_or_else(|| {
            panic!(
                "missing param metadata for {}::{}",
                endpoint.name, field.name
            )
        });
    let arg_name = direct_method_arg_name(endpoint, param);
    let is_method_param = is_method_call_param(param);

    match field.name.as_str() {
        "symbol" if field.is_repeated => {
            if param.param_type == "Symbols" {
                "symbols.clone()".into()
            } else if list_context {
                format!("vec![{arg_name}.to_string()]")
            } else {
                format!("vec![{arg_name}.clone()]")
            }
        }
        "interval" => format!("normalize_interval(&{arg_name})"),
        "time_of_day" => format!("normalize_time_of_day(&{arg_name})"),
        // Top-level `expiration` fields on query messages get the same
        // wire canonicalization as the ContractSpec copy: `0` -> `*`, ISO
        // dashes stripped. Keeps the two expiration values on the request
        // in agreement and prevents the server from seeing a raw `0` on
        // either path. In list-endpoint context the arg is already `&str`
        // (no extra borrow); in the parsed-endpoint context it's owned.
        "expiration" => {
            if list_context {
                format!("normalize_expiration({arg_name})")
            } else {
                format!("normalize_expiration(&{arg_name})")
            }
        }
        "start_time" | "end_time" => format!("Some({arg_name}.clone())"),
        "venue" if endpoint.category == "stock" => {
            "venue.clone().or_else(|| Some(\"nqb\".to_string()))".into()
        }
        _ if field.proto_type == "string" => {
            if field.is_optional {
                if is_method_param {
                    format!("Some({arg_name}.clone())")
                } else {
                    format!("{arg_name}.clone()")
                }
            } else if list_context {
                format!("{arg_name}.to_string()")
            } else {
                format!("{arg_name}.clone()")
            }
        }
        _ => arg_name,
    }
}

fn to_pascal_case(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<String>()
}

fn generate_endpoint_dispatch_arm(out: &mut String, endpoint: &GeneratedEndpoint) {
    writeln!(out, "        \"{}\" => {{", endpoint.name).unwrap();

    if is_simple_list_endpoint(endpoint) {
        for param in &endpoint.params {
            emit_required_arg(out, endpoint, param);
        }
        let args = endpoint
            .params
            .iter()
            .map(call_arg_name)
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            out,
            "            let values = client.{}({args}).await?;",
            endpoint.name
        )
        .unwrap();
        writeln!(
            out,
            "            Ok(EndpointOutput::{}(values))",
            endpoint.return_type
        )
        .unwrap();
        out.push_str("        }\n");
        return;
    }

    let method_call_params = endpoint
        .params
        .iter()
        .filter(|param| is_method_call_param(param))
        .collect::<Vec<_>>();
    let builder_params = endpoint
        .params
        .iter()
        .filter(|param| !is_method_call_param(param))
        .collect::<Vec<_>>();

    for param in &method_call_params {
        emit_required_arg(out, endpoint, param);
    }

    let call_args = method_call_params
        .into_iter()
        .map(call_arg_name)
        .collect::<Vec<_>>()
        .join(", ");

    if is_streaming_endpoint(endpoint) {
        writeln!(
            out,
            "            let mut builder = client.{}({call_args});",
            endpoint.name
        )
        .unwrap();

        for param in builder_params {
            let getter = optional_getter_name(&param.param_type);
            writeln!(
                out,
                "            if let Some(value) = args.{getter}(\"{}\")? {{",
                param.name
            )
            .unwrap();
            writeln!(
                out,
                "                builder = builder.{}(value);",
                param.name
            )
            .unwrap();
            out.push_str("            }\n");
        }

        out.push_str("            let mut result = Vec::new();\n");
        out.push_str("            builder\n");
        out.push_str("                .stream(|chunk| result.extend_from_slice(chunk))\n");
        out.push_str("                .await?;\n");
        writeln!(
            out,
            "            Ok(EndpointOutput::{}(result))",
            endpoint.return_type
        )
        .unwrap();
        out.push_str("        }\n");
        return;
    }

    if builder_params.is_empty() {
        writeln!(
            out,
            "            let result = client.{}({call_args}).await?;",
            endpoint.name
        )
        .unwrap();
    } else {
        writeln!(
            out,
            "            let mut builder = client.{}({call_args});",
            endpoint.name
        )
        .unwrap();

        for param in builder_params {
            let getter = optional_getter_name(&param.param_type);
            writeln!(
                out,
                "            if let Some(value) = args.{getter}(\"{}\")? {{",
                param.name
            )
            .unwrap();
            writeln!(
                out,
                "                builder = builder.{}(value);",
                param.name
            )
            .unwrap();
            out.push_str("            }\n");
        }

        out.push_str("            let result = builder.await?;\n");
    }
    writeln!(
        out,
        "            Ok(EndpointOutput::{}(result))",
        endpoint.return_type
    )
    .unwrap();
    out.push_str("        }\n");
}

fn emit_required_arg(out: &mut String, _endpoint: &GeneratedEndpoint, param: &GeneratedParam) {
    if param.param_type == "Symbols" {
        writeln!(
            out,
            "            let symbol_values = args.required_symbols(\"{}\")?;",
            param.name
        )
        .unwrap();
        out.push_str(
            "            let symbol_refs: Vec<&str> = symbol_values.iter().map(String::as_str).collect();\n",
        );
        return;
    }

    let getter = required_getter_name(&param.param_type);
    writeln!(
        out,
        "            let {} = args.{getter}(\"{}\")?;",
        param.name, param.name
    )
    .unwrap();
}

fn normalize_method_params(method: &str, params: &mut [GeneratedParam]) {
    let supports_symbol_lists =
        method.starts_with("stock_snapshot_") || method.starts_with("index_snapshot_");

    if !supports_symbol_lists {
        for param in params.iter_mut() {
            if param.name == "symbol" && param.param_type == "Symbols" {
                param.param_type = "Symbol".into();
                param.description = "Ticker symbol (e.g. AAPL)".into();
            }
        }
    }
}

fn is_simple_list_method(method: &str) -> bool {
    method.ends_with("_list_symbols")
        || method.ends_with("_list_dates")
        || method.ends_with("_list_expirations")
        || method.ends_with("_list_strikes")
}

/// Convert `GetStockHistoryEod` → `stock_history_eod`.
fn rpc_to_method(rpc_name: &str) -> String {
    // Strip leading "Get"
    let name = rpc_name.strip_prefix("Get").unwrap_or(rpc_name); // build script: panic is intentional
                                                                 // PascalCase → snake_case
    let mut result = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap()); // build script: panic is intentional
        } else {
            result.push(ch);
        }
    }
    result
}

/// Expand proto fields, replacing `contract_spec` with (symbol, expiration, strike, right).
///
/// Many option query messages carry both a `ContractSpec` (contract identity,
/// expanded here to 4 fields) AND an explicit top-level `expiration` field
/// (the query range expiration — e.g. "include all contracts expiring by..."),
/// which would otherwise collide with the contract's own expiration. Any
/// post-expansion duplicate parameter name is dropped in favor of the first
/// occurrence (ContractSpec wins, since it is structurally the contract
/// identity the user really cares about).
fn expand_fields(fields: &[ProtoField]) -> Vec<(String, String, String, bool)> {
    let mut params: Vec<(String, String, String, bool)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let push = |params: &mut Vec<(String, String, String, bool)>,
                seen: &mut std::collections::HashSet<String>,
                entry: (String, String, String, bool)| {
        if seen.insert(entry.0.clone()) {
            params.push(entry);
        }
    };

    for f in fields {
        if f.proto_type == "ContractSpec" {
            // Expand to the 4 contract spec fields (symbol, expiration, strike, right).
            push(
                &mut params,
                &mut seen,
                (
                    "symbol".into(),
                    "Underlying symbol (e.g. AAPL)".into(),
                    "Symbol".into(),
                    true,
                ),
            );
            push(
                &mut params,
                &mut seen,
                (
                    "expiration".into(),
                    "Expiration date YYYYMMDD".into(),
                    "Expiration".into(),
                    true,
                ),
            );
            push(
                &mut params,
                &mut seen,
                (
                    "strike".into(),
                    "Strike price (raw integer)".into(),
                    "Strike".into(),
                    true,
                ),
            );
            push(
                &mut params,
                &mut seen,
                (
                    "right".into(),
                    "C for call, P for put".into(),
                    "Right".into(),
                    true,
                ),
            );
        } else {
            let (param_type, desc) = map_field(&f.name, &f.proto_type, f.is_repeated);
            let required = !f.is_optional;
            push(
                &mut params,
                &mut seen,
                (f.name.clone(), desc, param_type, required),
            );
        }
    }
    params
}

/// Map a proto field (name + type + repeated) to (`ParamType` variant name, description).
fn map_field(name: &str, proto_type: &str, is_repeated: bool) -> (String, String) {
    // Repeated string symbol → Symbols
    if is_repeated && name == "symbol" {
        return (
            "Symbols".into(),
            "Comma-separated ticker symbols (e.g. AAPL,MSFT)".into(),
        );
    }

    match (proto_type, name) {
        ("string", "symbol") => ("Symbol".into(), "Ticker symbol (e.g. AAPL)".into()),
        ("string", "start_date") => ("Date".into(), "Start date YYYYMMDD".into()),
        ("string", "end_date") => ("Date".into(), "End date YYYYMMDD".into()),
        ("string", "date") => ("Date".into(), "Date YYYYMMDD".into()),
        ("string", "interval") => (
            "Interval".into(),
            "Accepts milliseconds (60000) or shorthand (1m). Presets: 100ms, 500ms, 1s, 5s, 10s, 15s, 30s, 1m, 5m, 10m, 15m, 30m, 1h.".into(),
        ),
        ("string", "right") => ("Right".into(), "C for call, P for put".into()),
        ("string", "strike") => (
            "Strike".into(),
            "Strike price in dollars as a string (e.g. 500 or 17.5)".into(),
        ),
        ("string", "expiration") => ("Expiration".into(), "Expiration date YYYYMMDD".into()),
        ("string", "request_type") => (
            "RequestType".into(),
            "Request type: EOD, TRADE, QUOTE, OHLC, etc.".into(),
        ),
        ("string", "year") => ("Year".into(), "4-digit year (e.g. 2024)".into()),
        ("string", "time_of_day") => (
            "Str".into(),
            "ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM; legacy 34200000 is also accepted)".into(),
        ),
        ("string", "venue") => ("Str".into(), "Venue/exchange filter".into()),
        ("string", "min_time") => ("Str".into(), "Minimum time filter".into()),
        ("string", "start_time") => ("Str".into(), "Start time filter".into()),
        ("string", "end_time") => ("Str".into(), "End time filter".into()),
        ("string", "rate_type") => ("Str".into(), "Rate type".into()),
        ("string", "version") => ("Str".into(), "Greeks model version".into()),
        ("double", _) => ("Float".into(), humanize_name(name).clone()),
        ("int32", "max_dte") => ("Int".into(), "Maximum days to expiration".into()),
        ("int32", "strike_range") => ("Int".into(), "Strike range filter".into()),
        ("int32", _) => ("Int".into(), humanize_name(name).clone()),
        ("bool", "exclusive") => ("Bool".into(), "Exclusive time boundary".into()),
        ("bool", "use_market_value") => ("Bool".into(), "Use market value for Greeks".into()),
        ("bool", "underlyer_use_nbbo") => ("Bool".into(), "Use NBBO for underlyer price".into()),
        ("bool", _) => ("Bool".into(), humanize_name(name).clone()),
        _ => ("Str".into(), humanize_name(name).clone()),
    }
}

fn humanize_name(name: &str) -> String {
    name.replace('_', " ")
        .split_whitespace()
        .enumerate()
        .map(|(i, w)| {
            if i == 0 {
                let mut c = w.chars();
                match c.next() {
                    Some(first) => first.to_uppercase().to_string() + c.as_str(),
                    None => String::new(),
                }
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn derive_return_type(method: &str) -> String {
    if is_simple_list_method(method) {
        return "StringList".into();
    }

    if method == "option_list_contracts" {
        return "OptionContracts".into();
    }

    if method.starts_with("calendar_") {
        return "CalendarDays".into();
    }

    if method.starts_with("interest_rate_") {
        return "InterestRateTicks".into();
    }

    if method.contains("_trade_quote") {
        return "TradeQuoteTicks".into();
    }

    if method.contains("_open_interest") {
        return "OpenInterestTicks".into();
    }

    if method.contains("_market_value") {
        return "MarketValueTicks".into();
    }

    if method.contains("greeks_implied_volatility") {
        return "IvTicks".into();
    }

    if method.contains("_greeks_") {
        return "GreeksTicks".into();
    }

    if method == "index_snapshot_price"
        || method == "index_history_price"
        || method == "index_at_time_price"
    {
        return "PriceTicks".into();
    }

    if method.ends_with("_history_eod") {
        return "EodTicks".into();
    }

    if method.contains("_ohlc") {
        return "OhlcTicks".into();
    }

    if method.contains("_trade") || method.ends_with("_trade") {
        return "TradeTicks".into();
    }

    if method.contains("_quote") || method.ends_with("_quote") {
        return "QuoteTicks".into();
    }

    panic!("unhandled return type mapping for endpoint {method}");
}

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
            contents: render_ffi_endpoint_request_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "ffi/src/endpoint_with_options.rs",
            contents: render_ffi_with_options(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/endpoint_request_options.h.inc",
            contents: render_c_endpoint_request_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/endpoint_options.go",
            contents: render_go_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/historical.go",
            contents: render_go_historical(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/endpoint_with_options.h.inc",
            contents: render_go_endpoint_with_options_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/endpoint_request_options.h.inc",
            contents: render_c_endpoint_request_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/endpoint_options.hpp.inc",
            contents: render_cpp_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/historical.hpp.inc",
            contents: render_cpp_historical_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/endpoint_with_options.h.inc",
            contents: render_c_endpoint_with_options_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/historical.cpp.inc",
            contents: render_cpp_historical_defs(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/historical_methods.rs",
            contents: render_python_historical_methods(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "scripts/validate_cli.py",
            contents: render_cli_validate(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "scripts/validate_python.py",
            contents: render_python_validate(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/validate.go",
            contents: render_go_validate(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/examples/validate.cpp",
            contents: render_cpp_validate(&parsed.endpoints),
        },
    ])
}

fn method_params(endpoint: &GeneratedEndpoint) -> Vec<&GeneratedParam> {
    endpoint
        .params
        .iter()
        .filter(|param| is_method_call_param(param))
        .collect()
}

fn builder_params(endpoint: &GeneratedEndpoint) -> Vec<&GeneratedParam> {
    endpoint
        .params
        .iter()
        .filter(|param| !is_method_call_param(param))
        .collect()
}

fn has_builder_params(endpoint: &GeneratedEndpoint) -> bool {
    endpoint
        .params
        .iter()
        .any(|param| !is_method_call_param(param))
}

fn collect_builder_params(endpoints: &[GeneratedEndpoint]) -> Vec<GeneratedParam> {
    let mut seen = HashSet::new();
    let mut params = Vec::new();
    for endpoint in endpoints {
        for param in builder_params(endpoint) {
            if seen.insert(param.name.clone()) {
                params.push(param.clone());
            }
        }
    }
    params
}

fn go_segment_pascal(segment: &str) -> String {
    match segment {
        "eod" => "EOD".into(),
        "ohlc" => "OHLC".into(),
        "iv" => "IV".into(),
        "dte" => "DTE".into(),
        "nbbo" => "NBBO".into(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        }
    }
}

fn to_go_exported_name(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(go_segment_pascal)
        .collect::<String>()
}

fn to_camel_case(value: &str) -> String {
    let pascal = to_go_exported_name(value);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn python_optional_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "Option<i32>",
        "Float" => "Option<f64>",
        "Bool" => "Option<bool>",
        _ => "Option<&str>",
    }
}

fn go_result_type(return_type: &str) -> &'static str {
    match return_type {
        "StringList" => "[]string",
        "EodTicks" => "[]EodTick",
        "OhlcTicks" => "[]OhlcTick",
        "TradeTicks" => "[]TradeTick",
        "QuoteTicks" => "[]QuoteTick",
        "TradeQuoteTicks" => "[]TradeQuoteTick",
        "OpenInterestTicks" => "[]OpenInterestTick",
        "MarketValueTicks" => "[]MarketValueTick",
        "GreeksTicks" => "[]GreeksTick",
        "IvTicks" => "[]IVTick",
        "PriceTicks" => "[]PriceTick",
        "CalendarDays" => "[]CalendarDay",
        "InterestRateTicks" => "[]InterestRateTick",
        "OptionContracts" => "[]OptionContract",
        other => panic!("unsupported Go result type: {other}"),
    }
}

fn go_converter_name(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "convertEodTicks",
        "OhlcTicks" => "convertOhlcTicks",
        "TradeTicks" => "convertTradeTicks",
        "QuoteTicks" => "convertQuoteTicks",
        "TradeQuoteTicks" => "convertTradeQuoteTicks",
        "OpenInterestTicks" => "convertOpenInterestTicks",
        "MarketValueTicks" => "convertMarketValueTicks",
        "GreeksTicks" => "convertGreeksTicks",
        "IvTicks" => "convertIvTicks",
        "PriceTicks" => "convertPriceTicks",
        "CalendarDays" => "convertCalendarDays",
        "InterestRateTicks" => "convertInterestRateTicks",
        "OptionContracts" => "convertOptionContracts",
        other => panic!("unsupported Go converter type: {other}"),
    }
}

fn ffi_array_type(return_type: &str) -> &'static str {
    match return_type {
        "StringList" => "TdxStringArray",
        "EodTicks" => "TdxEodTickArray",
        "OhlcTicks" => "TdxOhlcTickArray",
        "TradeTicks" => "TdxTradeTickArray",
        "QuoteTicks" => "TdxQuoteTickArray",
        "TradeQuoteTicks" => "TdxTradeQuoteTickArray",
        "OpenInterestTicks" => "TdxOpenInterestTickArray",
        "MarketValueTicks" => "TdxMarketValueTickArray",
        "GreeksTicks" => "TdxGreeksTickArray",
        "IvTicks" => "TdxIvTickArray",
        "PriceTicks" => "TdxPriceTickArray",
        "CalendarDays" => "TdxCalendarDayArray",
        "InterestRateTicks" => "TdxInterestRateTickArray",
        "OptionContracts" => "TdxOptionContractArray",
        other => panic!("unsupported FFI array type: {other}"),
    }
}

fn ffi_array_empty_expr(return_type: &str) -> &'static str {
    match return_type {
        "OptionContracts" => {
            "TdxOptionContractArray {\n        data: ptr::null(),\n        len: 0,\n    }"
        }
        _ => "ARRAY_EMPTY",
    }
}

fn ffi_output_variant(return_type: &str) -> &'static str {
    match return_type {
        "StringList" => "StringList",
        "EodTicks" => "EodTicks",
        "OhlcTicks" => "OhlcTicks",
        "TradeTicks" => "TradeTicks",
        "QuoteTicks" => "QuoteTicks",
        "TradeQuoteTicks" => "TradeQuoteTicks",
        "OpenInterestTicks" => "OpenInterestTicks",
        "MarketValueTicks" => "MarketValueTicks",
        "GreeksTicks" => "GreeksTicks",
        "IvTicks" => "IvTicks",
        "PriceTicks" => "PriceTicks",
        "CalendarDays" => "CalendarDays",
        "InterestRateTicks" => "InterestRateTicks",
        "OptionContracts" => "OptionContracts",
        other => panic!("unsupported endpoint output variant: {other}"),
    }
}

fn ffi_from_vec_expr(return_type: &str) -> &'static str {
    match return_type {
        "StringList" => "TdxStringArray::from_vec(values)",
        "EodTicks" => "TdxEodTickArray::from_vec(values)",
        "OhlcTicks" => "TdxOhlcTickArray::from_vec(values)",
        "TradeTicks" => "TdxTradeTickArray::from_vec(values)",
        "QuoteTicks" => "TdxQuoteTickArray::from_vec(values)",
        "TradeQuoteTicks" => "TdxTradeQuoteTickArray::from_vec(values)",
        "OpenInterestTicks" => "TdxOpenInterestTickArray::from_vec(values)",
        "MarketValueTicks" => "TdxMarketValueTickArray::from_vec(values)",
        "GreeksTicks" => "TdxGreeksTickArray::from_vec(values)",
        "IvTicks" => "TdxIvTickArray::from_vec(values)",
        "PriceTicks" => "TdxPriceTickArray::from_vec(values)",
        "CalendarDays" => "TdxCalendarDayArray::from_vec(values)",
        "InterestRateTicks" => "TdxInterestRateTickArray::from_vec(values)",
        "OptionContracts" => "TdxOptionContractArray::from_vec(values)",
        other => panic!("unsupported FFI from_vec return type: {other}"),
    }
}

fn ffi_header_return_type(return_type: &str) -> &'static str {
    match return_type {
        "OptionContracts" => "TdxOptionContractArray",
        "StringList" | "EodTicks" | "OhlcTicks" | "TradeTicks" | "QuoteTicks"
        | "TradeQuoteTicks" | "OpenInterestTicks" | "MarketValueTicks" | "GreeksTicks"
        | "IvTicks" | "PriceTicks" | "CalendarDays" | "InterestRateTicks" => "TdxTickArray",
        other => panic!("unsupported Go/C header return type: {other}"),
    }
}

fn ffi_free_fn(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "C.tdx_eod_tick_array_free",
        "OhlcTicks" => "C.tdx_ohlc_tick_array_free",
        "TradeTicks" => "C.tdx_trade_tick_array_free",
        "QuoteTicks" => "C.tdx_quote_tick_array_free",
        "TradeQuoteTicks" => "C.tdx_trade_quote_tick_array_free",
        "OpenInterestTicks" => "C.tdx_open_interest_tick_array_free",
        "MarketValueTicks" => "C.tdx_market_value_tick_array_free",
        "GreeksTicks" => "C.tdx_greeks_tick_array_free",
        "IvTicks" => "C.tdx_iv_tick_array_free",
        "PriceTicks" => "C.tdx_price_tick_array_free",
        "CalendarDays" => "C.tdx_calendar_day_array_free",
        "InterestRateTicks" => "C.tdx_interest_rate_tick_array_free",
        "OptionContracts" => "C.tdx_option_contract_array_free",
        other => panic!("unsupported FFI free fn for Go: {other}"),
    }
}

fn cpp_value_type(return_type: &str) -> &'static str {
    match return_type {
        "StringList" => "std::string",
        "EodTicks" => "EodTick",
        "OhlcTicks" => "OhlcTick",
        "TradeTicks" => "TradeTick",
        "QuoteTicks" => "QuoteTick",
        "TradeQuoteTicks" => "TradeQuoteTick",
        "OpenInterestTicks" => "OpenInterestTick",
        "MarketValueTicks" => "MarketValueTick",
        "GreeksTicks" => "GreeksTick",
        "IvTicks" => "IvTick",
        "PriceTicks" => "PriceTick",
        "CalendarDays" => "CalendarDay",
        "InterestRateTicks" => "InterestRateTick",
        "OptionContracts" => "OptionContract",
        other => panic!("unsupported C++ value type: {other}"),
    }
}

fn cpp_converter_expr(return_type: &str) -> String {
    match return_type {
        "StringList" => "return detail::check_string_array(arr);".into(),
        "OptionContracts" => "return detail::option_contract_array_to_vector(arr);".into(),
        other => format!(
            "auto result = detail::to_vector(arr.data, arr.len);\n    {}(arr);\n    return result;",
            ffi_free_fn(other).trim_start_matches("C.")
        ),
    }
}

fn python_converter(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "eod_tick_to_dict",
        "OhlcTicks" => "ohlc_tick_to_dict",
        "TradeTicks" => "trade_tick_to_dict",
        "QuoteTicks" => "quote_tick_to_dict",
        "TradeQuoteTicks" => "trade_quote_tick_to_dict",
        "OpenInterestTicks" => "open_interest_tick_to_dict",
        "MarketValueTicks" => "market_value_tick_to_dict",
        "GreeksTicks" => "greeks_tick_to_dict",
        "IvTicks" => "iv_tick_to_dict",
        "PriceTicks" => "price_tick_to_dict",
        "CalendarDays" => "calendar_day_to_dict",
        "InterestRateTicks" => "interest_rate_tick_to_dict",
        "OptionContracts" => "option_contract_to_dict",
        other => panic!("unsupported Python converter: {other}"),
    }
}

fn python_columnar_converter(return_type: &str) -> &'static str {
    match return_type {
        "EodTicks" => "eod_ticks_to_columnar",
        "OhlcTicks" => "ohlc_ticks_to_columnar",
        "TradeTicks" => "trade_ticks_to_columnar",
        "QuoteTicks" => "quote_ticks_to_columnar",
        "TradeQuoteTicks" => "trade_quote_ticks_to_columnar",
        "OpenInterestTicks" => "open_interest_ticks_to_columnar",
        "MarketValueTicks" => "market_value_ticks_to_columnar",
        "GreeksTicks" => "greeks_ticks_to_columnar",
        "IvTicks" => "iv_ticks_to_columnar",
        "PriceTicks" => "price_ticks_to_columnar",
        "CalendarDays" => "calendar_days_to_columnar",
        "InterestRateTicks" => "interest_rate_ticks_to_columnar",
        "OptionContracts" => "option_contracts_to_columnar",
        other => panic!("unsupported Python columnar converter: {other}"),
    }
}

fn builder_value_type_name(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "int32_t",
        "Float" => "double",
        "Bool" => "bool",
        _ => "std::string",
    }
}

fn builder_copy_expr(param: &GeneratedParam, source: &str) -> String {
    match param.param_type.as_str() {
        "Int" => format!("{} = {}", param.name, source),
        "Float" => format!("{} = {}", param.name, source),
        "Bool" => format!("{} = {}", param.name, source),
        _ => format!("{} = std::move({})", param.name, source),
    }
}

fn ffi_option_value_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" | "Bool" => "i32",
        "Float" => "f64",
        _ => "*const c_char",
    }
}

fn c_option_value_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "int32_t",
        "Bool" => "int32_t",
        "Float" => "double",
        _ => "const char*",
    }
}

fn ffi_option_insert_expr(param: &GeneratedParam) -> String {
    match param.param_type.as_str() {
        "Int" => format!(
            "        insert_int_arg(args, {:?}, options.{});",
            param.name, param.name
        ),
        "Float" => {
            format!(
                "        insert_float_arg(args, {:?}, options.{});",
                param.name, param.name
            )
        }
        "Bool" => format!(
            "        insert_bool_arg(args, {:?}, options.{})?;",
            param.name, param.name
        ),
        _ => format!(
            "    insert_optional_str_arg(args, {:?}, options.{})?;",
            param.name, param.name
        ),
    }
}

fn ffi_option_has_flag(param: &GeneratedParam) -> bool {
    matches!(param.param_type.as_str(), "Int" | "Float" | "Bool")
}

fn render_ffi_endpoint_request_options(params: &[GeneratedParam]) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from endpoint_surface.toml\n\n",
    );
    out.push_str(
        "/// Optional builder parameters for registry-driven endpoint requests over FFI.\n",
    );
    out.push_str("///\n");
    out.push_str("/// Fields use simple C-friendly sentinels:\n");
    out.push_str("/// - integer/boolean/float fields: check the companion `has_*` flag\n");
    out.push_str("/// - string pointers: null means unset\n");
    out.push_str("#[repr(C)]\n");
    out.push_str("pub struct TdxEndpointRequestOptions {\n");
    for param in params {
        writeln!(
            out,
            "    pub {}: {},",
            param.name,
            ffi_option_value_type(param)
        )
        .unwrap();
        if ffi_option_has_flag(param) {
            writeln!(out, "    pub has_{}: i32,", param.name).unwrap();
        }
    }
    out.push_str("}\n\n");
    out.push_str("fn apply_endpoint_request_options(\n");
    out.push_str("    args: &mut thetadatadx::EndpointArgs,\n");
    out.push_str("    options: *const TdxEndpointRequestOptions,\n");
    out.push_str(") -> Result<(), String> {\n");
    out.push_str("    if options.is_null() {\n        return Ok(());\n    }\n\n");
    out.push_str("    let options = unsafe { &*options };\n");
    for param in params {
        if !ffi_option_has_flag(param) {
            out.push_str(&ffi_option_insert_expr(param));
            out.push('\n');
        } else {
            writeln!(out, "    if options.has_{} != 0 {{", param.name).unwrap();
            out.push_str(&ffi_option_insert_expr(param));
            out.push('\n');
            out.push_str("    }\n");
        }
    }
    out.push_str("    Ok(())\n");
    out.push_str("}\n");
    out
}

fn render_c_endpoint_request_options(params: &[GeneratedParam]) -> String {
    let mut out = String::new();
    out.push_str(
        "/* @generated DO NOT EDIT — regenerated by build.rs from endpoint_surface.toml */\n",
    );
    out.push_str("typedef struct {\n");
    for param in params {
        writeln!(out, "    {} {};", c_option_value_type(param), param.name).unwrap();
        if ffi_option_has_flag(param) {
            writeln!(out, "    int32_t has_{};", param.name).unwrap();
        }
    }
    out.push_str("} TdxEndpointRequestOptions;\n");
    out
}

fn sdk_method_arg_name(param: &GeneratedParam) -> String {
    if param.param_type == "Symbols" {
        "symbols".into()
    } else {
        param.name.clone()
    }
}

fn go_method_arg_decl(param: &GeneratedParam) -> String {
    let name = to_camel_case(&sdk_method_arg_name(param));
    if param.param_type == "Symbols" {
        format!("{name} []string")
    } else {
        format!("{name} string")
    }
}

fn python_method_arg_decl(param: &GeneratedParam) -> String {
    let name = sdk_method_arg_name(param);
    if param.param_type == "Symbols" {
        format!("{name}: Vec<String>")
    } else {
        format!("{name}: &str")
    }
}

fn cpp_method_arg_decl(param: &GeneratedParam) -> String {
    let name = sdk_method_arg_name(param);
    if param.param_type == "Symbols" {
        format!("const std::vector<std::string>& {name}")
    } else {
        format!("const std::string& {name}")
    }
}

fn go_c_var_name(param: &GeneratedParam) -> String {
    format!("c{}", to_go_exported_name(&sdk_method_arg_name(param)))
}

fn render_go_options(params: &[GeneratedParam]) -> String {
    let mut out = String::new();
    out.push_str("// Code generated by build.rs from endpoint_surface.toml; DO NOT EDIT.\n\n");
    out.push_str("package thetadatadx\n\n");
    out.push_str("/*\n#include \"ffi_bridge.h\"\n*/\nimport \"C\"\n\n");
    out.push_str("import \"unsafe\"\n\n");
    out.push_str("// EndpointRequestOptions contains the shared optional request fields projected from endpoint_surface.toml.\n");
    out.push_str("type EndpointRequestOptions struct {\n");
    for param in params {
        let field_name = to_go_exported_name(&param.name);
        let field_type = match param.param_type.as_str() {
            "Int" => "*int32",
            "Float" => "*float64",
            "Bool" => "*bool",
            _ => "*string",
        };
        writeln!(out, "\t// {}", param.description).unwrap();
        writeln!(out, "\t{} {}", field_name, field_type).unwrap();
    }
    out.push_str("}\n\n");

    out.push_str("// EndpointOption applies one optional request field to an endpoint request.\n");
    out.push_str("type EndpointOption func(*EndpointRequestOptions)\n\n");
    out.push_str(
        "func collectEndpointRequestOptions(opts []EndpointOption) *EndpointRequestOptions {\n",
    );
    out.push_str("\tif len(opts) == 0 {\n\t\treturn nil\n\t}\n");
    out.push_str("\toptions := &EndpointRequestOptions{}\n");
    out.push_str(
        "\tfor _, opt := range opts {\n\t\tif opt != nil {\n\t\t\topt(options)\n\t\t}\n\t}\n",
    );
    out.push_str("\treturn options\n}\n\n");

    for param in params {
        let helper_name = format!("With{}", to_go_exported_name(&param.name));
        let field_name = to_go_exported_name(&param.name);
        let arg_type = match param.param_type.as_str() {
            "Int" => "int32",
            "Float" => "float64",
            "Bool" => "bool",
            _ => "string",
        };
        writeln!(out, "// {} sets {}.", helper_name, param.name).unwrap();
        writeln!(
            out,
            "func {}(value {}) EndpointOption {{",
            helper_name, arg_type
        )
        .unwrap();
        out.push_str("\treturn func(options *EndpointRequestOptions) {\n");
        out.push_str("\t\tvalueCopy := value\n");
        writeln!(out, "\t\toptions.{} = &valueCopy", field_name).unwrap();
        out.push_str("\t}\n");
        out.push_str("}\n\n");
    }

    out.push_str("func endpointRequestOptionsToC(opts *EndpointRequestOptions) (*C.TdxEndpointRequestOptions, func()) {\n");
    out.push_str("\tif opts == nil {\n\t\treturn nil, func() {}\n\t}\n\n");
    out.push_str("\tcOpts := &C.TdxEndpointRequestOptions{}\n\n");
    out.push_str("\tvar allocations []unsafe.Pointer\n");
    out.push_str("\tfree := func() {\n");
    out.push_str("\t\tfor _, allocation := range allocations {\n");
    out.push_str("\t\t\tC.free(allocation)\n");
    out.push_str("\t\t}\n");
    out.push_str("\t}\n\n");
    for param in params {
        let field_name = to_go_exported_name(&param.name);
        match param.param_type.as_str() {
            "Int" => {
                writeln!(out, "\tif opts.{field_name} != nil {{").unwrap();
                writeln!(
                    out,
                    "\t\tcOpts.{0} = C.int32_t(*opts.{1})",
                    param.name, field_name
                )
                .unwrap();
                writeln!(out, "\t\tcOpts.has_{} = 1", param.name).unwrap();
                out.push_str("\t}\n");
            }
            "Float" => {
                writeln!(out, "\tif opts.{field_name} != nil {{").unwrap();
                writeln!(
                    out,
                    "\t\tcOpts.{0} = C.double(*opts.{1})",
                    param.name, field_name
                )
                .unwrap();
                writeln!(out, "\t\tcOpts.has_{} = 1", param.name).unwrap();
                out.push_str("\t}\n");
            }
            "Bool" => {
                writeln!(out, "\tif opts.{field_name} != nil {{").unwrap();
                writeln!(out, "\t\tif *opts.{field_name} {{").unwrap();
                writeln!(out, "\t\t\tcOpts.{0} = 1", param.name).unwrap();
                out.push_str("\t\t} else {\n");
                writeln!(out, "\t\t\tcOpts.{0} = 0", param.name).unwrap();
                out.push_str("\t\t}\n");
                writeln!(out, "\t\tcOpts.has_{} = 1", param.name).unwrap();
                out.push_str("\t}\n");
            }
            _ => {
                writeln!(out, "\tif opts.{field_name} != nil {{").unwrap();
                writeln!(out, "\t\tvalue := C.CString(*opts.{field_name})").unwrap();
                writeln!(out, "\t\tcOpts.{0} = value", param.name).unwrap();
                out.push_str("\t\tallocations = append(allocations, unsafe.Pointer(value))\n");
                out.push_str("\t}\n");
            }
        }
    }
    out.push_str("\n\treturn cOpts, free\n");
    out.push_str("}\n");

    out
}

fn render_go_endpoint_with_options_decls(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str("/* @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml */\n");
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| has_builder_params(endpoint) && !is_streaming_endpoint(endpoint))
    {
        let ffi_name = format!("tdx_{}_with_options", endpoint.name);
        let return_type = ffi_header_return_type(&endpoint.return_type);
        let params = method_params(endpoint);
        write!(
            out,
            "extern {} {}(const TdxClient* client",
            return_type, ffi_name
        )
        .unwrap();
        for param in &params {
            if param.param_type == "Symbols" {
                write!(out, ", const char* const* symbols, size_t symbols_len").unwrap();
            } else {
                write!(out, ", const char* {}", param.name).unwrap();
            }
        }
        out.push_str(", const TdxEndpointRequestOptions* options);\n");
    }
    out
}

fn render_c_endpoint_with_options_decls(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str("/* @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml */\n");
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| has_builder_params(endpoint) && !is_streaming_endpoint(endpoint))
    {
        let ffi_name = format!("tdx_{}_with_options", endpoint.name);
        let return_type = ffi_array_type(&endpoint.return_type);
        let params = method_params(endpoint);
        write!(
            out,
            "extern {} {}(const TdxClient* client",
            return_type, ffi_name
        )
        .unwrap();
        for param in &params {
            if param.param_type == "Symbols" {
                write!(out, ", const char* const* symbols, size_t symbols_len").unwrap();
            } else {
                write!(out, ", const char* {}", param.name).unwrap();
            }
        }
        out.push_str(", const TdxEndpointRequestOptions* options);\n");
    }
    out
}

fn render_go_historical(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str("// Code generated by build.rs from endpoint_surface.toml; DO NOT EDIT.\n\n");
    out.push_str("package thetadatadx\n\n");
    out.push_str("/*\n#include \"ffi_bridge.h\"\n*/\nimport \"C\"\n\n");
    out.push_str("import \"unsafe\"\n\n");

    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        out.push_str(&render_go_endpoint_method(endpoint));
        out.push('\n');
    }

    out
}

fn render_go_endpoint_method(endpoint: &GeneratedEndpoint) -> String {
    let method_name = to_go_exported_name(&endpoint.name);
    let method_params = method_params(endpoint);
    let builder_params = builder_params(endpoint);
    let mut out = String::new();

    let mut signature_parts = method_params
        .iter()
        .map(|param| go_method_arg_decl(param))
        .collect::<Vec<_>>();
    if !builder_params.is_empty() {
        signature_parts.push("opts ...EndpointOption".into());
    }
    writeln!(
        out,
        "func (c *Client) {}({}) ({}, error) {{",
        method_name,
        signature_parts.join(", "),
        go_result_type(&endpoint.return_type)
    )
    .unwrap();

    let has_symbols = method_params
        .iter()
        .any(|param| param.param_type == "Symbols");
    if has_symbols {
        out.push_str("\tcSymbols, cSymbolsLen := symbolsToCArray(symbols)\n");
        out.push_str("\tdefer freeSymbolArray(cSymbols, cSymbolsLen)\n");
    }

    for param in method_params
        .iter()
        .filter(|param| param.param_type != "Symbols")
    {
        let var_name = go_c_var_name(param);
        let arg_name = to_camel_case(&sdk_method_arg_name(param));
        writeln!(out, "\t{} := C.CString({})", var_name, arg_name).unwrap();
        writeln!(out, "\tdefer C.free(unsafe.Pointer({}))", var_name).unwrap();
    }

    if !builder_params.is_empty() {
        out.push_str(
            "\tcOpts, freeOpts := endpointRequestOptionsToC(collectEndpointRequestOptions(opts))\n",
        );
        out.push_str("\tdefer freeOpts()\n");
    }

    write!(
        out,
        "\tarr := C.{}(c.handle",
        if builder_params.is_empty() {
            format!("tdx_{}", endpoint.name)
        } else {
            format!("tdx_{}_with_options", endpoint.name)
        }
    )
    .unwrap();
    for param in &method_params {
        if param.param_type == "Symbols" {
            out.push_str(", cSymbols, cSymbolsLen");
        } else {
            write!(out, ", {}", go_c_var_name(param)).unwrap();
        }
    }
    if !builder_params.is_empty() {
        out.push_str(", cOpts");
    }
    out.push_str(")\n");

    if endpoint.return_type == "StringList" {
        out.push_str("\treturn stringArrayToGo(arr)\n");
        out.push_str("}\n");
        return out;
    }

    writeln!(
        out,
        "\tresult := {}(arr)",
        go_converter_name(&endpoint.return_type)
    )
    .unwrap();
    writeln!(out, "\t{}(arr)", ffi_free_fn(&endpoint.return_type)).unwrap();
    out.push_str("\treturn result, nil\n");
    out.push_str("}\n");
    out
}

fn render_python_historical_methods(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml\n\n",
    );
    out.push_str("#[pymethods]\n");
    out.push_str("impl ThetaDataDx {\n");
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        out.push_str(&render_python_endpoint_method(endpoint));
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

fn render_python_endpoint_method(endpoint: &GeneratedEndpoint) -> String {
    let method_params = method_params(endpoint);
    let builder_params = builder_params(endpoint);
    let mut out = String::new();

    writeln!(out, "    /// {}", endpoint.description).unwrap();
    if !builder_params.is_empty() {
        let mut signature_parts = method_params
            .iter()
            .map(|param| sdk_method_arg_name(param))
            .collect::<Vec<_>>();
        signature_parts.push("*".into());
        signature_parts.extend(
            builder_params
                .iter()
                .map(|param| format!("{}=None", param.name)),
        );
        writeln!(
            out,
            "    #[pyo3(signature = ({}))]",
            signature_parts.join(", ")
        )
        .unwrap();
    }

    writeln!(out, "    fn {}(", endpoint.name).unwrap();
    out.push_str("        &self,\n");
    out.push_str("        py: Python<'_>,\n");
    for param in &method_params {
        writeln!(out, "        {},", python_method_arg_decl(param)).unwrap();
    }
    for param in &builder_params {
        writeln!(
            out,
            "        {}: {},",
            param.name,
            python_optional_type(param)
        )
        .unwrap();
    }
    out.push_str("    ) -> PyResult<");
    if endpoint.return_type == "StringList" {
        out.push_str("Vec<String>> {\n");
    } else {
        out.push_str("Py<PyAny>> {\n");
    }

    let has_symbols = method_params
        .iter()
        .any(|param| param.param_type == "Symbols");
    if has_symbols {
        out.push_str(
            "        let refs: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();\n",
        );
    }

    out.push_str("        ");
    if endpoint.return_type == "StringList" {
        out.push_str("py.detach(|| {\n");
    } else {
        out.push_str("let ticks = py.detach(|| {\n");
    }
    if builder_params.is_empty() {
        out.push_str("            runtime()\n");
        out.push_str("                .block_on(async {\n");
        write!(out, "                    self.tdx.{}(", endpoint.name).unwrap();
        out.push_str(
            &method_params
                .iter()
                .map(|param| {
                    if param.param_type == "Symbols" {
                        "&refs".into()
                    } else {
                        sdk_method_arg_name(param)
                    }
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push_str(").await\n");
        out.push_str("                })\n");
        out.push_str("                .map_err(to_py_err)\n");
    } else {
        write!(
            out,
            "            let mut request = self.tdx.{}(",
            endpoint.name
        )
        .unwrap();
        out.push_str(
            &method_params
                .iter()
                .map(|param| {
                    if param.param_type == "Symbols" {
                        "&refs".into()
                    } else {
                        sdk_method_arg_name(param)
                    }
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push_str(");\n");
        for param in &builder_params {
            writeln!(out, "            if let Some(value) = {} {{", param.name).unwrap();
            writeln!(
                out,
                "                request = request.{}(value);",
                param.name
            )
            .unwrap();
            out.push_str("            }\n");
        }
        if endpoint.kind == "stream" {
            out.push_str("            let mut collected = Vec::new();\n");
            out.push_str("            runtime()\n");
            out.push_str(
                "                .block_on(request.stream(|chunk| collected.extend_from_slice(chunk)))\n",
            );
            out.push_str("                .map_err(to_py_err)?;\n");
            out.push_str("            pyo3::PyResult::Ok(collected)\n");
        } else {
            out.push_str("            runtime()\n");
            out.push_str("                .block_on(async { request.await })\n");
            out.push_str("                .map_err(to_py_err)\n");
        }
    }
    if endpoint.return_type == "StringList" {
        out.push_str("        })\n");
        out.push_str("    }\n");
        return out;
    }

    out.push_str("        })?;\n");
    writeln!(
        out,
        "        Ok({}(py, &ticks))",
        python_columnar_converter(&endpoint.return_type)
    )
    .unwrap();
    out.push_str("    }\n");
    out
}

/// Map a `param_type` from the endpoint surface to a dummy value for Python validation.
fn validation_symbol(endpoint: &GeneratedEndpoint) -> &'static str {
    match endpoint.category.as_str() {
        "stock" => "AAPL",
        "option" => "SPY",
        "index" => "SPX",
        "rate" => "SOFR",
        other => panic!("unsupported validation endpoint category: {other}"),
    }
}

fn cli_command_name(endpoint: &GeneratedEndpoint) -> String {
    match endpoint.category.as_str() {
        "stock" | "option" | "index" | "calendar" => endpoint
            .name
            .strip_prefix(&format!("{}_", endpoint.category))
            .expect("endpoint name should match category prefix")
            .into(),
        "rate" => endpoint
            .name
            .strip_prefix("interest_rate_")
            .expect("rate endpoint should use interest_rate_ prefix")
            .into(),
        other => panic!("unsupported CLI endpoint category: {other}"),
    }
}

fn cli_command_tokens_for_mode(endpoint: &GeneratedEndpoint, mode: &TestMode) -> Vec<String> {
    let mut tokens = vec![
        match endpoint.category.as_str() {
            "rate" => "rate".into(),
            other => other.into(),
        },
        cli_command_name(endpoint),
    ];
    tokens.extend(mode.args.iter().cloned());
    tokens
}

// ───────────────────────── Multi-mode parameter matrix ──────────────────────
//
// `TestMode` captures one (parameter-shape × tier) cell that the live
// validator should exercise. Modes are derived per endpoint by
// [`test_modes_for`] from the endpoint's wire shape — list endpoints get one
// mode, ContractSpec endpoints get the full wildcard cross-product, and so
// on. Each mode carries language-agnostic string args so per-language
// renderers (CLI / Python / Go / C++) can format them appropriately.

/// One parameter-mode test cell to run against a live endpoint.
#[derive(Debug, Clone)]
struct TestMode {
    /// Mode identifier (`concrete`, `bulk_chain`, `iso_date`, ...). Used in
    /// validator output so failures point at a specific cell.
    name: &'static str,
    /// Method-call positional arguments, in declaration order. Each entry is
    /// the language-agnostic string value (e.g. `"SPY"`, `"20260417"`,
    /// `"*"`). `Symbols`-typed params are still rendered as a single string
    /// here — per-language renderers wrap them in the target list literal.
    args: Vec<String>,
    /// Highest subscription tier this mode requires (`"free"`, `"value"`,
    /// `"standard"`, `"professional"`). The validator skips the cell with a
    /// clear `SKIP: tier<X>` line if the account tier is below.
    min_tier: &'static str,
    /// Outcome the validator should expect.
    ///   - `non_empty`: a normal successful call (rows or "no data" both PASS)
    ///   - `empty_ok`: a successful call that may legitimately return zero rows
    ///   - `error_permission`: tier/permission errors are PASS, real errors FAIL
    expect: &'static str,
}

/// Minimum subscription tier each endpoint requires.
///
/// Derived at generator-run-time from the pinned upstream OpenAPI snapshot at
/// `scripts/upstream_openapi.yaml` (see [`super::upstream_openapi`]), keyed
/// on the endpoint's `operationId`. Upstream is the sole source of truth for
/// `x-min-subscription`, so docs-site `<TierBadge>` and this function agree
/// as long as the snapshot is fresh.
///
/// Four kinds of endpoints don't have an upstream entry and fall back to a
/// tiny override table ([`sdk_only_min_tier`]): streaming RPCs (FPSS, not
/// MDDS), SDK-private endpoints like `interest_rate_history_eod`, and
/// SDK-only synthetic clones like `stock_history_ohlc_range`.
fn endpoint_min_tier(name: &str) -> &'static str {
    if let Some(tier) = sdk_only_min_tier(name) {
        return tier;
    }
    let spec = super::upstream_openapi::UpstreamOpenApi::load();
    let endpoint = spec.endpoint(name).unwrap_or_else(|| {
        panic!(
            "endpoint '{name}' is missing from the upstream OpenAPI snapshot \
             at scripts/upstream_openapi.yaml; if this is a new endpoint, add \
             it as an SDK-only override in `sdk_only_min_tier`, or refresh the \
             snapshot with `python3 scripts/check_tier_badges.py --refresh-snapshot`."
        )
    });
    match endpoint.min_subscription.as_str() {
        "free" => "free",
        "value" => "value",
        "standard" => "standard",
        "professional" => "professional",
        other => panic!(
            "endpoint '{name}': upstream min-subscription '{other}' is not a known tier. \
             Expected one of free/value/standard/professional."
        ),
    }
}

/// Minimum-tier override for endpoints that aren't in the upstream OpenAPI spec.
///
/// Returns `None` for every endpoint that upstream documents — those flow
/// through [`endpoint_min_tier`]'s snapshot lookup.
fn sdk_only_min_tier(name: &str) -> Option<&'static str> {
    Some(match name {
        // Streaming endpoints (FPSS, covered by scripts/fpss_smoke.py, not the
        // live matrix validator). The value here is still used by
        // `test_modes_for` for display-only `min_tier` on test cells, but the
        // streaming surface is excluded from the matrix anyway.
        "stock_history_trade_stream"
        | "stock_history_quote_stream"
        | "option_history_trade_stream"
        | "option_history_quote_stream" => "standard",
        // Synthetic clone sharing a wire RPC with `stock_history_ohlc`.
        "stock_history_ohlc_range" => "value",
        // SDK-only endpoint not documented upstream (FRED-backed, thetadatadx-local).
        "interest_rate_history_eod" => "free",
        _ => return None,
    })
}

/// Render the language-agnostic value for a method-call parameter at a
/// concrete fixture (no wildcards, compact dates).
fn concrete_value(endpoint: &GeneratedEndpoint, param: &GeneratedParam) -> String {
    if param.name == "end_date" {
        return "20250307".into();
    }
    match param.param_type.as_str() {
        "Symbol" | "Symbols" => validation_symbol(endpoint).into(),
        "Date" => "20250303".into(),
        "Expiration" => "20250321".into(),
        "Strike" => "570".into(),
        "Right" => "C".into(),
        "Interval" => "60000".into(),
        "RequestType" => "TRADE".into(),
        "Year" => "2025".into(),
        "Str" => "12:00:00.000".into(),
        other => panic!("concrete_value: unsupported param type {other}"),
    }
}

/// Build the args vector for a concrete (no-wildcard) call.
fn concrete_args(endpoint: &GeneratedEndpoint) -> Vec<String> {
    method_params(endpoint)
        .iter()
        .map(|param| concrete_value(endpoint, param))
        .collect()
}

/// Build args for a mode that overrides specific parameter names with given
/// values — everything else falls back to [`concrete_value`].
fn args_with_overrides(
    endpoint: &GeneratedEndpoint,
    overrides: &[(&'static str, &str)],
) -> Vec<String> {
    method_params(endpoint)
        .iter()
        .map(|param| {
            overrides
                .iter()
                .find_map(|(name, value)| (*name == param.name).then(|| (*value).to_string()))
                .unwrap_or_else(|| concrete_value(endpoint, param))
        })
        .collect()
}

/// Whether the endpoint's method-call params include the full ContractSpec
/// quartet (symbol, expiration, strike, right). Drives wildcard mode
/// generation for option snapshot / history / at-time endpoints.
fn has_full_contract_spec(endpoint: &GeneratedEndpoint) -> bool {
    let names: HashSet<&str> = method_params(endpoint)
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    names.contains("symbol")
        && names.contains("expiration")
        && names.contains("strike")
        && names.contains("right")
}

/// Whether an option endpoint accepts `expiration=*` at the v3 server.
///
/// Derived from the pinned upstream snapshot
/// (`scripts/upstream_openapi.yaml`): upstream binds endpoints that reject
/// wildcards to its `expiration_no_star` component parameter (they return
/// `InvalidArgument -- Cannot specify '*' for the date` if we send `*`),
/// and wildcard-accepting endpoints to `expiration`. See
/// [`super::upstream_openapi::UpstreamEndpoint::supports_expiration_wildcard`].
///
/// Endpoints absent from upstream (streaming, SDK-only clones) fall back to
/// `true` — they don't participate in the wildcard matrix anyway (streaming
/// is skipped upstream of this call, and the SDK-only endpoints don't take
/// an expiration parameter).
fn endpoint_supports_expiration_wildcard(name: &str) -> bool {
    let spec = super::upstream_openapi::UpstreamOpenApi::load();
    spec.endpoint(name)
        .map(|endpoint| endpoint.supports_expiration_wildcard)
        .unwrap_or(true)
}

/// Compute the comprehensive mode set for a given endpoint.
///
/// The taxonomy:
///   * **List** endpoints (`*_list_*`): one `basic` mode. Server rejects
///     `*` for `expiration` here, so we don't emit a wildcard variant.
///   * **Stock / index snapshot or history** (no ContractSpec): one
///     `concrete` mode plus an `iso_date` mode where dates are involved.
///   * **Option ContractSpec** endpoints: the full cross-product —
///     `concrete`, `concrete_iso`, `all_strikes_one_exp`,
///     `all_exps_one_strike`, `bulk_chain`, `legacy_zero_wildcard`.
///   * **Calendar / rate**: one mode each.
///
/// Stream endpoints are covered by `scripts/fpss_smoke.py` /
/// `scripts/fpss_soak.py` and intentionally skipped here.
fn test_modes_for(endpoint: &GeneratedEndpoint) -> Vec<TestMode> {
    if is_streaming_endpoint(endpoint) {
        return Vec::new();
    }
    let endpoint_tier = endpoint_min_tier(&endpoint.name);

    // ── List endpoints: one mode, no wildcard expiration (server rejects). ──
    if is_simple_list_endpoint(endpoint) {
        return vec![TestMode {
            name: "basic",
            args: concrete_args(endpoint),
            min_tier: endpoint_tier,
            expect: "non_empty",
        }];
    }

    // ── Calendar / rate: one mode. ──────────────────────────────────────────
    if matches!(endpoint.category.as_str(), "calendar" | "rate") {
        return vec![TestMode {
            name: "basic",
            args: concrete_args(endpoint),
            min_tier: endpoint_tier,
            expect: "non_empty",
        }];
    }

    // ── Option ContractSpec: full wildcard cross-product, except where the
    // v3 server explicitly disallows `expiration=*` on an endpoint (it binds
    // that endpoint to the `expiration_no_star` parameter in upstream's
    // openapiv3.yaml, and returns `InvalidArgument -- Cannot specify '*' for
    // the date` if we pass it). Those endpoints get only the concrete +
    // ISO-dashed fixtures plus the `all_strikes_one_exp` mode, which uses a
    // concrete expiration.
    if has_full_contract_spec(endpoint) {
        let mut modes = vec![
            TestMode {
                name: "concrete",
                args: concrete_args(endpoint),
                min_tier: endpoint_tier,
                expect: "non_empty",
            },
            TestMode {
                name: "concrete_iso",
                args: args_with_overrides(endpoint, &[("expiration", "2025-03-21")]),
                min_tier: endpoint_tier,
                expect: "non_empty",
            },
            TestMode {
                name: "all_strikes_one_exp",
                args: args_with_overrides(endpoint, &[("strike", "*"), ("right", "both")]),
                min_tier: endpoint_tier,
                expect: "non_empty",
            },
        ];
        if endpoint_supports_expiration_wildcard(&endpoint.name) {
            modes.extend([
                TestMode {
                    name: "all_exps_one_strike",
                    args: args_with_overrides(endpoint, &[("expiration", "*"), ("right", "both")]),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                },
                TestMode {
                    name: "bulk_chain",
                    args: args_with_overrides(
                        endpoint,
                        &[("expiration", "*"), ("strike", "*"), ("right", "both")],
                    ),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                },
                TestMode {
                    name: "legacy_zero_wildcard",
                    args: args_with_overrides(
                        endpoint,
                        &[("expiration", "0"), ("strike", "0"), ("right", "both")],
                    ),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                },
            ]);
        }
        modes.dedup_by(|a, b| a.args == b.args && a.name == b.name);
        return modes;
    }

    // ── Stock / index / non-ContractSpec endpoints. ─────────────────────────
    //
    // We deliberately do NOT emit an `iso_date` mode for stock/index
    // endpoints with `start_date`/`end_date`. Those parameters are typed as
    // `Date` in the SDK, and `validate::validate_date` is strict
    // `YYYYMMDD` only — ISO-dashed acceptance is scoped to `Expiration`
    // (see PR #284). Adding an `iso_date` cell here would test behavior the
    // SDK contract intentionally does not support, so it would always fail.
    vec![TestMode {
        name: "concrete",
        args: concrete_args(endpoint),
        min_tier: endpoint_tier,
        expect: "non_empty",
    }]
}

/// Render a single arg string as a Python literal expression, taking the
/// param's wire type into account so `Symbols` becomes a list.
fn python_arg_literal(param: &GeneratedParam, value: &str) -> String {
    match param.param_type.as_str() {
        "Symbols" => format!("[\"{value}\"]"),
        _ => format!("\"{value}\""),
    }
}

/// Render a single arg string as a Go literal expression.
fn go_arg_literal(param: &GeneratedParam, value: &str) -> String {
    match param.param_type.as_str() {
        "Symbols" => format!("[]string{{\"{value}\"}}"),
        _ => format!("\"{value}\""),
    }
}

/// Render a single arg string as a C++ literal expression.
fn cpp_arg_literal(param: &GeneratedParam, value: &str) -> String {
    match param.param_type.as_str() {
        "Symbols" => format!("std::vector<std::string>{{\"{value}\"}}"),
        _ => format!("\"{value}\""),
    }
}

/// Generate the CLI validator (one row per (endpoint, mode) pair).
fn render_cli_validate(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str("#!/usr/bin/env python3\n");
    out.push_str(
        "# @generated DO NOT EDIT \u{2014} regenerated by generate_sdk_surfaces from endpoint_surface.toml\n",
    );
    out.push_str("\"\"\"Live parameter-mode matrix validator for the CLI surface.\n\n");
    out.push_str("Each row is one (endpoint, mode) cell. Modes cover concrete fixtures plus\n");
    out.push_str("wildcard / ISO-date / legacy-zero variants for option ContractSpec\n");
    out.push_str("endpoints. Every cell is attempted against production; the server is the\n");
    out.push_str("ground truth for what the account can access. Cells whose documented\n");
    out.push_str("`min_tier` exceeds the live account tier come back as a permission error\n");
    out.push_str("and are classified `SKIP: tier-permission`. Real configuration bugs\n");
    out.push_str("(invalid arguments, wire-format errors) surface as `FAIL`. See issue\n");
    out.push_str("#287.\n\n");
    out.push_str("Each cell is bounded by a 60-second subprocess timeout. A stuck cell\n");
    out.push_str("classifies as FAIL with `timeout after 60s` rather than stalling CI\n");
    out.push_str("indefinitely -- a human should investigate whether the fixture is too\n");
    out.push_str("broad or the server is misbehaving. See issue #290.\n\"\"\"\n");
    out.push_str("from __future__ import annotations\n\n");
    out.push_str("import os\n");
    out.push_str("import pathlib\n");
    out.push_str("import subprocess\n");
    out.push_str("import sys\n\n");
    out.push_str("PER_CELL_TIMEOUT_SECS = 60\n\n");
    out.push_str("REPO = pathlib.Path(__file__).resolve().parents[1]\n");
    out.push_str("TDX = REPO / \"target\" / \"release\" / (\"tdx.exe\" if os.name == \"nt\" else \"tdx\")\n\n");
    out.push_str("# (endpoint, mode_name, declared_min_tier, [argv...])\n");
    out.push_str("# `declared_min_tier` is informational only (printed on tier-permission\n");
    out.push_str("# skips so you can see which modes the server refused).\n");
    out.push_str("CELLS = [\n");
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        for mode in test_modes_for(endpoint) {
            let tokens = cli_command_tokens_for_mode(endpoint, &mode)
                .into_iter()
                .map(|token| format!("{token:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                out,
                "    ({:?}, {:?}, {:?}, [{}]),",
                endpoint.name, mode.name, mode.min_tier, tokens
            )
            .unwrap();
        }
    }
    out.push_str("]\n\n");
    out.push_str("if not TDX.exists():\n");
    out.push_str("    raise SystemExit(f\"missing CLI binary: {TDX}\")\n\n");
    out.push_str("creds = sys.argv[1]\n");
    out.push_str("pass_count = skip_count = fail_count = 0\n");
    out.push_str("for endpoint, mode, min_tier, argv in CELLS:\n");
    out.push_str("    label = f\"{endpoint}::{mode}\"\n");
    out.push_str("    try:\n");
    out.push_str("        proc = subprocess.run(\n");
    out.push_str("            [str(TDX), \"--creds\", creds, *argv, \"--format\", \"json\"],\n");
    out.push_str("            cwd=REPO,\n");
    out.push_str("            stdout=subprocess.PIPE,\n");
    out.push_str("            stderr=subprocess.STDOUT,\n");
    out.push_str("            check=False,\n");
    out.push_str("            timeout=PER_CELL_TIMEOUT_SECS,\n");
    out.push_str("        )\n");
    out.push_str("    except subprocess.TimeoutExpired:\n");
    out.push_str(
        "        print(f\"  {label:60s} FAIL  timeout after {PER_CELL_TIMEOUT_SECS}s\")\n",
    );
    out.push_str("        fail_count += 1\n");
    out.push_str("        continue\n");
    out.push_str("    output = (proc.stdout or b\"\").decode(\"utf-8\", errors=\"replace\")\n");
    out.push_str("    msg = output.lower()\n");
    out.push_str("    if proc.returncode == 0:\n");
    out.push_str("        print(f\"  {label:60s} PASS\")\n");
    out.push_str("        pass_count += 1\n");
    out.push_str("    elif \"permission\" in msg or \"subscription\" in msg:\n");
    out.push_str(
        "        print(f\"  {label:60s} SKIP: tier-permission (declared min_tier={min_tier})\")\n",
    );
    out.push_str("        skip_count += 1\n");
    out.push_str("    elif \"no data found\" in msg:\n");
    out.push_str("        print(f\"  {label:60s} PASS  (no data)\")\n");
    out.push_str("        pass_count += 1\n");
    out.push_str("    else:\n");
    out.push_str("        print(f\"  {label:60s} FAIL  {output.strip()}\")\n");
    out.push_str("        fail_count += 1\n");
    out.push('\n');
    out.push_str("print(f\"\\nCLI: {pass_count} PASS, {skip_count} SKIP, {fail_count} FAIL\")\n");
    out.push_str("print(f\"COUNTS:{pass_count}:{skip_count}:{fail_count}\")\n");
    out.push_str("sys.exit(1 if fail_count > 0 else 0)\n");
    out
}

/// Generate the Python SDK validator (one row per (endpoint, mode) pair).
fn render_python_validate(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str("#!/usr/bin/env python3\n");
    out.push_str(
        "# @generated DO NOT EDIT \u{2014} regenerated by generate_sdk_surfaces from endpoint_surface.toml\n",
    );
    out.push_str("\"\"\"Live parameter-mode matrix validator for the Python SDK.\n\n");
    out.push_str("Each row is one (endpoint, mode) cell. Modes cover concrete fixtures plus\n");
    out.push_str("wildcard / ISO-date / legacy-zero variants for option ContractSpec\n");
    out.push_str("endpoints. Every cell is attempted against production; the server is the\n");
    out.push_str("ground truth for what the account can access. Cells whose documented\n");
    out.push_str("`min_tier` exceeds the live account tier come back as a permission error\n");
    out.push_str("and are classified `SKIP: tier-permission`. Real configuration bugs\n");
    out.push_str("(invalid arguments, wire-format errors) surface as `FAIL`. See issue\n");
    out.push_str("#287.\n\n");
    out.push_str("Each cell is bounded by a 60-second per-call timeout enforced by a\n");
    out.push_str("daemon `threading.Thread` + `queue.Queue.get(timeout=...)`. A stuck\n");
    out.push_str("cell classifies as FAIL with `timeout after 60s`. On completion we\n");
    out.push_str("call `os._exit()` to bypass the interpreter's non-daemon-thread join\n");
    out.push_str("and atexit machinery -- otherwise any truly-wedged PyO3 call would\n");
    out.push_str("block process teardown. Daemon threads are abruptly killed at\n");
    out.push_str("`_exit`, which is exactly what we want for a single-shot validator.\n");
    out.push_str("See issue #290.\n\"\"\"\n");
    out.push_str("import os\n");
    out.push_str("import queue\n");
    out.push_str("import sys\n");
    out.push_str("import threading\n\n");
    out.push_str("from thetadatadx import Credentials, Config, ThetaDataDx\n\n");
    out.push_str("PER_CELL_TIMEOUT_SECS = 60\n\n");
    out.push_str(
        "client = ThetaDataDx(Credentials.from_file(sys.argv[1]), Config.production())\n\n",
    );
    out.push_str("# (endpoint, mode_name, declared_min_tier, callable)\n");
    out.push_str("# `declared_min_tier` is informational only (printed on tier-permission\n");
    out.push_str("# skips so you can see which modes the server refused).\n");
    out.push_str("CELLS = [\n");
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        let mp = method_params(endpoint);
        for mode in test_modes_for(endpoint) {
            let args = mp
                .iter()
                .zip(mode.args.iter())
                .map(|(param, value)| python_arg_literal(param, value))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                out,
                "    ({:?}, {:?}, {:?}, lambda: client.{}({})),",
                endpoint.name, mode.name, mode.min_tier, endpoint.name, args
            )
            .unwrap();
        }
    }
    out.push_str("]\n\n");
    out.push_str("pass_count = skip_count = fail_count = 0\n\n");
    // Daemon thread + queue: a stuck call leaves the daemon thread running,
    // and os._exit() at the end kills it without waiting. Unlike
    // ThreadPoolExecutor, we don't register an atexit joiner -- that's the
    // precise reason a raw daemon thread is used here.
    out.push_str("def _run_with_timeout(call, timeout):\n");
    out.push_str("    \"\"\"Run `call()` on a daemon thread; return (kind, value).\n\n");
    out.push_str("    kind is 'ok' (value is the return), 'err' (value is the exception),\n");
    out.push_str("    or 'timeout' (value is None). A stuck call leaks the daemon thread\n");
    out.push_str("    until process exit, which os._exit() at end-of-script kills cleanly.\n");
    out.push_str("    \"\"\"\n");
    out.push_str("    q: \"queue.Queue[tuple[str, object]]\" = queue.Queue(maxsize=1)\n\n");
    out.push_str("    def _worker():\n");
    out.push_str("        try:\n");
    out.push_str("            q.put((\"ok\", call()))\n");
    out.push_str("        except BaseException as exc:  # noqa: BLE001\n");
    out.push_str("            q.put((\"err\", exc))\n\n");
    out.push_str("    t = threading.Thread(target=_worker, daemon=True)\n");
    out.push_str("    t.start()\n");
    out.push_str("    try:\n");
    out.push_str("        return q.get(timeout=timeout)\n");
    out.push_str("    except queue.Empty:\n");
    out.push_str("        return (\"timeout\", None)\n\n");
    out.push_str("for endpoint, mode, min_tier, call in CELLS:\n");
    out.push_str("    label = f\"{endpoint}::{mode}\"\n");
    out.push_str("    kind, value = _run_with_timeout(call, PER_CELL_TIMEOUT_SECS)\n");
    out.push_str("    if kind == \"ok\":\n");
    out.push_str("        print(f\"  {label:60s} PASS\", flush=True)\n");
    out.push_str("        pass_count += 1\n");
    out.push_str("    elif kind == \"timeout\":\n");
    out.push_str(
        "        print(f\"  {label:60s} FAIL  timeout after {PER_CELL_TIMEOUT_SECS}s\", flush=True)\n",
    );
    out.push_str("        fail_count += 1\n");
    out.push_str("    else:  # 'err'\n");
    out.push_str("        msg = str(value).lower()\n");
    out.push_str("        if \"permission\" in msg or \"subscription\" in msg:\n");
    out.push_str(
        "            print(f\"  {label:60s} SKIP: tier-permission (declared min_tier={min_tier})\", flush=True)\n",
    );
    out.push_str("            skip_count += 1\n");
    out.push_str("        elif \"no data found\" in msg:\n");
    out.push_str("            print(f\"  {label:60s} PASS  (no data)\", flush=True)\n");
    out.push_str("            pass_count += 1\n");
    out.push_str("        else:\n");
    out.push_str("            print(f\"  {label:60s} FAIL  {value}\", flush=True)\n");
    out.push_str("            fail_count += 1\n");
    out.push('\n');
    out.push_str(
        "print(f\"\\nPython: {pass_count} PASS, {skip_count} SKIP, {fail_count} FAIL\", flush=True)\n",
    );
    out.push_str("print(f\"COUNTS:{pass_count}:{skip_count}:{fail_count}\", flush=True)\n");
    // os._exit bypasses Python's interpreter shutdown, which otherwise joins
    // non-daemon threads (and ThreadPoolExecutor's atexit hook). A timed-out
    // PyO3 call sitting on a daemon thread is killed abruptly by _exit, which
    // is exactly the behavior we want for a single-shot validator. sys.stdout
    // is already flushed above via flush=True on every print.
    out.push_str("sys.stdout.flush()\n");
    out.push_str("sys.stderr.flush()\n");
    out.push_str("os._exit(1 if fail_count > 0 else 0)\n");
    out
}

/// Generate the Go SDK validator (one row per (endpoint, mode) pair).
fn render_go_validate(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str(
        "// Code generated by generate_sdk_surfaces from endpoint_surface.toml; DO NOT EDIT.\n\n",
    );
    out.push_str("package thetadatadx\n\n");
    out.push_str("import (\n");
    out.push_str("\t\"errors\"\n");
    out.push_str("\t\"fmt\"\n");
    out.push_str("\t\"strings\"\n");
    out.push_str("\t\"time\"\n");
    out.push_str(")\n\n");
    out.push_str("// perCellTimeout caps each live cell at 60 seconds. A stuck cell classifies\n");
    out.push_str("// as FAIL rather than stalling CI indefinitely. See issue #290.\n");
    out.push_str("const perCellTimeout = 60 * time.Second\n\n");
    out.push_str("// errCellTimeout is the sentinel the timeout path returns. We recognize\n");
    out.push_str("// it in classify so the FAIL line clearly says \"timeout\".\n");
    out.push_str("var errCellTimeout = errors.New(\"cell timeout after 60s\")\n\n");
    out.push_str("// runWithTimeout runs call in a goroutine and returns either its error or\n");
    out.push_str("// errCellTimeout if the call doesn't finish within perCellTimeout. The Go\n");
    out.push_str("// SDK doesn't currently thread a context.Context into its CGo calls, so\n");
    out.push_str("// a timed-out goroutine leaks until the blocking CGo call returns on its\n");
    out.push_str("// own. That's acceptable for a validator (process exits at the end) but\n");
    out.push_str("// not a substitute for real cancellation -- which is a separate, larger\n");
    out.push_str("// change.\n");
    out.push_str("func runWithTimeout(call func() error) error {\n");
    out.push_str("\tdone := make(chan error, 1)\n");
    out.push_str("\tgo func() { done <- call() }()\n");
    out.push_str("\tselect {\n");
    out.push_str("\tcase err := <-done:\n");
    out.push_str("\t\treturn err\n");
    out.push_str("\tcase <-time.After(perCellTimeout):\n");
    out.push_str("\t\treturn errCellTimeout\n");
    out.push_str("\t}\n");
    out.push_str("}\n\n");
    out.push_str("// ValidateAllEndpoints runs the live parameter-mode matrix against `c`.\n");
    out.push_str("// Every cell is attempted against production; the server is the ground\n");
    out.push_str("// truth for what the account can access. Cells whose documented min_tier\n");
    out.push_str("// exceeds the live account tier come back as a permission error and are\n");
    out.push_str("// classified as SKIP: tier-permission. Real configuration bugs surface\n");
    out.push_str("// as FAIL. Cells that don't finish within 60 seconds classify as FAIL\n");
    out.push_str("// with \"timeout after 60s\". Returns (pass, skip, fail) counts. See issues\n");
    out.push_str("// #287, #290.\n");
    out.push_str("func ValidateAllEndpoints(c *Client) (int, int, int) {\n");
    out.push_str("\tpass, skip, fail := 0, 0, 0\n");
    out.push_str("\tvar err error\n\n");

    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        let go_name = to_go_exported_name(&endpoint.name);
        let mp = method_params(endpoint);
        for mode in test_modes_for(endpoint) {
            let label = format!("{}::{}", endpoint.name, mode.name);
            let args = mp
                .iter()
                .zip(mode.args.iter())
                .map(|(param, value)| go_arg_literal(param, value))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                out,
                "\terr = runWithTimeout(func() error {{ _, e := c.{}({}); return e }})",
                go_name, args
            )
            .unwrap();
            writeln!(
                out,
                "\tclassify({label:?}, {:?}, err, &pass, &skip, &fail)",
                mode.min_tier
            )
            .unwrap();
        }
        out.push('\n');
    }

    out.push_str("\treturn pass, skip, fail\n");
    out.push_str("}\n\n");
    out.push_str("// classify maps a live call outcome into PASS / SKIP / FAIL buckets.\n");
    out.push_str("// `declaredMinTier` is echoed on tier-permission skips so the caller can\n");
    out.push_str("// see which documented tier the server refused.\n");
    out.push_str(
        "func classify(label, declaredMinTier string, err error, pass, skip, fail *int) {\n",
    );
    out.push_str("\tif err == nil {\n");
    out.push_str("\t\tfmt.Printf(\"  %-60s PASS\\n\", label)\n");
    out.push_str("\t\t*pass++\n");
    out.push_str("\t\treturn\n");
    out.push_str("\t}\n");
    out.push_str("\tif errors.Is(err, errCellTimeout) {\n");
    out.push_str("\t\tfmt.Printf(\"  %-60s FAIL  timeout after 60s\\n\", label)\n");
    out.push_str("\t\t*fail++\n");
    out.push_str("\t\treturn\n");
    out.push_str("\t}\n");
    out.push_str("\tlowered := strings.ToLower(err.Error())\n");
    out.push_str(
        "\tif strings.Contains(lowered, \"permission\") || strings.Contains(lowered, \"subscription\") {\n",
    );
    out.push_str(
        "\t\tfmt.Printf(\"  %-60s SKIP: tier-permission (declared min_tier=%s)\\n\", label, declaredMinTier)\n",
    );
    out.push_str("\t\t*skip++\n");
    out.push_str("\t\treturn\n");
    out.push_str("\t}\n");
    out.push_str("\tif strings.Contains(lowered, \"no data found\") {\n");
    out.push_str("\t\tfmt.Printf(\"  %-60s PASS  (no data)\\n\", label)\n");
    out.push_str("\t\t*pass++\n");
    out.push_str("\t\treturn\n");
    out.push_str("\t}\n");
    out.push_str("\tfmt.Printf(\"  %-60s FAIL  %v\\n\", label, err)\n");
    out.push_str("\t*fail++\n");
    out.push_str("}\n");

    out
}

/// Generate the C++ SDK validator (one row per (endpoint, mode) pair).
fn render_cpp_validate(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT \u{2014} regenerated by generate_sdk_surfaces from endpoint_surface.toml\n",
    );
    out.push_str("// Live parameter-mode matrix validator for the C++ SDK. Every cell is\n");
    out.push_str("// attempted against production; the server is the ground truth for what\n");
    out.push_str("// the account can access. Cells whose documented min_tier exceeds the\n");
    out.push_str("// live account tier come back as a permission error and are classified\n");
    out.push_str("// SKIP: tier-permission. Real configuration bugs surface as FAIL. Cells\n");
    out.push_str("// that don't finish within 60 seconds classify as FAIL with \"timeout\n");
    out.push_str("// after 60s\". The worker thread is detached and keeps running the SDK\n");
    out.push_str("// call; to avoid a use-after-free race between that leaked thread and\n");
    out.push_str("// the Client/Config/Credentials destructors on main's unwind, the\n");
    out.push_str("// validator exits via std::_Exit whenever any timeout fired -- skipping\n");
    out.push_str("// destructors is exactly what we want when a leaked thread might still\n");
    out.push_str("// touch FFI state. When no timeouts fired the normal return path is\n");
    out.push_str("// taken so RAII runs as usual. See issues #287, #290.\n");
    out.push_str("#include <algorithm>\n");
    out.push_str("#include <cctype>\n");
    out.push_str("#include <chrono>\n");
    out.push_str("#include <cstdlib>\n");
    out.push_str("#include <future>\n");
    out.push_str("#include <iomanip>\n");
    out.push_str("#include <iostream>\n");
    out.push_str("#include <string>\n");
    out.push_str("#include <thread>\n");
    out.push_str("#include <utility>\n");
    out.push_str("#include <vector>\n");
    out.push_str("#include \"thetadx.hpp\"\n\n");
    out.push_str("namespace {\n\n");
    out.push_str("constexpr auto kPerCellTimeout = std::chrono::seconds(60);\n\n");
    out.push_str("std::string lower(std::string value) {\n");
    out.push_str(
        "    std::transform(value.begin(), value.end(), value.begin(), [](unsigned char c) {\n",
    );
    out.push_str("        return static_cast<char>(std::tolower(c));\n");
    out.push_str("    });\n");
    out.push_str("    return value;\n");
    out.push_str("}\n\n");
    out.push_str("} // namespace\n\n");
    out.push_str("int main(int argc, char** argv) {\n");
    out.push_str("    const std::string creds_path = argc > 1 ? argv[1] : \"creds.txt\";\n");
    out.push_str("    int pass = 0;\n");
    out.push_str("    int skip = 0;\n");
    out.push_str("    int fail = 0;\n");
    out.push_str("    // Track whether any cell timed out. Leaked worker threads racing with\n");
    out.push_str("    // RAII Close() on client/config/creds would be UB, so we _Exit at end\n");
    out.push_str("    // if this flag is set instead of returning.\n");
    out.push_str("    bool any_timeout = false;\n\n");
    out.push_str("    try {\n");
    out.push_str("        auto creds = tdx::Credentials::from_file(creds_path);\n");
    out.push_str("        auto config = tdx::Config::production();\n");
    out.push_str("        auto client = tdx::Client::connect(creds, config);\n\n");
    out.push_str(
        "        auto cell = [&](const char* label, const char* declared_min_tier, auto&& call) {\n",
    );
    out.push_str("            try {\n");
    out.push_str("                // std::packaged_task + detached std::thread so a timed-out\n");
    out.push_str("                // future doesn't block in its destructor (which std::async's\n");
    out.push_str("                // future does). Worker thread leaks until the SDK call\n");
    out.push_str("                // returns; we flag any_timeout so main exits via _Exit and\n");
    out.push_str("                // avoids destroying the Client handle while the leaked\n");
    out.push_str("                // thread may still be reading from it (UAF).\n");
    out.push_str("                using Result = decltype(call());\n");
    out.push_str(
        "                std::packaged_task<Result()> task(std::forward<decltype(call)>(call));\n",
    );
    out.push_str("                auto future = task.get_future();\n");
    out.push_str("                std::thread(std::move(task)).detach();\n");
    out.push_str(
        "                if (future.wait_for(kPerCellTimeout) == std::future_status::timeout) {\n",
    );
    out.push_str(
        "                    std::cout << \"  \" << std::left << std::setw(60) << label << \" FAIL  timeout after 60s\" << std::endl;\n",
    );
    out.push_str("                    ++fail;\n");
    out.push_str("                    any_timeout = true;\n");
    out.push_str("                    return;\n");
    out.push_str("                }\n");
    out.push_str("                (void)future.get();\n");
    out.push_str(
        "                std::cout << \"  \" << std::left << std::setw(60) << label << \" PASS\" << std::endl;\n",
    );
    out.push_str("                ++pass;\n");
    out.push_str("            } catch (const std::exception& e) {\n");
    out.push_str("                const std::string msg = lower(e.what());\n");
    out.push_str(
        "                if (msg.find(\"permission\") != std::string::npos || msg.find(\"subscription\") != std::string::npos) {\n",
    );
    out.push_str(
        "                    std::cout << \"  \" << std::left << std::setw(60) << label << \" SKIP: tier-permission (declared min_tier=\" << declared_min_tier << \")\" << std::endl;\n",
    );
    out.push_str("                    ++skip;\n");
    out.push_str(
        "                } else if (msg.find(\"no data found\") != std::string::npos) {\n",
    );
    out.push_str(
        "                    std::cout << \"  \" << std::left << std::setw(60) << label << \" PASS  (no data)\" << std::endl;\n",
    );
    out.push_str("                    ++pass;\n");
    out.push_str("                } else {\n");
    out.push_str(
        "                    std::cout << \"  \" << std::left << std::setw(60) << label << \" FAIL  \" << e.what() << std::endl;\n",
    );
    out.push_str("                    ++fail;\n");
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("        };\n\n");

    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        let mp = method_params(endpoint);
        for mode in test_modes_for(endpoint) {
            let label = format!("{}::{}", endpoint.name, mode.name);
            let args = mp
                .iter()
                .zip(mode.args.iter())
                .map(|(param, value)| cpp_arg_literal(param, value))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                out,
                "        cell({:?}, {:?}, [&] {{ return client.{}({}); }});",
                label, mode.min_tier, endpoint.name, args
            )
            .unwrap();
        }
    }

    out.push_str("    } catch (const std::exception& e) {\n");
    out.push_str(
        "        std::cerr << \"validator bootstrap failure: \" << e.what() << std::endl;\n",
    );
    out.push_str("        return 1;\n");
    out.push_str("    }\n\n");
    out.push_str(
        "    std::cout << \"\\nC++: \" << pass << \" PASS, \" << skip << \" SKIP, \" << fail << \" FAIL\" << std::endl;\n",
    );
    out.push_str(
        "    std::cout << \"COUNTS:\" << pass << \":\" << skip << \":\" << fail << std::endl;\n",
    );
    out.push_str("    std::cout.flush();\n");
    out.push_str("    std::cerr.flush();\n");
    out.push_str("    if (any_timeout) {\n");
    out.push_str("        // Skip destructors -- leaked worker thread may still be reading\n");
    out.push_str("        // Client/Config/Credentials FFI handles. _Exit terminates the\n");
    out.push_str("        // process without unwinding.\n");
    out.push_str("        std::_Exit(fail > 0 ? 1 : 0);\n");
    out.push_str("    }\n");
    out.push_str("    return fail > 0 ? 1 : 0;\n");
    out.push_str("}\n");
    out
}

fn render_cpp_options(params: &[GeneratedParam]) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml\n",
    );
    out.push_str("/// Optional builder parameters for registry-driven endpoint wrappers.\n");
    out.push_str("struct EndpointRequestOptions {\n");
    for param in params {
        let ty = match param.param_type.as_str() {
            "Int" => "std::optional<int32_t>",
            "Float" => "std::optional<double>",
            "Bool" => "std::optional<bool>",
            _ => "std::optional<std::string>",
        };
        writeln!(out, "    /// {}", param.description).unwrap();
        writeln!(out, "    {} {};", ty, param.name).unwrap();
    }
    out.push('\n');
    for param in params {
        let arg_ty = builder_value_type_name(param);
        writeln!(
            out,
            "    EndpointRequestOptions& with_{}({} value) {{",
            param.name, arg_ty
        )
        .unwrap();
        writeln!(out, "        {};", builder_copy_expr(param, "value")).unwrap();
        out.push_str("        return *this;\n");
        out.push_str("    }\n");
    }
    out.push_str("};\n");
    out.push_str("\nnamespace detail {\n\n");
    out.push_str("struct FfiEndpointRequestOptions {\n");
    out.push_str("    TdxEndpointRequestOptions raw{};\n");
    for param in params {
        if param.param_type != "Int" && param.param_type != "Float" && param.param_type != "Bool" {
            writeln!(out, "    std::string {}_storage;", param.name).unwrap();
        }
    }
    out.push('\n');
    out.push_str(
        "    explicit FfiEndpointRequestOptions(const EndpointRequestOptions& options) {\n",
    );
    for param in params {
        match param.param_type.as_str() {
            "Int" => {
                writeln!(out, "        if (options.{0}) {{", param.name).unwrap();
                writeln!(out, "            raw.{0} = *options.{0};", param.name).unwrap();
                writeln!(out, "            raw.has_{0} = 1;", param.name).unwrap();
                out.push_str("        }\n");
            }
            "Float" => {
                writeln!(out, "        if (options.{0}) {{", param.name).unwrap();
                writeln!(out, "            raw.{0} = *options.{0};", param.name).unwrap();
                writeln!(out, "            raw.has_{0} = 1;", param.name).unwrap();
                out.push_str("        }\n");
            }
            "Bool" => {
                writeln!(out, "        if (options.{0}) {{", param.name).unwrap();
                writeln!(
                    out,
                    "            raw.{0} = *options.{0} ? 1 : 0;",
                    param.name
                )
                .unwrap();
                writeln!(out, "            raw.has_{0} = 1;", param.name).unwrap();
                out.push_str("        }\n");
            }
            _ => {
                writeln!(out, "        if (options.{0}) {{", param.name).unwrap();
                writeln!(out, "            {0}_storage = *options.{0};", param.name).unwrap();
                writeln!(
                    out,
                    "            raw.{0} = {0}_storage.c_str();",
                    param.name
                )
                .unwrap();
                out.push_str("        }\n");
            }
        }
    }
    out.push_str("    }\n");
    out.push_str("};\n\n");
    out.push_str("} // namespace detail\n");
    out
}

fn render_cpp_historical_decls(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str("    // @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml\n\n");
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        writeln!(out, "    /** {} */", endpoint.description).unwrap();
        let mut params = method_params(endpoint)
            .iter()
            .map(|param| cpp_method_arg_decl(param))
            .collect::<Vec<_>>();
        if has_builder_params(endpoint) {
            params.push("const EndpointRequestOptions& options = {}".into());
        }
        writeln!(
            out,
            "    std::vector<{}> {}({}) const;\n",
            cpp_value_type(&endpoint.return_type),
            endpoint.name,
            params.join(", ")
        )
        .unwrap();
    }
    out
}

fn render_cpp_historical_defs(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml\n\n",
    );
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| !is_streaming_endpoint(endpoint))
    {
        out.push_str(&render_cpp_endpoint_def(endpoint));
        out.push('\n');
    }
    out
}

fn render_cpp_endpoint_def(endpoint: &GeneratedEndpoint) -> String {
    let method_params = method_params(endpoint);
    let mut signature_parts = method_params
        .iter()
        .map(|param| cpp_method_arg_decl(param))
        .collect::<Vec<_>>();
    if has_builder_params(endpoint) {
        signature_parts.push("const EndpointRequestOptions& options".into());
    }

    let mut out = String::new();
    writeln!(
        out,
        "std::vector<{}> Client::{}({}) const {{",
        cpp_value_type(&endpoint.return_type),
        endpoint.name,
        signature_parts.join(", ")
    )
    .unwrap();

    let has_symbols = method_params
        .iter()
        .any(|param| param.param_type == "Symbols");
    if has_symbols {
        out.push_str("    auto symbol_ptrs = detail::string_ptrs(symbols);\n");
    }
    if has_builder_params(endpoint) {
        out.push_str("    detail::FfiEndpointRequestOptions ffi_options(options);\n");
    }

    if endpoint.return_type == "StringList" {
        write!(
            out,
            "    return detail::check_string_array(tdx_{}",
            endpoint.name
        )
        .unwrap();
        if has_builder_params(endpoint) {
            out.push_str("_with_options");
        }
        out.push_str("(handle_.get()");
        for param in &method_params {
            if param.param_type == "Symbols" {
                out.push_str(", symbol_ptrs.data(), symbol_ptrs.size()");
            } else {
                write!(out, ", {}.c_str()", sdk_method_arg_name(param)).unwrap();
            }
        }
        if has_builder_params(endpoint) {
            out.push_str(", &ffi_options.raw");
        }
        out.push_str("));\n");
        out.push_str("}\n");
        return out;
    }

    if endpoint.return_type == "OptionContracts" {
        write!(
            out,
            "    TdxOptionContractArray arr = tdx_{}",
            endpoint.name
        )
        .unwrap();
        if has_builder_params(endpoint) {
            out.push_str("_with_options");
        }
        out.push_str("(handle_.get()");
        for param in &method_params {
            if param.param_type == "Symbols" {
                out.push_str(", symbol_ptrs.data(), symbol_ptrs.size()");
            } else {
                write!(out, ", {}.c_str()", sdk_method_arg_name(param)).unwrap();
            }
        }
        if has_builder_params(endpoint) {
            out.push_str(", &ffi_options.raw");
        }
        out.push_str(");\n");
        out.push_str("    std::vector<OptionContract> result;\n");
        out.push_str("    result.reserve(arr.len);\n");
        out.push_str("    for (size_t i = 0; i < arr.len; ++i) {\n");
        out.push_str("        OptionContract c;\n");
        out.push_str("        c.root = arr.data[i].root ? std::string(arr.data[i].root) : \"\";\n");
        out.push_str("        c.expiration = arr.data[i].expiration;\n");
        out.push_str("        c.strike = arr.data[i].strike;\n");
        out.push_str("        c.right = arr.data[i].right;\n");
        out.push_str("        result.push_back(std::move(c));\n");
        out.push_str("    }\n");
        out.push_str("    tdx_option_contract_array_free(arr);\n");
        out.push_str("    return result;\n");
        out.push_str("}\n");
        return out;
    }

    write!(out, "    auto arr = tdx_{}", endpoint.name).unwrap();
    if has_builder_params(endpoint) {
        out.push_str("_with_options");
    }
    out.push_str("(handle_.get()");
    for param in &method_params {
        if param.param_type == "Symbols" {
            out.push_str(", symbol_ptrs.data(), symbol_ptrs.size()");
        } else {
            write!(out, ", {}.c_str()", sdk_method_arg_name(param)).unwrap();
        }
    }
    if has_builder_params(endpoint) {
        out.push_str(", &ffi_options.raw");
    }
    out.push_str(");\n");
    writeln!(out, "    {}", cpp_converter_expr(&endpoint.return_type)).unwrap();
    out.push_str("}\n");
    out
}

fn render_ffi_with_options(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml\n\n",
    );
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| has_builder_params(endpoint) && !is_streaming_endpoint(endpoint))
    {
        out.push_str(&render_ffi_with_options_endpoint(endpoint));
        out.push('\n');
    }
    out
}

fn render_ffi_with_options_endpoint(endpoint: &GeneratedEndpoint) -> String {
    let method_params = method_params(endpoint);
    let array_type = ffi_array_type(&endpoint.return_type);
    let output_variant = ffi_output_variant(&endpoint.return_type);
    let from_vec_expr = ffi_from_vec_expr(&endpoint.return_type);
    let mut out = String::new();

    writeln!(
        out,
        "/// {} with optional builder parameters.",
        endpoint.description
    )
    .unwrap();
    out.push_str("#[no_mangle]\n");
    write!(
        out,
        "pub unsafe extern \"C\" fn tdx_{}_with_options(\n    client: *const TdxClient",
        endpoint.name
    )
    .unwrap();
    for param in &method_params {
        if param.param_type == "Symbols" {
            out.push_str(",\n    symbols: *const *const c_char,\n    symbols_len: usize");
        } else {
            writeln!(out, ",\n    {}: *const c_char", param.name).unwrap();
        }
    }
    out.push_str(",\n    options: *const TdxEndpointRequestOptions,\n");
    writeln!(out, ") -> {} {{", array_type).unwrap();
    writeln!(
        out,
        "    let empty = {} {{ data: ptr::null(), len: 0 }};",
        array_type
    )
    .unwrap();
    out.push_str("    if client.is_null() {\n");
    out.push_str("        set_error(\"client handle is null\");\n");
    out.push_str("        return empty;\n");
    out.push_str("    }\n\n");
    out.push_str("    let mut args = thetadatadx::EndpointArgs::new();\n");

    for param in &method_params {
        if param.param_type == "Symbols" {
            out.push_str(
                "    let symbols = match unsafe { parse_symbol_array(symbols, symbols_len) } {\n",
            );
            out.push_str("        Some(values) => values,\n");
            out.push_str("        None => return empty,\n");
            out.push_str("    };\n");
            out.push_str("    args.insert(\n");
            out.push_str("        \"symbol\".to_string(),\n");
            out.push_str("        thetadatadx::EndpointArgValue::Str(symbols.join(\",\")),\n");
            out.push_str("    );\n");
        } else {
            writeln!(
                out,
                "    let {} = match unsafe {{ cstr_to_str({}) }} {{",
                param.name, param.name
            )
            .unwrap();
            out.push_str("        Some(value) => value,\n");
            writeln!(
                out,
                "        None => {{\n            set_error(\"{} is null or invalid UTF-8\");\n            return empty;\n        }}",
                param.name
            )
            .unwrap();
            out.push_str("    };\n");
            out.push_str("    args.insert(\n");
            writeln!(out, "        {:?}.to_string(),", param.name).unwrap();
            writeln!(
                out,
                "        thetadatadx::EndpointArgValue::Str({}.to_string()),",
                param.name
            )
            .unwrap();
            out.push_str("    );\n");
        }
    }

    out.push_str(
        "\n    if let Err(message) = apply_endpoint_request_options(&mut args, options) {\n",
    );
    out.push_str("        set_error(&message);\n");
    out.push_str("        return empty;\n");
    out.push_str("    }\n\n");
    out.push_str("    let client = unsafe { &*client };\n");
    out.push_str("    match runtime().block_on(async {\n");
    writeln!(
        out,
        "        thetadatadx::endpoint::invoke_endpoint(&client.inner, {:?}, &args).await",
        endpoint.name
    )
    .unwrap();
    out.push_str("    }) {\n");
    writeln!(
        out,
        "        Ok(thetadatadx::EndpointOutput::{}(values)) => {},",
        output_variant, from_vec_expr
    )
    .unwrap();
    out.push_str("        Ok(other) => {\n");
    writeln!(
        out,
        "            set_error(&format!(\"internal error: unexpected endpoint output for {}: {{other:?}}\"));",
        endpoint.name
    )
    .unwrap();
    out.push_str("            empty\n");
    out.push_str("        }\n");
    out.push_str("        Err(error) => {\n");
    out.push_str("            set_error(&error.to_string());\n");
    out.push_str("            empty\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}
