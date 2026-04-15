//! Parameter-mode matrix used by the live validator renderers.
//!
//! [`TestMode`] captures one (parameter-shape × tier) cell to exercise against
//! a real account. Modes are derived per-endpoint by [`test_modes_for`] from
//! the endpoint's wire shape: list endpoints get one mode, option ContractSpec
//! endpoints get the full wildcard cross-product, and so on. Tier information
//! flows from the pinned upstream OpenAPI snapshot.
//!
//! Every representative value (symbols, dates, expirations, wildcard
//! sentinels, optional defaults) comes from the `[test_fixtures]` block in
//! `endpoint_surface.toml`. `modes.rs` carries no fixture literals — swap
//! `20250303` for `20260303` by editing one TOML row, not this file.
//!
//! Renderers in `render/*_validate.rs` format each mode for their target
//! language.

use std::collections::HashSet;

use super::helpers::{
    builder_params, is_simple_list_endpoint, is_streaming_endpoint, method_params,
};
use super::model::{GeneratedEndpoint, GeneratedParam, TestFixtures};

// ───────────────────────── Multi-mode parameter matrix ──────────────────────
//
// `TestMode` captures one (parameter-shape × tier) cell that the live
// validator should exercise. Modes are derived per endpoint by
// [`test_modes_for`] from the endpoint's wire shape — list endpoints get one
// mode, ContractSpec endpoints get the full wildcard cross-product, and so
// on. Each mode carries language-agnostic string args so per-language
// renderers (CLI / Python / Go / C++) can format them appropriately.

/// One parameter-mode test cell to run against a live endpoint.
#[derive(Debug, Clone)]
pub(super) struct TestMode {
    /// Mode identifier (`concrete`, `bulk_chain`, `iso_date`, ...). Used in
    /// validator output so failures point at a specific cell.
    pub(super) name: String,
    /// One-sentence description of what this cell proves. Emitted as a
    /// comment in the generated validators, as a field in the per-cell JSON
    /// artifact, and shown in `validate_agreement.py` disagreement output so
    /// a reviewer reading a FAIL immediately sees which feature broke.
    pub(super) rationale: &'static str,
    /// Method-call positional arguments, in declaration order. Each entry is
    /// the language-agnostic string value (e.g. `"SPY"`, `"20260417"`,
    /// `"*"`). `Symbols`-typed params are still rendered as a single string
    /// here — per-language renderers wrap them in the target list literal.
    pub(super) args: Vec<String>,
    /// Highest subscription tier this mode requires (`"free"`, `"value"`,
    /// `"standard"`, `"professional"`). The validator skips the cell with a
    /// clear `SKIP: tier<X>` line if the account tier is below.
    pub(super) min_tier: &'static str,
    /// Outcome the validator should expect.
    ///   - `non_empty`: a normal successful call (rows or "no data" both PASS)
    ///   - `empty_ok`: a successful call that may legitimately return zero rows
    ///   - `error_permission`: tier/permission errors are PASS, real errors FAIL
    pub(super) expect: &'static str,
    /// Optional (builder-bound) parameter overrides to apply on this mode.
    /// Each entry is `(param_name, representative_value)`. Rendered per
    /// language: Python kwargs, Go `thetadatadx.WithXxx()` opts, C++
    /// `EndpointRequestOptions{}.with_xxx()`. CLI skips these (positional
    /// clap args don't support targeted optional injection); see PR #291.
    pub(super) builder_overrides: Vec<(String, String)>,
}

