//! Render-side helpers used only by `sdk_render/*` emitters.
//!
//! These are the param-classification, naming, casing, per-language type
//! tables, FFI / builder option mappers, arg-declaration / arg-literal
//! renderers, and CLI command derivations consumed by the per-language
//! SDK projection emitters.
//!
//! Cross-renderer utilities stay in `helpers.rs`; each render file imports
//! from both modules.

use heck::ToSnakeCase;

use super::super::ticks::table::tick_for_collection;
use super::table::{GeneratedEndpoint, GeneratedParam};

// ───────────────────────── Param classification ────────────────────────────

/// Returns the endpoint's required positional parameters (those passed
/// in the method call rather than chained on the builder).
pub(super) fn method_params(endpoint: &GeneratedEndpoint) -> Vec<&GeneratedParam> {
    endpoint
        .params
        .iter()
        .filter(|param| param.required)
        .collect()
}

/// `true` for snapshot endpoints that can safely keep the Python plain-list
/// fast path.
///
/// Multi-symbol snapshots carry per-row symbol attribution on
/// `ColumnPresence`, so they must return the typed `<TickName>List` wrapper
/// rather than a plain `PyList`; otherwise the per-row `symbols()` vector is
/// dropped before callers can materialise a DataFrame.
pub(super) fn snapshot_returns_plain_pylist(endpoint: &GeneratedEndpoint) -> bool {
    is_snapshot_endpoint(endpoint)
        && !method_params(endpoint)
            .iter()
            .any(|param| param.param_type == "Symbols")
}

/// Returns the endpoint's optional parameters (those chained on the
/// builder rather than passed in the method call).
pub(super) fn builder_params(endpoint: &GeneratedEndpoint) -> Vec<&GeneratedParam> {
    endpoint
        .params
        .iter()
        .filter(|param| !param.required)
        .collect()
}

/// Return `true` if the endpoint is a latency-sensitive single-row (or
/// ≤10-row) snapshot/calendar lookup. Triggers the Python fast path:
/// no `<T>List` wrapper, no `run_blocking` signal-check ticker, bounded
/// `tokio::time::timeout` instead.
///
/// Re-exports the shared predicate (`helpers::is_snapshot_endpoint`) so the
/// Python / docs emitters classify snapshots identically: one definition
/// keyed off the endpoint's table shape, never a hand-curated allowlist. The
/// streaming emitters import the streaming predicates (`endpoint_streams`,
/// `endpoint_streams_repr_c_ticks`) straight from `helpers`.
pub(super) use super::helpers::is_snapshot_endpoint;

// ───────────────────────── Casing ────────────────────────────────────────────

/// Returns the PascalCase form of a single name segment, keeping known
/// initialisms (EOD, OHLC, IV, DTE, NBBO) fully upper.
pub(super) fn pascal_segment(segment: &str) -> String {
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

/// Returns the PascalCase name for an underscore-separated identifier,
/// applying `pascal_segment` to each segment.
pub(super) fn to_pascal_case(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(pascal_segment)
        .collect::<String>()
}

/// Returns the camelCase form of an underscore-separated identifier
/// (PascalCase with a lowercased first letter).
pub(super) fn to_camel_case(value: &str) -> String {
    let pascal = to_pascal_case(value);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

// ───────────────────────── Per-language type tables ─────────────────────────

/// Returns the FFI `#[repr(C)]` array type name for a return type
/// (e.g. `ThetaDataDxStringArray`, `ThetaDataDxTradeTickArray`).
pub(super) fn ffi_array_type(return_type: &str) -> String {
    if return_type == "StringList" {
        return "ThetaDataDxStringArray".into();
    }
    format!("ThetaDataDx{}Array", tick_for_collection(return_type).name)
}

/// Returns the C++ element type for a return type's row vector: the tick
/// type, which the C++ SDK aliases to its C mirror.
pub(super) fn cpp_value_type(return_type: &str) -> String {
    if return_type == "StringList" {
        return "std::string".into();
    }
    tick_for_collection(return_type).name.into()
}

/// Returns the C++ expression that converts the FFI array result for a
/// return type into a `std::vector`, checking the error slot first.
///
/// Forwards to the `detail::check_tick_array<Tick>` template, which consults
/// the error slot before converting: success-empty and failure (e.g. timeout)
/// both return the `{nullptr, 0}` sentinel, and the generated method
/// `thetadatadx_clear_error()`s before the FFI call so a stale error isn't
/// misattributed. The template frees the FFI allocation on every exit path.
/// `StringList`/`OptionContracts` return types take their own early-return
/// path in the emitter and never reach this converter.
pub(super) fn cpp_converter_expr(return_type: &str) -> String {
    let tick = tick_for_collection(return_type).name;
    format!(
        "return detail::check_tick_array<{tick}>(arr, [](auto a){{ return detail::to_vector(a.data, a.len); }}, thetadatadx_{}_array_free);",
        tick.to_snake_case()
    )
}

/// Name of the generated `<TickName>List` pyclass wrapper (e.g.
/// `EodTickList`). Market-data endpoints return `Py<<TickName>List>`
/// directly so callers can chain `.to_polars()` / `.to_arrow()` /
/// `.to_pandas()` / `.to_list()` off the endpoint return value.
///
/// The row class name plus the `List` suffix every pyclass-list type takes.
pub(super) fn python_pyclass_list_class(return_type: &str) -> String {
    format!("{}List", tick_for_collection(return_type).name)
}

/// Name of the generated row `#[pyclass]` (e.g. `EodTick`) for a return
/// type. Snapshot endpoints return a plain `list[<row pyclass>]` rather
/// than the chainable `<TickName>List` wrapper, so the type-stub emitter
/// annotates them as `List[<row pyclass>]`.
pub(super) fn python_pyclass_row_class(return_type: &str) -> String {
    tick_for_collection(return_type).name.into()
}

// ───────────────────────── Builder / FFI option tables ─────────────────────

/// Returns the C++ value type for a builder setter argument of the
/// given parameter.
pub(super) fn builder_value_type_name(param: &GeneratedParam) -> &'static str {
    match param.param_type {
        "Int" => "int32_t",
        "Float" => "double",
        "Bool" => "bool",
        _ => "std::string",
    }
}

/// Returns the C++ assignment expression storing `source` into the
/// builder field, moving for string types and copying for scalars.
pub(super) fn builder_copy_expr(param: &GeneratedParam, source: &str) -> String {
    match param.param_type {
        "Int" => format!("{} = {}", param.name, source),
        "Float" => format!("{} = {}", param.name, source),
        "Bool" => format!("{} = {}", param.name, source),
        _ => format!("{} = std::move({})", param.name, source),
    }
}

/// Returns the C field type for an optional builder parameter in the
/// generated `ThetaDataDxEndpointRequestOptions` struct.
pub(super) fn c_option_value_type(param: &GeneratedParam) -> &'static str {
    match param.param_type {
        "Int" => "int32_t",
        "Bool" => "int32_t",
        "Float" => "double",
        _ => "const char*",
    }
}

