//! Parse `proto/mdds.proto` into a wire-truth intermediate form.
//!
//! Discovers `Get*` RPCs, extracts their `*RequestQuery` message fields,
//! expands `ContractSpec` to (symbol, expiration, strike, right), derives the
//! return type from the method name, and applies SDK-specific normalizations
//! (e.g. single-symbol endpoints demote `Symbols` back to `Symbol`).

use std::collections::HashMap;

use super::model::{GeneratedEndpoint, GeneratedParam, ProtoField, Rpc, WireEndpoints};

/// Parse endpoint metadata from `mdds.proto` into a reusable intermediate form.
///
/// This build-time parser performs several tightly-coupled passes over the same
/// proto source: RPC discovery, request-query extraction, field expansion,
/// endpoint normalization, and a small set of SDK-specific augmentations. It is
/// intentionally kept in one place so the generated registry, shared endpoint
/// runtime, and SDK surface stay aligned while the explicit endpoint surface
/// spec is validated against the wire contract.
#[allow(clippy::too_many_lines)] // Reason: build-time endpoint parser coordinates multiple passes over one proto source.
pub(super) fn load_proto_endpoints() -> Result<WireEndpoints, Box<dyn std::error::Error>> {
    let proto = std::fs::read_to_string("proto/mdds.proto")?;

    // ── Parse RPCs ──────────────────────────────────────────────────────────
    let rpc_re = regex::Regex::new(r"rpc\s+(Get\w+)\s*\((\w+)\)\s*returns")?;
    let rpcs: Vec<Rpc> = rpc_re
        .captures_iter(&proto)
        .map(|c| Rpc {
            rpc_name: c[1].to_string(),
            request_type: c[2].to_string(),
        })
        .collect();

    // ── Parse query messages ────────────────────────────────────────────────
    // Everything lives in one package, so ContractSpec is referenced
    // unqualified instead of `endpoints.ContractSpec`.
    let msg_re = regex::Regex::new(r"message\s+(\w+RequestQuery)\s*\{([^}]*)}")?;
    let field_re = regex::Regex::new(
        r"(optional\s+|repeated\s+)?(string|int32|double|bool|ContractSpec)\s+(\w+)\s*=\s*\d+",
    )?;

    let mut query_messages: HashMap<String, Vec<ProtoField>> = HashMap::new();
    for cap in msg_re.captures_iter(&proto) {
        let msg_name = cap[1].to_string();
        let body = &cap[2];
        let fields: Vec<ProtoField> = field_re
            .captures_iter(body)
            .map(|f| ProtoField {
                name: f[3].to_string(),
                proto_type: f[2].to_string(),
                is_optional: f.get(1).is_some_and(|m| m.as_str().starts_with("optional")),
                is_repeated: f.get(1).is_some_and(|m| m.as_str().starts_with("repeated")),
            })
            .collect();
        query_messages.insert(msg_name, fields);
    }

    let mut endpoints = Vec::new();

    for rpc in &rpcs {
        // Derive snake_case method name: GetStockHistoryEod → stock_history_eod
        let method = rpc_to_method(&rpc.rpc_name);

        // Find the query message: StockHistoryEodRequest → StockHistoryEodRequestQuery
        let query_msg_name = format!("{}Query", rpc.request_type);
        let fields = if let Some(f) = query_messages.get(&query_msg_name) {
            f.clone()
        } else {
            eprintln!(
                "warning: no query message '{}' found, skipping {}",
                query_msg_name, rpc.rpc_name
            );
            continue;
        };

        // Expand fields (contract_spec → symbol, expiration, strike, right)
        let params = expand_fields(&fields);

        // Only return_type is cross-validated against the surface spec (line ~804).
        // Category, subcategory, rest_path, description come entirely from the TOML.
        let return_type = derive_return_type(&method);
        let mut params = params
            .into_iter()
            .map(|(name, description, param_type, required)| GeneratedParam {
                name,
                description,
                param_type,
                required,
                binding: String::new(),
                _arg_name: None,
                default: None,
            })
            .collect::<Vec<_>>();
        normalize_method_params(&method, &mut params);

        endpoints.push(GeneratedEndpoint {
            name: method,
            description: String::new(),
            category: String::new(),
            subcategory: String::new(),
            _rest_path: String::new(),
            grpc_name: format!("get_{}", rpc_to_method(&rpc.rpc_name)),
            request_type: rpc.request_type.clone(),
            query_type: query_msg_name,
            fields,
            params,
            return_type,
            kind: String::new(),
            list_column: None,
            // Proto-derived entries never carry a vendor docstring — the
            // surface spec (endpoint_surface.toml) is the SSOT; this field
            // is merged in during `parser::merge_surface_and_wire`.
            vendor_docstring: None,
        });
    }

    Ok(WireEndpoints { endpoints })
}