/// One-sentence rationale describing what each mode proves.
///
/// Surfaces in three places:
/// * inline `# rationale:` comment in the generated validator scripts so a
///   reader of `scripts/validate_python.py` sees per-cell intent;
/// * `rationale` field in the per-cell JSON artifact; and
/// * `validate_agreement.py` failure output, so a FAIL line carries the
///   feature description not just the mode name.
///
/// Kept ≤100 chars so it fits on one line in failure tables. Per-optional
/// `with_<name>` modes are produced from
/// [`with_optional_rationale`] which builds a string at generator runtime.
fn rationale_for_mode(name: &str) -> &'static str {
    match name {
        "basic" => "list/calendar/rate baseline call — no parameter variation",
        "concrete" => "required params set, no optionals — baseline wire path",
        "concrete_iso" => {
            "expiration in YYYY-MM-DD form — tests ISO-date canonicalization to YYYYMMDD"
        }
        "all_strikes_one_exp" => {
            "strike=* — collapses to proto-unset ContractSpec.strike (server default)"
        }
        "all_exps_one_strike" => "expiration=* — sent as literal `*` on the wire (server fan-out)",
        "bulk_chain" => "expiration=* + strike=* + right=both — tests full-chain server mode",
        "legacy_zero_wildcard" => {
            "expiration=0 → wire `*`; strike=0 + right=both → proto-unset — legacy-input compat"
        }
        "with_intraday_window" => "start_time + end_time pair — intraday window optional wiring",
        "with_date_range" => "start_date + end_date pair — date range optional wiring",
        "all_optionals" => "every applicable optional set at once — proves multi-optional wiring",
        _ => panic!(
            "rationale_for_mode: unknown mode '{name}'; add a rationale to TestMode generation"
        ),
    }
}

/// Build a one-sentence rationale string for a `with_<param>` mode at
/// generator runtime. The literal value is threaded in from the
/// [`optional_fixture_value`] table so the two can never drift.
fn with_optional_rationale(param_name: &str, literal: &str) -> String {
    let label = match param_name {
        "max_dte" | "strike_range" | "min_time" | "exclusive" | "start_time" | "end_time"
        | "start_date" | "end_date" => "optional filter wiring",
        "venue" => "optional venue selector wiring",
        "annual_dividend" | "rate_type" | "rate_value" | "stock_price" => {
            "optional Greeks-input wiring"
        }
        "version" => "optional Greeks-version selector wiring",
        "use_market_value" | "underlyer_use_nbbo" => "optional flag wiring",
        _ => panic!(
            "with_optional_rationale: unknown optional param '{param_name}'; \
             add a rationale class before adding a new optional fixture"
        ),
    };
    format!("{param_name}={literal} {label}")
}

/// Minimum subscription tier each endpoint requires.
///
/// Derived at generator-run-time from the pinned upstream OpenAPI snapshot at
/// `scripts/upstream_openapi.yaml` (see [`super::upstream_openapi`]), keyed
/// on the endpoint's `operationId`. Upstream is the sole source of truth for
/// `x-min-subscription`, so docs-site `<TierBadge>` and this function agree
/// as long as the snapshot is fresh.
///
/// Four kinds of endpoints don't have an upstream entry and fall back to a
/// tiny override table ([`sdk_only_min_tier`]): streaming RPCs (FPSS, not
/// MDDS), SDK-private endpoints like `interest_rate_history_eod`, and
/// SDK-only synthetic clones like `stock_history_ohlc_range`.
fn endpoint_min_tier(name: &str) -> &'static str {
    if let Some(tier) = sdk_only_min_tier(name) {
        return tier;
    }
    let spec = super::super::upstream_openapi::UpstreamOpenApi::load();
    let endpoint = spec.endpoint(name).unwrap_or_else(|| {
        panic!(
            "endpoint '{name}' is missing from the upstream OpenAPI snapshot \
             at scripts/upstream_openapi.yaml; if this is a new endpoint, add \
             it as an SDK-only override in `sdk_only_min_tier`, or refresh the \
             snapshot with `python3 scripts/check_tier_badges.py --refresh-snapshot`."
        )
    });
    match endpoint.min_subscription.as_str() {
        "free" => "free",
        "value" => "value",
        "standard" => "standard",
        "professional" => "professional",
        other => panic!(
            "endpoint '{name}': upstream min-subscription '{other}' is not a known tier. \
             Expected one of free/value/standard/professional."
        ),
    }
}

