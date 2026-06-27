//! Per-language identifier escaping for schema-driven field names.
//!
//! Schema column names are wire vocabulary, not identifiers — a column
//! is free to be called `lambda` (a Python keyword) or, in principle,
//! `class` (a keyword in several targets). Every emitter that renders a
//! schema field as a language identifier must route the name through
//! the matching helper here so a reserved-word column can never produce
//! an unreachable attribute (the failure mode behind the Python
//! `tick.lambda` SyntaxError) or an uncompilable binding.
//!
//! Escape conventions per language:
//!
//! * **Python** — PEP 8 keyword convention: append a trailing
//!   underscore (`lambda` -> `lambda_`). Applied to pyclass struct
//!   fields (the `#[pyo3(get)]` attribute name), constructor kwargs,
//!   `__repr__` labels, and stubs. Columnar surfaces (Arrow fields,
//!   pandas columns, dict keys) keep the logical column name — column
//!   names are strings, not identifiers, so no collision exists there.
//! * **Rust** — raw-identifier prefix (`type` -> `r#type`). Keywords
//!   that cannot be raw (`self`, `super`, `crate`, `Self`) get the
//!   trailing-underscore fallback.
//! * **TypeScript / JavaScript** — no escape needed: every generated
//!   TS surface renders schema fields as object KEYS or property
//!   accesses (`{ class: 1 }` and `t.class` are both legal for
//!   reserved words), never as bare identifiers. The Rust side of each
//!   `#[napi(object)]` struct routes through the Rust escape.
//! * **C / C++** — trailing underscore (`register` -> `register_`).
//!
//! Keyword tables are deliberately broad (soft keywords included):
//! escaping a name that did not strictly need it is a cosmetic cost,
//! while missing one is an unusable public surface.

/// Python keywords (`keyword.kwlist`, 3.12) plus soft keywords
/// (`keyword.softkwlist`: `match` / `case` / `type` / `_`). Soft
/// keywords are legal attribute names today, but escaping them keeps
/// generated surfaces clear of parser ambiguity in pattern contexts.
const PYTHON_RESERVED: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "match", "case", "type", "_",
];

/// Rust strict + reserved keywords (2024 edition). Names here are
/// emitted as raw identifiers (`r#name`); the four that cannot be raw
/// fall back to a trailing underscore.
const RUST_RESERVED: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield", "Self",
];

/// Rust keywords that cannot take the `r#` prefix.
const RUST_NO_RAW: &[&str] = &["self", "super", "crate", "Self"];

/// C keywords (C11) + the C++ superset that matters for the generated
/// headers. One table — the generated `.h` is compiled by both.
const C_CPP_RESERVED: &[&str] = &[
    "alignas",
    "alignof",
    "auto",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "constexpr",
    "continue",
    "default",
    "delete",
    "do",
    "double",
    "else",
    "enum",
    "explicit",
    "extern",
    "false",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "namespace",
    "new",
    "noexcept",
    "nullptr",
    "operator",
    "private",
    "protected",
    "public",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "template",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

/// Python attribute spelling for a schema field name. `lambda` ->
/// `lambda_`; non-keywords pass through unchanged.
pub(crate) fn python_field_ident(field: &str) -> String {
    if PYTHON_RESERVED.contains(&field) {
        format!("{field}_")
    } else {
        field.to_string()
    }
}

/// Rust identifier spelling for a schema field name. Keywords become
/// raw identifiers (`r#type`); the non-raw-able four take a trailing
/// underscore.
pub(crate) fn rust_field_ident(field: &str) -> String {
    if RUST_NO_RAW.contains(&field) {
        format!("{field}_")
    } else if RUST_RESERVED.contains(&field) {
        format!("r#{field}")
    } else {
        field.to_string()
    }
}

/// C / C++ identifier spelling for a schema field name. Keywords take
/// a trailing underscore.
pub(crate) fn c_field_ident(field: &str) -> String {
    if C_CPP_RESERVED.contains(&field) {
        format!("{field}_")
    } else {
        field.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_escapes_keywords_and_soft_keywords() {
        assert_eq!(python_field_ident("lambda"), "lambda_");
        assert_eq!(python_field_ident("class"), "class_");
        assert_eq!(python_field_ident("match"), "match_");
        assert_eq!(python_field_ident("delta"), "delta");
    }

    #[test]
    fn rust_raw_escapes_keywords() {
        assert_eq!(rust_field_ident("type"), "r#type");
        assert_eq!(rust_field_ident("self"), "self_");
        assert_eq!(rust_field_ident("lambda"), "lambda");
    }

    #[test]
    fn c_escapes_its_keywords() {
        assert_eq!(c_field_ident("register"), "register_");
        assert_eq!(c_field_ident("vega"), "vega");
    }
}
