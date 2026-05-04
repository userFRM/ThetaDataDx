//! `TicksArrowExt::to_arrow` coverage.
//!
//! Runs under `cargo test -p thetadatadx --features arrow`. Mirrors the
//! polars test — asserts column set, order, row count, and dtypes match
//! the schema emitted by the Python slice_arrow path.

#![cfg(feature = "arrow")]

use arrow_array::RecordBatch;
use arrow_schema::DataType;
use tdbe::types::tick;
use thetadatadx::frames::TicksArrowExt;

fn columns(batch: &RecordBatch) -> Vec<String> {
    batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect()
}

fn dtype_of(batch: &RecordBatch, name: &str) -> DataType {
    batch
        .schema()
        .field_with_name(name)
        .unwrap()
        .data_type()
        .clone()
}

#[test]
fn calendar_day_to_arrow() {
    let ticks = vec![tick::CalendarDay {
        date: 20240102,
        is_open: 1,
        open_time: 34200000,
        close_time: 57600000,
        status: 0,
    }];
    let batch = ticks.as_slice().to_arrow().unwrap();
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(
        columns(&batch),
        vec!["date", "is_open", "open_time", "close_time", "status"]
    );
    assert_eq!(dtype_of(&batch, "date"), DataType::Int32);
}

#[test]
fn ohlc_tick_to_arrow_schema_matches_python() {
    let ticks = vec![tick::OhlcTick {
        ms_of_day: 34200000,
        open: 100.5,
        high: 101.0,
        low: 99.5,
        close: 100.75,
        volume: 123456,
        count: 100,
        date: 20240102,
        expiration: 20240119,
        strike: 500.0,
        right: 67,
    }];
    let batch = ticks.as_slice().to_arrow().unwrap();
    assert_eq!(batch.num_rows(), 1);
    // i64-widened volume / count match the Python slice_arrow dtype.
    assert_eq!(dtype_of(&batch, "volume"), DataType::Int64);
    assert_eq!(dtype_of(&batch, "count"), DataType::Int64);
    assert_eq!(dtype_of(&batch, "strike"), DataType::Float64);
    assert_eq!(dtype_of(&batch, "right"), DataType::Utf8);
}

#[test]
fn quote_tick_to_arrow_emits_midpoint() {
    let ticks = vec![tick::QuoteTick {
        ms_of_day: 34200000,
        bid_size: 10,
        bid_exchange: 1,
        bid: 99.99,
        bid_condition: 0,
        ask_size: 20,
        ask_exchange: 2,
        ask: 100.01,
        ask_condition: 0,
        date: 20240102,
        midpoint: 100.0,
        expiration: 0,
        strike: 0.0,
        right: 0,
    }];
    let batch = ticks.as_slice().to_arrow().unwrap();
    assert_eq!(batch.num_rows(), 1);
    let cols = columns(&batch);
    assert!(cols.contains(&"midpoint".to_string()));
    assert_eq!(dtype_of(&batch, "midpoint"), DataType::Float64);
}

#[test]
fn option_contract_right_stringifies() {
    let ticks = vec![
        tick::OptionContract {
            symbol: "AAPL".into(),
            expiration: 20240119,
            strike: 195.0,
            right: 67,
        },
        tick::OptionContract {
            symbol: "AAPL".into(),
            expiration: 20240119,
            strike: 195.0,
            right: 80,
        },
    ];
    let batch = ticks.as_slice().to_arrow().unwrap();
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(dtype_of(&batch, "right"), DataType::Utf8);
    assert_eq!(dtype_of(&batch, "symbol"), DataType::Utf8);
}

#[test]
fn empty_slice_produces_zero_row_batch() {
    let ticks: Vec<tick::OhlcTick> = vec![];
    let batch = ticks.as_slice().to_arrow().unwrap();
    assert_eq!(batch.num_rows(), 0);
    assert!(batch.num_columns() > 0);
}

#[test]
fn every_tick_type_has_arrow_impl() {
    fn _assert<T: ?Sized + TicksArrowExt>() {}
    _assert::<[tick::CalendarDay]>();
    _assert::<[tick::EodTick]>();
    _assert::<[tick::GreeksTick]>();
    _assert::<[tick::InterestRateTick]>();
    _assert::<[tick::IvTick]>();
    _assert::<[tick::MarketValueTick]>();
    _assert::<[tick::OhlcTick]>();
    _assert::<[tick::OpenInterestTick]>();
    _assert::<[tick::OptionContract]>();
    _assert::<[tick::PriceTick]>();
    _assert::<[tick::QuoteTick]>();
    _assert::<[tick::TradeQuoteTick]>();
    _assert::<[tick::TradeTick]>();
}
