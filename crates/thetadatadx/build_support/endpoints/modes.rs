//! Parameter-mode matrix used by the live validator renderers.
//!
//! [`TestMode`] captures one (parameter-shape × tier) cell to exercise against
//! a real account. Modes are derived per-endpoint by [`test_modes_for`] from
//! the endpoint's wire shape: list endpoints get one mode, option ContractSpec
//! endpoints get the full wildcard cross-product, and so on. Tier information
//! flows from the pinned upstream OpenAPI snapshot.
//!
//! Renderers in `render/*_validate.rs` format each mode for their target
//! language.

use std::collections::HashSet;

use super::helpers::{
    builder_params, is_simple_list_endpoint, is_streaming_endpoint, method_params,
    validation_symbol,
};
use super::model::{GeneratedEndpoint, GeneratedParam};

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
            "strike=* on a supported endpoint — exercises strike wildcard wire-unset"
        }
        "all_exps_one_strike" => "expiration=* — exercises expiration wildcard wire-unset",
        "bulk_chain" => "expiration=* + strike=* + right=both — tests full-chain server mode",
        "legacy_zero_wildcard" => {
            "expiration=0 + strike=0 → translated to * on wire — backward compat"
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
/// generator runtime. Uses a small static table to keep entries succinct.
fn with_optional_rationale(param_name: &str) -> &'static str {
    match param_name {
        "max_dte" => "max_dte=30 optional filter wiring",
        "strike_range" => "strike_range=10 optional filter wiring",
        "min_time" => "min_time=09:45:00 optional filter wiring",
        "venue" => "venue=nqb optional venue selector wiring",
        "exclusive" => "exclusive=true optional filter wiring",
        "annual_dividend" => "annual_dividend=0.015 optional Greeks-input wiring",
        "rate_type" => "rate_type=sofr optional Greeks-input wiring",
        "rate_value" => "rate_value=0.05 optional Greeks-input wiring",
        "stock_price" => "stock_price=150 optional Greeks-input wiring",
        "version" => "version=dg3 optional Greeks-version selector wiring",
        "use_market_value" => "use_market_value=true optional flag wiring",
        "underlyer_use_nbbo" => "underlyer_use_nbbo=true optional flag wiring",
        // start_time/end_time/start_date/end_date are exercised via the paired
        // `with_intraday_window` / `with_date_range` modes; if a future
        // endpoint accepts only one half they would fall through to here.
        "start_time" => "start_time=09:30:00 optional filter wiring",
        "end_time" => "end_time=10:00:00 optional filter wiring",
        "start_date" => "start_date=20250303 optional filter wiring",
        "end_date" => "end_date=20250303 optional filter wiring",
        _ => panic!(
            "with_optional_rationale: unknown optional param '{param_name}'; \
             add a rationale before adding a new optional fixture"
        ),
    }
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

/// Render the language-agnostic value for a method-call parameter at a
/// concrete fixture (no wildcards, compact dates).
///
/// Date range is deliberately narrowed to a single day (20250303 = Mon)
/// to keep the matrix within the ~10-minute live-run budget even when
/// bulk-expiration / bulk-strike cells stack multiple 60s timeouts.
/// Widening this back to a multi-day window is the first lever to pull
/// if a cell's volume coverage matters more than runtime. See #290.
fn concrete_value(endpoint: &GeneratedEndpoint, param: &GeneratedParam) -> String {
    if param.name == "end_date" {
        return "20250303".into();
    }
    match param.param_type.as_str() {
        "Symbol" | "Symbols" => validation_symbol(endpoint).into(),
        "Date" => "20250303".into(),
        "Expiration" => "20250321".into(),
        "Strike" => "570".into(),
        "Right" => "C".into(),
        "Interval" => "60000".into(),
        "RequestType" => "TRADE".into(),
        "Year" => "2025".into(),
        "Str" => "12:00:00.000".into(),
        other => panic!("concrete_value: unsupported param type {other}"),
    }
}

/// Build the args vector for a concrete (no-wildcard) call.
fn concrete_args(endpoint: &GeneratedEndpoint) -> Vec<String> {
    method_params(endpoint)
        .iter()
        .map(|param| concrete_value(endpoint, param))
        .collect()
}

/// Build args for a mode that overrides specific parameter names with given
/// values — everything else falls back to [`concrete_value`].
fn args_with_overrides(
    endpoint: &GeneratedEndpoint,
    overrides: &[(&'static str, &str)],
) -> Vec<String> {
    method_params(endpoint)
        .iter()
        .map(|param| {
            overrides
                .iter()
                .find_map(|(name, value)| (*name == param.name).then(|| (*value).to_string()))
                .unwrap_or_else(|| concrete_value(endpoint, param))
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
pub(super) fn test_modes_for(endpoint: &GeneratedEndpoint) -> Vec<TestMode> {
    if is_streaming_endpoint(endpoint) {
        return Vec::new();
    }
    let endpoint_tier = endpoint_min_tier(&endpoint.name);

    // ── List endpoints: one mode, no wildcard expiration (server rejects). ──
    if is_simple_list_endpoint(endpoint) {
        return collapse_redundant_wires(append_optional_modes(
            endpoint,
            endpoint_tier,
            vec![TestMode {
                name: "basic".to_string(),
                rationale: rationale_for_mode("basic"),
                args: concrete_args(endpoint),
                min_tier: endpoint_tier,
                expect: "non_empty",
                builder_overrides: Vec::new(),
            }],
        ));
    }

    // ── Calendar / rate: one mode. ──────────────────────────────────────────
    if matches!(endpoint.category.as_str(), "calendar" | "rate") {
        return collapse_redundant_wires(append_optional_modes(
            endpoint,
            endpoint_tier,
            vec![TestMode {
                name: "basic".to_string(),
                rationale: rationale_for_mode("basic"),
                args: concrete_args(endpoint),
                min_tier: endpoint_tier,
                expect: "non_empty",
                builder_overrides: Vec::new(),
            }],
        ));
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
                args: concrete_args(endpoint),
                min_tier: endpoint_tier,
                expect: "non_empty",
                builder_overrides: Vec::new(),
            },
            TestMode {
                name: "concrete_iso".to_string(),
                rationale: rationale_for_mode("concrete_iso"),
                args: args_with_overrides(endpoint, &[("expiration", "2025-03-21")]),
                min_tier: endpoint_tier,
                expect: "non_empty",
                builder_overrides: Vec::new(),
            },
            TestMode {
                name: "all_strikes_one_exp".to_string(),
                rationale: rationale_for_mode("all_strikes_one_exp"),
                args: args_with_overrides(endpoint, &[("strike", "*"), ("right", "both")]),
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
                    args: args_with_overrides(endpoint, &[("expiration", "*"), ("right", "both")]),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                    builder_overrides: Vec::new(),
                },
                TestMode {
                    name: "bulk_chain".to_string(),
                    rationale: rationale_for_mode("bulk_chain"),
                    args: args_with_overrides(
                        endpoint,
                        &[("expiration", "*"), ("strike", "*"), ("right", "both")],
                    ),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                    builder_overrides: Vec::new(),
                },
                TestMode {
                    name: "legacy_zero_wildcard".to_string(),
                    rationale: rationale_for_mode("legacy_zero_wildcard"),
                    args: args_with_overrides(
                        endpoint,
                        &[("expiration", "0"), ("strike", "0"), ("right", "both")],
                    ),
                    min_tier: endpoint_tier,
                    expect: "non_empty",
                    builder_overrides: Vec::new(),
                },
            ]);
        }
        modes.dedup_by(|a, b| a.args == b.args && a.name == b.name);
        return collapse_redundant_wires(append_optional_modes(endpoint, endpoint_tier, modes));
    }

    // ── Stock / index / non-ContractSpec endpoints. ─────────────────────────
    //
    // We deliberately do NOT emit an `iso_date` mode for stock/index
    // endpoints with `start_date`/`end_date`. Those parameters are typed as
    // `Date` in the SDK, and `validate::validate_date` is strict
    // `YYYYMMDD` only — ISO-dashed acceptance is scoped to `Expiration`
    // (see PR #284). Adding an `iso_date` cell here would test behavior the
    // SDK contract intentionally does not support, so it would always fail.
    collapse_redundant_wires(append_optional_modes(
        endpoint,
        endpoint_tier,
        vec![TestMode {
            name: "concrete".to_string(),
            rationale: rationale_for_mode("concrete"),
            args: concrete_args(endpoint),
            min_tier: endpoint_tier,
            expect: "non_empty",
            builder_overrides: Vec::new(),
        }],
    ))
}

/// Representative value to feed each builder-bound (optional) parameter in
/// `with_<name>` and `all_optionals` modes. Kept dense here so callers see
/// the full matrix at a glance.
///
/// Covers every optional currently exposed in `endpoint_surface.toml`. If a
/// future builder param isn't listed, the generator falls back to `None`,
/// which drops the mode (avoids emitting a cell with no actual coverage).
fn optional_fixture_value(param_name: &str) -> Option<&'static str> {
    Some(match param_name {
        "max_dte" => "30",
        "strike_range" => "10",
        "min_time" => "09:45:00",
        "venue" => "nqb",
        "start_time" => "09:30:00",
        "end_time" => "10:00:00",
        "start_date" => "20250303",
        "end_date" => "20250303",
        "exclusive" => "true",
        "annual_dividend" => "0.015",
        "rate_type" => "sofr",
        "rate_value" => "0.05",
        "stock_price" => "150.0",
        "version" => "dg3",
        "use_market_value" => "true",
        "underlyer_use_nbbo" => "true",
        _ => return None,
    })
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
                optional_fixture_value("start_time").unwrap().to_string(),
            ),
            (
                "end_time".to_string(),
                optional_fixture_value("end_time").unwrap().to_string(),
            ),
        ];
        modes.push(TestMode {
            name: "with_intraday_window".to_string(),
            rationale: rationale_for_mode("with_intraday_window"),
            args: concrete_args(endpoint),
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
                optional_fixture_value("start_date").unwrap().to_string(),
            ),
            (
                "end_date".to_string(),
                optional_fixture_value("end_date").unwrap().to_string(),
            ),
        ];
        modes.push(TestMode {
            name: "with_date_range".to_string(),
            rationale: rationale_for_mode("with_date_range"),
            args: concrete_args(endpoint),
            min_tier: endpoint_tier,
            expect: "non_empty",
            builder_overrides: overrides,
        });
        handled.insert("start_date".into());
        handled.insert("end_date".into());
    }

    // Per-parameter `with_<name>` modes for everything else.
    for param_name in &optional_names {
        if handled.contains(param_name) {
            continue;
        }
        let Some(value) = optional_fixture_value(param_name) else {
            continue;
        };
        modes.push(TestMode {
            name: format!("with_{param_name}"),
            rationale: with_optional_rationale(param_name),
            args: concrete_args(endpoint),
            min_tier: endpoint_tier,
            expect: "non_empty",
            builder_overrides: vec![(param_name.clone(), value.to_string())],
        });
    }

    // `all_optionals` mode — set every applicable optional at once. Uses
    // the compound fixtures for paired params (single intraday window, single
    // date range) so the compound cell and this one agree on wire shape.
    let mut all_overrides: Vec<(String, String)> = Vec::new();
    for param_name in &optional_names {
        if let Some(value) = optional_fixture_value(param_name) {
            all_overrides.push((param_name.clone(), value.to_string()));
        }
    }
    if !all_overrides.is_empty() {
        modes.push(TestMode {
            name: "all_optionals".to_string(),
            rationale: rationale_for_mode("all_optionals"),
            args: concrete_args(endpoint),
            min_tier: endpoint_tier,
            expect: "non_empty",
            builder_overrides: all_overrides,
        });
    }

    modes
}

