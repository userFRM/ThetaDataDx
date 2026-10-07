//! Fixture parameter values for the interactive request builder.
//!
//! These derive from the same `GeneratedEndpoint` model the SDK
//! projection emitters consume, so a registry change reshapes the
//! builder's seeded values on the next generator run.

use super::super::model::{GeneratedEndpoint, GeneratedParam};
use super::super::sdk_helpers::builder_params;

// ───────────────────────── Sample fixture values ────────────────────────────
//
// Mirrors `[test_fixtures]` in `endpoint_surface.toml`: the same values the
// generated live validators exercise, so every sample is a request the
// production server is known to answer.

pub(super) fn sample_value(param: &GeneratedParam, category: &str) -> &'static str {
    match param.param_type.as_str() {
        "Symbol" | "Symbols" => match category {
            "stock" => "AAPL",
            "option" => "SPY",
            "index" => "SPX",
            "rate" => "SOFR",
            other => panic!("no sample symbol for category {other}"),
        },
        "Date" => match param.name.as_str() {
            "start_date" => "20250303",
            "end_date" => "20250306",
            _ => "20250303",
        },
        "Expiration" => "20250321",
        "Strike" => "570",
        "Right" => "C",
        "Interval" => "1m",
        "RequestType" => "trade",
        "Year" => "2025",
        "Str" if param.name == "time_of_day" => "10:30:00.000",
        "Str" => "10:30:00",
        other => panic!("no sample value for param type {other}"),
    }
}

/// Builder params showcased in the runnable samples. Pinning `strike` +
/// `right` turns the wildcard default into the canonical single-contract
/// request; `interval` is the one tuning knob most requests set.
///
/// A date joins them wherever the route takes one only as an optional. The
/// vendor marks `date`, `start_date` and `end_date` optional on the history
/// routes and then refuses a request that carries none of them, so a sample
/// seeded without a date is a call that can only fail. The anchor follows
/// `anchor_dates` in `modes.rs`, which gives the validator cells a date for the
/// same reason: a single `date` where the route takes one, both ends of the
/// range where it takes a range instead.
pub(super) fn showcased_builder_params(endpoint: &GeneratedEndpoint) -> Vec<&GeneratedParam> {
    let builder = builder_params(endpoint);
    let has = |name: &str| builder.iter().any(|p| p.name == name);
    let anchors: Vec<&str> = if has("date") {
        vec!["date"]
    } else if has("start_date") && has("end_date") {
        vec!["start_date", "end_date"]
    } else {
        Vec::new()
    };
    builder
        .into_iter()
        .filter(|p| {
            matches!(p.name.as_str(), "strike" | "right" | "interval")
                || anchors.contains(&p.name.as_str())
        })
        .collect()
}
