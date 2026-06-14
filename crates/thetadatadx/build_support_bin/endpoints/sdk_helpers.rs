//! Render-side helpers used only by `sdk_render/*` emitters.
//!
//! These are the param-classification, naming, casing, per-language type
//! tables, FFI / builder option mappers, arg-declaration / arg-literal
//! renderers, and CLI command derivations consumed by the per-language
//! SDK projection emitters. The build script never reaches them.
//!
//! Shared cross-renderer utilities (the `direct_*` family the in-house
//! Rust client also needs) stay in `build_support/endpoints/helpers.rs`.
//! Each bin-side render file imports from both modules.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use super::helpers::is_method_call_param;
use super::model::{GeneratedEndpoint, GeneratedParam};
use super::modes::TestMode;

// ─────────────────────── Bin-side render-map loader ─────────────────────────

/// Per-tick binding-name map keyed by wire-collection plural (e.g.
/// `"GreeksAllTicks"`). Carries every per-language name the SDK
/// projection emitters need. Loaded once from `tick_schema.toml` on the
/// first call.
#[derive(Debug, Clone)]
struct BinTickRender {
    ffi_array: String,
    ffi_output_variant: String,
    ffi_from_vec_array: String,
    ffi_free_fn: String,
    cpp_value: String,
    python_pyclass_list: String,
    python_vec_to_pylist: String,
    ts_class: String,
    ts_class_vec: String,
    pyclass: String,
}

#[derive(serde::Deserialize)]
struct SchemaToml {
    types: HashMap<String, TickTypeToml>,
}

#[derive(serde::Deserialize)]
struct TickTypeToml {
    render: TickRenderToml,
}

#[derive(serde::Deserialize)]
struct TickRenderToml {
    collection: String,
    ffi_array: String,
    ffi_output_variant: String,
    ffi_from_vec_array: String,
    ffi_free_fn: String,
    cpp_value: String,
    python_pyclass_list: String,
    python_vec_to_pylist: String,
    ts_class: String,
    ts_class_vec: String,
    pyclass: String,
}

static RENDER_MAP: OnceLock<HashMap<String, BinTickRender>> = OnceLock::new();

fn load_render_map() -> HashMap<String, BinTickRender> {
    let schema_path = "tick_schema.toml";
    let raw = std::fs::read_to_string(schema_path)
        .unwrap_or_else(|e| panic!("failed to read {schema_path}: {e}"));
    let parsed: SchemaToml =
        toml::from_str(&raw).unwrap_or_else(|e| panic!("failed to parse {schema_path}: {e}"));
    let mut map = HashMap::new();
    let mut tick_to_collection: HashMap<String, String> = HashMap::new();
    for (tick_name, def) in parsed.types {
        let render = def.render;
        let collection = render.collection.clone();
        if let Some(prev) = tick_to_collection.insert(tick_name.clone(), collection.clone()) {
            panic!("tick type '{tick_name}' duplicates collection '{prev}' / '{collection}'");
        }
        let entry = BinTickRender {
            ffi_array: render.ffi_array,
            ffi_output_variant: render.ffi_output_variant,
            ffi_from_vec_array: render.ffi_from_vec_array,
            ffi_free_fn: render.ffi_free_fn,
            cpp_value: render.cpp_value,
            python_pyclass_list: render.python_pyclass_list,
            python_vec_to_pylist: render.python_vec_to_pylist,
            ts_class: render.ts_class,
            ts_class_vec: render.ts_class_vec,
            pyclass: render.pyclass,
        };
        if map.insert(collection.clone(), entry).is_some() {
            panic!("duplicate render collection '{collection}' in tick_schema.toml");
        }
    }
    map
}

/// Look up the per-language render names for a wire-collection plural
/// (e.g. `"GreeksTicks"`). Panics with the available keys when the
/// collection name is missing -- a missing TOML row is a build-time bug.
fn render_for(collection: &str) -> &'static BinTickRender {
    let map = RENDER_MAP.get_or_init(load_render_map);
    map.get(collection).unwrap_or_else(|| {
        let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
        keys.sort();
        panic!(
            "no render entry for collection '{collection}' in tick_schema.toml; available: {}",
            keys.join(", ")
        )
    })
}

// ───────────────────────── Param classification ────────────────────────────

/// Returns the endpoint's required positional parameters (those passed
/// in the method call rather than chained on the builder).
pub(super) fn method_params(endpoint: &GeneratedEndpoint) -> Vec<&GeneratedParam> {
    endpoint
        .params
        .iter()
        .filter(|param| is_method_call_param(param))
        .collect()
}

/// Returns the endpoint's optional parameters (those chained on the
/// builder rather than passed in the method call).
pub(super) fn builder_params(endpoint: &GeneratedEndpoint) -> Vec<&GeneratedParam> {
    endpoint
        .params
        .iter()
        .filter(|param| !is_method_call_param(param))
        .collect()
}

