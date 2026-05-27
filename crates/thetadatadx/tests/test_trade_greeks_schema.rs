//! `option_history_trade_greeks_*` regression tests (BL-14).
//!
//! Five fixtures replay the verified-live wire shape (terminal jar build
//! `202605221`) for the five `option_history_trade_greeks_*` endpoints
//! and assert that the new sibling tick types preserve every column the
//! server publishes -- specifically the nine trade-side execution
//! columns (`sequence`, `ext_condition1..4`, `condition`, `size`,
//! `exchange`, `price`) that the v10 line silently dropped by routing
//! these endpoints through the interval-sampled `Greeks*Tick` parsers.
//!
//! These tests would have caught BL-14 the moment the silent reroute
//! landed: the old parsers either errored on the missing `bid`/`ask`
//! columns or zero-filled them while dropping every trade column -- the
//! `expected_headers` assert and the typed first-row asserts here pin
//! both shapes.

use std::fs;

use thetadatadx::decode;
use thetadatadx::wire as proto;
use thetadatadx::{
    TradeGreeksAllTick, TradeGreeksFirstOrderTick, TradeGreeksImpliedVolatilityTick,
    TradeGreeksSecondOrderTick, TradeGreeksThirdOrderTick,
};

#[path = "common/capture_loader.rs"]
mod capture_loader;

use capture_loader::{fixtures_dir, load_response_data as load_response};

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