fn is_simple_list_method(method: &str) -> bool {
    method.ends_with("_list_symbols")
        || method.ends_with("_list_dates")
        || method.ends_with("_list_expirations")
        || method.ends_with("_list_strikes")
}

/// Convert `GetStockHistoryEod` → `stock_history_eod`.
fn rpc_to_method(rpc_name: &str) -> String {
    // Strip leading "Get"
    let name = rpc_name.strip_prefix("Get").unwrap_or(rpc_name); // build script: panic is intentional
                                                                 // PascalCase → snake_case
    let mut result = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap()); // build script: panic is intentional
        } else {
            result.push(ch);
        }
    }
    result
}

/// Expand proto fields, replacing `contract_spec` with (symbol, expiration, strike, right).
///
/// Many option query messages carry both a `ContractSpec` (contract identity,
/// expanded here to 4 fields) AND an explicit top-level `expiration` field
/// (the query range expiration — e.g. "include all contracts expiring by..."),
/// which would otherwise collide with the contract's own expiration. Any
/// post-expansion duplicate parameter name is dropped in favor of the first
/// occurrence (ContractSpec wins, since it is structurally the contract
/// identity the user really cares about).
fn expand_fields(fields: &[ProtoField]) -> Vec<(String, String, String, bool)> {
    let mut params: Vec<(String, String, String, bool)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let push = |params: &mut Vec<(String, String, String, bool)>,
                seen: &mut std::collections::HashSet<String>,
                entry: (String, String, String, bool)| {
        if seen.insert(entry.0.clone()) {
            params.push(entry);
        }
    };

    for f in fields {
        if f.proto_type == "ContractSpec" {
            // Expand to the 4 contract spec fields (symbol, expiration, strike, right).
            push(
                &mut params,
                &mut seen,
                (
                    "symbol".into(),
                    "Underlying symbol (e.g. AAPL)".into(),
                    "Symbol".into(),
                    true,
                ),
            );
            push(
                &mut params,
                &mut seen,
                (
                    "expiration".into(),
                    "Expiration date YYYYMMDD".into(),
                    "Expiration".into(),
                    true,
                ),
            );
            push(
                &mut params,
                &mut seen,
                (
                    "strike".into(),
                    "Strike price (raw integer)".into(),
                    "Strike".into(),
                    true,
                ),
            );
            push(
                &mut params,
                &mut seen,
                (
                    "right".into(),
                    "C for call, P for put".into(),
                    "Right".into(),
                    true,
                ),
            );
        } else {
            let (param_type, desc) = map_field(&f.name, &f.proto_type, f.is_repeated);
            // A `repeated` field is inherently optional on the wire: an empty
            // list is a valid, fully-omitted value (proto3 has no presence
            // bit for repeated fields). Treating it as wire-required would
            // forbid a surface from relaxing it even when upstream accepts
            // the omission — e.g. `option/list/contracts`, where dropping the
            // symbol filter lists the full date universe. Endpoints that do
            // require the field still declare `required = true` on their
            // surface param, which stays valid (a surface may be stricter
            // than the wire, never looser).
            let required = !f.is_optional && !f.is_repeated;
            push(
                &mut params,
                &mut seen,
                (f.name.clone(), desc, param_type, required),
            );
        }
    }
    params
}

