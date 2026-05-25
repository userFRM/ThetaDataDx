//! Build-time parser for the pinned upstream ThetaData OpenAPI snapshot.
//!
//! The committed snapshot at `scripts/upstream_openapi.yaml` (captured from
//! `https://docs.thetadata.us/openapiv3.yaml`) is the authoritative source for:
//!   * each endpoint's minimum subscription tier (`x-min-subscription`)
//!   * whether each endpoint accepts `expiration=*`; upstream binds the strict
//!     variant to its `expiration_no_star` component parameter, and the v3
//!     server rejects `*` on those endpoints with
//!     `InvalidArgument -- Cannot specify '*' for the date`.
//!
//! The parser is deliberately line-based. The OpenAPI file is large but has a
//! very regular shape (two-space indent for paths, four-space indent for
//! `x-min-subscription` and `operationId`, six-space for the parameter
//! `$ref` lines under `parameters:`). A full YAML parser would be overkill
//! for two fields; a small state machine keeps the build dep-free.
//!
//! If upstream ever changes this shape, [`UpstreamOpenApi::load`] fails the
//! build with a clear message — which is a feature, not a bug. We'd rather
//! notice a drift than silently generate the wrong matrix.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Endpoint metadata derived from the pinned upstream OpenAPI snapshot.
#[derive(Debug, Clone)]
pub struct UpstreamEndpoint {
    /// Minimum subscription tier the endpoint requires. One of
    /// `"free"`, `"value"`, `"standard"`, `"professional"`.
    pub min_subscription: String,
    /// `true` iff the endpoint's parameter block references
    /// `#/components/parameters/expiration` (wildcard-allowed).
    /// `false` iff it references `expiration_no_star` (wildcard-disallowed)
    /// *or* doesn't take an expiration parameter at all. Consumers use this
    /// only to decide whether to emit `expiration=*` test modes, so the
    /// "no expiration at all" case degenerates to "don't emit the mode" —
    /// which is the correct behavior.
    pub supports_expiration_wildcard: bool,
}

/// Joined view of the upstream OpenAPI snapshot, keyed by `operationId`.
///
/// The snapshot uses two keys: the REST path (as a YAML map key) and the
/// `operationId` (as a string value inside the `get:` block). The Rust
/// endpoint names match upstream `operationId` exactly (`stock_list_symbols`,
/// `option_history_ohlc`, ...) so the public API is a `by_operation` map.
#[derive(Debug)]
pub struct UpstreamOpenApi {
    by_operation: HashMap<String, UpstreamEndpoint>,
}

