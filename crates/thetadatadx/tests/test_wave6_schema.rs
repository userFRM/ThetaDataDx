//! Wave-6 audit closure regression tests.
//!
//! Two fixtures replay the verified-live wire shapes (terminal jar build
//! `202605221`) for:
//!
//!   * `option_history_greeks_eod` (BLOCKER) -- the bare `GreeksAllTick`
//!     (28 columns) silently dropped the twelve EOD trade/quote columns
//!     (`open`, `high`, `low`, `close`, `volume`, `count`, `bid_size`,
//!     `bid_exchange`, `bid_condition`, `ask_size`, `ask_exchange`,
//!     `ask_condition`) from the 39-column EOD response. The new
//!     `GreeksEodTick` carries the full wire shape end-to-end.
//!
//!   * `index_at_time_price` (SERIOUS) -- the bare `PriceTick` (3
//!     columns) silently dropped the seven trade-side execution columns
//!     (`sequence`, `ext_condition1..4`, `condition`, `size`,
//!     `exchange`) including the SIP-exchange attribution field. The new
//!     `IndexPriceAtTimeTick` carries the full trade-shaped wire row.
//!
//! These tests would have caught both regressions at PR time: the
//! `expected_headers` assert pins the upstream column list and the
//! typed first-row asserts pin the twelve EOD / seven trade-side
//! columns that the silent-routing dropped.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use prost::Message;
use tdbe::types::tick::{GreeksEodTick, IndexPriceAtTimeTick};
use thetadatadx::decode;
use thetadatadx::wire as proto;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("captures")
}

fn load_response(endpoint: &str) -> proto::ResponseData {
    let path = fixtures_dir().join(format!("{endpoint}.pb.zst"));
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
    // Older fixtures are zstd-wrapped at the file level; newer fixtures
    // (PR #605 onwards, and these wave-6 fixtures) carry the raw
    // `ResponseData` proto bytes with the zstd payload on the inner
    // `compressed_data` field. Sniff the zstd frame magic to pick the
    // right path.
    if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let mut decoder = zstd::Decoder::new(&bytes[..])
            .unwrap_or_else(|e| panic!("zstd::Decoder::new({}): {e}", path.display()));
        let mut inner = Vec::new();
        decoder
            .read_to_end(&mut inner)
            .unwrap_or_else(|e| panic!("zstd read_to_end {}: {e}", path.display()));
        proto::ResponseData::decode(inner.as_slice())
            .unwrap_or_else(|e| panic!("proto::ResponseData::decode {}: {e}", path.display()))
    } else {
        proto::ResponseData::decode(bytes.as_slice())
            .unwrap_or_else(|e| panic!("proto::ResponseData::decode {}: {e}", path.display()))
    }
}

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

// ────────────────────────────────────────────────────────────────────────
// BLOCKER: option_history_greeks_eod -> GreeksEodTick
// ────────────────────────────────────────────────────────────────────────