/// Minimum-tier override for endpoints that aren't in the upstream OpenAPI spec.
///
/// Returns `None` for every endpoint that upstream documents — those flow
/// through [`endpoint_min_tier`]'s snapshot lookup.
fn sdk_only_min_tier(name: &str) -> Option<&'static str> {
    Some(match name {
        // Streaming endpoints (FPSS, covered by scripts/fpss_smoke.py, not the
        // live matrix validator). The value here is still used by
        // `test_modes_for` for display-only `min_tier` on test cells, but the
        // streaming surface is excluded from the matrix anyway.
        "stock_history_trade_stream"
        | "stock_history_quote_stream"
        | "option_history_trade_stream"
        | "option_history_quote_stream" => "standard",
        // Synthetic clone sharing a wire RPC with `stock_history_ohlc`.
        "stock_history_ohlc_range" => "value",
        // SDK-only endpoint not documented upstream (FRED-backed, thetadatadx-local).
        "interest_rate_history_eod" => "free",
        _ => return None,
    })
}

/// Resolve the anchor symbol fixture for an endpoint's category. Panics on
/// missing rows so a category without a fixture fails loudly at generator
/// time rather than silently producing empty cells. Pre-flight check in
/// `parser.rs::validate_test_fixtures` guarantees this never trips for
/// known-good TOML.
fn category_symbol<'a>(fixtures: &'a TestFixtures, category: &str) -> &'a str {
    fixtures
        .category_symbol
        .get(category)
        .map(String::as_str)
        .unwrap_or_else(|| {
            panic!(
                "test_fixtures.category_symbol is missing an entry for category '{category}' in \
                 endpoint_surface.toml"
            )
        })
}

/// Render the language-agnostic value for a method-call parameter at a
/// concrete fixture (no wildcards, compact dates).
///
/// Resolution order:
///   1. `test_fixtures.concrete_overrides[param.name]` — per-name overrides
///      (e.g. compressed `end_date` keeping bulk cells under the 60s
///      per-cell timeout; see issue #290).
///   2. `test_fixtures.category_symbol[endpoint.category]` — for
///      `Symbol`/`Symbols` params.
///   3. `test_fixtures.concrete_by_type[param.param_type]` — default
///      representative value per wire type.
fn concrete_value(
    endpoint: &GeneratedEndpoint,
    param: &GeneratedParam,
    fixtures: &TestFixtures,
) -> String {
    if let Some(value) = fixtures.concrete_overrides.get(&param.name) {
        return value.clone();
    }
    match param.param_type.as_str() {
        "Symbol" | "Symbols" => category_symbol(fixtures, &endpoint.category).to_string(),
        other => fixtures
            .concrete_by_type
            .get(other)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "test_fixtures.concrete_by_type is missing an entry for param_type '{other}' \
                     (required by '{}.{}'); add a row in endpoint_surface.toml",
                    endpoint.name, param.name
                )
            }),
    }
}

/// Build the args vector for a concrete (no-wildcard) call.
fn concrete_args(endpoint: &GeneratedEndpoint, fixtures: &TestFixtures) -> Vec<String> {
    method_params(endpoint)
        .iter()
        .map(|param| concrete_value(endpoint, param, fixtures))
        .collect()
}

/// Build args for a named mode whose overrides live under
/// `[test_fixtures.mode_overrides.<mode_name>]`. Any param not listed in the
/// TOML block falls back to its concrete fixture.
fn args_for_mode(
    endpoint: &GeneratedEndpoint,
    fixtures: &TestFixtures,
    mode_name: &str,
) -> Vec<String> {
    let overrides = fixtures.mode_overrides.get(mode_name).unwrap_or_else(|| {
        panic!(
            "test_fixtures.mode_overrides is missing an entry for mode '{mode_name}'; \
             add one in endpoint_surface.toml"
        )
    });
    method_params(endpoint)
        .iter()
        .map(|param| match overrides.get(&param.name) {
            Some(value) => value.clone(),
            None => concrete_value(endpoint, param, fixtures),
        })
        .collect()
}

/// Whether the endpoint's method-call params include the full ContractSpec
/// quartet (symbol, expiration, strike, right). Drives wildcard mode
/// generation for option snapshot / history / at-time endpoints.
fn has_full_contract_spec(endpoint: &GeneratedEndpoint) -> bool {
    let names: HashSet<&str> = method_params(endpoint)
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    names.contains("symbol")
        && names.contains("expiration")
        && names.contains("strike")
        && names.contains("right")
}

