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
            root: "AAPL".into(),
            expiration: 20240119,
            strike: 195.0,
            right: 67,
        },
        tick::OptionContract {
            root: "AAPL".into(),
            expiration: 20240119,
            strike: 195.0,
            right: 80,
        },
    ];
    let batch = ticks.as_slice().to_arrow().unwrap();
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(dtype_of(&batch, "right"), DataType::Utf8);
    assert_eq!(dtype_of(&batch, "root"), DataType::Utf8);
}

#[test]
fn empty_slice_produces_zero_row_batch() {
    let ticks: Vec<tick::OhlcTick> = vec![];
    let batch = ticks.as_slice().to_arrow().unwrap();
    assert_eq!(batch.num_rows(), 0);
    assert!(batch.num_columns() > 0);
}

/// `GreeksFirstOrderTick` to_arrow: pin column count, exact column
/// order, dtype per column, and one row's value for every column. The
/// schema is generator-emitted so a row reorder or dtype regression on
/// `crates/thetadatadx/build_support/ticks/rust_frames.rs` surfaces
/// here. Mirrors the per-order parser test in `decode.rs`.
#[test]
fn greeks_first_order_tick_to_arrow() {
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
    let batch = ticks.as_slice().to_arrow().unwrap();
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.num_columns(), 17);
    assert_eq!(
        columns(&batch),
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
        assert_eq!(dtype_of(&batch, f64_col), DataType::Float64, "{f64_col}");
    }
    for i32_col in ["ms_of_day", "underlying_ms_of_day", "date", "expiration"] {
        assert_eq!(dtype_of(&batch, i32_col), DataType::Int32, "{i32_col}");
    }
    assert_eq!(dtype_of(&batch, "right"), DataType::Utf8);

    // Row value spot-check: every column reads back through the
    // typed Arrow array. f64 columns use bit-exact == (the values
    // round-trip without arithmetic).
    use arrow_array::{Float64Array, Int32Array, StringArray};
    let f64_at = |name: &str| {
        let col = batch.column_by_name(name).unwrap();
        col.as_any()
            .downcast_ref::<Float64Array>()
            .unwrap()
            .value(0)
    };
    let i32_at = |name: &str| {
        let col = batch.column_by_name(name).unwrap();
        col.as_any().downcast_ref::<Int32Array>().unwrap().value(0)
    };
    assert_eq!(i32_at("ms_of_day"), 34_200_000);
    assert_eq!(f64_at("bid"), 1.5022);
    assert_eq!(f64_at("ask"), 1.5041);
    assert_eq!(f64_at("delta"), 0.5023);
    assert_eq!(f64_at("epsilon"), -0.1976);
    assert_eq!(f64_at("implied_volatility"), 0.2142);
    assert_eq!(i32_at("underlying_ms_of_day"), 34_200_001);
    assert_eq!(f64_at("underlying_price"), 58.0025);
    assert_eq!(i32_at("date"), 20_240_614);
    assert_eq!(i32_at("expiration"), 20_240_621);
    assert_eq!(f64_at("strike"), 500.0);
    let right_arr = batch
        .column_by_name("right")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(right_arr.value(0), "C");
}

/// `GreeksSecondOrderTick` to_arrow: same shape guarantees as the
/// first-order test above. Different column subset (gamma / vanna /
/// charm / vomma / veta).
#[test]
fn greeks_second_order_tick_to_arrow() {
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
    let batch = ticks.as_slice().to_arrow().unwrap();
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.num_columns(), 16);
    assert_eq!(
        columns(&batch),
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
    assert_eq!(dtype_of(&batch, "gamma"), DataType::Float64);
    assert_eq!(dtype_of(&batch, "vanna"), DataType::Float64);
    assert_eq!(dtype_of(&batch, "right"), DataType::Utf8);

    use arrow_array::{Float64Array, Int32Array, StringArray};
    let f64_at = |name: &str| {
        batch
            .column_by_name(name)
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap()
            .value(0)
    };
    let i32_at = |name: &str| {
        batch
            .column_by_name(name)
            .unwrap()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .value(0)
    };
    assert_eq!(i32_at("ms_of_day"), 34_200_000);
    assert_eq!(f64_at("gamma"), 0.012);
    assert_eq!(f64_at("vanna"), 0.0045);
    assert_eq!(f64_at("charm"), -0.0012);
    assert_eq!(f64_at("vomma"), 0.09);
    assert_eq!(f64_at("veta"), -0.0003);
    assert_eq!(f64_at("implied_volatility"), 0.2142);
    assert_eq!(i32_at("expiration"), 20_240_621);
    let right_arr = batch
        .column_by_name("right")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(right_arr.value(0), "P");
}

/// `GreeksThirdOrderTick` to_arrow: same shape guarantees. Third-order
/// subset (speed / zomma / color / ultima); `vera` is intentionally
/// not in the third-order schema.
#[test]
fn greeks_third_order_tick_to_arrow() {
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
    let batch = ticks.as_slice().to_arrow().unwrap();
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(batch.num_columns(), 15);
    assert_eq!(
        columns(&batch),
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
    assert!(!columns(&batch).contains(&"vera".to_string()));
    assert_eq!(dtype_of(&batch, "speed"), DataType::Float64);
    assert_eq!(dtype_of(&batch, "ultima"), DataType::Float64);
    assert_eq!(dtype_of(&batch, "right"), DataType::Utf8);

    use arrow_array::Float64Array;
    let f64_at = |name: &str| {
        batch
            .column_by_name(name)
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap()
            .value(0)
    };
    assert_eq!(f64_at("speed"), 0.0007);
    assert_eq!(f64_at("zomma"), 0.0015);
    assert_eq!(f64_at("color"), -0.0002);
    assert_eq!(f64_at("ultima"), 0.0033);
}

#[test]
fn every_tick_type_has_arrow_impl() {
    fn _assert<T: ?Sized + TicksArrowExt>() {}
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
