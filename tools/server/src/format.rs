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
/// Returns `None` if the response is empty. Each object's keys become CSV
/// column headers (order taken from the first row).
pub fn json_to_csv(response: &[sonic_rs::Value]) -> Option<String> {
    let first = response.first()?;
    let obj = first.as_object()?;
    let null_val = sonic_rs::Value::default();
    let keys: Vec<&str> = obj.iter().map(|(k, _)| k).collect();
    if keys.is_empty() {
        return None;
    }

    let mut out = String::with_capacity(response.len() * 128);
    // Header row
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(k);
    }
    out.push('\n');

    // Data rows
    for row in response {
        if let Some(row_obj) = row.as_object() {
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let val = row_obj.get(k).unwrap_or(&null_val);
                if val.is_str() {
                    if let Some(s) = val.as_str() {
                        out.push_str(s);
                    }
                } else if val.is_null() {
                    // empty cell
                } else {
                    let rendered = sonic_rs::to_string(val).unwrap_or_default();
                    out.push_str(&rendered);
                }
            }
            out.push('\n');
        }
    }

    Some(out)
}
