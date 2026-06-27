//! Static config accessor emitter.
//!
//! Renders the uniform `thetadatadx_config_{set,get}_*` family from
//! `config_surface.toml`: the FFI Rust entry points
//! (`ffi/src/config_accessors.rs`) and the matching C++ inline `Config`
//! methods (`sdks/cpp/include/config_accessors.hpp.inc`). Divergent
//! accessors (string, enum, Option-widened, policy-aware, validated)
//! stay hand-written in `ffi/src/auth.rs` and `thetadatadx.hpp`.

use std::fmt::Write as _;

use serde::Deserialize;

use super::super::sdk_helpers::{to_camel_case, to_pascal_case};

/// One generated accessor. `set` produces a unit-returning setter via
/// `require_config_mut!`; `get` produces an `i32`-returning getter with
/// the dual null-check.
///
/// Beyond the uniform scalar `set` / `get`, five carve-out KINDs share
/// one reusable template each, parameterised purely by TOML data (a
/// variant table, a limit-field name, an ABI shape) — never by a
/// per-field emitter branch:
///
/// * `policy_limit` — a `ReconnectAttemptLimits` field reached through
///   `ReconnectPolicy::Auto`; the setter mutates only under `Auto`, the
///   getter falls back to `ReconnectAttemptLimits::default()`.
///
/// The enum / string / Option language-binding surfaces stay
/// hand-written: their bodies diverge per binding (int codes vs string
/// labels vs `(has_value, n)` vs `std::optional`) in ways that are not a
/// single reusable template. The FFI layer is where the uniformity
/// lives, so the carve-out KINDs generate the FFI entry points and the
/// thin C++ pass-through wrappers; the richer per-language idioms stay
/// hand-written.
#[derive(Debug, Deserialize)]
struct Accessor {
    symbol: String,
    kind: String,
    param: String,
    abi_type: String,
    #[serde(default)]
    path: String,
    /// `ms` / `secs` Duration wrap (setter) or `.as_secs()` / `.as_millis()`
    /// read (getter). `ms` getters emit the saturating `u64::try_from`
    /// millisecond read used by the `retry.*` `Duration` fields.
    #[serde(default)]
    duration_unit: Option<String>,
    /// Per-site `// SAFETY:` block for the getter write, when it differs
    /// from the canonical one-liner. Carries its own indentation.
    #[serde(default)]
    out_safety: Option<String>,
    /// Carve-out template selector, orthogonal to `kind` (which stays the
    /// set/get discriminator). Absent = the uniform scalar / duration
    /// accessor. `"policy_limit"` reaches a `ReconnectAttemptLimits` field
    /// through `ReconnectPolicy::Auto`; `"string"` is a UTF-8 `String`
    /// field with the owned-`*mut c_char` getter convention.
    #[serde(default)]
    shape: Option<String>,
    /// `policy_limit` only: the `ReconnectAttemptLimits` field this
    /// accessor reads / writes (e.g. `max_attempts`, `stable_window`).
    #[serde(default)]
    limit_field: Option<String>,
    /// `string` setter only: route the value through a `&mut self` method
    /// (e.g. `set_historical_host`) instead of a direct `path` assignment.
    #[serde(default)]
    setter_call: Option<String>,
    /// `string` getter only: read the value through a `&self` method
    /// (e.g. `historical_host`) instead of a direct `path` read.
    #[serde(default)]
    getter_call: Option<String>,
    /// `enum` only: the FFI-side enum type (`thetadatadx::`-prefixed,
    /// e.g. `thetadatadx::StreamingFlushMode`) the int code maps to / from.
    #[serde(default)]
    enum_type: Option<String>,
    /// `enum` only: the binding-side enum type (`config::`-prefixed) whose
    /// `parse` / `as_str` the Python / TypeScript string surfaces use.
    #[serde(default)]
    enum_core: Option<String>,
    /// `enum` setter only: the trailing `expected …` clause of the
    /// `INVALID_PARAMETER` rejection message (the int-domain list spelled
    /// with its enum-specific oxford-comma grammar).
    #[serde(default)]
    enum_expected: Option<String>,
    /// `enum` setter only: typed code for the null-handle leaf. `"config"`
    /// (default) pins `THETADATADX_ERR_CONFIG`; `"other"` falls back to the
    /// untyped `set_error` (`THETADATADX_ERR_OTHER`). Selects per setter so
    /// the generated null-handle classification stays byte-stable.
    #[serde(default)]
    null_err: Option<String>,
    /// `enum` / `option` setter only: the Python `ValueError` body for a
    /// rejected value, verbatim (the per-accessor prose stays byte-stable
    /// across bindings). `{…}` placeholders interpolate the setter param /
    /// the matched value.
    #[serde(default)]
    py_err: Option<String>,
    /// `enum` / `option` setter only: the TypeScript `invalid_parameter_err`
    /// body for a rejected value, verbatim.
    #[serde(default)]
    ts_err: Option<String>,
    /// `enum` getter only: the int the `#[non_exhaustive]` `_` arm maps a
    /// future/unknown core variant to. Absent = the last listed variant's
    /// int. Every known variant is still matched explicitly; the `_` arm
    /// only ever fires for a variant added to the core after this table.
    #[serde(default)]
    catch_all_int: Option<i32>,
    /// `enum` setter only: lowercase the binding-string param before
    /// `parse` + the rejection-message interpolation, so the diagnostic
    /// echoes the normalized value (`got "bogus"`, not `"BOGUS"`).
    #[serde(default)]
    lowercase_err: bool,
    /// `enum` only: the int ↔ variant ↔ lowercase-label bijection. Drives
    /// the FFI `match` both ways and the variant arms the C++ doc and the
    /// Python / TypeScript surfaces reference.
    #[serde(default)]
    variant: Vec<EnumVariant>,
    doc: String,
    cpp_doc: String,
    /// Verbatim PyO3 `#[setter]`/`#[getter]` doc block (no `///` prefix).
    py_doc: String,
    /// Verbatim NAPI doc block (no `///` prefix) — produces `index.d.ts`.
    ts_doc: String,
    /// Setter argument name on the Python surface (`ms`/`secs`/`n`/…). Absent
    /// on getter rows.
    #[serde(default)]
    py_param: Option<String>,
    /// Setter argument name on the TypeScript surface — usually equal to
    /// `py_param`, but a few `*_secs` knobs spell it `ms` for back-compat.
    #[serde(default)]
    ts_param: Option<String>,
}

/// One `int` code of an `enum` accessor and its core-variant identifier.
#[derive(Debug, Deserialize)]
struct EnumVariant {
    /// The wire/ABI integer code.
    int: i32,
    /// The variant identifier without the enum path (e.g. `Batched`).
    rust: String,
}

#[derive(Debug, Deserialize)]
struct ConfigSurface {
    accessor: Vec<Accessor>,
}

fn load() -> Result<Vec<Accessor>, Box<dyn std::error::Error>> {
    let spec_str = std::fs::read_to_string("config_surface.toml")?;
    let spec: ConfigSurface = toml::from_str(&spec_str)?;
    for a in &spec.accessor {
        assert_unsupported_shape(a);
    }
    Ok(spec.accessor)
}

