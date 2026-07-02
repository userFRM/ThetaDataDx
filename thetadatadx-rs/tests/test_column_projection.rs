//! Per-response column-projection coverage.
//!
//! Runs under `cargo test -p thetadatadx --features "arrow,polars,__internal"`.
//!
//! The gRPC decode emits exactly the columns each response's wire carried —
//! terminal-exact, no superset. Two long-standing symptoms are one bug:
//!
//!   * gRPC trade endpoints never send `condition_flags` / `price_flags` /
//!     `volume_type` / `records_back`, so those columns showed as always-0.
//!   * equity / index endpoints never send `expiration` / `strike` /
//!     `right`, so those columns showed as always-null.
//!
//! `WireColumns::present_columns(headers)` computes the present column set
//! from the response header list (the wire truth pinned in each
//! `*.meta.toml`), and `to_arrow_projected` / `to_polars_projected` emit
//! only those columns. These tests decode the checked-in captures through
//! the production decode path and assert the projected frame's column set.

#![cfg(all(feature = "arrow", feature = "polars", feature = "__internal"))]

use thetadatadx::frames::{TicksArrowExt, TicksPolarsExt};
use thetadatadx::wire as proto;
use thetadatadx::{
    decode, ColumnPresence, EodTick, QuoteTick, TradeQuoteTick, TradeTick, WireColumns,
};

#[path = "common/capture_loader.rs"]
mod capture_loader;
use capture_loader::load_data_table;

/// Present-column set for a decoded table, computed from its headers via
/// the tick type's generated `WireColumns` impl (the same alias-aware
/// resolution the parser uses).
fn presence_of<T: WireColumns>(table: &proto::DataTable) -> ColumnPresence {
    let headers: Vec<&str> = table.headers.iter().map(String::as_str).collect();
    T::present_columns(&headers)
}

fn arrow_columns(batch: &arrow_array::RecordBatch) -> Vec<String> {
    batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect()
}

