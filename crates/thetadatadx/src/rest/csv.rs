//! Minimal CSV decoder for the local Terminal's `format=csv` responses.
//!
//! The Terminal serves CSV bodies in the shape:
//!
//! ```text
//! ms_of_day,bid_size,bid,ask_size,ask,date
//! 34200000,50,1.5022,75,1.5041,20220414
//! 34200500,...
//! ```
//!
//! The decoder is hand-written instead of pulling in the `csv` crate
//! for two reasons:
//!
//! 1. The bodies we need to parse are flat numeric tables; there are
//!    no embedded quotes, no escapes, and no UTF-8 multi-byte fields.
//!    A 60-line hand parser covers the contract.
//!
//! 2. Adding a new workspace dep for one transport's wire format would
//!    couple the rest of the crate's build graph to it. The cost of
//!    maintaining a small parser here is lower than the long-term cost
//!    of an extra crate in the lockfile.
//!
//! The decoder is **lenient on absent columns** -- the legacy 6-field
//! NBBO layout (`ms_of_day, bid_size, bid, ask_size, ask, date`) is
//! served verbatim by the patched Terminal on REST for 2022-era rows;
//! when the four exchange / condition columns are absent from the
//! header, [`Table::column_index`] returns `None` and downstream row
//! decoders default the absent fields to 0 -- the same contract as the
//! gRPC path's `opt_number(row, None) -> 0` arm (see
//! `crates/thetadatadx/build_support/ticks/parser.rs`).

use super::error::RestError;

/// Parsed CSV body: an owned header vector + a slice-by-line view of
/// the remaining rows. Holding the rows as `&'a str` slices lets the
/// row decoders avoid per-row `String` allocations on the hot path.
#[derive(Debug)]
pub(crate) struct Table<'a> {
    pub(crate) headers: Vec<String>,
    pub(crate) rows: Vec<Vec<&'a str>>,
}

impl<'a> Table<'a> {
    /// Parse a CSV body. The first non-empty line is the header.
    ///
    /// # Errors
    ///
    /// Returns [`RestError::CsvDecode`] when the body is empty or
    /// every line has zero columns.
    pub(crate) fn parse(body: &'a str) -> Result<Self, RestError> {
        let mut lines = body.lines().filter(|l| !l.trim().is_empty());
        let header_line = lines.next().ok_or_else(|| RestError::CsvDecode {
            reason: "empty body".to_string(),
            row: usize::MAX,
        })?;
        let headers: Vec<String> = header_line
            .split(',')
            .map(str::trim)
            .map(String::from)
            .collect();
        if headers.is_empty() {
            return Err(RestError::CsvDecode {
                reason: "header row has zero columns".to_string(),
                row: usize::MAX,
            });
        }

        let rows: Vec<Vec<&str>> = lines
            .map(|l| l.split(',').map(str::trim).collect())
            .collect();
        Ok(Self { headers, rows })
    }