#[test]
fn decode_greeks_eod_carries_twelve_eod_columns_and_every_greek() {
    let endpoint = "option_history_greeks_eod";
    let meta = load_meta(endpoint);
    let mut response = load_response(endpoint);
    let table = decode::decode_data_table(&mut response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<GreeksEodTick> =
        decode::parse_greeks_eod_ticks(&table).expect("parse_greeks_eod_ticks");
    assert_eq!(ticks.len(), table.data_table.len(), "parser dropped rows");

    let first = ticks.first().expect("non-empty");

    // Twelve EOD trade/quote columns -- the data the bare `GreeksAllTick`
    // silently dropped. Pinning all twelve here catches any regression
    // that routes `option_history_greeks_eod` back through the
    // interval-sampled `GreeksAllTicks` collection.
    assert_f64_eq("open", first.open, meta_float(&meta, "first_row_open"));
    assert_f64_eq("high", first.high, meta_float(&meta, "first_row_high"));
    assert_f64_eq("low", first.low, meta_float(&meta, "first_row_low"));
    assert_f64_eq("close", first.close, meta_float(&meta, "first_row_close"));
    assert_eq!(first.volume, meta_int(&meta, "first_row_volume"));
    assert_eq!(first.count, meta_int(&meta, "first_row_count"));
    assert_eq!(first.bid_size as i64, meta_int(&meta, "first_row_bid_size"),);
    assert_eq!(
        first.bid_exchange as i64,
        meta_int(&meta, "first_row_bid_exchange"),
    );
    assert_eq!(
        first.bid_condition as i64,
        meta_int(&meta, "first_row_bid_condition"),
    );
    assert_eq!(first.ask_size as i64, meta_int(&meta, "first_row_ask_size"),);
    assert_eq!(
        first.ask_exchange as i64,
        meta_int(&meta, "first_row_ask_exchange"),
    );
    assert_eq!(
        first.ask_condition as i64,
        meta_int(&meta, "first_row_ask_condition"),
    );

    // Quote pair (also in the bare `GreeksAllTick`, pinned for
    // completeness so the structural drift would surface here).
    assert_f64_eq("bid", first.bid, meta_float(&meta, "first_row_bid"));
    assert_f64_eq("ask", first.ask, meta_float(&meta, "first_row_ask"));

    // Timestamp + contract identity.
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day"),
    );
    assert_eq!(
        first.expiration as i64,
        meta_int(&meta, "first_row_expiration"),
    );
    assert_f64_eq(
        "strike",
        first.strike,
        meta_float(&meta, "first_row_strike"),
    );
    assert_eq!(first.right as i64, meta_int(&meta, "first_row_right"));

    // Greek anchors -- delta = 1.0 (deep-ITM clamp) + iv_error anchor
    // the 4-decimal Greek-precision decode through `dv_price4`.
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

// ────────────────────────────────────────────────────────────────────────
// SERIOUS: index_at_time_price -> IndexPriceAtTimeTick
// ────────────────────────────────────────────────────────────────────────

#[test]
fn decode_index_at_time_price_carries_seven_trade_side_columns() {
    let endpoint = "index_at_time_price";
    let meta = load_meta(endpoint);
    let mut response = load_response(endpoint);
    let table = decode::decode_data_table(&mut response).expect("decode_data_table");

    assert_headers(&meta, &table);
    assert_row_count(&meta, table.data_table.len());

    let ticks: Vec<IndexPriceAtTimeTick> =
        decode::parse_index_price_at_time_ticks(&table).expect("parse_index_price_at_time_ticks");
    assert_eq!(ticks.len(), table.data_table.len(), "parser dropped rows");

    let first = ticks.first().expect("non-empty");

    // Seven trade-side execution columns -- the data the bare
    // `PriceTick` silently dropped. Pinning all seven here catches any
    // regression that routes `index_at_time_price` back through the
    // bare `PriceTicks` collection. `exchange = 5` is the SIP source
    // code (CBOE), the per-row attribution field that was lost.
    assert_eq!(first.sequence as i64, meta_int(&meta, "first_row_sequence"),);
    assert_eq!(
        first.ext_condition1 as i64,
        meta_int(&meta, "first_row_ext_condition1"),
    );
    assert_eq!(
        first.ext_condition2 as i64,
        meta_int(&meta, "first_row_ext_condition2"),
    );
    assert_eq!(
        first.ext_condition3 as i64,
        meta_int(&meta, "first_row_ext_condition3"),
    );
    assert_eq!(
        first.ext_condition4 as i64,
        meta_int(&meta, "first_row_ext_condition4"),
    );
    assert_eq!(
        first.condition as i64,
        meta_int(&meta, "first_row_condition"),
    );
    assert_eq!(first.size as i64, meta_int(&meta, "first_row_size"));
    assert_eq!(first.exchange as i64, meta_int(&meta, "first_row_exchange"));

    // Timestamp + price.
    assert_eq!(
        first.ms_of_day as i64,
        meta_int(&meta, "first_row_ms_of_day"),
    );
    assert_f64_eq("price", first.price, meta_float(&meta, "first_row_price"));
    assert_eq!(first.date as i64, meta_int(&meta, "first_row_date"));
}
