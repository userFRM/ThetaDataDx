//! Cross-check the `[test_fixtures]` block against the resolved endpoint set.
//!
//! Bin-only: the live-validator matrix only runs from the
//! `generate_sdk_surfaces` binary path. The build script never emits
//! validator code, so it never needs these checks.

use std::collections::{HashMap, HashSet};

use super::model::GeneratedEndpoint;
use super::test_fixtures::TestFixtures;

/// Closed sets the live-validator matrix in `modes.rs` knows how to consume.
/// Anything outside these tables is a typo in the TOML and silently dropped
/// coverage, so we reject it at build time. Keep these in lockstep with
/// `modes.rs::test_modes_for` and the wire-type tables in `helpers.rs`.
const KNOWN_CATEGORIES_WITH_SYMBOL: &[&str] = &["stock", "option", "index", "rate"];
const KNOWN_WIRE_TYPES: &[&str] = &[
    "Date",
    "Expiration",
    "Strike",
    "Right",
    "Interval",
    "Venue",
    "RateType",
    "Version",
    "RequestType",
    "Year",
    "Str",
];
const KNOWN_MODE_OVERRIDES: &[&str] = &[
    "concrete_iso",
    "all_strikes_one_exp",
    "all_exps_one_strike",
    "bulk_chain",
    "legacy_zero_wildcard",
];