fn polars_columns(df: &polars::prelude::DataFrame) -> Vec<String> {
    df.get_column_names()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// ── Symptom 1: gRPC trade responses omit the four flag columns ────────────

/// A stock `trade_quote` response carries neither the four trade-flag
/// columns nor the contract-identity trio — the projected frame omits both,
/// rather than emitting them always-zero / always-null.
#[test]
fn stock_trade_quote_projects_out_flags_and_contract_id() {
    let table = load_data_table("stock_history_trade_quote");
    let present = presence_of::<TradeQuoteTick>(&table);
    let ticks: Vec<TradeQuoteTick> =
        decode::parse_trade_quote_ticks(&table).expect("parse_trade_quote_ticks");

    let batch = ticks.as_slice().to_arrow_projected(&present).unwrap();
    let cols = arrow_columns(&batch);

    for flag in [
        "condition_flags",
        "price_flags",
        "volume_type",
        "records_back",
    ] {
        assert!(
            !cols.contains(&flag.to_string()),
            "stock trade_quote must not carry gRPC-absent flag column {flag}; got {cols:?}"
        );
    }
    for cid in ["expiration", "strike", "right"] {
        assert!(
            !cols.contains(&cid.to_string()),
            "stock trade_quote must not carry contract-id column {cid}; got {cols:?}"
        );
    }
    // The columns the wire DID send are present.
    for kept in ["ms_of_day", "quote_ms_of_day", "bid", "ask", "price"] {
        assert!(
            cols.contains(&kept.to_string()),
            "missing {kept} in {cols:?}"
        );
    }
    // Polars frame agrees.
    let df = ticks.as_slice().to_polars_projected(&present).unwrap();
    assert_eq!(
        polars_columns(&df),
        cols,
        "arrow/polars column sets diverge"
    );
    assert_eq!(df.height(), ticks.len());
}

/// An OPTION `trade` response also omits the four flag columns (the gRPC
/// wire never sends them), but KEEPS the contract-identity trio the server
/// injected. The projected `TradeTick` frame reflects exactly that.
#[test]
fn option_trade_keeps_contract_id_drops_flags() {
    let table = load_data_table("option_history_trade");
    let present = presence_of::<TradeTick>(&table);
    let ticks: Vec<TradeTick> = decode::parse_trade_ticks(&table).expect("parse_trade_ticks");

    let batch = ticks.as_slice().to_arrow_projected(&present).unwrap();
    let cols = arrow_columns(&batch);

    for cid in ["expiration", "strike", "right"] {
        assert!(
            cols.contains(&cid.to_string()),
            "option trade must keep contract-id column {cid}; got {cols:?}"
        );
    }
    for flag in [
        "condition_flags",
        "price_flags",
        "volume_type",
        "records_back",
    ] {
        assert!(
            !cols.contains(&flag.to_string()),
            "no gRPC trade response carries flag column {flag}; got {cols:?}"
        );
    }
}

/// The option `trade_quote` wildcard response keeps the contract-id trio.
#[test]
fn option_trade_quote_keeps_contract_id() {
    let table = load_data_table("option_history_trade_quote");
    let present = presence_of::<TradeQuoteTick>(&table);
    let ticks: Vec<TradeQuoteTick> =
        decode::parse_trade_quote_ticks(&table).expect("parse_trade_quote_ticks");

    let cols = arrow_columns(&ticks.as_slice().to_arrow_projected(&present).unwrap());
    for cid in ["expiration", "strike", "right"] {
        assert!(cols.contains(&cid.to_string()), "missing {cid} in {cols:?}");
    }
}

// ── The response's constant `symbol` (root) rides as the leading column ───
//
// Option + index endpoints send a `symbol` header constant across the
// response; the flat POD tick structs can't hold a per-row `String`, so the
// decode carries it once on the `ColumnPresence` and the projected builders
// broadcast it as the first column. Stock endpoints send no `symbol` header.
// These tests drive the same presence the decode seam builds — the wire
// header set plus the response's root read through the public
// `extract_text_column` (the alias-aware `root` <- `symbol` lookup).

/// The presence the decode seam builds for `table`: the wire's column set plus
/// the response's constant `symbol` (root) when the wire carried one.
fn presence_with_symbol<T: WireColumns>(table: &proto::DataTable) -> ColumnPresence {
    let present = presence_of::<T>(table);
    match decode::extract_text_column(table, "root")
        .into_iter()
        .flatten()
        .next()
    {
        Some(symbol) => present.with_symbol(symbol),
        None => present,
    }
}

/// The values of the `symbol` column in a projected batch, or `None` when the
/// batch carries no `symbol` column.
fn symbol_column(batch: &arrow_array::RecordBatch) -> Option<Vec<String>> {
    let idx = batch.schema().index_of("symbol").ok()?;
    use arrow_array::Array as _;
    let arr = batch
        .column(idx)
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .expect("symbol column is Utf8");
    Some((0..arr.len()).map(|i| arr.value(i).to_string()).collect())
}

/// An `option_history_trade` response carries a constant `symbol` (root); the
/// projected frame prepends a `symbol` Utf8 column valued on every row, first
/// in schema order to match the wire header layout.
#[test]
fn option_trade_broadcasts_symbol_as_leading_column() {
    let table = load_data_table("option_history_trade");
    let present = presence_with_symbol::<TradeTick>(&table);
    let ticks: Vec<TradeTick> = decode::parse_trade_ticks(&table).expect("parse_trade_ticks");

    let batch = ticks.as_slice().to_arrow_projected(&present).unwrap();
    let cols = arrow_columns(&batch);
    assert_eq!(
        cols.first().map(String::as_str),
        Some("symbol"),
        "symbol must be the leading column; got {cols:?}"
    );
    let values = symbol_column(&batch).expect("symbol column present");
    assert_eq!(values.len(), ticks.len(), "symbol broadcast to every row");
    assert!(
        values.iter().all(|v| v == "SPY"),
        "symbol value must be the queried root on every row; got {values:?}"
    );

    // Polars agrees on the column set (symbol first).
    let df = ticks.as_slice().to_polars_projected(&present).unwrap();
    assert_eq!(
        polars_columns(&df),
        cols,
        "arrow/polars column sets diverge"
    );
    assert_eq!(df.height(), ticks.len());
}

/// A wildcard option `trade_quote` response also broadcasts `symbol` first,
/// alongside the contract-identity trio that varies per contract.
#[test]
fn option_trade_quote_broadcasts_symbol() {
    let table = load_data_table("option_history_trade_quote");
    let present = presence_with_symbol::<TradeQuoteTick>(&table);
    let ticks: Vec<TradeQuoteTick> =
        decode::parse_trade_quote_ticks(&table).expect("parse_trade_quote_ticks");

    let batch = ticks.as_slice().to_arrow_projected(&present).unwrap();
    let cols = arrow_columns(&batch);
    assert_eq!(
        cols.first().map(String::as_str),
        Some("symbol"),
        "got {cols:?}"
    );
    let values = symbol_column(&batch).expect("symbol column present");
    assert!(
        !values.is_empty() && values.iter().all(|v| v == &values[0]),
        "symbol constant across a wildcard response; got {values:?}"
    );
    // The contract-id trio still rides after the symbol.
    for cid in ["expiration", "strike", "right"] {
        assert!(cols.contains(&cid.to_string()), "missing {cid} in {cols:?}");
    }
}

/// `OptionContract` (the `option_list_contracts` tick) already owns a per-row
/// `symbol` column. Even when the decode attaches a broadcast root, its
/// projected frame must carry EXACTLY ONE `symbol` column — its per-row one —
/// not a duplicate broadcast field.
#[test]
fn option_contract_projects_exactly_one_symbol_column() {
    use thetadatadx::OptionContract;
    // The shape the decode seam builds: the tick's own schema columns present,
    // plus a broadcast root attached (as it would be off the `symbol` header).
    let present =
        ColumnPresence::from_names(["symbol", "expiration", "strike", "right"]).with_symbol("SPY");
    let empty: Vec<OptionContract> = Vec::new();

    let cols = arrow_columns(&empty.as_slice().to_arrow_projected(&present).unwrap());
    assert_eq!(
        cols.iter().filter(|c| c.as_str() == "symbol").count(),
        1,
        "OptionContract must carry exactly one (per-row) symbol column; got {cols:?}"
    );
    // Polars agrees.
    let dcols = polars_columns(&empty.as_slice().to_polars_projected(&present).unwrap());
    assert_eq!(
        dcols.iter().filter(|c| c.as_str() == "symbol").count(),
        1,
        "polars OptionContract must carry exactly one symbol column; got {dcols:?}"
    );
}

/// A stock response carries no `symbol` header — the projected frame gains no
/// `symbol` column.
#[test]
fn stock_trade_quote_has_no_symbol_column() {
    let table = load_data_table("stock_history_trade_quote");
    let present = presence_with_symbol::<TradeQuoteTick>(&table);
    let ticks: Vec<TradeQuoteTick> =
        decode::parse_trade_quote_ticks(&table).expect("parse_trade_quote_ticks");

    let batch = ticks.as_slice().to_arrow_projected(&present).unwrap();
    assert!(
        symbol_column(&batch).is_none(),
        "stock response must not carry a symbol column; got {:?}",
        arrow_columns(&batch)
    );
    assert!(present.symbol().is_none(), "no symbol on a stock presence");
}

// ── Symptom 2: equity responses omit the contract-identity trio ───────────

/// A stock EOD response carries no contract-identity trio — the projected
/// `EodTick` frame omits `expiration` / `strike` / `right`.
#[test]
fn stock_eod_projects_out_contract_id() {
    let table = load_data_table("stock_history_eod");
    let present = presence_of::<EodTick>(&table);
    let ticks: Vec<EodTick> = decode::parse_eod_ticks(&table).expect("parse_eod_ticks");

    let batch = ticks.as_slice().to_arrow_projected(&present).unwrap();
    let cols = arrow_columns(&batch);
    for cid in ["expiration", "strike", "right"] {
        assert!(
            !cols.contains(&cid.to_string()),
            "stock EOD must not carry contract-id column {cid}; got {cols:?}"
        );
    }
    // The EOD wire sends one `created` Timestamp that the schema splits into
    // `created_ms_of_day` (time) AND `date` (YYYYMMDD). Both must survive the
    // projection: the ms-of-day field claims the header and `date` rides the
    // same column, so the trading day is not lost. Without `date` the four
    // rows are indistinguishable (their `created_ms_of_day` is a ~17:1x ET
    // near-constant).
    assert!(
        cols.contains(&"created_ms_of_day".to_string()),
        "got {cols:?}"
    );
    assert!(
        cols.contains(&"date".to_string()),
        "stock EOD must keep the trading `date` split from the `created` Timestamp; got {cols:?}"
    );
    // The `date` column carries the distinct trading days, not a constant.
    use arrow_array::Array as _;
    let date_idx = batch.schema().index_of("date").unwrap();
    let dates = batch
        .column(date_idx)
        .as_any()
        .downcast_ref::<arrow_array::Int32Array>()
        .expect("date column is Int32");
    let values: Vec<i32> = (0..dates.len()).map(|i| dates.value(i)).collect();
    assert_eq!(
        values,
        vec![20240102, 20240103, 20240104, 20240105],
        "date column must carry the distinct YYYYMMDD trading days"
    );
    assert!(
        values.windows(2).all(|w| w[0] != w[1]),
        "dates must differ across rows; got {values:?}"
    );
    // But the EOD data columns the wire sent are all present.
    for kept in [
        "open", "high", "low", "close", "volume", "count", "bid", "ask",
    ] {
        assert!(
            cols.contains(&kept.to_string()),
            "missing {kept} in {cols:?}"
        );
    }
}

// ── Per-response, not per-type: the same tick type projects differently ───
//
// `stock_snapshot_trade` sends a 6-column subset while `stock_history_trade`
// sends 8 columns, and `index_snapshot_market_value` drops `market_bid` /
// `market_ask` that stock/option market_value carry. No capture fixtures
// exist for these offline, so drive the projection from the documented wire
// header lists directly: `present_columns(headers)` is a pure function of
// the header set, and `to_arrow_projected` on an (empty) slice emits exactly
// that projected schema — keyed on the wire headers, not a static per-type
// column list.

fn present_columns_for<T: WireColumns>(headers: &[&str]) -> ColumnPresence {
    T::present_columns(headers)
}

/// Column set the projected frame would carry for `headers`, taken from the
/// projected schema on an empty slice (row values are irrelevant to the
/// column set).
fn projected_columns<T>(headers: &[&str]) -> Vec<String>
where
    [T]: TicksArrowExt,
    T: WireColumns,
{
    let present = present_columns_for::<T>(headers);
    let empty: Vec<T> = Vec::new();
    arrow_columns(&empty.as_slice().to_arrow_projected(&present).unwrap())
}

/// `stock_snapshot_trade` sends only `timestamp,symbol,sequence,size,
/// condition,price` — the projected `TradeTick` frame carries none of the
/// `ext_condition1..4`, `exchange`, or flag columns a full trade row has.
#[test]
fn stock_snapshot_trade_projects_six_column_subset() {
    let cols = projected_columns::<TradeTick>(&[
        "timestamp",
        "symbol",
        "sequence",
        "size",
        "condition",
        "price",
    ]);

    // Present: the columns the wire sent (timestamp aliases to ms_of_day).
    for kept in ["ms_of_day", "sequence", "size", "condition", "price"] {
        assert!(
            cols.contains(&kept.to_string()),
            "missing {kept} in {cols:?}"
        );
    }
    // Absent: everything the snapshot omits.
    for dropped in [
        "ext_condition1",
        "ext_condition2",
        "ext_condition3",
        "ext_condition4",
        "exchange",
        "condition_flags",
        "price_flags",
        "volume_type",
        "records_back",
        "expiration",
        "strike",
        "right",
    ] {
        assert!(
            !cols.contains(&dropped.to_string()),
            "snapshot trade must not carry {dropped}; got {cols:?}"
        );
    }
}

/// `index_snapshot_market_value` omits `market_bid` / `market_ask` (the
/// index wire has no book), while stock/option market_value carry them. The
/// projection drops them for the index subset but keeps them for the full
/// shape — same tick type, different per-response column set.
#[test]
fn index_snapshot_market_value_drops_bid_ask() {
    let full = projected_columns::<thetadatadx::MarketValueTick>(&[
        "timestamp",
        "symbol",
        "market_bid",
        "market_ask",
        "market_price",
    ]);
    assert!(full.contains(&"market_bid".to_string()), "got {full:?}");
    assert!(full.contains(&"market_ask".to_string()), "got {full:?}");

    let index =
        projected_columns::<thetadatadx::MarketValueTick>(&["timestamp", "symbol", "market_price"]);
    assert!(index.contains(&"market_price".to_string()), "got {index:?}");
    for dropped in ["market_bid", "market_ask"] {
        assert!(
            !index.contains(&dropped.to_string()),
            "index market_value must not carry {dropped}; got {index:?}"
        );
    }
}

/// `QuoteTick.midpoint` is computed at decode from `bid` + `ask` and is never a
/// wire header, so the projected frame must key its presence on both inputs:
/// present when bid + ask are, absent otherwise. Regression guard — a decode
/// path that dropped midpoint left `ticks[0].midpoint` populated while the
/// frame omitted the column.
#[test]
fn quote_projects_midpoint_when_bid_and_ask_present() {
    let with = projected_columns::<QuoteTick>(&[
        "ms_of_day",
        "bid_size",
        "bid",
        "ask_size",
        "ask",
        "date",
    ]);
    assert!(
        with.contains(&"midpoint".to_string()),
        "midpoint must ride whenever bid + ask do; got {with:?}"
    );

    // A subset without bid/ask carries no midpoint.
    let without = projected_columns::<QuoteTick>(&["ms_of_day", "date"]);
    assert!(
        !without.contains(&"midpoint".to_string()),
        "midpoint must be absent when its inputs are; got {without:?}"
    );
}

// ── The full (hand-built) path is unchanged ───────────────────────────────

/// `to_arrow` (no presence) still emits the complete schema — the
/// hand-built-slice default a caller who never touched the wire relies on.
#[test]
fn full_to_arrow_still_emits_every_column() {
    let ticks = vec![TradeTick {
        ms_of_day: 1,
        sequence: 2,
        ext_condition1: 0,
        ext_condition2: 0,
        ext_condition3: 0,
        ext_condition4: 0,
        condition: 0,
        size: 5,
        exchange: 7,
        price: 1.0,
        condition_flags: 0,
        price_flags: 0,
        volume_type: 0,
        records_back: 0,
        date: 20240101,
        expiration: 0,
        strike: 0.0,
        right: '\0',
    }];
    let cols = arrow_columns(&ticks.as_slice().to_arrow().unwrap());
    // Full schema carries the flags and the contract-id trio unconditionally.
    for every in [
        "condition_flags",
        "price_flags",
        "volume_type",
        "records_back",
        "expiration",
        "strike",
        "right",
    ] {
        assert!(
            cols.contains(&every.to_string()),
            "missing {every} in {cols:?}"
        );
    }
}

/// A response whose wire headers resolve to zero schema columns has an empty
/// `ColumnPresence`. The projected builders must still succeed — a 0-column
/// frame that keeps its row count — not error. Arrow's plain
/// `RecordBatch::try_new` cannot infer a row count from zero columns, so the
/// builder pins it via `try_new_with_options`. Polars carries the height in
/// `DataFrame::new(height, cols)` already.
#[test]
fn empty_presence_projects_to_zero_columns_keeping_rows() {
    let ticks = vec![
        TradeTick {
            ms_of_day: 1,
            sequence: 2,
            ext_condition1: 0,
            ext_condition2: 0,
            ext_condition3: 0,
            ext_condition4: 0,
            condition: 0,
            size: 5,
            exchange: 7,
            price: 1.0,
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            date: 20240101,
            expiration: 0,
            strike: 0.0,
            right: '\0',
        };
        3
    ];
    let empty = ColumnPresence::default();
    assert!(empty.is_empty());

    let batch = ticks
        .as_slice()
        .to_arrow_projected(&empty)
        .expect("empty presence must not error");
    assert_eq!(batch.num_columns(), 0, "empty presence -> zero columns");
    assert_eq!(batch.num_rows(), 3, "zero-column batch keeps its row count");

    let df = ticks
        .as_slice()
        .to_polars_projected(&empty)
        .expect("empty presence must not error");
    assert_eq!(df.width(), 0, "empty presence -> zero columns");
    assert_eq!(df.height(), 3, "zero-column frame keeps its height");
}
