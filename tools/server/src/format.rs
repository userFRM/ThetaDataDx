//! Response formatting that matches the Java terminal's JSON output exactly.
//!
//! Uses `sonic_rs` (SIMD-accelerated) instead of `serde_json` for all
//! serialization. The Java terminal wraps every REST response in:
//!
//! ```json
//! {
//!     "header": { "format": "json", "error_type": "null" },
//!     "response": [ ... ]
//! }
//! ```

use sonic_rs::prelude::*;
use tdbe::types::tick::*;
use thetadatadx::endpoint::EndpointOutput;

// ---------------------------------------------------------------------------
//  JSON envelope
// ---------------------------------------------------------------------------

/// Wrap a response array in the Java terminal's standard envelope.
pub fn ok_envelope(response: Vec<sonic_rs::Value>) -> sonic_rs::Value {
    sonic_rs::json!({
        "header": {
            "format": "json",
            "error_type": "null"
        },
        "response": response
    })
}

/// Error envelope matching the Java terminal's error format.
pub fn error_envelope(error_type: &str, message: &str) -> sonic_rs::Value {
    sonic_rs::json!({
        "header": {
            "format": "json",
            "error_type": error_type
        },
        "error": {
            "message": message
        }
    })
}

/// Wrap a list of strings in the envelope (for list endpoints).
pub fn list_envelope(items: &[String]) -> sonic_rs::Value {
    let response: Vec<sonic_rs::Value> = items
        .iter()
        .map(|s| sonic_rs::Value::from(s.as_str()))
        .collect();
    ok_envelope(response)
}

/// Convert a shared endpoint output into the Java-terminal JSON envelope.
pub fn output_envelope(output: &EndpointOutput) -> sonic_rs::Value {
    let response = match output {
        EndpointOutput::StringList(items) => {
            return list_envelope(items);
        }
        EndpointOutput::EodTicks(ticks) => eod_ticks_to_json(ticks),
        EndpointOutput::OhlcTicks(ticks) => ohlc_ticks_to_json(ticks),
        EndpointOutput::TradeTicks(ticks) => trade_ticks_to_json(ticks),
        EndpointOutput::QuoteTicks(ticks) => quote_ticks_to_json(ticks),
        EndpointOutput::TradeQuoteTicks(ticks) => trade_quote_ticks_to_json(ticks),
        EndpointOutput::OpenInterestTicks(ticks) => open_interest_ticks_to_json(ticks),
        EndpointOutput::MarketValueTicks(ticks) => market_value_ticks_to_json(ticks),
        EndpointOutput::GreeksTicks(ticks) => greeks_ticks_to_json(ticks),
        EndpointOutput::IvTicks(ticks) => iv_ticks_to_json(ticks),
        EndpointOutput::PriceTicks(ticks) => price_ticks_to_json(ticks),
        EndpointOutput::CalendarDays(days) => calendar_days_to_json(days),
        EndpointOutput::InterestRateTicks(ticks) => interest_rate_ticks_to_json(ticks),
        EndpointOutput::OptionContracts(contracts) => option_contracts_to_json(contracts),
    };
    ok_envelope(response)
}

// ---------------------------------------------------------------------------
//  Contract identification helpers
// ---------------------------------------------------------------------------

fn right_label(right: i32) -> sonic_rs::Value {
    match right {
        67 => sonic_rs::Value::from("C"),
        80 => sonic_rs::Value::from("P"),
        _ => sonic_rs::Value::from(right),
    }
}

fn insert_contract_id_fields(row: &mut sonic_rs::Value, expiration: i32, strike: f64, right: i32) {
    if expiration == 0 {
        return;
    }
    let object = row
        .as_object_mut()
        .expect("serialized tick rows must always be JSON objects");
    object.insert(
        "expiration",
        sonic_rs::to_value(&expiration).expect("i32 should serialize"),
    );
    object.insert(
        "strike",
        sonic_rs::to_value(&strike).expect("f64 should serialize"),
    );
    object.insert("right", right_label(right));
}

// ---------------------------------------------------------------------------
//  Tick -> sonic_rs::Value conversions
// ---------------------------------------------------------------------------

