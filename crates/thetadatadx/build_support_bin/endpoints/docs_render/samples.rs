//! Sample-output decoding for the endpoint reference pages.
//!
//! Each endpoint that has a checked-in capture fixture
//! (`tests/fixtures/captures/<endpoint>.pb.zst` — a real production
//! response, zstd-wrapped protobuf) gets a small example-response table
//! decoded through the production pipeline (`decode_data_table` + the
//! generated per-tick parser). Endpoints without a capture render the
//! schema only — sample data is never fabricated.

use std::io::Read as _;
use std::path::PathBuf;

use prost::Message as _;

use super::super::model::GeneratedEndpoint;

const SAMPLE_ROWS: usize = 3;

/// Decoded sample rows from a capture fixture, ready for the example
/// table on a reference page.
pub(super) struct DecodedSample {
    /// First rows, stringified in `tick_schema.toml` column order.
    pub(super) rows: Vec<Vec<String>>,
    /// Total row count of the capture.
    pub(super) total_rows: usize,
}

fn captures_dir() -> PathBuf {
    // The generator binary sets its cwd to the package root.
    PathBuf::from("tests/fixtures/captures")
}

/// Format an `f64` for the sample table: up to six decimals, trailing
/// zeros trimmed (display precision only — values come straight from
/// the production decode pipeline).
fn fmt_f64(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Stringify the first rows of a decoded tick vec using a field list
/// that mirrors the `columns` order in `tick_schema.toml`. The row
/// width is asserted against the schema by the caller.
macro_rules! rows {
    ($ticks:expr, [$($field:ident : $kind:tt),+ $(,)?]) => {{
        let ticks = $ticks;
        let total = ticks.len();
        let rows: Vec<Vec<String>> = ticks
            .iter()
            .take(SAMPLE_ROWS)
            .map(|t| vec![$(rows!(@cell t, $field, $kind)),+])
            .collect();
        DecodedSample { rows, total_rows: total }
    }};
    (@cell $t:ident, $field:ident, f) => { fmt_f64($t.$field) };
    (@cell $t:ident, $field:ident, i) => { $t.$field.to_string() };
    (@cell $t:ident, $field:ident, s) => { $t.$field.to_string() };
}

/// Decode the capture for `endpoint`, if one exists.
pub(super) fn decode_capture(
    endpoint: &GeneratedEndpoint,
) -> Result<Option<DecodedSample>, Box<dyn std::error::Error>> {
    let path = captures_dir().join(format!("{}.pb.zst", endpoint.name));
    if !path.exists() {
        return Ok(None);
    }

    let bytes = std::fs::read(&path)?;
    let mut response = if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let mut decoder = zstd::Decoder::new(&bytes[..])?;
        let mut inner = Vec::new();
        decoder.read_to_end(&mut inner)?;
        thetadatadx::wire::ResponseData::decode(inner.as_slice())?
    } else {
        thetadatadx::wire::ResponseData::decode(bytes.as_slice())?
    };
    let table = thetadatadx::decode::decode_data_table(&mut response)?;

    use thetadatadx::decode as d;
    let sample = match endpoint.return_type.as_str() {
        "TradeTicks" => rows!(d::parse_trade_ticks(&table)?, [
            ms_of_day: i, sequence: i, ext_condition1: i, ext_condition2: i,
            ext_condition3: i, ext_condition4: i, condition: i, size: i,
            exchange: i, price: f, condition_flags: i, price_flags: i,
            volume_type: i, records_back: i, date: i,
        ]),
        "TradeQuoteTicks" => rows!(d::parse_trade_quote_ticks(&table)?, [
            ms_of_day: i, sequence: i, ext_condition1: i, ext_condition2: i,
            ext_condition3: i, ext_condition4: i, condition: i, size: i,
            exchange: i, price: f, condition_flags: i, price_flags: i,
            volume_type: i, records_back: i, quote_ms_of_day: i, bid_size: i,
            bid_exchange: i, bid: f, bid_condition: i, ask_size: i,
            ask_exchange: i, ask: f, ask_condition: i, date: i,
        ]),
        "OhlcTicks" => rows!(d::parse_ohlc_ticks(&table)?, [
            ms_of_day: i, open: f, high: f, low: f, close: f, volume: i,
            count: i, vwap: f, date: i,
        ]),
        "EodTicks" => rows!(d::parse_eod_ticks(&table)?, [
            created_ms_of_day: i, last_trade_ms_of_day: i, open: f, high: f, low: f, close: f,
            volume: i, count: i, bid_size: i, bid_exchange: i, bid: f,
            bid_condition: i, ask_size: i, ask_exchange: i, ask: f,
            ask_condition: i, date: i,
        ]),
        "GreeksAllTicks" => rows!(d::parse_greeks_all_ticks(&table)?, [
            ms_of_day: i, bid: f, ask: f, implied_volatility: f, delta: f,
            gamma: f, theta: f, vega: f, rho: f, iv_error: f, vanna: f,
            charm: f, vomma: f, veta: f, speed: f, zomma: f, color: f,
            ultima: f, d1: f, d2: f, dual_delta: f, dual_gamma: f,
            epsilon: f, lambda: f, vera: f, underlying_ms_of_day: i,
            underlying_price: f, date: i,
        ]),
        "GreeksEodTicks" => rows!(d::parse_greeks_eod_ticks(&table)?, [
            ms_of_day: i, open: f, high: f, low: f, close: f, volume: i,
            count: i, bid_size: i, bid_exchange: i, bid: f, bid_condition: i,
            ask_size: i, ask_exchange: i, ask: f, ask_condition: i, delta: f,
            theta: f, vega: f, rho: f, epsilon: f, lambda: f, gamma: f,
            vanna: f, charm: f, vomma: f, veta: f, vera: f, speed: f,
            zomma: f, color: f, ultima: f, d1: f, d2: f, dual_delta: f,
            dual_gamma: f, implied_volatility: f, iv_error: f,
            underlying_ms_of_day: i, underlying_price: f, date: i,
        ]),
        "TradeGreeksAllTicks" => rows!(d::parse_trade_greeks_all_ticks(&table)?, [
            ms_of_day: i, sequence: i, ext_condition1: i, ext_condition2: i,
            ext_condition3: i, ext_condition4: i, condition: i, size: i,
            exchange: i, price: f, delta: f, theta: f, vega: f, rho: f,
            epsilon: f, lambda: f, gamma: f, vanna: f, charm: f, vomma: f,
            veta: f, vera: f, speed: f, zomma: f, color: f, ultima: f,
            d1: f, d2: f, dual_delta: f, dual_gamma: f,
            implied_volatility: f, iv_error: f, underlying_ms_of_day: i,
            underlying_price: f, date: i,
        ]),
        "TradeGreeksFirstOrderTicks" => rows!(d::parse_trade_greeks_first_order_ticks(&table)?, [
            ms_of_day: i, sequence: i, ext_condition1: i, ext_condition2: i,
            ext_condition3: i, ext_condition4: i, condition: i, size: i,
            exchange: i, price: f, delta: f, theta: f, vega: f, rho: f,
            epsilon: f, lambda: f, implied_volatility: f, iv_error: f,
            underlying_ms_of_day: i, underlying_price: f, date: i,
        ]),
        "TradeGreeksSecondOrderTicks" => rows!(d::parse_trade_greeks_second_order_ticks(&table)?, [
            ms_of_day: i, sequence: i, ext_condition1: i, ext_condition2: i,
            ext_condition3: i, ext_condition4: i, condition: i, size: i,
            exchange: i, price: f, gamma: f, vanna: f, charm: f, vomma: f,
            veta: f, implied_volatility: f, iv_error: f,
            underlying_ms_of_day: i, underlying_price: f, date: i,
        ]),
        "TradeGreeksThirdOrderTicks" => rows!(d::parse_trade_greeks_third_order_ticks(&table)?, [
            ms_of_day: i, sequence: i, ext_condition1: i, ext_condition2: i,
            ext_condition3: i, ext_condition4: i, condition: i, size: i,
            exchange: i, price: f, speed: f, zomma: f, color: f, ultima: f,
            implied_volatility: f, iv_error: f, underlying_ms_of_day: i,
            underlying_price: f, date: i,
        ]),
        "TradeGreeksImpliedVolatilityTicks" => {
            rows!(d::parse_trade_greeks_implied_volatility_ticks(&table)?, [
                ms_of_day: i, sequence: i, ext_condition1: i, ext_condition2: i,
                ext_condition3: i, ext_condition4: i, condition: i, size: i,
                exchange: i, price: f, implied_volatility: f, iv_error: f,
                underlying_ms_of_day: i, underlying_price: f, date: i,
            ])
        }
        "IndexPriceAtTimeTicks" => rows!(d::parse_index_price_at_time_ticks(&table)?, [
            ms_of_day: i, sequence: i, ext_condition1: i, ext_condition2: i,
            ext_condition3: i, ext_condition4: i, condition: i, size: i,
            exchange: i, price: f, date: i,
        ]),
        "CalendarDays" => rows!(d::parse_calendar_days_v3(&table)?, [
            date: i, is_open: i, open_time: i, close_time: i, status: i,
        ]),
        other => panic!(
            "capture fixture exists for endpoint {} but decode_capture has no \
             dispatch arm for collection {other}",
            endpoint.name
        ),
    };
    Ok(Some(sample))
}