impl UpstreamOpenApi {
    /// Load and parse the snapshot, caching the result across callers.
    ///
    /// Callers come from both `build.rs` (cwd = crate root) and the
    /// `generate_sdk_surfaces` binary (same cwd), so the snapshot path
    /// is resolved relative to the crate root at `../../scripts/upstream_openapi.yaml`.
    pub fn load() -> &'static Self {
        static CACHE: OnceLock<UpstreamOpenApi> = OnceLock::new();
        CACHE.get_or_init(|| {
            let candidates: [PathBuf; 2] = [
                PathBuf::from("../../scripts/upstream_openapi.yaml"),
                PathBuf::from("scripts/upstream_openapi.yaml"),
            ];
            let (path, text) = candidates
                .iter()
                .find_map(|candidate| {
                    fs::read_to_string(candidate)
                        .ok()
                        .map(|s| (candidate.clone(), s))
                })
                .unwrap_or_else(|| {
                    panic!(
                        "upstream OpenAPI snapshot not found; expected one of {candidates:?} \
                         relative to the current directory. Run \
                         `python3 scripts/check_tier_badges.py --refresh-snapshot` to populate."
                    )
                });
            // Emit rerun-if-changed so cargo picks up refreshed snapshots
            // without requiring a clean build. Harmless in the
            // generate_sdk_surfaces binary — cargo ignores these prints
            // outside of build scripts.
            println!("cargo:rerun-if-changed={}", path.display());
            Self::parse(&text).unwrap_or_else(|err| {
                panic!(
                    "failed to parse {}: {err}. \
                     Upstream YAML shape may have changed; \
                     inspect the snapshot or refresh with `--refresh-snapshot`.",
                    path.display()
                )
            })
        })
    }

    /// Parse the raw YAML text into an [`UpstreamOpenApi`].
    ///
    /// Public for unit tests. Accepts any text that matches the shape of the
    /// committed snapshot — see the module docs for the exact assumptions.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut by_operation: HashMap<String, UpstreamEndpoint> = HashMap::new();

        // The OpenAPI snapshot's `paths:` block always uses this exact shape:
        //   <2 spaces>/stock/snapshot/ohlc:
        //   <4 spaces>x-min-subscription: value
        //   <4 spaces>get:
        //   <6 spaces>operationId: stock_snapshot_ohlc
        //   <6 spaces>parameters:
        //   <8 spaces>- $ref: "#/components/parameters/expiration_no_star"
        //
        // The parser is a small state machine: when it sees a 2-indent path
        // line it resets accumulators; once it sees `operationId` it knows
        // which endpoint the surrounding x-min-subscription + expiration ref
        // belong to, and it emits a row. Commented-out blocks (`#` prefix)
        // and the `components:` section at the bottom are ignored by the
        // 2-space-indent rule on paths.

        let mut current_path: Option<String> = None;
        let mut current_min_sub: Option<String> = None;
        let mut current_operation_id: Option<String> = None;
        let mut uses_expiration: bool = false;
        let mut uses_expiration_no_star: bool = false;
        // Tracks whether we've seen a `paths:` top-level key. `components:` or
        // anything else at column 0 resets us out of paths mode.
        let mut in_paths_section = false;
        // Count ANY endpoint referencing either `expiration` or
        // `expiration_no_star` across the whole file. If the snapshot has
        // zero such references, upstream has almost certainly renamed the
        // component (or restructured the params block) and we must fail the
        // build — otherwise the generator would silently default every
        // option endpoint to "wildcard-allowed" and emit wildcard modes that
        // the server rejects.
        let mut saw_any_expiration_ref = false;

        for (line_no, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim_end();
            // Skip empty lines and pure comment lines (including frontmatter).
            if line.is_empty() {
                continue;
            }
            let trimmed_leading = line.trim_start();
            if trimmed_leading.starts_with('#') {
                continue;
            }

            // Top-level keys at column 0.
            if !line.starts_with(' ') {
                // Flush any pending endpoint when leaving `paths:`.
                flush(
                    &mut by_operation,
                    &current_path,
                    &current_operation_id,
                    &current_min_sub,
                    uses_expiration,
                    uses_expiration_no_star,
                )?;
                current_path = None;
                current_min_sub = None;
                current_operation_id = None;
                uses_expiration = false;
                uses_expiration_no_star = false;
                in_paths_section = line.trim_end_matches(':') == "paths";
                continue;
            }

            if !in_paths_section {
                continue;
            }

            // A path entry starts at exactly 2 spaces with a leading `/`.
            if let Some(path) = parse_path_key(line) {
                flush(
                    &mut by_operation,
                    &current_path,
                    &current_operation_id,
                    &current_min_sub,
                    uses_expiration,
                    uses_expiration_no_star,
                )?;
                current_path = Some(path);
                current_min_sub = None;
                current_operation_id = None;
                uses_expiration = false;
                uses_expiration_no_star = false;
                continue;
            }

            if current_path.is_none() {
                continue;
            }

            // `x-min-subscription: <tier>` at 4-space indent under a path.
            if let Some(tier) = strip_indented_kv(line, 4, "x-min-subscription") {
                current_min_sub = Some(tier.to_string());
                continue;
            }

            // `operationId: <name>` at 6-space indent under `get:`.
            if let Some(op_id) = strip_indented_kv(line, 6, "operationId") {
                current_operation_id = Some(op_id.to_string());
                continue;
            }

            // Parameter `$ref` under the endpoint's `parameters:` list. These
            // appear at 8 spaces + `- $ref:`. We recognize the two known
            // expiration variants, and we also fail the build if any other
            // ref name *contains* "expiration" — that would be upstream
            // drifting the parameter (e.g. renaming to `expiration_strict`
            // or `expiration_v2`) and we must not silently default the
            // affected endpoint to wildcard-allowed. Catching this per
            // endpoint, not just globally, is the point.
            if let Some(rest) = trimmed_leading.strip_prefix("- $ref:") {
                let rest = rest.trim();
                if ref_points_at(rest, "expiration_no_star") {
                    uses_expiration_no_star = true;
                    saw_any_expiration_ref = true;
                } else if ref_points_at(rest, "expiration") {
                    uses_expiration = true;
                    saw_any_expiration_ref = true;
                } else if let Some(name) = extract_param_name(rest) {
                    if name.contains("expiration") {
                        let op = current_operation_id
                            .as_deref()
                            .or(current_path.as_deref())
                            .unwrap_or("<unknown>");
                        return Err(format!(
                            "endpoint {op}: parameter ref {rest:?} contains \
                             \"expiration\" but is not one of the two known \
                             variants (expiration, expiration_no_star). \
                             Upstream likely renamed or split the expiration \
                             parameter; teach `build_support/upstream_openapi.rs` \
                             how to classify {name:?} before proceeding."
                        ));
                    }
                }
                continue;
            }

            // Inline parameter block (no `$ref` — upstream sometimes inlines
            // `- name: foo\n  in: query\n  ...` directly on an endpoint).
            // We don't know the wildcard semantics of an inline expiration
            // param, so we fail the build rather than silently default it.
            // Match lines that start `- name: expiration...` at any indent
            // inside the parameters block.
            if let Some(rest) = trimmed_leading.strip_prefix("- name:") {
                let inline_name = rest.trim().trim_matches(|c| c == '"' || c == '\'');
                if inline_name.contains("expiration") {
                    let op = current_operation_id
                        .as_deref()
                        .or(current_path.as_deref())
                        .unwrap_or("<unknown>");
                    return Err(format!(
                        "endpoint {op}: inline parameter named {inline_name:?} \
                         contains \"expiration\". Upstream inlined the expiration \
                         parameter (rather than using `$ref` to a known \
                         component); teach `build_support/upstream_openapi.rs` \
                         how to classify this inline form before proceeding."
                    ));
                }
                continue;
            }

            // Ignore all other lines.
            let _ = line_no; // reserved for future diagnostics
        }

        // Flush the final endpoint.
        flush(
            &mut by_operation,
            &current_path,
            &current_operation_id,
            &current_min_sub,
            uses_expiration,
            uses_expiration_no_star,
        )?;

        if by_operation.is_empty() {
            return Err("parsed 0 endpoints from snapshot -- upstream YAML shape changed?".into());
        }
        // Fail closed if we parsed endpoints but saw zero references to either
        // expiration parameter variant. Upstream currently binds ~15 option
        // endpoints to one of these two components, so zero refs means the
        // component was renamed or the params structure drifted — silently
        // defaulting every endpoint to "wildcard-allowed" would spray the
        // matrix with cells that the v3 server rejects.
        if !saw_any_expiration_ref {
            return Err("parsed the snapshot but found zero references to either \
                 #/components/parameters/expiration or expiration_no_star. \
                 Upstream likely renamed the expiration parameter component; \
                 update build_support/upstream_openapi.rs to match the new \
                 shape and refresh the snapshot."
                .into());
        }
        Ok(Self { by_operation })
    }

    /// Look up an endpoint by its `operationId`.
    pub fn endpoint(&self, operation_id: &str) -> Option<&UpstreamEndpoint> {
        self.by_operation.get(operation_id)
    }

    /// Number of endpoints parsed. Used by tests; kept on the lib surface so
    /// consumers can sanity-check the snapshot shape if needed.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.by_operation.len()
    }
}