#[track_caller]
fn assert_f64_eq(field: &str, got: f64, expected: f64) {
    let tol = 1e-4_f64.max(expected.abs() * 1e-4);
    assert!(
        (got - expected).abs() < tol,
        "{field}: got {got}, expected {expected} (tol {tol})",
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Per-endpoint tests. Each pins the verified-live header list, decodes
// through the production `decode_data_table` -> per-tick parser path,
// and asserts the nine trade-side columns are present AND the Greek
// values match the captured row 1 verbatim.
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn decode_trade_greeks_all_carries_trade_columns_and_every_greek() {
    let endpoint = "option_history_trade_greeks_all";
    let meta = load_meta(endpoint);
    let mut response = load_response(endpoint);
    let table = decode::decode_data_table(&mut response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<TradeGreeksAllTick> =
        decode::parse_trade_greeks_all_ticks(&table).expect("parse_trade_greeks_all_ticks");
    assert_eq!(ticks.len(), table.data_table.len(), "parser dropped rows");

    let first = ticks.first().expect("non-empty");
    // Trade-side execution columns -- the nine fields the silent
    // mis-routing dropped. Pinning all nine here catches any
    // regression that routes the endpoint through a non-trade Greeks
    // parser.
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_eq!(first.sequence as i64, meta_int(&meta, "first_row_sequence"));
    assert_eq!(
        first.ext_condition1 as i64,
        meta_int(&meta, "first_row_ext_condition1")
    );
    assert_eq!(
        first.ext_condition2 as i64,
        meta_int(&meta, "first_row_ext_condition2")
    );
    assert_eq!(
        first.ext_condition3 as i64,
        meta_int(&meta, "first_row_ext_condition3")
    );
    assert_eq!(
        first.ext_condition4 as i64,
        meta_int(&meta, "first_row_ext_condition4")
    );
    assert_eq!(
        first.condition as i64,
        meta_int(&meta, "first_row_condition")
    );
    assert_eq!(first.size as i64, meta_int(&meta, "first_row_size"));
    assert_eq!(first.exchange as i64, meta_int(&meta, "first_row_exchange"));
    assert_f64_eq("price", first.price, meta_float(&meta, "first_row_price"));
    // Contract identification (auto-injected from the lead-in columns).
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
    // Greek anchors -- delta / gamma + IV pair are the same identity
    // checks the `option_history_greeks_all` fixture uses.
    assert_f64_eq("delta", first.delta, meta_float(&meta, "first_row_delta"));
    assert_f64_eq("gamma", first.gamma, meta_float(&meta, "first_row_gamma"));
    assert_f64_eq(
        "implied_volatility",
        first.implied_volatility,
        meta_float(&meta, "first_row_implied_volatility"),
    );
    assert_f64_eq(
        "iv_error",
        first.iv_error,
        meta_float(&meta, "first_row_iv_error"),
    );
    // Underlying snapshot tail.
    assert_eq!(
        first.underlying_ms_of_day as i64,
        meta_int(&meta, "first_row_underlying_ms_of_day"),
    );
    assert_f64_eq(
        "underlying_price",
        first.underlying_price,
        meta_float(&meta, "first_row_underlying_price"),
    );
    assert_eq!(first.date as i64, meta_int(&meta, "first_row_date"));
}

#[test]
fn decode_trade_greeks_first_order_carries_trade_columns() {
    let endpoint = "option_history_trade_greeks_first_order";
    let meta = load_meta(endpoint);
    let mut response = load_response(endpoint);
    let table = decode::decode_data_table(&mut response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<TradeGreeksFirstOrderTick> =
        decode::parse_trade_greeks_first_order_ticks(&table)
            .expect("parse_trade_greeks_first_order_ticks");
    assert_eq!(ticks.len(), table.data_table.len());

    let first = ticks.first().expect("non-empty");
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_eq!(first.sequence as i64, meta_int(&meta, "first_row_sequence"));
    assert_eq!(first.size as i64, meta_int(&meta, "first_row_size"));
    assert_eq!(first.exchange as i64, meta_int(&meta, "first_row_exchange"));
    assert_eq!(
        first.condition as i64,
        meta_int(&meta, "first_row_condition")
    );
    assert_f64_eq("price", first.price, meta_float(&meta, "first_row_price"));
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
    assert_f64_eq("theta", first.theta, meta_float(&meta, "first_row_theta"));
    assert_f64_eq(
        "implied_volatility",
        first.implied_volatility,
        meta_float(&meta, "first_row_implied_volatility"),
    );
    assert_eq!(
        first.underlying_ms_of_day as i64,
        meta_int(&meta, "first_row_underlying_ms_of_day"),
    );
    assert_f64_eq(
        "underlying_price",
        first.underlying_price,
        meta_float(&meta, "first_row_underlying_price"),
    );
    assert_eq!(first.date as i64, meta_int(&meta, "first_row_date"));
}

#[test]
fn decode_trade_greeks_second_order_carries_trade_columns() {
    let endpoint = "option_history_trade_greeks_second_order";
    let meta = load_meta(endpoint);
    let mut response = load_response(endpoint);
    let table = decode::decode_data_table(&mut response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<TradeGreeksSecondOrderTick> =
        decode::parse_trade_greeks_second_order_ticks(&table)
            .expect("parse_trade_greeks_second_order_ticks");
    assert_eq!(ticks.len(), table.data_table.len());

    let first = ticks.first().expect("non-empty");
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_eq!(first.sequence as i64, meta_int(&meta, "first_row_sequence"));
    assert_eq!(first.size as i64, meta_int(&meta, "first_row_size"));
    assert_eq!(first.exchange as i64, meta_int(&meta, "first_row_exchange"));
    assert_eq!(
        first.condition as i64,
        meta_int(&meta, "first_row_condition")
    );
    assert_f64_eq("price", first.price, meta_float(&meta, "first_row_price"));
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
    assert_f64_eq("gamma", first.gamma, meta_float(&meta, "first_row_gamma"));
    assert_f64_eq("vanna", first.vanna, meta_float(&meta, "first_row_vanna"));
    assert_f64_eq(
        "implied_volatility",
        first.implied_volatility,
        meta_float(&meta, "first_row_implied_volatility"),
    );
}

#[test]
fn decode_trade_greeks_third_order_carries_trade_columns() {
    let endpoint = "option_history_trade_greeks_third_order";
    let meta = load_meta(endpoint);
    let mut response = load_response(endpoint);
    let table = decode::decode_data_table(&mut response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<TradeGreeksThirdOrderTick> =
        decode::parse_trade_greeks_third_order_ticks(&table)
            .expect("parse_trade_greeks_third_order_ticks");
    assert_eq!(ticks.len(), table.data_table.len());

    let first = ticks.first().expect("non-empty");
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_eq!(first.sequence as i64, meta_int(&meta, "first_row_sequence"));
    assert_eq!(first.size as i64, meta_int(&meta, "first_row_size"));
    assert_f64_eq("price", first.price, meta_float(&meta, "first_row_price"));
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
    assert_f64_eq("speed", first.speed, meta_float(&meta, "first_row_speed"));
    assert_f64_eq("zomma", first.zomma, meta_float(&meta, "first_row_zomma"));
    assert_f64_eq(
        "ultima",
        first.ultima,
        meta_float(&meta, "first_row_ultima"),
    );
    assert_f64_eq(
        "implied_volatility",
        first.implied_volatility,
        meta_float(&meta, "first_row_implied_volatility"),
    );
}

#[test]
fn decode_trade_greeks_implied_volatility_carries_trade_columns() {
    let endpoint = "option_history_trade_greeks_implied_volatility";
    let meta = load_meta(endpoint);
    let mut response = load_response(endpoint);
    let table = decode::decode_data_table(&mut response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<TradeGreeksImpliedVolatilityTick> =
        decode::parse_trade_greeks_implied_volatility_ticks(&table)
            .expect("parse_trade_greeks_implied_volatility_ticks");
    assert_eq!(ticks.len(), table.data_table.len());

    let first = ticks.first().expect("non-empty");
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day")
    );
    assert_eq!(first.sequence as i64, meta_int(&meta, "first_row_sequence"));
    assert_eq!(first.size as i64, meta_int(&meta, "first_row_size"));
    assert_f64_eq("price", first.price, meta_float(&meta, "first_row_price"));
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
    assert_f64_eq(
        "implied_volatility",
        first.implied_volatility,
        meta_float(&meta, "first_row_implied_volatility"),
    );
    assert_eq!(
        first.underlying_ms_of_day as i64,
        meta_int(&meta, "first_row_underlying_ms_of_day"),
    );
}
