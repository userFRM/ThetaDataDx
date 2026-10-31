//! The C declarations, the C++ `Config` accessors and the Python stub's
//! `Config` properties, rendered from the core config table.
//!
//! `thetadatadx::__config!` calls back into `config_records!`, which turns
//! each entry into a [`ConfigAccessor`]. The C declarations are those of the
//! entry points the FFI crate expands from the same table; the C++ methods
//! forward to them and are spliced into the `Config` class body of
//! `thetadatadx.hpp`; the Python properties are those the Python binding
//! expands from the table.

use std::fmt::Write as _;

/// One accessor of the table.
struct ConfigAccessor {
    symbol: &'static str,
    setter: bool,
    /// The C parameter: the value for a setter, the out-parameter for a getter.
    param: &'static str,
    kind: Kind,
    doc: &'static [&'static str],
    /// The default as the table writes it, and the same value in other units
    /// or `""`.
    default: Option<(&'static str, &'static str)>,
    /// The C error code an enum setter reports for a null handle.
    null_error: &'static str,
}

/// How a value crosses the C ABI. The scalar and optional kinds carry the
/// value's types.
enum Kind {
    /// An enum, as its `int` code; Python reads and writes the lowercase
    /// variant name. Each value is its code, its variant and, on a setter,
    /// what it does.
    Enum(&'static [(&'static str, &'static str, &'static str)]),
    /// An `Option`, as a `has_value` flag beside the value.
    Optional(Type),
    /// A UTF-8 string.
    String,
    Scalar(Type),
}

/// A value's C, C++ and Python type names.
struct Type {
    c: &'static str,
    cpp: &'static str,
    py: &'static str,
}

macro_rules! config_records {
    ($($dir:ident $sym:ident ($p:ident: $($ty:tt)+) $target:tt {
        $(param: $bp:ident,)?
        $(default: $default:tt $($gloss:literal)?,)?
        $(values: $enum:ident { $($code:literal $variant:ident $($vdoc:literal)?),* $(,)? },)?
        $(errors: { null: $null:ident, $($error:tt)* },)?
        doc: [$($doc:literal),* $(,)?] $(,)?
    })*) => {
        &[$(ConfigAccessor {
            symbol: stringify!($sym),
            setter: config_records!(@setter $dir),
            param: stringify!($p),
            kind: config_records!(@kind [$($ty)+] $([$((stringify!($code), stringify!($variant), concat!($($vdoc)?))),*])?),
            doc: &[$($doc),*],
            default: config_records!(@default $([$default $($gloss)?])?),
            null_error: concat!($(stringify!($null))?),
        }),*]
    };
    (@setter set) => { true };
    (@setter get) => { false };
    (@kind [$($ty:tt)+] [$($value:expr),*]) => { Kind::Enum(&[$($value),*]) };
    (@kind [String]) => { Kind::String };
    (@kind [Option<$t:ident>]) => { Kind::Optional(config_records!(@type $t)) };
    (@kind [$t:ident]) => { Kind::Scalar(config_records!(@type $t)) };
    (@type u64) => { Type { c: "uint64_t", cpp: "std::uint64_t", py: "int" } };
    (@type u32) => { Type { c: "uint32_t", cpp: "std::uint32_t", py: "int" } };
    (@type u16) => { Type { c: "uint16_t", cpp: "std::uint16_t", py: "int" } };
    (@type usize) => { Type { c: "size_t", cpp: "std::size_t", py: "int" } };
    (@type bool) => { Type { c: "bool", cpp: "bool", py: "bool" } };
    (@default) => { None };
    (@default [$v:tt $($gloss:literal)?]) => { Some((stringify!($v), concat!($($gloss)?))) };
}

const ACCESSORS: &[ConfigAccessor] = thetadatadx::__config! { config_records {} };

impl ConfigAccessor {
    /// The table's documentation, the default with its value spelled by
    /// `spell`, then each note as a paragraph of its lines.
    fn render(&self, spell: fn(&str) -> String, notes: Vec<Vec<String>>) -> Vec<String> {
        let mut doc: Vec<String> = self.doc.iter().map(ToString::to_string).collect();
        if let Some((value, gloss)) = self.default {
            let gloss = if gloss.is_empty() {
                String::new()
            } else {
                format!(" ({gloss})")
            };
            doc.push(format!("Default `{}`{gloss}.", spell(value)));
        }
        for note in notes {
            doc.push(String::new());
            doc.extend(note);
        }
        doc
    }

