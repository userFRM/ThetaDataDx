//! Cross-check that the TLS-reader marker list in the Rust generator
//! and the Go static audit stay byte-identical. W3 round-7.
//!
//! This is a crate-level integration test so it runs under the default
//! `cargo test --workspace --locked` invocation that CI uses. It does
//! not depend on the `config-file` feature (which gates
//! `generate_sdk_surfaces`) because it parses both files textually.
//!
//! Any divergence fails the test naming the missing markers on each
//! side, in both directions (Rust-only addition AND Go-only addition
//! are caught).

use std::collections::BTreeSet;
use std::path::PathBuf;

const RUST_CONST: &str = "const GO_TLS_READER_MARKERS: &[&str] = &[";
const RUST_FILE: &str = "build_support/sdk_surface.rs";
const GO_VAR: &str = "var tlsReaderMarkers = []string{";
const GO_FILE: &str = "../../sdks/go/timeout_pin_test.go";

#[test]
fn go_tls_marker_list_mirrors_rust() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let rust_path = crate_root.join(RUST_FILE);
    let go_path = crate_root.join(GO_FILE);

    let rust_text = std::fs::read_to_string(&rust_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", rust_path.display()));
    let go_text = std::fs::read_to_string(&go_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", go_path.display()));

    let rust_markers = extract_markers(&rust_text, RUST_CONST, "];")
        .unwrap_or_else(|e| panic!("parse {}::GO_TLS_READER_MARKERS: {e}", rust_path.display()));
    let go_markers = extract_markers(&go_text, GO_VAR, "}")
        .unwrap_or_else(|e| panic!("parse {}::tlsReaderMarkers: {e}", go_path.display()));

    let rust_set: BTreeSet<&String> = rust_markers.iter().collect();
    let go_set: BTreeSet<&String> = go_markers.iter().collect();

    let missing_in_go: Vec<&&String> = rust_set.difference(&go_set).collect();
    let missing_in_rust: Vec<&&String> = go_set.difference(&rust_set).collect();

    if !missing_in_go.is_empty() || !missing_in_rust.is_empty() {
        panic!(
            "TLS marker lists diverged.\n  \
             In {rust_const} but not in {go_var}: {missing_in_go:?}\n  \
             In {go_var} but not in {rust_const}: {missing_in_rust:?}\n  \
             Sync the two lists — both files cross-reference each other in comments.",
            rust_const = "GO_TLS_READER_MARKERS",
            go_var = "tlsReaderMarkers",
        );
    }
}

/// Extract the string literals from a block opened by `open_marker`
/// and closed by `close_marker`. Works for both the Rust `&[&str]`
/// literal and the Go `[]string` literal because both use
/// double-quoted string syntax without escapes in our marker table.
/// Lines without a string literal are ignored (comments, blank
/// lines). Only the first quoted substring on each line contributes.
fn extract_markers(
    src: &str,
    open_marker: &str,
    close_marker: &str,
) -> Result<Vec<String>, String> {
    let start = src
        .find(open_marker)
        .ok_or_else(|| format!("`{open_marker}` not found"))?;
    let after_open = &src[start + open_marker.len()..];
    let close = after_open
        .find(close_marker)
        .ok_or_else(|| format!("closing `{close_marker}` not found after `{open_marker}`"))?;
    let body = &after_open[..close];

    let mut markers = Vec::new();
    for line in body.lines() {
        let Some(first_quote) = line.find('"') else {
            continue;
        };
        let rest = &line[first_quote + 1..];
        let Some(end_quote) = rest.find('"') else {
            continue;
        };
        markers.push(rest[..end_quote].to_string());
    }
    if markers.is_empty() {
        return Err("no markers extracted".into());
    }
    Ok(markers)
}