/// Map a proto field (name + type + repeated) to (`ParamType` variant name, description).
fn map_field(name: &str, proto_type: &str, is_repeated: bool) -> (String, String) {
    // Repeated string symbol → Symbols
    if is_repeated && name == "symbol" {
        return (
            "Symbols".into(),
            "Comma-separated ticker symbols (e.g. AAPL,MSFT)".into(),
        );
    }

    match (proto_type, name) {
        ("string", "symbol") => ("Symbol".into(), "Ticker symbol (e.g. AAPL)".into()),
        ("string", "start_date") => ("Date".into(), "Start date YYYYMMDD".into()),
        ("string", "end_date") => ("Date".into(), "End date YYYYMMDD".into()),
        ("string", "date") => ("Date".into(), "Date YYYYMMDD".into()),
        ("string", "interval") => (
            "Interval".into(),
            "One of the interval presets: tick, 10ms, 100ms, 500ms, 1s, 5s, 10s, 15s, 30s, 1m, 5m, 10m, 15m, 30m, 1h.".into(),
        ),
        ("string", "right") => ("Right".into(), "C for call, P for put".into()),
        ("string", "strike") => (
            "Strike".into(),
            "Strike price in dollars as a string (e.g. 500 or 17.5)".into(),
        ),
        ("string", "expiration") => ("Expiration".into(), "Expiration date YYYYMMDD".into()),
        ("string", "request_type") => (
            "RequestType".into(),
            "Request type: EOD, TRADE, QUOTE, OHLC, etc.".into(),
        ),
        ("string", "year") => ("Year".into(), "4-digit year (e.g. 2024)".into()),
        ("string", "time_of_day") => (
            "Str".into(),
            "ET wall-clock time in HH:MM:SS.SSS (e.g. 09:30:00.000 for 9:30 AM; legacy 34200000 is also accepted)".into(),
        ),
        ("string", "venue") => ("Venue".into(), "Venue/exchange filter".into()),
        ("string", "min_time") => ("Str".into(), "Minimum time filter".into()),
        ("string", "start_time") => ("Str".into(), "Start time filter".into()),
        ("string", "end_time") => ("Str".into(), "End time filter".into()),
        ("string", "rate_type") => ("RateType".into(), "Rate type".into()),
        ("string", "version") => ("Version".into(), "Greeks model version".into()),
        ("double", _) => ("Float".into(), humanize_name(name).clone()),
        ("int32", "max_dte") => ("Int".into(), "Maximum days to expiration".into()),
        ("int32", "strike_range") => ("Int".into(), "Strike range filter".into()),
        ("int32", _) => ("Int".into(), humanize_name(name).clone()),
        ("bool", "exclusive") => ("Bool".into(), "Exclusive time boundary".into()),
        ("bool", "use_market_value") => ("Bool".into(), "Use market value for Greeks".into()),
        ("bool", "underlyer_use_nbbo") => ("Bool".into(), "Use NBBO for underlyer price".into()),
        ("bool", _) => ("Bool".into(), humanize_name(name).clone()),
        _ => ("Str".into(), humanize_name(name).clone()),
    }
}

