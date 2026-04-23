# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [8.0.13] - 2026-04-23

### Fixed

- Mid-stream chunk header drift in the MDDS response accumulator was
  silently masked: `MddsClient::collect_stream` / `for_each_chunk` would
  keep the first chunk's `headers` and pile subsequent chunks' rows
  underneath, even if a later chunk carried a different non-empty
  header set. A server-side schema change mid-response would therefore
  surface as silent data corruption instead of an error. Both paths
  now compare the saved first-chunk schema against every non-empty
  chunk header set and raise a new `DecodeError::ChunkHeaderDrift`
  on mismatch (P13 from the external bench handoff).

### Added

- `decode::DecodeError::ChunkHeaderDrift { chunk_index, first, chunk }`
  variant.

### Known

- **`option_at_time_quote` 0.67× vs vendor** (bench handoff §8 #1).
  The v8.0.5 uniform `mdds_query_field_expr` rule that empties the
  top-level `expiration` field on any option query carrying a
  `ContractSpec` may have flipped this specific endpoint into a
  slower server-side path. Needs a bench-validated per-endpoint
  override in `endpoint_surface.toml`. Not fixed in this release
  because a speculative generator carve-out without bench
  re-validation would risk regressing the other option endpoints
  that benefit from the current rule.
- **`option_history_greeks_eod` 0.704× vs vendor** (bench handoff §8
  #2). Persistent across v8.0.0 / v8.0.4 / v8.0.10. Likely server-
  side per-contract aggregation path rather than a wire-shape
  issue; needs proto-level diff against the other
  `option_history_greeks_*` endpoints (which are DX wins at
  4-6× faster).

## [8.0.12] - 2026-04-23

### Removed

- `scripts/test_drift_injection.sh` + the `FPSS drift injection` CI job
  (`.github/workflows/ci.yml`). The test was designed when the C++
  `static_assert(offsetof)` guards in `thetadx.hpp` were hand-maintained
  against a Rust-generated C struct layout. v8.0.11 moved both sides
  under the same SSOT generator, so swapping a field in
  `fpss_event_schema.toml` regenerates the C struct and the assert
  value in lockstep and the assertion can no longer fail. Removed
  rather than kept as a misleading safety net; `regen_byte_identical`
  covers generator consistency and the assertions still fire at C++
  compile time against hand-committed C header corruption.

## [8.0.11] - 2026-04-23

### Added

- `endpoint_surface.toml` now declares the endpoint-surface enums used by
  `right`, `venue`, `interval`, `rate_type`, `request_type`, and
  `version`. The generator emits the Rust `tdbe` enums, Python enum
  pyclasses, and the TypeScript napi string enums from the same TOML
  variant lists.
- Go now gets generator-owned FFI drift artifacts for every checked size
  and offset: `endpoint_ffi_sizes_generated.go`,
  `tick_ffi_sizes_generated.go`, `fpss_ffi_sizes_generated.go`,
  `ffi_layout_generated_test.go`, and
  `fpss_ffi_offset_checks_generated.go`.
- C++ now gets generator-owned layout assertion includes:
  `tick_layout_asserts.hpp.inc` from `tick_schema.toml` and
  `fpss_layout_asserts.hpp.inc` from `fpss_event_schema.toml`.
- `.github/release-notes/v8.0.11.md` records the SSOT refactor and local
  verification plan for this release.

### Changed

- `crates/tdbe/src/types/enums.rs` now includes generator-emitted
  endpoint-surface enums instead of hand-maintaining `Right`, `Venue`,
  `Interval`, `RateType`, `RequestType`, and `Version`.
- `sdks/python/src/coerce.rs` now includes generator-emitted enum
  pyclasses instead of hand-maintaining the `string_enum!` block.
- `sdks/go/tick_ffi_mirrors.go` no longer embeds hand-maintained expected
  sizes or FPSS offset literals; it consumes generator-owned constants and
  offset tables.
- `sdks/go/ffi_layout_test.go` has been replaced by the generated
  `sdks/go/ffi_layout_generated_test.go`, so the Go tick-layout drift
  detector now reads its expected values from TOML-derived generation.
- `sdks/cpp/include/thetadx.hpp` now includes generated layout assertion
  fragments instead of hand-maintaining `static_assert(sizeof(...))` and
  `static_assert(offsetof(...))` blocks.
- Live docs and READMEs no longer hardcode endpoint, tick-type, or tool
  counts; they describe the generated surface instead.
- Release metadata bumps `8.0.10 -> 8.0.11` across `thetadatadx`,
  `thetadatadx-ffi`, `thetadatadx-cli`, `thetadatadx-server`,
  `thetadatadx-mcp`, `thetadatadx-py`, and `thetadatadx-napi`. TypeScript
  package metadata, loader version guards, and the checked-in OpenAPI
  version now match `8.0.11`.
- `tdbe` stays at `0.12.0`.

## [8.0.10] - 2026-04-23

### Added

- `endpoint_surface.toml` now carries upstream-verified defaults for
  every builder-bound optional param that the ThetaData OpenAPI spec
  documents as optional with a server-side fallback: `venue = "nqb"`,
  `rate_type = "sofr"`, `version = "latest"`, `exclusive = true`,
  `use_market_value = false`, `underlyer_use_nbbo = false`. These flow
  through the `parsed_endpoint!` macro as the initial builder value, so
  callers that omit the field hit the same wire payload the official
  Python library produces — no per-endpoint runtime fallback needed.
- Parameter descriptions in the SSOT now enumerate accepted values for
  `venue`, `rate_type`, `version`, `exclusive`, `use_market_value`, and
  `underlyer_use_nbbo`, which propagates into the per-language generator
  outputs (Rust docstrings, Go `endpoint_options.go`, C++
  `endpoint_options.hpp.inc`, Python builder docstrings).
- SSOT defaults now cover `right = "both"`, `strike = "*"`, and
  `interval = "1s"`. The option contract endpoints no longer require
  `right` and `strike` as positional Rust method arguments; callers set
  concrete values through the existing options builder fields when they
  need to override the server defaults.
- Python bindings expose module-level `Right`, `Venue`, `Interval`,
  `RateType`, `RequestType`, and `Version` string enum classes. Enum
  constrained parameters accept either plain strings or those enum
  objects.
- TypeScript declarations expose matching literal-union types and const
  companions for `Right`, `Venue`, `Interval`, `RateType`, `RequestType`,
  and `Version`.

### Changed

- The `venue=nqb` default moved from a runtime constant
  (`wire_semantics::DEFAULT_STOCK_VENUE`) into the SSOT, making
  `endpoint_surface.toml` the single source of truth for every
  parameter default across every emitter. The generator's query-
  assembly path now wraps default-bearing `Str` fields in `Some(...)`
  when marshalling into the proto request, keeping the wire shape
  identical to the previous release.
- `collapse_redundant_wires` in the build-time mode matrix now reads
  per-endpoint SSOT defaults instead of the hardcoded `venue=nqb`
  branch, so future additions to the default set automatically collapse
  their redundant `with_<name>` validator cells.
- Release metadata bumps 8.0.9 -> 8.0.10 across every Rust crate
  (`thetadatadx`, `thetadatadx-ffi`, `thetadatadx-cli`,
  `thetadatadx-server`, `thetadatadx-mcp`, `thetadatadx-py`,
  `thetadatadx-napi`), every TypeScript package (`sdks/typescript` root
  plus the three platform subpackages under `sdks/typescript/npm/`),
  the TypeScript native binding version guard in
  `sdks/typescript/index.js`, and the OpenAPI contract in
  `docs-site/public/thetadatadx.yaml`.
- `tdbe` stays at `0.12.0`; the encoding crate is untouched.
- Rust, Python, TypeScript, Go, and C++ endpoint surfaces now project
  proto `repeated string symbol` endpoints as bulk-capable symbol inputs.
  Singular-symbol wire endpoints remain singular.
- Python historical date parameters (`date`, `expiration`, `start_date`,
  `end_date`) accept `str`, `datetime.date`, or `datetime.datetime`.
  Python time parameters (`start_time`, `end_time`, `min_time`,
  `time_of_day`) accept `str` or `datetime.time`.
- TypeScript historical date and time parameters accept either `string`
  or JavaScript `Date` values at the native binding boundary.

## [8.0.9] - 2026-04-23

### Fixed

- The TypeScript package lock now matches `package.json` for version,
  license, Node engine, and platform optional dependency pins.
- The requested repo-root `scripts/regen_byte_identical.sh` gate now
  delegates to the checked-in generator determinism harness, and the docs
  consistency and tier badge scripts are executable.
- User-facing docs and release notes no longer point at deleted
  `thetadatadx` modules or removed FPSS shortcut APIs.
- `CHANGELOG.md` and `docs-site/docs/changelog.md` use only the
  Keep-a-Changelog section buckets and avoid banned performance phrasing.

### Changed

- Release metadata now points at `8.0.9` across Rust crates, the
  TypeScript root package and platform packages, the TypeScript native
  binding version guard, the C++ package metadata, and the checked-in
  OpenAPI contract.
- Every Rust crate version bumps `8.0.8 -> 8.0.9`: `thetadatadx`,
  `thetadatadx-ffi`, `thetadatadx-cli`, `thetadatadx-server`,
  `thetadatadx-mcp`, `thetadatadx-py`, `thetadatadx-napi`.
- `sdks/typescript/package.json` and every platform subpackage under
  `sdks/typescript/npm/` bump to `8.0.9` so the npm dependency graph
  stays coherent.
- `tdbe` stays at `0.12.0`; this patch is metadata, docs, and tooling
  hygiene only.

## [8.0.8] - 2026-04-23

Follow-up patch to v8.0.7. Addresses the audit findings surfaced against
the code-strip release: rustdoc breakage inside `tdbe`, TypeScript loader
and subpackage versions drifting from the root package, a `[8.0.7]`
changelog section that accidentally absorbed v8.0.6 content, stale
references to removed modules, and a handful of doc
inaccuracies around DataFrame terminals and SDK parameter names. No
behaviour changes; every item is documentation, packaging metadata, or
tooling hygiene.

### Fixed

- `crates/tdbe/src/codec/fit.rs` — broken intra-doc link on
  `FitReader`'s module-level docstring now resolves via
  `[FitReader::read_changes]`.
- `crates/tdbe/src/right.rs` — five redundant explicit link targets on
  `[Error::Config]` references dropped; rustdoc resolves the bare path
  against the in-scope `use crate::error::Error`.
- `sdks/typescript/index.js` — native-binding version guard now compares
  against `'8.0.8'` (was stale sentinel `'8.0.0'`). Mismatched binaries
  are caught when `NAPI_RS_ENFORCE_VERSION_CHECK` is set.
- `sdks/typescript/package.json` — `optionalDependencies` pin each
  platform subpackage to `8.0.8` (was `8.0.4`). The three published
  subpackages (`thetadatadx-linux-x64-gnu`, `thetadatadx-darwin-arm64`,
  `thetadatadx-win32-x64-msvc`) bump from `8.0.7` to `8.0.8` in lockstep.
- `CHANGELOG.md` / `docs-site/docs/changelog.md` — v8.0.6 content
  (snapshot fast-path, Rust `frames` module) split back out of the
  v8.0.7 section into a standalone `[8.0.6]` entry; the `### Changed`
  bucket on v8.0.6 was renamed `### Changed` to stay within the Keep a
  Changelog vocabulary.
- `docs/api-reference.md` — two references to the old `tdbe` error
  module repointed to `tdbe::error`.
- `docs/java-parity-checklist.md` — stale normalization-module path
  updated to `mdds/endpoints.rs`, the current home of
  `normalize_interval` after the v8.0.7 fold.
- `crates/thetadatadx/src/wire_semantics.rs` — stale normalization-module
  parenthetical removed from the module docstring.
- `docs-site/docs/api-reference.md` — DataFrame-terminals section
  narrowed: `.to_pandas()` / `.to_polars()` / `.to_arrow()` are
  available on the `<TickName>List` list-wrapper return types;
  snapshot-fast-path endpoints return a plain `list[TickClass]` and do
  not carry the chainable terminals.
- `sdks/python/README.md`, `sdks/go/README.md`, `sdks/cpp/README.md` —
  parameter-name tables now use the canonical SSOT names
  (`expiration`, `start_date`, `end_date`) instead of the `exp`,
  `start`, `end` shorthand.

### Changed

- `docs-site/docs/.vitepress/config.ts` — `vite.build.chunkSizeWarningLimit`
  raised to `1500` kB. The docs site bundles Mermaid and Vue chunks that
  exceed the default 500 kB threshold; the warning was non-actionable.
- `deny.toml` — unused license allowances pruned from `[licenses].allow`;
  remaining entries carry a short comment explaining why each is there.
  `cargo deny check` now produces zero warnings.

## [8.0.7] - 2026-04-23

Code-strip release. No new features. Every item removes dead or
near-dead code, narrows module visibility, or consolidates parallel
FFI surfaces. `tdbe` bumps to `0.12.0` (public module removed).

### Removed

- MDDS normalization forwarding layer over `crate::wire_semantics`. The
  three wire canonicalizers
  (`normalize_expiration`, `wire_strike_opt`, `wire_right_opt`) stay
  at `crate::wire_semantics`; the MDDS-scoped `normalize_interval`,
  `normalize_time_of_day`, and `contract_spec!` macro move next to
  their generated consumers in `crates/thetadatadx/src/mdds/endpoints.rs`.
- `fpss::session::reconnect` — 90 LOC public function, zero callers.
  `ThetaDataDx::reconnect_streaming` remains the reconnect entry point.
  `reconnect_delay` is kept (used by `fpss::decode`).
- The crate-local right-parser re-export shim was removed.
  `parse_right`, `parse_right_strict`, and `ParsedRight` stay at the
  crate root via a direct `pub use tdbe::right::*`.
- The unreachable retry helper trio and the crate-level
  `#![allow(dead_code)]` attribute that masked them were removed.
  `StatusClass` moved into `macros.rs` as a private enum.
- `crates/tdbe/src/errors.rs` — folded into `tdbe::error`. The two
  used items (`HTTP_STATUS_CODE_KEY`, `error_from_http_code`) are now
  reachable at `tdbe::error::*`; the unused `error_name` helper and
  the `errors` module itself are gone.
- 24 `FpssClient` / `ThetaDataDx` per-security shortcut methods (and
  their unsubscribe twins). Callers use the
  `Contract`-taking `subscribe_quotes` / `subscribe_trades` /
  `subscribe_open_interest` methods directly.
- 61 `MddsClient::<endpoint>_with_deadline` sibling methods on every
  list endpoint. Per-call deadlines route through
  `EndpointArgs::with_timeout_ms` (FFI / Python / TS / Go / C++) or
  the builder `.with_deadline(Duration)` setter on parsed endpoints.
  SDK generators now wrap the bare call in `tokio::time::timeout`
  locally instead of calling the deleted `_with_deadline` variant.
- 61 `tdx_<endpoint>` (no-options) FFI entry points. The C++ SDK
  already calls the `tdx_<endpoint>_with_options` variants, so the
  plain-name declarations in `sdks/cpp/include/thetadx.h` and the
  hand-written historical FFI wrappers are gone.
- `pub use prost` at the `thetadatadx` crate root. Downstream
  consumers that need `prost::Message` (`sdks/python`) now pull it
  in as a direct dependency pinned to the same `=0.14.3` version.
- `MddsClient::raw_query`, `MddsClient::raw_query_info`,
  `MddsClient::channel` — zero callers anywhere in the tree.

### Changed

- `pub mod unified` and `pub mod registry` narrowed to `pub(crate)`.
  The documented types (`ThetaDataDx`, `SubscriptionInfo`,
  `ConnectionStatus`, `EndpointMeta`, `ParamMeta`, `ParamType`,
  `ReturnType`, `ENDPOINTS`, plus `by_category`, `find`,
  `param_type_to_json_type`, `CATEGORIES` for the CLI / MCP tools)
  stay public via `pub use`.
- `DirectConfig::production_defaults` narrowed to `pub(crate)`; the
  only caller outside `config.rs` is in-crate (`observability.rs`).
- `crates/tdbe` bumps to `0.12.0` (breaking: `pub mod errors`
  removed). The public `ThetaDataError` struct, `error_from_http_code`
  fn, and `HTTP_STATUS_CODE_KEY` const are still reachable at the
  new `tdbe::error::*` path.
- FFI surface consolidated: every SDK — C++, Go, Python,
  TypeScript — now calls the `tdx_<endpoint>_with_options` entry
  points. The plain-name FFI entry points are no longer exported.

## [8.0.6] - 2026-04-23

Snapshot-endpoint latency fast-path on the Python binding and new opt-in
Rust `frames` module. Reduces residual latency on the 5 flagged snapshot /
calendar endpoints (`stock_snapshot_ohlc`, `stock_snapshot_quote`,
`stock_snapshot_market_value`, `calendar_on_date`, `calendar_open_today`),
and brings chainable `.to_polars()` / `.to_arrow()` DataFrame ergonomics
to Rust consumers behind opt-in Cargo features so polars and arrow stay
out of the default dep graph.

### Added

- **Rust `frames` module — `TicksPolarsExt` / `TicksArrowExt` extension traits behind `polars` / `arrow` / `frames` Cargo features.** Chain `.to_polars()` / `.to_arrow()` off a decoder-owned `&[tick::T]` in Rust the same way Python users chain off `<TickName>List`. Per-tick-type impls are generator-emitted from `tick_schema.toml` into `crates/thetadatadx/src/frames_generated.rs` (new file), covering every entry — `CalendarDay`, `EodTick`, `GreeksTick`, `InterestRateTick`, `IvTick`, `MarketValueTick`, `OhlcTick`, `OpenInterestTick`, `OptionContract`, `PriceTick`, `QuoteTick`, `TradeQuoteTick`, `TradeTick`. Column-shape SSOT with the Python slice_arrow path: both generators read `tick_schema.toml` and apply the same field-type → Arrow-dtype mapping, so `ticks.as_slice().to_polars()?` in Rust produces the same DataFrame schema (column order, dtypes, the `QuoteTick.midpoint` virtual column, the contract-id `expiration` / `strike` / `right` tail, the `OptionContract.right` i32 → string projection) as `tdx.stock_history_eod(...).to_polars()` in Python. Dep footprint stays opt-in: `polars = ["dep:polars"]`, `arrow = ["dep:arrow-array", "dep:arrow-schema"]`, `frames = ["polars", "arrow"]`; polars pins to `0.46` with `default-features = false` (no lazy, no parquet, no SQL, no compute kernels) and `arrow-array` / `arrow-schema` pin to `58.1.0` matching `sdks/python/Cargo.toml` so the repo sees a single major version of the arrow family. Opt-in form: `thetadatadx = { version = "8", features = ["polars"] }`.

### Changed

- **Snapshot-kind endpoints now return plain `list[TickClass]` instead of the `<TickName>List` wrapper.** Applies to every endpoint with `subcategory = "snapshot"` or `"snapshot_greeks"` in `endpoint_surface.toml`, plus every `category = "calendar"` + `kind = "parsed"` entry — 20 endpoints total: 4 `stock_snapshot_*`, 11 `option_snapshot_*` (OHLC, trade, quote, open_interest, market_value, + 5 greeks variants + 1 IV variant), 3 `index_snapshot_*`, 3 `calendar_*`. The `<T>List` allocation cost was pure overhead on the latency-sensitive path — callers never chain `.to_polars()` on a 1-row calendar result. Classification is entirely TOML-driven via `helpers::is_snapshot_endpoint`; no hand-curated allowlist, so adding a new snapshot-kind endpoint to the TOML automatically opts it into the fast path on the next generator run. Return-type annotation changes (`list[CalendarDay]` instead of `CalendarDayList`); positional args and kwargs on the public pymethod signature are unchanged.
- **Snapshot pymethods now dispatch via a new `run_blocking_snapshot` helper — bounded `tokio::time::timeout` instead of the 100 ms signal-check ticker.** `run_blocking`'s `tokio::select!` poll loop taxed every sub-100 ms call with 1-5 ms of first-tick jitter in the worst case. `run_blocking_snapshot` drops the ticker entirely: `py.detach { runtime().block_on(tokio::time::timeout(5s, fut)) }`. The 5-second upper bound is a liveness safeguard — every observed production snapshot call completes in <200 ms, so the bound adds zero steady-state cost. Ctrl+C is still honoured after the future resolves or the timeout fires. Emitted by the generator only when `is_snapshot_endpoint` is true; parsed / list / streaming endpoints keep the existing `run_blocking` path unchanged.
- **`run_blocking` signal-check poll cadence reduced from 100 ms to 20 ms.** Drops the worst-case select-wait on short parsed-kind calls from ~100 ms to ~20 ms. `Python::check_signals()` is ~1 µs per call so driving the ticker 5× as often has negligible steady-state cost. Long-running endpoints see no behavioural change beyond a slightly finer-grained Ctrl+C cancellation window. One-line constant edit in `sdks/python/src/lib.rs`; the matching doc-comment is updated.
- **`README.md` / `sdks/python/README.md` — positioning refreshed.** Dropped the old small snapshot / calendar latency caveat now that the fast-path reduces overhead on every measured endpoint. Added a feature-gated Rust DataFrame quickstart example showing `thetadatadx = { version = "8", features = ["polars"] }` plus the chained `ticks.as_slice().to_polars()?` call site.
- **Generator-emitted snapshot fast-path converters (`<tick>_vec_to_pylist`) in `sdks/python/src/tick_classes.rs`.** One helper per snapshot-return tick type (9 total: `calendar_days_vec_to_pylist`, `ohlc_ticks_vec_to_pylist`, `quote_ticks_vec_to_pylist`, `trade_ticks_vec_to_pylist`, `market_value_ticks_vec_to_pylist`, `open_interest_ticks_vec_to_pylist`, `iv_ticks_vec_to_pylist`, `greeks_ticks_vec_to_pylist`, `price_ticks_vec_to_pylist`); one helper per tick type that is NOT reached by any snapshot endpoint is suppressed at generation time to avoid dead-code. Emission is gated on a TOML-derived set computed by the new `endpoints::snapshot_return_types` helper — adding a snapshot endpoint of a new tick type to `endpoint_surface.toml` automatically opts its converter into emission on the next generator run. Row-building body reuses `pyclass_from_tick_expr` from the `<TickName>List.to_list()` path so both surfaces emit byte-identical pylist contents.

## [8.0.5] - 2026-04-22

Endpoint performance fixes discovered during a pre-release performance review.
Four regressions on the MDDS wire surface, all converging on one generator-level
asymmetry: the Rust request builder was sending a different wire shape than the
request contract on option endpoints, and on a subset of calls that
difference tipped the server into an enumeration slow-path. No behaviour changes
on the returned tick data, no signature changes on the SDK surface.

### Fixed

- **`option_list_dates` — duplicate expiration field removed from the request wire shape.** The v3 `OptionListDatesRequestQuery` proto carries both a `ContractSpec` (whose `expiration` is the contract identity) and a top-level `string expiration` field (a vestigial wire field that predates `ContractSpec`). The generator was populating both with the same canonicalized date, which forced the server onto a per-contract enumeration path. Fixed in `build_support/endpoints/render/mdds.rs::mdds_query_field_expr`: when the query message also carries a `ContractSpec`, the top-level `expiration` field now emits `String::new()` to match the request contract. Same one-line generator rule covers every option query message that carries both fields; no hand-written per-endpoint edits.
- **`option_at_time_quote` — duplicate expiration field removed from the at-time quote path.** The same top-level `expiration` duplicate that bottlenecked `option_list_dates` also penalized the at-time-quote path on dense option chains. Same generator-level fix applies: `expiration` on `OptionAtTimeQuoteRequestQuery` now emits `String::new()`.
- **`option_history_greeks_eod` — wire-shape parity restored on the wide-schema path.** Same fix as the two items above; greeks-EOD sent the duplicate `expiration` field through the same code path.
- **`ContractSpec.strike` / `ContractSpec.right` — wildcard sentinels now marshal as literal `"*"` / `"both"` on the wire.** The previous `wire_strike_opt` / `wire_right_opt` mapping reinterpreted the SDK-surface wildcards (`""`, `"*"`, `"0"` for strike; `"both"` for right) as proto-unset optional fields. Upstream request examples populate these fields literally; the v3 server treats an **unset** optional as "enumerate every strike / right for this contract" (slow path) and an explicit `"*"` / `"both"` as "chain-wide lookup" (fast path). Both helpers now always return `Some(...)` with the canonical wildcard literal. No signature changes on the SDK surface; callers continue to pass `"*"` / `"both"` unchanged.

### Changed

- **`README.md` / `sdks/python/README.md` — positioning corrected to measured v8.0.4 bench numbers.** Dropped legacy headline claims from v8.0.0-era measurements and replaced them with endpoint-specific, reproducible notes. Small snapshot / calendar calls are no longer described as speedups because network round-trip time dominates those calls.
- **8.0.2 slice-direct Arrow narrative scoped to builder terminals.** The 8.0.2 changelog bullet ("`.arrow()` / `.pandas()` / `.polars()` feed decoder-owned `Vec<tick::T>` straight into Arrow column builders, peaking RSS at about the tick payload") described the builder-terminal path. The `<Type>List.to_polars()` non-builder terminal also reaches the slice-direct converter (`slice_arrow::<tick>_slice_to_arrow_table`), but the column-builder pass holds both the decoder-owned slice and the column vectors in memory simultaneously. The narrative in both `CHANGELOG.md` and `docs-site/docs/changelog.md` now scopes the memory note to the implementation path that provides it.

## [8.0.4] - 2026-04-22

Pre-release review hotfixes on the Python binding. Four silent bugs on the
hand-written pyo3 glue — Gregorian date validation, Python logging-hierarchy
normalization, async GIL contention on heavy convert paths, and
interpreter-finalization safety on Python 3.13+. No behaviour changes on
the generated endpoint surface; every fix is confined to the hand-written
utility files the endpoint generator layers depend on.

### Fixed

- **`sdks/python/src/chunking.rs` — `Ymd::from_yyyymmdd` accepted Gregorian-impossible dates.** The hand-rolled parser range-checked month `1..=12` and day `1..=31` independently, so `20230229` (Feb 29 in a non-leap year), `20240231` (Feb 31), `20240431` (Apr 31) and every other calendar-invalid combination slipped through. `to_ord` then silently normalized the bogus day to a neighbouring valid one, producing wrong chunk boundaries when the 365-day auto-chunk helper split a range starting or ending on an impossible date. The validator now delegates to `chrono::NaiveDate::parse_from_str(_, "%Y%m%d")`, which enforces leap-year and month-length rules from the canonical Gregorian tables. `chrono` is adopted as a new direct dep on `sdks/python/Cargo.toml` (pinned to `=0.4.44`, `default-features = false`, `alloc` only) and is already a transitive dep via the tzdb chain pulled in by `thetadatadx`, so the crate graph does not gain any new package. Covered by 12 new tests: Feb 29 in 2023/2024/1900/2000, Feb 30, Feb 31, Apr 31, Jun/Sep/Nov 31, month 0/13/99, day 0, and end-to-end rejection through `split_date_range`.
- **`sdks/python/src/logging_bridge.rs` — Rust `tracing` targets were passed to `logging.getLogger` with `::` separators.** Rust `tracing` emits targets as `::`-separated module paths (`thetadatadx::auth::nexus`, `thetadatadx::fpss::io_loop`, …). Python's stdlib `logging` hierarchy is `.`-separated. Consequence: `logging.getLogger("thetadatadx").setLevel(logging.DEBUG)` did NOT propagate to `thetadatadx::auth::nexus` events — Python treated those as unrelated top-level loggers with no parent-level filtering. The v8.0.2 release notes' claim that parent-level `setLevel` filters Rust-side events was therefore false. Fixed by rewriting `target.replace("::", ".")` in the `Layer::on_event` hook before calling `logging.getLogger(...)`. Covered by one new test pinning the transformation on the canonical targets plus a Python-level test that exercises the full `getLogger → setLevel → isEnabledFor` hierarchy propagation with both the post-fix (normalized) and pre-fix (unnormalized) names.
- **`sdks/python/src/async_runtime.rs` — `spawn_awaitable` ran the convert closure on the tokio runtime worker under GIL contention.** The helper's inner async block wrapped `convert` in `Python::attach(|py| convert(py, value))` directly inside the `future_into_py` body, so heavy convert work (e.g. building a 955 237-row `QuoteTickList` pyclass) parked the runtime worker for the duration of the Python-object build. Two concurrent `*_async` calls on the same worker serialized end-to-end on the GIL even though tokio had other workers free. Fixed by offloading the convert closure to `tokio::task::spawn_blocking`, which is tokio's designated lane for synchronous / long-running work — the runtime worker is free to service other endpoints while the current call synthesizes its Python payload on a blocking-pool thread. Join-error handling routes panics through `JoinError::into_panic()` to a `PyRuntimeError` so the shape of the awaitable's error surface is unchanged. The module-level docstring walks through why option A (return `T: IntoPyObject` and let pyo3-async-runtimes handle materialization) was rejected — the 122 generator-emitted callsites in `historical_methods.rs` and the matching templates in `build_support/endpoints/render/python.rs` all pass typed pyclass-wrapper helpers (`strings_to_string_list`, `trade_ticks_to_pyclass_list`, …) that aren't plain `IntoPyObject` impls on `Vec<T>`; routing the existing convert closures to the blocking pool resolves the contention with zero ripple to the helper surface. Covered by a new wall-clock test that fires two concurrent `spawn_awaitable` calls with 100 ms convert closures and asserts the combined elapsed time is less than 1.5× single-task (pre-fix serial behaviour would be ~ 2×).
- **`sdks/python/src/logging_bridge.rs` — `Python::attach` could panic during interpreter finalization on Python 3.13+.** A background Rust thread emitting a `tracing` event during CPython teardown would call `Python::attach`, which panics when the interpreter is mid-finalization (documented pyo3 behaviour, sharpened on 3.13+). The panic took down the process before the layer's existing `Err(_) => return` guard could swallow the resulting logger error. Fixed by switching to `Python::try_attach` (pyo3 0.28 API), which returns `None` when the interpreter is unavailable (finalizing, not initialized, or mid-GC traversal) and lets us silently drop the event. Shutdown-time event loss is an acceptable tradeoff vs. a crash during interpreter exit. Covered by a new test asserting `try_attach` returns `Some` on the live-interpreter path (the regression guard — a revert to plain `attach` would lose the finalization-safety property) and by a documentation note in the module docstring's "Threading model" section.

## [8.0.3] - 2026-04-22

Python-UX polish: DataFrame conversion is now a chain on the returned list
(`tdx.stock_history_eod(...).to_polars()`). The free-function and client-method
`to_polars(ticks)` / `to_arrow(ticks)` / `to_pandas(ticks)` / `to_dataframe(ticks)`
entry points are removed hard — there is now exactly one surface for converting
tick data into a DataFrame.

### Changed

- **Chained DataFrame conversion on every list-returning endpoint.** Every endpoint wraps its result in a typed `<ReturnType>List` pyclass (`EodTickList`, `TradeTickList`, `QuoteTickList`, …, plus `StringList`, `OptionContractList`, `CalendarDayList` for non-tick list returns). The wrapper exposes `.to_polars()`, `.to_arrow()`, `.to_pandas()`, `.to_list()` and the list protocol. Usage is `tdx.stock_history_eod(...).to_polars()` — no intermediate variable, no free-function round-trip. Builder terminals collapse from four parallel `.list()` / `.arrow()` / `.pandas()` / `.polars()` methods to a single `.list()` whose return carries the same chained terminals.

### Removed

- **Free-function and client-method conversion helpers removed.** `thetadatadx.to_polars(ticks)`, `thetadatadx.to_arrow(ticks)`, `thetadatadx.to_pandas(ticks)`, `thetadatadx.to_dataframe(ticks)` and the identically-named methods on the client handle are deleted. Consumers migrate by chaining the terminal off the endpoint return value (`tdx.stock_history_eod(...).to_polars()` in place of `thetadatadx.to_polars(tdx.stock_history_eod(...))`). One path, one SSOT, one place to audit.

### Changed

- **Generator-emitted `_async` methods delegate to a `spawn_awaitable` helper** mirroring the sync `run_blocking` pattern. One call per emit replaces the open-coded `pyo3_async_runtimes::tokio::future_into_py(...)` + `Python::attach` + `map_err(to_py_err)` scaffolding that every `_async` method previously inlined. `sdks/python/src/historical_methods.rs` sheds ~599 lines of duplicated plumbing.
- **Docs-site restructure.** Deleted the standalone benchmark page, the migration-from-thetadata guide, the five per-language `quickstart/*.md` files, and the separate async-python narrative. Replaced with a unified code-group quickstart exposing Rust / Python / TypeScript / Go / C++ via language tabs so one page stays in sync across SDKs.

## [8.0.2] - 2026-04-21

Bigger than a typical patch: ships a P0 decode-correctness fix alongside
a feature-additive wave across the Rust SDK and the Python bindings.
Every surface added here is backward-compatible — no method signatures
change, no types are removed, no client code needs to migrate. The
patch-level version reflects that existing callers continue to compile
unchanged; the additive surface opens new opt-in paths.

### Fixed

- **P11 — `stock_history_trade_quote` / `option_history_trade_quote` silently returned `Ok(vec![])` on non-empty responses.** The v3 MDDS server emits the combined-row pair as `trade_timestamp` / `quote_timestamp`; `tick_schema.toml` declared them as `ms_of_day` / `quote_ms_of_day` with no aliases. `find_header` failed both required-header guards and the parser short-circuited before decoding any row. Added aliases `ms_of_day` ↔ `trade_timestamp`, `quote_ms_of_day` ↔ `quote_timestamp`, `date` ↔ `trade_timestamp`. Verified against a fresh prod capture: AAPL `stock_history_trade_quote` now returns 955 237 rows, SPY option returns 98. Captured-response regression fixtures ship for seven endpoints (`stock_history_trade_quote`, `option_history_trade_quote`, `stock_history_eod`, `option_history_greeks_all`, `option_history_trade`, `option_snapshot_ohlc`, `calendar_open_today`) so the same class of schema drift fails at PR time next release.
- **Decoder audit — `parse_<tick>_ticks` guard no longer drops rows on schema drift.** Generator template and the hand-written `parse_option_contracts_v3` now raise `DecodeError::MissingRequiredHeader` when the `DataTable` carries rows but declares none of the expected columns. Empty responses continue to return `Ok(vec![])` (a holiday with no trades remains a legitimate outcome). Walked every `Vec::new()` / `unwrap_or_default()` call-site in `decode.rs` and `fpss/decode.rs` — the remaining ones are intentional soft-fail accessors (bench / macro) or per-event nibble buffers, flagged as such in the audit report.

### Added

- **Async Python surface — every historical endpoint gains an `_async` companion.** `client.stock_history_eod_async(...)` returns an awaitable built on `pyo3_async_runtimes::tokio::future_into_py`. Sync and async paths share the same `OnceLock<tokio::runtime::Runtime>` singleton — one runtime, one connection pool, one request semaphore.
- **Fluent builders — `tdx.<endpoint>_builder(...)` returns a per-endpoint `#[pyclass]` with chainable setters and `.list()` / `.arrow()` / `.pandas()` / `.polars()` terminals plus `_async` companions.** Builder holds `Arc<thetadatadx::ThetaDataDx>` so every terminal drives the original client without re-authenticating.
- **`decode_response_bytes(endpoint, chunks)`** — generator-emitted `#[pyfunction]` that feeds recorded `Vec<&[u8]>` `proto::ResponseData` frames through the Rust decoder and returns the typed pyclass list, so external parity benches can attribute wall-clock cost between network and decode without an MDDS round-trip. Auto-wired for every endpoint that has a typed decoder.
- **Layered exception hierarchy** — `thetadatadx.ThetaDataError` root plus nine leaves: `AuthenticationError`, `InvalidCredentialsError`, `SubscriptionError`, `RateLimitError`, `SchemaMismatchError`, `NetworkError`, `TimeoutError`, `NoDataFoundError`, `StreamError`. `to_py_err` maps every `thetadatadx::Error` variant (plus gRPC status strings) onto the correct leaf. `#[non_exhaustive]` catch-all.
- **Python logging bridge** — `tracing_subscriber::Layer` that forwards every `tracing` event to `logging.getLogger(target).log(...)`. Filter-first via `isEnabledFor(level)` so default WARN loggers pay a single bool check per event with no formatting. Installed at module init.
- **Slice-based Arrow fast path on builder terminals** — `.arrow()` / `.pandas()` / `.polars()` (and their `_async` companions) feed the decoder-owned `Vec<tick::T>` straight into the Arrow column builders, skipping the pyclass-list double-buffer. The `<Type>List.to_polars()` terminals on the typed-list wrapper also reach this slice-direct path; the column-builder pass holds the decoder-owned slice and the column vectors simultaneously. Schema is bit-identical to the pyclass-list path so downstream consumers alias either source interchangeably. (Language narrowed from the initial memory-footprint claim in v8.0.5 — see that entry.)
- **`RetryPolicy`** — initial_delay 250 ms, max_delay 30 s, max_attempts 5, full jitter by default. Retries only on `Unavailable` / `DeadlineExceeded` / `ResourceExhausted`. Unit-tested backoff math, jitter bounds, and the `disabled()` shortcut.
- **Session auto-refresh** — `auth::SessionToken` holds the session UUID behind a `tokio::sync::Mutex` + monotonic version counter. On `Unauthenticated` the retry loop snapshots the token, re-auths via Nexus, swaps the UUID in place, and retries exactly once. A second 401 fails permanently. Concurrent 401s dedupe into a single Nexus round-trip via version-check short-circuit.
- **Environment-variable config matrix** — `DirectConfig::production()` layers env vars on the hardcoded defaults: `THETADATA_MDDS_HOST`, `THETADATA_MDDS_PORT` (upstream-compat), plus DX extensions `THETADATA_NEXUS_URL`, `THETADATA_FPSS_HOST`, `THETADATA_FPSS_PORT`, `THETADATA_CLIENT_TYPE`. Precedence: explicit builder setter > env var > hardcoded default.
- **Optional `metrics-prometheus` cargo feature** — pulls `metrics-exporter-prometheus` and wires an HTTP `/metrics` listener on `DirectConfig::metrics_port`. Exporter starts inside `ThetaDataDx::connect` so the first RPC counter is already covered. Feature-gated; default build stays dep-free.
- **Vendor docstring lift** — 60 endpoint docstrings threaded through `endpoint_surface.toml` → model → parser → generator so sync / async / builder variants share one SSOT. Attribution recorded in `docs/ATTRIBUTION.md`.
- **`split_date_range(start, end)`** — pure Rust 365-day-window splitter exposed as `thetadatadx.split_date_range` for tooling and the auto-chunk pre-flight. Tested on single-day, exact boundary, multi-year contiguity, leap-day, and invalid input.
- **Capture fixtures** — seven `tests/fixtures/captures/<endpoint>.{pb.zst,meta.toml}` pairs anchor expected row counts, exact server header lists, and first-row field values. `tests/test_decode_captures.rs` feeds each fixture through the same `decode_data_table` → tick-parser path the `MddsClient` uses and asserts three invariants per fixture. Two regression guards ensure `MissingRequiredHeader` fires on non-empty schema drift and empty responses still return `Ok(vec![])`.

### Changed

- **Regenerated SDK surfaces** — `historical_methods.rs`, `tick_arrow.rs`, `decode_bench.rs` rebuilt off the merged generator. Byte-identical check passes.
- **Parser generator raises `MissingRequiredHeader` on schema drift** — the generated `parse_<tick>_ticks` template no longer silently returns `Ok(vec![])` when a required column is absent on a non-empty `DataTable`. Empty responses continue to pass through unchanged.

## [8.0.1] - 2026-04-21

### Fixed

- **`tdbe` bumped to 0.11.0 to publish the new `SecType::Unknown` variant to crates.io** — the 8.0.0 release added `SecType::Unknown` (empty-contract sentinel) but kept `tdbe` at `0.10.0`. `cargo publish --verify` for `thetadatadx 8.0.0` pulled `tdbe = 0.10.0` from the registry, which does not contain `Unknown`, and failed with `E0599`. The `thetadatadx`, `ffi`, `cli`, `mcp`, `server`, `py`, and `napi` crates bump to `8.0.1` so all three ecosystems (crates.io, PyPI, npm) end up on matching, publishable versions. npm and PyPI had already published 8.0.0 successfully; crates.io 8.0.0 was never materialized.
- **FPSS handshake surfaces every typed control frame** — `wait_for_login` collects `Connected` (code 4), `Ping` (code 10), `ReconnectedServer` (code 13), and `Restart` (code 31) frames that arrive before `METADATA` into an ordered buffer; the I/O loop drains the buffer onto the event bus before emitting `LoginSuccess` so user callbacks see the exact wire order. Previously all typed control frames except `Connected` were silently dropped by the handshake's trace-and-continue branch. Applies to the initial login AND the reconnect-path login.
- **Reconnect-path login short-circuits on permanent rejection** — `LoginResult::Disconnected(reason)` during the reconnect handshake now consults `reconnect_delay(reason)` as the single source of truth for "no retry will fix this" and exits the I/O loop with `shutdown = true` + a `FpssControl::Disconnected` event. Previously bad credentials burned `MAX_RECONNECT_ATTEMPTS` (5) cycles of `Reconnecting` / `Disconnected` noise before giving up.
- **Mid-frame reader yields to the command drain on a bounded budget** — `FrameReadState` threads partial-frame progress across `read_frame_into` calls. A new `MID_FRAME_DRAIN_WINDOW_MS = 200` (4× the 50 ms drain cadence) caps the total wall time spent retrying a partial frame before the reader yields control to the I/O loop, which drains outbound commands and re-enters the reader with the preserved state. Previously a trickling sender could block heartbeats / user writes for up to `READ_TIMEOUT_MS` (10 s) because the per-stall deadline reset on every successful byte.
- **`Contract::from_str` accepts 1..=16-char roots** — `validate_root` widens from `1..=6` to `1..=16` chars, matching the wire-codec upper bound in `Contract::to_bytes()` / `Contract::from_bytes()`. `from_str` / `to_bytes` / `from_bytes` now round-trip symmetrically; the wire is the ground truth. Round-trip coverage for every length 7..=16 added.
- **Auth email redacted across `Debug` and tracing** — `AuthResponse::Debug`, `AuthUser::Debug`, and the `authenticate()` tracing line that previously rendered `email = %creds.email` now emit `<redacted>` / a prefix-only `ali...@example.com` form. Full emails no longer land in panic output, structured logs, or crash dumps.
- **Credentials parsing pipeline wraps every transient in `Zeroizing`** — `from_file` reads the file into `Zeroizing<String>` so the on-disk password bytes are wiped on drop; `parse()` / `new()` wrap the intermediate owned password `String` in `Zeroizing` before assigning to the struct. A panic or early-return between allocation and struct construction still wipes the plaintext on unwind. Completes the coverage the 8.0 release notes claimed; the previous implementation zeroed only the final `Credentials.password` field.
- **Empty-contract sentinel documentation unified** — `FpssData::{Quote,Trade,OpenInterest,Ohlcvc}` docstrings now promote `contract.sec_type == SecType::Unknown` as the canonical check for the empty-contract placeholder (matching `fpss::decode`'s guidance). `root.is_empty()` is retained as a secondary mention but no longer the primary documented check -- it was brittle against future root-charset relaxations.

## [8.0.0] - 2026-04-21

Major release. Three headline groups land in one pass:

1. **FPSS events now carry a parsed `Arc<Contract>`** (#389). Every `FpssData::{Quote,Trade,OpenInterest,Ohlcvc}` replaces the `symbol: Arc<str>` field with `contract: Arc<Contract>`, and the `contract_map` lifts from `HashMap<i32, Contract>` to `HashMap<i32, Arc<Contract>>`. Decoded events carry the full typed contract (`root`, `sec_type`, `exp_date?`, `is_call?`, `strike?`) at refcount cost rather than a bare symbol string; every language SDK exposes a matching typed `Contract`. `SecType::Unknown` is added as the sentinel for not-yet-assigned contract IDs so exhaustive matches stay sound.
2. **`impl FromStr for Contract` plus historical FPSS subscribe shortcuts** (#389). `"AAPL".parse::<Contract>()?` yields a stock contract; `"SPY   260417C00550000".parse::<Contract>()?` parses the OCC 21-char option identifier (2000–2099 scope, trim-tolerant 20-char pad, every parse failure returns `Error::Config` with the offending input). `FpssClient` and `ThetaDataDx` gained per-security subscribe and unsubscribe shortcuts — one-liners over the underlying typed subscribe machinery.
3. **FPSS control codes 4 / 10 / 13 / 31 decode into typed variants** (#389). `FpssControl::{Connected, Ping { payload }, ReconnectedServer, Restart}` replace the `UnknownFrame` fallthrough these codes used to hit. The `Restart` arm clears delta decode state so subsequent ticks no longer decode against a stale baseline. FFI kind tags grow 13..=16; every SDK mirrors the new constants.

### Removed

- **`FpssData::{Quote,Trade,OpenInterest,Ohlcvc}::symbol` removed** (#389) — migrate to `event.contract.root` for the symbol string; option fields `exp_date`, `strike`, `is_call` are now direct attribute access on `contract`.
- **`FpssControl::ContractAssigned { contract: Contract }` → `{ contract: Arc<Contract> }`** (#389) — pattern matches that bind by value must bind by `Arc<Contract>` and clone via `Arc::clone` if owned value was previously expected.
- **`contract_lookup()` / `contract_map()` return `Arc<Contract>` / `HashMap<i32, Arc<Contract>>`** (#389) — was by-value `Contract` / `HashMap<i32, Contract>` before. Call-site fix: drop one layer of `.clone()`.
- **`Restart` (code 31) and `Connected` (code 4) frames no longer arrive as `UnknownFrame`** (#389) — handlers matching on `FpssControl::UnknownFrame { code: 4 | 10 | 13 | 31, .. }` need updated arms or a fallthrough on the new typed variants.
- **`SecType::Unknown` variant added to `tdbe::types::enums::SecType`** (#389) — exhaustive `match` statements without a wildcard arm must add a branch.
- **`FpssData::{Quote,Trade,OpenInterest,Ohlcvc}` no longer `derive(Clone)` on the Python SDK pyclasses** (#389) — `Py<Contract>` needs a GIL token for cloning; the derive was dead code (events flow one-way from Rust to Python).

### Changed

- **License switched to Apache-2.0** across every `Cargo.toml`, `package.json`, `pyproject.toml`, and the top-level `LICENSE`. `deny.toml` allowlist cleaned up accordingly.
- **Top-level `README.md` rewritten** as a professional SDK landing page: tagline, highlights, per-SDK quickstart (Rust / Python / TypeScript / Go / C++), architecture diagram, Java parity note. Neutral technical framing throughout.
- **`docs/java-parity-checklist.md` added** as the single source of truth for Java terminal parity — feature-by-feature table (parity / deviation / partial) covering wire protocol, authentication, control events, reconnection, FPSS streaming, tick decoding, Greeks, validation, and intentional improvements over the Java terminal. Three earlier stand-alone documents (`docs/jvm-deviations.md`, `docs/java-class-mapping.md`, and a prior protocol-archaeology note) folded in.
- **Internal `docs/dev/` design notes removed** (no longer load-bearing).
- **`DirectClient` renamed to `MddsClient`** (#383) — the historical-data gRPC client now carries the name of the service it actually speaks to (MDDS = Market Data Delivery Service). `use thetadatadx::DirectClient` call sites break; update to `use thetadatadx::MddsClient`. The `DirectConfig` associated config type keeps its name. High-level consumers of `ThetaDataDx` (Python / TypeScript / Go / C++ / Rust facade) are unaffected.
- **`crates/thetadatadx/src/direct.rs` split into `crates/thetadatadx/src/mdds/` module** (#383) — 732-line monolith broken into six concern-separated files (`client`, `endpoints`, `endpoint_arg_ext`, `normalize`, `validate`, `mod`). Pure move; wire behavior unchanged; all 304 workspace tests pass.
- **`crates/thetadatadx/proto/external.proto` renamed to `mdds.proto`** (#385) — the proto file described only MDDS (`BetaEndpoints`) messages; the filename now reflects that. `tonic::include_proto!("beta_endpoints")` and every downstream Rust import resolve unchanged (package declaration drove the module name, not the filename). `build.rs`, `proto_parser`, generated-header strings, `MAINTENANCE.md`, `CONTRIBUTING.md`, `ROADMAP.md`, and every `docs/` reference updated (17 files, 51 lines).
- **`fpss_event_schema.toml` schema version bumped 2 → 3** (#389) — carries the new nested `Contract` column type for every data-event variant. Every SDK Contract type (Python pyclass, TypeScript `#[napi(object)]`, Go struct with `*int32`/`*bool` pointer optionals, C/C++ typedef with `has_*` tagged-optional flags, Rust FFI `#[repr(C)] TdxContract` with `CString`-backed root pointer) is generator-emitted from the updated schema.

### Added

- **Parsed `Arc<Contract>` on every FPSS data event** (#389) — `FpssData::{Quote,Trade,OpenInterest,Ohlcvc}::contract: Arc<Contract>` replaces the former `symbol: Arc<str>`. Option events now expose `event.contract.exp_date`, `.strike`, `.is_call` without a second lookup; stock events read `event.contract.root`. Refcount-only per-event clone. Mirrors `net.thetadata.fpssclient.Contract` from the Java terminal without the JSON round-trip. `contract_lookup` and `contract_map` return `Arc<Contract>` / `HashMap<i32, Arc<Contract>>` on every SDK.
- **`impl FromStr for Contract`** (#389) — `"AAPL".parse::<Contract>()?` yields a stock contract (1..=6 ASCII A-Z, `.` permitted); `"SPY   260417C00550000".parse::<Contract>()?` parses the OCC 21-char institutional option identifier (6-byte root right-padded with spaces, 6-byte YYMMDD century-adjusted to 2000–2099 YYYYMMDD, single-byte `C`/`P`, 8-byte strike in thousandths of a dollar). 20-byte inputs are tolerated with a trailing-space pad. Parse failures return `Error::Config` naming the offending input and the specific failure (length, root charset, expiration digits, right byte, strike digits).
- **Historical FPSS subscribe shortcuts** (#389) — per-security subscribe and matching unsubscribe counterparts were added on `FpssClient` and `ThetaDataDx`. Each wraps the `Contract` builder plus the typed `subscribe` / `unsubscribe` call into one line; no duplicate request-ID or frame-build machinery.
- **Typed decoding of FPSS control codes 4 / 10 / 13 / 31** (#389) — `FpssControl::Connected` (4), `FpssControl::Ping { payload }` (10), `FpssControl::ReconnectedServer` (13 — server-side ack, distinct from the client-side auto-reconnect `Reconnected` variant), and `FpssControl::Restart` (31) replace the `UnknownFrame` fallthrough these codes used to hit. The `Restart` arm clears delta decode state so subsequent ticks no longer decode against a stale baseline. FFI `TdxFpssControl` kind tags grow 13..=16; Go `FpssCtrl*` constants mirror them.
- **`Contract` type surfaced on every language SDK** (#389) — Python pyclass (`Py<Contract>` embedded in each event, cloned via `clone_ref(py)`), TypeScript `#[napi(object)]`, Go struct with `*int32` / `*bool` pointer optional fields, C/C++ typedef with `has_*` tagged-optional flags, Rust FFI `#[repr(C)] TdxContract` with a `CString`-backed `root` pointer. `Contract.sec_type == SecType::Unknown` is the sentinel for not-yet-assigned contract IDs; every SDK exposes the new variant.
- **`thetadatadx.to_arrow(ticks) -> pyarrow.Table`** (#379) — new public Python entry point that returns the Arrow table directly, for users wiring DuckDB / Arrow-Flight / cuDF / polars-arrow pipelines without a pandas or polars roundtrip. Requires `pip install thetadatadx[arrow]` (pyarrow only).
- **`hint=` kwarg on `to_arrow` / `to_dataframe` / `to_polars`** (#380) — optional `hint: str` names the tick pyclass (e.g. `hint="EodTick"`) so the Arrow schema is materialised even when the input list is empty. Previous empty-list calls returned a zero-column table; downstream pipelines asserting a fixed schema now get the right columns on empty market-hours windows.
- **Generated `#[new]` constructors on every tick pyclass** (#379) — `EodTick(ms_of_day=1, volume=1_000_000, ...)`, `OhlcTick(...)`, `TradeTick(...)`, etc. All fields are keyword-only with zero / empty-string defaults, so test fixtures and user-side data construction are possible from Python (previously pyclass instances could only be produced by Rust endpoints).
- **`AllGreeks` pyclass** (#378) — `all_greeks(...)` now returns a frozen `AllGreeks` pyclass with 22 `#[pyo3(get)]` f64 fields (value / iv / delta / gamma / theta / vega / rho plus every second- and third-order Greek) and a `__repr__` showing the six most-referenced values. Replaces the untyped 22-key `PyDict` that was the sole remaining dict-typed public return in the Python SDK.
- **`__repr__` on every FPSS event pyclass** (#380) — `Ohlcvc`, `Quote`, `Trade`, `OpenInterest`, `Simple`, `RawData` now render up to six live field values at the Jupyter / print boundary (matching the pattern already on tick pyclasses). Opaque `Vec<u8>` payloads and `received_at_ns` skipped as noise.
- **`dropped_events()` counter on every streaming SDK** (#377) — `Arc<AtomicU64>` hoisted onto `ThetaDataDx` survives reconnect and is exposed as `tdx.dropped_events() -> int` (Python), `tdx.droppedEvents(): bigint` (TypeScript), `client.DroppedEvents() uint64` (Go), `client.dropped_events() -> uint64_t` (C++), `tdx_fpss_dropped_events(handle)` / `tdx_unified_dropped_events(handle)` (FFI). Previously silent `let _ = tx.send(buffered)` call-sites now bump the counter and emit `tracing::debug!` on target `thetadatadx::sdk::streaming`.
- **`POST /v3/system/shutdown` endpoint on `thetadatadx-server`** (#377) — graceful shutdown over a privileged route gated by a per-startup random UUID `X-Shutdown-Token` header (constant-time compared via `subtle::ConstantTimeEq`). Prints the token to stderr at startup only; never into structured logs. Dedicated governor allows one attempt per hour, burst 3. Method is `POST` (not `GET`) so the action is neither cached nor prefetched.

### Changed

- **DataFrame adapter migrated to Apache Arrow columnar pipeline** (#379) — `to_dataframe(ticks)` / `to_polars(ticks)` / `to_arrow(ticks)` build a single `arrow::RecordBatch` in Rust and hand it to pyarrow via the Arrow C Data Interface (zero-copy at the pyo3 boundary). pandas 2.x aliases the numeric columns in place; polars consumes via `polars.from_arrow`. At 100k x 20 `EodTick` rows wall-clock drops from ~300-500 ms (legacy dict-of-lists) to ~8 ms — substantially. SSOT preserved: Arrow schema + converters are generated from `tick_schema.toml`; no hand-maintained Arrow code.
- **Per-endpoint DataFrame convenience wrappers removed** (#379) — the four per-endpoint `stock_history_{eod,ohlc,trade,quote}` Rust-tick-slice fast-path helpers on `ThetaDataDx` were deleted. The unified recipe is one extra line with identical performance:

  ```python
  ticks = client.stock_history_eod("AAPL", "20240101", "20240301")
  df    = thetadatadx.to_dataframe(ticks)   # Arrow-backed, zero-copy on pandas 2.x
  pdf   = thetadatadx.to_polars(ticks)      # Arrow-backed, zero-copy
  table = thetadatadx.to_arrow(ticks)       # DuckDB / cuDF / Arrow-Flight
  ```

  Single code path, single generator, single test surface — 100% SSOT restored on the Python DataFrame surface.
- **Deleted** `sdks/python/src/tick_columnar.rs` (the old PyDict-based emission) (#379) — replaced end-to-end by the generator-emitted `sdks/python/src/tick_arrow.rs`. `pip install thetadatadx[pandas]` / `[polars]` now pull `pyarrow>=14.0` alongside the DataFrame library; `pip install thetadatadx[arrow]` is the pyarrow-only extras bundle.

### Changed

- **Historical endpoints now return `list[TickClass]` instead of a columnar `dict[str, list]`** (#364 / #365). The 53 tick-returning historical methods (list endpoints returning scalar `Vec<String>` — symbols, dates, expirations, strikes — are unchanged) in the Python SDK (`stock_history_eod`, `option_history_trade`, `calendar_*`, ...) now return a Python list of typed pyclass objects — `EodTick`, `TradeTick`, `QuoteTick`, `OhlcTick`, `TradeQuoteTick`, `OpenInterestTick`, `MarketValueTick`, `GreeksTick`, `IvTick`, `PriceTick`, `CalendarDay`, `InterestRateTick`, `OptionContract`. Brings the Python SDK into line with Rust core, TypeScript, Go, and C++ FFI. Migration:

  ```python
  # before
  ticks = tdx.stock_history_eod("AAPL", "20240101", "20240301")
  close = ticks["close"][i]            # string key, silent typo failures

  # after
  ticks = tdx.stock_history_eod("AAPL", "20240101", "20240301")
  close = ticks[i].close               # attribute access, typed
  ```

  `to_dataframe(ticks)`, `to_polars(ticks)`, and `to_arrow(ticks)` transparently pivot the new shape into a pandas / polars frame or a `pyarrow.Table`.

### Changed

- **C++ `TdxFpssEvent` field order realigned with Rust + Go** (#376) — the hand-written `TdxFpssEvent` in `sdks/cpp/include/thetadx.h` declared `{ kind, quote, trade, open_interest, ohlcvc, control, raw_data }` while the Rust generator (and the Go C header) emits `{ kind, ohlcvc, open_interest, quote, trade, control, raw_data }`. Every `event->quote.*` / `event->trade.*` / `event->ohlcvc.*` access in existing C++ consumers was reading from the wrong offset — data corruption with no compile-time signal. `thetadx.h` now `#include`s the generator-emitted `fpss_event_structs.h.inc` (byte-identical to the Go C header) and `thetadx.hpp` gains `static_assert(offsetof / sizeof)` covering every field of every `TdxFpss*` struct. Any future drift is compile-fatal.
- **Go `FpssControlData` renamed to `FpssControl`, `FpssOpenInterest*` → `FpssOpenInterest`** (#376) — Go-idiomatic naming on the mirror struct set. Callers referencing the old names will fail to compile; rename one-for-one. The nested field names on `FpssEvent` (`ev.RawData.Code`, `ev.RawData.Payload`) are unchanged.

### Changed

- **`thetadatadx::direct` module removed; replaced by `thetadatadx::mdds`** — the 732-line flat `src/direct.rs` is split into a concern-separated `src/mdds/` module that mirrors the existing `fpss/` layout: `client.rs` (struct + connect), `stream.rs` (gRPC response helpers), `validate.rs` (param validators), `normalize.rs` (wire-format canonicalizers + `contract_spec!` macro), `endpoints.rs` (generated `include!` sites). The generator module `build_support/endpoints/render/direct.rs` is renamed to `render/mdds.rs` and now emits `mdds_*_generated.rs` into `OUT_DIR`; the template directory `templates/direct/` is renamed to `templates/mdds/`. "MDDS" is the actual upstream gRPC service name — "direct" conveyed nothing.
- **`DirectClient` renamed to `MddsClient`** — the struct inside the (now) `mdds/` module takes its module's name. Re-exported at the crate root as `thetadatadx::MddsClient`. `ThetaDataDx` still `Deref<Target = MddsClient>`s, so every historical endpoint method is reached unchanged via the unified client.

### Changed

- **`thetadatadx-server`: governor layer is now outermost, rate-limited traffic short-circuits first** (#377) — axum `.layer(X).layer(Y)` makes Y the outer wrapper, so the previous `ConcurrencyLimit → BodyLimit → Governor` order had the per-IP limiter innermost. Every rate-limited request still consumed a concurrency permit and ran the body-length check before being rejected. Reordered so the governor runs first; body-limit and concurrency gates are only touched by allowed traffic.
- **`thetadatadx-server`: `PeerIpKeyExtractor` on the REST + WS routers** (#377 / #378) — the per-IP rate limiter now keys on the real TCP socket source instead of the forwarded-header-trusting extractor used before. The server defaults to `127.0.0.1` without a trusted reverse proxy in front, so trusting `X-Forwarded-For` / `X-Real-IP` / `Forwarded` let a local attacker cycle fake IPs and bypass the per-IP rate limit. Module doc comment spells out the deployment policy.
- **`thetadatadx-server`: `BoundedQuery<N>` extractor caps query-string params during parse** (#378) — the previous check ran after axum's `Query<HashMap<String, String>>` had already parsed the entire query string into a HashMap, so a `?a=1&b=2&...` flood still allocated MB+ before hitting the count check. `BoundedQuery<32>` counts `&`-delimited pairs on the raw URI before `serde_urlencoded::from_str`, rejects over-limit with 400, and caps HashMap capacity.
- **`thetadatadx-server`: WS subscribe + every REST validator now run `ensure_no_control_chars` + per-field length caps** (#377) — symbol / root ≤ 16, expiration == 8 (YYYYMMDD), strike ≤ 10, right == 1, date == 8, venue ≤ 8. Returns 400 with a descriptive error, never 500. Unknown query-param names surface the real name in the error instead of an opaque `"parameter"` fallback.
- **`thetadatadx-server`: REST global concurrency limit 256, per-IP governor 20 rps / burst 40, body limit 64 KiB, WS text-frame cap 4 KiB** (#377 / #378) — explicit layers on both routers. Legitimate subscribe commands are <200 B; 4 KiB is generous for pathological clients.
- **`thetadatadx-server`: shutdown rate limit fixed — one token per hour, burst 3** (#377 follow-up) — `per_second(3600)` treats the argument as "requests per second", so the "3 attempts per hour" config was actually allowing ~3600 rps. Switched to `.period(Duration::from_secs(3600))`; constant renamed to `SHUTDOWN_REPLENISH_PERIOD`.
- **`thetadatadx-server`: hot-path `String::clone` eliminated on FPSS TOCTOU contract map** (#378) — the broadcast path now holds `HashMap<i32, Arc<Contract>>` instead of `HashMap<i32, Contract>`; mpsc channel carries `(FpssEvent, Option<Arc<Contract>>)`. Hot-path clone is an `Arc` refcount bump instead of a `String` allocation. Micro-bench (100k lookups): 26 ns/op → 22 ns/op, zero hot-path heap allocations. Regression test `arc_contract_clone_is_refcount_bump_not_string_alloc` asserts `Arc::as_ptr` equality to prevent future regressions.
- **TypeScript `const enum FpssEventKind` removed** (#376) — the generated enum broke downstream consumers with `"isolatedModules": true` in `tsconfig.json` (all modern Vite / esbuild / ts-jest / Next.js setups). `FpssEvent.kind` is now `pub kind: &'static str` with a `#[napi(ts_type = "'ohlcvc' | 'open_interest' | 'quote' | 'trade' | 'simple' | 'raw_data'")]` override. Zero-allocation preserved; discriminated-union narrowing unchanged.
- **`go.mod` toolchain bumped to 1.23** (#378) — Go 1.21 released mid-2023; CI matrix already runs 1.23. Node.js `engines.node` bumped from `">= 18"` to `">= 20"` (Node 18 EOL 2025-04-30).
- **`paste` crate replaced by `pastey`** (#377) — upstream `paste` was archived on 2024-10-07 (RUSTSEC-2024-0436). `pastey = "0.2.1"` is the actively-maintained successor; API compatible (`::paste::paste!` → `::pastey::paste!`). Single call-site in `crates/thetadatadx/src/macros.rs`.

### Fixed

- **FFI boundary catches Rust panics** (#380) — zero `catch_unwind` existed across the FFI crate before this change. A Rust panic crossing an `extern "C"` boundary on Rust 1.81+ aborts the host process — C / Go / Python / C++ callers died with no way to recover. New `ffi_boundary!` macro wraps every extern body in `std::panic::catch_unwind(AssertUnwindSafe(|| { ... }))`. Panic payloads are downcast to `&'static str` then `String`, routed to `tracing::error!` on target `thetadatadx::ffi::panic`, written to the thread-local `LAST_ERROR` slot via the existing `set_error`, and the fn returns the caller-declared default (`ptr::null_mut()` / `-1` / `0` / sentinel-empty-array). **Coverage: 145 production `extern "C"` functions wrapped** — 84 in `ffi/src/lib.rs` plus 61 in the generated `ffi/src/endpoint_with_options.rs`. Generator-emitted so future regeneration preserves parity. Regression tests at `ffi/tests/panic_boundary.rs`.
- **Python `next_event(timeout_ms)` honours Ctrl+C within 100 ms** (#380) — previously the generator emitted a single `recv_timeout(Duration::from_millis(timeout_ms))` with the GIL released for the full user-supplied timeout (up to 5 minutes), so Ctrl+C was swallowed for the duration of the wait. `build_support/sdk_surface.rs` now emits a 100 ms polling loop that calls `Python::check_signals()?` per iteration and returns on deadline.
- **`ThetaDataDx::new` constructor is cancellable** (#380) — swapped `run_in_tokio_blocking` for `run_blocking(py, async { connect(...).await })` so a TLS / auth handshake hang stays Ctrl+C-interruptible.
- **FPSS TLS: SPKI pinning replaces `NoVerifier`** (#377) — `PinnedVerifier` parses the leaf cert via `x509-parser`, computes SHA-256 over the SubjectPublicKeyInfo DER bytes, and constant-time compares (`subtle::ConstantTimeEq`) against the captured `FPSS_SPKI_SHA256` (verified identical across prod `nj-a:20000` / `nj-b:20000`, dev `:20200`, stage `:20100` — single keypair across every FPSS environment). Rejects with `CertificateError::NotValidForName` on hostname mismatch (allowlist) or `RustlsError::General("FPSS SPKI pin mismatch: ...")` on pin mismatch. `verify_tls12_signature` / `verify_tls13_signature` delegate to rustls' proper signature verification. Previously any on-path attacker terminating TLS to `nj-a.thetadata.us:20000` could present any cert and harvest the plaintext `StreamMsgType::Credentials` frame.
- **Password `Zeroizing<String>`** (#377) — `Credentials.password` wrapped in `zeroize::Zeroizing<String>`. Every clone (`ThetaDataDx`, `io_loop`, reconnect re-serialise) now wipes the backing buffer on drop. Core dump / `/proc/<pid>/mem` no longer recovers the password after `Credentials` drops. `Deref<Target = str>` means call-sites are unchanged.
- **CSV formula injection defused on `thetadatadx-server` exports** (#377) — `escape_csv_field` now prefixes cells whose first byte is `=`, `+`, `-`, `@`, or `\t` with a single-quote `'` and encloses in CSV quotes. Defuses `=cmd|'/C calc'!A1`, `@SUM(A1:A10)`, `+1+cmd|...` etc from executing in Excel downloads. Regression test covers all five payload shapes.
- **FPSS `io_loop`: Java-parity mid-frame read retry with per-read deadline reset** (#370) — previously a mid-frame read timeout desynced the decoder. The client now retries transparently with the per-read deadline reset, matching the Java terminal's reconnect behaviour.
- **WS subscribe strike / expiration use `i32::try_from`** (#377) — client-supplied expiration / strike no longer silently narrow via `as i32`. Returns `REQ_RESPONSE { response: "ERROR", ... }` with a descriptive message on overflow. Validates `exp` against `[19000101, 21000101]` YYYYMMDD bounds and `strike > 0` before building the FPSS frame (#378).
- **`validate_generic_named` sanitises parameter names in error messages** (#377 / follow-up) — ANSI escape sequences / control chars in a user-supplied param name can no longer escape into terminal-rendered log output. Names are passed through `sanitize_param_name` (ASCII alphanumeric + `_` + `-`).
- **Shutdown token constant-time compare** (#377) — `tools/server/src/state.rs::validate_shutdown_token` swapped `==` for `subtle::ConstantTimeEq::ct_eq`. Timing oracle on UUID prefix closed.
- **Reconnect-path write errors are surfaced, not masked** (#377) — `crates/thetadatadx/src/fpss/io_loop.rs` had `let _ = write_raw_frame_no_flush(...)` silently dropping write failures on reconnect command-drain. Now `tracing::warn!` with `error = %e, frame_code = ?frame.code`.
- **FFI reconnect paths surface resubscribe errors** (#378) — unified + FPSS reconnect paths previously silent-dropped resubscribe errors; now `tracing::warn!` with `error`, `kind`, and contract context.
- **Python `Credentials.__repr__` redacts the email** (#377 / #378) — was `Credentials(email="user@example.com")`; email leaked into Jupyter, pytest output, and crash logs. Now `Credentials(email=<redacted>)`. Matches the redacted `Debug` impl in `crates/thetadatadx/src/auth/creds.rs`.
- **CSV headers union across rows** (#376) — `tools/server/src/format.rs` seeded column keys from the first row only; mixed-type queries (index rows without `expiration` / `strike` / `right` ahead of option rows with them) silently dropped those columns. Headers now union across every row via `BTreeSet` (sorted for free).
- **FPSS `Simple` control events carry `event_type` + nullable `detail` / `id`** (#378) — OpenAPI `Control` variant was documenting the internal numeric `kind: int32`, which no SDK surfaces. Aligned to the client-facing shape (`kind: "simple"` + `event_type` enum + nullable `detail` / `id` + `received_at_ns`).
- **Python `greeks.py` example + README quick-start use attribute access on `AllGreeks`** (#380) — `g['iv']` / `g['delta']` dict subscripts would have crashed at runtime because `AllGreeks` is a frozen pyclass without `__getitem__`. Rewritten to `g.iv`, `g.delta`, etc.
- **Typed `list[TickClass]` examples across every endpoint page** (#378) — ~50 files under `docs-site/docs/historical/` had stale dict-key Python examples (subscript access on the old columnar shape). Switched to attribute access on the typed pyclass surface. `scripts/fpss_smoke.py` / `scripts/fpss_soak.py` likewise switched from dict subscript on streaming events to attribute access (both scripts are wired into live CI).

### Security

- **FPSS TLS authenticity anchored on captured SPKI pin, no longer trust-on-first-use** (#377) — see `Fixed` above. Cert rotation tolerated as long as the keypair stays; expiry sidestepped entirely (current ThetaData leaf expired 2024-01-12). Six new tests cover captured-leaf positive, hostname mismatch rejection, malformed-cert rejection, and openssl fingerprint reproducibility.
- **Cargo-deny advisory / licence / drift gates in CI** (#377) — new `.github/workflows/security-audit.yml` runs RustSec `audit-check` on PR + push + weekly Monday 03:00 UTC cron + manual dispatch. New `cargo-deny` job reads policy from `deny.toml` (advisories deny, licences allowlist, bans duplicates warn, sources crates.io only). New `drift-injection` job runs `scripts/test_drift_injection.sh` which flips `bid` ↔ `ask` in the FPSS schema, regenerates, and verifies the C++ `static_assert(offsetof)` guards fail the cmake build.

### Changed

- **Generator audit cleanup** (#380) — `PYTHON_TICK_ARROW_DIRECT_TYPES` constant + `render_python_tick_arrow_batch_fn` (~70-line emitter) were orphaned by the `*_df` removal in #379 and survived only because of the module-level `#![allow(dead_code)]` umbrella. Deleted. The trait-driven `pyclass_list_to_arrow_table` path is the sole public DataFrame entry point, backed by `<T as ArrowFromPyclassList>::read_batch`. `render_python_tick_arrow` doc rewritten to describe the two still-emitted surfaces (`arrow_schema_for_qualname` + `pyclass_list_to_arrow_table`). `clippy::type_complexity` on a 4-tuple in `sdk_surface.rs` cleared via a `MethodShape<'a>` alias.
- **Go layout regression: `TestTickFieldOffsets` covers every tick mirror field** (#376) — the previous `ffi_layout_test.go` only asserted total struct `sizeof`; same-size field reorders (e.g. swapping two i32 slots) passed the test while silently corrupting data. FPSS mirror types were not tested at all. cgo-typed FPSS offset asserts moved into `tick_ffi_mirrors.go::init()` (Go forbids cgo in `_test.go`).
- **Full stale-data sweep + i64 widening across every doc surface** (#375 / #378) — `OhlcTick` / `EodTick` volume + count widened from `i32` to `i64` (#372 on the Rust side). Docs updated across `docs/api-reference.md`, `docs-site/docs/api-reference.md`, `docs-site/public/thetadatadx.yaml`, and every per-endpoint page. Stale `14 tick types` references corrected to 13. `[Unreleased]` compare link fixed from `v7.2.0...HEAD` to `v7.3.1...HEAD`; missing `v7.2.1` / `v7.3.0` / `v7.3.1` tag compares added.
- **Toml crate metadata warning silenced** (#377) — `toml = "1.1.2+spec-1.1.0"` → `toml = "1.1.2"` in both `[dependencies]` and `[build-dependencies]`. Every `cargo build` invocation no longer warns about ignored semver metadata.
- **Workspace manifest consolidated via `[workspace.package]` + `[workspace.lints]`** (#384) — duplicate `edition`/`license`/`authors`/`repository`/`homepage`/`rust-version` removed from every member `Cargo.toml` and hoisted to the workspace root; each member inherits via `x.workspace = true`. A new `[workspace.lints.rust]` table denies the rustc `warnings` group (matching CI's `-D warnings`) and promotes `unsafe_op_in_unsafe_fn` to deny alongside; `[workspace.lints.clippy]` denies `clippy::all`. Every member crate opts in via `[lints] workspace = true`. Versions intentionally stay per-crate because `tdbe` ships on a `0.x` track independent of the `7.x` SDK line.
- **`tools/server/src/ws.rs` split into `tools/server/src/ws/` module** (#384) — 1044 lines reorganised into `upgrade` · `session` · `subscribe` · `broadcast` · `contract_map` · `format` · `mod`. Visibility tightened from `pub(crate)` to `pub(super)` where external visibility wasn't needed. Pure move; every server unit / integration test passes.
- **`ffi/src/lib.rs` split into topic modules** (#384) — 4054 lines reorganised into `types` / `auth` / `historical` / `streaming` / `utility` / `error` / `panic`. The `ffi_boundary!` macro moves to `panic.rs` and is `#[macro_use]`'d from `lib.rs`. **ABI byte-for-byte identical**: `nm -D --defined-only` lists the same 211 `tdx_*` symbols on both `cdylib` and `staticlib` before and after the split. Downstream C / C++ / Go / Node consumers see zero difference.
- **Three largest code generators split by render target** (#384) — `build_support/endpoints/sdk_surface.rs` (2905 LoC), `ticks.rs` (2094 LoC), and `fpss_events.rs` (1551 LoC) broken into concern-separated sub-modules:
  - `sdk_surface/{spec,common,python,typescript,go,cpp,mcp,cli}.rs`
  - `ticks/{schema,parser,cli_headers,python_arrow,python_classes,typescript,go}.rs`
  - `fpss_events/{schema,common,buffered,python,typescript,ffi_rust,ffi_c,go_structs}.rs`

  A regen byte-identical harness (`crates/thetadatadx/tests/regen_byte_identical.sh`) hashes every generated artifact before + after a clean rebuild and fails on any drift. **Verified: 450 files, zero diff.**
- **50 multi-line `format!(r#"..."#)` templates externalised into `.tmpl` files** (#386) — Rust generators no longer carry embedded Python / TypeScript / Go / C++ source as raw string literals. Templates loaded via `include_str!` and rendered through the existing `format!` machinery (named positional args). No new runtime dependency (no tera / handlebars / askama). LoC reductions on the offender files: `sdk_surface/cpp.rs` -35%, `fpss_events/ffi_rust.rs` -33%, `fpss_events/buffered.rs` -38%, `ticks/parser.rs` -33%. `.gitattributes` extended to pin every `.tmpl` to `eol=lf` so Windows checkouts can't leak CRLF into `include_str!` output. Regen byte-identical harness confirms zero drift across 49 generated artifacts.

## [7.3.1] - 2026-04-16

### Added

- **npm pre-built native binaries for Linux x64, macOS arm64, Windows x64** (#335) -- `npm install thetadatadx` now works without a Rust toolchain. Platform-specific packages (`thetadatadx-linux-x64-gnu`, `thetadatadx-darwin-arm64`, `thetadatadx-win32-x64-msvc`) are selected automatically via `optionalDependencies`. Unsupported platforms get a clear error message at import time. CI publishes all platform packages via GitHub Actions with OIDC provenance.

## [7.3.0] - 2026-04-16

### Added

- **TypeScript/Node.js SDK via napi-rs** (#332) -- native addon exposing all 61 historical endpoints, 20+ streaming methods, and 13 tick types to Node.js 18+. Every method, type, and streaming dispatch is SSOT-generated from the same TOML surface that drives Python, Go, and C++. TypeScript type definitions included. CI builds and smoke-tests on every PR. npm publish workflow coming in a follow-up.

### Fixed

- **FPSS auto-reconnect now re-subscribes all active contracts** (#333) -- the `io_loop` reconnect path authenticated successfully but never re-sent subscription frames, so data stopped flowing after an involuntary disconnect. `active_subs` and `active_full_subs` are now shared via `Arc<Mutex<...>>` between the client and the I/O thread; after reconnect login, every active subscription is re-sent before draining the command channel.
- **Unrecognized FPSS frame codes now emitted as `UnknownFrame` with raw bytes** -- previously logged at trace level and silently dropped, so users had no visibility into unexpected server frames. Now surfaced as `FpssControl::UnknownFrame { code, payload }` with hex-encoded wire bytes in the Python and TypeScript SDKs.
- **Python and TypeScript SDKs explicitly map `Reconnecting`, `Reconnected`, and `MarketClose` control events** -- these previously fell through to the catch-all `"unknown_control"` label, which was confusing in soak-test logs.
- **FFI + Go SDK now expose `UnknownFrame` with raw payload bytes** -- the C FFI bridge maps `UnknownFrame` to kind 11 with the hex-encoded payload in the detail field (was kind 99 with no detail). Go SDK adds the `FpssCtrlUnknownFrame` constant and a complete control-kind enum for all 11 event types. All four SDKs (Python, TypeScript, Go, C++) now surface unrecognized server frames consistently.

### Changed

- **`active_subs` / `active_full_subs` promoted to `Arc<Mutex<...>>`** (#333) -- subscription tables are now shared between the `FpssClient` and the `io_loop` thread so the reconnect path can read them without a command-channel round-trip. Snapshots are cloned before writing frames to avoid holding the lock during I/O.

## [7.2.1] - 2026-04-16

### Fixed

- **Greek and IV decoders regressed by v7.2.0 strict decode** -- every Greek endpoint (`option_snapshot_greeks_*`, `option_history_greeks_*`) returned `Decode failed: column N: expected Number, got Price` on live payloads. The v7.2.0 tightening routed every `f64` tick column through `row_float`, which accepts only `Number` cells, but the v3 MDDS server legitimately sends Greeks and implied-volatility values as `Price`-encoded cells (matching Java's `PojoMessageUtils.dataValue2Object` PRICE → BigDecimal arm). `f64` columns now decode through `row_price_f64` and accept both `Price` and `Number` cells. Regression surfaced on live run 24520486541.
- **Bulk option-chain validator cells timed out at 60 s** -- `all_strikes_one_exp` and `bulk_chain` cells on `option_history_ohlc`, `option_history_quote`, `option_history_trade_quote`, `option_history_greeks_first_order`, `option_history_greeks_implied_volatility`, and `option_at_time_quote` legitimately stream a full-chain payload that does not fit in the 60-second per-cell budget. The CLI / Python / Go / C++ validators now apply a 180-second deadline to bulk-chain / all-strike modes and keep the 60-second baseline for every other cell.

## [7.2.0] - 2026-04-16

### Added

- **Per-request deadlines and async cancellation** (#298) -- every historical endpoint now accepts `with_timeout_ms(u64)` or `with_deadline(Instant)` on its builder and a matching `WithTimeoutMs` / `WithDeadline` option in the Go SDK, C FFI, Python SDK, and C++ SDK. Underlying implementation routes through `tokio::time::timeout` on the gRPC future, so cancellation is cooperative and frees server-side work promptly. Python surfaces a new `TimeoutError` class distinct from `ThetaDataError` so callers can catch slow endpoints without swallowing other failures.
- **New `tdbe::error::DecodeError` enum** (#325) -- per-cell decoding errors now carry structured `{ column, expected, observed }` context instead of a generic string. Folds cleanly into `thetadatadx::Error::Decode` at the `DirectClient` boundary.
- **`tdbe::codec::fit::FitRows`** -- a typed container replacing the previous `Vec<Vec<i32>>` return from the bulk FIT decoder. Exposes `row(i)` and `iter()` for column-major access without per-row heap allocations, materially reducing FPSS decode allocation pressure in sustained streaming.
- **Live parameter-mode matrix validator** (#287, #288, #290, #291) -- every SDK release validator (`scripts/validate_cli.py`, `scripts/validate_python.py`, `sdks/go/validate.go`, `sdks/cpp/examples/validate.cpp`) now runs one test per `(endpoint, mode)` pair instead of one per endpoint. Modes are emitted by the endpoint generator from the wire shape:
  - **List** endpoints: one `basic` mode.
  - **Stock / index / calendar / rate** endpoints: one `concrete` mode.
  - **Option `ContractSpec` endpoints** (29 endpoints): six modes each -- `concrete`, `concrete_iso`, `all_strikes_one_exp`, `all_exps_one_strike`, `bulk_chain`, `legacy_zero_wildcard`.
  - **Per-optional-parameter coverage**: every optional builder parameter gets its own `with_<param>` cell, plus a compound `all_optionals` cell. Compound pairs like `start_time`+`end_time` collapse into a single `with_intraday_window` cell.
  - Streaming endpoints remain exercised by `scripts/fpss_smoke.py` / `fpss_soak.py`.
- **Upstream-derived tier and wildcard maps** (#290, #291) -- dropped hand-maintained `endpoint_min_tier` and `endpoint_supports_expiration_wildcard` match statements in favor of generator-time lookups against a pinned upstream OpenAPI snapshot. The parser fails closed on three drift classes: missing `x-min-subscription`, zero-endpoint snapshots, and unknown `expiration` variants. Surfaced and corrected one stale label (`option_snapshot_market_value` was `value`, upstream says `standard`).
- **Cross-language agreement check** (#290, #291) -- `scripts/validate_agreement.py` loads per-language validator artifacts at `artifacts/validator_<lang>.json` and asserts every `(endpoint, mode)` cell present in at least two SDKs agrees on `status` and `row_count`. `scripts/validate_release.sh` runs CLI -> Python -> Go -> C++ -> agreement in order.
- **Structured field-level diff in validator output** (#293) -- the release validator now emits per-field diffs instead of opaque equality failures, so drift between SDKs is traceable without re-running.
- **Per-cell 60-second timeout on every validator** -- every cell is bounded by a hard 60-second timeout with language-specific hygiene (daemon thread + queue on Python, `packaged_task` + `_Exit` on C++, goroutine + timeout-channel + deferred-close gate on Go, `subprocess.run(timeout=60)` on CLI).
- **Public API redesign charter** (#282) -- `docs/public-api-redesign.md` lays out the layered ergonomic facade plan (canonical parity layer, handwritten `historical` / `realtime` / `analytics` facades, typed value foundations, compatibility window). The streaming category is named `realtime` to avoid overloading the meanings of `live` in CI and run-mode contexts.

### Changed

- **SDK surface is now fully declarative TOML** (#300) -- every generated method signature, optional-parameter shape, streaming dispatch, FFI wrapper, Python binding, Go function, and C++ method is projected from `sdk_surface.toml`, `endpoint_surface.toml`, and `tick_schema.toml`. Adding or changing a method is a TOML edit plus `generate_sdk_surfaces`, with no hand-editing of per-language glue.
- **`parse_*_ticks`, `parse_option_contracts_v3`, `parse_calendar_days_v3` now return `Result<Vec<T>, DecodeError>`** (#325) -- the generated and hand-written row-decoders previously returned `Vec<T>` and silently coalesced per-cell type mismatches to zero. Mismatches now propagate as `DecodeError::TypeMismatch { column, expected, observed }` which folds into `Error::Decode` at the `DirectClient` boundary. This is a Rust-caller-visible breaking change for anyone reaching past `DirectClient::*` into the free functions; the SDK `Result<Vec<T>, Error>` shape users actually call is unchanged, so no ABI / FFI / Python / Go / C++ contract moves.
- **`Contract::option` now returns `Result`** (#324) -- constructing an option contract from user-supplied strings can now surface invalid `expiration` / `strike` / `right` input through `?` instead of panicking on malformed callers.
- **FIT decoder exposes `FitRows`** (tdbe 0.10.0) -- bulk decode returns a dedicated type instead of `Vec<Vec<i32>>`. Callers who passed the old nested-vec shape into downstream helpers need to switch to `FitRows::row()` / `iter()`.
- **`Error::Decode` display text now reads "Decode failed: ..."** (was "Protobuf decode failed: ...") -- the variant now carries both protobuf deserialization errors and post-decode per-cell type-mismatch failures, so the old label was misleading.
- **`build_support/endpoints.rs` split into a focused module tree** (#294) -- what was one 2700-line file is now `helpers`, `model`, `modes`, `parser`, and `render/{build_out,cli_validate,cpp,direct,ffi,go,python}` under `build_support/endpoints/`. Public behavior is unchanged; discovering where a code-gen step lives is now a two-click navigation instead of a search.
- **Generator templates moved to `include_str!`** (#296, #301) -- every remaining `push_str(...)` emitter in `build_support` is now an `include_str!` of a `.tmpl` file under `build_support/endpoints/render/templates/`. Each generated language has its own template directory (`cpp/`, `direct/`, `ffi/`, `go/`, `python/`). Editing a generated code shape no longer requires editing a Rust string literal with embedded Rust syntax.
- **Test-mode fixtures now live in TOML** (#295) -- per-mode test-fixture values were previously a Rust match statement; they are now in `sdk_surface.toml` under `[test_modes.<mode>]`. The generator reads them and emits identical code.
- **`scripts/check_tier_badges.py` live-fetches upstream `openapiv3.yaml`** (#280) -- removed `scripts/upstream_tiers.json` and pulls the authoritative `x-min-subscription` map at check time, with 4 retries + exponential backoff and fail-closed on exhaustion. Eliminates the manual snapshot-refresh drift vector.
- **Validator tier gating is server-driven** -- the four live matrix validators no longer depend on a client-side `VALIDATOR_ACCOUNT_TIER` env var. Every cell is attempted; `PermissionDenied` / `subscription` errors from the server classify as `SKIP: tier-permission` with the declared min_tier echoed, and real bugs continue to surface as `FAIL`. Wildcard-expiration modes (`all_exps_one_strike`, `bulk_chain`, `legacy_zero_wildcard`) are suppressed on the 7 endpoints upstream binds to `expiration_no_star`, because the v3 server rejects `*` for those.
- **Full-vocabulary wildcard support for option contract parameters** (#284) -- `validate_expiration` accepts `*`, `YYYYMMDD`, and `YYYY-MM-DD`; new `validate_strike` accepts `*` / `0` / empty (wildcard) or a positive decimal. `direct::wire_strike_opt` and `direct::wire_right_opt` map wildcard sentinels to `None` so `ContractSpec` leaves the field unset on the proto, matching what the server documents. Live-verified against production across 64 parameter-mode combinations. A full option chain's open interest for QQQ now returns all 10,158 rows in ~1s (a single bulk call), down from a 34-expiration serial loop (~22s).
- **`tdbe` bumped to 0.10.0** -- carries the `FitRows` shape change and the `DecodeError` enum (both public-surface breaking under 0.x rules).

### Fixed

- **FPSS client is now `Sync`-safe** (#324) -- the internal read/write halves and session state are now properly guarded so sharing an `FpssClient` across threads is sound. Previously a latent data race existed on reconnection bookkeeping. Marked `unsafe impl Sync` with the exact invariants documented inline.
- **Python streaming deadlock on shutdown** (#324) -- `next_event()` now releases the GIL before blocking on the ring buffer, and shutdown coordinates with the blocking reader so Ctrl+C interrupts streaming loops cleanly instead of hanging.
- **Python Ctrl+C interruptibility** (#324) -- long-running gRPC calls now release the GIL and cooperate with Python's signal handling, so Ctrl+C returns control to the interpreter without waiting for the server.
- **FFI `CString` interior-NUL swallowing** (#303, #324) -- string outputs across the C ABI now surface `CString::new` failures via `tdx_last_error` instead of silently truncating at the embedded NUL byte. Callers that previously saw empty strings on malformed input now see a diagnosable error.
- **gRPC `Status` parsing propagates ThetaData error codes** (#303) -- the server's numeric error codes are now extracted from the `Status` trailers and surfaced by name, so failures like `INVALID_SYMBOL` read as `INVALID_SYMBOL` instead of the raw integer.
- **Protobuf `DataValue` type coercion** (#303) -- mixed `Price` / `Number` encoding on OHLC cells is normalized consistently across all endpoints; previously a minority of Greeks rows decoded as zero when the server encoded them differently from the cell type hint.
- **Go TLS error-channel races on reconnect** (#324) -- closing an FPSS TLS connection concurrently with an in-flight read no longer produces a spurious send-on-closed-channel panic on Go. The error channel is now drained with a select-default rather than assuming the receiver is still alive. CGo callbacks are also pinned to the calling OS thread to keep the TLS session's thread-local state consistent.
- **Subscription drop on lock poison** (#324) -- active FPSS subscriptions used to silently vanish if a panic poisoned the internal state mutex; the subscription tables now recover via `.into_inner()` so reconnection still finds the intended subscriptions.
- **Float → i32 overflow and panic on invalid strike input** (#324) -- strike parsing now bounds-checks the implied i32 representation before conversion, returning a structured error instead of panicking on a pathological user input (e.g. `"999999999.99"`).
- **Greeks recomputation avoided on unchanged inputs** (#324) -- the Black-Scholes call path memoizes on the common `(spot, strike, vol, rate, t)` tuple so the analytics endpoints no longer recompute identical Greeks on back-to-back rows.
- **FIT decoder allocator thrash** (#324) -- the bulk FIT decoder now reuses a single backing buffer through `FitRows` instead of allocating per-row, cutting sustained streaming allocation rate by roughly an order of magnitude on busy symbols.
- **Double string allocation on `Contract` clone** (#324) -- `Contract` now wraps its symbol in `Arc<str>` so cloning into per-subscription bookkeeping does not copy the byte buffer twice.
- **JSON serialization moved off the FPSS I/O thread** (#324) -- `next_event` now returns typed structs and the serialization step is only paid at the FFI boundary when the caller asks for JSON, keeping the streaming hot path allocation-free.
- **`parse_right` no longer panics on unrecognized input** (#324) -- the canonical right parser returns a structured error for unknown vocabulary instead of panicking, so a single malformed row can no longer take down the decoder.
- **Unset `DataValue` oneof fails loud in every strict decoder** (#326) -- `parse_option_contracts_v3` (expiration, right), `parse_calendar_days_v3` (date, type, open, close), and the generator-emitted EOD helpers plus contract-id injected `expiration` / `right` used to treat a `DataValue` whose `data_type` oneof was unset as a legitimate null and coalesce to `0`. They now return `DecodeError::TypeMismatch { observed: "Unset" }`, matching `row_number` / `row_date` / `row_float` / `row_text` / `row_number_i64` / `row_price_f64` and the Java terminal's default arm. `NullValue` is still coalesced (legitimate null); only the wire-anomaly path changes.
- **Option contract wildcard rejection** (#284) -- before this release the SDK had no working path to the server's bulk-chain mode: `*` was rejected client-side by `validate_expiration`, and `0` was rejected server-side. The SDK vocabulary now covers the full cross-product the server accepts.
- **Validator tier detection drift** (#289) -- dropped the static tier gate that classified legitimate server responses as SKIP. The runtime permission fallback still catches drift between docs and the wire (for example, `interest_rate_history_eod` being labelled `free` on docs but gated higher by the server).
- **CI unbroken on `main`** (#299) -- fixed a `timeout_ms` TOML field mismatch and made the Go pin-test CRLF-robust.
- **FPSS internal visibility tightening** -- `active_subs` and `active_full_subs` are now `pub(in crate::fpss)` rather than `pub(super)`, keeping per-contract and firehose subscription state visible only to the `fpss` module tree. The reconnect-delay tests also now assert against the `TOO_MANY_REQUESTS_DELAY_MS` / `RECONNECT_DELAY_MS` constants instead of hard-coded millisecond literals, so the tests cannot drift from the real protocol values.

### Security

- **Session token no longer leaks via `Debug`** (#324) -- `AuthResponse`'s `session_token` field is now redacted in its `Debug` impl. Previously a `tracing::debug!("{auth:?}")` would write the bearer token into logs. Credentials were already redacted; this closes the parallel leak on the response side.

### Changed

- **Generator bloat cleanup** (#302) -- stripped roughly 1,500 lines of ceremony, over-abstraction, and redundant tests across `build_support/` and the SDK layers. Behavior identical, surface identical, just less to read.
- **`fpss/mod.rs` split into focused submodules** (#327) -- what was a 2,143-line single file is now `accumulator`, `decode`, `delta`, `events`, `io_loop`, `session`, and a slim `mod.rs` under `src/fpss/`. Each submodule owns one responsibility; public behavior is unchanged.
- **Per-cell rationale + redundancy audit in tests** (#297) -- generated test cells now carry a one-line rationale in the comment, so deleted or merged cells leave an obvious trail for reviewers.
- **Consolidated CI workflow cleanup** (#323) -- shared the Rust-dep setup across jobs via a reusable composite action (`.github/actions/setup-rust-deps`), removed duplicated workflow steps, and narrowed `live` to manual dispatch so routine CI stays deterministic.
- **Python abi3 smoke CI no longer rebuilds the wheel** (#304) -- the smoke job now reuses the wheel built earlier in the pipeline, cutting the job's runtime materially.

## [7.1.0] - 2026-04-14

### Removed

- **Greeks utilities now take `right: &str` instead of `is_call: bool`** (#278) -- `tdbe::greeks::all_greeks` and `tdbe::greeks::implied_volatility` accept the same permissive vocabulary as the rest of the SDK (`"C"`/`"P"`, `"call"`/`"put"`, case-insensitive) via the canonical `parse_right_strict`. Panics with a descriptive message on unrecognised input or the `both`/`*` wildcards. The signature change cascades to the Python SDK (`right: str`), Go SDK (`right string`), C++ SDK (`const std::string& right`), C FFI ABI (`tdx_all_greeks` / `tdx_implied_volatility` take `const char* right`), the `tdx greeks` / `tdx iv` CLI subcommands, and the MCP `all_greeks` / `implied_volatility` tool input schemas. The low-level per-Greek primitives (`value`, `delta`, `theta`, ...) continue to take raw `bool` — they are pure-math helpers not in scope. Motivation: consistency with `Contract::option`, `normalize_right`, and `validate_right` so callers stop flipping between `"C"` strings and `true` bools in the same session.
- **`tdbe` bumped to 0.9.0** -- breaking public signature change in `greeks`.
- **`thetadatadx`, `thetadatadx-ffi`, `thetadatadx-cli`, `thetadatadx-mcp`, `thetadatadx-server`, `thetadatadx-py`, and the C++ SDK (CMake project) bumped to 7.1.0** -- downstream version bumps to carry the breaking FFI ABI change.

### Changed

- **`thetadatadx::right` is now a thin re-export of `tdbe::right`** (#278) -- the canonical `right` parser moved into the pure-data `tdbe` crate so `tdbe::greeks` could reuse it without `tdbe` reverse-depending on `thetadatadx`. Public API (`parse_right` / `parse_right_strict` / `ParsedRight` with all four projections) is unchanged at the `thetadatadx::right` path. The error type now returns `tdbe::error::Error::Config` instead of `thetadatadx::error::Error::Config`; a `From<tdbe::error::Error> for thetadatadx::Error` conversion is provided so `?` in `thetadatadx`-returning functions keeps working.
- **Top-level re-exports for offline Greeks** (#278) -- `thetadatadx::{all_greeks, implied_volatility, GreeksResult}` now re-export from `tdbe::greeks` so SDK consumers can avoid reaching into the `tdbe` crate directly. Docs prefer `use thetadatadx::all_greeks;`.
- **Centralized `right` parsing** (#270) -- new `thetadatadx::right` module exposes `parse_right` / `parse_right_strict` returning a `ParsedRight` enum that carries every downstream representation (MDDS lowercase string, FPSS `is_call` bool, short-form `"C"`/`"P"`, FPSS wire byte). `normalize_right` in `direct.rs`, `validate_right` in `validate.rs`, and `Contract::option` in `fpss/protocol.rs` all route through it.
- **OpenAPI YAML aligned with upstream ThetaData** (#270) -- `right-param` enum in `docs-site/public/thetadatadx.yaml` extended to `[call, put, both, C, P, c, p, CALL, PUT, Call, Put, "*"]` to match what the server actually accepts (strict superset of upstream's `[call, put, both]`). Response `right` stays `type: string` with a note documenting the current `"C"`/`"P"` output shape.

### Fixed

- **Silent put-default on invalid `right` in `Contract::option`** (#270) -- previously `Contract::option(..., "xyz")` silently constructed a put contract because the parser only checked for call forms. Now panics with a descriptive message, consistent with the existing strike/expiration panic style.

### Changed

- Every Greeks example in the docs-site, READMEs, Python example, and notebooks updated to pass `right: "C"` / `right="C"` / `right: "C"` instead of `is_call: true`.
- Note added to `docs-site/docs/api-reference.md` and `docs/api-reference.md` clarifying that the low-level per-Greek primitives still take `is_call: bool`, while the user-facing aggregates take `right: &str`.
- **Corrected 31 subscription-tier badges across `docs-site/docs/historical/**/*.md`** (#276) -- audit against ThetaData's canonical `openapiv3.yaml` (`x-min-subscription` field) found 31 of 57 endpoint docs advertised the wrong subscription tier. Fixed against upstream truth.
- **Renamed misnamed doc file** (#276) -- `historical/option/at-time/ohlc.md` actually documented the `option_at_time_quote` endpoint; renamed to `quote.md`, fixed the nav link in `docs-site/docs/.vitepress/config.ts`, and updated the sole inbound reference in `historical/option/index.md`.
- **New `scripts/check_tier_badges.py`** (#276) -- validates every `<TierBadge>` in the historical docs against `scripts/upstream_tiers.json`, a checked-in snapshot of ThetaData's authoritative `x-min-subscription` map (with `_source` and `_captured_at` keys for traceability). Wired into `scripts/check_docs_consistency.py` so the existing `Extended Surfaces` CI job gates tier drift automatically. No network calls at CI time.
- **Deleted orphan docs-site pages** (#272) -- removed top-level single-page versions (`getting-started.md`, `historical.md`, `historical/{stock,option,index-data,calendar}.md`, `streaming.md`, `tools/index.md`) superseded by the subdirectory navigation. Added a `## Client Model` section to `docs-site/docs/streaming/index.md` that makes the per-SDK split (Rust/Python unified `ThetaDataDx`, Go/C++ standalone `FpssClient`) unmistakable. Removed `ignoreDeadLinks: true` from `docs-site/docs/.vitepress/config.ts` so future link rot fails the VitePress build.
- **Sidebar landings for Historical Data and Tools sections** (#274) -- added `link:` fields on both top-level sidebar entries so clicking the section headers lands on the category overview. Created a new `tools/index.md` overview describing the CLI / MCP / REST Server trio.

## [7.0.0] - 2026-04-14

### Removed

- **`SnapshotTradeTick` deleted from all layers** -- removed from Rust core, FFI, Python, Go, and C++ SDKs. Dead type that was never returned by any endpoint.
- **FFI options use explicit `has_*` flags** -- replaced NaN/`-1` sentinel-based optional fields with `has_exclusive`, `has_max_dte`, `has_strike_range`, `has_annual_dividend`, etc. C, Go, and C++ consumers must check the companion `has_*` i32 flag (0 = unset, 1 = set) before reading the value.
- **`generate_sdk_surfaces` restored as the checked-in surface authority** -- the standalone codegen binary is required again and is the canonical way to regenerate and verify generated SDK/FFI/tool surfaces from TOML.
- **Streaming endpoints generated from TOML** -- hand-written streaming endpoint blocks in `direct.rs` replaced by TOML-driven codegen. Method signatures unchanged but internal dispatch is generated.
- **Endpoint, utility, FPSS wrapper, and tick projection surfaces are spec-driven** -- Rust, FFI, Python, Go, C++, CLI, and MCP now project their generated public surfaces from `endpoint_surface.toml`, `sdk_surface.toml`, and `tick_schema.toml`.
- Removed the misleading per-contract `subscribe_option_full_*` / `unsubscribe_option_full_*` FPSS methods from the C FFI, Go SDK, and C++ SDK. Per-contract streams use `subscribe_option_*`; full firehose streams remain `subscribe_full_*` by security type.
- Python FPSS option subscription helpers now take `(symbol, expiration, strike, right)` to match Rust, Go, and C++ argument order.
- **Go/C++ `contract_map` API replaced** -- `ContractMapJSON()` / `contract_map_json()` removed; replaced with typed `ContractMap()` / `contract_map()` returning `map[int32]string` / `std::map<int32_t, std::string>`. Callers of the old JSON variant will fail to compile.

### Removed

- `public-api-redesign.md` and README reference.
- `migration-from-rest-ws.md` and navigation/index references.
- 1,134 lines of commented-out legacy Python methods.
- obsolete claim that `generate_sdk_surfaces` had been removed.

### Changed

- Workspace version bumped from 6.0.0 to 7.0.0.
- `tdbe` bumped from 0.7.0 to 0.8.0. `tdbe@0.7.0` was yanked from crates.io because it shipped with a broken `MarketValueTick` schema (five stale fundamental fields); the 0.8.0 release carries the corrected `market_bid` / `market_ask` / `market_price` layout.
- Docs consistency checker now points at correct generated files.
- `FpssControl::LoginSuccess { permissions }` documented as opaque diagnostic metadata.
- Public endpoint and utility surfaces now project optional request parameters consistently across Rust, Python, Go, C++, CLI, MCP, and REST from the checked-in specs.
- Python now exposes `reconnect()` on the unified streaming client, matching the existing Go/C++ FPSS reconnect capability.
- `time_of_day` accepts both legacy millisecond strings and formatted wall-clock inputs such as `9:30`, `09:30:00`, and `09:30:00.000`, then normalizes to canonical `HH:MM:SS.SSS`.
- Release validation and live smoke harnesses were added and the GitHub live workflow was narrowed to manual dispatch so routine CI stays deterministic.

### Fixed

- `market_value` endpoints now decode `Price` cells correctly instead of returning zeroed prices.
- Release validation, generated Python/Go validators, and cross-platform CLI validation now use valid fixtures and treat legitimate empty responses correctly.
- C++ tick ABI layout now matches the aligned Rust FFI structs, fixing multi-element array stepping bugs.
- Windows Go FFI builds now use the correct GNU-targeted Rust artifacts when building with CGo on GitHub runners.
- Docs and OpenAPI now reflect the real at-time contract and strike wildcard semantics.
- Docs consistency checker no longer references deleted `migration-from-rest-ws.md`.
- `cargo fmt` applied to `build_support/endpoints.rs`.

## [6.0.1] - 2026-04-06

### Removed

- **All tick price fields changed from `i32` to `f64`** -- prices are decoded during parsing. Users access `tick.bid`, `tick.price`, `tick.open` directly as `f64`. No more `price_type` or `_f64()` helpers.
- **`price_type` removed from all public APIs** -- historical ticks, FPSS streaming events, FFI, Python, Go, C++.
- **`strike_price_type` removed** -- `strike` is now `f64` on all tick structs.
- **All `_f64()` and `_price()` helper methods removed** -- `bid_f64()`, `get_price()`, `open_price()`, `trade_price()`, `midpoint_price()`, `midpoint_value()`, `strike_price()` no longer exist.
- **FPSS streaming events: prices are `f64`** -- `FpssData::Quote`, `Trade`, `Ohlcvc` expose `f64` fields directly. No `price_type`. No `_f64` dual fields.
- **`Contract::option()` takes 4 strings** -- `Contract::option("SPY", "20260417", "550", "C")` instead of `(root, i32, bool, i32)`. Matches the MDDS historical API experience.
- **Python SDK**: `subscribe_option_*` takes `(symbol, exp_date, right, strike)` as strings. Removed `price_raw`, `bid_raw`, `price_type` from dicts.
- **Go SDK**: removed `RightRaw`, `StrikePriceType`, `PriceRaw`, `BidRaw`/`AskRaw`/`OpenRaw`/etc., `PriceToF64()`.
- **C++ SDK**: all price fields are `double`. Removed `tdx::price_to_f64()`, `tdx::bid_f64()`, `tdx::open_f64()`, etc.
- **CLI**: `price_type` column removed from all table output.

### Added

- **`QuoteTick.midpoint`** -- pre-computed `(bid + ask) / 2.0` at parse time.
- **`Contract::option_raw()`** -- raw wire-format constructor for the drop-in REST/WS server.
- **Go FFI layout tests** -- compile-time `unsafe.Sizeof` assertions for all 12 C-mirror structs.
- **WebSocket zero-copy fan-out** -- per-client `mpsc<Arc<str>>`, JSON serialized once.
- **Server `--no-ohlcvc` flag** -- disable OHLCVC bar derivation from trades.
- **CLI price formatting** -- preserves up to 6 meaningful decimals, trims trailing zeros.

### Fixed

- **`tools/server` and `tools/mcp` compilation** -- updated for f64 migration (were excluded from workspace, broke silently).
- **Go FFI struct padding** -- 8 structs had incorrect tail padding causing memory corruption on multi-element arrays.
- **`OptionContract` missing `Debug + Clone` derives** -- accidentally removed during refactor.
- **Server dead match arm** -- removed v2 parameter fallback code.

### Changed

- All 60+ endpoint pages updated: f64 fields, no `price_type`, no `_f64()` helpers.
- All SDK READMEs updated (Rust, Python, Go, C++).
- Streaming docs rewritten for f64 events.
- OpenAPI spec purged of `price_type`.
- JVM deviations doc: new sections for FPSS f64 streaming and `Contract::option` clean API.
- Internal docs (architecture, api-reference, endpoint-schema) updated.
- README now explicitly warns that FPSS is not yet production-ready due to the upstream framing issue tracked in `#192`.

## [5.4.0] - 2026-04-05

### Removed

- **`start_streaming_no_ohlcvc()` removed** -- use `DirectConfig::derive_ohlcvc(false)` instead. (#129)
- **Go SDK**: `SnapshotTradeTick` type removed (was dead code after FFI cleanup).

### Added

- **`DirectConfig::derive_ohlcvc(bool)`** -- config-driven OHLCVC opt-out, replaces duplicate method. (#129)
- **REST server drop-in replacement** -- `--email`/`--password`, `--config`, `--fpss-region` CLI args. `/v3/system/status` endpoint. Startup banner. (#128)
- **Error suppression 5s after STOP** -- matches Java terminal behavior. (#124)
- **Auth retry on transient errors** -- 3 attempts, 2s delay, network errors only. (#125)
- **Config validation** -- clamps queue_depth (16-1M), window_size (64-1024) with warnings. (#126)
- **Password character warning** -- on INVALID_CREDENTIALS disconnect. (#127)
- **Clippy pedantic zero warnings** -- `#[must_use]`, inlined format args, numeric separators, `try_from` casts, error docs. No blanket suppression. (#131)

### Fixed

- Zero `#[allow(dead_code)]` in entire project.
- Go SDK dangling extern for removed `TdxSnapshotTradeTickArray`.
- Doc comment typo `100_0000` -> `1_000_000`.
- Test warning on unused `#[must_use]` return.
- All `#[allow]` annotations have reason comments.

## [5.3.1] - 2026-04-04

### Added

- **FPSS auto-reconnect** with configurable policy: `Auto` (default, matches Java terminal), `Manual`, `Custom(fn)`. New control events: `Reconnecting`, `Reconnected`. (#119)
- **Trade/quote condition descriptions** with special-case annotations (e.g., `*update last if only trade`).

### Fixed

- **Greeks returned all zeros** on intraday endpoints (`greeks_first_order`, `greeks_iv`, etc.). The v3 server sends Greeks as Price-encoded cells; `row_float()` now decodes them. (#118)
- **`expiration=0` on wildcard EOD** -- contract ID extraction now handles ISO date text ("2024-01-31" -> 20240131). (#117)
- **`implied_volatility` -> `implied_vol`** header alias added for v3 server column name.
- **Raw strike encoding in docs** -- replaced "500000" with "500" (dollar amounts) across 37 files.
- **`"EOD"` removed from docs** -- v3 uses `"TRADE"` / `"QUOTE"` only.
- **Options examples** rewritten to use wildcard bulk queries instead of per-strike loops.

## [5.3.0] - 2026-04-04

### Removed

- **Go SDK**: `EodTick`, `OhlcTick`, `TradeTick`, `QuoteTick`, `TradeQuoteTick`, `PriceTick`, `SnapshotTradeTick` gain additional fields (raw prices, ext_conditions, price_type). `Right` is now `string` ("C"/"P") with `RightRaw int32` for raw access.
- **Python SDK**: trade dicts gain `ext_condition1..4`. Quote/OHLC/EOD/TradeQuote dicts gain raw price and detail fields.
- **Rust**: `normalize_right()` maps `"C"` -> `"call"`, `"P"` -> `"put"`, `"*"` -> `"both"` for v3 server.

### Added

- **`tdbe::exchange`** -- 78 exchange codes with O(1) lookup: `exchange_name()`, `exchange_symbol()`. (#112)
- **`tdbe::conditions`** -- 149 trade conditions + 75 quote conditions with semantic flags (cancel, volume, high, low, last). (#112)
- **`tdbe::sequences`** -- FPSS sequence tracking with wrapping-aware gap detection. (#112)
- **`tdbe::error`** -- 14 ThetaData HTTP error codes mapped to human-readable names. gRPC errors now include the ThetaData error name. (#113)
- **OHLC price normalization** -- `row_price_value_normalized()` and `change_price_type()` handle mixed price_types across OHLC fields. (#106)
- **Greeks from Price cells** -- `row_float()` decodes Price-typed cells. `implied_vol` header alias. (#106)
- **Calendar v3 parser** -- handles text dates, text times, and type codes from v3 server. (#109)
- **`normalize_right()`** -- maps C/P/* to call/put/both for v3 server. Go `RightStr()` helper. (#111)
- **Full SDK parity** -- Python and Go SDKs now expose every field from every Rust tick type.
- **Latency physics documentation** -- speed-of-light calculations, colocation guidance, Mermaid diagrams.

### Fixed

- **37% of OHLC intraday bars had wrong prices** -- mixed price_type per cell caused 10x errors. (#106)
- **All Greeks returned 0.0** -- server sends Greeks as Price cells, not Number cells. (#106)
- **`option_list_contracts` returned 0** -- v3 server uses "symbol" not "root", ISO dates, text right. (#97)
- **Calendar endpoints returned zeros** -- v3 text format mismatch. (#109)
- **Dev server FPSS crashes** -- binary Error frames and unknown codes handled gracefully. (#85)
- **`PriceToF64` Go formula wrong** -- was `value / 10^pt`, corrected to `value * 10^(pt-10)`.
- **Python `greeks_tick_to_dict` missing 15 fields** -- now has all 24.

### Changed

- 14 documentation fixes across 13 files
- Mermaid diagrams replacing ASCII art in VitePress docs
- Latency physics section with speed-of-light calculations per geography
- 3 new JVM deviations documented
- v3 migration guide compliance verified

## [5.2.1] - 2026-04-04

### Fixed

- `option_list_contracts` returned 0 contracts. The v3 MDDS server sends `symbol` (not `root`), ISO date strings (not YYYYMMDD integers), and `PUT`/`CALL` text (not integer codes). Added `root` -> `symbol` header alias and a v3-aware parser. (#97)
- Dev server FPSS replay boundary corruption handled gracefully. Binary Error frames are silently skipped. Unknown message codes are skipped with bounded retry (5 consecutive = framing corruption -> clean disconnect). (#85)

## [5.2.0] - 2026-04-04

### Removed

- **Go SDK**: price fields on public structs are now `float64` (decoded). Raw `int32` values available as `*Raw` fields. `PriceType` removed from public structs.
- **Go FPSS events**: `FpssQuote.Bid`/`Ask`, `FpssTrade.Price`, `FpssOhlcvc.Open`/`High`/`Low`/`Close` are now `float64`. Raw values as `*Raw` fields.
- **Rust FPSS events**: `FpssData::Quote`, `Trade`, `Ohlcvc` gain pre-decoded `*_f64` fields (`bid_f64`, `price_f64`, etc.).

### Added

- **Rust `_f64()` convenience methods** on all tick types: `price_f64()`, `bid_f64()`, `ask_f64()`, `open_f64()`, `high_f64()`, `low_f64()`, `close_f64()`, `midpoint_f64()`. (#95)
- **Go pre-decoded f64 prices** on all public structs and FPSS events. Users get `tick.Price` as `float64` ready to use. (#95)
- **C++ `tdx::` price helpers** -- 17 inline functions for f64 price decoding on all tick types.
- **FFI FPSS events** gain `*_f64` fields (`bid_f64`, `ask_f64`, `price_f64`, `open_f64`, `high_f64`, `low_f64`, `close_f64`) pre-computed during event construction.

### Fixed

- **Go `PriceToF64` formula** was `value / 10^pt` instead of `value * 10^(pt-10)`. All FPSS streaming prices would have been wrong. (#95)

## [5.1.1] - 2026-04-03

### Fixed

- `tdbe` dependency bumped to 0.2.0 for crates.io publish (0.1.x was yanked). No code changes.

## [5.1.0] - 2026-04-03

### Removed

- **FPSS FFI events now use `#[repr(C)]` typed structs** instead of JSON serialization. `tdx_fpss_next_event` and `tdx_unified_next_event` return `*mut TdxFpssEvent` (a flat tagged struct with quote, trade, open interest, OHLCVC, control, and raw_data variants). Free with `tdx_fpss_event_free`. (#82)
- C++ SDK: `FpssClient::next_event()` returns `FpssEventPtr` (RAII unique_ptr to `TdxFpssEvent`).
- Go SDK: `FpssClient.NextEvent()` returns `*FpssEvent` with typed Go structs.
- Streaming event prices are now raw integers with `price_type` (matching the wire format). Callers decode with `Price::new(value, price_type).to_f64()` or `tdx::price_to_f64(value, price_type)`.
- `serde_json` removed from FFI crate dependencies -- zero JSON crosses the FFI boundary.

### Added

- **Contract identification on all 10 option tick types** -- `expiration`, `strike`, `right`, `strike_price_type` fields populated by the server on wildcard queries. Helper methods `strike_price()`, `is_call()`, `is_put()`, `has_contract_id()` on all 10 tick types via `impl_contract_id!` macro. (#84)
- **8-field trade tick support** -- FPSS dev server sends abbreviated 8-field trade ticks; production sends 16-field. `decode_tick()` now auto-detects the field count from the first absolute tick per contract and dispatches to the correct index mapping. (#86)
- **`#[repr(C)]` FPSS event structs** in all SDKs -- `TdxFpssQuote`, `TdxFpssTrade`, `TdxFpssOpenInterest`, `TdxFpssOhlcvc`, `TdxFpssControl`, `TdxFpssRawData` with tagged `TdxFpssEvent` wrapper. (#82)
- `FfiBufferedEvent` with owned backing storage for safe cross-thread `Send` of pointer-containing structs.
- Go SDK: `FpssQuote`, `FpssTrade`, `FpssOpenInterestData`, `FpssOhlcvc`, `FpssControlData` Go structs mirroring Rust `#[repr(C)]` layout.
- C++ SDK: `FpssClient` class with RAII `FpssEventPtr` for streaming.
- Python SDK: `greeks_tick_to_dict` now emits all 24 fields (was 8). (#92)
- `tdbe`: contract ID fields and `impl_contract_id!` macro on all 10 tick types.

### Fixed

- **9 stale JSON references** in FFI doc comments, FFI README, Go README, docs-site API reference, and macro guide -- all now correctly describe typed structs. (#92)
- Python SDK `greeks_tick_to_dict` missing 16 fields (vanna, charm, vomma, veta, speed, zomma, color, ultima, d1, d2, dual_delta, dual_gamma, epsilon, lambda, vera, date). (#92)
- Go SDK README documented `ActiveSubscriptions()` return type as `json.RawMessage` -- actually returns `[]Subscription`. (#92)
- docs-site Go streaming example said "returns json.RawMessage or nil" -- now says "*FpssEvent or nil".

## [5.0.2] - 2026-04-03

### Fixed

- OHLCVC accumulator `volume` and `count` fields widened from `i32` to `i64` to prevent integer overflow on high-volume symbols during dev server replay. (#80)

## [5.0.1] - 2026-04-03

### Fixed

- `FpssClient::connect()` now uses `DirectConfig::fpss_hosts` instead of hardcoded production servers. `dev()` and `stage()` configs now correctly connect to their respective FPSS servers. (#77)
- Removed dead `SERVERS` constant from `protocol.rs`

## [5.0.0] - 2026-04-02

### Removed

- **Builder pattern on all 61 endpoints** -- methods return builders with `IntoFuture`. `start_time`/`end_time` are now builder methods, not positional params. All optional proto params exposed as chainable setters.
- `received_at_ns: u64` added to every `FpssData` variant (Quote, Trade, OpenInterest, Ohlcvc)
- `DirectConfig::dev()` now uses actual ThetaData dev FPSS servers (port 20200, infinite replay) instead of production with reduced buffers

### Added

- **Builder pattern** -- all endpoints return chainable builders. Zero noise for simple calls, all optional proto params discoverable via autocomplete.
- **`received_at_ns`** -- nanosecond receive timestamp on every FPSS event for latency measurement
- **`tdbe::latency::latency_ns()`** -- DST-aware wire-to-application latency computation
- **`FpssFlushMode`** -- `Batched` (default, matches Java) or `Immediate` (lowest latency)
- **Metrics** -- `metrics` crate integration. Counters/histograms on all gRPC, FPSS, and auth operations. Zero overhead when no backend installed.
- **Config file** -- `DirectConfig::from_file()` behind `config-file` feature flag. TOML format matching v3 terminal.
- **`DirectConfig::stage()`** -- staging FPSS servers (port 20100)
- **3 FPSS methods** in all SDKs -- `subscribe_full_open_interest`, `unsubscribe_full_trades`, `unsubscribe_full_open_interest`
- **Cross-platform CI** -- Format, Lint, Test, FFI Build on Ubuntu + macOS + Windows
- **Macro guide** -- `docs/macro-guide.md` for contributors
- **DST pre-2007 safety net** -- handles old US DST rules (April-October) for pre-2007 dates
- **`unsubscribe_option_open_interest`** in Python SDK (was missing)
- **Go `FpssClient`** -- complete standalone streaming client wrapper (`sdks/go/fpss.go`)

### Fixed

- 30 documentation findings from production audit (version pins, method tables, CHANGELOG, SECURITY)
- 14 public methods missing doc comments on `ThetaDataDx`
- Python SDK `lock().unwrap()` changed to poison recovery
- Legacy `config.default.properties` removed (v2 artifact)

## [4.5.0] - 2026-04-02

### Removed

- **FFI: `#[repr(C)]` typed struct arrays replace JSON** -- all 60 data endpoints now return native struct arrays across the FFI boundary. C++ and Go SDKs read fields directly, zero JSON serialization. FPSS streaming events remain JSON (variable schemas).
- C++ `OptionContract` now uses `std::string root` (was `const char*`)
- Go SDK gains 9 previously missing Greeks endpoints

### Added

- **DST-aware timezone conversion** -- `eastern_offset_ms()` correctly handles EST/EDT transitions using US Energy Policy Act 2005 rules. Historical data from November-March now has correct ms_of_day values. (#32)
- **gRPC flow control config** -- `DirectConfig` gained `mdds_window_size_kb` and `mdds_connection_window_size_kb`, wired into tonic channel builder. (#36)
- Go SDK: `OptionSnapshotGreeksFirstOrder`, `OptionSnapshotGreeksSecondOrder`, `OptionSnapshotGreeksThirdOrder`, `OptionHistoryGreeksFirstOrder/SecondOrder/ThirdOrder`, `OptionHistoryTradeGreeksFirstOrder/SecondOrder/ThirdOrder` (#39)
- Go SDK: `SnapshotTradeTick` type and converter
- Go SDK: `Vera` field on `GreeksTick`
- FFI: 13 typed tick array types (`TdxEodTickArray`, `TdxOhlcTickArray`, etc.) with `from_vec`/`free`
- FFI: `TdxStringArray` for list endpoints, `TdxOptionContractArray` for contracts
- C++ header: `thetadx.h` with all `#[repr(C)]` struct definitions and function signatures

### Fixed

- **Timezone hardcoded UTC-4** -- was producing ms_of_day shifted +1 hour for all Nov-Mar historical data. Now DST-aware with 5 unit tests. (#32)
- **EOD parser divergent alias system** -- unified to shared `find_header()`. (#34)
- **reconnect_wait_ms** -- changed from 1000 to 2000 to match Java terminal. (#35)
- **C++ OptionContract use-after-free** -- root string was dangling after array free. Now deep-copies to `std::string`. (#39)
- **Active subscriptions not cleared on explicit shutdown** -- `shutdown()` clears, involuntary disconnect preserves for reconnect. (#38)
- Mermaid diagram syntax in architecture.md (#30)

### Changed

- Price type per-row variation as known limitation in jvm-deviations.md (#37)
- FPSS ring buffer capacity monitoring as known limitation

## [4.4.0] - 2026-04-02

v3 MDDS DataTable parsing (Timestamp cells), DST-aware timezone, gRPC flow control, header aliases for EOD. See v4.5.0 for cumulative details.

## [4.3.0] - 2026-04-02

### Added

- **`start_time` and `end_time` parameters** exposed on all 25 endpoints that support time filtering. Pass `Some("04:00:00")` for pre-market, `Some("20:00:00")` for extended hours, or `None` for RTH defaults (09:30:00-16:00:00). Affects stock history/snapshot/at-time, option history, and index history endpoints.

### Fixed

- Version pins in README and getting-started docs updated to `"4.2"`
- Default venue `"nqb"` (NASDAQ Best) documented in jvm-deviations.md

## [4.2.0] - 2026-04-01

### Fixed

- **Interval conversion**: MDDS server accepts preset shorthand (`1m`, `5m`, `1h`), not raw milliseconds. `normalize_interval()` now converts `"60000"` -> `"1m"`, `"300000"` -> `"5m"`, etc. Sub-second presets supported: `"100"` -> `"100ms"`, `"500"` -> `"500ms"`. Users can pass either milliseconds or shorthand directly.
- **Default start_time/end_time**: the Java terminal defaults these to `"09:30:00"` and `"16:00:00"`. Our SDK left them as None, causing `"Invalid time format: Expected hh:mm:ss.SSS"` on trade/quote/greeks endpoints. Now defaults to RTH.
- **extract_text_column**: now handles Number and Price DataTable values. `option_list_strikes` was returning 0 results because strikes come as Number values, not Text.
- **FPSS TLS certificate**: ThetaData's FPSS servers have certificates expired since Jan 2024. Skip certificate verification for FPSS connections (matching Java terminal behavior).

### Added

`100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`

## [4.1.2] - 2026-04-01

Interval format conversion (later superseded by shorthand normalization in v4.2.0).

## [4.1.1] - 2026-04-01

### Fixed

- PyPI publish workflow: add `skip-existing: true` to prevent duplicate upload failures on tag re-push

## [4.1.0] - 2026-04-01

### Added

- `subscribe_full_open_interest(sec_type)` -- firehose open interest subscription (was missing, Java terminal has it)
- `unsubscribe_full_trades(sec_type)` -- firehose trade unsubscribe (was missing)
- `unsubscribe_full_open_interest(sec_type)` -- firehose OI unsubscribe (was missing)
- `reconnect_streaming(handler)` on `ThetaDataDx` -- saves active subscriptions, stops streaming, restarts with new handler, re-subscribes all per-contract and full-type subscriptions automatically
- `active_full_subscriptions()` accessor for full-type subscription tracking
- `docs/java-class-mapping.md` -- complete enumeration of all 588 Java terminal classes with Rust equivalents or justification for exclusion

### Fixed

- DNS hostname resolution in FPSS connection -- `SocketAddr::parse()` replaced with `ToSocketAddrs` to resolve hostnames like `nj-a.thetadata.us` (was silently failing)

### Changed

- Greeks operator precedence (veta, speed, zomma, color, dual_gamma) -- Java decompiler may have lost parenthesization, Rust follows textbook Black-Scholes formulas
- FPSS ring buffer capacity monitoring -- documented as known limitation (disruptor-rs v4 has no fill-level API)

## [4.0.0] - 2026-04-01

### Removed

- **`tdbe` crate extracted** -- all data types, codecs, greeks, price, enums, and flags moved to standalone `tdbe` crate with zero networking dependencies. Users must add `tdbe` as a dependency and change imports: `use tdbe::{Price, TradeTick, EodTick}`.
- `thetadatadx` no longer exports `types/`, `codec/`, `greeks.rs`. These modules live in `tdbe`.

### Added

- **`tdbe` crate** (`crates/tdbe/`) -- pure data-format crate. Single dependency (`thiserror`). Contains:
  - 14 hand-written tick structs (no build.rs codegen)
  - FIT/FIE nibble codecs
  - Price fixed-point encoding
  - 22 Black-Scholes Greeks + IV solver
  - All enums (SecType, DataType, StreamMsgType, etc.)
  - Error types (Decode, Encode, Conversion, Io)
  - Flags module (trade conditions, price flags, volume types)
  - 6 criterion benchmarks
- **Interactive Query Builder** on docs site -- 13 real-world recipes (GEX, vol surface, option chains, live trade tape, etc.) with symbol autocomplete, dynamic dates, and copy-paste code generation for Rust and Python
- **Inline credential construction** -- all SDK examples now show both `from_file("creds.txt")` and `Credentials::new("email", "password")` patterns
- **serde_json vs sonic_rs benchmark** (`bench_json`) -- criterion benchmark covering FPSS events, REST responses, DataTable serialization, and JSON parsing

### Fixed

- Query builder syntax highlighter regex cross-contamination (visible `class="hl-string"` in rendered code)

### Changed

- Tick types in `tdbe` are hand-written (no `include!()`, no `tick_schema.toml` codegen). IDE-navigable, visible in source.
- Magic numbers in `TradeTick` impl replaced with `tdbe::flags::` named constants
- Documentation updated across 17+ files for new import paths

## [3.2.2] - 2026-03-30

### Fixed

- Cleaned git history and consolidated documentation commits.
- Added contributor workflow documentation (conventional commits, pre-commit checks).

## [3.2.0] - 2026-03-30

### Added

- **Fully typed returns for all 61 endpoints** - 9 new tick types (`TradeQuoteTick`, `OpenInterestTick`, `MarketValueTick`, `GreeksTick`, `IvTick`, `PriceTick`, `CalendarDay`, `InterestRateTick`, `OptionContract`). All 31 endpoints that returned raw `proto::DataTable` now return typed `Vec<T>`. The `raw_endpoint!` macro has been removed entirely. Zero raw protobuf in the public API.
- **TOML-driven codegen** - `tick_schema.toml` is the single source of truth for all tick type definitions and DataTable column schemas. `build.rs` generates Rust structs and parsers at compile time. Adding a new column = one line in the TOML.
- **Proto maintenance guide** (`proto/MAINTENANCE.md`) - step-by-step instructions for ThetaData engineers to add columns, RPCs, or replace proto files.
- 10 new parse functions in `decode.rs` (including `parse_eod_ticks` moved from inline in `direct.rs`)
- All downstream consumers updated: FFI (9 new JSON converters), CLI (9 new renderers), Server (9 new sonic_rs serializers), MCP (9 new serializers), Python SDK (9 new dict converters)
- Crate README (`crates/thetadatadx/README.md`) and FFI README (`ffi/README.md`)
- Python SDK: polars support documented (`pip install thetadatadx[polars]`)

### Fixed

- **Comprehensive documentation sweep** - every doc page, README, notebook, and example file audited against the actual source code. Fixed fabricated homepage examples, wrong C++ include paths (`thetadatadx.hpp` -> `thetadx.hpp`), stale `client.` variable names, missing typed return annotations, wrong Python `all_greeks()` parameter name, version pins (`3.0` -> `3.1`), `for_each_chunk` signature in API reference, and incorrect license in footer.
- **Parameter/response display redesign** - replaced flat markdown tables with vertical card layout across 60 endpoint documentation pages.
- Root README streamlined with navigation table (removed 90-line endpoint listing)
- Notebook 105: fixed event kinds and removed raw payload access pattern
- OpenAPI yaml: fixed license, GitHub URLs, removed DataTable response types

## [3.1.0] - 2026-03-27

### Fixed

- **Go SDK: price encoding was fundamentally wrong** - `priceToFloat()` used a switch-case instead of `value * 10^(price_type - 10)`. Every price returned by the Go SDK was incorrect. Now matches Rust exactly.
- **Python docs: streaming examples used wrong event key** - streaming-event dict access changed from the legacy `type` key to the canonical `kind` key across README and all docs-site pages.
- **`Price::new()` no longer panics in release** - `assert!` replaced with `debug_assert!` + `clamp(0, 19)` with `tracing::warn!`. A corrupt frame no longer crashes production.
- **C++ `FpssClient`: added missing `unsubscribe_quotes()`** - was present in FFI but missing from C++ RAII wrapper.
- **FFI FPSS: mutex poison safety** - all 12 `.lock().unwrap()` calls replaced with `.unwrap_or_else(|e| e.into_inner())`. Prevents undefined behavior (panic across `extern "C"`) on mutex poisoning.
- **`Credentials.password` visibility** - changed from `pub` to `pub(crate)` with `password()` accessor. Prevents accidental credential logging by downstream code.
- **WebSocket server: added OPEN_INTEREST + FULL_TRADES dispatch** - previously silently dropped.
- **C++ SDK type parity** - `MarketValueTick` expanded from 3 to 7 fields, `CalendarDay` added `status`, `InterestRateTick` added `ms_of_day`.
- **Python README: removed ghost methods** - `is_authenticated()` and `server_addr()` were listed but did not exist.
- **Root README: stock method count** - "Stock (13)" corrected to "Stock (14)".

## [3.0.0] - 2026-03-27

### Removed

- **Unified `ThetaDataDx` client** — single entry point replacing `DirectClient` + `FpssClient`.
  Connect once, auth once. Historical available immediately, streaming connects lazily.
- **`DirectClient` removed from crate root re-exports** — still accessible as `thetadatadx::direct::DirectClient` but all methods available via `ThetaDataDx` (Deref)
- **`FpssClient` removed from crate root re-exports** — use `tdx.start_streaming(handler)` instead
- **Python SDK**: `DirectClient` and `FpssClient` classes removed. Use `ThetaDataDx` only.

### Added

- `ThetaDataDx::connect(creds, config)` — one auth, gRPC channel ready, no FPSS yet
- `tdx.start_streaming(handler)` — lazy FPSS connection on demand (reads `derive_ohlcvc` from config)
- `tdx.stop_streaming()` — clean shutdown of streaming, historical stays alive
- `tdx.is_streaming()` — check if FPSS is active
- All 61 historical methods via `Deref<Target = DirectClient>`
- All streaming methods (subscribe/unsubscribe) directly on `ThetaDataDx`
- FFI: `tdx_unified_connect()`, `tdx_unified_start_streaming()`, `tdx_unified_stop_streaming()`
- Server: graceful `stop_streaming()` on shutdown

### Fixed

- Server shutdown now calls `stop_streaming()` before notifying waiters
- Python SDK: removed duplicate method definitions (DirectClient + ThetaDataDx had same methods)

## [2.0.0] - 2026-03-27

### Added

- **`tdx` CLI** (`tools/cli/`) — command-line tool with all 61 endpoints + Greeks + IV.
  Dynamically generated from endpoint registry. `cargo install thetadatadx-cli`
- **MCP Server** (`tools/mcp/`) — Model Context Protocol server giving LLMs instant
  access to 64 tools (61 endpoints + ping + greeks + IV) over JSON-RPC stdio.
  Works with Cursor and every other MCP-compatible client.
- **REST+WS Server** (`tools/server/`) — drop-in replacement for the Java terminal.
  v3 API on port 25503, WebSocket on 25520 with real FPSS bridge. sonic-rs JSON.
- **VitePress documentation site** (`docs-site/`) — 33 pages covering API reference,
  guides, SDK docs, wire protocol internals. Deployed to GitHub Pages.

### Removed

- **FpssEvent split** — `FpssEvent::Quote { .. }` is now `FpssEvent::Data(FpssData::Quote { .. })`.
  Control events are `FpssEvent::Control(FpssControl::*)`. Migration: wrap your match arms.
- **OHLCVC derivation opt-in/out** — `connect()` still derives OHLCVC (default).
  Set `DirectConfig::derive_ohlcvc` to `false` to disable for lower overhead on full trade streams.
- **FpssClient is fully sync** — no tokio in the streaming path. LMAX Disruptor
  ring buffer. Callback API: `FnMut(&FpssEvent)`.

### Added

- **Endpoint registry** — auto-generated from proto at build time. Single source of
  truth consumed by CLI, MCP, server. 61 endpoints.
- **Repo reorganization** — `tools/cli/`, `tools/mcp/`, `tools/server/` (was `crates/*`)
- **sonic-rs** — SIMD-accelerated JSON in CLI, MCP, and server (replaces serde_json)
- **Zero-alloc FPSS hot path** — reusable frame buffer, tuple return (no Vec per frame),
  pre-allocated decode buffer, wrapping_add for delta parity
- **Full SDK parity** — all FPSS methods (subscribe_full_trades, contract_lookup,
  active_subscriptions, etc.) exposed in Python, Go, C++, FFI
- **Full trade stream docs** — explains the server's quote+trade+OHLC bundle behavior
- **v3 REST API** — server routes match ThetaData's OpenAPI v3 spec (was v2)
- **43 benchmarks** — 10 per-module bench files covering every hot path

### Fixed

- **SIMD FIT removed** — was 2.2x slower than scalar (regression). Pure scalar now.
- **Server trade_greeks routes** — 5 option history trade_greeks endpoints were silently
  dropped due to subcategory mismatch in path generation
- **Audit findings (hot-path)** — hot-path allocations, wrapping_add, BufWriter, find_header
  fallback, DATE marker handling, MCP sanitization, Price dedup
- **Audit findings (server/CLI)** — server security (CORS, shutdown auth), CLI expect(), MCP
  JSON-RPC validation, stale docs
- **Auth response parsing** — subscription fields are integers not strings

### Changed

- FPSS frame read: zero-alloc (reusable buffer)
- FPSS decode: zero-alloc (tuple return, pre-allocated tick buffer)
- Delta: wrapping_add (matches Java, no branch)
- Required column validation (skip rows on missing headers, no garbage parse)
- 43 criterion benchmarks across all modules

## [1.2.2] - 2026-03-26

### Added

- **Polars support** in Python SDK: `pip install thetadatadx[polars]`
- `to_polars(ticks)` function converts tick dicts directly to polars DataFrame via `polars.from_dicts()`
- Optional dependency groups: `[pandas]`, `[polars]`, `[all]` for both

### Fixed

- **Multi-platform Python wheels** — now builds for Linux, macOS, and Windows (was Linux-only)
- Source distribution (sdist) included for pip build-from-source fallback
- Auth response parsing: subscription fields are integers (0-3), not strings — fixes connection failures

## [1.2.1] - 2026-03-26

### Fixed

- **Auth: subscription fields are integers** — Nexus API returns `"stockSubscription": 0` (int), not strings. Fixes `"failed to parse Nexus API response"` error on connect.
- **Multi-platform Python wheels** — CI now builds for Linux + macOS + Windows (was Linux x86_64 only). Fixes `"no matching distribution found"` for macOS/Windows users.
- **Source distribution** — sdist included so `pip install` can build from source when no pre-built wheel matches.
- Removed hallucinated "row deduplication" from docs (was never implemented, would have dropped real trades).

## [1.2.0] - 2026-03-26

### Added

- **OHLCVC-from-trade derivation** — `OhlcvcAccumulator` derives OHLCVC bars from trade
  ticks in real time. Only emits `FpssEvent::Data(FpssData::Ohlcvc { .. })` after a
  server-seeded initial bar, matching the Java terminal's behavior. Subsequent trades
  update open/high/low/close/volume/count incrementally.
- **FpssEvent split: `FpssData` + `FpssControl`** — the monolithic `FpssEvent` enum is now
  a 3-variant wrapper: `Data(FpssData)` for market data (Quote, Trade, OpenInterest, Ohlcvc),
  `Control(FpssControl)` for lifecycle events (LoginSuccess, Disconnected, MarketOpen, etc.),
  and `RawData` for unparsed frames. This enables `match` arms that handle all data without
  touching control flow, and vice versa — an intentional improvement not present in Java.
- **Streaming `_stream` endpoint variants** — `stock_history_trade_stream`,
  `stock_history_quote_stream`, `option_history_trade_stream`, `option_history_quote_stream`
  process gRPC response chunks via callback without materializing the full response in memory.
  Ideal for endpoints returning millions of rows.
- **Slab-recycled zstd decompressor** — thread-local `(Decompressor, Vec<u8>)` pair reuses
  the working buffer across calls. The internal slab retains its capacity, avoiding allocator
  pressure for repeated decompressions of similar-sized payloads.
- **148 tests** — new tests for OHLCVC accumulator, FpssEvent split, and
  streaming endpoints.

### Fixed

18 correctness and protocol-conformance fixes from a full audit against the Java terminal:

**FPSS Protocol**

1. **FPSS contract ID is FIT-decoded** — CONTRACT message contract IDs are now FIT-decoded
   (matching the Java terminal), not read as raw big-endian i32. Previously produced wrong
   contract-to-symbol mappings.
2. **Delta off-by-one fixed** — `apply_deltas` field indexing corrected; previous
   implementation could shift all fields by one position, corrupting tick data.
3. **Delta state cleared on START/STOP** — per-contract delta accumulators are now reset
   when the server sends START (market open) or STOP (market close), matching Java behavior.
   Previously, stale deltas from the previous session leaked into the next session's ticks.
4. **ROW_SEP unconditional reset** — ROW_SEP (0xC) now unconditionally resets the field
   index to SPACING (5), matching the Java FIT reader. Previously this was conditional,
   which could produce misaligned fields.
5. **Credential sign-extension** — credential length fields are now read as unsigned,
   matching Java's `readUnsignedShort()`. Previously, passwords longer than 127 bytes
   could produce a negative length.
6. **Flush only on PING** — the FPSS write buffer is now flushed only when sending PING
   messages, matching Java's batching behavior. Previously, every write triggered a flush,
   increasing syscall overhead and wire chattiness.
7. **Ping 2000ms initial delay** — the first PING is now delayed by 2000ms after
   authentication, matching the Java terminal's `Thread.sleep(2000)` before entering the
   ping loop. Previously, pings started immediately.

**MDDS / gRPC Protocol**

8. **`null_value` added to DataValue proto** — the `DataValue` oneof now includes a
   `null_value` variant (bool), matching the server's proto definition. Previously,
   null cells were silently dropped during deserialization.
9. **`"client": "terminal"` in query_parameters** — all gRPC requests now include
   `"client": "terminal"` in the `query_parameters` map, matching the Java terminal.
   Previously this field was omitted.
10. **Dynamic concurrency from subscription tier** — `mdds_concurrent_requests` is now
    derived from the `AuthUser` response's subscription tier (`2^tier`), matching the
    Java terminal's concurrency model. The config field still allows manual override.
11. **Unknown compression returns error** — `decompress_response` now returns
    `Error::Decompress` for unrecognized compression algorithms instead of silently
    treating the data as uncompressed.
12. **Empty stream returns empty DataTable** — `collect_stream` now returns an empty
    `DataTable` (with headers, zero rows) when the gRPC stream contains no data chunks,
    instead of returning `Error::NoData`. Callers can check `.data_table.is_empty()`.
13. **gRPC flow control window** — the gRPC channel now configures
    `initial_connection_window_size` and `initial_stream_window_size` to match the Java
    terminal's Netty settings, preventing throughput bottlenecks on large responses.

**Auth / User Model**

14. **Per-asset subscription fields in AuthUser** — `AuthUser` now includes `stock_tier`,
    `option_tier`, `index_tier`, and `futures_tier` fields from the Nexus auth response,
    enabling per-asset-class concurrency and permission checks.
15. **Auth 401/404 handling** — Nexus HTTP responses with status 401 (Unauthorized) or
    404 (Not Found) are now treated as invalid credentials, matching the Java terminal's
    behavior. Previously these could surface as generic HTTP errors.

**Observability**

16. **Column lookup warns instead of silent fallback** — `extract_*_column` functions now
    emit a `warn!` log when a requested column header is not found in the DataTable,
    instead of silently returning a vec of `None`s. This makes schema mismatches
    immediately visible in logs.

**Greeks**

17. **6 Greeks formula fixes** — operator precedence corrections across 6 Greek functions
    to match Java's evaluation order. All formulas now produce bit-identical results to
    the Java terminal for the same inputs.
18. **`Vera` DataType code (166)** — second-order Greek `Vera` added to the `DataType` enum,
    completing the full set of second-order Greeks (vanna, charm, vomma, veta, vera, sopdk).

### Security

- **Contract wire format fix** — contract binary serialization now matches the Java terminal
  exactly. Previous versions could produce incorrect wire bytes for option contracts, causing
  subscription failures or wrong contract assignments. This was a **protocol-level bug**;
  upgrading to 1.2.x is strongly recommended.

### Changed

- **Slab-recycled zstd** — thread-local decompressor reuses its working buffer, eliminating
  per-chunk allocation overhead.
- **Streaming `_stream` endpoints** — process gRPC responses chunk-by-chunk without
  materializing the full DataTable in memory.

See `TODO.md` (as of the 1.2.0 release) for the production readiness checklist and performance roadmap.

## [1.1.1] - 2026-03-26

### Added

- **`mdds_concurrent_requests` semaphore** on DirectClient — configurable limit on in-flight
  gRPC requests (default 2), exposed via `DirectConfig.mdds_concurrent_requests`
- **Streaming `for_each_chunk` method** on DirectClient — process gRPC response chunks via
  callback without materializing the full response in memory
- **Pre-allocation hint in `collect_stream`** — uses `original_size` from `ResponseData` to
  pre-allocate the decompression buffer, reducing reallocations
- **Horner-form `norm_cdf`** — replaced Abramowitz & Stegun polynomial approximation with
  Zelen & Severo Horner-form evaluation (~1e-7 accuracy, fewer multiplications)
- **Python SDK: FPSS streaming** — `FpssClient` class with `subscribe()`, `next_event()`,
  and `shutdown()` methods for real-time market data in Python
- **Python SDK: pandas DataFrame conversion** — `to_dataframe()` function plus per-endpoint
  DataFrame convenience methods on DirectClient (later superseded in #379 by the unified
  `to_dataframe(ticks)` Arrow-backed path); install with `pip install thetadatadx[pandas]`
- **FFI crate: FPSS support** — 7 new `extern "C"` functions for FPSS lifecycle
  (`fpss_connect`, `fpss_subscribe_quotes`, `fpss_subscribe_trades`,
  `fpss_subscribe_open_interest`, `fpss_next_event`, `fpss_shutdown`, `fpss_free_event`)
- **Go SDK: FPSS streaming** — `FpssClient` Go struct wrapping the FFI FPSS functions
- **C++ SDK: FPSS streaming** — `FpssClient` C++ RAII class wrapping the FFI FPSS functions

### Fixed

- Version bump for crates.io/PyPI publish (v1.1.0 tag was re-pushed during history restore)

### Changed

- All TODO performance items now complete: streaming iterator (`for_each_chunk`),
  optimized `norm_cdf` (Horner-form), concurrent request semaphore (`mdds_concurrent_requests`)

## [1.1.0] - 2026-03-26

### Added

- **All 61 endpoints** via declarative macro (was 19 hand-written) — covers every
  v3 gRPC RPC: stock, option, index, interest rate, calendar
- **All 61 endpoints in every SDK** — Python, Go, C++, C FFI all match Rust core
- **Zero-allocation FPSS path** — fully sync I/O thread + LMAX Disruptor ring buffer
  (`disruptor-rs` v4), no tokio in the streaming hot path
- **Cache-line aligned tick types** — `#[repr(C, align(64))]` on TradeTick, QuoteTick, OhlcTick, EodTick
- **Cached QueryInfo template** — no per-request String allocation
- **Precomputed DataTable column indices** — O(1) per row, not O(headers)
- **pow10 lookup tables** for Price comparison and conversion
- **`#[inline]`** on all hot-path functions (FIT decode, Price ops, tick accessors)
- **Reusable thread-local zstd decompressor** — no fresh allocation per chunk
- **Criterion benchmarks** — fit_decode, price_to_f64, price_compare, all_greeks, fie_encode
- **AdaptiveWaitStrategy** — 3-phase spin/yield/hint tuned for ~100us FPSS tick intervals

### Changed

- Authenticated against real Nexus API (session established)
- Retrieved 25,341 stock symbols from MDDS
- Retrieved 42 AAPL EOD ticks (Jan-Mar 2024) with correct OHLCV data
- Retrieved 2,010 SPY option expirations
- Retrieved 13,160 index symbols
- Calendar endpoint returned valid data
- `client_type = "rust-thetadatadx"` accepted by server

## [1.0.1] - 2026-03-26

### Changed

- Renamed crate from `thetadx` to `thetadatadx` (crates.io + PyPI)
- Renamed repository from `thetadx` to `ThetaDataDx`
- Changed license metadata
- Updated top-level README
- README updated with GitHub callouts (NOTE, TIP, IMPORTANT, WARNING, CAUTION)
- Fixed PyPI package description (was empty — added readme field to pyproject.toml)

## [1.0.0] - 2026-03-26

### Added

- **DirectClient** for MDDS gRPC — all 60 gRPC RPCs exposed as 61 typed endpoint methods
  (stock/option/index/rate/calendar: list, history, snapshot, at-time, greeks) via
  declarative `define_endpoint!` macro
- **FpssClient** for FPSS streaming — real-time quotes, trades, open interest, OHLC
  via TLS/TCP with heartbeat and manual reconnection
- **Auth module** — Nexus API authentication (email/password → session UUID)
- **FIT/FIE codec** — nibble-based tick compression/decompression (ported from Java)
- **Greeks calculator** — full Black-Scholes: 22 Greeks + IV bisection solver with
  precomputed shared intermediates and edge-case guards (t=0, v=0)
- **All tick types** — TradeTick, QuoteTick, OhlcTick, EodTick, OpenInterestTick,
  SnapshotTradeTick, TradeQuoteTick with fixed-point Price encoding
- **80+ DataType enum codes** — quotes, trades, OHLC, all Greek orders, dividends,
  splits, fundamentals
- **Proto definitions** — extracted via runtime FileDescriptor reflection from
  ThetaData Terminal v202603181 (endpoints.proto + v3_endpoints.proto)
- **Runtime configuration** — `DirectConfig` with all JVM-equivalent tuning knobs
- `contract_lookup(id)` on `FpssClient` for single-entry hot-path lookup
- `FpssEvent::Error` variant for surfacing protocol parse failures
- Date parameter validation on all `DirectClient` methods
- `async-zstd` feature flag for optional streaming decompression
- **Python SDK** (PyO3/maturin) — wraps the Rust crate, not a reimplementation
- **Go SDK** — CGo FFI bindings over the C ABI layer
- **C++ SDK** — RAII C++ wrapper over the C header
- **C FFI crate** (`thetadatadx-ffi`) — stable `extern "C"` ABI for all SDKs
- **Documentation** — architecture (Mermaid), API reference, Java parity checklist
- **CI/CD** — GitHub Actions (fmt, clippy, test, FFI build, crates.io publish, PyPI publish, GitHub Release)
- **Project infrastructure** — CHANGELOG, CONTRIBUTING, SECURITY, CODE_OF_CONDUCT,
  clippy.toml, cliff.toml, rust-toolchain.toml, LICENSE

### Security

- Credential `Debug` redaction — passwords never appear in debug output
- `AuthRequest` does not derive `Debug` (prevents password in error traces)
- Session UUID redaction — bearer tokens logged at `debug!` level only, first 8 chars
- `assert!` on FPSS frame size limits — enforced in release builds
- Unified TLS via rustls for all connections (MDDS gRPC + FPSS TCP + Nexus HTTP)
- Timeouts on all network operations (auth 10s/5s, gRPC keepalive, FPSS connect, FPSS read 10s)
- 7 credential/account errors treated as permanent disconnect (no futile reconnect loops)
- Contract root length validated before wire serialization
- FIT decoder uses i64 accumulator with i32 saturation (no silent overflow)
- Price type range enforced with `assert!` in release builds

[Unreleased]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.11...HEAD
[8.0.11]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.10...v8.0.11
[8.0.10]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.9...v8.0.10
[8.0.9]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.8...v8.0.9
[8.0.8]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.7...v8.0.8
[8.0.7]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.6...v8.0.7
[8.0.6]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.5...v8.0.6
[8.0.5]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.4...v8.0.5
[8.0.4]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.3...v8.0.4
[8.0.3]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.2...v8.0.3
[8.0.2]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.1...v8.0.2
[8.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v8.0.0...v8.0.1
[8.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v7.3.1...v8.0.0
[7.3.1]: https://github.com/userFRM/ThetaDataDx/compare/v7.3.0...v7.3.1
[7.3.0]: https://github.com/userFRM/ThetaDataDx/compare/v7.2.1...v7.3.0
[7.2.1]: https://github.com/userFRM/ThetaDataDx/compare/v7.2.0...v7.2.1
[7.2.0]: https://github.com/userFRM/ThetaDataDx/compare/v7.1.0...v7.2.0
[7.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v7.0.0...v7.1.0
[7.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v6.0.1...v7.0.0
[6.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v5.4.0...v6.0.1
[5.4.0]: https://github.com/userFRM/ThetaDataDx/compare/v5.3.1...v5.4.0
[5.3.1]: https://github.com/userFRM/ThetaDataDx/compare/v5.3.0...v5.3.1
[5.3.0]: https://github.com/userFRM/ThetaDataDx/compare/v5.2.1...v5.3.0
[5.2.1]: https://github.com/userFRM/ThetaDataDx/compare/v5.2.0...v5.2.1
[5.2.0]: https://github.com/userFRM/ThetaDataDx/compare/v5.1.1...v5.2.0
[5.1.1]: https://github.com/userFRM/ThetaDataDx/compare/v5.1.0...v5.1.1
[5.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v5.0.2...v5.1.0
[5.0.2]: https://github.com/userFRM/ThetaDataDx/compare/v5.0.1...v5.0.2
[5.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v5.0.0...v5.0.1
[5.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v4.5.0...v5.0.0
[4.5.0]: https://github.com/userFRM/ThetaDataDx/compare/v4.4.0...v4.5.0
[4.4.0]: https://github.com/userFRM/ThetaDataDx/compare/v4.3.0...v4.4.0
[4.3.0]: https://github.com/userFRM/ThetaDataDx/compare/v4.2.0...v4.3.0
[4.2.0]: https://github.com/userFRM/ThetaDataDx/compare/v4.1.2...v4.2.0
[4.1.2]: https://github.com/userFRM/ThetaDataDx/compare/v4.1.1...v4.1.2
[4.1.1]: https://github.com/userFRM/ThetaDataDx/compare/v4.1.0...v4.1.1
[4.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v4.0.0...v4.1.0
[4.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v3.2.2...v4.0.0
[3.2.2]: https://github.com/userFRM/ThetaDataDx/compare/v3.2.0...v3.2.2
[3.2.0]: https://github.com/userFRM/ThetaDataDx/compare/v3.1.0...v3.2.0
[3.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v3.0.0...v3.1.0
[3.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v2.0.0...v3.0.0
[2.0.0]: https://github.com/userFRM/ThetaDataDx/compare/v1.2.2...v2.0.0
[1.2.2]: https://github.com/userFRM/ThetaDataDx/compare/v1.2.1...v1.2.2
[1.2.1]: https://github.com/userFRM/ThetaDataDx/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/userFRM/ThetaDataDx/compare/v1.1.1...v1.2.0
[1.1.1]: https://github.com/userFRM/ThetaDataDx/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/userFRM/ThetaDataDx/releases/tag/v1.0.0