/// Collects the deduplicated set of builder parameters across every
/// endpoint, preserving first-appearance order.
pub(super) fn collect_builder_params(endpoints: &[GeneratedEndpoint]) -> Vec<GeneratedParam> {
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

/// Return `true` if the endpoint is a latency-sensitive single-row (or
/// ≤10-row) snapshot/calendar lookup. Triggers the Python fast path:
/// no `<T>List` wrapper, no `run_blocking` signal-check ticker, bounded
/// `tokio::time::timeout` instead.
///
/// Re-exports the shared SSOT predicate (`helpers::is_snapshot_endpoint`) so
/// the Python / docs emitters classify snapshots identically — a single
/// definition keyed off `endpoint_surface.toml` structure, never a
/// hand-curated allowlist. The streaming emitters import the streaming
/// predicates (`endpoint_streams`, `endpoint_streams_repr_c_ticks`) straight
/// from `helpers`.
pub(super) use super::helpers::is_snapshot_endpoint;

// ───────────────────────── Docstring composition ───────────────────────────

/// Format an endpoint doc body as a sequence of Rust `///` lines with the
/// given indent. Used by the Python + TypeScript pymethod/napi emitters
/// so that sync, async, and builder variants share one render path.
pub(super) fn render_rust_doc_block(indent: &str, doc: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for line in doc.lines() {
        if line.is_empty() {
            writeln!(out, "{indent}///").unwrap();
        } else {
            writeln!(out, "{indent}/// {line}").unwrap();
        }
    }
    out
}

// ───────────────────────── Casing ────────────────────────────────────────────

/// Returns the Go-exported PascalCase form of a single name segment,
/// keeping known initialisms (EOD, OHLC, IV, DTE, NBBO) fully upper.
pub(super) fn go_segment_pascal(segment: &str) -> String {
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

/// Returns the Go-exported PascalCase name for an underscore-separated
/// identifier, applying `go_segment_pascal` to each segment.
pub(super) fn to_go_exported_name(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(go_segment_pascal)
        .collect::<String>()
}

/// Returns the camelCase form of an underscore-separated identifier
/// (PascalCase with a lowercased first letter).
pub(super) fn to_camel_case(value: &str) -> String {
    let pascal = to_go_exported_name(value);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

// ───────────────────────── Per-language type tables ─────────────────────────

/// Returns the Rust `Option<...>` type used for an optional Python
/// keyword argument of the given parameter.
pub(super) fn python_optional_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "Option<i32>",
        "Float" => "Option<f64>",
        "Bool" => "Option<bool>",
        "Date" | "Expiration" => "Option<PyDateArg>",
        _ if is_time_arg(param) => "Option<PyTimeArg>",
        _ => "Option<PyStringArg>",
    }
}

/// Returns the Rust argument type used for a required Python parameter
/// of the given wire type.
pub(super) fn python_string_arg_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Symbols" => "PySymbols",
        "Date" | "Expiration" => "PyDateArg",
        _ if is_time_arg(param) => "PyTimeArg",
        _ => "PyStringArg",
    }
}

/// Returns `true` if the parameter is a time-of-day argument
/// (`start_time`, `end_time`, `min_time`, or `time_of_day`).
pub(super) fn is_time_arg(param: &GeneratedParam) -> bool {
    matches!(
        param.name.as_str(),
        "start_time" | "end_time" | "min_time" | "time_of_day"
    )
}

/// Returns the FFI `#[repr(C)]` array type name for a return type
/// (e.g. `TdxStringArray`), looked up from the tick render map.
pub(super) fn ffi_array_type(return_type: &str) -> String {
    if return_type == "StringList" {
        return "TdxStringArray".into();
    }
    render_for(return_type).ffi_array.clone()
}

/// Returns the `EndpointOutput` variant name for a return type, looked
/// up from the tick render map.
pub(super) fn ffi_output_variant(return_type: &str) -> String {
    if return_type == "StringList" {
        return "StringList".into();
    }
    render_for(return_type).ffi_output_variant.clone()
}

/// Returns the `#[repr(C)]` array type name for the given `EndpointOutput`
/// variant (e.g. `TdxEodTickArray`). The emitter wraps `<type>::from_vec(...)`
/// — which returns `Result<Self, NulError>` — in an inline match that routes
/// interior-NUL failures through the FFI error slot.
pub(super) fn ffi_from_vec_array_type(return_type: &str) -> String {
    if return_type == "StringList" {
        return "TdxStringArray".into();
    }
    render_for(return_type).ffi_from_vec_array.clone()
}

