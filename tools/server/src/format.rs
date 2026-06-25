//! Response formatting that matches the JVM terminal's JSON output exactly.
//!
//! Uses `sonic_rs` (SIMD-accelerated) instead of `serde_json` for all
//! serialization. The JVM terminal wraps every REST response in:
//!
//! ```json
//! {
//!     "header": { "format": "json", "error_type": "null" },
//!     "response": [ ... ]
//! }
//! ```

use sonic_rs::prelude::*;
use thetadatadx::endpoint::EndpointOutput;
use thetadatadx::*;

// ---------------------------------------------------------------------------
//  JSON envelope
// ---------------------------------------------------------------------------

/// Wrap a response array in the JVM terminal's standard envelope.
pub fn ok_envelope(response: Vec<sonic_rs::Value>) -> sonic_rs::Value {
    // v3 contract: the success body is `{"response": [...]}` with no
    // `header` key (the v3 spec carries no header on any path).
    sonic_rs::json!({ "response": response })
}

/// Error envelope matching the JVM terminal's error format.
///
/// Canonical shape across every route family (registry endpoints, flat
/// files, rate-limit rejections):
///
/// ```json
/// {
///     "header": { "error_type": "<type>", "error_msg": "<detail>" },
///     "response": []
/// }
/// ```
///
/// Clients parse `header.error_type` to drive retry / backoff logic, so
/// the keys must be identical regardless of which layer produced the
/// failure. The serialise-failure fallbacks in `handler::error_response`
/// and `flatfile_routes::error_response` hand-write the same shape.
pub fn error_envelope(error_type: &str, message: &str) -> sonic_rs::Value {
    sonic_rs::json!({
        "header": {
            "error_type": error_type,
            "error_msg": message
        },
        "response": []
    })
}

/// Wrap a list of string values in the envelope (for list endpoints).
///
/// v3 list endpoints return an array of single-key objects rather than
/// bare scalars: `stock_list_symbols` emits `[{"symbol":"AAPL"}, ...]`,
/// `stock_list_dates` emits `[{"date":"2016-08-16"}, ...]`, and so on.
/// `key` names the per-row field for the endpoint family in play.
///
/// The keyless [`EndpointOutput::StringList`] variant that reaches
/// [`output_envelope`] carries only the raw `Vec<String>` with no
/// per-endpoint key or ISO formatting, so the symbol-paired
/// (`option_list_expirations`/`option_list_strikes`) and ISO-date
/// (`list_dates`) shapes require the endpoint name to be threaded from
/// the handler; that wiring lives outside this module.
pub fn list_envelope(items: &[String], key: &str) -> sonic_rs::Value {
    let response: Vec<sonic_rs::Value> = items
        .iter()
        .map(|s| {
            let mut row = sonic_rs::json!({});
            row.as_object_mut()
                .expect("freshly built JSON object")
                .insert(key, sonic_rs::Value::from(s.as_str()));
            row
        })
        .collect();
    ok_envelope(response)
}

/// Convert a shared endpoint output into the JVM terminal JSON envelope.
pub fn output_envelope(output: &EndpointOutput) -> sonic_rs::Value {
    let response = match output {
        EndpointOutput::StringList(items) => {
            // The generic handler does not thread the endpoint name into
            // this module, so the keyless `StringList` arm cannot tell a
            // symbol list from a date / expiration / strike list. Default
            // to the canonical `symbol` key the bulk of the list endpoints
            // use; per-endpoint keys + ISO formatting need the endpoint
            // name to be plumbed through from the caller.
            return list_envelope(items, "symbol");
        }
        EndpointOutput::EodTicks(ticks) => eod_ticks_to_json(ticks),
        EndpointOutput::OhlcTicks(ticks) => ohlc_ticks_to_json(ticks),
        EndpointOutput::TradeTicks(ticks) => trade_ticks_to_json(ticks),
        EndpointOutput::QuoteTicks(ticks) => quote_ticks_to_json(ticks),
        EndpointOutput::TradeQuoteTicks(ticks) => trade_quote_ticks_to_json(ticks),
        EndpointOutput::OpenInterestTicks(ticks) => open_interest_ticks_to_json(ticks),
        EndpointOutput::MarketValueTicks(ticks) => market_value_ticks_to_json(ticks),
        EndpointOutput::GreeksAllTicks(ticks) => greeks_all_ticks_to_json(ticks),
        EndpointOutput::GreeksEodTicks(ticks) => greeks_eod_ticks_to_json(ticks),
        EndpointOutput::GreeksFirstOrderTicks(ticks) => greeks_first_order_ticks_to_json(ticks),
        EndpointOutput::GreeksSecondOrderTicks(ticks) => greeks_second_order_ticks_to_json(ticks),
        EndpointOutput::GreeksThirdOrderTicks(ticks) => greeks_third_order_ticks_to_json(ticks),
        EndpointOutput::TradeGreeksAllTicks(ticks) => trade_greeks_all_ticks_to_json(ticks),
        EndpointOutput::TradeGreeksFirstOrderTicks(ticks) => {
            trade_greeks_first_order_ticks_to_json(ticks)
        }
        EndpointOutput::TradeGreeksSecondOrderTicks(ticks) => {
            trade_greeks_second_order_ticks_to_json(ticks)
        }
        EndpointOutput::TradeGreeksThirdOrderTicks(ticks) => {
            trade_greeks_third_order_ticks_to_json(ticks)
        }
        EndpointOutput::TradeGreeksImpliedVolatilityTicks(ticks) => {
            trade_greeks_implied_volatility_ticks_to_json(ticks)
        }
        EndpointOutput::IvTicks(ticks) => iv_ticks_to_json(ticks),
        EndpointOutput::PriceTicks(ticks) => price_ticks_to_json(ticks),
        EndpointOutput::IndexPriceAtTimeTicks(ticks) => index_price_at_time_ticks_to_json(ticks),
        EndpointOutput::CalendarDays(days) => calendar_days_to_json(days),
        EndpointOutput::InterestRateTicks(ticks) => interest_rate_ticks_to_json(ticks),
        EndpointOutput::OptionContracts(contracts) => option_contracts_to_json(contracts),
    };
    ok_envelope(response)
}