/// Whether an option endpoint accepts `expiration=*` at the v3 server.
///
/// Derived from the pinned upstream snapshot
/// (`scripts/upstream_openapi.yaml`): upstream binds endpoints that reject
/// wildcards to its `expiration_no_star` component parameter (they return
/// `InvalidArgument -- Cannot specify '*' for the date` if we send `*`),
/// and wildcard-accepting endpoints to `expiration`. See
/// [`super::upstream_openapi::UpstreamEndpoint::supports_expiration_wildcard`].
///
/// Endpoints absent from upstream (streaming, SDK-only clones) fall back to
/// `true` — they don't participate in the wildcard matrix anyway (streaming
/// is skipped upstream of this call, and the SDK-only endpoints don't take
/// an expiration parameter).
fn endpoint_supports_expiration_wildcard(name: &str) -> bool {
    let spec = super::super::upstream_openapi::UpstreamOpenApi::load();
    spec.endpoint(name)
        .map(|endpoint| endpoint.supports_expiration_wildcard)
        .unwrap_or(true)
}

/// Compute the comprehensive mode set for a given endpoint.
///
/// The taxonomy:
///   * **List** endpoints (`*_list_*`): one `basic` mode. Server rejects
///     `*` for `expiration` here, so we don't emit a wildcard variant.
///   * **Stock / index snapshot or history** (no ContractSpec): one
///     `concrete` mode plus an `iso_date` mode where dates are involved.
///   * **Option ContractSpec** endpoints: the full cross-product —
///     `concrete`, `concrete_iso`, `all_strikes_one_exp`,
///     `all_exps_one_strike`, `bulk_chain`, `legacy_zero_wildcard`.
///   * **Calendar / rate**: one mode each.
///
/// Stream endpoints are covered by `scripts/fpss_smoke.py` /
/// `scripts/fpss_soak.py` and intentionally skipped here.
pub(super) fn test_modes_for(
    endpoint: &GeneratedEndpoint,
    fixtures: &TestFixtures,
) -> Vec<TestMode> {
    if is_streaming_endpoint(endpoint) {
        return Vec::new();
    }
    let endpoint_tier = endpoint_min_tier(&endpoint.name);

    // ── List endpoints: one mode, no wildcard expiration (server rejects). ──
    if is_simple_list_endpoint(endpoint) {
        return collapse_redundant_wires(
            endpoint,
            append_optional_modes(
                endpoint,
                fixtures,
                endpoint_tier,
                vec![TestMode {
                    name: "basic".to_string(),
                    rationale: rationale_for_mode("basic"),
                    args: concrete_args(endpoint, fixtures),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                    builder_overrides: Vec::new(),
                }],
            ),
        );
    }

    // ── Calendar / rate: one mode. ──────────────────────────────────────────
    if matches!(endpoint.category.as_str(), "calendar" | "rate") {
        return collapse_redundant_wires(
            endpoint,
            append_optional_modes(
                endpoint,
                fixtures,
                endpoint_tier,
                vec![TestMode {
                    name: "basic".to_string(),
                    rationale: rationale_for_mode("basic"),
                    args: concrete_args(endpoint, fixtures),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                    builder_overrides: Vec::new(),
                }],
            ),
        );
    }

    // ── Option ContractSpec: full wildcard cross-product, except where the
    // v3 server explicitly disallows `expiration=*` on an endpoint (it binds
    // that endpoint to the `expiration_no_star` parameter in upstream's
    // openapiv3.yaml, and returns `InvalidArgument -- Cannot specify '*' for
    // the date` if we pass it). Those endpoints get only the concrete +
    // ISO-dashed fixtures plus the `all_strikes_one_exp` mode, which uses a
    // concrete expiration.
    if has_full_contract_spec(endpoint) {
        let mut modes = vec![
            TestMode {
                name: "concrete".to_string(),
                rationale: rationale_for_mode("concrete"),
                args: concrete_args(endpoint, fixtures),
                min_tier: endpoint_tier,
                expect: "non_empty",
                builder_overrides: Vec::new(),
            },
            TestMode {
                name: "concrete_iso".to_string(),
                rationale: rationale_for_mode("concrete_iso"),
                args: args_for_mode(endpoint, fixtures, "concrete_iso"),
                min_tier: endpoint_tier,
                expect: "non_empty",
                builder_overrides: Vec::new(),
            },
            TestMode {
                name: "all_strikes_one_exp".to_string(),
                rationale: rationale_for_mode("all_strikes_one_exp"),
                args: args_for_mode(endpoint, fixtures, "all_strikes_one_exp"),
                min_tier: endpoint_tier,
                expect: "non_empty",
                builder_overrides: Vec::new(),
            },
        ];
        if endpoint_supports_expiration_wildcard(&endpoint.name) {
            modes.extend([
                TestMode {
                    name: "all_exps_one_strike".to_string(),
                    rationale: rationale_for_mode("all_exps_one_strike"),
                    args: args_for_mode(endpoint, fixtures, "all_exps_one_strike"),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                    builder_overrides: Vec::new(),
                },
                TestMode {
                    name: "bulk_chain".to_string(),
                    rationale: rationale_for_mode("bulk_chain"),
                    args: args_for_mode(endpoint, fixtures, "bulk_chain"),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                    builder_overrides: Vec::new(),
                },
                TestMode {
                    name: "legacy_zero_wildcard".to_string(),
                    rationale: rationale_for_mode("legacy_zero_wildcard"),
                    args: args_for_mode(endpoint, fixtures, "legacy_zero_wildcard"),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                    builder_overrides: Vec::new(),
                },
            ]);
        }
        modes.dedup_by(|a, b| a.args == b.args && a.name == b.name);
        return collapse_redundant_wires(
            endpoint,
            append_optional_modes(endpoint, fixtures, endpoint_tier, modes),
        );
    }

    // ── Stock / index / non-ContractSpec endpoints. ─────────────────────────
    //
    // We deliberately do NOT emit an `iso_date` mode for stock/index
    // endpoints with `start_date`/`end_date`. Those parameters are typed as
    // `Date` in the SDK, and `validate::validate_date` is strict
    // `YYYYMMDD` only — ISO-dashed acceptance is scoped to `Expiration`
    // (see PR #284). Adding an `iso_date` cell here would test behavior the
    // SDK contract intentionally does not support, so it would always fail.
    collapse_redundant_wires(
        endpoint,
        append_optional_modes(
            endpoint,
            fixtures,
            endpoint_tier,
            vec![TestMode {
                name: "concrete".to_string(),
                rationale: rationale_for_mode("concrete"),
                args: concrete_args(endpoint, fixtures),
                min_tier: endpoint_tier,
                expect: "non_empty",
                builder_overrides: Vec::new(),
            }],
        ),
    )
}