/// Convert EOD ticks to JSON array matching the Java terminal format.
pub fn eod_ticks_to_json(ticks: &[EodTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            let mut row = sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "ms_of_day2": t.ms_of_day2,
                "open": t.open,
                "high": t.high,
                "low": t.low,
                "close": t.close,
                "volume": t.volume,
                "count": t.count,
                "bid_size": t.bid_size,
                "bid_exchange": t.bid_exchange,
                "bid": t.bid,
                "bid_condition": t.bid_condition,
                "ask_size": t.ask_size,
                "ask_exchange": t.ask_exchange,
                "ask": t.ask,
                "ask_condition": t.ask_condition,
                "date": t.date
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert OHLC ticks to JSON array.
pub fn ohlc_ticks_to_json(ticks: &[OhlcTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            let mut row = sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "open": t.open,
                "high": t.high,
                "low": t.low,
                "close": t.close,
                "volume": t.volume,
                "count": t.count,
                "date": t.date
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert trade ticks to JSON array.
pub fn trade_ticks_to_json(ticks: &[TradeTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            let mut row = sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "sequence": t.sequence,
                "size": t.size,
                "condition": t.condition,
                "price": t.price,
                "exchange": t.exchange,
                "date": t.date
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert quote ticks to JSON array.
pub fn quote_ticks_to_json(ticks: &[QuoteTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            let mut row = sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "bid_size": t.bid_size,
                "bid_exchange": t.bid_exchange,
                "bid": t.bid,
                "bid_condition": t.bid_condition,
                "ask_size": t.ask_size,
                "ask_exchange": t.ask_exchange,
                "ask": t.ask,
                "ask_condition": t.ask_condition,
                "midpoint": t.midpoint,
                "date": t.date
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert trade+quote ticks to JSON array.
pub fn trade_quote_ticks_to_json(ticks: &[TradeQuoteTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            let mut row = sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "sequence": t.sequence,
                "size": t.size,
                "condition": t.condition,
                "price": t.price,
                "exchange": t.exchange,
                "ext_condition1": t.ext_condition1,
                "ext_condition2": t.ext_condition2,
                "ext_condition3": t.ext_condition3,
                "ext_condition4": t.ext_condition4,
                "condition_flags": t.condition_flags,
                "price_flags": t.price_flags,
                "volume_type": t.volume_type,
                "records_back": t.records_back,
                "quote_ms_of_day": t.quote_ms_of_day,
                "bid_size": t.bid_size,
                "bid_exchange": t.bid_exchange,
                "bid": t.bid,
                "bid_condition": t.bid_condition,
                "ask_size": t.ask_size,
                "ask_exchange": t.ask_exchange,
                "ask": t.ask,
                "ask_condition": t.ask_condition,
                "date": t.date
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert open interest ticks to JSON array.
pub fn open_interest_ticks_to_json(ticks: &[OpenInterestTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            let mut row = sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "open_interest": t.open_interest,
                "date": t.date
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert market value ticks to JSON array.
pub fn market_value_ticks_to_json(ticks: &[MarketValueTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            let mut row = sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "market_bid": t.market_bid,
                "market_ask": t.market_ask,
                "market_price": t.market_price,
                "date": t.date
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert greeks ticks to JSON array.
pub fn greeks_ticks_to_json(ticks: &[GreeksTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            let mut row = sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "implied_volatility": t.implied_volatility,
                "delta": t.delta,
                "gamma": t.gamma,
                "theta": t.theta,
                "vega": t.vega,
                "rho": t.rho,
                "iv_error": t.iv_error,
                "vanna": t.vanna,
                "charm": t.charm,
                "vomma": t.vomma,
                "veta": t.veta,
                "speed": t.speed,
                "zomma": t.zomma,
                "color": t.color,
                "ultima": t.ultima,
                "d1": t.d1,
                "d2": t.d2,
                "dual_delta": t.dual_delta,
                "dual_gamma": t.dual_gamma,
                "epsilon": t.epsilon,
                "lambda": t.lambda,
                "vera": t.vera,
                "date": t.date
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert IV ticks to JSON array.
pub fn iv_ticks_to_json(ticks: &[IvTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            let mut row = sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "implied_volatility": t.implied_volatility,
                "iv_error": t.iv_error,
                "date": t.date
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert price ticks to JSON array.
pub fn price_ticks_to_json(ticks: &[PriceTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "price": t.price,
                "date": t.date
            })
        })
        .collect()
}

/// Convert calendar days to JSON array.
pub fn calendar_days_to_json(days: &[CalendarDay]) -> Vec<sonic_rs::Value> {
    days.iter()
        .map(|d| {
            sonic_rs::json!({
                "date": d.date,
                "is_open": d.is_open,
                "open_time": d.open_time,
                "close_time": d.close_time,
                "status": d.status
            })
        })
        .collect()
}

/// Convert interest rate ticks to JSON array.
pub fn interest_rate_ticks_to_json(ticks: &[InterestRateTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "rate": t.rate,
                "date": t.date
            })
        })
        .collect()
}

/// Convert option contracts to JSON array.
pub fn option_contracts_to_json(contracts: &[OptionContract]) -> Vec<sonic_rs::Value> {
    contracts
        .iter()
        .map(|c| {
            sonic_rs::json!({
                "root": c.root,
                "expiration": c.expiration,
                "strike": c.strike,
                "right": c.right,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
//  CSV formatting
// ---------------------------------------------------------------------------

/// Convert a JSON response array to CSV with headers.
///
/// Returns `None` if the response is empty or contains unsupported row shapes.
///
/// Object rows are emitted with one column per key using the first row's key
/// order. Scalar rows are emitted as a single-column CSV with the `value`
/// header so list endpoints can round-trip through `format=csv`.
pub fn json_to_csv(response: &[sonic_rs::Value]) -> Option<String> {
    let first = response.first()?;
    let mut out = String::with_capacity(response.len() * 128);

    if let Some(obj) = first.as_object() {
        let null_val = sonic_rs::Value::default();
        let mut keys: Vec<&str> = obj.iter().map(|(k, _)| k).collect();
        if keys.is_empty() {
            return None;
        }
        keys.sort_unstable();

        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&escape_csv_field(key));
        }
        out.push('\n');

        for row in response {
            let row_obj = row.as_object()?;
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let value = row_obj.get(key).unwrap_or(&null_val);
                out.push_str(&render_csv_value(value));
            }
            out.push('\n');
        }

        return Some(out);
    }

    if response.iter().any(|row| row.is_object() || row.is_array()) {
        return None;
    }

    out.push_str("value\n");
    for row in response {
        out.push_str(&render_csv_value(row));
        out.push('\n');
    }

    Some(out)
}

fn render_csv_value(value: &sonic_rs::Value) -> String {
    if let Some(s) = value.as_str() {
        return escape_csv_field(s);
    }
    if value.is_null() {
        return String::new();
    }
    let rendered = sonic_rs::to_string(value).unwrap_or_default();
    escape_csv_field(&rendered)
}

fn escape_csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdbe::types::tick::{GreeksTick, QuoteTick, TradeQuoteTick};

    #[test]
    fn json_to_csv_formats_scalar_lists_as_single_column() {
        let csv = json_to_csv(&[
            sonic_rs::Value::from("AAPL"),
            sonic_rs::Value::from("MS,FT"),
            sonic_rs::Value::from("He said \"hi\""),
        ])
        .expect("scalar list should format as CSV");

        assert_eq!(csv, "value\nAAPL\n\"MS,FT\"\n\"He said \"\"hi\"\"\"\n");
    }

    #[test]
    fn json_to_csv_formats_object_rows_with_headers() {
        let csv = json_to_csv(&[
            sonic_rs::json!({ "symbol": "AAPL", "count": 1 }),
            sonic_rs::json!({ "symbol": "MSFT", "count": 2 }),
        ])
        .expect("object rows should format as CSV");

        assert_eq!(csv, "count,symbol\n1,AAPL\n2,MSFT\n");
    }

    #[test]
    fn json_to_csv_rejects_mixed_row_shapes() {
        let csv = json_to_csv(&[
            sonic_rs::json!({ "symbol": "AAPL" }),
            sonic_rs::Value::from("MSFT"),
        ]);

        assert!(csv.is_none(), "mixed row shapes should not format as CSV");
    }

    #[test]
    fn quote_ticks_includes_midpoint() {
        let t = QuoteTick {
            ms_of_day: 0,
            bid_size: 1,
            bid_exchange: 2,
            bid: 3.0,
            bid_condition: 4,
            ask_size: 5,
            ask_exchange: 6,
            ask: 7.0,
            ask_condition: 8,
            date: 20260410,
            expiration: 20260417,
            strike: 150.0,
            right: 67,
            midpoint: 5.0,
        };
        let r = quote_ticks_to_json(&[t]);
        let r = r.first().unwrap();
        assert!(r.get("midpoint").is_some());
        assert_eq!(
            r.get("expiration")
                .and_then(|v: &sonic_rs::Value| v.as_i64()),
            Some(20260417)
        );
    }
    #[test]
    fn trade_quote_ticks_has_extended_fields() {
        let t = TradeQuoteTick {
            ms_of_day: 0,
            sequence: 1,
            ext_condition1: 10,
            ext_condition2: 20,
            ext_condition3: 30,
            ext_condition4: 40,
            condition: 1,
            size: 100,
            exchange: 11,
            price: 150.0,
            condition_flags: 3,
            price_flags: 7,
            volume_type: 1,
            records_back: 5,
            quote_ms_of_day: 0,
            bid_size: 100,
            bid_exchange: 11,
            bid: 149.0,
            bid_condition: 1,
            ask_size: 200,
            ask_exchange: 12,
            ask: 151.0,
            ask_condition: 2,
            date: 20260410,
            expiration: 0,
            strike: 0.0,
            right: 0,
        };
        let r = trade_quote_ticks_to_json(&[t]);
        let r = r.first().unwrap();
        for k in [
            "ext_condition1",
            "ext_condition2",
            "ext_condition3",
            "ext_condition4",
            "condition_flags",
            "price_flags",
            "volume_type",
            "records_back",
        ] {
            assert!(r.get(k).is_some(), "missing: {k}");
        }
    }
    #[test]
    fn greeks_ticks_has_all_greeks() {
        let t = GreeksTick {
            ms_of_day: 0,
            implied_volatility: 0.25,
            delta: 0.5,
            gamma: 0.1,
            theta: -0.01,
            vega: 0.2,
            rho: 0.05,
            iv_error: 0.0,
            vanna: 0.0,
            charm: 0.0,
            vomma: 0.0,
            veta: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
            ultima: 0.0,
            d1: 0.0,
            d2: 0.0,
            dual_delta: 0.0,
            dual_gamma: 0.0,
            epsilon: 0.0,
            lambda: 0.0,
            vera: 0.0,
            date: 20260410,
            expiration: 20260417,
            strike: 150.0,
            right: 67,
        };
        let r = greeks_ticks_to_json(&[t]);
        let r = r.first().unwrap();
        for k in [
            "implied_volatility",
            "delta",
            "gamma",
            "theta",
            "vega",
            "rho",
            "iv_error",
            "vanna",
            "charm",
            "vomma",
            "veta",
            "speed",
            "zomma",
            "color",
            "ultima",
            "d1",
            "d2",
            "dual_delta",
            "dual_gamma",
            "epsilon",
            "lambda",
            "vera",
        ] {
            assert!(r.get(k).is_some(), "missing: {k}");
        }
        assert_eq!(
            r.get("expiration")
                .and_then(|v: &sonic_rs::Value| v.as_i64()),
            Some(20260417)
        );
    }
    #[test]
    fn greeks_ticks_omits_ids_single_contract() {
        let t = GreeksTick {
            ms_of_day: 0,
            implied_volatility: 0.0,
            delta: 0.0,
            gamma: 0.0,
            theta: 0.0,
            vega: 0.0,
            rho: 0.0,
            iv_error: 0.0,
            vanna: 0.0,
            charm: 0.0,
            vomma: 0.0,
            veta: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
            ultima: 0.0,
            d1: 0.0,
            d2: 0.0,
            dual_delta: 0.0,
            dual_gamma: 0.0,
            epsilon: 0.0,
            lambda: 0.0,
            vera: 0.0,
            date: 20260410,
            expiration: 0,
            strike: 0.0,
            right: 0,
        };
        let r = greeks_ticks_to_json(&[t]);
        let r = r.first().unwrap();
        assert!(r.get("expiration").is_none());
        assert!(r.get("strike").is_none());
        assert!(r.get("right").is_none());
    }
}