    /// The C documentation, as the FFI crate writes it on the entry point.
    fn c_doc(&self) -> Vec<String> {
        let p = self.param;
        let notes = match (&self.kind, self.setter) {
            (Kind::Enum(values), true) => vec![
                values
                    .iter()
                    .map(|(code, variant, what)| format!("- `{p} = {code}` (`{variant}`): {what}"))
                    .collect(),
                vec![format!(
                    "Returns `0` on success, or `-1` with `thetadatadx_last_error()` set when `config` is null or \
                     `{p}` is not one of these codes; an unknown code carries `THETADATADX_ERR_INVALID_PARAMETER`."
                )],
            ],
            (Kind::Enum(values), false) => vec![
                vec![format!("Writes the code into `*{p}`:")],
                values
                    .iter()
                    .map(|(code, variant, _)| format!("- `{code}`: `{variant}`"))
                    .collect(),
                vec!["Returns `0` on success, `-1` if either pointer is null.".into()],
            ],
            (Kind::Optional(_), true) => vec![vec![format!(
                "`has_value = false` unsets the value and ignores `{p}`. Returns `0` on success, `-1` if \
                 `config` is null."
            )]],
            (Kind::Optional(_), false) => vec![vec![format!(
                "Writes whether a value is set into `*out_has_value`, and the value, or `0` when unset, into \
                 `*{p}`. Returns `0` on success, `-1` if any pointer is null."
            )]],
            (Kind::String, true) => vec![vec![format!(
                "`{p}` must be a non-null, NUL-terminated UTF-8 string. Returns `0` on success, or `-1` with \
                 `thetadatadx_last_error()` set when `config` or `{p}` is null or not valid UTF-8."
            )]],
            (Kind::String, false) => vec![vec![
                "Returns a heap-owned NUL-terminated string the caller releases with `thetadatadx_string_free`, \
                 or null with `thetadatadx_last_error()` set when `config` is null or the value holds an \
                 interior NUL."
                    .into(),
            ]],
            (Kind::Scalar(_), true) => vec![],
            (Kind::Scalar(_), false) => vec![vec![format!(
                "Writes the value into `*{p}`. Returns `0` on success, `-1` if either pointer is null."
            )]],
        };
        self.render(str::to_string, notes)
    }

    /// The C++ documentation of the `Config` method.
    fn cpp_doc(&self) -> Vec<String> {
        let note = match (&self.kind, self.setter) {
            (Kind::Enum(values), true) => {
                let null = match self.null_error {
                    "THETADATADX_ERR_CONFIG" => "thetadatadx::ConfigError",
                    _ => "thetadatadx::ThetaDataError",
                };
                let list = values
                    .iter()
                    .map(|(code, variant, what)| format!("- `{code}` (`{variant}`): {what}"))
                    .collect();
                let throws = format!(
                    "Throws `thetadatadx::InvalidParameterError` on any other code and `{null}` on a null handle."
                );
                return self.render(str::to_string, vec![list, vec![throws]]);
            }
            (Kind::Enum(_), false) => "Returns the code the setter takes, or `0` on a null handle.",
            (Kind::Optional(_), true) => {
                "`std::nullopt` unsets the value. Throws `thetadatadx::ThetaDataError` on a null handle."
            }
            (Kind::Optional(_), false) => {
                "Returns `std::nullopt` when unset. Throws `thetadatadx::ThetaDataError` on a null handle."
            }
            (Kind::String, true) => {
                "Throws `thetadatadx::ThetaDataError` on a null handle or a value that is not valid UTF-8."
            }
            (Kind::String, false) => {
                "Returns an empty string on a null handle or a value holding an interior NUL."
            }
            (Kind::Scalar(_), true) => return self.render(str::to_string, vec![]),
            (Kind::Scalar(Type { cpp: "bool", .. }), false) => "Returns `false` on a null handle.",
            (Kind::Scalar(_), false) => "Returns `0` on a null handle.",
        };
        self.render(str::to_string, vec![vec![note.into()]])
    }

