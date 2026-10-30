//! C and C++ identifier escaping for schema-driven field names.
//!
//! Schema column names are wire vocabulary, not identifiers: a column is free
//! to be called `class` or `register`, keywords in C or C++. The C++ emitter
//! routes every schema field it renders as an identifier through
//! [`c_field_ident`], so a reserved-word column can never produce an
//! uncompilable header. The keyword table is deliberately broad: escaping a
//! name that did not strictly need it is a cosmetic cost, while missing one is
//! an unusable public surface.

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
    fn c_escapes_its_keywords() {
        assert_eq!(c_field_ident("register"), "register_");
        assert_eq!(c_field_ident("vega"), "vega");
    }
}
