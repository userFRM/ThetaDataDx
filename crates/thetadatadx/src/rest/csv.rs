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
//! The decoder is **lenient on absent columns** -- defense-in-depth
//! against subset NBBO layouts the upstream may emit for older
//! storage tiers (`ms_of_day, bid_size, bid, ask_size, ask, date`
//! without the exchange / condition columns). When the four
//! exchange / condition columns are absent from the header, the
//! table's `column_index` lookup returns `None` and the downstream
//! row decoders default the absent fields to 0 -- the same contract
//! as the gRPC path's `opt_number(row, None) -> 0` arm (see
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

    /// Decode a column as `i32` with the following three-way contract:
    ///
    /// * `None` column index -> `Ok(0)`. The header lacks the column
    ///   entirely (subset NBBO row missing the four exchange /
    ///   condition columns); absent fields zero-fill, mirroring the
    ///   gRPC decoder's `opt_number(row, None) -> 0` contract.
    /// * `Some` index but `""` cell -> `Ok(0)`. The Terminal serializes
    ///   a deliberately-null cell as an empty string; absence semantics
    ///   apply, same as a missing column.
    /// * `Some` index, non-empty cell that fails to parse -> structured
    ///   [`RestError::CsvDecode`] with the row index, column number,
    ///   and offending cell value. Previously the parse failure was
    ///   silently coerced to `0`, hiding decoder bugs and upstream
    ///   wire-format drift.
    ///
    /// # Errors
    ///
    /// Returns [`RestError::CsvDecode`] when a non-empty cell does not
    /// parse as `i32`.
    pub(crate) fn cell_i32_or_zero(
        &self,
        row_idx: usize,
        col_idx: Option<usize>,
    ) -> Result<i32, RestError> {
        let Some(col) = col_idx else {
            return Ok(0);
        };
        let Some(cell) = self.rows.get(row_idx).and_then(|r| r.get(col)) else {
            return Ok(0);
        };
        if cell.is_empty() {
            return Ok(0);
        }
        cell.parse::<i32>().map_err(|e| RestError::CsvDecode {
            reason: format!("bad i32 at row {row_idx} col {col}: {cell:?}: {e}"),
            row: row_idx,
        })
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

    /// Decode a column as `f64` with the same three-way absent /
    /// empty / malformed contract as [`Self::cell_i32_or_zero`].
    /// Additionally rejects `NaN` and `±Inf`: Rust's `f64::from_str`
    /// happily parses the literal string `"NaN"` as a number, but
    /// `NaN` propagating into a `QuoteTick` field is observably wrong
    /// (every downstream `==`, `<`, `>` comparison silently returns
    /// `false`) and indicates upstream wire-format corruption, not a
    /// legitimate zero.
    ///
    /// # Errors
    ///
    /// Returns [`RestError::CsvDecode`] when a non-empty cell does not
    /// parse as a finite `f64`.
    pub(crate) fn cell_f64_or_zero(
        &self,
        row_idx: usize,
        col_idx: Option<usize>,
    ) -> Result<f64, RestError> {
        let Some(col) = col_idx else {
            return Ok(0.0);
        };
        let Some(cell) = self.rows.get(row_idx).and_then(|r| r.get(col)) else {
            return Ok(0.0);
        };
        if cell.is_empty() {
            return Ok(0.0);
        }
        let parsed = cell.parse::<f64>().map_err(|e| RestError::CsvDecode {
            reason: format!("bad f64 at row {row_idx} col {col}: {cell:?}: {e}"),
            row: row_idx,
        })?;
        if !parsed.is_finite() {
            return Err(RestError::CsvDecode {
                reason: format!(
                    "non-finite f64 at row {row_idx} col {col}: {cell:?} parsed as {parsed}"
                ),
                row: row_idx,
            });
        }
        Ok(parsed)
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
        assert_eq!(t.cell_i32_or_zero(0, bid_exg_idx).unwrap(), 0);
        let bid_size_idx = t.column_index("bid_size");
        assert_eq!(t.cell_i32_or_zero(0, bid_size_idx).unwrap(), 50);
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
        assert_eq!(t.cell_i32_or_zero(0, bid_exg_idx).unwrap(), 7);
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

    // -- cell_i32_or_zero three-arm contract -----------------------

    /// Empty cell on a present column zero-fills (Terminal-null
    /// contract); does NOT surface as a parse error.
    #[test]
    fn cell_i32_or_zero_empty_cell_returns_zero() {
        let body = "ms_of_day,bid_size,date\n100,,20240605\n";
        let t = Table::parse(body).unwrap();
        let bid_size_idx = t.column_index("bid_size");
        assert_eq!(t.cell_i32_or_zero(0, bid_size_idx).unwrap(), 0);
    }

    /// Non-empty cell that does not parse as i32 must surface a
    /// structured `CsvDecode` carrying the offending cell + row +
    /// column. The pre-M4 implementation silently returned 0.
    #[test]
    fn cell_i32_or_zero_malformed_cell_errors() {
        let body = "ms_of_day,bid_size,date\n100,abc,20240605\n";
        let t = Table::parse(body).unwrap();
        let bid_size_idx = t.column_index("bid_size");
        let err = t.cell_i32_or_zero(0, bid_size_idx).unwrap_err();
        match err {
            RestError::CsvDecode { reason, row } => {
                assert_eq!(row, 0);
                assert!(reason.contains("\"abc\""), "reason: {reason}");
                assert!(reason.contains("i32"), "reason: {reason}");
            }
            other => panic!("expected CsvDecode, got {other:?}"),
        }
    }

    // -- cell_f64_or_zero three-arm contract + NaN reject ----------

    /// Same empty-cell contract as i32.
    #[test]
    fn cell_f64_or_zero_empty_cell_returns_zero() {
        let body = "ms_of_day,bid,date\n100,,20240605\n";
        let t = Table::parse(body).unwrap();
        let bid_idx = t.column_index("bid");
        assert!(t.cell_f64_or_zero(0, bid_idx).unwrap().abs() < 1e-12);
    }

    /// Malformed f64 cell errors with row + column in the message.
    #[test]
    fn cell_f64_or_zero_malformed_cell_errors() {
        let body = "ms_of_day,bid,date\n100,nope,20240605\n";
        let t = Table::parse(body).unwrap();
        let bid_idx = t.column_index("bid");
        let err = t.cell_f64_or_zero(0, bid_idx).unwrap_err();
        assert!(matches!(err, RestError::CsvDecode { row: 0, .. }));
    }

    /// The literal string `NaN` parses successfully as `f64::NAN` on
    /// Rust's `f64::from_str`. Letting it through would propagate a
    /// silently-broken value into every downstream comparison; the
    /// decoder must reject it as wire-format corruption.
    #[test]
    fn cell_f64_or_zero_rejects_nan_literal() {
        for bad in ["NaN", "nan", "NAN", "+nan", "-nan"] {
            let body = format!("ms_of_day,bid,date\n100,{bad},20240605\n");
            let t = Table::parse(&body).unwrap();
            let bid_idx = t.column_index("bid");
            let err = t.cell_f64_or_zero(0, bid_idx).expect_err(bad);
            assert!(
                matches!(err, RestError::CsvDecode { .. }),
                "{bad} should yield CsvDecode, got {err:?}"
            );
        }
    }

    /// `inf` / `-inf` / `Inf` likewise parse but are non-finite;
    /// rejected for the same reason as `NaN`.
    #[test]
    fn cell_f64_or_zero_rejects_infinities() {
        for bad in ["inf", "-inf", "Inf", "+infinity", "infinity"] {
            let body = format!("ms_of_day,bid,date\n100,{bad},20240605\n");
            let t = Table::parse(&body).unwrap();
            let bid_idx = t.column_index("bid");
            let err = t.cell_f64_or_zero(0, bid_idx).expect_err(bad);
            assert!(
                matches!(err, RestError::CsvDecode { .. }),
                "{bad} should yield CsvDecode, got {err:?}"
            );
        }
    }
}