/// Fail the build on a TOML row whose shape the templates do not handle,
/// rather than emit Rust that silently miscompiles. These branches are
/// reachable only by a future field, so the guard is the contract that
/// such a field must extend the emitter first.
///
/// * `policy_limit` + `duration_unit = "ms"` — the policy read only
///   wraps `secs`; an `ms` limit would write a raw `Duration` into a
///   `u64` out-parameter.
/// * `bool`-typed `option` — the `None` sentinel is the integer `0`,
///   which is not a `bool` (Rust) / triggers `-Wbool-conversion` (C++).
/// * `Option<Duration>` (`option` + `duration_unit`) — the option
///   branch ignores `duration_unit`, so it would move a raw `u64` into
///   an `Option<Duration>` and read a `Duration` into a `u64` out-param.
fn assert_unsupported_shape(a: &Accessor) {
    assert!(
        !(a.shape.as_deref() == Some("policy_limit") && a.duration_unit.as_deref() == Some("ms")),
        "config_surface: policy_limit '{}' has duration_unit=\"ms\"; the policy-limit getter only wraps secs — extend ffi_get_read_expr / py_get_read / the TS policy_limit arm before adding an ms limit",
        a.symbol
    );
    assert!(
        !(a.kind == "option" && a.abi_type == "bool"),
        "config_surface: option '{}' is bool-typed; the None sentinel `0` is not a bool — give the option getter a Default-based sentinel before adding a bool option",
        a.symbol
    );
    assert!(
        !(a.kind == "option" && a.duration_unit.is_some()),
        "config_surface: option '{}' carries duration_unit; the option branch ignores it and would mishandle Option<Duration> — add the Duration wrap/unwrap before adding one",
        a.symbol
    );
}

const CFG_SAFETY: &str = "        // SAFETY: config is a non-null `*const ThetaDataDxConfig` returned by `thetadatadx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.";
const OUT_SAFETY: &str = "        // SAFETY: out pointer checked non-null above; the FFI contract pins the storage for the call duration and forbids concurrent calls on the same handle.";

/// Append `text` as `///` Rust doc lines, emitting a bare `///` for a
/// blank line so the round-trip is byte-exact (no trailing space).
fn rust_doc(buf: &mut String, text: &str) {
    for line in text.trim_matches('\n').split('\n') {
        if line.is_empty() {
            buf.push_str("///\n");
        } else {
            writeln!(buf, "/// {line}").unwrap();
        }
    }
}

/// Append `text` as indented `///` C++ doc lines under `indent`.
fn cpp_doc(buf: &mut String, indent: &str, text: &str) {
    for line in text.trim_matches('\n').split('\n') {
        if line.is_empty() {
            writeln!(buf, "{indent}///").unwrap();
        } else {
            writeln!(buf, "{indent}/// {line}").unwrap();
        }
    }
}

/// The reconnect-limit getter fallback: `ReconnectPolicy::Manual` /
/// `Custom` carry no limits, so the getter reports the `Auto` defaults
/// (the per-class budgets only apply under `Auto`, and the setters are
/// no-ops there too).
const LIMIT_DEFAULT: &str = "thetadatadx::ReconnectAttemptLimits::default()";

/// The `ReconnectAttemptLimits` field a `policy_limit` accessor touches.
fn limit_field(a: &Accessor) -> &str {
    a.limit_field
        .as_deref()
        .expect("policy_limit needs limit_field")
}

/// `true` when this row is a setter (its symbol carries the `set_`
/// prefix). The scalar / string carve-outs key off `kind == "set"`, but
/// the `enum` / `option` kinds use one `kind` value for both directions,
/// so they branch on the symbol instead.
fn is_setter(a: &Accessor) -> bool {
    a.symbol.contains("_config_set_")
}

/// The `policy_limit` getter read: a two-arm `match` on
/// `<receiver>.reconnect.policy` reading `limits.<field><suffix>`, with
/// the `Auto`-defaults fallback. `receiver` is the binding's config
/// expression (`config.inner` / `guard`) and `limits_default` its
/// `ReconnectAttemptLimits::default()` spelling.
fn policy_get_match(
    receiver: &str,
    policy_ty: &str,
    limits_default: &str,
    field: &str,
    suffix: &str,
) -> String {
    format!(
        "match &{receiver}.reconnect.policy {{\n            {policy_ty}::Auto(limits) => limits.{field}{suffix},\n            _ => {limits_default}.{field}{suffix},\n        }}"
    )
}

/// The `enum` setter null-handle leaf. `null_err = "other"` reproduces
/// the untyped `set_error` (`ERR_OTHER`); the default pins `ERR_CONFIG`.
/// Both spellings carry the identical `"{sym}: config handle is null"`
/// text — only the typed code differs.
fn ffi_enum_null_leaf(a: &Accessor) -> String {
    match a.null_err.as_deref() {
        Some("other") => format!("set_error(\"{sym}: config handle is null\");", sym = a.symbol),
        Some("config") | None => format!(
            "crate::error::set_error_with_code(\n                \"{sym}: config handle is null\",\n                crate::error::THETADATADX_ERR_CONFIG,\n            );",
            sym = a.symbol,
        ),
        Some(other) => panic!("config_surface: unknown null_err '{other}' on {}", a.symbol),
    }
}

/// The `enum` setter int→variant decode `match` (with the typed-error
/// `_` arm), emitted into the FFI setter body. `set_error_with_code`
/// keeps the rejected-int path on `INVALID_PARAMETER` across every
/// binding.
fn ffi_enum_set_match(a: &Accessor) -> String {
    let ty = a.enum_type.as_deref().expect("enum needs enum_type");
    let mut arms = String::new();
    for v in &a.variant {
        writeln!(arms, "            {} => {ty}::{},", v.int, v.rust).unwrap();
    }
    let expected = a
        .enum_expected
        .as_deref()
        .expect("enum setter needs enum_expected");
    format!(
        "let value = match {p} {{\n{arms}            other => {{\n                crate::error::set_error_with_code(\n                    &format!(\n                        \"{sym}: invalid {p} {{other}}; {expected}\"\n                    ),\n                    crate::error::THETADATADX_ERR_INVALID_PARAMETER,\n                );\n                return -1;\n            }}\n        }};",
        p = a.param, sym = a.symbol,
    )
}

/// The `enum` getter variant→int decode `match`, emitted into the FFI
/// getter body. Every known variant is matched explicitly; a trailing
/// `_ => <catch_all_int>` covers the `#[non_exhaustive]` core enum
/// (mandatory across the crate boundary) and pins the int a future
/// variant decodes to, instead of silently inheriting the last listed
/// variant's code.
fn ffi_enum_get_match(a: &Accessor) -> String {
    let ty = a.enum_type.as_deref().expect("enum needs enum_type");
    let mut arms = String::new();
    for v in &a.variant {
        writeln!(arms, "            {ty}::{} => {},", v.rust, v.int).unwrap();
    }
    let catch_all = a
        .catch_all_int
        .unwrap_or_else(|| a.variant.last().expect("enum needs variants").int);
    writeln!(arms, "            _ => {catch_all},").unwrap();
    format!(
        "let value = match {recv}.{path} {{\n{arms}        }};",
        recv = "config.inner",
        path = a.path,
    )
}