    /// The documentation of the Python property, which is its setter's.
    fn py_doc(&self) -> Vec<String> {
        match &self.kind {
            Kind::Enum(values) => {
                let list = values
                    .iter()
                    .map(|(_, variant, what)| format!("- `\"{}\"`: {what}", variant.to_lowercase()))
                    .collect();
                self.render(
                    |v| format!("\"{}\"", v.to_lowercase()),
                    vec![vec!["Accepts, case-insensitively:".into()], list],
                )
            }
            Kind::Optional(_) => self.render(
                str::to_string,
                vec![vec!["`None` unsets the value.".into()]],
            ),
            Kind::String | Kind::Scalar(_) => self.render(
                |v| match v {
                    "true" => "True".into(),
                    "false" => "False".into(),
                    _ => v.into(),
                },
                vec![],
            ),
        }
    }
}

/// The C++ include: one inline `Config` method per accessor, spliced
/// into the class body in `thetadatadx.hpp`.
pub(super) fn render_cpp_config_accessors() -> String {
    let mut out = String::from(
        "/* @generated DO NOT EDIT: regenerated by generate_sdk_surfaces from the config table */\n",
    );
    for a in ACCESSORS {
        let (sym, p) = (a.symbol, a.param);
        let method = sym.strip_prefix("thetadatadx_config_").unwrap();
        out.push('\n');
        for line in a.cpp_doc() {
            if line.is_empty() {
                out.push_str("    ///\n");
            } else {
                writeln!(out, "    /// {line}").unwrap();
            }
        }
        // A setter that reports errors throws the FFI's last error; a scalar
        // setter ignores a null handle, and a getter returns the default.
        match (&a.kind, a.setter) {
            (Kind::Enum(_), true) => write!(
                out,
                "    void {method}(int {p}) {{\n        if ({sym}(handle_.get(), {p}) != 0) {{\n            detail::throw_last_ffi_error();\n        }}\n    }}\n"
            ),
            (Kind::Enum(_), false) => write!(
                out,
                "    int {method}() const {{\n        int32_t out{{}};\n        {sym}(handle_.get(), &out);\n        return out;\n    }}\n"
            ),
            (Kind::Optional(Type { cpp: ty, .. }), true) => write!(
                out,
                "    void {method}(std::optional<{ty}> {p}) {{\n        const bool has_value = {p}.has_value();\n        const {ty} arg = {p}.value_or(0);\n        if ({sym}(handle_.get(), has_value, arg) != 0) {{\n            detail::throw_last_ffi_error();\n        }}\n    }}\n"
            ),
            (Kind::Optional(Type { cpp: ty, .. }), false) => {
                let val = p.strip_prefix("out_").unwrap_or(p);
                write!(
                    out,
                    "    std::optional<{ty}> {method}() const {{\n        bool has_value = false;\n        {ty} {val} = 0;\n        if ({sym}(handle_.get(), &has_value, &{val}) != 0) {{\n            detail::throw_last_ffi_error();\n        }}\n        return has_value ? std::optional<{ty}>{{{val}}} : std::nullopt;\n    }}\n"
                )
            }
            (Kind::String, true) => write!(
                out,
                "    void {method}(const std::string& {p}) {{\n        if ({sym}(handle_.get(), {p}.c_str()) != 0) {{\n            detail::throw_last_ffi_error();\n        }}\n    }}\n"
            ),
            (Kind::String, false) => write!(
                out,
                "    std::string {method}() const {{\n        detail::FfiString s({sym}(handle_.get()));\n        return s.str();\n    }}\n"
            ),
            (Kind::Scalar(Type { cpp: ty, .. }), true) => write!(
                out,
                "    void {method}({ty} {p}) {{\n        {sym}(handle_.get(), {p});\n    }}\n"
            ),
            (Kind::Scalar(Type { cpp: ty, .. }), false) => write!(
                out,
                "    {ty} {method}() const {{\n        {ty} out{{}};\n        {sym}(handle_.get(), &out);\n        return out;\n    }}\n"
            ),
        }
        .unwrap();
    }
    out
}

