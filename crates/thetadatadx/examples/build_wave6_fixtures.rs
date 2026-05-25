//! Build verified-live fixtures for the wave-6 audit closure:
//!   * `option_history_greeks_eod` -> `GreeksEodTick`
//!   * `index_at_time_price`        -> `IndexPriceAtTimeTick`
//!
//! Each fixture is a single `proto::ResponseData` whose `compressed_data`
//! is a zstd-compressed `proto::DataTable` with the EXACT wire shape the
//! terminal jar build `202605221` emitted (queried via `curl
//! http://127.0.0.1:25503/v3/...`).
//!
//! Headers come from the captured CSV header row; the per-row values are
//! captured CSV row 1 verbatim, encoded as the same `DataValue` shapes
//! the live server uses (Number for ints/dates, Price for prices/floats,
//! Timestamp for timestamps).
//!
//! Run with: `cargo run -p thetadatadx --example build_wave6_fixtures`
//! Writes:
//!   - `crates/thetadatadx/tests/fixtures/captures/option_history_greeks_eod.pb.zst`
//!   - `crates/thetadatadx/tests/fixtures/captures/index_at_time_price.pb.zst`
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

/// Wire `DataValue` for a price column at 4-decimal precision (Greeks).
fn dv_price4(value_x_10000: i32) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Price(proto::Price {
            value: value_x_10000,
            r#type: 6,
        })),
    }
}

/// Wire `DataValue` for a price column at 2-decimal precision (USD).
fn dv_price2(value_x_100: i32) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Price(proto::Price {
            value: value_x_100,
            r#type: 8,
        })),
    }
}

/// Wire `DataValue` for a Timestamp column. The decoder converts to
/// ms-of-day (New York) via `row_number`'s timestamp arm.
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

/// Wire `DataValue` for a `Text` column (contract id `symbol` / `right`).
fn dv_text(s: &str) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Text(s.to_string())),
    }
}

// ────────────────────────────────────────────────────────────────────────
// option_history_greeks_eod -- SPY 2024-06-21 500 CALL on 2024-06-14
// ────────────────────────────────────────────────────────────────────────
//
// CSV row 1 (verified-live from terminal jar build `202605221`):
//   "SPY","2024-06-21",500.000,"CALL",
//   2024-06-14T16:11:12.295,41.71,42.78,40.48,42.78,683,99,
//   325,5,42.39,50,418,5,43.25,50,
//   1.0000,0.0000,0.0000,0.0000,0.0000,0.0000,0.0000,
//   0.0000,0.0000,0.0000,0.0000,0.0000,0.0000,0.0000,0.0000,0.0000,
//   0.0000,0.0000,0.0000,0.0000,0.0000,0.0109,
//   2024-06-14T17:15:39.187,542.7799
//
// Deep-ITM call with delta clamped to 1.0; zero second/third-order Greeks
// is expected at this depth. iv_error=0.0109 anchors the IV-pair decoding.

/// 16:11:12.295 EDT on 2024-06-14 -> epoch_ms.
const EOD_TRADE_TS_MS: u64 = 1_718_395_872_295;
/// 17:15:39.187 EDT on 2024-06-14 -> epoch_ms (EOD underlying snapshot).
const EOD_UND_TS_MS: u64 = 1_718_399_739_187;
/// 2024-06-21 expiration, encoded as Number(20240621).
const EOD_EXP_YYYYMMDD: i64 = 20_240_621;
/// 2024-06-14 date row marker (parser derives from the timestamp).
const EOD_DATE_YYYYMMDD: i64 = 20_240_614;
/// $500.00 strike at 2-decimal `type = 8`.
const EOD_STRIKE_X100: i32 = 50_000;
/// $542.7799 underlying at 4-decimal `type = 6` (16,777,216 fits in i32 ok).
const EOD_UND_PRICE_X10000: i32 = 5_427_799;

