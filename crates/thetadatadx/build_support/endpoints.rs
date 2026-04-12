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
#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
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
            writeln!(
                out,
                "        {}: {},",
                field.name,
                direct_query_field_expr(endpoint, field, true)
            )
            .unwrap();
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
            writeln!(
                out,
                "        {}: {},",
                field.name,
                direct_query_field_expr(endpoint, field, false)
            )
            .unwrap();
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
    writeln!(out, "streaming_endpoint! {{").unwrap();
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
    let tick_type = direct_stream_tick_type(&endpoint.return_type);
    writeln!(
        out,
        "    fn {}({signature}) -> {};",
        endpoint.name, tick_type
    )
    .unwrap();

    writeln!(out, "    grpc: {};", endpoint.grpc_name).unwrap();
    writeln!(out, "    request: {};", endpoint.request_type).unwrap();
    if endpoint.fields.is_empty() {
        writeln!(out, "    query: {} {{}};", endpoint.query_type).unwrap();
    } else {
        writeln!(out, "    query: {} {{", endpoint.query_type).unwrap();
        for field in &endpoint.fields {
            writeln!(
                out,
                "        {}: {},",
                field.name,
                direct_query_field_expr(endpoint, field, false)
            )
            .unwrap();
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
/// (e.g. `TradeTick`) used by `streaming_endpoint!`.
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
            "Milliseconds from midnight ET (e.g. 34200000 = 9:30 AM)".into(),
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
        if actual != file.contents {
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
            relative_path: "ffi/src/generated_endpoint_with_options.rs",
            contents: render_ffi_with_options(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/generated_endpoint_options.go",
            contents: render_go_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/generated_historical.go",
            contents: render_go_historical(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/go/generated_endpoint_with_options.h.inc",
            contents: render_go_endpoint_with_options_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/generated_endpoint_options.hpp.inc",
            contents: render_cpp_options(&builder_params),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/generated_historical.hpp.inc",
            contents: render_cpp_historical_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/generated_endpoint_with_options.h.inc",
            contents: render_c_endpoint_with_options_decls(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/src/generated_historical.cpp.inc",
            contents: render_cpp_historical_defs(&parsed.endpoints),
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/generated_historical_methods.rs",
            contents: render_python_historical_methods(&parsed.endpoints),
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
    out.push_str(
        "// @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml\n\n",
    );
    out.push_str("package thetadatadx\n\n");
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

    out
}

fn render_go_endpoint_with_options_decls(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str("/* @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml */\n");
    for endpoint in endpoints
        .iter()
        .filter(|endpoint| has_builder_params(endpoint))
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
        .filter(|endpoint| has_builder_params(endpoint))
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
    out.push_str(
        "// @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml\n\n",
    );
    out.push_str("package thetadatadx\n\n");
    out.push_str("/*\n#include \"ffi_bridge.h\"\n*/\nimport \"C\"\n\n");
    out.push_str("import \"unsafe\"\n\n");

    for endpoint in endpoints {
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
    for endpoint in endpoints {
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
        out.push_str("Vec<Py<PyAny>>> {\n");
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
        "        Ok(ticks.iter().map(|t| {}(py, t)).collect())",
        python_converter(&endpoint.return_type)
    )
    .unwrap();
    out.push_str("    }\n");
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
    out
}

fn render_cpp_historical_decls(endpoints: &[GeneratedEndpoint]) -> String {
    let mut out = String::new();
    out.push_str("    // @generated DO NOT EDIT \u{2014} regenerated by build.rs from endpoint_surface.toml\n\n");
    for endpoint in endpoints {
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
    for endpoint in endpoints {
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
        .filter(|endpoint| has_builder_params(endpoint))
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