fn humanize_name(name: &str) -> String {
    name.replace('_', " ")
        .split_whitespace()
        .enumerate()
        .map(|(i, w)| {
            if i == 0 {
                let mut c = w.chars();
                match c.next() {
                    Some(first) => first.to_uppercase().to_string() + c.as_str(),
                    None => String::new(),
                }
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn derive_return_type(method: &str) -> String {
    if is_simple_list_method(method) {
        return "StringList".into();
    }

    if method == "option_list_contracts" {
        return "OptionContracts".into();
    }

    if method.starts_with("calendar_") {
        return "CalendarDays".into();
    }

    if method.starts_with("interest_rate_") {
        return "InterestRateTicks".into();
    }

    if method.contains("_trade_quote") {
        return "TradeQuoteTicks".into();
    }

    if method.contains("_open_interest") {
        return "OpenInterestTicks".into();
    }

    if method.contains("_market_value") {
        return "MarketValueTicks".into();
    }

    // The `option_history_trade_greeks_*` endpoints calculate Greeks per
    // OPRA trade and ship nine trade-side execution columns alongside the
    // Greek values. They route to dedicated `TradeGreeks*Tick` types --
    // distinct from the interval-sampled `Greeks*Tick` variants whose
    // wire rows carry the bid/ask quote pair instead. Match these BEFORE
    // the bare `greeks_*` arms so the trade-side prefix takes precedence.
    if method.contains("trade_greeks_implied_volatility") {
        return "TradeGreeksImpliedVolatilityTicks".into();
    }
    if method.contains("trade_greeks_first_order") {
        return "TradeGreeksFirstOrderTicks".into();
    }
    if method.contains("trade_greeks_second_order") {
        return "TradeGreeksSecondOrderTicks".into();
    }
    if method.contains("trade_greeks_third_order") {
        return "TradeGreeksThirdOrderTicks".into();
    }
    if method.contains("trade_greeks") {
        return "TradeGreeksAllTicks".into();
    }

    if method.contains("greeks_implied_volatility") {
        return "IvTicks".into();
    }

    if method.contains("greeks_first_order") {
        return "GreeksFirstOrderTicks".into();
    }

    if method.contains("greeks_second_order") {
        return "GreeksSecondOrderTicks".into();
    }

    if method.contains("greeks_third_order") {
        return "GreeksThirdOrderTicks".into();
    }

    // `_greeks_eod` routes to a dedicated `GreeksEodTick` whose wire
    // shape (39 data columns) fuses the full Greeks union with the
    // twelve EOD trade/quote columns (`open`, `high`, `low`, `close`,
    // `volume`, `count`, `bid_size`, `bid_exchange`, `bid_condition`,
    // `ask_size`, `ask_exchange`, `ask_condition`) the bare
    // `GreeksAllTick` silently dropped. Match BEFORE the generic
    // `_greeks_` arm so the EOD specialisation takes precedence.
    if method.contains("greeks_eod") {
        return "GreeksEodTicks".into();
    }

    // `_greeks_all` and any future un-suffixed Greeks endpoint default
    // to the full-union type.
    if method.contains("_greeks_") {
        return "GreeksAllTicks".into();
    }

    // `index_at_time_price` returns a trade-shaped row (10 columns:
    // `timestamp`, `sequence`, `ext_condition1..4`, `condition`,
    // `size`, `exchange`, `price`) -- distinct from the bare
    // `PriceTick` (3 columns) used by `index_snapshot_price` /
    // `index_history_price`. Match BEFORE those routes so the trade
    // shape takes precedence.
    if method == "index_at_time_price" {
        return "IndexPriceAtTimeTicks".into();
    }

    if method == "index_snapshot_price" || method == "index_history_price" {
        return "PriceTicks".into();
    }

    if method.ends_with("_history_eod") {
        return "EodTicks".into();
    }

    if method.contains("_ohlc") {
        return "OhlcTicks".into();
    }

    if method.contains("_trade") || method.ends_with("_trade") {
        return "TradeTicks".into();
    }

    if method.contains("_quote") || method.ends_with("_quote") {
        return "QuoteTicks".into();
    }

    panic!("unhandled return type mapping for endpoint {method}");
}

fn normalize_method_params(method: &str, params: &mut [GeneratedParam]) {
    let supports_symbol_lists =
        method.starts_with("stock_snapshot_") || method.starts_with("index_snapshot_");

    if !supports_symbol_lists {
        for param in params.iter_mut() {
            if param.name == "symbol" && param.param_type == "Symbols" {
                param.param_type = "Symbol".into();
                param.description = "Ticker symbol (e.g. AAPL)".into();
            }
        }
    }
}