/// Emit the current accumulator as a completed `UpstreamEndpoint` if all
/// required fields are present.
///
/// Missing `operationId` is tolerated: upstream commented-out preview blocks
/// (like `option_snapshot_trade_greeks_*`) have no live `operationId` and
/// should not appear in the map. Missing `x-min-subscription` on a real
/// endpoint is an upstream bug and we fail closed.
fn flush(
    map: &mut HashMap<String, UpstreamEndpoint>,
    path: &Option<String>,
    operation_id: &Option<String>,
    min_sub: &Option<String>,
    uses_expiration: bool,
    uses_expiration_no_star: bool,
) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    let Some(operation_id) = operation_id else {
        return Ok(());
    };
    let Some(min_sub) = min_sub else {
        return Err(format!(
            "endpoint {path} (operationId={operation_id}) has no x-min-subscription"
        ));
    };
    // If the endpoint has both refs (shouldn't happen but be defensive), the
    // strict one wins — matches prior manual mapping semantics.
    let supports_expiration_wildcard = if uses_expiration_no_star {
        false
    } else if uses_expiration {
        true
    } else {
        // No expiration parameter at all. The only consumer
        // (`endpoint_supports_expiration_wildcard`) won't emit an expiration
        // wildcard mode in that case, so the default is inert.
        true
    };
    // `path` is consumed only for diagnostics above (fail message); we
    // don't carry it into `UpstreamEndpoint` since nothing downstream
    // keys on path once `operationId` is resolved. Reintroduce it if a
    // future consumer needs path-based lookup.
    let _ = path;
    map.insert(
        operation_id.clone(),
        UpstreamEndpoint {
            min_subscription: min_sub.clone(),
            supports_expiration_wildcard,
        },
    );
    Ok(())
}

