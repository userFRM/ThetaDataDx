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

#[derive(Debug, Deserialize)]
struct ConfigSurface {
    accessor: Vec<Accessor>,
}

fn load() -> Result<Vec<Accessor>, Box<dyn std::error::Error>> {
    let spec_str = std::fs::read_to_string("config_surface.toml")?;
    let spec: ConfigSurface = toml::from_str(&spec_str)?;
    Ok(spec.accessor)
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

/// The Rust read expression for one getter (the value written into the
/// out-parameter), keyed by KIND. Scalar / duration reads come straight
/// off `config.inner.<path>`; the `policy_limit` read walks
/// `reconnect.policy` and falls back to the `Auto` defaults.
fn ffi_get_read_expr(a: &Accessor) -> String {
    if a.shape.as_deref() == Some("policy_limit") {
        let field = a
            .limit_field
            .as_deref()
            .expect("policy_limit needs limit_field");
        let suffix = if a.duration_unit.as_deref() == Some("secs") {
            ".as_secs()"
        } else {
            ""
        };
        return format!(
            "match &config.inner.reconnect.policy {{\n            thetadatadx::ReconnectPolicy::Auto(limits) => limits.{field}{suffix},\n            _ => {LIMIT_DEFAULT}.{field}{suffix},\n        }}"
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
                write!(
                    out,
                    "pub unsafe extern \"C\" fn {sym}(\n    config: *mut ThetaDataDxConfig,\n    {p}: *const c_char,\n) -> i32 {{\n    ffi_boundary!(-1, {{\n        if config.is_null() {{\n            set_error(\"config handle is null\");\n            return -1;\n        }}\n        // SAFETY: caller supplies a NUL-terminated C string allocated by the host runtime; cstr_to_str validates non-null + UTF-8.\n        let {p} = match unsafe {{ cstr_to_str({p}) }} {{\n            Ok(Some(s)) => s,\n            Ok(None) => {{\n                set_error(\"{p} is null\");\n                return -1;\n            }}\n            Err(e) => {{\n                set_error(&format!(\"{p} is not valid UTF-8: {{e}}\"));\n                return -1;\n            }}\n        }};\n        // SAFETY: config is a non-null pointer returned by thetadatadx_config_* and not yet freed.\n        let config = unsafe {{ &mut *config }};\n        {assign}\n        0\n    }})\n}}\n\n",
                    sym = a.symbol, p = a.param, assign = assign,
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
                    let field = a
                        .limit_field
                        .as_deref()
                        .expect("policy_limit needs limit_field");
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
                    "u32" => ("u32", String::new(), param.to_string()),
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
                "u32" => ("u32".to_string(), String::new(), param.to_string()),
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
