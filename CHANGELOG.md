# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **Option bulk-query wire semantics match the Java terminal exactly** (#284) -- full audit against the decompiled v3 Java terminal (`net.thetadata.generated.v3grpc.RestResource`) revealed our SDK was producing wire requests the terminal doesn't. Fixes:
  - **`expiration`** -- \`validate_expiration\` accepts \`"*"\` (upstream's canonical wildcard), \`"0"\` (legacy sentinel), \`"YYYYMMDD"\`, and \`"YYYY-MM-DD"\` (ISO). \`direct::normalize_expiration\` translates \`"0"\` -> \`"*"\` and strips dashes from the ISO form before the wire so the server receives the canonical \`YYYYMMDD\` or \`*\`. Previously \`"*"\` was rejected client-side and \`"0"\` was rejected server-side with \`InvalidArgument -- Error parsing expiration\`, so the SDK had no working path to bulk-expiration mode.
  - **`strike`** -- new \`validate_strike\` accepts \`"*"\` / \`"0"\` / empty (wildcard) or a positive decimal. New \`direct::wire_strike_opt\` emits \`None\` for wildcards so \`ContractSpec.strike\` is left unset on the proto, matching the terminal's \`if (strike != null) { contractSpecBuilder.setStrike(strike); }\` pattern.
  - **`right`** -- new \`direct::wire_right_opt\` emits \`None\` for \`"*"\` / \`"both"\` / \`"0"\` / empty, matching the terminal's optional-field pattern. Single sides (\`"C"\`, \`"P"\`, \`"call"\`, \`"put"\`) normalize to \`"call"\` / \`"put"\` on the wire as before.
  - **Wired `required_strike` / `optional_strike` accessors** in the endpoint layer (generator now dispatches \`Strike\` to them).
  - Verified live against production across 64 parameter-mode combinations on \`option_snapshot_open_interest\` (every cross of \`{*, 0, YYYYMMDD, YYYY-MM-DD}\` × \`{*, 0, 550, ""}\` × \`{*, both, C, P}\`): every combination returns correct, consistent data (9,966 rows for full chain / 4,983 per side / 454 per expiration / exact single-contract lookups). Bulk query for all of QQQ's open interest now takes ~1s (single call) vs ~22s (34-expiration loop).

  Applies to every option snapshot / history / at-time endpoint that builds a \`ContractSpec\`.

### Added

- **Public API redesign doc** (#282) -- \`docs/public-api-redesign.md\` charters the layered ergonomic-façade migration plan (exact generated surface as canonical parity layer, handwritten \`historical\` / \`realtime\` / \`analytics\` façades on top, typed value foundations, Python result containers, spec enrichment, compatibility window, semver cleanup). The streaming category is named \`realtime\` rather than \`live\` to avoid overloading \`live\`'s CI / run-mode meanings.

### Changed

- **`scripts/check_tier_badges.py` now live-fetches ThetaData's \`openapiv3.yaml\`** (#280) -- removes \`scripts/upstream_tiers.json\` (a data file that didn't belong under \`scripts/\` anyway) and pulls the authoritative \`x-min-subscription\` map at check time, with 4 retries + exponential backoff and fail-closed on exhaustion. Eliminates the manual snapshot-refresh drift vector. The \`Extended Surfaces\` CI job already has outbound access, so no workflow changes required.

## [7.1.0] - 2026-04-14

### Breaking Changes

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

### Docs

- Every Greeks example in the docs-site, READMEs, Python example, and notebooks updated to pass `right: "C"` / `right="C"` / `right: "C"` instead of `is_call: true`.
- Note added to `docs-site/docs/api-reference.md` and `docs/api-reference.md` clarifying that the low-level per-Greek primitives still take `is_call: bool`, while the user-facing aggregates take `right: &str`.
- **Corrected 31 subscription-tier badges across `docs-site/docs/historical/**/*.md`** (#276) -- audit against ThetaData's canonical `openapiv3.yaml` (`x-min-subscription` field) found 31 of 57 endpoint docs advertised the wrong subscription tier. Fixed against upstream truth.
- **Renamed misnamed doc file** (#276) -- `historical/option/at-time/ohlc.md` actually documented the `option_at_time_quote` endpoint; renamed to `quote.md`, fixed the nav link in `docs-site/docs/.vitepress/config.ts`, and updated the sole inbound reference in `historical/option/index.md`.
- **New `scripts/check_tier_badges.py`** (#276) -- validates every `<TierBadge>` in the historical docs against `scripts/upstream_tiers.json`, a checked-in snapshot of ThetaData's authoritative `x-min-subscription` map (with `_source` and `_captured_at` keys for traceability). Wired into `scripts/check_docs_consistency.py` so the existing `Extended Surfaces` CI job gates tier drift automatically. No network calls at CI time.
- **Deleted orphan docs-site pages** (#272) -- removed top-level single-page versions (`getting-started.md`, `historical.md`, `historical/{stock,option,index-data,calendar}.md`, `streaming.md`, `tools/index.md`) superseded by the subdirectory navigation. Added a `## Client Model` section to `docs-site/docs/streaming/index.md` that makes the per-SDK split (Rust/Python unified `ThetaDataDx`, Go/C++ standalone `FpssClient`) unmistakable. Removed `ignoreDeadLinks: true` from `docs-site/docs/.vitepress/config.ts` so future link rot fails the VitePress build.
- **Sidebar landings for Historical Data and Tools sections** (#274) -- added `link:` fields on both top-level sidebar entries so clicking the section headers lands on the category overview. Created a new `tools/index.md` overview describing the CLI / MCP / REST Server trio.

## [7.0.0] - 2026-04-14

### Breaking Changes

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

### Breaking Changes

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

### Documentation

- All 60+ endpoint pages updated: f64 fields, no `price_type`, no `_f64()` helpers.
- All SDK READMEs updated (Rust, Python, Go, C++).
- Streaming docs rewritten for f64 events.
- OpenAPI spec purged of `price_type`.
- JVM deviations doc: new sections for FPSS f64 streaming and `Contract::option` clean API.
- Internal docs (architecture, api-reference, endpoint-schema) updated.
- README now explicitly warns that FPSS is not yet production-ready due to the upstream framing issue tracked in `#192`.

## [5.4.0] - 2026-04-05

### Breaking Changes

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

### Breaking Changes

- **Go SDK**: `EodTick`, `OhlcTick`, `TradeTick`, `QuoteTick`, `TradeQuoteTick`, `PriceTick`, `SnapshotTradeTick` gain additional fields (raw prices, ext_conditions, price_type). `Right` is now `string` ("C"/"P") with `RightRaw int32` for raw access.
- **Python SDK**: trade dicts gain `ext_condition1..4`. Quote/OHLC/EOD/TradeQuote dicts gain raw price and detail fields.
- **Rust**: `normalize_right()` maps `"C"` -> `"call"`, `"P"` -> `"put"`, `"*"` -> `"both"` for v3 server.

### Added

- **`tdbe::exchange`** -- 78 exchange codes with O(1) lookup: `exchange_name()`, `exchange_symbol()`. (#112)
- **`tdbe::conditions`** -- 149 trade conditions + 75 quote conditions with semantic flags (cancel, volume, high, low, last). (#112)
- **`tdbe::sequences`** -- FPSS sequence tracking with wrapping-aware gap detection. (#112)
- **`tdbe::errors`** -- 14 ThetaData HTTP error codes mapped to human-readable names. gRPC errors now include the ThetaData error name. (#113)
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

### Documentation

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

### Breaking Changes

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

### Breaking Changes

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

### Breaking Changes

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

### Breaking Changes

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

### Documented

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

### Fixed (battle-tested against live MDDS -- all 61 endpoints verified)

- **Interval conversion**: MDDS server accepts preset shorthand (`1m`, `5m`, `1h`), not raw milliseconds. `normalize_interval()` now converts `"60000"` -> `"1m"`, `"300000"` -> `"5m"`, etc. Sub-second presets supported: `"100"` -> `"100ms"`, `"500"` -> `"500ms"`. Users can pass either milliseconds or shorthand directly.
- **Default start_time/end_time**: the Java terminal defaults these to `"09:30:00"` and `"16:00:00"`. Our SDK left them as None, causing `"Invalid time format: Expected hh:mm:ss.SSS"` on trade/quote/greeks endpoints. Now defaults to RTH.
- **extract_text_column**: now handles Number and Price DataTable values. `option_list_strikes` was returning 0 results because strikes come as Number values, not Text.
- **FPSS TLS certificate**: ThetaData's FPSS servers have certificates expired since Jan 2024. Skip certificate verification for FPSS connections (matching Java terminal behavior).

### Supported interval presets

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

### Documented

- Greeks operator precedence (veta, speed, zomma, color, dual_gamma) -- Java decompiler may have lost parenthesization, Rust follows textbook Black-Scholes formulas
- FPSS ring buffer capacity monitoring -- documented as known limitation (disruptor-rs v4 has no fill-level API)

## [4.0.0] - 2026-04-01

### Breaking Changes

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
- **TOML-driven codegen** - `tick_schema.toml` is the single source of truth for all 14 tick type definitions and DataTable column schemas. `build.rs` generates Rust structs and parsers at compile time. Adding a new column = one line in the TOML.
- **Proto maintenance guide** (`proto/MAINTENANCE.md`) - step-by-step instructions for ThetaData engineers to add columns, RPCs, or replace proto files.
- 10 new parse functions in `decode.rs` (including `parse_eod_ticks` moved from inline in `direct.rs`)
- All downstream consumers updated: FFI (9 new JSON converters), CLI (9 new renderers), Server (9 new sonic_rs serializers), MCP (9 new serializers), Python SDK (9 new dict converters)
- Crate README (`crates/thetadatadx/README.md`) and FFI README (`ffi/README.md`)
- Python SDK: polars support documented (`pip install thetadatadx[polars]`)

### Fixed

- **Comprehensive documentation sweep** - 6 parallel agents audited every doc page, README, notebook, and example file against the actual source code. Fixed fabricated homepage examples, wrong C++ include paths (`thetadatadx.hpp` -> `thetadx.hpp`), stale `client.` variable names, missing typed return annotations, wrong Python `all_greeks()` parameter name, version pins (`3.0` -> `3.1`), `for_each_chunk` signature in API reference, and MIT license in footer (should be GPL-3.0).
- **Parameter/response display redesign** - replaced flat markdown tables with vertical card layout across 60 endpoint documentation pages.
- Root README streamlined with navigation table (removed 90-line endpoint listing)
- Notebook 105: fixed event kinds and removed raw payload access pattern
- OpenAPI yaml: fixed license, GitHub URLs, removed DataTable response types

## [3.1.0] - 2026-03-27

### Fixed

- **Go SDK: price encoding was fundamentally wrong** - `priceToFloat()` used a switch-case instead of `value * 10^(price_type - 10)`. Every price returned by the Go SDK was incorrect. Now matches Rust exactly.
- **Python docs: streaming examples used wrong event key** - `event["type"]` changed to `event["kind"]` across README and all docs-site pages.
- **`Price::new()` no longer panics in release** - `assert!` replaced with `debug_assert!` + `clamp(0, 19)` with `tracing::warn!`. A corrupt frame no longer crashes production.
- **C++ `FpssClient`: added missing `unsubscribe_quotes()`** - was present in FFI but missing from C++ RAII wrapper.
- **FFI FPSS: mutex poison safety** - all 12 `.lock().unwrap()` calls replaced with `.unwrap_or_else(|e| e.into_inner())`. Prevents undefined behavior (panic across `extern "C"`) on mutex poisoning.
- **`Credentials.password` visibility** - changed from `pub` to `pub(crate)` with `password()` accessor. Prevents accidental credential logging by downstream code.
- **WebSocket server: added OPEN_INTEREST + FULL_TRADES dispatch** - previously silently dropped.
- **C++ SDK type parity** - `MarketValueTick` expanded from 3 to 7 fields, `CalendarDay` added `status`, `InterestRateTick` added `ms_of_day`.
- **Python README: removed ghost methods** - `is_authenticated()` and `server_addr()` were listed but did not exist.
- **Root README: stock method count** - "Stock (13)" corrected to "Stock (14)".

## [3.0.0] - 2026-03-27

### Breaking Changes

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

### New Products

- **`tdx` CLI** (`tools/cli/`) — command-line tool with all 61 endpoints + Greeks + IV.
  Dynamically generated from endpoint registry. `cargo install thetadatadx-cli`
- **MCP Server** (`tools/mcp/`) — Model Context Protocol server giving LLMs instant
  access to 64 tools (61 endpoints + ping + greeks + IV) over JSON-RPC stdio.
  Works with Claude Code, Cursor, Codex.
- **REST+WS Server** (`tools/server/`) — drop-in replacement for the Java terminal.
  v3 API on port 25503, WebSocket on 25520 with real FPSS bridge. sonic-rs JSON.
- **VitePress documentation site** (`docs-site/`) — 33 pages covering API reference,
  guides, SDK docs, wire protocol internals. Deployed to GitHub Pages.

### Breaking Changes

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
- **All Gemini findings** — hot-path allocations, wrapping_add, BufWriter, find_header
  fallback, DATE marker handling, MCP sanitization, Price dedup
- **All Codex findings** — server security (CORS, shutdown auth), CLI expect(), MCP
  JSON-RPC validation, stale docs
- **Auth response parsing** — subscription fields are integers not strings

### Performance

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

### Added (PR #13)

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

### Fixed (PR #12)

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

### Performance

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
- **Python SDK: pandas DataFrame conversion** — `to_dataframe()` function and `_df` method
  variants on DirectClient (e.g. `stock_history_eod_df()`); install with
  `pip install thetadatadx[pandas]`
- **FFI crate: FPSS support** — 7 new `extern "C"` functions for FPSS lifecycle
  (`fpss_connect`, `fpss_subscribe_quotes`, `fpss_subscribe_trades`,
  `fpss_subscribe_open_interest`, `fpss_next_event`, `fpss_shutdown`, `fpss_free_event`)
- **Go SDK: FPSS streaming** — `FpssClient` Go struct wrapping the FFI FPSS functions
- **C++ SDK: FPSS streaming** — `FpssClient` C++ RAII class wrapping the FFI FPSS functions

### Fixed

- Version bump for crates.io/PyPI publish (v1.1.0 tag was re-pushed during history restore)

### Performance

- All TODO performance items now complete: streaming iterator (`for_each_chunk`),
  faster `norm_cdf` (Horner-form), concurrent request semaphore (`mdds_concurrent_requests`)

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

### Verified

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
- Switched license to GPL-3.0-or-later
- Added disclaimer, legal considerations, and EU interoperability section
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
- **Documentation** — architecture (Mermaid), API reference, reverse-engineering guide, JVM deviations
- **CI/CD** — GitHub Actions (fmt, clippy, test, FFI build, crates.io publish, PyPI publish, GitHub Release)
- **Project infrastructure** — CHANGELOG, CONTRIBUTING, SECURITY, CODE_OF_CONDUCT,
  clippy.toml, cliff.toml, rust-toolchain.toml, LICENSE (GPL-3.0-or-later)

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

[Unreleased]: https://github.com/userFRM/ThetaDataDx/compare/v7.0.0...HEAD
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