/// Returns the C++ element type for a return type's row vector, looked
/// up from the tick render map.
pub(super) fn cpp_value_type(return_type: &str) -> String {
    if return_type == "StringList" {
        return "std::string".into();
    }
    render_for(return_type).cpp_value.clone()
}

/// Returns the C++ expression that converts the FFI array result for a
/// return type into a `std::vector`, checking the error slot first.
pub(super) fn cpp_converter_expr(return_type: &str) -> String {
    match return_type {
        "StringList" => "return detail::check_string_array(arr);".into(),
        "OptionContracts" => "return detail::option_contract_array_to_vector(arr);".into(),
        other => {
            // Check `tdx_last_error_raw` before converting: success-empty
            // and failure (e.g. timeout) both return `{nullptr, 0}` arrays,
            // so we have to consult the error slot directly. The generated
            // Client method `tdx_clear_error()`s before the FFI call so a
            // stale error from a prior call isn't misattributed.
            let free_fn = render_for(other).ffi_free_fn.clone();
            format!(
                "{{\n        const std::string err = detail::last_ffi_error_raw();\n        if (!err.empty()) {{\n            {free_fn}(arr);\n            throw std::runtime_error(\"thetadatadx: \" + err);\n        }}\n    }}\n    auto result = detail::to_vector(arr.data, arr.len);\n    {free_fn}(arr);\n    return result;"
            )
        }
    }
}

/// Name of the generated `*_to_pyclass_list` converter for a given tick
/// return type. This is the PRIMARY return path for Python historical
/// endpoints — typed `#[pyclass]` objects matching Rust/TS/Go/C++ SDKs.
/// See `build_support_bin/ticks/python_classes.rs::render_python_tick_classes`.
pub(super) fn python_pyclass_list_converter(return_type: &str) -> String {
    render_for(return_type).python_pyclass_list.clone()
}

/// Name of the generated `<TickName>List` pyclass wrapper (e.g.
/// `EodTickList`). Historical endpoints return `Py<<TickName>List>`
/// directly so callers can chain `.to_polars()` / `.to_arrow()` /
/// `.to_pandas()` / `.to_list()` off the endpoint return value.
///
/// Derived from the per-language `pyclass` render name + the `List`
/// suffix the emitter applies to every pyclass-list type.
pub(super) fn python_pyclass_list_class(return_type: &str) -> String {
    format!("{}List", render_for(return_type).pyclass)
}

/// Map a collection return type (e.g. `CalendarDays`) to the generated
/// `<tick>_vec_to_pylist` converter in `tick_classes.rs`. This is the
/// snapshot-endpoint fast path: takes a decoder-owned `Vec<tick::T>` and
/// materialises a plain `Py<PyList>` of typed pyclass instances, skipping
/// the `<TickName>List` wrapper allocation. Used only for snapshot- and
/// calendar-kind endpoints (see `is_snapshot_endpoint`). Parsed list
/// endpoints keep the wrapper because users chain `.to_polars()` on bulk
/// results.
pub(super) fn python_vec_to_pylist_converter(return_type: &str) -> String {
    render_for(return_type).python_vec_to_pylist.clone()
}

/// Map a collection return type (e.g. `TradeTicks`) to the generated
/// `#[napi(object)]` struct name emitted in `tick_classes.rs`. The TS SDK
/// binds each Rust tick struct (the `thetadatadx::*` tick types) to this
/// flat napi-object variant so `Vec<T>` surfaces as `T[]` in `index.d.ts`.
pub(super) fn ts_class_name(return_type: &str) -> String {
    render_for(return_type).ts_class.clone()
}

/// Map a collection return type to the generated
/// `{tick}_to_class_vec` factory name. Complements `ts_class_name`.
pub(super) fn ts_class_vec_converter(return_type: &str) -> String {
    render_for(return_type).ts_class_vec.clone()
}

// ───────────────────────── Builder / FFI option tables ─────────────────────

/// Returns the C++ value type for a builder setter argument of the
/// given parameter.
pub(super) fn builder_value_type_name(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "int32_t",
        "Float" => "double",
        "Bool" => "bool",
        _ => "std::string",
    }
}

/// Returns the C++ assignment expression storing `source` into the
/// builder field, moving for string types and copying for scalars.
pub(super) fn builder_copy_expr(param: &GeneratedParam, source: &str) -> String {
    match param.param_type.as_str() {
        "Int" => format!("{} = {}", param.name, source),
        "Float" => format!("{} = {}", param.name, source),
        "Bool" => format!("{} = {}", param.name, source),
        _ => format!("{} = std::move({})", param.name, source),
    }
}

/// Returns the Rust FFI field type for an optional builder parameter in
/// the `#[repr(C)]` options struct.
pub(super) fn ffi_option_value_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" | "Bool" => "i32",
        "Float" => "f64",
        _ => "*const c_char",
    }
}

