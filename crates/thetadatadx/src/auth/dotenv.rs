//! Shared `.env`-file parsing.
//!
//! A single parser backs both credential sourcing
//! ([`crate::auth::Credentials::from_dotenv`]) and configuration sourcing
//! ([`crate::config::DirectConfig::from_dotenv`]). One file can therefore
//! carry both the credential keys (`THETADATA_API_KEY`, or
//! `THETADATA_EMAIL` + `THETADATA_PASSWORD`) and the environment selector
//! (`THETADATA_MDDS_TYPE`, plus the documented host overrides); the
//! credential reader picks up the secret keys and the configuration reader
//! picks up the cluster keys, from the same parse.
//!
//! # Grammar
//!
//! The common `.env` subset: one `KEY=VALUE` assignment per line, with an
//! optional `export ` prefix, `#` comment lines, blank lines, and optional
//! matching single or double quotes around the value. Whitespace around the
//! key and the (unquoted) value is trimmed. A later assignment to the same
//! key wins, matching shell `source` semantics.
//!
//! # Secret handling
//!
//! The parser borrows the caller's buffer; it never copies a value into a
//! fresh allocation. Callers that read a `.env` which may contain
//! `THETADATA_API_KEY` keep the buffer in [`zeroize::Zeroizing`] so the
//! on-disk secret bytes are wiped on drop — the borrowed value slices share
//! that backing buffer rather than escaping into an unmanaged `String`.

/// Look up `key` in the parsed `.env` assignments, returning the trimmed
/// value when present and non-empty.
///
/// The returned slice borrows the buffer that owns the file contents (wrapped
/// in [`zeroize::Zeroizing`] on the credential path), so the matched value is
/// never copied into a separate plain `String` before it reaches the caller.
pub(crate) fn lookup<'a>(pairs: &'a [(String, &'a str)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| *value)
        .filter(|value| !value.is_empty())
}

/// Parse `.env`-format text into `(key, value)` pairs.
///
/// See the [module docs](self) for the grammar and the secret-handling
/// contract. The value slices borrow `contents`; the caller owns that buffer
/// so no secret value is copied into an unmanaged allocation here.
pub(crate) fn parse(contents: &str) -> Vec<(String, &str)> {
    let mut pairs: Vec<(String, &str)> = Vec::new();
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map_or(line, str::trim_start);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        if key.is_empty() {
            continue;
        }
        let value = value.trim();
        // Strip one layer of matching surrounding quotes; leave the
        // inner bytes verbatim (no escape processing — secrets are
        // opaque).
        let value = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        // Last assignment wins.
        if let Some(slot) = pairs.iter_mut().find(|(name, _)| name == &key) {
            slot.1 = value;
        } else {
            pairs.push((key, value));
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parser handles comments, blanks, quotes, `export`, and
    /// last-assignment-wins without touching the filesystem.
    #[test]
    fn parse_grammar() {
        let pairs = parse(
            "# comment\n\n  export A = \"one\" \nB='two'\nA=three\nbad-line-no-eq\n=novalue\n",
        );
        assert_eq!(lookup(&pairs, "A"), Some("three"));
        assert_eq!(lookup(&pairs, "B"), Some("two"));
        assert_eq!(lookup(&pairs, "MISSING"), None);
    }

    #[test]
    fn lookup_treats_empty_value_as_absent() {
        let pairs = parse("A=\nB=value\n");
        assert_eq!(lookup(&pairs, "A"), None);
        assert_eq!(lookup(&pairs, "B"), Some("value"));
    }
}
