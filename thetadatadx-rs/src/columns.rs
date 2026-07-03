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
    /// one constant across every row. Option + index historical endpoints
    /// send a `symbol` column constant across the response (the queried
    /// underlying); single-symbol stock snapshots do too. It is not a
    /// tick-struct field (the flat POD ticks hold no per-row `String`), so it
    /// rides here once and the projected builders broadcast it as the first
    /// Arrow/Polars column. `None` for a multi-symbol response (see
    /// [`Self::symbols`]), for stock history responses, and for every
    /// hand-built / streaming frame.
    symbol: Option<Box<str>>,
    /// The response's per-row `symbol` values, when the wire carried a
    /// `symbol` column that VARIES across rows — a multi-symbol snapshot
    /// (`stock_snapshot_quote(["AAPL","MSFT"])`) attributes each row to its
    /// underlying. Like [`Self::symbol`] it is not a tick-struct field (the
    /// flat POD ticks stay flat), so it rides here once as one value per row
    /// and the projected builders emit it as the leading `symbol` column.
    /// Length equals the decoded row count. Mutually exclusive with
    /// [`Self::symbol`]: the wire's symbol column is either constant
    /// (broadcast) or varying (per-row), never both. `None` when the wire
    /// carried no varying `symbol` column.
    symbols: Option<Vec<Box<str>>>,
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
            symbols: None,
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

    /// Attach the response's per-row `symbol` values, so the projected
    /// builders emit a real per-row `symbol` column attributing each row to
    /// its underlying. The decode seam calls this once per response when the
    /// wire's `symbol` column varies across rows (a multi-symbol snapshot);
    /// `symbols.len()` must equal the row count. Takes precedence over
    /// [`Self::with_symbol`] in the projected builders.
    #[must_use]
    pub fn with_symbols<I, S>(mut self, symbols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Box<str>>,
    {
        self.symbols = Some(symbols.into_iter().map(Into::into).collect());
        self
    }

    /// The response's constant `symbol` (root), when the wire carried one
    /// constant across every row. `None` for multi-symbol responses (see
    /// [`Self::symbols`]), stock history responses, and hand-built /
    /// streaming frames.
    #[must_use]
    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    /// The response's per-row `symbol` values, one per row, when the wire
    /// carried a `symbol` column that varies across rows (a multi-symbol
    /// snapshot). The projected builders emit these as the leading `symbol`
    /// column. `None` when the response has no varying `symbol` column.
    #[must_use]
    pub fn symbols(&self) -> Option<&[Box<str>]> {
        self.symbols.as_deref()
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
/// starves a column that names the header exactly. Present columns are
/// emitted in schema order regardless of which pass claimed them, honouring
/// the [`ColumnPresence::present_names`] contract.
///
/// Two derived fields are handled specially because they are not their own
/// physical column:
///
///   * `date` is resolved after the time fields. When its header is still
///     unclaimed it is the primary claimant (the interest-rate `created` ->
///     `date` column) and claims it; when a `*_ms_of_day` field already
///     claimed the shared `Timestamp` header, `date` is that column's derived
///     `YYYYMMDD` sibling and rides it without a second claim. Either way it
///     is present whenever it resolves.
///   * `midpoint` (`QuoteTick`) is computed at decode from `bid` + `ask` and
///     is never a wire header, so it is present exactly when both inputs are.
#[must_use]
pub fn present_columns_from(
    headers: &[&str],
    schema_columns: &[(&'static str, &'static str)],
    contract_id: bool,
    derive_midpoint: bool,
) -> ColumnPresence {
    use crate::mdds::decode::headers::find_header;

    let mut claimed = vec![false; headers.len()];
    // Per-column claimed header index; `Some` marks the column present.
    let mut owner: Vec<Option<usize>> = vec![None; schema_columns.len()];

    // Pass 1: exact header matches claim first. `date` is deferred (it may be
    // a derived sibling of a `*_ms_of_day` field on the same header).
    for (ci, &(wire, field)) in schema_columns.iter().enumerate() {
        if field == "date" {
            continue;
        }
        if let Some(i) = headers.iter().position(|&h| h == wire) {
            if !claimed[i] {
                claimed[i] = true;
                owner[ci] = Some(i);
            }
        }
    }
    // Pass 2: alias matches claim any header still unclaimed.
    for (ci, &(wire, field)) in schema_columns.iter().enumerate() {
        if field == "date" || owner[ci].is_some() || headers.contains(&wire) {
            continue;
        }
        if let Some(i) = find_header(headers, wire) {
            if !claimed[i] {
                claimed[i] = true;
                owner[ci] = Some(i);
            }
        }
    }
    // `date`: primary claimant when its header is free, derived sibling when a
    // time field already claimed the shared `Timestamp` — present either way.
    for (ci, &(wire, field)) in schema_columns.iter().enumerate() {
        if field == "date" {
            if let Some(i) = find_header(headers, wire) {
                owner[ci] = Some(i);
                // Keep `claimed` consistent: `date` feeds a present column, so
                // its header is claimed (idempotent when a `*_ms_of_day` sibling
                // already claimed the shared `Timestamp`).
                claimed[i] = true;
            }
        }
    }

    // Emit present columns in schema order.
    let mut present: Vec<&'static str> = owner
        .iter()
        .zip(schema_columns)
        .filter_map(|(o, &(_, field))| o.map(|_| field))
        .collect();
    // Contract-identity trio: injected under exact wire names on wildcard
    // responses.
    if contract_id {
        for name in ["expiration", "strike", "right"] {
            if headers.contains(&name) {
                present.push(name);
            }
        }
    }
    // `midpoint` rides whenever both its inputs do.
    if derive_midpoint && present.contains(&"bid") && present.contains(&"ask") {
        present.push("midpoint");
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

    /// The constant broadcast and the per-row vector are distinct carriers:
    /// `with_symbol` populates only `symbol()`, `with_symbols` only `symbols()`.
    #[test]
    fn constant_and_per_row_symbol_are_distinct() {
        let constant = ColumnPresence::from_names(["bid"]).with_symbol("SPY");
        assert_eq!(constant.symbol(), Some("SPY"));
        assert_eq!(constant.symbols(), None);

        let per_row = ColumnPresence::from_names(["bid"]).with_symbols(["AAPL", "MSFT"]);
        assert_eq!(per_row.symbol(), None);
        assert_eq!(
            per_row.symbols().map(<[Box<str>]>::to_vec),
            Some(vec!["AAPL".into(), "MSFT".into()])
        );
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

    /// The interest-rate shape maps wire `created` -> field `date`: `date` is
    /// the primary claimant of the `created` header (no sibling ms-of-day
    /// field), so it claims it directly and both fields are present.
    #[test]
    fn present_date_is_primary_claimant_when_no_time_sibling() {
        const COLS: &[(&str, &str)] = &[("created", "date"), ("rate", "rate")];
        let p = present_columns_from(&["created", "rate"], COLS, false, false);
        assert!(
            p.contains("date"),
            "date claims the `created` header directly"
        );
        assert!(p.contains("rate"));
    }

    /// Present columns are emitted in schema order regardless of which pass
    /// claimed them (an alias-resolved time field before a later exact field).
    #[test]
    fn present_names_follow_schema_order() {
        const COLS: &[(&str, &str)] = &[
            ("ms_of_day", "ms_of_day"),
            ("date", "date"),
            ("rate", "rate"),
        ];
        // `ms_of_day` resolves via alias (pass 2), `rate`/`date` are exact —
        // yet the order must stay schema order, not claim order.
        let p = present_columns_from(&["timestamp", "date", "rate"], COLS, false, false);
        let got: Vec<&str> = p.present_names().collect();
        assert_eq!(got, ["ms_of_day", "date", "rate"]);
    }
}