// ---------------------------------------------------------------------------
//  Contract identification helpers
// ---------------------------------------------------------------------------

fn right_label(right: char) -> sonic_rs::Value {
    // v3 spells the option right out as `CALL` / `PUT` (v2 used `C`/`P`).
    match right {
        'C' => sonic_rs::Value::from("CALL"),
        'P' => sonic_rs::Value::from("PUT"),
        other => sonic_rs::Value::from(other.to_string().as_str()),
    }
}

/// Format a `YYYYMMDD` integer as the vendor's documented ISO
/// `YYYY-MM-DD` expiration shape (`20260618` -> `"2026-06-18"`).
fn expiration_label(expiration: i32) -> sonic_rs::Value {
    let year = expiration / 10_000;
    let month = (expiration / 100) % 100;
    let day = expiration % 100;
    sonic_rs::Value::from(format!("{year:04}-{month:02}-{day:02}").as_str())
}

/// Combine a `YYYYMMDD` date with a millisecond-of-day offset into the v3
/// ISO local-datetime shape (`20240102`, `62273606` ->
/// `"2024-01-02T17:17:53.606"`). v3 folds the separate v2 `date` +
/// `ms_of_day` columns into one ISO timestamp string.
fn ms_of_day_to_iso(date: i32, ms_of_day: i32) -> sonic_rs::Value {
    let year = date / 10_000;
    let month = (date / 100) % 100;
    let day = date % 100;
    let ms = ms_of_day.max(0);
    let hour = ms / 3_600_000;
    let minute = (ms / 60_000) % 60;
    let second = (ms / 1_000) % 60;
    let millis = ms % 1_000;
    sonic_rs::Value::from(
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}")
            .as_str(),
    )
}

/// Format a `YYYYMMDD` integer as the v3 ISO `YYYY-MM-DD` date string.
/// Shares the calendar `date` and interest-rate `created` columns, which
/// the spec renders as bare dates (no time component).
fn date_label(date: i32) -> sonic_rs::Value {
    let year = date / 10_000;
    let month = (date / 100) % 100;
    let day = date % 100;
    sonic_rs::Value::from(format!("{year:04}-{month:02}-{day:02}").as_str())
}

/// Format a millisecond-of-day offset as the v3 `HH:mm:ss` clock string
/// (the calendar `open` / `close` columns). Milliseconds are truncated:
/// the calendar publishes whole-second session boundaries.
fn ms_of_day_to_clock(ms_of_day: i32) -> sonic_rs::Value {
    let ms = ms_of_day.max(0);
    let hour = ms / 3_600_000;
    let minute = (ms / 60_000) % 60;
    let second = (ms / 1_000) % 60;
    sonic_rs::Value::from(format!("{hour:02}:{minute:02}:{second:02}").as_str())
}

fn insert_contract_id_fields(row: &mut sonic_rs::Value, expiration: i32, strike: f64, right: char) {
    if expiration == 0 {
        return;
    }
    let object = row
        .as_object_mut()
        .expect("serialized tick rows must always be JSON objects");
    object.insert("expiration", expiration_label(expiration));
    object.insert(
        "strike",
        sonic_rs::to_value(&strike).expect("f64 should serialize"),
    );
    object.insert("right", right_label(right));
}

// ---------------------------------------------------------------------------
//  Tick -> sonic_rs::Value conversions
// ---------------------------------------------------------------------------