/// The Rust read expression for one getter (the value written into the
/// out-parameter), keyed by KIND. Scalar / duration reads come straight
/// off `config.inner.<path>`; the `policy_limit` read walks
/// `reconnect.policy` and falls back to the `Auto` defaults.
fn ffi_get_read_expr(a: &Accessor) -> String {
    if a.shape.as_deref() == Some("policy_limit") {
        let field = limit_field(a);
        let suffix = if a.duration_unit.as_deref() == Some("secs") {
            ".as_secs()"
        } else {
            ""
        };
        return policy_get_match(
            "config.inner",
            "thetadatadx::ReconnectPolicy",
            LIMIT_DEFAULT,
            field,
            suffix,
        );
    }
    match a.duration_unit.as_deref() {
        // The `retry.*` `Duration` fields cross the ABI as `u64` ms via a
        // saturating `as_millis` clamp; `secs` fields read `.as_secs()`.
        Some("ms") => format!(
            "u64::try_from(config.inner.{}.as_millis()).unwrap_or(u64::MAX)",
            a.path
        ),
        Some("secs") => format!("config.inner.{}.as_secs()", a.path),
        _ => format!("config.inner.{}", a.path),
    }
}

/// The FFI Rust file: one `#[no_mangle]` entry point per accessor.
pub(super) fn render_ffi_config_accessors() -> Result<String, Box<dyn std::error::Error>> {
    let accessors = load()?;
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from config_surface.toml\n\n",
    );
    for a in &accessors {
        rust_doc(&mut out, &a.doc);
        out.push_str("#[no_mangle]\n");
        match (a.shape.as_deref(), a.kind.as_str()) {
            (_, "enum") if is_setter(a) => {
                // `i32` code → core variant. The null-handle leaf is typed
                // per setter (`null_err`); a rejected int carries
                // `ERR_INVALID_PARAMETER` (unified across all four enums).
                write!(
                    out,
                    "pub unsafe extern \"C\" fn {sym}(\n    config: *mut ThetaDataDxConfig,\n    {p}: {ty},\n) -> i32 {{\n    ffi_boundary!(-1, {{\n        if config.is_null() {{\n            {null_leaf}\n            return -1;\n        }}\n        {decode}\n        // SAFETY: config is a non-null pointer returned by `thetadatadx_config_*` and not yet freed; `&mut *` produces a unique reference valid for the call duration because the caller owns the Box and the FFI contract forbids concurrent calls on the same handle.\n        let config = unsafe {{ &mut *config }};\n        config.inner.{path} = value;\n        0\n    }})\n}}\n\n",
                    sym = a.symbol, p = a.param, ty = a.abi_type,
                    null_leaf = ffi_enum_null_leaf(a),
                    decode = ffi_enum_set_match(a), path = a.path,
                )?;
            }
            (_, "enum") => {
                // Core variant → `i32` code. Dual null-check, then the
                // variant `match` folded onto the canonical out-write.
                write!(
                    out,
                    "pub unsafe extern \"C\" fn {sym}(\n    config: *const ThetaDataDxConfig,\n    {p}: *mut {ty},\n) -> i32 {{\n    ffi_boundary!(-1, {{\n        if config.is_null() || {p}.is_null() {{\n            set_error(\"config or out-parameter pointer is null\");\n            return -1;\n        }}\n{cfg}\n        let config = unsafe {{ &*config }};\n        {decode}\n{outs}\n        unsafe {{\n            *{p} = value;\n        }}\n        0\n    }})\n}}\n\n",
                    sym = a.symbol, p = a.param, ty = a.abi_type,
                    cfg = CFG_SAFETY, decode = ffi_enum_get_match(a), outs = OUT_SAFETY,
                )?;
            }
            (_, "option") if is_setter(a) => {
                // `(has_value, value)` widened ABI → `Option<T>`; the
                // `None` sentinel survives the C boundary.
                write!(
                    out,
                    "pub unsafe extern \"C\" fn {sym}(\n    config: *mut ThetaDataDxConfig,\n    has_value: bool,\n    {p}: {ty},\n) -> i32 {{\n    ffi_boundary!(-1, {{\n        if config.is_null() {{\n            set_error(\"config handle is null\");\n            return -1;\n        }}\n        // SAFETY: config is a non-null pointer returned by `thetadatadx_config_*` and not yet freed; `&mut *` produces a unique reference valid for the call duration because the caller owns the Box and the FFI contract forbids concurrent calls on the same handle.\n        let config = unsafe {{ &mut *config }};\n        config.inner.{path} = if has_value {{ Some({p}) }} else {{ None }};\n        0\n    }})\n}}\n\n",
                    sym = a.symbol, p = a.param, ty = a.abi_type, path = a.path,
                )?;
            }
            (_, "option") => {
                // `Option<T>` → `(has_value, value)`; `None` writes
                // `(false, 0)`.
                write!(
                    out,
                    "pub unsafe extern \"C\" fn {sym}(\n    config: *const ThetaDataDxConfig,\n    out_has_value: *mut bool,\n    {p}: *mut {ty},\n) -> i32 {{\n    ffi_boundary!(-1, {{\n        if config.is_null() || out_has_value.is_null() || {p}.is_null() {{\n            set_error(\"config or out-parameter pointer is null\");\n            return -1;\n        }}\n{cfg}\n        let config = unsafe {{ &*config }};\n        let (has_value, value) = match config.inner.{path} {{\n            Some(v) => (true, v),\n            None => (false, 0),\n        }};\n        // SAFETY: out_has_value / {p} null-checked above; caller pins the storage they point at for the call duration.\n        unsafe {{\n            *out_has_value = has_value;\n            *{p} = value;\n        }}\n        0\n    }})\n}}\n\n",
                    sym = a.symbol, p = a.param, ty = a.abi_type, path = a.path,
                    cfg = CFG_SAFETY,
                )?;
            }
            (Some("string"), "set") => {
                // `*const c_char` → validated UTF-8 → `String`. `path`
                // assigns the field directly; `setter_call` routes the
                // value through a `&mut self` method instead.
                let assign = match a.setter_call.as_deref() {
                    Some(call) => format!("config.inner.{call}({p}.to_string());", p = a.param),
                    None => format!(
                        "config.inner.{path} = {p}.to_string();",
                        path = a.path,
                        p = a.param
                    ),
                };
                // Diagnostics name the logical field (`nexus_url`,
                // `historical_host`), not the local C param (`url`, `host`).
                write!(
                    out,
                    "pub unsafe extern \"C\" fn {sym}(\n    config: *mut ThetaDataDxConfig,\n    {p}: *const c_char,\n) -> i32 {{\n    ffi_boundary!(-1, {{\n        if config.is_null() {{\n            set_error(\"config handle is null\");\n            return -1;\n        }}\n        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.\n        let {p} = match unsafe {{ cstr_to_str({p}) }} {{\n            Ok(Some(s)) => s,\n            Ok(None) => {{\n                set_error(\"{field} is null\");\n                return -1;\n            }}\n            Err(e) => {{\n                set_error(&format!(\"{field} is not valid UTF-8: {{e}}\"));\n                return -1;\n            }}\n        }};\n        // SAFETY: config is a non-null pointer returned by thetadatadx_config_* and not yet freed.\n        let config = unsafe {{ &mut *config }};\n        {assign}\n        0\n    }})\n}}\n\n",
                    sym = a.symbol, p = a.param, field = field_name(a), assign = assign,
                )?;
            }
            (Some("string"), _) => {
                // Heap-owned `*mut c_char` the caller frees with
                // `thetadatadx_string_free`; rejects an interior NUL.
                let read = match a.getter_call.as_deref() {
                    Some(call) => format!("config.inner.{call}()"),
                    None => format!("config.inner.{}.as_str()", a.path),
                };
                write!(
                    out,
                    "pub unsafe extern \"C\" fn {sym}(\n    config: *const ThetaDataDxConfig,\n) -> *mut c_char {{\n    ffi_boundary!(ptr::null_mut(), {{\n        if config.is_null() {{\n            set_error(\"config handle is null\");\n            return ptr::null_mut();\n        }}\n        // SAFETY: config is a non-null `*const ThetaDataDxConfig` returned by `thetadatadx_config_*` and not yet freed; `&*` produces a shared reference valid for the call duration.\n        let config = unsafe {{ &*config }};\n        match std::ffi::CString::new({read}) {{\n            Ok(c) => c.into_raw(),\n            Err(e) => {{\n                set_error(&format!(\"{field} contains an interior NUL: {{e}}\"));\n                ptr::null_mut()\n            }}\n        }}\n    }})\n}}\n\n",
                    sym = a.symbol, read = read, field = field_name(a),
                )?;
            }
            (_, "set") => {
                // Scalar / duration / `policy_limit` setter.
                let rhs = match a.duration_unit.as_deref() {
                    Some("ms") => format!("std::time::Duration::from_millis({})", a.param),
                    Some("secs") => format!("std::time::Duration::from_secs({})", a.param),
                    _ => a.param.clone(),
                };
                let body = if a.shape.as_deref() == Some("policy_limit") {
                    let field = limit_field(a);
                    format!(
                        "        let config = require_config_mut!(config);\n        if let thetadatadx::ReconnectPolicy::Auto(ref mut limits) = config.inner.reconnect.policy {{\n            limits.{field} = {rhs};\n        }}"
                    )
                } else {
                    format!(
                        "        let config = require_config_mut!(config);\n        config.inner.{path} = {rhs};",
                        path = a.path
                    )
                };
                write!(
                    out,
                    "pub unsafe extern \"C\" fn {sym}(\n    config: *mut ThetaDataDxConfig,\n    {p}: {ty},\n) {{\n    ffi_boundary!((), {{\n{body}\n    }})\n}}\n\n",
                    sym = a.symbol, p = a.param, ty = a.abi_type, body = body,
                )?;
            }
            (_, _) => {
                // Scalar / duration / `policy_limit` getter: one
                // dual-null-check skeleton, the read expression varies.
                // The `policy_limit` read is a multi-arm `match`, so it is
                // bound to a `let value` before the `unsafe` write to keep
                // the output rustfmt-stable (matching the hand-written
                // scalar-via-local shape); scalar reads inline directly.
                let out_safety = a
                    .out_safety
                    .as_deref()
                    .map(|s| s.trim_end_matches('\n'))
                    .unwrap_or(OUT_SAFETY);
                let (pre, read) = if a.shape.as_deref() == Some("policy_limit") {
                    (
                        format!("        let value = {};\n", ffi_get_read_expr(a)),
                        "value".to_string(),
                    )
                } else {
                    (String::new(), ffi_get_read_expr(a))
                };
                write!(
                    out,
                    "pub unsafe extern \"C\" fn {sym}(\n    config: *const ThetaDataDxConfig,\n    {p}: *mut {ty},\n) -> i32 {{\n    ffi_boundary!(-1, {{\n        if config.is_null() || {p}.is_null() {{\n            set_error(\"config or out-parameter pointer is null\");\n            return -1;\n        }}\n{cfg}\n        let config = unsafe {{ &*config }};\n{pre}{outs}\n        unsafe {{\n            *{p} = {read};\n        }}\n        0\n    }})\n}}\n\n",
                    sym = a.symbol, p = a.param, ty = a.abi_type,
                    cfg = CFG_SAFETY, pre = pre, outs = out_safety, read = read,
                )?;
            }
        }
    }
    Ok(out)
}

