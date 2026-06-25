//! Response formatting for the v3 REST contract.
//!
//! Uses `sonic_rs` (SIMD-accelerated) instead of `serde_json` for all
//! serialization. The v3 success body is `{ "response": [ ... ] }` with
//! no `header` key. Option / contract endpoints group their rows under
//! the owning contract:
//!
//! ```json
//! {
//!     "response": [
//!         { "contract": { "symbol": "AAPL", "strike": 550.0,
//!                         "expiration": "2026-06-18", "right": "CALL" },
//!           "data": [ { "timestamp": "2024-01-02T17:17:53.606", ... } ] }
//!     ]
//! }
//! ```
//!
//! Stock and index endpoints stay flat. Timestamps are ISO strings and
//! the option `right` is spelled `CALL` / `PUT`.

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
//  v3 endpoint-aware response building
// ---------------------------------------------------------------------------
//
//  The handler knows the endpoint (`EndpointMeta`) and the request `symbol`
//  param; this module knows the per-row v3 shape. The two meet here: the
//  handler hands us both and we produce the flat v3 rows once, then the JSON
//  path groups option rows under their contract while CSV / NDJSON consume
//  the flat rows directly — mirroring the vendor terminal, whose CSV / NDJSON
//  writers emit every contract column inline and only the grouped-JSON writer
//  lifts the contract out of each row.

/// `true` when an endpoint's rows carry the option contract identity
/// (`symbol` + `expiration` + `strike` + `right`). v3 groups exactly these
/// rows under a `contract` object; every other family stays flat. Keyed on
/// the REST path segment `/option/` (per the v3 routing) — list endpoints
/// take the [`EndpointOutput::StringList`] / [`EndpointOutput::OptionContracts`]
/// paths and never reach the contract-grouped tick path.
fn endpoint_is_option(ep: &EndpointMeta) -> bool {
    ep.rest_path.contains("/option/")
}

/// `true` when an endpoint's rows carry a leading `symbol` column. Every
/// option endpoint does (alongside the rest of the contract identity), and
/// so do the stock / index *snapshot* endpoints (which the v3 spec renders
/// with a `symbol` column but no `expiration` / `strike` / `right`, so they
/// stay flat). Stock / index history / at-time rows carry no `symbol`.
fn endpoint_carries_symbol(ep: &EndpointMeta) -> bool {
    endpoint_is_option(ep) || ep.rest_path.contains("/snapshot/")
}

/// Pick the per-row key for a `StringList` list endpoint from the endpoint
/// name suffix. v3 list rows are single-key objects keyed by the listed
/// dimension (`symbol` / `date` / `expiration` / `strike`).
fn list_value_key(ep: &EndpointMeta) -> &'static str {
    if ep.name.ends_with("_list_symbols") {
        "symbol"
    } else if ep.name.ends_with("_list_dates") {
        "date"
    } else if ep.name.ends_with("_list_expirations") {
        "expiration"
    } else if ep.name.ends_with("_list_strikes") {
        "strike"
    } else {
        "symbol"
    }
}

/// Convert a raw `YYYYMMDD` list value (`"20160816"`) to the v3 ISO
/// `YYYY-MM-DD` string. Non-8-digit values pass through unchanged so a
/// surprise wire shape is observable rather than silently mangled.
fn iso_date_string(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.len() == 8 && bytes.iter().all(u8::is_ascii_digit) {
        format!("{}-{}-{}", &raw[0..4], &raw[4..6], &raw[6..8])
    } else {
        raw.to_string()
    }
}