/// The C include: the declaration of every table entry point, with the
/// documentation the FFI crate gives it.
pub(super) fn render_c_config_accessors() -> String {
    let mut out = String::from(
        "/* @generated DO NOT EDIT: regenerated by generate_sdk_surfaces from the config table */\n\
         /* A setter that returns void ignores a NULL config handle. Every other\n\
         \x20* accessor returns -1 (a string getter NULL) and sets\n\
         \x20* thetadatadx_last_error() when a pointer it needs is NULL. */\n",
    );
    for a in ACCESSORS {
        let (sym, p) = (a.symbol, a.param);
        out.push('\n');
        for (i, line) in a.c_doc().iter().enumerate() {
            let lead = if i == 0 { "/**" } else { " *" };
            if line.is_empty() {
                writeln!(out, "{lead}").unwrap();
            } else {
                writeln!(out, "{lead} {line}").unwrap();
            }
        }
        out.push_str(" */\n");
        match (&a.kind, a.setter) {
            (Kind::Enum(_), true) => writeln!(
                out,
                "int32_t {sym}(ThetaDataDxConfig* config, int32_t {p});"
            ),
            (Kind::Enum(_), false) => writeln!(
                out,
                "int32_t {sym}(const ThetaDataDxConfig* config, int32_t* {p});"
            ),
            (Kind::Optional(Type { c, .. }), true) => {
                writeln!(
                    out,
                    "int32_t {sym}(ThetaDataDxConfig* config, bool has_value, {c} {p});"
                )
            }
            (Kind::Optional(Type { c, .. }), false) => writeln!(
                out,
                "int32_t {sym}(const ThetaDataDxConfig* config, bool* out_has_value, {c}* {p});"
            ),
            (Kind::String, true) => writeln!(
                out,
                "int32_t {sym}(ThetaDataDxConfig* config, const char* {p});"
            ),
            (Kind::String, false) => writeln!(out, "char* {sym}(const ThetaDataDxConfig* config);"),
            (Kind::Scalar(Type { c, .. }), true) => {
                writeln!(out, "void {sym}(ThetaDataDxConfig* config, {c} {p});")
            }
            (Kind::Scalar(Type { c, .. }), false) => {
                writeln!(
                    out,
                    "int32_t {sym}(const ThetaDataDxConfig* config, {c}* {p});"
                )
            }
        }
        .unwrap();
    }
    out
}

pub(super) const PY_BEGIN_MARKER: &str =
    "# --- BEGIN GENERATED CONFIG PROPERTIES (config table) ---";
pub(super) const PY_END_MARKER: &str = "# --- END GENERATED CONFIG PROPERTIES ---";

/// The region of the Python stub's `Config` class holding one property per
/// table setter, with the type and documentation the runtime property has,
/// its code spans in reST: a property takes the documentation of its setter,
/// which comes first.
pub(super) fn render_python_config_properties() -> String {
    let mut out =
        format!("{PY_BEGIN_MARKER}\n    # Generated from the config table; do not edit by hand.\n");
    for a in ACCESSORS.iter().filter(|a| a.setter) {
        let name = a.symbol.strip_prefix("thetadatadx_config_set_").unwrap();
        let ty = match &a.kind {
            Kind::Enum(values) => format!(
                "Literal[{}]",
                values
                    .iter()
                    .map(|(_, variant, _)| format!("\"{}\"", variant.to_lowercase()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Kind::Optional(Type { py, .. }) => format!("Optional[{py}]"),
            Kind::String => "str".to_string(),
            Kind::Scalar(Type { py, .. }) => py.to_string(),
        };
        writeln!(out, "    {name}: {ty}").unwrap();
        let doc: Vec<String> = a
            .py_doc()
            .iter()
            .map(|line| line.replace('`', "``"))
            .collect();
        let (first, rest) = doc.split_first().expect("every setter is documented");
        if rest.is_empty() {
            writeln!(out, "    \"\"\"{first}\"\"\"").unwrap();
            continue;
        }
        writeln!(out, "    \"\"\"{first}").unwrap();
        for line in rest {
            if line.is_empty() {
                out.push('\n');
            } else {
                writeln!(out, "    {line}").unwrap();
            }
        }
        out.push_str("    \"\"\"\n");
    }
    out.push_str("    ");
    out.push_str(PY_END_MARKER);
    out
}
