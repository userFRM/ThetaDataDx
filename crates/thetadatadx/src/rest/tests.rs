//! REST transport unit tests.
//!
//! The decoders are pure functions over a CSV body, so a real
//! HTTP server is not required for the contract under test. The
//! integration test that drives the live patched Terminal lives in
//! `tests/test_rest_live.rs` behind `#[ignore]` + the
//! `THETADX_LIVE_PATCHED_TERMINAL` env gate.

use super::client::{
    decode_greeks_first_order_csv, decode_iv_csv, decode_quote_csv, decode_trade_quote_csv,
};

/// The patched Terminal serves legacy 2022-era NBBO rows in the
/// 6-field CSV layout. `decode_quote_csv` must accept it and
/// zero-fill the absent exchange / condition columns.
#[test]
fn decode_quote_csv_handles_legacy_six_field_layout() {
    let body = "\
ms_of_day,bid_size,bid,ask_size,ask,date
34200000,50,1.5022,75,1.5041,20220414
34200500,55,1.5023,80,1.5042,20220414
";
    let ticks = decode_quote_csv(body).unwrap();
    assert_eq!(ticks.len(), 2);

    let t0 = &ticks[0];
    assert_eq!(t0.ms_of_day, 34_200_000);
    assert_eq!(t0.bid_size, 50);
    assert!((t0.bid - 1.5022).abs() < 1e-9);
    assert_eq!(t0.ask_size, 75);
    assert!((t0.ask - 1.5041).abs() < 1e-9);
    assert_eq!(t0.date, 20_220_414);
    // Wire-absent columns zero-fill.
    assert_eq!(t0.bid_exchange, 0);
    assert_eq!(t0.bid_condition, 0);
    assert_eq!(t0.ask_exchange, 0);
    assert_eq!(t0.ask_condition, 0);
    // Midpoint computed from bid + ask.
    assert!((t0.midpoint - 1.50315).abs() < 1e-9);
}

/// Current 11-field layout still decodes every column bit-exact.
#[test]
fn decode_quote_csv_handles_current_eleven_field_layout() {
    let body = "\
ms_of_day,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date
34200000,50,7,1.5022,1,75,8,1.5041,2,20240605
";
    let ticks = decode_quote_csv(body).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];
    assert_eq!(t.bid_exchange, 7);
    assert_eq!(t.bid_condition, 1);
    assert_eq!(t.ask_exchange, 8);
    assert_eq!(t.ask_condition, 2);
    assert_eq!(t.date, 20_240_605);
}

/// Empty body (header only, no rows) decodes to empty Vec -- mirrors
/// the gRPC path's "no data today" behaviour.
#[test]
fn decode_quote_csv_empty_response_yields_empty_vec() {
    let body = "ms_of_day,bid_size,bid,ask_size,ask,date\n";
    let ticks = decode_quote_csv(body).unwrap();
    assert!(ticks.is_empty());
}

/// A response missing `ms_of_day` on a non-empty body surfaces the
/// structured MissingColumn error.
#[test]
fn decode_quote_csv_missing_required_column_errors() {
    let body = "bid,ask,date\n1.50,1.51,20220414\n";
    let err = decode_quote_csv(body).unwrap_err();
    assert!(
        matches!(
            err,
            super::error::RestError::MissingColumn {
                column: "ms_of_day",
                ..
            }
        ),
        "expected MissingColumn(ms_of_day), got {err}"
    );
}

/// trade_quote CSV in the current shape (subset of the 25-field
/// schema) decodes its quote-side columns. The trade-side columns
/// the CSV typically omits default to 0 -- this is the patched
/// Terminal's contract on a forward-compat row.
#[test]
fn decode_trade_quote_csv_basic() {
    let body = "\
ms_of_day,price,size,exchange,bid_size,bid,ask_size,ask,date
34200000,1.5030,10,7,50,1.5022,75,1.5041,20240605
";
    let ticks = decode_trade_quote_csv(body).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];
    assert_eq!(t.ms_of_day, 34_200_000);
    assert!((t.price - 1.5030).abs() < 1e-9);
    assert_eq!(t.size, 10);
    assert_eq!(t.exchange, 7);
    assert!((t.bid - 1.5022).abs() < 1e-9);
    assert!((t.ask - 1.5041).abs() < 1e-9);
    assert_eq!(t.date, 20_240_605);
    // Trade-side columns the CSV omits zero-fill.
    assert_eq!(t.sequence, 0);
    assert_eq!(t.condition, 0);
}

/// IV CSV decodes the IV column. `IvTick` carries only the IV /
/// iv_error pair plus time + contract identity -- the underlying
/// snapshot and bid/ask are on the richer `GreeksFirstOrderTick`.
#[test]
fn decode_iv_csv_basic() {
    let body = "\
ms_of_day,implied_volatility,iv_error,date
34200000,0.2142,-0.0003,20240605
";
    let ticks = decode_iv_csv(body).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];
    assert!((t.implied_volatility - 0.2142).abs() < 1e-9);
    assert!((t.iv_error - -0.0003).abs() < 1e-9);
    assert_eq!(t.date, 20_240_605);
}

/// First-order Greeks CSV decodes the seven first-order Greeks plus
/// IV pair plus underlying.
#[test]
fn decode_greeks_first_order_csv_basic() {
    let body = "\
ms_of_day,bid,ask,delta,theta,vega,rho,epsilon,lambda,implied_volatility,iv_error,underlying_ms_of_day,underlying_price,date
34200000,1.5022,1.5041,0.5023,-0.0114,0.8741,1.3598,-0.1976,3.2052,0.2142,-0.0003,34200001,58.0025,20240605
";
    let ticks = decode_greeks_first_order_csv(body).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];
    assert!((t.delta - 0.5023).abs() < 1e-9);
    assert!((t.theta - -0.0114).abs() < 1e-9);
    assert!((t.vega - 0.8741).abs() < 1e-9);
    assert!((t.rho - 1.3598).abs() < 1e-9);
    assert!((t.epsilon - -0.1976).abs() < 1e-9);
    assert!((t.lambda - 3.2052).abs() < 1e-9);
    assert!((t.implied_volatility - 0.2142).abs() < 1e-9);
    assert!((t.underlying_price - 58.0025).abs() < 1e-9);
}

/// Trailing newlines / windows CRLF must not break the parser.
#[test]
fn decode_quote_csv_tolerates_crlf_and_trailing_blank_lines() {
    let body = "ms_of_day,bid,ask,date\r\n34200000,1.5022,1.5041,20220414\r\n\r\n";
    let ticks = decode_quote_csv(body).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];
    assert_eq!(t.ms_of_day, 34_200_000);
    assert_eq!(t.date, 20_220_414);
}