/// Build the flat v3 rows for a list endpoint, applying the per-endpoint
/// key, ISO date formatting, numeric strike values, and the `symbol`
/// pairing the symbol-scoped lists (`expirations` / `strikes`) carry.
fn list_rows(ep: &EndpointMeta, symbol: Option<&str>, items: &[String]) -> Vec<sonic_rs::Value> {
    let key = list_value_key(ep);
    let is_date = key == "date" || key == "expiration";
    let is_strike = key == "strike";
    // The symbol-scoped lists (`option_list_expirations` / `_strikes`) pair
    // each value with the requested `symbol`; the bare symbol / date lists
    // do not.
    let pair_symbol =
        symbol.filter(|s| !s.is_empty()).filter(|_| key == "expiration" || key == "strike");

    items
        .iter()
        .map(|raw| {
            let mut row = sonic_rs::json!({});
            let object = row.as_object_mut().expect("freshly built JSON object");
            if let Some(sym) = pair_symbol {
                object.insert("symbol", sonic_rs::Value::from(sym));
            }
            let value = if is_date {
                sonic_rs::Value::from(iso_date_string(raw).as_str())
            } else if is_strike {
                // v3 renders strikes as JSON numbers; fall back to the raw
                // string only if the wire value is not finite-parseable.
                match raw.parse::<f64>() {
                    Ok(n) if n.is_finite() => {
                        sonic_rs::to_value(&n).unwrap_or_else(|_| sonic_rs::Value::from(raw.as_str()))
                    }
                    _ => sonic_rs::Value::from(raw.as_str()),
                }
            } else {
                sonic_rs::Value::from(raw.as_str())
            };
            object.insert(key, value);
            row
        })
        .collect()
}

/// The request's contract-identity params, used to label the v3 contract.
///
/// `symbol` always comes from the request (the wire ticks never carry it).
/// `expiration` / `strike` / `right` are the raw request param strings
/// (`"20241108"`, `"220.000"`, `"call"`); they are only used as a *fallback*
/// when a row does not already carry the field. Wildcard responses
/// (`expiration=*`) inject the contract columns per row, so the row value
/// wins there; a single-contract response carries no contract columns, so
/// the request params populate the v3 contract object the spec shows.
#[derive(Clone, Copy, Default)]
pub struct ContractParams<'a> {
    /// Request `symbol` param (the option / underlying root).
    pub symbol: Option<&'a str>,
    /// Request `expiration` param, raw `YYYYMMDD` (ignored when `*`).
    pub expiration: Option<&'a str>,
    /// Request `strike` param, raw dollars string (ignored when `*`).
    pub strike: Option<&'a str>,
    /// Request `right` param (`call` / `put` / `c` / `p`; ignored when `*`).
    pub right: Option<&'a str>,
}

