//! Captured-response regression tests.
//!
//! Each fixture is a real `ResponseData` protobuf captured from the production
//! MDDS server via the `tdx` CLI, zstd-compressed, checked in under
//! `tests/fixtures/captures/<endpoint>.pb.zst`, and paired with a sibling
//! `<endpoint>.meta.toml` that anchors the row count, the exact server-sent
//! header list, and the first-row field values.
//!
//! These tests run the full decode pipeline the client actually uses —
//! `decode_data_table` → tick-type parser — and would have caught P11 (the
//! `TradeQuoteTick` empty-vec silent-fallback) at PR time by decoding the
//! stock + option `trade_quote` captures and asserting 8_192 / 98 rows rather
//! than 0.
//!
//! When the upstream server changes its header surface, add the new column
//! to `HEADER_ALIASES` (in `decode.rs`) and regenerate the affected fixture
//! with `TDX_CAPTURE_RAW=… tdx …` + `zstd -19 *.pb > *.pb.zst`.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use prost::Message;
use serde::Deserialize;
use tdbe::types::tick::{CalendarDay, EodTick, GreeksTick, OhlcTick, TradeQuoteTick, TradeTick};
use thetadatadx::decode::{self, DecodeError};
use thetadatadx::proto;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("captures")
}

/// Load a `.pb.zst` fixture, decompress it with zstd, and return the embedded
/// `ResponseData` protobuf. The decompression + proto decode errors are
/// intentionally panics: a broken fixture is a test-infra bug, not a product
/// bug.
fn load_response(endpoint: &str) -> proto::ResponseData {
    let path = fixtures_dir().join(format!("{endpoint}.pb.zst"));
    let f =
        fs::File::open(&path).unwrap_or_else(|e| panic!("open fixture {}: {e}", path.display()));
    let mut decoder = zstd::Decoder::new(f)
        .unwrap_or_else(|e| panic!("zstd::Decoder::new({}): {e}", path.display()));
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("zstd read_to_end {}: {e}", path.display()));
    proto::ResponseData::decode(bytes.as_slice())
        .unwrap_or_else(|e| panic!("proto::ResponseData::decode {}: {e}", path.display()))
}