/// Parse a line like `  /stock/snapshot/ohlc:` and return the path.
fn parse_path_key(line: &str) -> Option<String> {
    if !line.starts_with("  /") {
        return None;
    }
    // Must be exactly 2-space indent, not deeper.
    if line.starts_with("   ") {
        return None;
    }
    let rest = &line[2..];
    let key = rest.strip_suffix(':')?;
    if key.contains(' ') || key.is_empty() {
        return None;
    }
    Some(key.to_string())
}

/// Parse `<indent>key: value` at an exact `indent` column and return the
/// value. Returns `None` if the indent doesn't match or the key isn't at the
/// expected depth.
fn strip_indented_kv<'a>(line: &'a str, indent: usize, key: &str) -> Option<&'a str> {
    if line.len() < indent {
        return None;
    }
    let (leading, rest) = line.split_at(indent);
    if !leading.chars().all(|c| c == ' ') {
        return None;
    }
    // Reject deeper indents — the next char must not be a space.
    if rest.starts_with(' ') {
        return None;
    }
    let without_key = rest.strip_prefix(key)?;
    // Accept `key:` or `key: value` only.
    let after_colon = without_key.strip_prefix(':')?;
    Some(after_colon.trim())
}

/// Extract the parameter component name from a ``$ref: "#/components/parameters/NAME"``
/// payload, regardless of `parameters` nesting or quoting style.
///
/// Returns `None` if the ref doesn't point at `#/components/parameters/*`.
/// Used to catch upstream drift: any ref whose name *contains* `"expiration"`
/// but isn't one of the two known variants (`expiration` / `expiration_no_star`)
/// is a hard error — we must not silently emit wildcard modes for it.
fn extract_param_name(ref_tail: &str) -> Option<&str> {
    let stripped = ref_tail
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            ref_tail
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(ref_tail);
    stripped.strip_prefix("#/components/parameters/")
}