/// Look up the representative value for a builder-bound optional parameter
/// from `[test_fixtures.optional_defaults]`. Returns `None` if the TOML has
/// no entry; the pre-flight check in `parser.rs::validate_test_fixtures`
/// rejects every endpoint whose builder param lacks an entry, so the `None`
/// branch is a defense in depth — only fires if the validator is bypassed.
fn optional_fixture_value<'a>(fixtures: &'a TestFixtures, param_name: &str) -> Option<&'a str> {
    fixtures
        .optional_defaults
        .get(param_name)
        .map(String::as_str)
}

/// Same as [`optional_fixture_value`] but for paired modes
/// (`with_intraday_window`, `with_date_range`) where the fixture row is
/// guaranteed by the design — both halves of the pair have to have
/// fixtures because the SDK rejects the half-set wire shape. Panics with
/// full context (endpoint, mode, key) so a missing row is debuggable
/// without `RUST_BACKTRACE=1`.
fn paired_optional_fixture(
    fixtures: &TestFixtures,
    endpoint: &GeneratedEndpoint,
    mode_name: &str,
    param_name: &str,
) -> String {
    optional_fixture_value(fixtures, param_name)
        .unwrap_or_else(|| {
            panic!(
                "test_fixtures.optional_defaults is missing key '{param_name}' (needed for \
                 {endpoint}.{mode_name}); add a row in endpoint_surface.toml. Note: \
                 parser.rs::validate_test_fixtures should have caught this earlier — if you see \
                 this panic, the validator was bypassed.",
                endpoint = endpoint.name,
            )
        })
        .to_string()
}