/// Map an FFI integer type to the C++ spelling used on the `Config`
/// method surface.
fn cpp_type(abi: &str) -> &'static str {
    match abi {
        "u32" => "std::uint32_t",
        "u64" => "std::uint64_t",
        "u16" => "std::uint16_t",
        "usize" => "std::size_t",
        "bool" => "bool",
        other => panic!("config_surface: unmapped abi_type '{other}'"),
    }
}

/// The C++ include: one inline `Config` method per accessor, spliced
/// into the class body in `thetadatadx.hpp`.
pub(super) fn render_cpp_config_accessors() -> Result<String, Box<dyn std::error::Error>> {
    let accessors = load()?;
    let mut out = String::new();
    out.push_str(
        "/* @generated DO NOT EDIT — regenerated by build.rs from config_surface.toml */\n",
    );
    for a in &accessors {
        let method = a.symbol.strip_prefix("thetadatadx_config_").unwrap();
        out.push('\n');
        cpp_doc(&mut out, "    ", &a.cpp_doc);
        match (a.shape.as_deref(), a.kind.as_str()) {
            (_, "enum") if is_setter(a) => {
                // Int passthrough; the FFI rejects an out-of-domain code
                // or null handle with a nonzero code routed through the
                // typed error leaf.
                write!(
                    out,
                    "    void {method}(int {p}) {{\n        if ({sym}(handle_.get(), {p}) != 0) {{\n            detail::throw_last_ffi_error();\n        }}\n    }}\n",
                    method = method, p = a.param, sym = a.symbol,
                )?;
            }
            (_, "enum") => {
                // Int passthrough getter; the FFI writes the default code
                // on a null handle.
                write!(
                    out,
                    "    int {method}() const {{\n        int32_t out{{}};\n        {sym}(handle_.get(), &out);\n        return out;\n    }}\n",
                    method = method, sym = a.symbol,
                )?;
            }
            (_, "option") if is_setter(a) => {
                // `std::optional<T>` → `(has_value, value)`; the FFI
                // rejects a null handle through the typed error leaf.
                let ty = cpp_type(&a.abi_type);
                write!(
                    out,
                    "    void {method}(std::optional<{ty}> {p}) {{\n        const bool has_value = {p}.has_value();\n        const {ty} arg = {p}.value_or(0);\n        if ({sym}(handle_.get(), has_value, arg) != 0) {{\n            detail::throw_last_ffi_error();\n        }}\n    }}\n",
                    method = method, ty = ty, p = a.param, sym = a.symbol,
                )?;
            }
            (_, "option") => {
                // `(has_value, value)` → `std::optional<T>`; `std::nullopt`
                // for the `None` sentinel.
                let ty = cpp_type(&a.abi_type);
                let val = a.param.strip_prefix("out_").unwrap_or(&a.param);
                write!(
                    out,
                    "    std::optional<{ty}> {method}() const {{\n        bool has_value = false;\n        {ty} {val} = 0;\n        if ({sym}(handle_.get(), &has_value, &{val}) != 0) {{\n            detail::throw_last_ffi_error();\n        }}\n        return has_value ? std::optional<{ty}>{{{val}}} : std::nullopt;\n    }}\n",
                    method = method, ty = ty, val = val, sym = a.symbol,
                )?;
            }
            (Some("string"), "set") => {
                // `std::string` → `const char*`; the FFI rejects a null
                // handle or non-UTF-8 with a nonzero code routed through
                // the typed error leaf.
                write!(
                    out,
                    "    void {method}(const std::string& {p}) {{\n        if ({sym}(handle_.get(), {p}.c_str()) != 0) {{\n            detail::throw_last_ffi_error();\n        }}\n    }}\n",
                    method = method, p = a.param, sym = a.symbol,
                )?;
            }
            (Some("string"), _) => {
                // Owned `char*` adopted by `FfiString` (auto-freed);
                // empty string on a null handle or interior-NUL value.
                write!(
                    out,
                    "    std::string {method}() const {{\n        detail::FfiString s({sym}(handle_.get()));\n        return s.str();\n    }}\n",
                    method = method, sym = a.symbol,
                )?;
            }
            (_, "set") => {
                // Scalar / duration / `policy_limit` pass-through setter.
                let ty = cpp_type(&a.abi_type);
                write!(
                    out,
                    "    void {method}({ty} {p}) {{\n        {sym}(handle_.get(), {p});\n    }}\n",
                    method = method,
                    ty = ty,
                    p = a.param,
                    sym = a.symbol,
                )?;
            }
            (_, _) => {
                // Scalar / duration / `policy_limit` pass-through getter.
                let ty = cpp_type(&a.abi_type);
                write!(
                    out,
                    "    {ty} {method}() const {{\n        {ty} out{{}};\n        {sym}(handle_.get(), &out);\n        return out;\n    }}\n",
                    ty = ty, method = method, sym = a.symbol,
                )?;
            }
        }
    }
    Ok(out)
}

