//! Build `option_history_trade_greeks_*` fixtures used by
//! `tests/test_trade_greeks_schema.rs`.
//!
//! Each fixture is a single `proto::ResponseData` whose `compressed_data`
//! is a zstd-compressed `proto::DataTable` with the EXACT wire shape the
//! server emits for SPY 2026-01-16 540 CALL on
//! 2025-05-02 (queried via `curl
//! http://127.0.0.1:25503/v3/option/history/trade_greeks/<sub>?...`).
//!
//! Headers come from the captured CSV header row; the per-row values are
//! the first row of each capture, encoded as the same `DataValue` shapes
//! the live server uses (Number for ints/dates, Price for prices/floats,
//! Timestamp for timestamps).
//!
//! Run with: `cargo run -p thetadatadx --example build_trade_greeks_fixtures`
//! Writes to `crates/thetadatadx/tests/fixtures/captures/option_history_trade_greeks_*.pb.zst`.
//!
//! This is a one-shot tool — once the fixtures are checked in, the
//! test suite consumes them directly via the same `load_response` path
//! used by every other capture-driven test.

use std::path::Path;

use prost::Message;
use thetadatadx::wire as proto;

/// Wire `DataValue` for an integer column.
fn dv_num(n: i64) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Number(n)),
    }
}

/// Wire `DataValue` for a price column. The decoder converts the
/// `(value, type)` pair to f64 via `value * 10^(type - 10)`.
///
/// We carry every Greek and quote value at `type = 6` (-> `value * 1e-4`)
/// so a 4-decimal value like `0.7045` rides as `value = 7045`. This
/// matches the precision of the live wire shape for these endpoints
/// (the server emits Greeks at 4-decimal digits).
fn dv_price4(value_x_10000: i32) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Price(proto::Price {
            value: value_x_10000,
            r#type: 6,
        })),
    }
}

/// Wire `DataValue` for a price column at 2-decimal precision (USD
/// quotes / strike / underlying). Encoded with `type = 8` so
/// `value * 1e-2` returns the dollar amount.
fn dv_price2(value_x_100: i32) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Price(proto::Price {
            value: value_x_100,
            r#type: 8,
        })),
    }
}

/// Wire `DataValue` for a Timestamp column (`timestamp` and
/// `underlying_timestamp` wire headers). Encoded as NY epoch_ms; the
/// decoder converts to ms-of-day in NY zone via `row_number`'s
/// timestamp arm.
fn dv_timestamp(epoch_ms: u64) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Timestamp(
            proto::ZonedDateTime {
                epoch_ms,
                zone: proto::TimeZone::NewYork as i32,
            },
        )),
    }
}

/// Wire `DataValue` for a `Text` (string) column -- used for the
/// contract identification leading columns (`symbol` / `right`).
fn dv_text(s: &str) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Text(s.to_string())),
    }
}

/// 09:32:52.940 EDT on 2025-05-02 -> epoch_ms.
const TS_MS: u64 = 1_746_192_772_940;
/// 09:32:52.000 EDT on 2025-05-02 -> epoch_ms (underlying snapshot).
const UND_TS_MS: u64 = 1_746_192_772_000;
/// 2026-01-16 expiration, encoded as Number(20260116).
const EXP_YYYYMMDD: i64 = 20_260_116;
/// 2025-05-02 date row marker, encoded as Number(20250502).
const DATE_YYYYMMDD: i64 = 20_250_502;
/// $540.00 strike at 2-decimal `type = 8`: 54_000 * 1e-2 = 540.00.
const STRIKE_X100: i32 = 54_000;
/// $61.05 print price at 2-decimal `type = 8`: 6_105 * 1e-2 = 61.05.
const PRICE_X100: i32 = 6_105;
/// $565.13 underlying at 2-decimal `type = 8`: 56_513 * 1e-2 = 565.13.
const UND_PRICE_X100: i32 = 56_513;

/// Encode the standard `[symbol, expiration, strike, right]` lead-in
/// the v3 server emits on every wildcard-decoded historical endpoint.
fn lead_in_values() -> Vec<proto::DataValue> {
    vec![
        dv_text("SPY"),
        dv_num(EXP_YYYYMMDD),
        dv_price2(STRIKE_X100),
        dv_text("CALL"),
    ]
}

/// 9 trade-side wire values shared by every trade_greeks variant.
///
/// Matches captured row 1 verbatim: `sequence=1846852600,
/// ext_condition1..4=255, condition=18, size=1, exchange=31, price=61.05`.
fn trade_side_values() -> Vec<proto::DataValue> {
    vec![
        dv_timestamp(TS_MS),   // `timestamp` -> ms_of_day
        dv_num(1_846_852_600), // sequence
        dv_num(255),           // ext_condition1
        dv_num(255),           // ext_condition2
        dv_num(255),           // ext_condition3
        dv_num(255),           // ext_condition4
        dv_num(18),            // condition
        dv_num(1),             // size
        dv_num(31),            // exchange
        dv_price2(PRICE_X100), // price
    ]
}

