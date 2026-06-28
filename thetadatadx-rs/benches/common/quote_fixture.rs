//! Shared quote-tick `DataTable` fixture used by every decode bench
//! in this crate.
//!
//! Pulled out of the four per-bench duplicates that landed alongside
//! `bench_decode` and `bench_protobuf_decode`. Each bench previously
//! stamped its own copy
//! of the same 10-column quote schema; diverging deterministic value
//! offsets would skew cross-bench comparison.
//!
//! Wired in via `#[path = "common/quote_fixture.rs"] mod fixture;` so
//! the criterion harness layout under `benches/*.rs` is unchanged.

use thetadatadx::wire::{data_value, DataTable, DataValue, DataValueList, Price};

/// Build a `DataValue` carrying a `Number` payload.
#[must_use]
pub fn dv_number(n: i64) -> DataValue {
    DataValue {
        data_type: Some(data_value::DataType::Number(n)),
    }
}

/// Build a `DataValue` carrying a `Price` payload.
#[must_use]
pub fn dv_price(value: i32, ty: i32) -> DataValue {
    DataValue {
        data_type: Some(data_value::DataType::Price(Price { value, r#type: ty })),
    }
}

/// Build a 10-column quote-tick `DataTable` of `n` rows.
///
/// Schema mirrors the canonical MDDS quote shape
/// (`ms_of_day`, `bid_size`, `bid_exchange`, `bid`, `bid_condition`,
/// `ask_size`, `ask_exchange`, `ask`, `ask_condition`, `date`). Row
/// values are deterministic functions of the row index so each bench
/// can rebuild an identical payload on demand without persisting one
/// between iterations.
#[must_use]
pub fn build_quote_data_table(n: usize) -> DataTable {
    let headers = vec![
        "ms_of_day".to_string(),
        "bid_size".to_string(),
        "bid_exchange".to_string(),
        "bid".to_string(),
        "bid_condition".to_string(),
        "ask_size".to_string(),
        "ask_exchange".to_string(),
        "ask".to_string(),
        "ask_condition".to_string(),
        "date".to_string(),
    ];
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        let bid = 15_020 + (i % 100) as i32;
        let ask = 15_030 + (i % 100) as i32;
        rows.push(DataValueList {
            values: vec![
                dv_number(34_200_000 + i as i64 * 50),
                dv_number(10 + (i % 100) as i64),
                dv_number(4),
                dv_price(bid, 8),
                dv_number(1),
                dv_number(5 + (i % 80) as i64),
                dv_number(4),
                dv_price(ask, 8),
                dv_number(1),
                dv_number(20_240_315),
            ],
        });
    }
    DataTable {
        headers,
        data_table: rows,
    }
}