impl<'a> ContractParams<'a> {
    /// A concrete (non-wildcard, non-empty) request param, else `None`.
    fn concrete(value: Option<&'a str>) -> Option<&'a str> {
        value.filter(|v| !v.is_empty() && *v != "*")
    }

    /// v3-formatted `expiration` fallback (`"20241108"` -> `"2026-11-08"`).
    fn expiration_value(&self) -> Option<sonic_rs::Value> {
        Self::concrete(self.expiration)
            .map(|raw| sonic_rs::Value::from(iso_date_string(raw).as_str()))
    }

    /// v3-formatted `strike` fallback (numeric where parseable).
    fn strike_value(&self) -> Option<sonic_rs::Value> {
        Self::concrete(self.strike).map(|raw| match raw.parse::<f64>() {
            Ok(n) if n.is_finite() => {
                sonic_rs::to_value(&n).unwrap_or_else(|_| sonic_rs::Value::from(raw))
            }
            _ => sonic_rs::Value::from(raw),
        })
    }

    /// v3-formatted `right` fallback (`call` -> `CALL`, `p` -> `PUT`).
    fn right_value(&self) -> Option<sonic_rs::Value> {
        Self::concrete(self.right).map(|raw| match raw.to_ascii_lowercase().as_str() {
            "c" | "call" => sonic_rs::Value::from("CALL"),
            "p" | "put" => sonic_rs::Value::from("PUT"),
            other => sonic_rs::Value::from(other),
        })
    }
}

/// Rebuild a flat tick row to carry the contract identity: `symbol` (from
/// the request) plus, for option rows, `expiration` / `strike` / `right`.
///
/// The serializers append `expiration` / `strike` / `right` via
/// [`insert_contract_id_fields`] only for wildcard responses (where the wire
/// injects them per row); a single-contract response omits them, so the
/// concrete request params (`contract`) populate them — matching the v3
/// contract object the spec renders for a single-contract query. The output
/// field iteration order is not significant (the v3 JSON `data` rows are
/// key-addressed and unordered, and the CSV column order is pinned
/// separately by [`csv_header_order`]); this only guarantees the identity
/// fields are *present* so grouping and the CSV identity columns resolve.
fn lead_with_contract(
    row: &sonic_rs::Value,
    symbol: &str,
    is_option: bool,
    contract: &ContractParams<'_>,
) -> sonic_rs::Value {
    let src = row
        .as_object()
        .expect("serialized tick rows must always be JSON objects");
    let mut out = sonic_rs::json!({});
    let dst = out.as_object_mut().expect("freshly built JSON object");
    dst.insert("symbol", sonic_rs::Value::from(symbol));
    if is_option {
        // Row value wins (wildcard responses carry it per row); the request
        // param is the single-contract fallback.
        let expiration = src.get(&"expiration").cloned().or_else(|| contract.expiration_value());
        let strike = src.get(&"strike").cloned().or_else(|| contract.strike_value());
        let right = src.get(&"right").cloned().or_else(|| contract.right_value());
        if let Some(v) = expiration {
            dst.insert("expiration", v);
        }
        if let Some(v) = strike {
            dst.insert("strike", v);
        }
        if let Some(v) = right {
            dst.insert("right", v);
        }
    }
    for (k, v) in src.iter() {
        if k == "expiration" || k == "strike" || k == "right" {
            continue;
        }
        dst.insert(k, v.clone());
    }
    out
}

/// Build the flat v3 response rows for an endpoint result.
///
/// Every row is emitted in the v3 wire shape with the contract identity (and
/// the request `symbol`) inline and leading where the endpoint carries it.
/// These rows feed the CSV and NDJSON renderers directly; the JSON renderer
/// groups option rows via [`json_envelope`].
pub fn response_rows(
    ep: &EndpointMeta,
    contract: &ContractParams<'_>,
    output: &EndpointOutput,
) -> Vec<sonic_rs::Value> {
    if let EndpointOutput::StringList(items) = output {
        return list_rows(ep, contract.symbol, items);
    }
    let rows = output_envelope(output);
    let response = rows
        .get("response")
        .and_then(|r: &sonic_rs::Value| r.as_array())
        .map(|arr| arr.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();

    // Inject the request `symbol` (and reorder the contract identity to the
    // front) only for endpoints whose v3 rows carry a `symbol` column. The
    // history / at-time stock + index families have none, so they pass
    // through untouched.
    match contract.symbol {
        Some(sym) if !sym.is_empty() && endpoint_carries_symbol(ep) => {
            let is_option = endpoint_is_option(ep);
            response
                .iter()
                .map(|row| lead_with_contract(row, sym, is_option, contract))
                .collect()
        }
        _ => response,
    }
}

/// Wrap flat v3 rows in the JSON envelope, grouping option rows under their
/// contract.
///
/// For an option endpoint the rows are grouped by `(expiration, strike,
/// right)` into `{"contract": {...}, "data": [...]}` blocks (the contract
/// fields removed from each data row); every other endpoint stays flat
/// (`{"response": [...]}`). Rows are already contract-leading (see
/// [`response_rows`]) so equal-contract rows are contiguous and grouping is a
/// single linear pass.
pub fn json_envelope(ep: &EndpointMeta, rows: Vec<sonic_rs::Value>) -> sonic_rs::Value {
    if !endpoint_is_option(ep) {
        return ok_envelope(rows);
    }
    ok_envelope(group_rows_by_contract(rows))
}

/// Contract-identity key for grouping: `(expiration, strike, right)` as the
/// rendered v3 strings / number. `symbol` is constant within a request, so
/// it is not part of the grouping key.
fn contract_key(row: &sonic_rs::Value) -> (String, String, String) {
    let field = |name: &str| -> String {
        row.get(name)
            .map(|v: &sonic_rs::Value| {
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| sonic_rs::to_string(v).unwrap_or_default())
            })
            .unwrap_or_default()
    };
    (field("expiration"), field("strike"), field("right"))
}

/// Group contract-leading rows into v3 `{contract, data}` blocks.
fn group_rows_by_contract(mut rows: Vec<sonic_rs::Value>) -> Vec<sonic_rs::Value> {
    // Wildcard responses already arrive grouped by contract, but a stable
    // sort by the contract key guarantees one block per contract even if a
    // future wire shape interleaves them — without it, an interleaved
    // contract would emit a duplicate `{contract, data}` block. Stable, so
    // each contract's rows keep their original (chronological) order.
    rows.sort_by_key(contract_key);

    let mut groups: Vec<sonic_rs::Value> = Vec::new();
    let mut current_key: Option<(String, String, String)> = None;
    let mut current_data: Vec<sonic_rs::Value> = Vec::new();
    let mut current_contract = sonic_rs::Value::new_null();

    for row in rows {
        let key = contract_key(&row);
        if current_key.as_ref() != Some(&key) {
            if current_key.is_some() {
                groups.push(sonic_rs::json!({
                    "contract": std::mem::replace(&mut current_contract, sonic_rs::Value::new_null()),
                    "data": std::mem::take(&mut current_data),
                }));
            }
            current_contract = contract_object(&row);
            current_key = Some(key);
        }
        current_data.push(strip_contract_fields(row));
    }
    if current_key.is_some() {
        groups.push(sonic_rs::json!({
            "contract": current_contract,
            "data": current_data,
        }));
    }
    groups
}

/// Build the v3 `contract` object from a contract-leading row, carrying the
/// four identity fields (`symbol`, `strike`, `right`, `expiration`).
///
/// The JSON field *order* within the object is not asserted: `sonic_rs`
/// objects do not preserve construction order on serialisation (their
/// storage is an indexed key array), and the vendor itself serialises this
/// object from an unordered map, so the wire order is not a stable contract.
/// Consumers read the contract by key. (The CSV column order, which *is*
/// positional, is pinned separately and deterministically by
/// [`csv_header_order`].)
fn contract_object(row: &sonic_rs::Value) -> sonic_rs::Value {
    let src = row.as_object().expect("contract row must be a JSON object");
    let mut out = sonic_rs::json!({});
    let dst = out.as_object_mut().expect("freshly built JSON object");
    for field in ["symbol", "strike", "right", "expiration"] {
        if let Some(v) = src.get(&field) {
            dst.insert(field, v.clone());
        }
    }
    out
}

/// Drop the contract-identity fields (`symbol`, `expiration`, `strike`,
/// `right`) from a row, leaving the per-tick data the v3 `data` array
/// carries.
fn strip_contract_fields(row: sonic_rs::Value) -> sonic_rs::Value {
    let src = row.as_object().expect("data row must be a JSON object");
    let mut out = sonic_rs::json!({});
    let dst = out.as_object_mut().expect("freshly built JSON object");
    for (k, v) in src.iter() {
        if matches!(k, "symbol" | "expiration" | "strike" | "right") {
            continue;
        }
        dst.insert(k, v.clone());
    }
    out
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

/// CRLF line terminator. v3 CSV is RFC-4180 framed: every record (header
/// and data rows) ends with `\r\n`, matching the vendor's documented
/// `text/csv` examples. A bare `\n` is the v2 framing and is not the v3
/// contract.
const CSV_CRLF: &str = "\r\n";

/// The v3 CSV leading column order. Contract identity first (matching the
/// vendor `text/csv` examples, where `symbol` / `expiration` / `strike` /
/// `right` lead an option row), then the time columns. Any column not in
/// this list is a data column and follows, ordered deterministically.
///
/// This fixed prefix is the source of the v3 column order rather than the
/// row's key iteration order: `sonic_rs::Value` objects built via
/// `insert` / `json!` do NOT preserve field insertion order (their storage
/// is an indexed key array whose iteration order is not the construction
/// order), so iterating a row would yield a non-deterministic, non-v3
/// column sequence. Pinning the semantically-significant leading columns
/// here keeps the header deterministic and v3-shaped.
const CSV_LEADING_COLUMNS: &[&str] = &[
    "symbol",
    "expiration",
    "strike",
    "right",
    "timestamp",
    "trade_timestamp",
    "quote_timestamp",
    "created",
    "last_trade",
    "date",
    "underlying_timestamp",
];

/// Order the union of a response's column keys into the v3 CSV header
/// sequence: the [`CSV_LEADING_COLUMNS`] that are present, in that fixed
/// order, followed by every remaining column sorted lexicographically for
/// determinism. Returns `None` only when no object row contributes a key.
fn csv_header_order(response: &[sonic_rs::Value]) -> Option<Vec<String>> {
    let mut present: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for row in response {
        let row_obj = row.as_object()?;
        for (k, _) in row_obj.iter() {
            present.insert(k);
        }
    }
    if present.is_empty() {
        return None;
    }
    let mut keys: Vec<String> = Vec::with_capacity(present.len());
    for lead in CSV_LEADING_COLUMNS {
        if present.remove(*lead) {
            keys.push((*lead).to_string());
        }
    }
    // Remaining (data) columns: lexicographic, which `BTreeSet` already
    // gives in iteration order.
    keys.extend(present.into_iter().map(str::to_string));
    Some(keys)
}

/// Convert a JSON response array to CSV with headers.
///
/// Returns `None` if the response is empty or contains unsupported row shapes.
///
/// Object rows are emitted with one column per key, the union across every
/// row so sparse rows (e.g. index ticks without `expiration` / `strike` /
/// `right` mixed with option ticks that have them) never silently drop
/// columns. The header order is the v3 semantic order (see
/// [`csv_header_order`]): contract identity and the time columns lead, then
/// the data columns. The earlier lexicographic (`BTreeSet`) order put every
/// column alphabetically and did not match the v3 `text/csv` examples.
/// Scalar rows are emitted as a single-column CSV with the `value` header so
/// list endpoints can round-trip through `format=csv`. Records are
/// CRLF-terminated per the v3 contract.
pub fn json_to_csv(response: &[sonic_rs::Value]) -> Option<String> {
    let first = response.first()?;
    let mut out = String::with_capacity(response.len() * 128);

    if first.as_object().is_some() {
        let keys = csv_header_order(response)?;
        let null_val = sonic_rs::Value::default();

        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&escape_csv_field(key));
        }
        out.push_str(CSV_CRLF);

        for row in response {
            let row_obj = row.as_object()?;
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let value = row_obj.get(key).unwrap_or(&null_val);
                out.push_str(&render_csv_value(value));
            }
            out.push_str(CSV_CRLF);
        }

