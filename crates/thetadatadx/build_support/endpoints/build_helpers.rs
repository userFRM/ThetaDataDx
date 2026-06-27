//! Helpers consumed only by the build-script render path
//! (`render::build_out` + `render::mdds`).
//!
//! The in-house Rust `HistoricalClient` extension impl emitter is the sole
//! consumer here, so these helpers never enter the bin's compile unit.
//! The bin's per-language SDK projection emitters (Python / TypeScript /
//! C++ / FFI / validators) keep their own analogues under
//! `build_support_bin/endpoints/sdk_helpers.rs`.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::model::GeneratedParam;

fn is_symbols_param(param: &GeneratedParam) -> bool {
    param.param_type == "Symbols"
}

// ─────────────────────── Build-side render-name lookup ──────────────────────

/// Per-tick in-house Rust client (`direct`) type-name map keyed by
/// wire-collection plural. Only the field this tree emits (`direct`) is
/// loaded. The parser name lives in `helpers::render_for`; the bin tree's
/// per-language render names live in its own `sdk_helpers`.
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
    direct: String,
}

static DIRECT_MAP: OnceLock<HashMap<String, String>> = OnceLock::new();

fn load_direct_map() -> HashMap<String, String> {
    let schema_path = "tick_schema.toml";
    let raw = std::fs::read_to_string(schema_path)
        .unwrap_or_else(|e| panic!("failed to read {schema_path}: {e}"));
    let parsed: SchemaToml =
        toml::from_str(&raw).unwrap_or_else(|e| panic!("failed to parse {schema_path}: {e}"));
    let mut map = HashMap::new();
    for (_, def) in parsed.types {
        let render = def.render;
        if map
            .insert(render.collection.clone(), render.direct)
            .is_some()
        {
            panic!(
                "duplicate render collection '{}' in tick_schema.toml",
                render.collection
            );
        }
    }
    map
}

fn direct_name(collection: &str) -> &'static str {
    let map = DIRECT_MAP.get_or_init(load_direct_map);
    map.get(collection).map(String::as_str).unwrap_or_else(|| {
        let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
        keys.sort();
        panic!(
            "no direct-name entry for collection '{collection}' in tick_schema.toml; available: {}",
            keys.join(", ")
        )
    })
}

/// Returns the call-site argument expression for a param, dereferencing
/// `Symbols` to the borrowed `&symbol_refs` slice and passing others by name.
pub(super) fn call_arg_name(param: &GeneratedParam) -> String {
    if is_symbols_param(param) {
        "&symbol_refs".into()
    } else {
        param.name.clone()
    }
}

// ───────────────────────── Runtime dispatch getters ─────────────────────────

/// Returns the `EndpointArgs` getter name that extracts a required value
/// of the given wire param type (e.g. `Date` maps to `required_date`).
pub(super) fn required_getter_name(param_type: &str) -> &'static str {
    match param_type {
        "Symbol" => "required_symbol",
        "Symbols" => "required_symbols",
        "Date" => "required_date",
        "Expiration" => "required_expiration",
        "Strike" => "required_strike",
        "Right" => "required_right",
        "Year" => "required_year",
        _ => "required_str",
    }
}

/// Returns the `EndpointArgs` getter name that extracts an optional value
/// of the given wire param type (e.g. `Date` maps to `optional_date`).
pub(super) fn optional_getter_name(param_type: &str) -> &'static str {
    match param_type {
        "Date" => "optional_date",
        "Expiration" => "optional_expiration",
        "Strike" => "optional_strike",
        "Interval" => "optional_interval",
        "Right" => "optional_right",
        "Int" => "optional_int32",
        "Float" => "optional_float64",
        "Bool" => "optional_bool",
        _ => "optional_str",
    }
}

// ───────────────────────── Direct (Rust) client type maps ──────────────────

/// Returns the in-house Rust client method argument name for a param,
/// honoring the `_arg_name` override and falling back to the param name.
pub(super) fn direct_method_arg_name(param: &GeneratedParam) -> String {
    param
        ._arg_name
        .clone()
        .unwrap_or_else(|| param.name.clone())
}

/// Returns the method argument name when the param is a date field
/// (`date`, `start_date`, `end_date`), and `None` otherwise.
pub(super) fn direct_date_arg_name(param: &GeneratedParam) -> Option<String> {
    match param.name.as_str() {
        "date" | "start_date" | "end_date" => Some(direct_method_arg_name(param)),
        _ => None,
    }
}

