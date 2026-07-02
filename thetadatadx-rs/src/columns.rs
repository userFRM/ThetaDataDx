//! Per-response column presence — the wire's actual column set for one
//! decoded response.
//!
//! MDDS gRPC responses carry only the columns the endpoint populates for
//! that request: `stock_history_trade` and `option_history_trade` are both
//! `TradeTick`, yet the option response injects the
//! `expiration`/`strike`/`right` contract-identity trio while the stock
//! response omits it; neither sends the four trade-flag columns
//! (`condition_flags`/`price_flags`/`volume_type`/`records_back`) the
//! flat-file path carries. The tick struct is a fixed superset of every
//! endpoint's columns (fields absent on the wire decode to their seed
//! value), so the struct alone cannot say which columns the response
//! actually contained.
//!
//! [`ColumnPresence`] captures exactly that: the set of schema-column names
//! present on one response's wire, in schema order. The decode produces it
//! from the response `DataTable.headers`; the Arrow / Polars builders read
//! it to emit only the present columns (terminal-exact output — the SDK
//! emits what the terminal emits, no superset). Hand-built tick slices that
//! never touched the wire have no presence and default to "every schema
//! column present" so they behave as a plain full-schema frame.

/// The set of schema-column names present on one MDDS response's wire.
///
/// Built by the decode from the response header list and carried alongside
/// the decoded rows so the DataFrame builders project to the wire's exact
/// column set. Column names are the public schema field names (e.g.
/// `"condition_flags"`, `"expiration"`), not the wire spellings the alias
/// table resolves — the builders key on the field name.
///
/// A `ColumnPresence` is always compiled (it crosses the transport /
/// binding boundary with the decoded rows); the projection that consumes it
/// lives behind the `arrow` / `polars` features.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ColumnPresence {
    /// Present schema-column names in schema order. Small (≤ ~24 entries),
    /// so a linear scan in [`Self::contains`] beats a hashed set.
    names: Vec<Box<str>>,
}

impl ColumnPresence {
    /// Build a presence set from an explicit list of present schema-column
    /// names (already resolved from wire spellings to schema field names).
    ///
    /// The decode is the only in-crate producer; it is `pub` so binding
    /// crates can reconstruct a presence set at their own decode seams.
    #[must_use]
    pub fn from_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Box<str>>,
    {
        Self {
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    /// `true` when the response carried the column `name` (a schema field
    /// name). Absent columns are omitted from the projected frame.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.names.iter().any(|n| n.as_ref() == name)
    }

    /// The present schema-column names, in schema order.
    pub fn present_names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(Box::as_ref)
    }

    /// Number of present columns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// `true` when no column is present (an empty header list).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// A tick type that knows which of its schema columns a response's wire
/// header list actually carried.
///
/// Implemented (generated) for every decoded tick type from the same
/// `tick_schema.toml` column list the parser uses, so
/// [`present_columns`](Self::present_columns) and the parser resolve the
/// wire against identical alias-aware lookups. The direct-client seam calls
/// it once per response — with `table.headers` in scope — and carries the
/// result alongside the decoded rows.
pub trait WireColumns {
    /// The schema columns present on a response whose wire header list is
    /// `headers`, as a [`ColumnPresence`] naming the public schema fields.
    fn present_columns(headers: &[&str]) -> ColumnPresence;

    /// Every column the tick type's full-schema frame carries, in schema
    /// order — the "all present" set for hand-built rows a caller assembled
    /// itself (which never touched a wire) and for the streaming collect
    /// path (which drains per-chunk slices and keeps no header list). A
    /// frame built from this set matches `TicksArrowExt::to_arrow` (in the
    /// `arrow`-gated `crate::frames` module).
    fn all_columns() -> ColumnPresence;
}

/// A decoded historical response: the tick rows plus the set of columns the
/// response's wire actually carried.
///
/// The buffered (`.await`) return of every historical endpoint. It derefs to
/// `[T]`, so it reads like the `Vec<T>` it replaced — `.len()`, `.iter()`,
/// indexing, `for row in &ticks`, `ticks.first()` all work — while carrying
/// the [`ColumnPresence`] the DataFrame terminals need. Use `to_arrow` /
/// `to_polars` (feature-gated on `arrow` / `polars`) for a terminal-exact
/// frame (only the wire's columns), or [`into_vec`](Self::into_vec) to drop
/// the presence and take the rows.
#[derive(Debug, Clone)]
pub struct Ticks<T> {
    rows: Vec<T>,
    columns: ColumnPresence,
}

impl<T> Ticks<T> {
    /// Pair decoded `rows` with the `columns` its response carried.
    #[must_use]
    pub fn new(rows: Vec<T>, columns: ColumnPresence) -> Self {
        Self { rows, columns }
    }

    /// The columns the response's wire carried.
    #[must_use]
    pub fn columns(&self) -> &ColumnPresence {
        &self.columns
    }

    /// The tick rows, dropping the column-presence set.
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        self.rows
    }

    /// The tick rows as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.rows
    }
}

impl<T> std::ops::Deref for Ticks<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.rows
    }
}

impl<T> IntoIterator for Ticks<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Ticks<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.iter()
    }
}

#[cfg(feature = "arrow")]
#[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
impl<T> Ticks<T>
where
    [T]: crate::frames::TicksArrowExt,
{
    /// Materialise the rows as an Arrow `RecordBatch` carrying only the
    /// columns the response's wire sent — terminal-exact.
    ///
    /// # Errors
    ///
    /// Returns an [`arrow_schema::ArrowError`] when the column arrays cannot
    /// be assembled into a `RecordBatch`.
    pub fn to_arrow(
        &self,
    ) -> ::core::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError> {
        crate::frames::TicksArrowExt::to_arrow_projected(self.as_slice(), &self.columns)
    }
}

#[cfg(feature = "polars")]
#[cfg_attr(docsrs, doc(cfg(feature = "polars")))]
impl<T> Ticks<T>
where
    [T]: crate::frames::TicksPolarsExt,
{
    /// Materialise the rows as a polars `DataFrame` carrying only the columns
    /// the response's wire sent — terminal-exact.
    ///
    /// # Errors
    ///
    /// Returns a [`polars::prelude::PolarsError`] when polars rejects the
    /// assembled columns.
    pub fn to_polars(&self) -> polars::prelude::PolarsResult<polars::prelude::DataFrame> {
        crate::frames::TicksPolarsExt::to_polars_projected(self.as_slice(), &self.columns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_matches_present_names_only() {
        let p = ColumnPresence::from_names(["ms_of_day", "price", "condition"]);
        assert!(p.contains("ms_of_day"));
        assert!(p.contains("price"));
        assert!(!p.contains("condition_flags"));
        assert!(!p.contains("expiration"));
        assert_eq!(p.len(), 3);
        assert!(!p.is_empty());
    }

    #[test]
    fn default_is_empty() {
        assert!(ColumnPresence::default().is_empty());
        assert_eq!(ColumnPresence::default().present_names().count(), 0);
    }

    #[test]
    fn present_names_preserve_order() {
        let p = ColumnPresence::from_names(["a", "b", "c"]);
        let got: Vec<&str> = p.present_names().collect();
        assert_eq!(got, ["a", "b", "c"]);
    }
}
