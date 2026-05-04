//! `TicksPolarsExt::to_polars` coverage.
//!
//! Runs under `cargo test -p thetadatadx --features polars`. Verifies
//! every tick type in `tdbe::types::tick` has a generator-emitted impl
//! and asserts the produced DataFrame has the expected column set,
//! column order, row count, and primitive dtypes.

#![cfg(feature = "polars")]

use polars::prelude::{DataFrame, DataType};
use tdbe::types::tick;
use thetadatadx::frames::TicksPolarsExt;

fn columns(df: &DataFrame) -> Vec<String> {
    df.get_column_names()
        .iter()
        .map(|c| c.to_string())
        .collect()
}

fn dtype_of(df: &DataFrame, name: &str) -> DataType {
    df.column(name).unwrap().dtype().clone()
}

#[test]
fn calendar_day_to_polars() {
    let ticks = vec![tick::CalendarDay {
        date: 20240102,
        is_open: 1,
        open_time: 34200000,
        close_time: 57600000,
        status: 0,
    }];
    let df = ticks.as_slice().to_polars().unwrap();
    assert_eq!(df.height(), 1);
    assert_eq!(
        columns(&df),
        vec!["date", "is_open", "open_time", "close_time", "status"]
    );
    assert_eq!(dtype_of(&df, "date"), DataType::Int32);
    assert_eq!(dtype_of(&df, "open_time"), DataType::Int32);
}

#[test]
fn ohlc_tick_to_polars_emits_contract_tail() {
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
        right: 67, // 'C'
    }];
    let df = ticks.as_slice().to_polars().unwrap();
    assert_eq!(df.height(), 1);
    let cols = columns(&df);
    assert!(cols.contains(&"open".to_string()));
    assert!(cols.contains(&"volume".to_string()));
    assert!(cols.contains(&"expiration".to_string()));
    assert!(cols.contains(&"strike".to_string()));
    assert!(cols.contains(&"right".to_string()));
    // Schema dtype spot-check: volume / count are widened to i64 even
    // though they stay i32 on the OhlcTick struct, matching the Python
    // slice_arrow schema.
    assert_eq!(dtype_of(&df, "volume"), DataType::Int64);
    assert_eq!(dtype_of(&df, "count"), DataType::Int64);
    assert_eq!(dtype_of(&df, "strike"), DataType::Float64);
    assert_eq!(dtype_of(&df, "right"), DataType::String);
}

#[test]
fn quote_tick_to_polars_emits_midpoint() {
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
    let df = ticks.as_slice().to_polars().unwrap();
    assert_eq!(df.height(), 1);
    assert!(columns(&df).contains(&"midpoint".to_string()));
    assert_eq!(dtype_of(&df, "midpoint"), DataType::Float64);
}

#[test]
fn option_contract_right_stringifies() {
    let ticks = vec![
        tick::OptionContract {
            symbol: "AAPL".into(),
            expiration: 20240119,
            strike: 195.0,
            right: 67, // 'C'
        },
        tick::OptionContract {
            symbol: "AAPL".into(),
            expiration: 20240119,
            strike: 195.0,
            right: 80, // 'P'
        },
    ];
    let df = ticks.as_slice().to_polars().unwrap();
    assert_eq!(df.height(), 2);
    assert_eq!(dtype_of(&df, "right"), DataType::String);
    assert_eq!(dtype_of(&df, "symbol"), DataType::String);
}

#[test]
fn empty_slice_produces_empty_dataframe() {
    let ticks: Vec<tick::OhlcTick> = vec![];
    let df = ticks.as_slice().to_polars().unwrap();
    assert_eq!(df.height(), 0);
    // Schema preserved on empty slices — same as the Python slice_arrow path.
    assert!(df.width() > 0);
}

// One test per tick type to prove the generator emits an impl for every
// entry in `tick_schema.toml`.
#[test]
fn every_tick_type_has_polars_impl() {
    // Compile-time proof the trait is implemented for each tick type.
    // If the generator drops an impl, this file fails to build.
    fn _assert<T: ?Sized + TicksPolarsExt>() {}
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