/// Expand the baseline (wildcard/concrete) modes with one `with_<name>` cell
/// per optional param the endpoint accepts, plus one `all_optionals` cell
/// that sets every applicable optional at once.
///
/// Design decisions:
/// * Start-time/end-time are a single `with_intraday_window` mode rather than
///   two independent cells. The SDK accepts them independently but sending
///   only one half makes the time window implicit which the server rejects.
/// * Start-date/end-date are a single `with_date_range` mode, and only
///   emitted if the endpoint has BOTH optional params. Sending only one
///   half is an invalid argument on the wire.
/// * The rest pair 1:1 with a single `with_<param_name>` mode.
/// * The `all_optionals` mode collects every applicable representative value
///   into one call — proves the SDK can serialize them all together.
///
/// No cell is ever deduplicated against another by wire shape: even if two
/// generated modes would hit the server with identical bytes, we keep both
/// so the cross-language agreement check can detect SDKs that diverge
/// *only* on that cell. See PR #291 / issue #290.
fn append_optional_modes(
    endpoint: &GeneratedEndpoint,
    fixtures: &TestFixtures,
    endpoint_tier: &'static str,
    mut modes: Vec<TestMode>,
) -> Vec<TestMode> {
    let optional_names: Vec<String> = builder_params(endpoint)
        .iter()
        .map(|param| param.name.clone())
        .collect();
    if optional_names.is_empty() {
        return modes;
    }

    // Single compound modes: when both halves of a pair are present, emit a
    // SINGLE cell that sets both. Otherwise skip (sending only one half is
    // invalid on the wire for this SDK).
    let has_param = |needle: &str| optional_names.iter().any(|n| n == needle);
    let mut handled: std::collections::HashSet<String> = std::collections::HashSet::new();

    // intraday window (start_time + end_time).
    if has_param("start_time") && has_param("end_time") {
        let overrides = vec![
            (
                "start_time".to_string(),
                paired_optional_fixture(fixtures, endpoint, "with_intraday_window", "start_time"),
            ),
            (
                "end_time".to_string(),
                paired_optional_fixture(fixtures, endpoint, "with_intraday_window", "end_time"),
            ),
        ];
        modes.push(TestMode {
            name: "with_intraday_window".to_string(),
            rationale: rationale_for_mode("with_intraday_window"),
            args: concrete_args(endpoint, fixtures),
            min_tier: endpoint_tier,
            expect: "non_empty",
            builder_overrides: overrides,
        });
        handled.insert("start_time".into());
        handled.insert("end_time".into());
    }

    // date range (start_date + end_date), only when BOTH are optional on
    // this endpoint. (They are required args on some endpoints; those skip
    // this compound mode.)
    if has_param("start_date") && has_param("end_date") {
        let overrides = vec![
            (
                "start_date".to_string(),
                paired_optional_fixture(fixtures, endpoint, "with_date_range", "start_date"),
            ),
            (
                "end_date".to_string(),
                paired_optional_fixture(fixtures, endpoint, "with_date_range", "end_date"),
            ),
        ];
        modes.push(TestMode {
            name: "with_date_range".to_string(),
            rationale: rationale_for_mode("with_date_range"),
            args: concrete_args(endpoint, fixtures),
            min_tier: endpoint_tier,
            expect: "non_empty",
            builder_overrides: overrides,
        });
        handled.insert("start_date".into());
        handled.insert("end_date".into());
    }

    // Per-parameter `with_<name>` modes for everything else. Every entry in
    // `optional_names` is guaranteed to have an `optional_defaults` row by
    // `parser.rs::validate_test_fixtures`, so a missing fixture here is a
    // bypassed-validator bug, not a routine "skip the cell" path.
    for param_name in &optional_names {
        if handled.contains(param_name) {
            continue;
        }
        let value = optional_fixture_value(fixtures, param_name).unwrap_or_else(|| {
            panic!(
                "test_fixtures.optional_defaults is missing key '{param_name}' (needed for \
                 {endpoint}.with_{param_name}); add a row in endpoint_surface.toml.",
                endpoint = endpoint.name
            )
        });
        // Rationale carries the exact fixture literal so the cell's text
        // can never drift from `optional_fixture_value`. `String` is
        // promoted to `&'static str` via `Box::leak` — generator runs once
        // per build, so the allocation is effectively one-time.
        let rationale: &'static str =
            Box::leak(with_optional_rationale(param_name, value).into_boxed_str());
        modes.push(TestMode {
            name: format!("with_{param_name}"),
            rationale,
            args: concrete_args(endpoint, fixtures),
            min_tier: endpoint_tier,
            expect: "non_empty",
            builder_overrides: vec![(param_name.clone(), value.to_string())],
        });
    }

    // `all_optionals` mode — set every applicable optional at once. Uses
    // the compound fixtures for paired params (single intraday window, single
    // date range) so the compound cell and this one agree on wire shape.
    // Same fail-fast contract as the `with_<name>` loop above.
    let mut all_overrides: Vec<(String, String)> = Vec::new();
    for param_name in &optional_names {
        let value = optional_fixture_value(fixtures, param_name).unwrap_or_else(|| {
            panic!(
                "test_fixtures.optional_defaults is missing key '{param_name}' (needed for \
                 {endpoint}.all_optionals); add a row in endpoint_surface.toml.",
                endpoint = endpoint.name
            )
        });
        all_overrides.push((param_name.clone(), value.to_string()));
    }
    if !all_overrides.is_empty() {
        modes.push(TestMode {
            name: "all_optionals".to_string(),
            rationale: rationale_for_mode("all_optionals"),
            args: concrete_args(endpoint, fixtures),
            min_tier: endpoint_tier,
            expect: "non_empty",
            builder_overrides: all_overrides,
        });
    }

    modes
}