fn build_greeks_eod_row() -> proto::DataValueList {
    let values = vec![
        // Contract id lead-in.
        dv_text("SPY"),
        dv_num(EOD_EXP_YYYYMMDD),
        dv_price2(EOD_STRIKE_X100),
        dv_text("CALL"),
        // EOD bar header.
        dv_timestamp(EOD_TRADE_TS_MS),
        dv_price2(4_171), // open=41.71
        dv_price2(4_278), // high=42.78
        dv_price2(4_048), // low=40.48
        dv_price2(4_278), // close=42.78
        dv_num(683),      // volume
        dv_num(99),       // count
        dv_num(325),      // bid_size
        dv_num(5),        // bid_exchange
        dv_price2(4_239), // bid=42.39
        dv_num(50),       // bid_condition
        dv_num(418),      // ask_size
        dv_num(5),        // ask_exchange
        dv_price2(4_325), // ask=43.25
        dv_num(50),       // ask_condition
        // Greeks (4-decimal precision).
        dv_price4(10_000), // delta=1.0000
        dv_price4(0),      // theta
        dv_price4(0),      // vega
        dv_price4(0),      // rho
        dv_price4(0),      // epsilon
        dv_price4(0),      // lambda
        dv_price4(0),      // gamma
        dv_price4(0),      // vanna
        dv_price4(0),      // charm
        dv_price4(0),      // vomma
        dv_price4(0),      // veta
        dv_price4(0),      // vera
        dv_price4(0),      // speed
        dv_price4(0),      // zomma
        dv_price4(0),      // color
        dv_price4(0),      // ultima
        dv_price4(0),      // d1
        dv_price4(0),      // d2
        dv_price4(0),      // dual_delta
        dv_price4(0),      // dual_gamma
        dv_price4(0),      // implied_vol
        dv_price4(109),    // iv_error=0.0109
        dv_timestamp(EOD_UND_TS_MS), // underlying_timestamp
        dv_price4(EOD_UND_PRICE_X10000), // underlying_price=542.7799
    ];
    proto::DataValueList { values }
}

// ────────────────────────────────────────────────────────────────────────
// index_at_time_price -- SPX 2024-06-14 10:30:00 ET
// ────────────────────────────────────────────────────────────────────────
//
// CSV row 1 (verified-live from terminal jar build `202605221`):
//   2024-06-14T10:30:00.000,0,255,255,255,255,0,0,5,5414.14
//
// Index prints carry sequence=0 (no SIP sequence assigned), ext_conditions
// at the wildcard sentinel 255, condition=0, size=0; only `exchange=5`
// (the SIP exchange code) and `price=5414.14` are meaningful per row.
// Wave-6 SERIOUS closure: those nine trade-side columns were silently
// dropped when the endpoint routed through `PriceTick` (3 columns).

/// 10:30:00.000 EDT on 2024-06-14 -> epoch_ms.
const INDEX_TS_MS: u64 = 1_718_375_400_000;
/// $5414.14 SPX at 2-decimal `type = 8`: 541_414 * 1e-2.
const INDEX_PRICE_X100: i32 = 541_414;

fn build_index_at_time_row() -> proto::DataValueList {
    let v = vec![
        dv_timestamp(INDEX_TS_MS),   // timestamp -> ms_of_day + date
        dv_num(0),                   // sequence
        dv_num(255),                 // ext_condition1
        dv_num(255),                 // ext_condition2
        dv_num(255),                 // ext_condition3
        dv_num(255),                 // ext_condition4
        dv_num(0),                   // condition
        dv_num(0),                   // size
        dv_num(5),                   // exchange (SIP source = CBOE)
        dv_price2(INDEX_PRICE_X100), // price=5414.14
    ];
    proto::DataValueList { values: v }
}

// ────────────────────────────────────────────────────────────────────────
// Common writer
// ────────────────────────────────────────────────────────────────────────

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

    // option_history_greeks_eod -- 43 wire columns.
    write_fixture(
        &fixtures_dir.join("option_history_greeks_eod.pb.zst"),
        vec![
            "symbol",
            "expiration",
            "strike",
            "right",
            "timestamp",
            "open",
            "high",
            "low",
            "close",
            "volume",
            "count",
            "bid_size",
            "bid_exchange",
            "bid",
            "bid_condition",
            "ask_size",
            "ask_exchange",
            "ask",
            "ask_condition",
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
        vec![build_greeks_eod_row()],
    )?;

    // index_at_time_price -- 10 wire columns.
    write_fixture(
        &fixtures_dir.join("index_at_time_price.pb.zst"),
        vec![
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
        ],
        vec![build_index_at_time_row()],
    )?;

    // Date marker is derived from the timestamp by the parser. Reference
    // it here so unused-const lints stay quiet without `#[allow]`.
    let _ = EOD_DATE_YYYYMMDD;
    Ok(())
}
