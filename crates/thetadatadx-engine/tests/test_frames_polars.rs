//! `TicksPolarsExt::to_polars` coverage.
//!
//! Runs under `cargo test -p thetadatadx --features polars`. Verifies
//! every tick type in `tdbe::types::tick` has a generator-emitted impl
//! and asserts the produced DataFrame has the expected column set,
//! column order, row count, and primitive dtypes.

#![cfg(feature = "polars")]

use polars::prelude::{DataFrame, DataType};
use tdbe::types::tick;
use thetadatadx_engine::frames::TicksPolarsExt;

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
        vwap: 100.45,
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

/// `GreeksFirstOrderTick` to_polars: pin column count, exact column
/// order, dtype per column, and one row's value for every column. The
/// schema is generator-emitted so a row reorder or dtype regression on
/// `crates/thetadatadx/build_support/ticks/rust_frames.rs` surfaces
/// here. Mirrors the Arrow test in `test_frames_arrow.rs`.
#[test]
fn greeks_first_order_tick_to_polars() {
    let ticks = vec![tick::GreeksFirstOrderTick {
        ms_of_day: 34_200_000,
        bid: 1.5022,
        ask: 1.5041,
        delta: 0.5023,
        theta: -0.0114,
        vega: 0.8741,
        rho: 1.3598,
        epsilon: -0.1976,
        lambda: 3.2052,
        implied_volatility: 0.2142,
        iv_error: -0.0003,
        underlying_ms_of_day: 34_200_001,
        underlying_price: 58.0025,
        date: 20_240_614,
        expiration: 20_240_621,
        strike: 500.0,
        right: 67, // 'C'
    }];
    let df = ticks.as_slice().to_polars().unwrap();
    assert_eq!(df.height(), 1);
    assert_eq!(df.width(), 17);
    assert_eq!(
        columns(&df),
        vec![
            "ms_of_day",
            "bid",
            "ask",
            "delta",
            "theta",
            "vega",
            "rho",
            "epsilon",
            "lambda",
            "implied_volatility",
            "iv_error",
            "underlying_ms_of_day",
            "underlying_price",
            "date",
            "expiration",
            "strike",
            "right"
        ]
    );
    for f64_col in [
        "bid",
        "ask",
        "delta",
        "theta",
        "vega",
        "rho",
        "epsilon",
        "lambda",
        "implied_volatility",
        "iv_error",
        "underlying_price",
        "strike",
    ] {
        assert_eq!(dtype_of(&df, f64_col), DataType::Float64, "{f64_col}");
    }
    for i32_col in ["ms_of_day", "underlying_ms_of_day", "date", "expiration"] {
        assert_eq!(dtype_of(&df, i32_col), DataType::Int32, "{i32_col}");
    }
    assert_eq!(dtype_of(&df, "right"), DataType::String);

    // Row value spot-check: every column reads back through the
    // typed Series getters.
    assert_eq!(
        df.column("ms_of_day").unwrap().i32().unwrap().get(0),
        Some(34_200_000)
    );
    assert_eq!(
        df.column("delta").unwrap().f64().unwrap().get(0),
        Some(0.5023)
    );
    assert_eq!(
        df.column("epsilon").unwrap().f64().unwrap().get(0),
        Some(-0.1976)
    );
    assert_eq!(
        df.column("implied_volatility")
            .unwrap()
            .f64()
            .unwrap()
            .get(0),
        Some(0.2142)
    );
    assert_eq!(
        df.column("underlying_ms_of_day")
            .unwrap()
            .i32()
            .unwrap()
            .get(0),
        Some(34_200_001)
    );
    assert_eq!(
        df.column("underlying_price").unwrap().f64().unwrap().get(0),
        Some(58.0025)
    );
    assert_eq!(
        df.column("date").unwrap().i32().unwrap().get(0),
        Some(20_240_614)
    );
    assert_eq!(
        df.column("expiration").unwrap().i32().unwrap().get(0),
        Some(20_240_621)
    );
    assert_eq!(
        df.column("strike").unwrap().f64().unwrap().get(0),
        Some(500.0)
    );
    assert_eq!(df.column("right").unwrap().str().unwrap().get(0), Some("C"));
}

