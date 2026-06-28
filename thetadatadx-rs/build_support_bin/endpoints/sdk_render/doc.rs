//! Safe doc-comment emitters for codegen.
//!
//! `endpoint_surface.toml` strings (`description`, `name`, ...) are
//! interpolated into generated source files. A description that
//! contains `"`, `\`, an embedded newline, or — for C-family
//! languages — `*/` would otherwise produce uncompilable output.
//! These helpers normalize the text into a form every target
//! language can safely consume.

use std::fmt::Write as _;

/// Append `text` as a sequence of `///` doc-comment lines to `buf`, with
/// the indicated `indent` prefix. Each input line lands on its own `///`
/// line so embedded `\n` cannot escape the comment; `\r` bytes are
/// stripped to keep Windows-checkout drift out of the generated artifact.
/// Line comments (never `/** ... */`) so an embedded `*/` in a C-family
/// target cannot close the block early.
///
/// When `text` is empty, nothing is emitted — the caller decides
/// whether to render a fallback paragraph.
pub(super) fn doc_lines(buf: &mut String, indent: &str, text: &str) {
    for line in text.replace('\r', "").lines() {
        // `///` plus the optional space lives in `indent` already
        // when the caller wants nested indentation.
        writeln!(buf, "{indent}/// {line}").unwrap();
    }
}