/// Cross-check the `[test_fixtures]` block against the resolved endpoint set.
///
/// Prevents every silent-coverage-regression path prior review has flagged:
///
///   * Missing `optional_defaults` row silently drops the corresponding
///     `with_<name>` cell and excludes the param from `all_optionals`.
///     Typos in any fixture map silently fall through to a default and
///     test the wrong value.
///   * Missing rows in `category_symbol`, `concrete_by_type`, or
///     `mode_overrides` fall through to opaque panics in `modes.rs`. Dead
///     keys under `mode_overrides.<mode>` are accepted even when no endpoint
///     emitting that mode binds the name (the override is silently unused).
///
/// Every check collects every offender across every fixture map and returns
/// them in one combined error so a dev fixing TOML drift sees the full
/// picture in a single rebuild, not one error per `cargo run`.
pub(super) fn validate_test_fixtures(
    fixtures: &TestFixtures,
    endpoints: &[GeneratedEndpoint],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut errors: Vec<String> = Vec::new();

    // Vocabulary derived from the resolved endpoint set.
    //
    // Streaming endpoints are skipped because `test_modes_for` returns an
    // empty mode vec for them — they never touch the fixture maps, so their
    // params shouldn't drive required-row checks. Non-streaming endpoints
    // are the matrix's full consumer set.
    let live_endpoints: Vec<&GeneratedEndpoint> =
        endpoints.iter().filter(|ep| ep.kind != "stream").collect();

    let known_method_param_names: HashSet<&str> = live_endpoints
        .iter()
        .flat_map(|ep| ep.params.iter())
        .filter(|p| p.binding == "method")
        .map(|p| p.name.as_str())
        .collect();
    let known_builder_param_names: HashSet<&str> = live_endpoints
        .iter()
        .flat_map(|ep| ep.params.iter())
        .filter(|p| p.binding == "builder")
        .map(|p| p.name.as_str())
        .collect();

    // Required rows: one check per fixture map. Every row that `modes.rs`
    // will touch at emission time must exist at validation time so a
    // missing TOML entry never falls through to an opaque panic.

    // category_symbol: every non-streaming endpoint whose method params
    // include a Symbol/Symbols reads `category_symbol[endpoint.category]`.
    // Build the required set from the live endpoint surface.
    let mut required_category_rows: HashMap<&str, Vec<&str>> = HashMap::new();
    for endpoint in &live_endpoints {
        let has_symbol_param = endpoint.params.iter().any(|p| {
            p.binding == "method" && matches!(p.param_type.as_str(), "Symbol" | "Symbols")
        });
        if !has_symbol_param {
            continue;
        }
        required_category_rows
            .entry(endpoint.category.as_str())
            .or_default()
            .push(endpoint.name.as_str());
    }
    let mut missing_category_rows: Vec<String> = required_category_rows
        .iter()
        .filter(|(category, _)| !fixtures.category_symbol.contains_key(**category))
        .map(|(category, consumers)| {
            let mut names = consumers.clone();
            names.sort();
            format!(
                "  '{}' (needed for {} endpoint(s): first is {})",
                category,
                names.len(),
                names.first().copied().unwrap_or("?"),
            )
        })
        .collect();
    if !missing_category_rows.is_empty() {
        missing_category_rows.sort();
        errors.push(format!(
            "[test_fixtures.category_symbol] is missing required entries:\n{}",
            missing_category_rows.join("\n")
        ));
    }

    // concrete_by_type: every method-bound param whose `param_type` is not
    // `Symbol`/`Symbols` (those route through `category_symbol`) reads
    // `concrete_by_type[param.param_type]` — unless `concrete_overrides` has
    // a row for the param name, which wins.
    let mut required_type_rows: HashMap<&str, Vec<String>> = HashMap::new();
    for endpoint in &live_endpoints {
        for param in &endpoint.params {
            if param.binding != "method"
                || matches!(param.param_type.as_str(), "Symbol" | "Symbols")
            {
                continue;
            }
            if fixtures.concrete_overrides.contains_key(&param.name) {
                continue;
            }
            required_type_rows
                .entry(param.param_type.as_str())
                .or_default()
                .push(format!("{}.{}", endpoint.name, param.name));
        }
    }
    let mut missing_type_rows: Vec<String> = required_type_rows
        .iter()
        .filter(|(ty, _)| !fixtures.concrete_by_type.contains_key(**ty))
        .map(|(ty, consumers)| {
            let mut consumers = consumers.clone();
            consumers.sort();
            format!(
                "  '{}' (needed for {} param(s): first is {})",
                ty,
                consumers.len(),
                consumers.first().map(String::as_str).unwrap_or("?")
            )
        })
        .collect();
    if !missing_type_rows.is_empty() {
        missing_type_rows.sort();
        errors.push(format!(
            "[test_fixtures.concrete_by_type] is missing required entries:\n{}",
            missing_type_rows.join("\n")
        ));
    }

    // mode_overrides: every mode that at least one endpoint actually emits
    // must have an entry (empty is fine; present-but-empty won't trip the
    // opaque `.get(mode_name).unwrap_or_else(panic!())` in `args_for_mode`).
    let mode_emitters = compute_mode_emitters(&live_endpoints);
    let mut missing_mode_entries: Vec<String> = Vec::new();
    for (mode_name, consumers) in &mode_emitters {
        if consumers.is_empty() {
            continue;
        }
        if !fixtures.mode_overrides.contains_key(*mode_name) {
            let mut sample = consumers.clone();
            sample.sort();
            missing_mode_entries.push(format!(
                "  '{}' (emitted by {} endpoint(s): first is {})",
                mode_name,
                sample.len(),
                sample.first().copied().unwrap_or("?")
            ));
        }
    }
    if !missing_mode_entries.is_empty() {
        missing_mode_entries.sort();
        errors.push(format!(
            "[test_fixtures.mode_overrides] is missing required entries:\n{}",
            missing_mode_entries.join("\n")
        ));
    }

    // optional_defaults: every builder-bound param the matrix references
    // needs a row, otherwise `with_<name>` silently drops and `all_optionals`
    // excludes the key. Walk the resolved endpoint set, report per-endpoint
    // so the dev sees blast radius.
    let mut missing_optional_defaults: Vec<String> = Vec::new();
    for endpoint in &live_endpoints {
        for param in &endpoint.params {
            if param.binding != "builder" {
                continue;
            }
            if !fixtures.optional_defaults.contains_key(&param.name) {
                missing_optional_defaults.push(format!(
                    "  '{}' (needed for {}.with_{})",
                    param.name, endpoint.name, param.name
                ));
            }
        }
    }
    if !missing_optional_defaults.is_empty() {
        missing_optional_defaults.sort();
        missing_optional_defaults.dedup();
        errors.push(format!(
            "[test_fixtures.optional_defaults] is missing required entries:\n{}",
            missing_optional_defaults.join("\n")
        ));
    }

    // Unknown / dead keys: every map's keys must match its expected
    // vocabulary. Typos and stale rows surface here with full context.

    // category_symbol → known-with-symbol categories.
    let unknown_categories: Vec<&str> = fixtures
        .category_symbol
        .keys()
        .map(String::as_str)
        .filter(|name| !KNOWN_CATEGORIES_WITH_SYMBOL.contains(name))
        .collect();
    if !unknown_categories.is_empty() {
        let mut sorted = unknown_categories;
        sorted.sort();
        errors.push(format!(
            "[test_fixtures.category_symbol] has unknown categories: {} (expected one of: {})",
            sorted.join(", "),
            KNOWN_CATEGORIES_WITH_SYMBOL.join(", ")
        ));
    }

    // concrete_by_type → known wire types.
    let unknown_wire_types: Vec<&str> = fixtures
        .concrete_by_type
        .keys()
        .map(String::as_str)
        .filter(|name| !KNOWN_WIRE_TYPES.contains(name))
        .collect();
    if !unknown_wire_types.is_empty() {
        let mut sorted = unknown_wire_types;
        sorted.sort();
        errors.push(format!(
            "[test_fixtures.concrete_by_type] has unknown wire types: {} (expected one of: {})",
            sorted.join(", "),
            KNOWN_WIRE_TYPES.join(", ")
        ));
    }

    // concrete_overrides → real method-call param names.
    let unknown_concrete_overrides: Vec<&str> = fixtures
        .concrete_overrides
        .keys()
        .map(String::as_str)
        .filter(|name| !known_method_param_names.contains(name))
        .collect();
    if !unknown_concrete_overrides.is_empty() {
        let mut sorted = unknown_concrete_overrides;
        sorted.sort();
        errors.push(format!(
            "[test_fixtures.concrete_overrides] references param names that are not method-bound \
             on any endpoint: {}",
            sorted.join(", ")
        ));
    }

    // mode_overrides.<mode> → known mode names; inner keys → per-mode valid
    // set (params bound by endpoints that emit that mode). A key that's
    // method-bound globally but dead under a specific mode — e.g. `year`
    // under `concrete_iso` where no ContractSpec endpoint binds `year` —
    // must fail for that mode, not just for a global union.
    let mut unknown_mode_names: Vec<&str> = Vec::new();
    let mut unused_mode_param_keys: Vec<String> = Vec::new();
    for (mode_name, overrides) in &fixtures.mode_overrides {
        if !KNOWN_MODE_OVERRIDES.contains(&mode_name.as_str()) {
            unknown_mode_names.push(mode_name.as_str());
            continue;
        }
        let valid_keys = mode_override_valid_keys(&live_endpoints, mode_name.as_str());
        for key in overrides.keys() {
            if !valid_keys.contains(key.as_str()) {
                unused_mode_param_keys.push(format!("{mode_name}.{key}"));
            }
        }
    }
    if !unknown_mode_names.is_empty() {
        unknown_mode_names.sort();
        errors.push(format!(
            "[test_fixtures.mode_overrides] has unknown mode names: {} (expected one of: {})",
            unknown_mode_names.join(", "),
            KNOWN_MODE_OVERRIDES.join(", ")
        ));
    }
    if !unused_mode_param_keys.is_empty() {
        unused_mode_param_keys.sort();
        errors.push(format!(
            "[test_fixtures.mode_overrides] contains dead keys — not bound by any endpoint \
             emitting that mode: {}",
            unused_mode_param_keys.join(", ")
        ));
    }

    // optional_defaults → real builder-bound param names.
    let unknown_optional_defaults: Vec<&str> = fixtures
        .optional_defaults
        .keys()
        .map(String::as_str)
        .filter(|name| !known_builder_param_names.contains(name))
        .collect();
    if !unknown_optional_defaults.is_empty() {
        let mut sorted = unknown_optional_defaults;
        sorted.sort();
        errors.push(format!(
            "[test_fixtures.optional_defaults] references param names that are not builder-bound \
             on any endpoint: {}",
            sorted.join(", ")
        ));
    }

    if !errors.is_empty() {
        return Err(format!(
            "endpoint_surface.toml [test_fixtures] validation failed:\n\n{}",
            errors.join("\n\n")
        )
        .into());
    }
    Ok(())
}