/// Convert EOD ticks to JSON array matching the JVM terminal format.
pub fn eod_ticks_to_json(ticks: &[EodTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `created` / `last_trade` are ISO datetimes built from the
            // EOD `date` + ms-of-day offsets; the standalone `date` column is
            // dropped (folded into the ISO strings).
            let mut row = sonic_rs::json!({
                "created": ms_of_day_to_iso(t.date, t.created_ms_of_day),
                "last_trade": ms_of_day_to_iso(t.date, t.last_trade_ms_of_day),
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
                "ask_condition": t.ask_condition
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
            // v3: the bar `timestamp` (ISO local-datetime built from the
            // `date` + ms-of-day offset) replaces the v2 `ms_of_day` + `date`
            // column pair.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "open": t.open,
                "high": t.high,
                "low": t.low,
                "close": t.close,
                "volume": t.volume,
                "count": t.count,
                "vwap": t.vwap
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
            // v3: `timestamp` (ISO from `date` + ms-of-day) replaces the v2
            // `ms_of_day` + `date` pair; the v2-only `condition_flags`,
            // `price_flags`, `volume_type`, and `records_back` wire columns
            // are not part of the v3 trade shape.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "sequence": t.sequence,
                "ext_condition1": t.ext_condition1,
                "ext_condition2": t.ext_condition2,
                "ext_condition3": t.ext_condition3,
                "ext_condition4": t.ext_condition4,
                "condition": t.condition,
                "size": t.size,
                "exchange": t.exchange,
                "price": t.price
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
            // v3: `timestamp` (ISO from `date` + ms-of-day) replaces the v2
            // `ms_of_day` + `date` pair; the v2-only computed `midpoint`
            // column is not part of the v3 quote shape.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "bid_size": t.bid_size,
                "bid_exchange": t.bid_exchange,
                "bid": t.bid,
                "bid_condition": t.bid_condition,
                "ask_size": t.ask_size,
                "ask_exchange": t.ask_exchange,
                "ask": t.ask,
                "ask_condition": t.ask_condition
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
            // v3: the trade and quote sides each carry their own ISO
            // datetime -- `trade_timestamp` (from `date` + the trade
            // ms-of-day) and `quote_timestamp` (from `date` + the paired
            // quote ms-of-day) -- replacing the v2 `ms_of_day` /
            // `quote_ms_of_day` / `date` integer columns. The v2-only
            // `condition_flags`, `price_flags`, `volume_type`, and
            // `records_back` columns are not part of the v3 shape.
            let mut row = sonic_rs::json!({
                "trade_timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "quote_timestamp": ms_of_day_to_iso(t.date, t.quote_ms_of_day),
                "sequence": t.sequence,
                "ext_condition1": t.ext_condition1,
                "ext_condition2": t.ext_condition2,
                "ext_condition3": t.ext_condition3,
                "ext_condition4": t.ext_condition4,
                "condition": t.condition,
                "size": t.size,
                "exchange": t.exchange,
                "price": t.price,
                "bid_size": t.bid_size,
                "bid_exchange": t.bid_exchange,
                "bid": t.bid,
                "bid_condition": t.bid_condition,
                "ask_size": t.ask_size,
                "ask_exchange": t.ask_exchange,
                "ask": t.ask,
                "ask_condition": t.ask_condition
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
            // v3: `timestamp` (ISO from `date` + ms-of-day) replaces the v2
            // `ms_of_day` + `date` pair.
            let mut row = sonic_rs::json!({
                "open_interest": t.open_interest,
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day)
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
            // v3 market-value snapshots carry no time column: the v2
            // `ms_of_day` + `date` pair is dropped, leaving the three
            // market-quote fields plus the contract id.
            let mut row = sonic_rs::json!({
                "market_bid": t.market_bid,
                "market_ask": t.market_ask,
                "market_price": t.market_price
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert full-union Greeks ticks (`option_*_greeks_all`,
/// `option_*_greeks_eod`) to JSON array.
pub fn greeks_all_ticks_to_json(ticks: &[GreeksAllTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` / `underlying_timestamp` (ISO from `date` +
            // the respective ms-of-day) replace the v2 `ms_of_day` /
            // `underlying_ms_of_day` / `date` integer columns, and the
            // implied-vol field is named `implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "bid": t.bid,
                "ask": t.ask,
                "implied_vol": t.implied_volatility,
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
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert end-of-day Greeks ticks (`option_history_greeks_eod`) to
/// JSON array. The JSON shape preserves the full 39-column wire
/// surface -- every Greek, the twelve EOD trade/quote context columns
/// (`open` / `high` / `low` / `close` / `volume` / `count` / `bid_size`
/// / `bid_exchange` / `bid_condition` / `ask_size` / `ask_exchange` /
/// `ask_condition`), and the underlying snapshot + contract id triple
/// -- so downstream MCP-side / REST-side consumers see the full EOD
/// trade-quote context that the earlier routing dropped; the current schema restores the
/// full schema.
pub fn greeks_eod_ticks_to_json(ticks: &[GreeksEodTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` / `underlying_timestamp` (ISO from `date` +
            // the respective ms-of-day) replace the v2 `ms_of_day` /
            // `underlying_ms_of_day` / `date` integer columns, and the
            // implied-vol field is named `implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
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
                "delta": t.delta,
                "theta": t.theta,
                "vega": t.vega,
                "rho": t.rho,
                "epsilon": t.epsilon,
                "lambda": t.lambda,
                "gamma": t.gamma,
                "vanna": t.vanna,
                "charm": t.charm,
                "vomma": t.vomma,
                "veta": t.veta,
                "vera": t.vera,
                "speed": t.speed,
                "zomma": t.zomma,
                "color": t.color,
                "ultima": t.ultima,
                "d1": t.d1,
                "d2": t.d2,
                "dual_delta": t.dual_delta,
                "dual_gamma": t.dual_gamma,
                "implied_vol": t.implied_volatility,
                "iv_error": t.iv_error,
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert first-order Greeks subset ticks
/// (`option_*_greeks_first_order`) to JSON array.
pub fn greeks_first_order_ticks_to_json(ticks: &[GreeksFirstOrderTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` / `underlying_timestamp` (ISO) replace the v2
            // `ms_of_day` / `underlying_ms_of_day` / `date` columns; the
            // implied-vol field is named `implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "bid": t.bid,
                "ask": t.ask,
                "delta": t.delta,
                "theta": t.theta,
                "vega": t.vega,
                "rho": t.rho,
                "epsilon": t.epsilon,
                "lambda": t.lambda,
                "implied_vol": t.implied_volatility,
                "iv_error": t.iv_error,
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert second-order Greeks subset ticks
/// (`option_*_greeks_second_order`) to JSON array.
pub fn greeks_second_order_ticks_to_json(ticks: &[GreeksSecondOrderTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` / `underlying_timestamp` (ISO) replace the v2
            // `ms_of_day` / `underlying_ms_of_day` / `date` columns; the
            // implied-vol field is named `implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "bid": t.bid,
                "ask": t.ask,
                "gamma": t.gamma,
                "vanna": t.vanna,
                "charm": t.charm,
                "vomma": t.vomma,
                "veta": t.veta,
                "implied_vol": t.implied_volatility,
                "iv_error": t.iv_error,
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert third-order Greeks subset ticks
/// (`option_*_greeks_third_order`) to JSON array. The vendor's
/// third-order schema does not publish `vera`, hence its absence here.
pub fn greeks_third_order_ticks_to_json(ticks: &[GreeksThirdOrderTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` / `underlying_timestamp` (ISO) replace the v2
            // `ms_of_day` / `underlying_ms_of_day` / `date` columns; the
            // implied-vol field is named `implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "bid": t.bid,
                "ask": t.ask,
                "speed": t.speed,
                "zomma": t.zomma,
                "color": t.color,
                "ultima": t.ultima,
                "implied_vol": t.implied_volatility,
                "iv_error": t.iv_error,
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert per-OPRA-trade union Greeks ticks
/// (`option_history_trade_greeks_all`) to JSON array. Carries the nine
/// trade-side execution columns alongside every Greek the server
/// publishes -- distinct from the interval-sampled `GreeksAllTick`
/// JSON whose rows carry the bid/ask quote pair instead.
pub fn trade_greeks_all_ticks_to_json(ticks: &[TradeGreeksAllTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` / `underlying_timestamp` (ISO) replace the v2
            // `ms_of_day` / `underlying_ms_of_day` / `date` columns; the
            // implied-vol field is named `implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "sequence": t.sequence,
                "ext_condition1": t.ext_condition1,
                "ext_condition2": t.ext_condition2,
                "ext_condition3": t.ext_condition3,
                "ext_condition4": t.ext_condition4,
                "condition": t.condition,
                "size": t.size,
                "exchange": t.exchange,
                "price": t.price,
                "delta": t.delta,
                "theta": t.theta,
                "vega": t.vega,
                "rho": t.rho,
                "epsilon": t.epsilon,
                "lambda": t.lambda,
                "gamma": t.gamma,
                "vanna": t.vanna,
                "charm": t.charm,
                "vomma": t.vomma,
                "veta": t.veta,
                "vera": t.vera,
                "speed": t.speed,
                "zomma": t.zomma,
                "color": t.color,
                "ultima": t.ultima,
                "d1": t.d1,
                "d2": t.d2,
                "dual_delta": t.dual_delta,
                "dual_gamma": t.dual_gamma,
                "implied_vol": t.implied_volatility,
                "iv_error": t.iv_error,
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert per-OPRA-trade first-order Greeks ticks
/// (`option_history_trade_greeks_first_order`) to JSON array.
pub fn trade_greeks_first_order_ticks_to_json(
    ticks: &[TradeGreeksFirstOrderTick],
) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` / `underlying_timestamp` (ISO) replace the v2
            // `ms_of_day` / `underlying_ms_of_day` / `date` columns; the
            // implied-vol field is named `implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "sequence": t.sequence,
                "ext_condition1": t.ext_condition1,
                "ext_condition2": t.ext_condition2,
                "ext_condition3": t.ext_condition3,
                "ext_condition4": t.ext_condition4,
                "condition": t.condition,
                "size": t.size,
                "exchange": t.exchange,
                "price": t.price,
                "delta": t.delta,
                "theta": t.theta,
                "vega": t.vega,
                "rho": t.rho,
                "epsilon": t.epsilon,
                "lambda": t.lambda,
                "implied_vol": t.implied_volatility,
                "iv_error": t.iv_error,
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert per-OPRA-trade second-order Greeks ticks
/// (`option_history_trade_greeks_second_order`) to JSON array.
pub fn trade_greeks_second_order_ticks_to_json(
    ticks: &[TradeGreeksSecondOrderTick],
) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` / `underlying_timestamp` (ISO) replace the v2
            // `ms_of_day` / `underlying_ms_of_day` / `date` columns; the
            // implied-vol field is named `implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "sequence": t.sequence,
                "ext_condition1": t.ext_condition1,
                "ext_condition2": t.ext_condition2,
                "ext_condition3": t.ext_condition3,
                "ext_condition4": t.ext_condition4,
                "condition": t.condition,
                "size": t.size,
                "exchange": t.exchange,
                "price": t.price,
                "gamma": t.gamma,
                "vanna": t.vanna,
                "charm": t.charm,
                "vomma": t.vomma,
                "veta": t.veta,
                "implied_vol": t.implied_volatility,
                "iv_error": t.iv_error,
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert per-OPRA-trade third-order Greeks ticks
/// (`option_history_trade_greeks_third_order`) to JSON array. The
/// vendor's third-order schema does not publish `vera`.
pub fn trade_greeks_third_order_ticks_to_json(
    ticks: &[TradeGreeksThirdOrderTick],
) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` / `underlying_timestamp` (ISO) replace the v2
            // `ms_of_day` / `underlying_ms_of_day` / `date` columns; the
            // implied-vol field is named `implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "sequence": t.sequence,
                "ext_condition1": t.ext_condition1,
                "ext_condition2": t.ext_condition2,
                "ext_condition3": t.ext_condition3,
                "ext_condition4": t.ext_condition4,
                "condition": t.condition,
                "size": t.size,
                "exchange": t.exchange,
                "price": t.price,
                "speed": t.speed,
                "zomma": t.zomma,
                "color": t.color,
                "ultima": t.ultima,
                "implied_vol": t.implied_volatility,
                "iv_error": t.iv_error,
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
            });
            insert_contract_id_fields(&mut row, t.expiration, t.strike, t.right);
            row
        })
        .collect()
}

/// Convert per-OPRA-trade implied-volatility ticks
/// (`option_history_trade_greeks_implied_volatility`) to JSON array.
/// Carries only the single `implied_volatility` + `iv_error` pair
/// (NOT the bid/mid/ask IV triple of the interval-sampled `IvTick`).
pub fn trade_greeks_implied_volatility_ticks_to_json(
    ticks: &[TradeGreeksImpliedVolatilityTick],
) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` / `underlying_timestamp` (ISO) replace the v2
            // `ms_of_day` / `underlying_ms_of_day` / `date` columns; the
            // implied-vol field is named `implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "sequence": t.sequence,
                "ext_condition1": t.ext_condition1,
                "ext_condition2": t.ext_condition2,
                "ext_condition3": t.ext_condition3,
                "ext_condition4": t.ext_condition4,
                "condition": t.condition,
                "size": t.size,
                "exchange": t.exchange,
                "price": t.price,
                "implied_vol": t.implied_volatility,
                "iv_error": t.iv_error,
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
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
            // v3: `timestamp` / `underlying_timestamp` (ISO) replace the v2
            // `ms_of_day` / `underlying_ms_of_day` / `date` columns; the
            // implied-vol fields are named `implied_vol` / `bid_implied_vol`
            // / `ask_implied_vol`.
            let mut row = sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "bid": t.bid,
                "bid_implied_vol": t.bid_implied_volatility,
                "midpoint": t.midpoint,
                "implied_vol": t.implied_volatility,
                "ask": t.ask,
                "ask_implied_vol": t.ask_implied_volatility,
                "iv_error": t.iv_error,
                "underlying_timestamp": ms_of_day_to_iso(t.date, t.underlying_ms_of_day),
                "underlying_price": t.underlying_price
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
            // v3: `timestamp` (ISO from `date` + ms-of-day) replaces the v2
            // `ms_of_day` + `date` pair.
            sonic_rs::json!({
                "price": t.price,
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day)
            })
        })
        .collect()
}

/// Convert trade-shaped index ticks (`index_at_time_price`) to JSON
/// array. The JSON shape preserves the full 10-column wire surface --
/// the seven trade-side execution columns (`sequence`,
/// `ext_condition1..4`, `condition`, `size`, `exchange`) plus
/// `ms_of_day`, `price`, and `date` -- so downstream MCP-side /
/// REST-side consumers see the per-row SIP-exchange attribution that
/// the earlier routing dropped; the current schema restores the full schema.
pub fn index_price_at_time_ticks_to_json(ticks: &[IndexPriceAtTimeTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3: `timestamp` (ISO from `date` + ms-of-day) replaces the v2
            // `ms_of_day` + `date` pair.
            sonic_rs::json!({
                "timestamp": ms_of_day_to_iso(t.date, t.ms_of_day),
                "sequence": t.sequence,
                "ext_condition1": t.ext_condition1,
                "ext_condition2": t.ext_condition2,
                "ext_condition3": t.ext_condition3,
                "ext_condition4": t.ext_condition4,
                "condition": t.condition,
                "size": t.size,
                "exchange": t.exchange,
                "price": t.price
            })
        })
        .collect()
}

/// Convert calendar days to JSON array.
///
/// v3 shape: `{date, type, open, close}`. `date` is the ISO `YYYY-MM-DD`
/// string and is omitted on the single-day `calendar_on_date` /
/// `calendar_open_today` responses (where the server sends no date column
/// and `CalendarDay.date` is `0`). `type` carries the vendor day
/// classification (`open` / `early_close` / `full_close` / `weekend`).
/// `open` / `close` are `HH:mm:ss` clock strings on trading days and
/// `null` on fully-closed days, so a consumer can branch on a present
/// time vs an explicit null rather than a sentinel midnight.
pub fn calendar_days_to_json(days: &[CalendarDay]) -> Vec<sonic_rs::Value> {
    days.iter()
        .map(|d| {
            let (open, close) = if d.status.is_open() {
                (
                    ms_of_day_to_clock(d.open_time),
                    ms_of_day_to_clock(d.close_time),
                )
            } else {
                (sonic_rs::Value::new_null(), sonic_rs::Value::new_null())
            };
            // Build the row by hand so `date` leads the object (matching the
            // multi-day spec example) yet drops out entirely on the
            // single-day responses where the server omits the column.
            let mut row = sonic_rs::json!({});
            let object = row.as_object_mut().expect("freshly built JSON object");
            if d.date != 0 {
                object.insert("date", date_label(d.date));
            }
            object.insert("type", sonic_rs::Value::from(d.status.as_str()));
            object.insert("open", open);
            object.insert("close", close);
            row
        })
        .collect()
}

/// Convert interest rate ticks to JSON array.
pub fn interest_rate_ticks_to_json(ticks: &[InterestRateTick]) -> Vec<sonic_rs::Value> {
    ticks
        .iter()
        .map(|t| {
            // v3 names the EOD interest-rate date column `created` and
            // renders it as the ISO `YYYY-MM-DD` string.
            sonic_rs::json!({
                "rate": t.rate,
                "created": date_label(t.date)
            })
        })
        .collect()
}

/// Convert option contracts to JSON array.
pub fn option_contracts_to_json(contracts: &[OptionContract]) -> Vec<sonic_rs::Value> {
    contracts
        .iter()
        .map(|c| {
            // v3 `option_list_contracts` row order: symbol, strike,
            // expiration (ISO `YYYY-MM-DD`), right (`CALL` / `PUT`).
            sonic_rs::json!({
                "symbol": c.symbol,
                "strike": c.strike,
                "expiration": expiration_label(c.expiration),
                "right": right_label(c.right),
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
/// Object rows are emitted with one column per key. Headers are the union
/// of keys across every row in lexicographic (sorted) order, so sparse
/// rows (e.g. index ticks without `expiration` / `strike` / `right`
/// mixed with option ticks that have them) never silently drop columns.
/// Scalar rows are emitted as a single-column CSV with the `value`
/// header so list endpoints can round-trip through `format=csv`.
pub fn json_to_csv(response: &[sonic_rs::Value]) -> Option<String> {
    let first = response.first()?;
    let mut out = String::with_capacity(response.len() * 128);

    if first.as_object().is_some() {
        // Union the key set across EVERY row — not just the first row. Object
        // rows can be sparse (index ticks have no `expiration/strike/right`
        // while option ticks do), and seeding the header from row 0 alone
        // silently dropped every column row 0 did not carry. `BTreeSet`
        // gives stable lexicographic header order for free, replacing the
        // previous explicit `sort_unstable` on the row-0 key list.
        let mut keys_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for row in response {
            let row_obj = row.as_object()?;
            for (k, _) in row_obj.iter() {
                // Pre-check membership so we only allocate a fresh
                // `String` for keys we are actually going to insert.
                // `BTreeSet<String>::insert` takes the key by value and
                // would otherwise heap-allocate for every duplicate.
                if !keys_set.contains(k) {
                    keys_set.insert(k.to_string());
                }
            }
        }
        if keys_set.is_empty() {
            return None;
        }
        let keys: Vec<String> = keys_set.into_iter().collect();
        let null_val = sonic_rs::Value::default();

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
    // Canonicalise into an owned tree before serialising. The non-finite f64
    // collapse already happened upstream in the JSON envelope, but a CSV
    // cell that was constructed independently (e.g. from a hand-built
    // `sonic_rs::Value`) might still carry a non-finite leaf — collapse it
    // here so the encoder cannot fail. If serialisation still errors, emit
    // an explicit sentinel string so the CSV column is observable rather
    // than silently empty.
    let mut owned = value.clone();
    thetadatadx::json_canon::canonicalize(&mut owned);
    match sonic_rs::to_string(&owned) {
        Ok(rendered) => escape_csv_field(&rendered),
        Err(err) => {
            tracing::warn!(error = %err, "csv cell serialisation failed; emitting sentinel");
            escape_csv_field(&format!("<csv-render-error: {err}>"))
        }
    }
}

/// CSV-escape a single field.
///
/// Handles two categories:
///
/// 1. **RFC 4180 special characters** (`,`, `"`, `\n`, `\r`) are escaped by
///    wrapping the whole field in double quotes and doubling any inner quote.
/// 2. **Formula-injection prefixes** (`=`, `+`, `-`, `@`, `\t`) cause Excel /
///    LibreOffice Calc / Google Sheets to evaluate the cell as a formula when
///    the CSV is opened. An attacker who can place a string of their choosing
///    into a symbol, condition, or any other CSV-rendered field could exfil
///    data or trigger `cmd|'/C calc'` style payloads on the viewer's machine.
///    We defuse by prepending a single quote (`'`) *inside* the quoted field,
///    which is the OWASP-recommended mitigation: spreadsheet apps display the
///    cell verbatim while refusing to evaluate it as a formula.
///
/// The leading single-quote forces the field into the "needs quoting" branch
/// unconditionally, so a risky field is always wrapped in `"`.
fn escape_csv_field(value: &str) -> String {
    let needs_formula_prefix = value
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t'));
    let has_special = value.contains([',', '"', '\n', '\r']);

    if !needs_formula_prefix && !has_special {
        return value.to_owned();
    }

    let escaped = value.replace('"', "\"\"");
    let prefix = if needs_formula_prefix { "'" } else { "" };
    format!("\"{prefix}{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonic_rs::JsonContainerTrait;
    use thetadatadx::{GreeksAllTick, QuoteTick, TradeQuoteTick};

    /// The error envelope must carry `header.error_type` + `header.error_msg`
    /// with an empty `response` array — the same shape the JVM terminal
    /// emits and the flat-file / handler fallback strings hand-write. The
    /// nested `error.message` form must never come back: clients parse one
    /// shape across every route family.
    #[test]
    fn error_envelope_uses_canonical_error_msg_shape() {
        let envelope = error_envelope("bad_request", "missing required parameter: 'date'");

        let header = envelope
            .get("header")
            .and_then(|h: &sonic_rs::Value| h.as_object())
            .expect("envelope must carry a header object");
        assert_eq!(
            header.get(&"error_type").and_then(sonic_rs::Value::as_str),
            Some("bad_request")
        );
        assert_eq!(
            header.get(&"error_msg").and_then(sonic_rs::Value::as_str),
            Some("missing required parameter: 'date'")
        );

        let response = envelope
            .get("response")
            .and_then(|r: &sonic_rs::Value| r.as_array())
            .expect("envelope must carry a response array");
        assert!(response.is_empty(), "error envelope response must be []");

        assert!(
            envelope.get("error").is_none(),
            "nested error.message form must not be emitted"
        );
    }

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

    /// Regression: CSV formula-injection defense.
    ///
    /// Any cell that starts with `=`, `+`, `-`, `@`, or `\t` is interpreted
    /// as a formula by Excel / LibreOffice Calc / Google Sheets. An attacker
    /// who can place a crafted string into a symbol, condition, or any
    /// other field rendered to CSV could trigger `cmd|'/C calc'!A1` style
    /// payloads on the viewer's machine. The fix prepends `'` *inside* the
    /// quoted field, which spreadsheet apps render verbatim without
    /// evaluating. Every payload below must round-trip as `"'<original>"`.
    #[test]
    fn json_to_csv_defuses_formula_injection() {
        let csv = json_to_csv(&[
            sonic_rs::json!({ "cell": "=cmd|'/C calc'!A1" }),
            sonic_rs::json!({ "cell": "+1+cmd|'/C calc'!A1" }),
            sonic_rs::json!({ "cell": "-2+cmd|'/C calc'!A1" }),
            sonic_rs::json!({ "cell": "@SUM(A1:A10)" }),
            sonic_rs::json!({ "cell": "\tnull-byte-start" }),
        ])
        .expect("formula payloads should still format as CSV");

        // Header row is trivially safe ("cell" starts with 'c').
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "cell");

        // Each dangerous payload must be quoted AND prefixed with a single
        // quote so the spreadsheet sees a literal string, not a formula.
        // Inner double-quotes in the payload are RFC-4180 doubled to `""`.
        assert_eq!(lines[1], "\"'=cmd|'/C calc'!A1\"");
        assert_eq!(lines[2], "\"'+1+cmd|'/C calc'!A1\"");
        assert_eq!(lines[3], "\"'-2+cmd|'/C calc'!A1\"");
        assert_eq!(lines[4], "\"'@SUM(A1:A10)\"");
        assert_eq!(lines[5], "\"'\tnull-byte-start\"");

        // Sanity: a benign string must NOT be quoted or prefixed -- the fix
        // must be surgical, not a blanket "quote everything".
        let benign = json_to_csv(&[sonic_rs::json!({ "cell": "AAPL" })]).unwrap();
        assert_eq!(benign, "cell\nAAPL\n");
    }

    /// Regression: the header key set must be the UNION of keys across
    /// every row, not just row 0. If row 0 is sparse (e.g. an index tick
    /// with no `expiration/strike/right`) and row 1 has extra columns,
    /// seeding from row 0 alone silently drops the missing columns from
    /// every subsequent row.
    #[test]
    fn json_to_csv_unions_keys_across_sparse_rows() {
        let csv = json_to_csv(&[
            // Row 0: index tick, no option-identifying fields.
            sonic_rs::json!({ "ms_of_day": 0, "price": 100.0 }),
            // Row 1: option tick, adds `expiration`, `strike`, `right`.
            sonic_rs::json!({
                "ms_of_day": 1,
                "price": 101.0,
                "expiration": 20240315,
                "strike": 150.0,
                "right": "C",
            }),
        ])
        .expect("sparse-object rows should format as CSV");

        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(
            lines[0], "expiration,ms_of_day,price,right,strike",
            "header should contain every key that appears in any row"
        );
        // Row 0 is missing `expiration`, `right`, `strike` — must render
        // as empty columns, not drop the whole column from the schema.
        assert_eq!(lines[1], ",0,100.0,,");
        assert_eq!(lines[2], "20240315,1,101.0,C,150.0");
    }

    /// v3 quote shape: the `date` + `ms_of_day` integer pair collapses
    /// into one ISO `timestamp`, and the v2-only computed `midpoint`
    /// column is gone. Contract id fields are emitted as ISO expiration +
    /// `CALL` / `PUT`.
    #[test]
    fn quote_ticks_emit_v3_timestamp_without_midpoint() {
        let t = QuoteTick {
            ms_of_day: 62_273_606,
            bid_size: 1,
            bid_exchange: 2,
            bid: 3.0,
            bid_condition: 4,
            ask_size: 5,
            ask_exchange: 6,
            ask: 7.0,
            ask_condition: 8,
            date: 20240102,
            expiration: 20260417,
            strike: 150.0,
            right: 'C',
            midpoint: 5.0,
        };
        let r = quote_ticks_to_json(&[t]);
        let r = r.first().unwrap();
        assert_eq!(
            r.get("timestamp")
                .and_then(|v: &sonic_rs::Value| v.as_str().map(str::to_string)),
            Some("2024-01-02T17:17:53.606".to_string())
        );
        assert!(r.get("midpoint").is_none(), "v3 quote drops midpoint");
        assert!(r.get("ms_of_day").is_none(), "v3 folds ms_of_day into timestamp");
        assert!(r.get("date").is_none(), "v3 folds date into timestamp");
        assert_eq!(
            r.get("expiration")
                .and_then(|v: &sonic_rs::Value| v.as_str().map(str::to_string)),
            Some("2026-04-17".to_string())
        );
        assert_eq!(
            r.get("right")
                .and_then(|v: &sonic_rs::Value| v.as_str().map(str::to_string)),
            Some("CALL".to_string())
        );
    }
    /// v3 trade_quote shape: the trade and quote sides each get their own
    /// ISO datetime (`trade_timestamp` / `quote_timestamp`) and the v2-only
    /// `condition_flags` / `price_flags` / `volume_type` / `records_back` /
    /// `date` columns are gone. The four `ext_condition` columns stay.
    #[test]
    fn trade_quote_ticks_emit_split_v3_timestamps() {
        let t = TradeQuoteTick {
            ms_of_day: 34_200_002,
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
            quote_ms_of_day: 34_200_001,
            bid_size: 100,
            bid_exchange: 11,
            bid: 149.0,
            bid_condition: 1,
            ask_size: 200,
            ask_exchange: 12,
            ask: 151.0,
            ask_condition: 2,
            date: 20230103,
            expiration: 0,
            strike: 0.0,
            right: '\0',
        };
        let r = trade_quote_ticks_to_json(&[t]);
        let r = r.first().unwrap();
        assert_eq!(
            r.get("trade_timestamp")
                .and_then(|v: &sonic_rs::Value| v.as_str().map(str::to_string)),
            Some("2023-01-03T09:30:00.002".to_string())
        );
        assert_eq!(
            r.get("quote_timestamp")
                .and_then(|v: &sonic_rs::Value| v.as_str().map(str::to_string)),
            Some("2023-01-03T09:30:00.001".to_string())
        );
        for k in [
            "ext_condition1",
            "ext_condition2",
            "ext_condition3",
            "ext_condition4",
        ] {
            assert!(r.get(k).is_some(), "missing: {k}");
        }
        for k in [
            "ms_of_day",
            "quote_ms_of_day",
            "date",
            "condition_flags",
            "price_flags",
            "volume_type",
            "records_back",
        ] {
            assert!(r.get(k).is_none(), "v3 trade_quote must drop: {k}");
        }
    }
    #[test]
    fn greeks_ticks_has_all_greeks() {
        let t = GreeksAllTick {
            ms_of_day: 0,
            bid: 0.0,
            ask: 0.0,
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
            underlying_ms_of_day: 0,
            underlying_price: 0.0,
            date: 20260410,
            expiration: 20260417,
            strike: 150.0,
            right: 'C',
        };
        let r = greeks_all_ticks_to_json(&[t]);
        let r = r.first().unwrap();
        for k in [
            "implied_vol",
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
            "bid",
            "ask",
            "underlying_timestamp",
            "underlying_price",
            "timestamp",
        ] {
            assert!(r.get(k).is_some(), "missing: {k}");
        }
        // v3 renames + folds: the integer time columns and the long
        // `implied_volatility` spelling must not survive.
        for k in ["implied_volatility", "ms_of_day", "underlying_ms_of_day", "date"] {
            assert!(r.get(k).is_none(), "v3 greeks must drop: {k}");
        }
        assert_eq!(
            r.get("expiration")
                .and_then(|v: &sonic_rs::Value| v.as_str().map(str::to_string)),
            Some("2026-04-17".to_string())
        );
    }
    #[test]
    fn greeks_ticks_omits_ids_single_contract() {
        let t = GreeksAllTick {
            ms_of_day: 0,
            bid: 0.0,
            ask: 0.0,
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
            underlying_ms_of_day: 0,
            underlying_price: 0.0,
            date: 20260410,
            expiration: 0,
            strike: 0.0,
            right: '\0',
        };
        let r = greeks_all_ticks_to_json(&[t]);
        let r = r.first().unwrap();
        assert!(r.get("expiration").is_none());
        assert!(r.get("strike").is_none());
        assert!(r.get("right").is_none());
    }
}