/// Returns the required-param store kind tag for a param: `str_vec` for
/// `Symbols`, `str` otherwise.
pub(super) fn direct_required_kind(param: &GeneratedParam) -> &'static str {
    if param.param_type == "Symbols" {
        "str_vec"
    } else {
        "str"
    }
}

/// Returns the optional-field storage kind tag and its default-value
/// initializer expression for a param, parsing any declared default.
pub(super) fn direct_optional_kind_and_default(param: &GeneratedParam) -> (&'static str, String) {
    if let Some(default) = param.default.as_deref() {
        return match param.param_type.as_str() {
            "Str" | "Strike" | "Right" | "Interval" | "Venue" | "RateType" | "Version" => {
                ("string", format!("{default:?}.to_string()"))
            }
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

/// Returns the Rust type for an optional builder field (e.g. `Option<i32>`
/// or `String`) based on the param's storage kind.
pub(super) fn direct_optional_rust_type(param: &GeneratedParam) -> &'static str {
    match direct_optional_kind_and_default(param).0 {
        "opt_i32" => "Option<i32>",
        "opt_f64" => "Option<f64>",
        "opt_bool" => "Option<bool>",
        "string" => "String",
        _ => "Option<String>",
    }
}

/// Returns the argument type for an optional builder setter (e.g. `i32`
/// or `&str`) based on the param's storage kind.
pub(super) fn direct_optional_setter_arg_type(param: &GeneratedParam) -> &'static str {
    match direct_optional_kind_and_default(param).0 {
        "opt_i32" => "i32",
        "opt_f64" => "f64",
        "opt_bool" => "bool",
        "string" => "&str",
        _ => "&str",
    }
}

/// Returns the assignment expression an optional builder setter stores from
/// its argument `v` (e.g. `Some(v)` or `v.to_string()`).
pub(super) fn direct_optional_setter_assign_expr(param: &GeneratedParam) -> &'static str {
    match direct_optional_kind_and_default(param).0 {
        "opt_i32" | "opt_f64" | "opt_bool" => "Some(v)",
        "string" => "v.to_string()",
        _ => "Some(v.to_string())",
    }
}

/// Returns the struct field type for a required param: `Vec<String>` for
/// `Symbols`, `String` otherwise.
pub(super) fn direct_required_field_type(param: &GeneratedParam) -> &'static str {
    if param.param_type == "Symbols" {
        "Vec<String>"
    } else {
        "String"
    }
}

/// Returns the method parameter type for a required param:
/// `impl Into<SymbolInput>` for `Symbols`, `&str` otherwise.
pub(super) fn direct_required_param_type(param: &GeneratedParam) -> &'static str {
    if param.param_type == "Symbols" {
        "impl Into<SymbolInput>"
    } else {
        "&str"
    }
}

/// Returns the expression that converts a required method argument into its
/// owned stored form (`Vec<String>` for `Symbols`, `String` otherwise).
pub(super) fn direct_required_store_expr(param: &GeneratedParam) -> String {
    let arg_name = direct_method_arg_name(param);
    if param.param_type == "Symbols" {
        format!("{arg_name}.into().into_vec()")
    } else {
        format!("{arg_name}.to_string()")
    }
}

/// Map a collection return type (e.g. `TradeTicks`) to the per-chunk tick type
/// (e.g. `TradeTick`) used by generated direct streaming builders.
///
/// Routes through the `tick_schema.toml`-loaded `DIRECT_MAP` so every tick
/// collection (Eod, Greeks*, OpenInterest, Calendar, OptionContract, ...)
/// can serve as a streaming-callback element without expanding a hand-written
/// match arm. Panics with the available keys when the collection is missing —
/// a missing TOML row is a build-time bug.
pub(super) fn direct_stream_tick_type(return_type: &str) -> &'static str {
    direct_name(return_type)
}

/// Returns the buffered return type for an endpoint, wrapping the per-tick
/// type for the wire collection in a `Vec` (e.g. `Vec<TradeTick>`).
pub(super) fn direct_return_type(return_type: &str) -> String {
    format!("Vec<{}>", direct_name(return_type))
}
