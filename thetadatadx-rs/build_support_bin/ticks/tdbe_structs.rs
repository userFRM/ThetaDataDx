//! Doc and accessor helpers the Python and TypeScript tick renderers share:
//! the epoch accessors a tick type carries, and the provenance scrub its
//! documentation goes through.

use super::schema::TickTypeDef;

/// `(accessor_name, ms_field, date_field)` triples: the epoch accessors of a
/// tick type, shared by the Python and TypeScript emitters so every binding
/// surfaces the same accessor set, built from the same pair of columns. Empty
/// when the type has no `date` column.
///
/// A prefixed time of day takes the date of its own prefix when the type
/// carries one. The Greeks ticks hold two moments: the row's own
/// (`date` + `ms_of_day`) and the underlying price's
/// (`underlying_date` + `underlying_ms_of_day`), and the vendor sends them on
/// different days whenever the option quote and the underlying price come
/// from different sessions. Pairing a prefixed time of day with the row's
/// `date` would hand back an instant a day out on exactly those rows.
pub(super) fn timestamp_accessor_fields(def: &TickTypeDef) -> Vec<(String, String, String)> {
    if !def.columns.iter().any(|c| c.field == "date") {
        return Vec::new();
    }
    let has_column = |name: &str| def.columns.iter().any(|c| c.field == name);
    let mut out = Vec::new();
    for column in &def.columns {
        if column.field == "ms_of_day" {
            out.push((
                "timestamp_ms".to_string(),
                column.field.clone(),
                "date".to_string(),
            ));
        } else if let Some(prefix) = column.field.strip_suffix("_ms_of_day") {
            let prefixed_date = format!("{prefix}_date");
            let date_field = if has_column(&prefixed_date) {
                prefixed_date
            } else {
                "date".to_string()
            };
            out.push((
                format!("{prefix}_timestamp_ms"),
                column.field.clone(),
                date_field,
            ));
        }
    }
    out
}

/// Drop wire-provenance prose from a schema doc so the generated public
/// surface describes only stable behavior. Two artifact shapes are
/// removed: prose sentences that record where a layout was checked, and
/// the wire-column listings (Markdown tables, indented header dumps) those
/// sentences introduce. Behavioral sentences sharing a paragraph with a
/// provenance sentence are preserved.
pub(super) fn scrub_provenance(doc: &str) -> String {
    let mut kept: Vec<String> = Vec::new();

    for paragraph in split_paragraphs(doc) {
        if is_wire_layout_block(&paragraph) {
            continue;
        }
        let cleaned = drop_provenance_sentences(&paragraph);
        if !cleaned.trim().is_empty() {
            kept.push(cleaned);
        }
    }

    kept.join("\n\n")
}

/// Split a doc string into paragraphs delimited by blank lines, trimming a
/// leading blank line (some schema docs open with `"""` on its own line).
fn split_paragraphs(doc: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in doc.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }
    paragraphs
}

/// A paragraph that is purely a wire-layout artifact: every non-blank line
/// is either a Markdown table row (`|`-fenced) or an indented
/// comma-separated header dump. These exist only as layout evidence and
/// duplicate the per-field doc comments, so they are dropped wholesale.
fn is_wire_layout_block(paragraph: &str) -> bool {
    let lines: Vec<&str> = paragraph.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return false;
    }
    lines.iter().all(|line| {
        let trimmed = line.trim_start();
        let is_table_row = trimmed.starts_with('|');
        let is_indented_header_dump = line.starts_with(' ') && trimmed.contains(',');
        is_table_row || is_indented_header_dump
    })
}

/// Remove sentences that record wire-layout provenance from a prose
/// paragraph, keeping every other sentence. A sentence is a run terminated
/// by `. ` (or the paragraph end); a sentence is provenance when it names
/// the verification act or the build it was checked against.
fn drop_provenance_sentences(paragraph: &str) -> String {
    let flat = paragraph.split_whitespace().collect::<Vec<_>>().join(" ");
    let sentences = split_sentences(&flat);
    // Leave paragraphs without provenance byte-for-byte intact (including
    // their original line wrapping); only reflow when a sentence is dropped.
    if !sentences.iter().any(|s| is_provenance_sentence(s)) {
        return paragraph.to_string();
    }
    sentences
        .into_iter()
        .filter(|s| !is_provenance_sentence(s))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split a single-line (whitespace-collapsed) paragraph into sentences on
/// `. ` boundaries, keeping each sentence's terminating punctuation.
fn split_sentences(flat: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let bytes = flat.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'.' && (i + 1 == bytes.len() || bytes[i + 1] == b' ') {
            let end = (i + 1).min(flat.len());
            sentences.push(flat[start..end].trim());
            start = (i + 2).min(flat.len());
        }
    }
    if start < flat.len() {
        sentences.push(flat[start..].trim());
    }
    sentences.into_iter().filter(|s| !s.is_empty()).collect()
}

/// Whether a sentence records where or against what a wire layout was
/// checked, rather than describing tick behavior.
fn is_provenance_sentence(sentence: &str) -> bool {
    let s = sentence.to_ascii_lowercase();
    s.contains("verified-live") || s.contains("jar build")
}