/// The accessor's logical field name (the `symbol` minus the
/// `thetadatadx_config_{set,get}_` prefix), e.g. `reconnect_wait_ms`.
fn field_name(a: &Accessor) -> &str {
    a.symbol
        .strip_prefix("thetadatadx_config_set_")
        .or_else(|| a.symbol.strip_prefix("thetadatadx_config_get_"))
        .expect("config symbol must carry the canonical set_/get_ prefix")
}

/// Embed a raw error message (carrying literal `"`) inside a Rust
/// `format!("…")` literal: escape the double quotes; the `{…}` braces
/// stay verbatim as format placeholders.
fn rust_lit(raw: &str) -> String {
    raw.replace('"', "\\\"")
}

/// The TypeScript `u32` setter validator for an accessor: the napi
/// boundary takes the argument as `f64` (V8 `ToUint32` on a bare `u32`
/// silently wraps `-1`/`2**32` and truncates `1.5`) and routes it through
/// a finite/whole/range check. A burst-size or attempt-budget knob — a
/// value the core rejects at `0` — additionally floors at `1`; every
/// other `u32` knob (iteration counts, keepalive retries) allows `0`.
///
/// The `_attempts` suffix and the `replay_burst_size` field are the
/// min-1 set; both spellings come straight off the field name so a new
/// attempt-budget row inherits the floor without a per-row flag.
fn ts_u32_validator(field: &str) -> &'static str {
    if field.ends_with("_attempts") || field == "reconnect_replay_burst_size" {
        "validate_u32_arg_min1"
    } else {
        "validate_u32_arg"
    }
}

/// The Rust scalar type each abi maps to on the PyO3 surface.
fn py_type(abi: &str) -> &'static str {
    match abi {
        "u64" => "u64",
        "u32" => "u32",
        "u16" => "u16",
        "usize" => "usize",
        "bool" => "bool",
        other => panic!("config_surface: unmapped abi_type '{other}'"),
    }
}

/// Append `text` as `///` doc lines indented under `indent` (no `///`
/// prefix in the source; a blank line emits a bare `///`).
fn indented_doc(buf: &mut String, indent: &str, text: &str) {
    for line in text.trim_matches('\n').split('\n') {
        if line.is_empty() {
            writeln!(buf, "{indent}///").unwrap();
        } else {
            writeln!(buf, "{indent}/// {line}").unwrap();
        }
    }
}