/// Underlying snapshot tail shared by every variant.
fn underlying_tail() -> Vec<proto::DataValue> {
    vec![
        dv_timestamp(UND_TS_MS),   // `underlying_timestamp` -> underlying_ms_of_day
        dv_price2(UND_PRICE_X100), // underlying_price
    ]
}

/// Build the `option_history_trade_greeks_all` row (every Greek the
/// server publishes). Greek values are captured-row-1 verbatim.
fn build_all_row() -> proto::DataValueList {
    let greek = dv_price4; // 4-decimal-digit price encoding for the Greek values
    let mut v = lead_in_values();
    v.extend(trade_side_values());
    // First-order
    v.push(greek(7_045)); // delta=0.7045
    v.push(greek(-1_037)); // theta=-0.1037
    v.push(greek(1_643_710)); // vega=164.3710
    v.push(greek(2_392_069)); // rho=239.2069
    v.push(greek(-2_825_279)); // epsilon=-282.5279
    v.push(greek(65_217)); // lambda=6.5217
                           // Second-order
    v.push(greek(36)); // gamma=0.0036
    v.push(greek(-6_361)); // vanna=-0.6361
    v.push(greek(4)); // charm=0.0004
    v.push(greek(1_627_885)); // vomma=162.7885
    v.push(greek(1_159_427)); // veta=115.9427
    v.push(greek(-3_717_558)); // vera=-371.7558
                               // Third-order
    v.push(greek(0)); // speed=0
    v.push(greek(-145)); // zomma=-0.0145
    v.push(greek(-25)); // color=-0.0025
    v.push(greek(-23_948_608)); // ultima=-2394.8608
                                // BSM intermediates
    v.push(greek(5_375)); // d1=0.5375
    v.push(greek(3_688)); // d2=0.3688
    v.push(greek(-6_242)); // dual_delta=-0.6242
    v.push(greek(39)); // dual_gamma=0.0039
                       // IV pair
    v.push(greek(2_001)); // implied_vol=0.2001
    v.push(greek(0)); // iv_error=0
    v.extend(underlying_tail());
    proto::DataValueList { values: v }
}

fn build_first_order_row() -> proto::DataValueList {
    let greek = dv_price4;
    let mut v = lead_in_values();
    v.extend(trade_side_values());
    v.push(greek(7_045)); // delta
    v.push(greek(-1_037)); // theta
    v.push(greek(1_643_710)); // vega
    v.push(greek(2_392_069)); // rho
    v.push(greek(-2_825_279)); // epsilon
    v.push(greek(65_217)); // lambda
    v.push(greek(2_001)); // implied_vol
    v.push(greek(0)); // iv_error
    v.extend(underlying_tail());
    proto::DataValueList { values: v }
}

fn build_second_order_row() -> proto::DataValueList {
    let greek = dv_price4;
    let mut v = lead_in_values();
    v.extend(trade_side_values());
    v.push(greek(36)); // gamma
    v.push(greek(-6_361)); // vanna
    v.push(greek(4)); // charm
    v.push(greek(1_627_885)); // vomma
    v.push(greek(1_159_427)); // veta
    v.push(greek(2_001)); // implied_vol
    v.push(greek(0)); // iv_error
    v.extend(underlying_tail());
    proto::DataValueList { values: v }
}

fn build_third_order_row() -> proto::DataValueList {
    let greek = dv_price4;
    let mut v = lead_in_values();
    v.extend(trade_side_values());
    v.push(greek(0)); // speed
    v.push(greek(-145)); // zomma
    v.push(greek(-25)); // color
    v.push(greek(-23_948_608)); // ultima
    v.push(greek(2_001)); // implied_vol
    v.push(greek(0)); // iv_error
    v.extend(underlying_tail());
    proto::DataValueList { values: v }
}

fn build_iv_row() -> proto::DataValueList {
    let greek = dv_price4;
    let mut v = lead_in_values();
    v.extend(trade_side_values());
    v.push(greek(2_001)); // implied_vol
    v.push(greek(0)); // iv_error
    v.extend(underlying_tail());
    proto::DataValueList { values: v }
}