/// Approximate the wire-level canonicalization performed by the runtime
/// client, so two modes whose proto messages differ only by SDK-level
/// sentinel translation are detected as duplicates here.
///
/// Mirrors three runtime transformations:
/// * `expiration`: `"0"` → `"*"`; ISO-dashed → compact `YYYYMMDD`.
///   See `crates/thetadatadx/src/direct.rs:84` (`normalize_expiration`).
/// * `strike`:     `""` / `"0"` / `"*"` → proto-unset.
///   See `crates/thetadatadx/src/direct.rs:100` (`wire_strike_opt`).
/// * `right`:      `"*"` / `"both"` (any case) → proto-unset.
///   See `crates/thetadatadx/src/direct.rs:118` (`wire_right_opt`).
///
/// The canonical token is the string we expect the server to see. `None`
/// means "proto field unset" — represented here as the sentinel
/// `"<unset>"` so the grouping key stays a plain `String`. Any divergence
/// between this pass and the runtime normalization is a bug (the audit
/// stops being load-bearing); the runtime side has unit tests in
/// `crates/thetadatadx/src/direct.rs`, the intent here is to stay in sync
/// with those exact rules.
fn canonicalize_wire_arg(param_name: &str, value: &str) -> String {
    const UNSET: &str = "<unset>";
    match param_name {
        "expiration" => match value {
            "0" => "*".to_string(),
            v if is_build_time_iso_date(v) => v.replace('-', ""),
            other => other.to_string(),
        },
        "strike" => {
            if value.is_empty() || value == "*" || value == "0" {
                UNSET.to_string()
            } else {
                value.to_string()
            }
        }
        "right" => match value.to_ascii_lowercase().as_str() {
            "*" | "both" => UNSET.to_string(),
            "c" | "call" => "call".to_string(),
            "p" | "put" => "put".to_string(),
            other => other.to_string(),
        },
        _ => value.to_string(),
    }
}

/// Build-time mirror of `crates/thetadatadx/src/validate.rs::is_iso_date`.
/// Kept here to avoid taking a build dependency on the runtime crate.
fn is_build_time_iso_date(value: &str) -> bool {
    let mut parts = value.splitn(3, '-');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(y), Some(m), Some(d), None)
            if y.len() == 4
                && m.len() == 2
                && d.len() == 2
                && y.bytes().all(|b| b.is_ascii_digit())
                && m.bytes().all(|b| b.is_ascii_digit())
                && d.bytes().all(|b| b.is_ascii_digit())
    )
}

