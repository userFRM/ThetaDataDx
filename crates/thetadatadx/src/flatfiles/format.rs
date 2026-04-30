//! Output format selector for the FLATFILES surface.
//!
//! The same logical row stream — `(contract_keys, [data_columns])` —
//! materializes to one of two on-disk formats:
//!
//! - `Csv`: vendor byte-format (lowercase headers, comma-separated, no
//!   quoting, Unix line-endings). Used for legacy interop and as the
//!   gold-standard for the byte-match integration test.
//! - `Jsonl`: one JSON object per line, keys identical to the CSV
//!   column names, integer columns stay numeric (no stringification).
//!
//! For columnar formats (Parquet, Arrow IPC, polars) callers should use
//! the in-memory typed entry point (see `flatfile_request_decoded`) and
//! drive their own writer — keeping the SDK free of heavy columnar
//! dependencies.

use std::fmt;
use std::path::{Path, PathBuf};

/// Selectable output format for any `flatfile_*` SDK call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatFileFormat {
    /// Vendor byte-format CSV. Header on line 1, then `\n`-terminated rows.
    Csv,
    /// JSON Lines — one JSON object per line.
    Jsonl,
}

impl FlatFileFormat {
    /// Canonical file extension (without the leading dot).
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Jsonl => "jsonl",
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

impl fmt::Display for FlatFileFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_round_trip() {
        assert_eq!(FlatFileFormat::Csv.extension(), "csv");
        assert_eq!(FlatFileFormat::Jsonl.extension(), "jsonl");
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
        // Already has an extension — leave it alone (caller's intent).
        assert_eq!(
            FlatFileFormat::Jsonl.ensure_extension(p),
            PathBuf::from("/tmp/foo.json")
        );
    }
}