/// Write a single fixture: encode the DataTable, zstd-compress it,
/// wrap in ResponseData, encode the outer proto, and write to disk.
fn write_fixture(
    out_path: &Path,
    headers: Vec<&str>,
    rows: Vec<proto::DataValueList>,
) -> std::io::Result<()> {
    let table = proto::DataTable {
        headers: headers.iter().map(|s| s.to_string()).collect(),
        data_table: rows,
    };
    let table_bytes = table.encode_to_vec();
    let original_size = i32::try_from(table_bytes.len()).expect("table fits in i32");
    let compressed = zstd::stream::encode_all(table_bytes.as_slice(), 19)?;
    let response = proto::ResponseData {
        compressed_data: compressed,
        compression_description: Some(proto::CompressionDescription {
            algo: proto::CompressionAlgo::Zstd as i32,
            level: 19,
        }),
        original_size,
    };
    let outer = response.encode_to_vec();
    std::fs::write(out_path, outer)?;
    println!(
        "wrote {} ({} bytes)",
        out_path.display(),
        out_path.metadata()?.len()
    );
    Ok(())
}

fn main() -> std::io::Result<()> {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("captures");

    // `option_history_trade_greeks_all`
    write_fixture(
        &fixtures_dir.join("option_history_trade_greeks_all.pb.zst"),
        vec![
            "symbol",
            "expiration",
            "strike",
            "right",
            "timestamp",
            "sequence",
            "ext_condition1",
            "ext_condition2",
            "ext_condition3",
            "ext_condition4",
            "condition",
            "size",
            "exchange",
            "price",
            "delta",
            "theta",
            "vega",
            "rho",
            "epsilon",
            "lambda",
            "gamma",
            "vanna",
            "charm",
            "vomma",
            "veta",
            "vera",
            "speed",
            "zomma",
            "color",
            "ultima",
            "d1",
            "d2",
            "dual_delta",
            "dual_gamma",
            "implied_vol",
            "iv_error",
            "underlying_timestamp",
            "underlying_price",
        ],
        vec![build_all_row()],
    )?;

    // `option_history_trade_greeks_first_order`
    write_fixture(
        &fixtures_dir.join("option_history_trade_greeks_first_order.pb.zst"),
        vec![
            "symbol",
            "expiration",
            "strike",
            "right",
            "timestamp",
            "sequence",
            "ext_condition1",
            "ext_condition2",
            "ext_condition3",
            "ext_condition4",
            "condition",
            "size",
            "exchange",
            "price",
            "delta",
            "theta",
            "vega",
            "rho",
            "epsilon",
            "lambda",
            "implied_vol",
            "iv_error",
            "underlying_timestamp",
            "underlying_price",
        ],
        vec![build_first_order_row()],
    )?;

    // `option_history_trade_greeks_second_order`
    write_fixture(
        &fixtures_dir.join("option_history_trade_greeks_second_order.pb.zst"),
        vec![
            "symbol",
            "expiration",
            "strike",
            "right",
            "timestamp",
            "sequence",
            "ext_condition1",
            "ext_condition2",
            "ext_condition3",
            "ext_condition4",
            "condition",
            "size",
            "exchange",
            "price",
            "gamma",
            "vanna",
            "charm",
            "vomma",
            "veta",
            "implied_vol",
            "iv_error",
            "underlying_timestamp",
            "underlying_price",
        ],
        vec![build_second_order_row()],
    )?;

    // `option_history_trade_greeks_third_order`
    write_fixture(
        &fixtures_dir.join("option_history_trade_greeks_third_order.pb.zst"),
        vec![
            "symbol",
            "expiration",
            "strike",
            "right",
            "timestamp",
            "sequence",
            "ext_condition1",
            "ext_condition2",
            "ext_condition3",
            "ext_condition4",
            "condition",
            "size",
            "exchange",
            "price",
            "speed",
            "zomma",
            "color",
            "ultima",
            "implied_vol",
            "iv_error",
            "underlying_timestamp",
            "underlying_price",
        ],
        vec![build_third_order_row()],
    )?;

    // `option_history_trade_greeks_implied_volatility`
    write_fixture(
        &fixtures_dir.join("option_history_trade_greeks_implied_volatility.pb.zst"),
        vec![
            "symbol",
            "expiration",
            "strike",
            "right",
            "timestamp",
            "sequence",
            "ext_condition1",
            "ext_condition2",
            "ext_condition3",
            "ext_condition4",
            "condition",
            "size",
            "exchange",
            "price",
            "implied_vol",
            "iv_error",
            "underlying_timestamp",
            "underlying_price",
        ],
        vec![build_iv_row()],
    )?;

    // Avoid the unused-fn warning when DATE_YYYYMMDD isn't otherwise
    // referenced from a fixture body (only as a documentation anchor).
    let _ = DATE_YYYYMMDD;
    Ok(())
}