/// Collapse cells with identical wire shape down to a single canonical cell.
///
/// "Wire shape" here is `(args, sorted(builder_overrides))` — the tuple of
/// values the SDK will marshal onto the proto request. Two cells that map to
/// the same tuple will hit the server with byte-identical messages, so
/// keeping both adds runtime cost (each ~60s timeout-bounded) without
/// covering any new code path.
///
/// Collapsing rules:
/// * Group modes by their wire-shape signature.
/// * Within each group keep ONE representative (the lowest-index entry, so
///   the canonical mode like `concrete`/`bulk_chain` wins over a later
///   `with_<name>` whose override happened to match an existing fixture).
/// * Append the names of the collapsed siblings into the kept cell's
///   rationale as `(also covers: a, b)`. The downstream agreement output
///   then makes it explicit that one cell is checking two named features.
///
/// This is the audit step from W6: the matrix used to silently include
/// duplicate cells whenever an optional fixture happened to overlap a
/// concrete value. Now no two emitted cells can share a wire shape; if they
/// would, the duplicates roll up under the canonical mode's name.
fn collapse_redundant_wires(modes: Vec<TestMode>) -> Vec<TestMode> {
    use std::collections::BTreeMap;
    // Wire-shape signature: positional args + sorted optional-override pairs.
    // Two modes whose signatures are equal will marshal to byte-identical
    // proto messages, so collapsing them removes only redundant runtime cost.
    type WireSignature = (Vec<String>, Vec<(String, String)>);
    let mut buckets: BTreeMap<WireSignature, Vec<usize>> = BTreeMap::new();
    for (idx, mode) in modes.iter().enumerate() {
        let mut sorted_overrides = mode.builder_overrides.clone();
        sorted_overrides.sort();
        buckets
            .entry((mode.args.clone(), sorted_overrides))
            .or_default()
            .push(idx);
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