/// The Python SDK config accessors: a `#[pymethods] impl Config` block
/// holding the mechanical scalar setter/getter pairs that mirror the FFI
/// `config_surface.toml` rows. Divergent accessors (enum, string,
/// `Option`-widened, policy-aware) stay hand-written in `lib.rs`.
pub(super) fn render_python_config_accessors() -> Result<String, Box<dyn std::error::Error>> {
    const LOCK: &str =
        "        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());";
    const RLOCK: &str = "        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());";
    let accessors = load()?;
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from config_surface.toml\n//\n// Mechanical scalar config accessors for the Python `Config` pyclass.\n// `include!`-d into `lib.rs`, which keeps the divergent accessors\n// (enum-parsing, string, `Option`-widened, policy-aware) hand-written.\n\n#[pymethods]\nimpl Config {\n",
    );
    for a in &accessors {
        let field = field_name(a);
        indented_doc(&mut out, "    ", &a.py_doc);
        match (a.shape.as_deref(), a.kind.as_str()) {
            (_, "enum") if is_setter(a) => {
                // Lowercase string → core variant via `parse`; the same
                // path for every enum (no inline per-enum match).
                // `lowercase_err` shadows the param so the rejection
                // message echoes the normalized value.
                let param = a.py_param.as_deref().expect("setter row needs py_param");
                let core = a.enum_core.as_deref().expect("enum needs enum_core");
                let err = rust_lit(a.py_err.as_deref().expect("enum setter needs py_err"));
                // The lowercased shadow is an owned `String`; `parse` takes
                // `&str`, so reborrow it. The raw-param path passes the
                // `&str` arg through unchanged.
                let (lower, parse_arg) = if a.lowercase_err {
                    (
                        format!("        let {param} = {param}.to_ascii_lowercase();\n"),
                        format!("&{param}"),
                    )
                } else {
                    (String::new(), param.to_string())
                };
                write!(
                    out,
                    "    #[setter]\n    fn set_{field}(&self, {param}: &str) -> PyResult<()> {{\n{lower}        let parsed = {core}::parse({parse_arg}).ok_or_else(|| {{\n            PyValueError::new_err(format!(\n                \"{err}\"\n            ))\n        }})?;\n{LOCK}\n        guard.{path} = parsed;\n        Ok(())\n    }}\n\n",
                    field = field, param = param, core = core, err = err, path = a.path,
                    lower = lower, parse_arg = parse_arg,
                )?;
            }
            (_, "enum") => {
                // Core variant → lowercase string via `as_str` (uniform).
                write!(
                    out,
                    "    #[getter]\n    fn get_{field}(&self) -> &'static str {{\n{RLOCK}\n        guard.{path}.as_str()\n    }}\n\n",
                    field = field, path = a.path,
                )?;
            }
            (_, "option") if is_setter(a) => {
                let param = a.py_param.as_deref().expect("setter row needs py_param");
                match a.abi_type.as_str() {
                    // u16 fields narrow from a Python `int`; a value outside
                    // `0..=65535` raises `ValueError`.
                    "u16" => {
                        let err =
                            rust_lit(a.py_err.as_deref().expect("option u16 setter needs py_err"));
                        write!(
                            out,
                            "    #[setter]\n    fn set_{field}(&self, {param}: Option<u32>) -> PyResult<()> {{\n        let resolved = match {param} {{\n            Some(v) => Some(u16::try_from(v).map_err(|_| {{\n                PyValueError::new_err(format!(\"{err}\"))\n            }})?),\n            None => None,\n        }};\n{LOCK}\n        guard.{path} = resolved;\n        Ok(())\n    }}\n\n",
                            field = field, param = param, err = err, path = a.path,
                        )?;
                    }
                    // Wider fields take the matching `Option<T>` verbatim.
                    other => {
                        let ty = py_type(other);
                        write!(
                            out,
                            "    #[setter]\n    fn set_{field}(&self, {param}: Option<{ty}>) {{\n{LOCK}\n        guard.{path} = {param};\n    }}\n\n",
                            field = field, param = param, ty = ty, path = a.path,
                        )?;
                    }
                }
            }
            (_, "option") => {
                let ty = py_type(&a.abi_type);
                write!(
                    out,
                    "    #[getter]\n    fn get_{field}(&self) -> Option<{ty}> {{\n{RLOCK}\n        guard.{path}\n    }}\n\n",
                    field = field, ty = ty, path = a.path,
                )?;
            }
            (Some("string"), "set") => {
                // `String` field assignment; `setter_call` routes the
                // value through a `&mut self` method instead.
                let param = a.py_param.as_deref().expect("setter row needs py_param");
                let assign = match a.setter_call.as_deref() {
                    Some(call) => format!("guard.{call}({param});"),
                    None => format!("guard.{path} = {param};", path = a.path),
                };
                write!(
                    out,
                    "    #[setter]\n    fn set_{field}(&self, {param}: String) {{\n{LOCK}\n        {assign}\n    }}\n\n",
                    field = field, param = param, assign = assign,
                )?;
            }
            (Some("string"), _) => {
                // Owned `String` read; `getter_call` routes through a
                // `&self` method instead of a direct field clone.
                let read = match a.getter_call.as_deref() {
                    Some(call) => format!("guard.{call}().to_string()"),
                    None => format!("guard.{}.clone()", a.path),
                };
                write!(
                    out,
                    "    #[getter]\n    fn get_{field}(&self) -> String {{\n{RLOCK}\n        {read}\n    }}\n\n",
                    field = field, read = read,
                )?;
            }
            (_, "set") => {
                let ty = py_type(&a.abi_type);
                let param = a.py_param.as_deref().expect("setter row needs py_param");
                let rhs = match a.duration_unit.as_deref() {
                    Some("ms") => format!("std::time::Duration::from_millis({param})"),
                    Some("secs") => format!("std::time::Duration::from_secs({param})"),
                    _ => param.to_string(),
                };
                let body = if a.shape.as_deref() == Some("policy_limit") {
                    let lf = a
                        .limit_field
                        .as_deref()
                        .expect("policy_limit needs limit_field");
                    format!(
                        "        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {{\n            limits.{lf} = {rhs};\n        }}"
                    )
                } else {
                    format!("        guard.{path} = {rhs};", path = a.path)
                };
                write!(
                    out,
                    "    #[setter]\n    fn set_{field}(&self, {param}: {ty}) {{\n{LOCK}\n{body}\n    }}\n\n",
                    field = field, param = param, ty = ty, body = body,
                )?;
            }
            (_, _) => {
                let ty = py_type(&a.abi_type);
                let read = py_get_read(a);
                write!(
                    out,
                    "    #[getter]\n    fn get_{field}(&self) -> {ty} {{\n{RLOCK}\n        {read}\n    }}\n\n",
                    field = field, ty = ty, read = read,
                )?;
            }
        }
    }
    out.push_str("}\n");
    Ok(out)
}

/// The Python getter read expression (the value the `#[getter]` returns),
/// keyed by `shape` and `duration_unit`.
fn py_get_read(a: &Accessor) -> String {
    if a.shape.as_deref() == Some("policy_limit") {
        let lf = a
            .limit_field
            .as_deref()
            .expect("policy_limit needs limit_field");
        let suffix = if a.duration_unit.as_deref() == Some("secs") {
            ".as_secs()"
        } else {
            ""
        };
        // The fallback default is split across lines to match rustfmt's
        // method-chain wrap for the `secs` variants.
        return if suffix.is_empty() {
            format!(
                "match &guard.reconnect.policy {{\n            config::ReconnectPolicy::Auto(limits) => limits.{lf},\n            _ => config::ReconnectAttemptLimits::default().{lf},\n        }}"
            )
        } else {
            format!(
                "match &guard.reconnect.policy {{\n            config::ReconnectPolicy::Auto(limits) => limits.{lf}{suffix},\n            _ => config::ReconnectAttemptLimits::default()\n                .{lf}\n                {suffix},\n        }}"
            )
        };
    }
    match a.duration_unit.as_deref() {
        Some("ms") => format!(
            "u64::try_from(guard.{}.as_millis()).unwrap_or(u64::MAX)",
            a.path
        ),
        Some("secs") => format!("guard.{}.as_secs()", a.path),
        _ => format!("guard.{}", a.path),
    }
}

