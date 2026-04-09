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
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
//  Tick -> sonic_rs::Value conversions
// ---------------------------------------------------------------------------

/// Convert EOD ticks to JSON array matching the Java terminal format.
pub fn eod_ticks_to_json(ticks: &[EodTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
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
            })
        })
        .collect()
}

/// Convert OHLC ticks to JSON array.
pub fn ohlc_ticks_to_json(ticks: &[OhlcTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "open": t.open,
                "high": t.high,
                "low": t.low,
                "close": t.close,
                "volume": t.volume,
                "count": t.count,
                "date": t.date
            })
        })
        .collect()
}

/// Convert trade ticks to JSON array.
pub fn trade_ticks_to_json(ticks: &[TradeTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "sequence": t.sequence,
                "size": t.size,
                "condition": t.condition,
                "price": t.price,
                "exchange": t.exchange,
                "date": t.date
            })
        })
        .collect()
}

/// Convert quote ticks to JSON array.
pub fn quote_ticks_to_json(ticks: &[QuoteTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "bid_size": t.bid_size,
                "bid_exchange": t.bid_exchange,
                "bid": t.bid,
                "bid_condition": t.bid_condition,
                "ask_size": t.ask_size,
                "ask_exchange": t.ask_exchange,
                "ask": t.ask,
                "ask_condition": t.ask_condition,
                "date": t.date
            })
        })
        .collect()
}

/// Convert trade+quote ticks to JSON array.
pub fn trade_quote_ticks_to_json(ticks: &[TradeQuoteTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "sequence": t.sequence,
                "size": t.size,
                "condition": t.condition,
                "price": t.price,
                "exchange": t.exchange,
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
            })
        })
        .collect()
}

/// Convert open interest ticks to JSON array.
pub fn open_interest_ticks_to_json(ticks: &[OpenInterestTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "open_interest": t.open_interest,
                "date": t.date
            })
        })
        .collect()
}

/// Convert market value ticks to JSON array.
pub fn market_value_ticks_to_json(ticks: &[MarketValueTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "market_cap": t.market_cap,
                "shares_outstanding": t.shares_outstanding,
                "enterprise_value": t.enterprise_value,
                "book_value": t.book_value,
                "free_float": t.free_float,
                "date": t.date
            })
        })
        .collect()
}

/// Convert greeks ticks to JSON array.
pub fn greeks_ticks_to_json(ticks: &[GreeksTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
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
            })
        })
        .collect()
}

/// Convert IV ticks to JSON array.
pub fn iv_ticks_to_json(ticks: &[IvTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            sonic_rs::json!({
                "ms_of_day": t.ms_of_day,
                "implied_volatility": t.implied_volatility,
                "iv_error": t.iv_error,
                "date": t.date
            })
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
        let keys: Vec<&str> = obj.iter().map(|(k, _)| k).collect();
        if keys.is_empty() {
            return None;
        }

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
    use super::json_to_csv;

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

        assert_eq!(csv, "symbol,count\nAAPL,1\nMSFT,2\n");
    }

    #[test]
    fn json_to_csv_rejects_mixed_row_shapes() {
        let csv = json_to_csv(&[
            sonic_rs::json!({ "symbol": "AAPL" }),
            sonic_rs::Value::from("MSFT"),
        ]);

        assert!(csv.is_none(), "mixed row shapes should not format as CSV");
    }
}