/// Returns `true` if the parameter needs a companion `has_<name>`
/// presence flag in the FFI options struct (scalar types).
pub(super) fn ffi_option_has_flag(param: &GeneratedParam) -> bool {
    matches!(param.param_type, "Int" | "Float" | "Bool")
}

// ───────────────────────── SDK method arg declarations ─────────────────────

/// Returns the SDK method argument name for a parameter, mapping the
/// `Symbols` wire type to `symbols` and otherwise using the param name.
pub(super) fn sdk_method_arg_name(param: &GeneratedParam) -> String {
    if param.param_type == "Symbols" {
        "symbols".into()
    } else {
        param.name.to_string()
    }
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
    match param.param_type {
        "Symbols" => format!("[\"{value}\"]"),
        _ => format!("\"{value}\""),
    }
}

/// A method argument as the Python validator passes it: a list for
/// `Symbols`, a string otherwise.
pub(super) fn json_arg(param: &GeneratedParam, value: &str) -> serde_json::Value {
    match param.param_type {
        "Symbols" => serde_json::json!([value]),
        _ => value.into(),
    }
}

/// Render a single arg string as a C++ literal expression.
pub(super) fn cpp_arg_literal(param: &GeneratedParam, value: &str) -> String {
    match param.param_type {
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
        .find(|p| p.name == name && !p.required)
}

/// A builder-bound param's value as the Python validator passes the keyword
/// argument, typed by the param's wire type.
pub(super) fn json_builder_value(
    endpoint: &GeneratedEndpoint,
    name: &str,
    value: &str,
) -> Option<serde_json::Value> {
    fn parse<T: std::str::FromStr>(param: &GeneratedParam, value: &str) -> T {
        value.parse().unwrap_or_else(|_| {
            panic!(
                "{}: fixture {value:?} is not a {}",
                param.name, param.param_type
            )
        })
    }
    let param = builder_param_for(endpoint, name)?;
    Some(match param.param_type {
        "Bool" => parse::<bool>(param, value).into(),
        "Int" => parse::<i64>(param, value).into(),
        "Float" => parse::<f64>(param, value).into(),
        _ => value.into(),
    })
}

/// Render a C++ `.with_<name>(value)` chained setter for a builder-bound param.
pub(super) fn cpp_builder_setter(
    endpoint: &GeneratedEndpoint,
    name: &str,
    value: &str,
) -> Option<String> {
    let param = builder_param_for(endpoint, name)?;
    let literal = match param.param_type {
        "Bool" => value.to_string(),
        "Int" => value.to_string(),
        "Float" => value.to_string(),
        _ => format!("\"{value}\""),
    };
    Some(format!(".with_{name}({literal})"))
}