/// Test whether a ``$ref: "..."`` payload points at the given `parameters/NAME`.
/// Accepts either the double-quoted or single-quoted form as upstream has
/// mixed both historically.
fn ref_points_at(ref_tail: &str, name: &str) -> bool {
    // Strip the leading quote (" or ') and trailing quote.
    let stripped = ref_tail
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            ref_tail
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        });
    let target = stripped.unwrap_or(ref_tail);
    target == format!("#/components/parameters/{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"
paths:
  /stock/snapshot/ohlc:
    x-min-subscription: value
    get:
      operationId: stock_snapshot_ohlc
      parameters:
        - $ref: "#/components/parameters/multi_symbol"
        - $ref: "#/components/parameters/format"
  /option/snapshot/trade:
    x-min-subscription: standard
    get:
      operationId: option_snapshot_trade
      parameters:
        - $ref: "#/components/parameters/single_symbol"
        - $ref: "#/components/parameters/expiration_no_star"
        - $ref: "#/components/parameters/strike"
  /option/history/ohlc:
    x-min-subscription: value
    get:
      operationId: option_history_ohlc
      parameters:
        - $ref: "#/components/parameters/single_symbol"
        - $ref: "#/components/parameters/expiration"
        - $ref: "#/components/parameters/strike"
components:
  parameters:
    expiration: {}
    expiration_no_star: {}
"##;

    #[test]
    fn parses_tier_and_wildcard() {
        let spec = UpstreamOpenApi::parse(SAMPLE).expect("parse ok");
        assert_eq!(spec.len(), 3);

        let ohlc = spec.endpoint("stock_snapshot_ohlc").unwrap();
        assert_eq!(ohlc.min_subscription, "value");
        // No expiration ref at all — default is "allow wildcards" which is
        // inert because test_modes_for won't emit expiration modes without an
        // expiration param.
        assert!(ohlc.supports_expiration_wildcard);

        let trade = spec.endpoint("option_snapshot_trade").unwrap();
        assert_eq!(trade.min_subscription, "standard");
        assert!(!trade.supports_expiration_wildcard);

        let hist = spec.endpoint("option_history_ohlc").unwrap();
        assert_eq!(hist.min_subscription, "value");
        assert!(hist.supports_expiration_wildcard);
    }

    #[test]
    fn ignores_commented_blocks() {
        let text = r##"
paths:
  /stock/snapshot/ohlc:
    x-min-subscription: value
    get:
      operationId: stock_snapshot_ohlc
  # /option/snapshot/trade_greeks_all:
  #   x-min-subscription: professional
  #   get:
  #     operationId: option_snapshot_trade_greeks_all
  /option/snapshot/trade:
    x-min-subscription: standard
    get:
      operationId: option_snapshot_trade
      parameters:
        - $ref: "#/components/parameters/expiration_no_star"
"##;
        let spec = UpstreamOpenApi::parse(text).expect("parse ok");
        assert_eq!(spec.len(), 2);
        assert!(spec.endpoint("option_snapshot_trade_greeks_all").is_none());
    }

    #[test]
    fn errors_when_min_subscription_missing() {
        let bad = r##"
paths:
  /broken:
    get:
      operationId: broken
"##;
        let err = UpstreamOpenApi::parse(bad).expect_err("should fail");
        assert!(err.contains("x-min-subscription"), "got: {err}");
    }

    #[test]
    fn errors_when_nothing_parsed() {
        let empty = "openapi: 3.1.0\ninfo:\n  title: empty\n";
        let err = UpstreamOpenApi::parse(empty).expect_err("should fail");
        assert!(err.contains("0 endpoints"), "got: {err}");
    }

    #[test]
    fn errors_when_single_endpoint_uses_unknown_expiration_variant() {
        // One endpoint has the known variant; another uses a new
        // `expiration_strict` variant. The parser must fail on the latter,
        // not silently default it to wildcard-allowed.
        let text = r##"
paths:
  /option/snapshot/trade:
    x-min-subscription: standard
    get:
      operationId: option_snapshot_trade
      parameters:
        - $ref: "#/components/parameters/single_symbol"
        - $ref: "#/components/parameters/expiration_no_star"
  /option/snapshot/quote:
    x-min-subscription: value
    get:
      operationId: option_snapshot_quote
      parameters:
        - $ref: "#/components/parameters/single_symbol"
        - $ref: "#/components/parameters/expiration_strict"
"##;
        let err = UpstreamOpenApi::parse(text).expect_err("should fail");
        assert!(err.contains("expiration_strict"), "got: {err}");
        assert!(err.contains("option_snapshot_quote"), "got: {err}");
    }

    #[test]
    fn errors_when_endpoint_inlines_expiration_parameter() {
        // Endpoint inlines an `expiration` param via `- name:` instead of
        // `$ref` — parser must fail rather than assume wildcard semantics.
        let text = r##"
paths:
  /option/snapshot/trade:
    x-min-subscription: standard
    get:
      operationId: option_snapshot_trade
      parameters:
        - $ref: "#/components/parameters/expiration_no_star"
  /option/snapshot/quote:
    x-min-subscription: value
    get:
      operationId: option_snapshot_quote
      parameters:
        - name: expiration_inline
          in: query
          required: true
"##;
        let err = UpstreamOpenApi::parse(text).expect_err("should fail");
        assert!(err.contains("inline parameter"), "got: {err}");
        assert!(err.contains("expiration_inline"), "got: {err}");
    }

    #[test]
    fn errors_when_expiration_component_missing() {
        // Well-formed shape but zero references to either expiration variant.
        // This is the "upstream silently renamed the expiration parameter"
        // scenario we need to catch.
        let text = r##"
paths:
  /stock/snapshot/ohlc:
    x-min-subscription: value
    get:
      operationId: stock_snapshot_ohlc
      parameters:
        - $ref: "#/components/parameters/multi_symbol"
  /option/snapshot/trade:
    x-min-subscription: standard
    get:
      operationId: option_snapshot_trade
      parameters:
        - $ref: "#/components/parameters/single_symbol"
        - $ref: "#/components/parameters/expiration_strict"
        - $ref: "#/components/parameters/strike"
"##;
        let err = UpstreamOpenApi::parse(text).expect_err("should fail");
        assert!(
            err.contains("expiration_no_star") || err.contains("renamed"),
            "got: {err}"
        );
    }

    #[test]
    fn tolerates_frontmatter_comments() {
        let text = format!("# _captured_at: 2026-04-14T00:00:00Z\n# _source: example\n{SAMPLE}");
        let spec = UpstreamOpenApi::parse(&text).expect("parse ok");
        assert_eq!(spec.len(), 3);
    }

    #[test]
    fn real_snapshot_parses() {
        // Skipped in environments without the snapshot checked in; the
        // path is resolved relative to the crate root so CI passes the
        // test when present and the test file is compiled from a
        // workspace-wide `cargo test`.
        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let snapshot_path = std::path::Path::new(&crate_root)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("scripts/upstream_openapi.yaml");
        let Ok(text) = std::fs::read_to_string(&snapshot_path) else {
            // Snapshot not present — test is effectively a no-op.
            return;
        };
        let spec = UpstreamOpenApi::parse(&text).expect("real snapshot parses");
        // Spot-check a few well-known endpoints to make sure the map
        // contents are what we expect.
        let ohlc = spec.endpoint("stock_snapshot_ohlc").unwrap();
        assert_eq!(ohlc.min_subscription, "value");

        let trade = spec.endpoint("option_snapshot_trade").unwrap();
        assert_eq!(trade.min_subscription, "standard");
        assert!(!trade.supports_expiration_wildcard);

        let list_symbols = spec.endpoint("stock_list_symbols").unwrap();
        assert_eq!(list_symbols.min_subscription, "free");
    }
}