/// For each mode in [`KNOWN_MODE_OVERRIDES`], collect the endpoints that emit
/// it according to `modes.rs::emitted_mode_names`.
///
/// Streaming endpoints are already filtered out by the caller. A mode with
/// zero consumers doesn't need a TOML row and isn't reported as missing.
fn compute_mode_emitters<'a>(
    endpoints: &[&'a GeneratedEndpoint],
) -> Vec<(&'static str, Vec<&'a str>)> {
    let mut out: Vec<(&'static str, Vec<&'a str>)> = Vec::new();
    for mode in KNOWN_MODE_OVERRIDES {
        let consumers: Vec<&'a str> = endpoints
            .iter()
            .filter(|ep| super::modes::emitted_mode_names(ep).contains(mode))
            .map(|ep| ep.name.as_str())
            .collect();
        out.push((*mode, consumers));
    }
    out
}

/// Union of method-param names belonging to every endpoint that emits the
/// given mode. This is the per-mode valid-key set used to detect dead keys
/// under `mode_overrides.<mode>`. A key that's valid globally (it's a
/// method-bound name on some endpoint somewhere) but unused under the
/// specific mode is flagged as dead.
fn mode_override_valid_keys<'a>(
    endpoints: &[&'a GeneratedEndpoint],
    mode: &str,
) -> HashSet<&'a str> {
    endpoints
        .iter()
        .filter(|ep| super::modes::emitted_mode_names(ep).contains(&mode))
        .flat_map(|ep| ep.params.iter())
        .map(|p| p.name.as_str())
        .collect()
}