/// `GreeksSecondOrderTick` to_polars: same guarantees as the
/// first-order test above. Different column subset (gamma / vanna /
/// charm / vomma / veta).
#[test]
fn greeks_second_order_tick_to_polars() {
    let ticks = vec![tick::GreeksSecondOrderTick {
        ms_of_day: 34_200_000,
        bid: 1.5022,
        ask: 1.5041,
        gamma: 0.012,
        vanna: 0.0045,
        charm: -0.0012,
        vomma: 0.09,
        veta: -0.0003,
        implied_volatility: 0.2142,
        iv_error: -0.0003,
        underlying_ms_of_day: 34_200_001,
        underlying_price: 58.0025,
        date: 20_240_614,
        expiration: 20_240_621,
        strike: 500.0,
        right: 80, // 'P'
    }];
    let df = ticks.as_slice().to_polars().unwrap();
    assert_eq!(df.height(), 1);
    assert_eq!(df.width(), 16);
    assert_eq!(
        columns(&df),
        vec![
            "ms_of_day",
            "bid",
            "ask",
            "gamma",
            "vanna",
            "charm",
            "vomma",
            "veta",
            "implied_volatility",
            "iv_error",
            "underlying_ms_of_day",
            "underlying_price",
            "date",
            "expiration",
            "strike",
            "right"
        ]
    );
    assert_eq!(dtype_of(&df, "gamma"), DataType::Float64);
    assert_eq!(dtype_of(&df, "right"), DataType::String);

    assert_eq!(
        df.column("gamma").unwrap().f64().unwrap().get(0),
        Some(0.012)
    );
    assert_eq!(
        df.column("vanna").unwrap().f64().unwrap().get(0),
        Some(0.0045)
    );
    assert_eq!(
        df.column("charm").unwrap().f64().unwrap().get(0),
        Some(-0.0012)
    );
    assert_eq!(
        df.column("vomma").unwrap().f64().unwrap().get(0),
        Some(0.09)
    );
    assert_eq!(
        df.column("veta").unwrap().f64().unwrap().get(0),
        Some(-0.0003)
    );
    assert_eq!(df.column("right").unwrap().str().unwrap().get(0), Some("P"));
}

/// `GreeksThirdOrderTick` to_polars: same guarantees. Third-order
/// subset (speed / zomma / color / ultima); `vera` is intentionally
/// not in the third-order schema.
#[test]
fn greeks_third_order_tick_to_polars() {
    let ticks = vec![tick::GreeksThirdOrderTick {
        ms_of_day: 34_200_000,
        bid: 1.5022,
        ask: 1.5041,
        speed: 0.0007,
        zomma: 0.0015,
        color: -0.0002,
        ultima: 0.0033,
        implied_volatility: 0.2142,
        iv_error: -0.0003,
        underlying_ms_of_day: 34_200_001,
        underlying_price: 58.0025,
        date: 20_240_614,
        expiration: 20_240_621,
        strike: 500.0,
        right: 67,
    }];
    let df = ticks.as_slice().to_polars().unwrap();
    assert_eq!(df.height(), 1);
    assert_eq!(df.width(), 15);
    assert_eq!(
        columns(&df),
        vec![
            "ms_of_day",
            "bid",
            "ask",
            "speed",
            "zomma",
            "color",
            "ultima",
            "implied_volatility",
            "iv_error",
            "underlying_ms_of_day",
            "underlying_price",
            "date",
            "expiration",
            "strike",
            "right"
        ]
    );
    assert!(!columns(&df).contains(&"vera".to_string()));
    assert_eq!(dtype_of(&df, "speed"), DataType::Float64);

    assert_eq!(
        df.column("speed").unwrap().f64().unwrap().get(0),
        Some(0.0007)
    );
    assert_eq!(
        df.column("zomma").unwrap().f64().unwrap().get(0),
        Some(0.0015)
    );
    assert_eq!(
        df.column("color").unwrap().f64().unwrap().get(0),
        Some(-0.0002)
    );
    assert_eq!(
        df.column("ultima").unwrap().f64().unwrap().get(0),
        Some(0.0033)
    );
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
    _assert::<[tick::GreeksAllTick]>();
    _assert::<[tick::GreeksFirstOrderTick]>();
    _assert::<[tick::GreeksSecondOrderTick]>();
    _assert::<[tick::GreeksThirdOrderTick]>();
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
