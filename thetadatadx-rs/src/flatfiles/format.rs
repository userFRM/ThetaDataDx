//! The formats a flat file is written in: the terminal's.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// The format a flat file is written in. Each is the terminal's own for the
/// same file, byte for byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatFileFormat {
    /// CSV: a header line of the column names, then one line per row with
    /// text quoted, integers bare and an empty cell for a missing value, and
    /// one blank line at the end.
    Csv,
    /// One JSON object per row, one row per line (the terminal's `ndjson`).
    Jsonl,
    /// One JSON object, `{"response": [...]}`, holding an object per row.
    Json,
    /// One JSON object holding an array per column (the terminal's
    /// `json_legacy`). No column closes before the last row, so the whole
    /// file is held in memory until then.
    JsonLegacy,
    /// The terminal's HTML page: one table, a header row and a row per row.
    Html,
}

impl FlatFileFormat {
    /// Canonical file extension (without the leading dot).
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Jsonl => "jsonl",
            Self::Json | Self::JsonLegacy => "json",
            Self::Html => "html",
        }
    }

    /// Append `extension()` to `path` if `path` has no extension at all.
    /// If `path` already carries any extension, it is preserved as-is.
    /// Lets callers pass a bare base name and still land on a
    /// correctly-named file.
    #[must_use]
    pub fn ensure_extension(self, path: &Path) -> PathBuf {
        match path.extension() {
            Some(_) => path.to_path_buf(),
            None => path.with_extension(self.extension()),
        }
    }
}

/// The format's name, as [`FromStr`] reads it.
impl fmt::Display for FlatFileFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::JsonLegacy => "json_legacy",
            other => other.extension(),
        })
    }
}

/// Parses the format case-insensitively: `csv`, `json`, `json_legacy`,
/// `jsonl` or its alias `ndjson`, and `html`.
impl FromStr for FlatFileFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "csv" => Ok(Self::Csv),
            "json" => Ok(Self::Json),
            "json_legacy" => Ok(Self::JsonLegacy),
            "jsonl" | "ndjson" => Ok(Self::Jsonl),
            "html" => Ok(Self::Html),
            _ => Err(format!(
                "format must be csv, json, json_legacy, jsonl, ndjson or html, got {s:?}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_round_trip() {
        assert_eq!(FlatFileFormat::Csv.extension(), "csv");
        assert_eq!(FlatFileFormat::Jsonl.extension(), "jsonl");
        assert_eq!(FlatFileFormat::Json.extension(), "json");
        assert_eq!(FlatFileFormat::Html.extension(), "html");
    }

    #[test]
    fn ensure_extension_appends_when_missing() {
        let p = Path::new("/tmp/foo");
        assert_eq!(
            FlatFileFormat::Csv.ensure_extension(p),
            PathBuf::from("/tmp/foo.csv")
        );
        assert_eq!(
            FlatFileFormat::Jsonl.ensure_extension(p),
            PathBuf::from("/tmp/foo.jsonl")
        );
    }

    #[test]
    fn ensure_extension_preserves_existing() {
        let p = Path::new("/tmp/foo.json");
        // Already has an extension: leave it alone (caller's intent).
        assert_eq!(
            FlatFileFormat::Jsonl.ensure_extension(p),
            PathBuf::from("/tmp/foo.json")
        );
    }
}