/// The TypeScript SDK config accessors: a `#[napi] impl Config` block
/// holding the mechanical scalar setter/getter pairs (`index.d.ts`
/// surface). `u64` knobs travel as `BigInt`; `u32`/`u16` as `number`;
/// the `Mutex` is held only for the single field write/read. Divergent
/// accessors (enum, string, `Option`-widened, policy-aware) stay
/// hand-written in `config_class.rs`.
pub(super) fn render_typescript_config_accessors() -> Result<String, Box<dyn std::error::Error>> {
    const LOCK: &str = "        let mut guard = self\n            .inner\n            .lock()\n            .map_err(|_| napi::Error::from_reason(\"Config mutex poisoned\"))?;";
    const RLOCK: &str = "        let guard = self\n            .inner\n            .lock()\n            .map_err(|_| napi::Error::from_reason(\"Config mutex poisoned\"))?;";
    let accessors = load()?;
    let mut out = String::new();
    out.push_str(
        "// @generated DO NOT EDIT — regenerated by build.rs from config_surface.toml\n//\n// Mechanical scalar config accessors for the TypeScript `Config` class.\n// `include!`-d into `config_class.rs`, which keeps the divergent accessors\n// (enum-parsing, string, `Option`-widened, policy-aware) hand-written and\n// owns the `bigint_to_u64` / `invalid_parameter_err` helpers used below.\n\n#[napi]\nimpl Config {\n",
    );
    for a in &accessors {
        let field = field_name(a);
        indented_doc(&mut out, "    ", &a.ts_doc);
        // ── enum carve-out (lowercase string ↔ core variant via the
        //    core enum `parse` / `as_str`; uniform across every enum) ──
        if a.kind == "enum" {
            if is_setter(a) {
                let param = a.ts_param.as_deref().expect("setter row needs ts_param");
                let set_js = format!("set{}", to_pascal_case(field));
                let core = a.enum_core.as_deref().expect("enum needs enum_core");
                let err = rust_lit(a.ts_err.as_deref().expect("enum setter needs ts_err"));
                // `lowercase_err` normalizes the value so the rejection
                // message echoes the lowercased input (parity with Python).
                let lower = if a.lowercase_err {
                    format!("        let {param} = {param}.to_ascii_lowercase();\n")
                } else {
                    String::new()
                };
                write!(
                    out,
                    "    #[napi(js_name = \"{set_js}\")]\n    pub fn set_{field}(&self, {param}: String) -> napi::Result<()> {{\n{lower}        let parsed = {core}::parse(&{param}).ok_or_else(|| {{\n            crate::invalid_parameter_err(format!(\n                \"{err}\"\n            ))\n        }})?;\n{LOCK}\n        guard.{path} = parsed;\n        Ok(())\n    }}\n\n",
                    set_js = set_js, field = field, param = param, core = core, err = err, path = a.path,
                    lower = lower,
                )?;
            } else {
                let get_js = to_camel_case(field);
                write!(
                    out,
                    "    #[napi(getter, js_name = \"{get_js}\")]\n    pub fn {field}(&self) -> napi::Result<&'static str> {{\n{RLOCK}\n        Ok(guard.{path}.as_str())\n    }}\n\n",
                    get_js = get_js, field = field, path = a.path,
                )?;
            }
            continue;
        }
        // ── option carve-out (`Option<T>` ↔ `T | null`; the abi switch
        //    picks `number` vs `BigInt`, the same the scalar arm runs) ──
        if a.kind == "option" {
            if is_setter(a) {
                let param = a.ts_param.as_deref().expect("setter row needs ts_param");
                let set_js = format!("set{}", to_pascal_case(field));
                match a.abi_type.as_str() {
                    // u16 ports arrive as `number`; range-checked to u16.
                    "u16" => {
                        let err =
                            rust_lit(a.ts_err.as_deref().expect("option u16 setter needs ts_err"));
                        write!(
                            out,
                            "    #[napi(js_name = \"{set_js}\")]\n    pub fn set_{field}(&self, {param}: Option<u32>) -> napi::Result<()> {{\n        let resolved = match {param} {{\n            Some(v) => Some(u16::try_from(v).map_err(|_| {{\n                crate::invalid_parameter_err(format!(\n                    \"{err}\"\n                ))\n            }})?),\n            None => None,\n        }};\n{LOCK}\n        guard.{path} = resolved;\n        Ok(())\n    }}\n\n",
                            set_js = set_js, field = field, param = param, err = err, path = a.path,
                        )?;
                    }
                    // Wider seeds arrive as `BigInt`; decoded losslessly.
                    "u64" => {
                        write!(
                            out,
                            "    #[napi(js_name = \"{set_js}\")]\n    pub fn set_{field}(\n        &self,\n        {param}: Option<napi::bindgen_prelude::BigInt>,\n    ) -> napi::Result<()> {{\n        let resolved = match {param} {{\n            Some(v) => Some(bigint_to_u64(\"{set_js}\", &v)?),\n            None => None,\n        }};\n{LOCK}\n        guard.{path} = resolved;\n        Ok(())\n    }}\n\n",
                            set_js = set_js, field = field, param = param, path = a.path,
                        )?;
                    }
                    other => panic!("config_surface: option abi '{other}'"),
                }
            } else {
                let get_js = to_camel_case(field);
                match a.abi_type.as_str() {
                    "u16" => {
                        write!(
                            out,
                            "    #[napi(getter, js_name = \"{get_js}\")]\n    pub fn {field}(&self) -> napi::Result<Option<u32>> {{\n{RLOCK}\n        Ok(guard.{path}.map(u32::from))\n    }}\n\n",
                            get_js = get_js, field = field, path = a.path,
                        )?;
                    }
                    "u64" => {
                        write!(
                            out,
                            "    #[napi(getter, js_name = \"{get_js}\")]\n    pub fn {field}(\n        &self,\n    ) -> napi::Result<Option<napi::bindgen_prelude::BigInt>> {{\n{RLOCK}\n        Ok(guard.{path}.map(napi::bindgen_prelude::BigInt::from))\n    }}\n\n",
                            get_js = get_js, field = field, path = a.path,
                        )?;
                    }
                    other => panic!("config_surface: option abi '{other}'"),
                }
            }
            continue;
        }
        // ── string carve-out (parity with the FFI `*const c_char`
        //    surface; the JS surface takes / returns a plain `string`) ──
        if a.shape.as_deref() == Some("string") {
            if a.kind == "set" {
                let param = a.ts_param.as_deref().expect("setter row needs ts_param");
                let set_js = format!("set{}", to_pascal_case(field));
                let assign = match a.setter_call.as_deref() {
                    Some(call) => format!("guard.{call}({param});"),
                    None => format!("guard.{path} = {param};", path = a.path),
                };
                write!(
                    out,
                    "    #[napi(js_name = \"{set_js}\")]\n    pub fn set_{field}(&self, {param}: String) -> napi::Result<()> {{\n{LOCK}\n        {assign}\n        Ok(())\n    }}\n\n",
                    set_js = set_js, field = field, param = param, assign = assign,
                )?;
            } else {
                let get_js = to_camel_case(field);
                let read = match a.getter_call.as_deref() {
                    Some(call) => format!("guard.{call}().to_string()"),
                    None => format!("guard.{}.clone()", a.path),
                };
                write!(
                    out,
                    "    #[napi(getter, js_name = \"{get_js}\")]\n    pub fn {field}(&self) -> napi::Result<String> {{\n{RLOCK}\n        Ok({read})\n    }}\n\n",
                    get_js = get_js, field = field, read = read,
                )?;
            }
            continue;
        }
        if a.kind == "set" {
            let param = a.ts_param.as_deref().expect("setter row needs ts_param");
            let set_js = format!("set{}", to_pascal_case(field));
            // `policy_limit` setters share the scalar BigInt/number decode
            // but assign into `ReconnectPolicy::Auto(limits)` rather than a
            // bare field; emit the decode + the `if let Auto` body.
            if a.shape.as_deref() == Some("policy_limit") {
                let lf = a
                    .limit_field
                    .as_deref()
                    .expect("policy_limit needs limit_field");
                let (arg_ty, decode, value_expr) = match a.abi_type.as_str() {
                    "u64" => (
                        "napi::bindgen_prelude::BigInt",
                        format!("        let value = bigint_to_u64(\"{set_js}\", &{param})?;\n"),
                        match a.duration_unit.as_deref() {
                            Some("secs") => "std::time::Duration::from_secs(value)".to_string(),
                            _ => "value".to_string(),
                        },
                    ),
                    // `u32` arrives as `f64` and is validated at the napi
                    // boundary; the attempt budgets here additionally floor
                    // at 1 (see `ts_u32_validator`). The diagnostic names the
                    // camelCase JS key, not the Rust param.
                    "u32" => (
                        "f64",
                        format!(
                            "        let value = crate::{}(\"{camel}\", {param})?;\n",
                            ts_u32_validator(field),
                            camel = to_camel_case(field),
                        ),
                        "value".to_string(),
                    ),
                    other => panic!("config_surface: policy_limit abi '{other}'"),
                };
                write!(
                    out,
                    "    #[napi(js_name = \"{set_js}\")]\n    pub fn set_{field}(&self, {param}: {arg_ty}) -> napi::Result<()> {{\n{decode}{LOCK}\n        if let config::ReconnectPolicy::Auto(ref mut limits) = guard.reconnect.policy {{\n            limits.{lf} = {value_expr};\n        }}\n        Ok(())\n    }}\n\n",
                    set_js = set_js, field = field, param = param, arg_ty = arg_ty,
                    decode = decode, lf = lf, value_expr = value_expr,
                )?;
                continue;
            }
            let (arg_ty, decode, value_expr) = match a.abi_type.as_str() {
                // u64 knobs arrive as a JS BigInt; decode losslessly.
                "u64" => (
                    "napi::bindgen_prelude::BigInt".to_string(),
                    format!("        let value = bigint_to_u64(\"{set_js}\", &{param})?;\n"),
                    match a.duration_unit.as_deref() {
                        Some("ms") => "std::time::Duration::from_millis(value)".to_string(),
                        Some("secs") => "std::time::Duration::from_secs(value)".to_string(),
                        _ => "value".to_string(),
                    },
                ),
                // Byte budgets exceed u32 and are stored as `usize`.
                "usize" => (
                    "napi::bindgen_prelude::BigInt".to_string(),
                    format!(
                        "        let value = bigint_to_u64(\"{set_js}\", &{param})?;\n        let value = usize::try_from(value)\n            .map_err(|_| napi::Error::from_reason(\"value exceeds usize on this platform\"))?;\n"
                    ),
                    "value".to_string(),
                ),
                // Ports are u16; the JS surface takes `number` and range-checks.
                "u16" => (
                    "u32".to_string(),
                    format!(
                        "        let value = u16::try_from({param}).map_err(|_| {{\n            crate::invalid_parameter_err(format!(\n                \"{set_js}: port must be in the u16 range 0..=65535; got {{{param}}}\"\n            ))\n        }})?;\n"
                    ),
                    "value".to_string(),
                ),
                // `u32` arrives as `f64` and is validated at the napi
                // boundary so a hostile `-1` / `1.5` / `2**32` is rejected
                // rather than silently wrapped by V8 `ToUint32`. Attempt
                // budgets additionally floor at 1 (`ts_u32_validator`); the
                // diagnostic names the camelCase JS key.
                "u32" => (
                    "f64".to_string(),
                    format!(
                        "        let value = crate::{}(\"{camel}\", {param})?;\n",
                        ts_u32_validator(field),
                        camel = to_camel_case(field),
                    ),
                    "value".to_string(),
                ),
                "bool" => ("bool".to_string(), String::new(), param.to_string()),
                other => panic!("config_surface: unmapped abi_type '{other}'"),
            };
            write!(
                out,
                "    #[napi(js_name = \"{set_js}\")]\n    pub fn set_{field}(&self, {param}: {arg_ty}) -> napi::Result<()> {{\n{decode}{LOCK}\n        guard.{path} = {value_expr};\n        Ok(())\n    }}\n\n",
                set_js = set_js, field = field, param = param, arg_ty = arg_ty,
                decode = decode, path = a.path, value_expr = value_expr,
            )?;
        } else {
            let get_js = to_camel_case(field);
            // `policy_limit` getters read through `ReconnectPolicy::Auto`,
            // falling back to the `Auto` defaults; the value then wraps the
            // same way as the matching scalar abi (BigInt for secs / u32).
            if a.shape.as_deref() == Some("policy_limit") {
                let lf = a
                    .limit_field
                    .as_deref()
                    .expect("policy_limit needs limit_field");
                let matched = format!(
                    "match &guard.reconnect.policy {{\n            config::ReconnectPolicy::Auto(limits) => limits.{lf},\n            _ => config::ReconnectAttemptLimits::default().{lf},\n        }}"
                );
                let (ret_ty, body) = match a.abi_type.as_str() {
                    "u64" => (
                        "napi::bindgen_prelude::BigInt",
                        format!(
                            "        let value = {matched};\n        Ok(napi::bindgen_prelude::BigInt::from(value.as_secs()))\n"
                        ),
                    ),
                    "u32" => ("u32", format!("        Ok({matched})\n")),
                    other => panic!("config_surface: policy_limit abi '{other}'"),
                };
                write!(
                    out,
                    "    #[napi(getter, js_name = \"{get_js}\")]\n    pub fn {field}(&self) -> napi::Result<{ret_ty}> {{\n{RLOCK}\n{body}    }}\n\n",
                    get_js = get_js, field = field, ret_ty = ret_ty, body = body,
                )?;
                continue;
            }
            let (ret_ty, body) = match a.abi_type.as_str() {
                "u64" => {
                    let read = match a.duration_unit.as_deref() {
                        // `retry.*` `Duration` fields read as saturating `u64` ms.
                        Some("ms") => format!(
                            "u64::try_from(guard.{}.as_millis()).unwrap_or(u64::MAX)",
                            a.path
                        ),
                        Some("secs") => format!("guard.{}.as_secs()", a.path),
                        _ => format!("guard.{}", a.path),
                    };
                    (
                        "napi::bindgen_prelude::BigInt".to_string(),
                        format!("        Ok(napi::bindgen_prelude::BigInt::from({read}))\n"),
                    )
                }
                "usize" => (
                    "napi::bindgen_prelude::BigInt".to_string(),
                    format!(
                        "        Ok(napi::bindgen_prelude::BigInt::from(guard.{} as u64))\n",
                        a.path
                    ),
                ),
                "u16" => (
                    "u32".to_string(),
                    format!("        Ok(u32::from(guard.{}))\n", a.path),
                ),
                "u32" => ("u32".to_string(), format!("        Ok(guard.{})\n", a.path)),
                "bool" => (
                    "bool".to_string(),
                    format!("        Ok(guard.{})\n", a.path),
                ),
                other => panic!("config_surface: unmapped abi_type '{other}'"),
            };
            write!(
                out,
                "    #[napi(getter, js_name = \"{get_js}\")]\n    pub fn {field}(&self) -> napi::Result<{ret_ty}> {{\n{RLOCK}\n{body}    }}\n\n",
                get_js = get_js, field = field, ret_ty = ret_ty, body = body,
            )?;
        }
    }
    out.push_str("}\n");
    Ok(out)
}