/// Collapse cells whose post-canonicalization wire shape is identical down
/// to a single canonical cell.
///
/// The signature combines:
/// * positional args run through [`canonicalize_wire_arg`] per-param name,
///   which mirrors the runtime's `expiration`/`strike`/`right` rewriting;
/// * builder-override pairs, also canonicalized, sorted, **and** with stock
///   endpoints' `"venue" → "nqb"` default synthesized in whenever the
///   endpoint's `venue` param is absent from the mode's overrides.
///   See `render/direct.rs:433` for the runtime default.
///
/// Two modes with equal signatures will marshal byte-identical proto
/// messages, so collapsing them removes only redundant runtime cost.
///
/// Collapsing rules:
/// * Group modes by their canonicalized signature.
/// * Within each group keep the lowest-index entry, so canonical modes like
///   `concrete`/`bulk_chain` win over a later `with_<name>` whose override
///   happened to match an existing fixture.
/// * Append the names of collapsed siblings to the kept cell's rationale as
///   `(also covers: a, b)` so the downstream agreement output makes the
///   roll-up visible.
///
/// This is the audit step from W6: before it, cells with overlapping wire
/// shapes co-existed silently. After it, no two emitted cells for a given
/// endpoint share a wire shape; siblings are documented inline.
fn collapse_redundant_wires(endpoint: &GeneratedEndpoint, modes: Vec<TestMode>) -> Vec<TestMode> {
    use std::collections::BTreeMap;
    // Canonicalized wire signature: positional args + sorted override pairs,
    // with runtime-equivalent normalization applied to both sides.
    type WireSignature = (Vec<String>, Vec<(String, String)>);

    let method_param_names: Vec<String> = method_params(endpoint)
        .iter()
        .map(|param| param.name.clone())
        .collect();
    let has_stock_venue_default = endpoint.category == "stock"
        && builder_params(endpoint)
            .iter()
            .any(|param| param.name == "venue");

    let canonical_overrides = |overrides: &[(String, String)]| -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = overrides
            .iter()
            .map(|(k, v)| (k.clone(), canonicalize_wire_arg(k, v)))
            .collect();
        // Synthesize the stock-endpoint `venue=nqb` default when the mode
        // doesn't override it: the runtime fills this in at request-build
        // time (`render/direct.rs:433`), so omitting it here would make
        // `concrete` and `with_venue` look like distinct wire shapes
        // despite producing identical proto messages.
        if has_stock_venue_default && !pairs.iter().any(|(k, _)| k == "venue") {
            pairs.push(("venue".to_string(), "nqb".to_string()));
        }
        pairs.sort();
        pairs
    };

    let canonical_args = |args: &[String]| -> Vec<String> {
        args.iter()
            .enumerate()
            .map(|(i, v)| {
                let name = method_param_names
                    .get(i)
                    .map(String::as_str)
                    .unwrap_or_default();
                canonicalize_wire_arg(name, v)
            })
            .collect()
    };

    let mut buckets: BTreeMap<WireSignature, Vec<usize>> = BTreeMap::new();
    for (idx, mode) in modes.iter().enumerate() {
        let key = (
            canonical_args(&mode.args),
            canonical_overrides(&mode.builder_overrides),
        );
        buckets.entry(key).or_default().push(idx);
    }
    let mut keep_idx: Vec<(usize, Vec<String>)> = buckets
        .values()
        .map(|indices| {
            let canonical = *indices.iter().min().unwrap();
            let collapsed: Vec<String> = indices
                .iter()
                .filter(|&&i| i != canonical)
                .map(|&i| modes[i].name.clone())
                .collect();
            (canonical, collapsed)
        })
        .collect();
    keep_idx.sort_by_key(|(idx, _)| *idx);

    let mut out = Vec::with_capacity(keep_idx.len());
    for (idx, collapsed) in keep_idx {
        let mut mode = modes[idx].clone();
        if !collapsed.is_empty() {
            // Build the appended rationale at generator runtime, then leak it
            // to satisfy the `&'static str` field — generator runs once per
            // build so the lifetime cost is one allocation per collapsed
            // group, never freed across the build's lifetime. Kept under
            // 200 chars to stay readable in the agreement table.
            let extended = format!("{} (also covers: {})", mode.rationale, collapsed.join(", "));
            mode.rationale = Box::leak(extended.into_boxed_str());
        }
        out.push(mode);
    }
    out
}