    /// Resolve a schema-side column name to its 0-based index in the
    /// header row. Honours the shared
    /// [`crate::mdds::decode::headers::HEADER_ALIASES`] table for cross-
    /// transport parity with the gRPC decoder.
    pub(crate) fn column_index(&self, name: &str) -> Option<usize> {
        // Direct match.
        if let Some(i) = self.headers.iter().position(|h| h == name) {
            return Some(i);
        }
        // Alias fallback. The REST and gRPC wire layers share the same
        // upstream column-rename history; the alias table is the single
        // source of truth.
        for &(schema_name, server_name) in crate::mdds::decode::headers::HEADER_ALIASES {
            if name == schema_name {
                if let Some(i) = self.headers.iter().position(|h| h == server_name) {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Decode a column as `i32`, defaulting to 0 when the cell is
    /// empty or the column is absent. Used for optional / legacy-fill
    /// columns (`bid_exchange`, `bid_condition`, ...) where the
    /// patched-Terminal `normalizeData()` upcast contract zero-fills
    /// absent fields.
    pub(crate) fn cell_i32_or_zero(&self, row_idx: usize, col_idx: Option<usize>) -> i32 {
        let Some(col) = col_idx else {
            return 0;
        };
        self.rows
            .get(row_idx)
            .and_then(|r| r.get(col))
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(0)
    }

    /// Decode a column as `i32`, surfacing a structured error on a
    /// missing column or a non-numeric cell. Used for the columns that
    /// the schema declares `required` -- `ms_of_day`, `date`.
    pub(crate) fn cell_i32_required(
        &self,
        row_idx: usize,
        col_idx: Option<usize>,
        column: &'static str,
    ) -> Result<i32, RestError> {
        let Some(col) = col_idx else {
            return Err(RestError::MissingColumn {
                column,
                available: self.headers.join(","),
            });
        };
        let cell = self
            .rows
            .get(row_idx)
            .and_then(|r| r.get(col))
            .ok_or_else(|| RestError::CsvDecode {
                reason: format!("row missing cell at column {col} ({column})"),
                row: row_idx,
            })?;
        cell.parse::<i32>().map_err(|e| RestError::CsvDecode {
            reason: format!("column {column}: expected i32, got {cell:?}: {e}"),
            row: row_idx,
        })
    }

    /// Decode a column as `f64`, defaulting to 0.0 when the cell is
    /// empty or the column is absent. Used by `bid` / `ask` / Greek
    /// columns where the REST wire payload is the already-scaled
    /// floating-point quote (the Terminal applies the
    /// `value * 10^(price_type - 10)` scaling before serializing CSV).
    pub(crate) fn cell_f64_or_zero(&self, row_idx: usize, col_idx: Option<usize>) -> f64 {
        let Some(col) = col_idx else {
            return 0.0;
        };
        self.rows
            .get(row_idx)
            .and_then(|r| r.get(col))
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_legacy_six_field_quote_csv() {
        let body = "\
ms_of_day,bid_size,bid,ask_size,ask,date
34200000,50,1.5022,75,1.5041,20220414
34200500,55,1.5023,80,1.5042,20220414
";
        let t = Table::parse(body).unwrap();
        assert_eq!(t.headers.len(), 6);
        assert_eq!(t.rows.len(), 2);

        // Wire-present columns resolve.
        assert_eq!(t.column_index("ms_of_day"), Some(0));
        assert_eq!(t.column_index("bid"), Some(2));
        assert_eq!(t.column_index("date"), Some(5));

        // Wire-absent columns return None -- the row decoder will
        // zero-fill them via cell_i32_or_zero.
        assert_eq!(t.column_index("bid_exchange"), None);
        assert_eq!(t.column_index("ask_condition"), None);

        // Row 0 decodes bit-exact for present columns; absent columns
        // zero-fill via cell_i32_or_zero.
        let bid_exg_idx = t.column_index("bid_exchange");
        assert_eq!(t.cell_i32_or_zero(0, bid_exg_idx), 0);
        let bid_size_idx = t.column_index("bid_size");
        assert_eq!(t.cell_i32_or_zero(0, bid_size_idx), 50);
    }

    #[test]
    fn parse_current_eleven_field_quote_csv() {
        let body = "\
ms_of_day,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date
34200000,50,7,1.5022,1,75,8,1.5041,2,20240605
";
        let t = Table::parse(body).unwrap();
        assert_eq!(t.column_index("bid_exchange"), Some(2));
        assert_eq!(t.column_index("ask_condition"), Some(8));

        let bid_exg_idx = t.column_index("bid_exchange");
        assert_eq!(t.cell_i32_or_zero(0, bid_exg_idx), 7);
    }

    #[test]
    fn empty_body_errors() {
        assert!(matches!(
            Table::parse(""),
            Err(RestError::CsvDecode {
                row: usize::MAX,
                ..
            })
        ));
    }

    #[test]
    fn cell_i32_required_errors_on_missing_column() {
        let body = "ms_of_day,bid\n100,1.5\n";
        let t = Table::parse(body).unwrap();
        assert!(matches!(
            t.cell_i32_required(0, t.column_index("missing_col"), "missing_col"),
            Err(RestError::MissingColumn {
                column: "missing_col",
                ..
            })
        ));
    }
}