        return Some(out);
    }

    if response.iter().any(|row| row.is_object() || row.is_array()) {
        return None;
    }

    out.push_str("value");
    out.push_str(CSV_CRLF);
    for row in response {
        out.push_str(&render_csv_value(row));
        out.push_str(CSV_CRLF);
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

        // v3 CSV is CRLF-framed.
        assert_eq!(csv, "value\r\nAAPL\r\n\"MS,FT\"\r\n\"He said \"\"hi\"\"\"\r\n");
    }

    #[test]
    fn json_to_csv_formats_object_rows_with_headers() {
        let csv = json_to_csv(&[
            sonic_rs::json!({ "symbol": "AAPL", "count": 1 }),
            sonic_rs::json!({ "symbol": "MSFT", "count": 2 }),
        ])
        .expect("object rows should format as CSV");

        // v3 CSV preserves the row's field (insertion) order and is
        // CRLF-framed — the column order matches the rows, not a sort.
        assert_eq!(csv, "symbol,count\r\nAAPL,1\r\nMSFT,2\r\n");
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
        // must be surgical, not a blanket "quote everything". (CRLF-framed.)
        let benign = json_to_csv(&[sonic_rs::json!({ "cell": "AAPL" })]).unwrap();
        assert_eq!(benign, "cell\r\nAAPL\r\n");
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
        // v3 order: the contract-identity leading columns first in their
        // fixed order (`expiration`, `strike`, `right`), then the data
        // columns lexicographically (`ms_of_day`, `price`). Every key that
        // appears in any row is present — sparse rows render empty cells.
        assert_eq!(
            lines[0], "expiration,strike,right,ms_of_day,price",
            "header should lead with contract identity then carry every data key"
        );
        // Row 0 lacks the contract identity — those leading columns render
        // empty, not dropped from the schema.
        assert_eq!(lines[1], ",,,0,100.0");
        assert_eq!(lines[2], "20240315,150.0,C,1,101.0");
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

    // -----------------------------------------------------------------------
    //  v3 endpoint-aware response building
    // -----------------------------------------------------------------------

    use thetadatadx::QuoteTick as TdQuoteTick;

    fn quote_tick(expiration: i32, strike: f64, right: char) -> TdQuoteTick {
        TdQuoteTick {
            ms_of_day: 34_200_000,
            bid_size: 1,
            bid_exchange: 2,
            bid: 3.0,
            bid_condition: 4,
            ask_size: 5,
            ask_exchange: 6,
            ask: 7.0,
            ask_condition: 8,
            date: 20240102,
            expiration,
            strike,
            right,
            midpoint: 5.0,
        }
    }

    /// A wildcard option snapshot response (two contracts) groups under one
    /// `{contract, data}` block per contract: the contract object carries the
    /// request `symbol` + the per-row identity in the v3 `symbol, strike,
    /// right, expiration` field order, and each data row drops the contract
    /// fields. Stock / index endpoints stay flat (covered separately).
    #[test]
    fn option_endpoint_groups_rows_by_contract() {
        let ep = thetadatadx::find("option_snapshot_quote").expect("endpoint exists");
        let contract = ContractParams {
            symbol: Some("AAPL"),
            expiration: Some("*"),
            strike: None,
            right: None,
        };
        let output = EndpointOutput::QuoteTicks(vec![
            quote_tick(20260116, 275.0, 'C'),
            quote_tick(20260116, 280.0, 'P'),
        ]);
        let rows = response_rows(ep, &contract, &output);
        let envelope = json_envelope(ep, rows);

        let response = envelope
            .get("response")
            .and_then(|v: &sonic_rs::Value| v.as_array())
            .expect("response array");
        assert_eq!(response.len(), 2, "one group per distinct contract");

        let first = &response[0];
        let contract_obj = first.get("contract").expect("contract object");
        // The contract object carries exactly the four identity keys (the
        // JSON field *order* within the object is not contractual — the
        // vendor serialises it from an unordered map, and clients read by
        // key — so only presence + values are asserted).
        let keys: std::collections::BTreeSet<String> = contract_obj
            .as_object()
            .expect("contract is an object")
            .iter()
            .map(|(k, _)| k.to_string())
            .collect();
        assert_eq!(
            keys,
            ["expiration", "right", "strike", "symbol"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        );
        assert_eq!(
            contract_obj.get("symbol").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("AAPL"),
            "contract symbol comes from the request param"
        );
        assert_eq!(
            contract_obj.get("right").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("CALL")
        );
        assert_eq!(
            contract_obj
                .get("expiration")
                .and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("2026-01-16")
        );

        let data = first
            .get("data")
            .and_then(|v: &sonic_rs::Value| v.as_array())
            .expect("data array");
        assert_eq!(data.len(), 1);
        let data_row = &data[0];
        for dropped in ["symbol", "expiration", "strike", "right"] {
            assert!(
                data_row.get(dropped).is_none(),
                "contract field must be lifted out of the data row: {dropped}"
            );
        }
        assert!(data_row.get("bid").is_some(), "data row keeps the quote fields");
    }

    /// A single-contract option query carries no contract columns on the
    /// wire (the tick's `expiration` is 0), so the v3 contract object is
    /// populated from the concrete request params.
    #[test]
    fn single_contract_option_labels_contract_from_request_params() {
        let ep = thetadatadx::find("option_history_quote").expect("endpoint exists");
        let contract = ContractParams {
            symbol: Some("AAPL"),
            expiration: Some("20241108"),
            strike: Some("220.000"),
            right: Some("call"),
        };
        // expiration == 0 -> the serializer omits the contract columns.
        let output = EndpointOutput::QuoteTicks(vec![quote_tick(0, 0.0, '\0')]);
        let rows = response_rows(ep, &contract, &output);
        let envelope = json_envelope(ep, rows);
        let response = envelope
            .get("response")
            .and_then(|v: &sonic_rs::Value| v.as_array())
            .expect("response array");
        assert_eq!(response.len(), 1);
        let c = response[0].get("contract").expect("contract object");
        assert_eq!(
            c.get("symbol").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("AAPL")
        );
        assert_eq!(
            c.get("expiration").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("2024-11-08"),
            "expiration falls back to the request param, ISO-formatted"
        );
        assert_eq!(
            c.get("strike").and_then(|v: &sonic_rs::Value| v.as_f64()),
            Some(220.0),
            "strike falls back to the request param, as a number"
        );
        assert_eq!(
            c.get("right").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("CALL"),
            "right falls back to the request param, spelled out"
        );
    }

    /// Stock / index endpoints never group: the JSON envelope stays a flat
    /// `{response: [...]}` array, with the `symbol` column inline on the
    /// snapshot families and absent on history.
    #[test]
    fn stock_snapshot_stays_flat_with_inline_symbol() {
        let ep = thetadatadx::find("stock_snapshot_quote").expect("endpoint exists");
        let contract = ContractParams {
            symbol: Some("AAPL"),
            ..ContractParams::default()
        };
        let output = EndpointOutput::QuoteTicks(vec![quote_tick(0, 0.0, '\0')]);
        let rows = response_rows(ep, &contract, &output);
        let envelope = json_envelope(ep, rows);
        let response = envelope
            .get("response")
            .and_then(|v: &sonic_rs::Value| v.as_array())
            .expect("response array");
        assert_eq!(response.len(), 1);
        let row = &response[0];
        assert!(
            row.get("contract").is_none(),
            "stock endpoints must not group under a contract"
        );
        assert_eq!(
            row.get("symbol").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("AAPL"),
            "snapshot rows carry the request symbol inline"
        );
        assert!(
            row.get("expiration").is_none() && row.get("strike").is_none(),
            "stock rows carry no option contract identity"
        );
    }

    /// `stock_history_*` rows carry no `symbol` column (the v3 spec renders
    /// them without one).
    #[test]
    fn stock_history_has_no_symbol_column() {
        let ep = thetadatadx::find("stock_history_quote").expect("endpoint exists");
        let contract = ContractParams {
            symbol: Some("AAPL"),
            ..ContractParams::default()
        };
        let output = EndpointOutput::QuoteTicks(vec![quote_tick(0, 0.0, '\0')]);
        let rows = response_rows(ep, &contract, &output);
        assert!(
            rows[0].get("symbol").is_none(),
            "stock history rows have no symbol column"
        );
    }

    /// v3 list endpoints: symbols -> `{symbol}`, dates -> `{date}` ISO,
    /// option expirations -> `{symbol, expiration}` ISO, option strikes ->
    /// `{symbol, strike}` numeric.
    #[test]
    fn list_endpoints_use_v3_keys_and_iso() {
        // Symbol list: single `symbol` key, value verbatim.
        let ep = thetadatadx::find("stock_list_symbols").expect("endpoint exists");
        let rows = response_rows(
            ep,
            &ContractParams::default(),
            &EndpointOutput::StringList(vec!["AAPL".into()]),
        );
        assert_eq!(
            rows[0].get("symbol").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("AAPL")
        );

        // Date list: `date` key, raw YYYYMMDD rendered ISO.
        let ep = thetadatadx::find("stock_list_dates").expect("endpoint exists");
        let rows = response_rows(
            ep,
            &ContractParams::default(),
            &EndpointOutput::StringList(vec!["20160816".into()]),
        );
        assert_eq!(
            rows[0].get("date").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("2016-08-16")
        );

        // Option expirations: symbol-paired, expiration ISO.
        let ep = thetadatadx::find("option_list_expirations").expect("endpoint exists");
        let rows = response_rows(
            ep,
            &ContractParams {
                symbol: Some("AAPL"),
                ..ContractParams::default()
            },
            &EndpointOutput::StringList(vec!["20120601".into()]),
        );
        assert_eq!(
            rows[0].get("symbol").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("AAPL")
        );
        assert_eq!(
            rows[0].get("expiration").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("2012-06-01")
        );

        // Option strikes: symbol-paired, strike numeric.
        let ep = thetadatadx::find("option_list_strikes").expect("endpoint exists");
        let rows = response_rows(
            ep,
            &ContractParams {
                symbol: Some("AAPL"),
                ..ContractParams::default()
            },
            &EndpointOutput::StringList(vec!["80.000".into()]),
        );
        assert_eq!(
            rows[0].get("symbol").and_then(|v: &sonic_rs::Value| v.as_str()),
            Some("AAPL")
        );
        assert_eq!(
            rows[0].get("strike").and_then(|v: &sonic_rs::Value| v.as_f64()),
            Some(80.0)
        );
    }

    /// v3 CSV: contract identity + time columns lead in fixed order, then
    /// data columns, CRLF-framed. Built from an option history row so the
    /// `symbol,expiration,strike,right,timestamp,...` prefix is exercised.
    #[test]
    fn csv_v3_column_order_leads_with_contract_then_crlf() {
        let ep = thetadatadx::find("option_history_trade").expect("endpoint exists");
        let tick = thetadatadx::TradeTick {
            ms_of_day: 34_200_471,
            sequence: 18902138,
            ext_condition1: 255,
            ext_condition2: 255,
            ext_condition3: 255,
            ext_condition4: 255,
            condition: 130,
            size: 2,
            exchange: 22,
            price: 3.90,
            condition_flags: 0,
            price_flags: 0,
            volume_type: 0,
            records_back: 0,
            date: 20241104,
            expiration: 20241108,
            strike: 220.0,
            right: 'C',
        };
        let contract = ContractParams {
            symbol: Some("AAPL"),
            expiration: Some("*"),
            strike: None,
            right: None,
        };
        let rows = response_rows(ep, &contract, &EndpointOutput::TradeTicks(vec![tick]));
        let csv = json_to_csv(&rows).expect("CSV");
        assert!(csv.ends_with("\r\n"), "v3 CSV is CRLF-framed: {csv:?}");
        let header = csv.split("\r\n").next().expect("header line");
        assert!(
            header.starts_with("symbol,expiration,strike,right,timestamp,"),
            "v3 CSV leads with contract identity then the time column: {header}"
        );
        // Every data column is still present somewhere in the header.
        for col in ["sequence", "condition", "size", "exchange", "price"] {
            assert!(header.contains(col), "data column {col} missing from {header}");
        }
    }
}