/// Returns the C field type for an optional builder parameter in the
/// generated `TdxEndpointRequestOptions` struct.
pub(super) fn c_option_value_type(param: &GeneratedParam) -> &'static str {
    match param.param_type.as_str() {
        "Int" => "int32_t",
        "Bool" => "int32_t",
        "Float" => "double",
        _ => "const char*",
    }
}

/// Returns the Rust statement that inserts an optional builder
/// parameter from the FFI options struct into the endpoint args.
pub(super) fn ffi_option_insert_expr(param: &GeneratedParam) -> String {
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

/// Returns `true` if the parameter needs a companion `has_<name>`
/// presence flag in the FFI options struct (scalar types).
pub(super) fn ffi_option_has_flag(param: &GeneratedParam) -> bool {
    matches!(param.param_type.as_str(), "Int" | "Float" | "Bool")
}

// ───────────────────────── SDK method arg declarations ─────────────────────

/// Returns the SDK method argument name for a parameter, mapping the
/// `Symbols` wire type to `symbols` and otherwise using the param name.
pub(super) fn sdk_method_arg_name(param: &GeneratedParam) -> String {
    if param.param_type == "Symbols" {
        "symbols".into()
    } else {
        param.name.clone()
    }
}

/// Returns the `name: Type` declaration for a required Python method
/// argument.
pub(super) fn python_method_arg_decl(param: &GeneratedParam) -> String {
    let name = sdk_method_arg_name(param);
    format!("{name}: {}", python_string_arg_type(param))
}

/// Returns the C++ `const T& name` declaration for a required method
/// argument.
pub(super) fn cpp_method_arg_decl(param: &GeneratedParam) -> String {
    let name = sdk_method_arg_name(param);
    if param.param_type == "Symbols" {
        format!("const std::vector<std::string>& {name}")
    } else {
        format!("const std::string& {name}")
    }
}

// ───────────────────────── Validator arg literals ──────────────────────────

/// Render a single arg string as a Python literal expression, taking the
/// param's wire type into account so `Symbols` becomes a list.
pub(super) fn python_arg_literal(param: &GeneratedParam, value: &str) -> String {
    match param.param_type.as_str() {
        "Symbols" => format!("[\"{value}\"]"),
        _ => format!("\"{value}\""),
    }
}

/// Render a single arg string as a C++ literal expression.
pub(super) fn cpp_arg_literal(param: &GeneratedParam, value: &str) -> String {
    match param.param_type.as_str() {
        "Symbols" => format!("std::vector<std::string>{{\"{value}\"}}"),
        _ => format!("\"{value}\""),
    }
}

/// Look up a builder-bound `GeneratedParam` on the endpoint by name.
fn builder_param_for<'a>(
    endpoint: &'a GeneratedEndpoint,
    name: &str,
) -> Option<&'a GeneratedParam> {
    endpoint
        .params
        .iter()
        .find(|p| p.name == name && !is_method_call_param(p))
}

/// Render a Python kwarg value (`key=value`) for a builder-bound param,
/// preserving the param's wire type (Bool → `True`/`False`, Int/Float → bare,
/// Str → quoted).
pub(super) fn python_builder_kwarg(
    endpoint: &GeneratedEndpoint,
    name: &str,
    value: &str,
) -> Option<String> {
    let param = builder_param_for(endpoint, name)?;
    let literal = match param.param_type.as_str() {
        "Bool" => match value {
            "true" => "True".to_string(),
            "false" => "False".to_string(),
            other => panic!("python_builder_kwarg: bool override {other:?} must be true/false"),
        },
        "Int" | "Float" => value.to_string(),
        _ => format!("\"{value}\""),
    };
    Some(format!("{name}={literal}"))
}

/// Render a C++ `.with_<name>(value)` chained setter for a builder-bound param.
pub(super) fn cpp_builder_setter(
    endpoint: &GeneratedEndpoint,
    name: &str,
    value: &str,
) -> Option<String> {
    let param = builder_param_for(endpoint, name)?;
    let literal = match param.param_type.as_str() {
        "Bool" => value.to_string(),
        "Int" => value.to_string(),
        "Float" => value.to_string(),
        _ => format!("\"{value}\""),
    };
    Some(format!(".with_{name}({literal})"))
}

// ───────────────────────── CLI / validator scaffolding ─────────────────────

/// Returns the CLI command name for an endpoint by stripping its
/// category (or `interest_rate_`) prefix from the endpoint name.
pub(super) fn cli_command_name(endpoint: &GeneratedEndpoint) -> String {
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

/// Returns the CLI invocation tokens for an endpoint under a test mode:
/// the category group, the command name, and the mode's arguments.
pub(super) fn cli_command_tokens_for_mode(
    endpoint: &GeneratedEndpoint,
    mode: &TestMode,
) -> Vec<String> {
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
