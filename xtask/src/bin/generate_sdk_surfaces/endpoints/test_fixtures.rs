//! The values the live validators and the published examples send.
//!
//! A lookup with no value for what it is asked panics, so the generator fails
//! rather than emitting a cell or an example that leaves the argument out.
//! [`check_fixtures`] holds the tables to the endpoints that read them.

use super::modes::emitted_mode_names;
use super::table::{ENUMS, GeneratedEndpoint};

/// The symbol sent to an endpoint, by category.
const CATEGORY_SYMBOLS: &[(&str, &str)] = &[
    ("stock", "AAPL"),
    ("option", "SPY"),
    ("index", "SPX"),
    ("rate", "SOFR"),
];

/// The value sent for a parameter that is not a symbol, by type.
const TYPE_VALUES: &[(&str, &str)] = &[
    ("Date", "20250303"),
    ("Expiration", "20250321"),
    ("Strike", "570"),
    ("Right", "C"),
    ("Interval", "1m"),
    ("RequestType", "trade"),
    ("Year", "2025"),
    ("Time", "12:00:00.000"),
];

/// The value a validator cell sets for each optional parameter.
const OPTIONAL_VALUES: &[(&str, &str)] = &[
    ("symbol", "SPY"),
    ("max_dte", "30"),
    ("strike", "570"),
    ("right", "call"),
    ("interval", "1m"),
    ("strike_range", "10"),
    ("min_time", "09:45:00"),
    ("venue", "nqb"),
    ("start_time", "09:30:00"),
    ("end_time", "10:00:00"),
    ("date", "20250303"),
    ("start_date", "20250303"),
    ("end_date", "20250303"),
    ("exclusive", "true"),
    ("annual_dividend", "0.015"),
    ("rate_type", "sofr"),
    ("rate_value", "0.05"),
    ("stock_price", "150.0"),
    ("version", "1"),
    ("use_market_value", "true"),
    ("underlyer_use_nbbo", "true"),
    ("binomial_steps", "51"),
    ("perf_boost_intraday", "true"),
    ("current_day_only", "true"),
    ("annual_dividend_amount", "0.96"),
    ("option_use_nbbo", "false"),
    ("use_time_based_trade_quote", "false"),
];

/// The arguments each wildcard mode sets in place of the concrete fixture.
const MODE_OVERRIDES: &[(&str, &[(&str, &str)])] = &[
    ("concrete_iso", &[("expiration", "2025-03-21")]),
    ("all_strikes_one_exp", &[("strike", "*"), ("right", "both")]),
    (
        "all_exps_one_strike",
        &[("expiration", "*"), ("right", "both")],
    ),
    (
        "bulk_chain",
        &[("expiration", "*"), ("strike", "*"), ("right", "both")],
    ),
];

fn lookup<T: Copy>(table: &[(&str, T)], key: &str) -> Option<T> {
    table
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, value)| *value)
}

/// The symbol sent to an endpoint of `category`.
pub(super) fn category_symbol(category: &str) -> &'static str {
    lookup(CATEGORY_SYMBOLS, category)
        .unwrap_or_else(|| panic!("no fixture symbol for category '{category}'"))
}

/// The value sent for a parameter of `param_type` that is not a symbol.
pub(super) fn type_value(param_type: &str) -> &'static str {
    lookup(TYPE_VALUES, param_type)
        .unwrap_or_else(|| panic!("no fixture value for parameter type '{param_type}'"))
}

/// The value a validator cell sets for the optional parameter `name`.
pub(super) fn optional_value(name: &str) -> &'static str {
    lookup(OPTIONAL_VALUES, name)
        .unwrap_or_else(|| panic!("no fixture value for optional parameter '{name}'"))
}

/// The arguments the wildcard mode `mode` sets.
pub(super) fn mode_overrides(mode: &str) -> &'static [(&'static str, &'static str)] {
    lookup(MODE_OVERRIDES, mode).unwrap_or_else(|| panic!("no fixture overrides for mode '{mode}'"))
}

/// Every value an endpoint needs must be present, including where no
/// validator cell asks for it yet. Every row must be read by some endpoint:
/// a misspelled override is otherwise ignored and leaves its mode sending the
/// concrete value. A value for an enum parameter must be one of the enum's
/// wire values, or the validators send a request the vendor never agreed to
/// serve.
pub(super) fn check_fixtures(
    endpoints: &[GeneratedEndpoint],
) -> Result<(), Box<dyn std::error::Error>> {
    let live: Vec<&GeneratedEndpoint> = endpoints.iter().filter(|e| e.shape != "stream").collect();
    let params = || live.iter().flat_map(|e| e.params.iter());
    let mut errors = Vec::new();

    for endpoint in &live {
        for param in endpoint.params {
            let value = match (param.required, param.param_type) {
                (true, "Symbol" | "Symbols") => lookup(CATEGORY_SYMBOLS, endpoint.category),
                (true, param_type) => lookup(TYPE_VALUES, param_type),
                (false, _) => lookup(OPTIONAL_VALUES, param.name),
            };
            if value.is_none() {
                errors.push(format!(
                    "{}.{} has no fixture value",
                    endpoint.name, param.name
                ));
            }
        }
        for mode in emitted_mode_names(endpoint) {
            if !matches!(mode, "basic" | "concrete") && lookup(MODE_OVERRIDES, mode).is_none() {
                errors.push(format!(
                    "mode '{mode}' of {} has no fixture overrides",
                    endpoint.name
                ));
            }
        }
    }

    for &(category, _) in CATEGORY_SYMBOLS {
        if !live.iter().any(|e| {
            e.category == category
                && e.params
                    .iter()
                    .any(|p| matches!(p.param_type, "Symbol" | "Symbols"))
        }) {
            errors.push(format!(
                "no endpoint reads the symbol for category '{category}'"
            ));
        }
    }
    for &(param_type, _) in TYPE_VALUES {
        if !params().any(|p| p.param_type == param_type) {
            errors.push(format!(
                "no endpoint reads the value for type '{param_type}'"
            ));
        }
    }
    for &(name, _) in OPTIONAL_VALUES {
        if !params().any(|p| !p.required && p.name == name) {
            errors.push(format!("no endpoint reads the optional value for '{name}'"));
        }
    }
    for &(mode, overrides) in MODE_OVERRIDES {
        let emitters: Vec<&&GeneratedEndpoint> = live
            .iter()
            .filter(|e| emitted_mode_names(e).contains(&mode))
            .collect();
        if emitters.is_empty() {
            errors.push(format!("no endpoint emits mode '{mode}'"));
        }
        for &(name, _) in overrides {
            if !emitters
                .iter()
                .any(|e| e.params.iter().any(|p| p.name == name))
            {
                errors.push(format!(
                    "no endpoint emitting mode '{mode}' reads its '{name}'"
                ));
            }
        }
    }

    // Only the rows sent verbatim: `TYPE_VALUES` sends `Right` as `C`, a short
    // form the SDK normalises to the wire value before the request.
    let overrides = MODE_OVERRIDES.iter().flat_map(|(_, rows)| rows.iter());
    for &(name, value) in OPTIONAL_VALUES.iter().chain(overrides) {
        for e in ENUMS
            .iter()
            .filter(|e| params().any(|p| p.name == name && p.param_type == e.name))
        {
            if !e.values.contains(&value) {
                errors.push(format!(
                    "fixture value '{value}' for '{name}' is not a value of enum '{}'",
                    e.name
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "test fixtures do not match the endpoint table:\n  {}",
            errors.join("\n  ")
        )
        .into())
    }
}