/// Load the sibling `<endpoint>.meta.toml` into a flat TOML value map.
fn load_meta(endpoint: &str) -> toml::Value {
    let path = fixtures_dir().join(format!("{endpoint}.meta.toml"));
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read meta {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parse meta {}: {e}", path.display()))
}

fn meta_int(meta: &toml::Value, key: &str) -> i64 {
    meta.get(key)
        .and_then(toml::Value::as_integer)
        .unwrap_or_else(|| panic!("meta key {key} not an integer"))
}

fn meta_float(meta: &toml::Value, key: &str) -> f64 {
    meta.get(key)
        .and_then(toml::Value::as_float)
        .or_else(|| {
            meta.get(key)
                .and_then(toml::Value::as_integer)
                .map(|n| n as f64)
        })
        .unwrap_or_else(|| panic!("meta key {key} not a float"))
}

fn meta_str_array(meta: &toml::Value, key: &str) -> Vec<String> {
    meta.get(key)
        .and_then(toml::Value::as_array)
        .unwrap_or_else(|| panic!("meta key {key} not an array"))
        .iter()
        .map(|v| {
            v.as_str()
                .unwrap_or_else(|| panic!("meta key {key} element not a string"))
                .to_string()
        })
        .collect()
}

/// Verify that every header the fixture promises actually shows up in the
/// decoded `DataTable`, in order. Silent drift in upstream column names is
/// exactly the class of bug these fixtures exist to catch.
fn assert_headers(meta: &toml::Value, table: &proto::DataTable) {
    let expected = meta_str_array(meta, "expected_headers");
    assert_eq!(
        table.headers, expected,
        "header list drifted from fixture. server columns = {:?}, fixture = {:?}",
        table.headers, expected,
    );
}

fn assert_row_count(meta: &toml::Value, rows: usize) {
    let expected =
        usize::try_from(meta_int(meta, "expected_rows")).expect("expected_rows fits in usize");
    assert_eq!(
        rows, expected,
        "row count mismatch vs fixture: decoded {rows} rows, expected {expected}",
    );
}

/// Approx-eq for f64 prices. Prices come out of `row_price_f64` with finite
/// precision from the server's `price_type` exponent; 1e-9 is safe for USD
/// quotes and Greeks.
#[track_caller]
fn assert_f64_eq(field: &str, got: f64, expected: f64) {
    let tol = 1e-9_f64.max(expected.abs() * 1e-9);
    assert!(
        (got - expected).abs() < tol,
        "{field}: got {got}, expected {expected} (tol {tol})",
    );
}

#[derive(Debug, Deserialize)]
struct EndpointInfo {
    endpoint: String,
    tick_type: String,
}

fn load_endpoint_info(endpoint: &str) -> EndpointInfo {
    let path = fixtures_dir().join(format!("{endpoint}.meta.toml"));
    let text = fs::read_to_string(&path).expect("read meta");
    toml::from_str::<EndpointInfo>(&text).expect("deserialize meta")
}

// ────────────────────────────────────────────────────────────────────────────
// Per-endpoint tests. Each test:
//  1. loads the capture,
//  2. runs `decode_data_table` (same path the MddsClient uses),
//  3. asserts the row count and header list match the fixture,
//  4. runs the tick-specific parser and asserts first-row field values.
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn decode_captures_stock_history_trade_quote() {
    let endpoint = "stock_history_trade_quote";
    let info = load_endpoint_info(endpoint);
    assert_eq!(info.endpoint, endpoint);
    assert_eq!(info.tick_type, "TradeQuoteTick");

    let meta = load_meta(endpoint);
    let response = load_response(endpoint);
    let table = decode::decode_data_table(&response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<TradeQuoteTick> =
        decode::parse_trade_quote_ticks(&table).expect("parse_trade_quote_ticks");
    assert_eq!(ticks.len(), table.data_table.len(), "parser dropped rows");

    let first = ticks.first().expect("non-empty");
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_eq!(
        first.quote_ms_of_day as i64,
        meta_int(&meta, "first_row_quote_ms_of_day")
    );
    assert_eq!(first.sequence as i64, meta_int(&meta, "first_row_sequence"));
    assert_eq!(first.size as i64, meta_int(&meta, "first_row_size"));
    assert_eq!(first.exchange as i64, meta_int(&meta, "first_row_exchange"));
    assert_f64_eq(
        "first.price",
        first.price,
        meta_float(&meta, "first_row_price"),
    );
    assert_f64_eq("first.bid", first.bid, meta_float(&meta, "first_row_bid"));
    assert_f64_eq("first.ask", first.ask, meta_float(&meta, "first_row_ask"));
    assert_eq!(first.bid_size as i64, meta_int(&meta, "first_row_bid_size"));
    assert_eq!(first.ask_size as i64, meta_int(&meta, "first_row_ask_size"));
    assert_eq!(
        first.condition as i64,
        meta_int(&meta, "first_row_condition")
    );
}

#[test]
fn decode_captures_option_history_trade_quote() {
    let endpoint = "option_history_trade_quote";
    let info = load_endpoint_info(endpoint);
    assert_eq!(info.tick_type, "TradeQuoteTick");

    let meta = load_meta(endpoint);
    let response = load_response(endpoint);
    let table = decode::decode_data_table(&response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<TradeQuoteTick> =
        decode::parse_trade_quote_ticks(&table).expect("parse_trade_quote_ticks");
    assert_eq!(ticks.len(), table.data_table.len());

    let first = ticks.first().unwrap();
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_eq!(
        first.quote_ms_of_day as i64,
        meta_int(&meta, "first_row_quote_ms_of_day")
    );
    assert_eq!(first.sequence as i64, meta_int(&meta, "first_row_sequence"));
    assert_f64_eq("price", first.price, meta_float(&meta, "first_row_price"));
    assert_f64_eq("bid", first.bid, meta_float(&meta, "first_row_bid"));
    assert_f64_eq("ask", first.ask, meta_float(&meta, "first_row_ask"));
    assert_eq!(
        first.expiration as i64,
        meta_int(&meta, "first_row_expiration")
    );
    assert_f64_eq(
        "strike",
        first.strike,
        meta_float(&meta, "first_row_strike"),
    );
    assert_eq!(first.right as i64, meta_int(&meta, "first_row_right"));
}

#[test]
fn decode_captures_stock_history_eod() {
    let endpoint = "stock_history_eod";
    let meta = load_meta(endpoint);
    let response = load_response(endpoint);
    let table = decode::decode_data_table(&response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<EodTick> = decode::parse_eod_ticks(&table).expect("parse_eod_ticks");
    assert_eq!(ticks.len(), table.data_table.len());

    let first = ticks.first().unwrap();
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_eq!(
        first.ms_of_day2 as i64,
        meta_int(&meta, "first_row_ms_of_day2")
    );
    assert_f64_eq("open", first.open, meta_float(&meta, "first_row_open"));
    assert_f64_eq("high", first.high, meta_float(&meta, "first_row_high"));
    assert_f64_eq("low", first.low, meta_float(&meta, "first_row_low"));
    assert_f64_eq("close", first.close, meta_float(&meta, "first_row_close"));
    assert_eq!(first.volume, meta_int(&meta, "first_row_volume"));
    assert_eq!(first.count, meta_int(&meta, "first_row_count"));
    assert_f64_eq("bid", first.bid, meta_float(&meta, "first_row_bid"));
    assert_f64_eq("ask", first.ask, meta_float(&meta, "first_row_ask"));
}

#[test]
fn decode_captures_option_history_greeks_all() {
    let endpoint = "option_history_greeks_all";
    let meta = load_meta(endpoint);
    let response = load_response(endpoint);
    let table = decode::decode_data_table(&response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<GreeksTick> = decode::parse_greeks_ticks(&table).expect("parse_greeks_ticks");
    assert_eq!(ticks.len(), table.data_table.len());

    let first = ticks.first().unwrap();
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_eq!(
        first.expiration as i64,
        meta_int(&meta, "first_row_expiration")
    );
    assert_f64_eq(
        "strike",
        first.strike,
        meta_float(&meta, "first_row_strike"),
    );
    assert_eq!(first.right as i64, meta_int(&meta, "first_row_right"));
    assert_f64_eq("delta", first.delta, meta_float(&meta, "first_row_delta"));
    assert_f64_eq(
        "iv_error",
        first.iv_error,
        meta_float(&meta, "first_row_iv_error"),
    );
}

#[test]
fn decode_captures_option_history_trade() {
    let endpoint = "option_history_trade";
    let meta = load_meta(endpoint);
    let response = load_response(endpoint);
    let table = decode::decode_data_table(&response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<TradeTick> = decode::parse_trade_ticks(&table).expect("parse_trade_ticks");
    assert_eq!(ticks.len(), table.data_table.len());

    let first = ticks.first().unwrap();
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_eq!(first.sequence as i64, meta_int(&meta, "first_row_sequence"));
    assert_eq!(first.size as i64, meta_int(&meta, "first_row_size"));
    assert_eq!(first.exchange as i64, meta_int(&meta, "first_row_exchange"));
    assert_f64_eq("price", first.price, meta_float(&meta, "first_row_price"));
    assert_eq!(
        first.condition as i64,
        meta_int(&meta, "first_row_condition")
    );
    assert_eq!(
        first.expiration as i64,
        meta_int(&meta, "first_row_expiration")
    );
    assert_f64_eq(
        "strike",
        first.strike,
        meta_float(&meta, "first_row_strike"),
    );
    assert_eq!(first.right as i64, meta_int(&meta, "first_row_right"));
}

#[test]
fn decode_captures_option_snapshot_ohlc() {
    let endpoint = "option_snapshot_ohlc";
    let meta = load_meta(endpoint);
    let response = load_response(endpoint);
    let table = decode::decode_data_table(&response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<OhlcTick> = decode::parse_ohlc_ticks(&table).expect("parse_ohlc_ticks");
    assert_eq!(ticks.len(), table.data_table.len());

    let first = ticks.first().unwrap();
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_f64_eq("open", first.open, meta_float(&meta, "first_row_open"));
    assert_f64_eq("high", first.high, meta_float(&meta, "first_row_high"));
    assert_f64_eq("low", first.low, meta_float(&meta, "first_row_low"));
    assert_f64_eq("close", first.close, meta_float(&meta, "first_row_close"));
    assert_eq!(first.volume, meta_int(&meta, "first_row_volume"));
    assert_eq!(first.count, meta_int(&meta, "first_row_count"));
    assert_eq!(
        first.expiration as i64,
        meta_int(&meta, "first_row_expiration")
    );
    assert_f64_eq(
        "strike",
        first.strike,
        meta_float(&meta, "first_row_strike"),
    );
    assert_eq!(first.right as i64, meta_int(&meta, "first_row_right"));
}

#[test]
fn decode_captures_calendar_open_today() {
    let endpoint = "calendar_open_today";
    let meta = load_meta(endpoint);
    let response = load_response(endpoint);
    let table = decode::decode_data_table(&response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    // Calendar uses the hand-written v3 parser, not a generated tick parser.
    let days: Vec<CalendarDay> =
        decode::parse_calendar_days_v3(&table).expect("parse_calendar_days_v3");
    assert_eq!(days.len(), table.data_table.len());

    let first = days.first().unwrap();
    assert_eq!(first.is_open as i64, meta_int(&meta, "first_row_is_open"));
    assert_eq!(
        first.open_time as i64,
        meta_int(&meta, "first_row_open_time")
    );
    assert_eq!(
        first.close_time as i64,
        meta_int(&meta, "first_row_close_time")
    );
    assert_eq!(first.status as i64, meta_int(&meta, "first_row_status"));
    assert_eq!(first.date as i64, meta_int(&meta, "first_row_date"));
}

// ────────────────────────────────────────────────────────────────────────────
// Regression guards for the silent-fallback paths the fix removed.
// ────────────────────────────────────────────────────────────────────────────

/// When a required header is missing on a *non-empty* DataTable, the parser
/// must surface `DecodeError::MissingRequiredHeader`. This is the sensor
/// that would have caught P11 the moment the server added
/// `trade_timestamp` without a matching alias.
#[test]
fn missing_required_header_on_nonempty_table_errors_loudly() {
    // Build a DataTable that looks like a trade_quote response but *without*
    // the `trade_timestamp` alias being honored. We pretend the alias didn't
    // exist by using a bogus column name for the required `ms_of_day`.
    // Price 187.18 with price_type=6 encodes as raw value 18718 × 10^(6−10)
    // = 18718 × 1e-4 = 1.8718 — close enough for a schema-drift sentinel
    // row; the test only checks the MissingRequiredHeader branch fires.
    let headers = vec!["not_ms_of_day".to_string(), "price".to_string()];
    let rows = vec![proto::DataValueList {
        values: vec![
            proto::DataValue {
                data_type: Some(proto::data_value::DataType::Number(34_200_000)),
            },
            proto::DataValue {
                data_type: Some(proto::data_value::DataType::Price(proto::Price {
                    value: 18718,
                    r#type: 6,
                })),
            },
        ],
    }];
    let table = proto::DataTable {
        headers,
        data_table: rows,
    };

    let err = decode::parse_trade_quote_ticks(&table).unwrap_err();
    match err {
        DecodeError::MissingRequiredHeader { header, rows, .. } => {
            assert_eq!(header, "ms_of_day");
            assert_eq!(rows, 1);
        }
        other => panic!("expected MissingRequiredHeader, got {other:?}"),
    }
}

/// Empty responses (no rows) stay on the `Ok(vec![])` path — no schema-drift
/// inference possible when there's nothing to schema-drift against. This
/// preserves legitimate "no trades on a holiday" behavior.
#[test]
fn empty_response_without_required_header_still_returns_empty_vec() {
    let table = proto::DataTable {
        headers: vec!["irrelevant".to_string()],
        data_table: vec![],
    };
    let ticks = decode::parse_trade_quote_ticks(&table).expect("empty is legal");
    assert!(ticks.is_empty());
}
