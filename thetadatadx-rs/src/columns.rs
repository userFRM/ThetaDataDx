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
    /// The response's `symbol` (root) header value, when the wire carried
    /// one. Option + index historical endpoints send a `symbol` column
    /// constant across the response (the queried underlying); stock
    /// endpoints do not. It is not a tick-struct field (the flat POD ticks
    /// hold no per-row `String`), so it rides here once and the projected
    /// builders broadcast it as the first Arrow/Polars column. `None` for
    /// stock responses and every hand-built / streaming frame.
    symbol: Option<Box<str>>,
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
            symbol: None,
        }
    }

    /// Attach the response's constant `symbol` (root) value, so the projected
    /// builders emit it as the leading broadcast column. The decode seam calls
    /// this once per response after reading the wire `symbol`/`root` header;
    /// binding decode seams reconstruct it the same way.
    #[must_use]
    pub fn with_symbol<S: Into<Box<str>>>(mut self, symbol: S) -> Self {
        self.symbol = Some(symbol.into());
        self
    }

    /// The response's constant `symbol` (root), when the wire carried one.
    /// `None` for stock responses and hand-built / streaming frames.
    #[must_use]
    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
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

/// Resolve the present schema-column set for one response from its wire
/// `headers`, shared by every generated [`WireColumns::present_columns`].
///
/// `schema_columns` is the tick's `(wire_name, field)` list in schema order.
/// Claiming is two-pass so ordering is never a latent invariant: exact
/// header matches claim first, then aliases claim any remaining header. One
/// physical header is claimed once (first-claim), so an aliased column never
/// starves a column that names the header exactly.
///
/// Two derived fields are exempt from first-claim because they are not their
/// own physical column:
///
///   * `date` is the `YYYYMMDD` split of a `Timestamp` header — it shares the
///     physical column with a `*_ms_of_day` field but carries distinct data,
///     so it is present whenever it resolves. It only claims a header on an
///     exact standalone `date` column; on the shared-`Timestamp` path the
///     ms-of-day field owns the claim.
///   * `midpoint` (`QuoteTick`) is computed at decode from `bid` + `ask` and
///     is never a wire header, so it is present exactly when both inputs are.
///
/// Any wire header no schema column claimed (an upstream rename or a new
/// column) is logged once so the silent drop is observable.
#[must_use]
pub fn present_columns_from(
    headers: &[&str],
    schema_columns: &[(&'static str, &'static str)],
    contract_id: bool,
    derive_midpoint: bool,
) -> ColumnPresence {
    use crate::mdds::decode::headers::find_header;

    let mut claimed = vec![false; headers.len()];
    let mut present: Vec<&'static str> = Vec::new();

    // Pass 1: exact header matches claim first.
    for &(wire, field) in schema_columns {
        if field == "date" {
            continue;
        }
        if let Some(i) = headers.iter().position(|&h| h == wire) {
            if !claimed[i] {
                claimed[i] = true;
                present.push(field);
            }
        }
    }
    // Pass 2: alias matches claim any header still unclaimed.
    for &(wire, field) in schema_columns {
        if field == "date" || headers.contains(&wire) {
            continue;
        }
        if let Some(i) = find_header(headers, wire) {
            if !claimed[i] {
                claimed[i] = true;
                present.push(field);
            }
        }
    }
    // `date`: present whenever it resolves; claims only an exact standalone
    // header, never the shared `Timestamp` the ms-of-day field owns.
    if schema_columns.iter().any(|&(_, f)| f == "date") {
        if let Some(i) = headers.iter().position(|&h| h == "date") {
            claimed[i] = true;
            present.push("date");
        } else if find_header(headers, "date").is_some() {
            present.push("date");
        }
    }
    // Contract-identity trio: injected under exact wire names on wildcard
    // responses; each claims its exact header.
    if contract_id {
        for name in ["expiration", "strike", "right"] {
            if let Some(i) = headers.iter().position(|&h| h == name) {
                claimed[i] = true;
                present.push(name);
            }
        }
    }
    // `midpoint` rides whenever both its inputs do.
    if derive_midpoint && present.contains(&"bid") && present.contains(&"ask") {
        present.push("midpoint");
    }
    // Observability: a wire header no column claimed is a rename or a new
    // upstream column silently absent from the frame. Surface it once.
    let unclaimed: Vec<&str> = headers
        .iter()
        .zip(&claimed)
        .filter_map(|(&h, &c)| (!c).then_some(h))
        .collect();
    if !unclaimed.is_empty() {
        tracing::warn!(
            target: "thetadatadx::columns",
            unclaimed = ?unclaimed,
            headers = ?headers,
            "response carried wire headers no schema column claimed; they are absent from the projected frame",
        );
    }

    ColumnPresence::from_names(present)
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

/// Wrap hand-built rows (a `Vec` that never crossed the wire) as a `Ticks`
/// carrying the full-schema presence — every column present — so a frame built
/// from it matches the all-columns `to_arrow`. Rows a decode produced are
/// paired with their wire presence via [`Ticks::new`] instead.
impl<T: WireColumns> From<Vec<T>> for Ticks<T> {
    fn from(rows: Vec<T>) -> Self {
        Self {
            rows,
            columns: T::all_columns(),
        }
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

    /// `date` rides the shared `created` Timestamp header: both the ms-of-day
    /// field and `date` are present even though they resolve to one column.
    #[test]
    fn present_date_rides_shared_timestamp() {
        const COLS: &[(&str, &str)] = &[
            ("created", "created_ms_of_day"),
            ("open", "open"),
            ("date", "date"),
        ];
        let p = present_columns_from(&["created", "open"], COLS, false, false);
        assert!(p.contains("created_ms_of_day"));
        assert!(p.contains("open"));
        assert!(
            p.contains("date"),
            "date must ride the shared Timestamp column"
        );
    }

    /// A real standalone `date` header is present and does not leave the
    /// header unclaimed (it claims its own exact column).
    #[test]
    fn present_date_standalone_header() {
        const COLS: &[(&str, &str)] = &[("ms_of_day", "ms_of_day"), ("date", "date")];
        let p = present_columns_from(&["timestamp", "date"], COLS, false, false);
        assert!(p.contains("ms_of_day"));
        assert!(p.contains("date"));
    }

    /// `midpoint` is present exactly when both `bid` and `ask` are.
    #[test]
    fn present_midpoint_keys_on_bid_and_ask() {
        const COLS: &[(&str, &str)] = &[
            ("ms_of_day", "ms_of_day"),
            ("bid", "bid"),
            ("ask", "ask"),
            ("date", "date"),
        ];
        let with = present_columns_from(&["ms_of_day", "bid", "ask", "date"], COLS, false, true);
        assert!(with.contains("midpoint"));

        let without = present_columns_from(&["ms_of_day", "date"], COLS, false, true);
        assert!(!without.contains("midpoint"));
    }

    /// Two-pass claiming: a column that names a header exactly claims it even
    /// when an alias-matching column is listed first, so ordering is not a
    /// latent invariant (`root` aliases to `symbol`; the exact `symbol` column
    /// must own the `symbol` header).
    #[test]
    fn present_exact_match_beats_earlier_alias() {
        const COLS: &[(&str, &str)] = &[("root", "root"), ("symbol", "symbol")];
        let p = present_columns_from(&["symbol"], COLS, false, false);
        assert!(
            p.contains("symbol"),
            "exact `symbol` column claims the header"
        );
        assert!(!p.contains("root"), "the aliased `root` must not steal it");
    }
}
