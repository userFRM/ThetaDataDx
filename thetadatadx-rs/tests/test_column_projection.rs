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
use thetadatadx::{decode, ColumnPresence, EodTick, TradeQuoteTick, TradeTick, WireColumns};

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

// ── Symptom 2: equity responses omit the contract-identity trio ───────────

/// A stock EOD response carries no contract-identity trio — the projected
/// `EodTick` frame omits `expiration` / `strike` / `right`.
#[test]
fn stock_eod_projects_out_contract_id() {
    let table = load_data_table("stock_history_eod");
    let present = presence_of::<EodTick>(&table);
    let ticks: Vec<EodTick> = decode::parse_eod_ticks(&table).expect("parse_eod_ticks");

    let cols = arrow_columns(&ticks.as_slice().to_arrow_projected(&present).unwrap());
    for cid in ["expiration", "strike", "right"] {
        assert!(
            !cols.contains(&cid.to_string()),
            "stock EOD must not carry contract-id column {cid}; got {cols:?}"
        );
    }
    // The EOD wire sends `created`, not a separate `date` column. The
    // `("date","created")` alias must NOT resurrect a phantom `date` column
    // off the same `created` header — one wire column feeds one schema
    // column (the exact `created` -> `created_ms_of_day` match claims it).
    assert!(
        !cols.contains(&"date".to_string()),
        "stock EOD must not carry a phantom `date` column (alias overlap with `created`); got {cols:?}"
    );
    assert!(
        cols.contains(&"created_ms_of_day".to_string()),
        "got {cols:?}"
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
