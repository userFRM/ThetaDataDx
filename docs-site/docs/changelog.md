# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

- Go SDK `priceToFloat()` was fundamentally wrong -- used a switch statement instead of `value * 10^(type-10)`. Every price in the Go SDK was incorrect.
- Python docs used `event["type"]` instead of `event["kind"]` in streaming examples
- `Price::new()` used `assert!` in release builds -- changed to `debug_assert!` + silent clamp. Corrupt frames no longer crash production.
- C++ `FpssClient` missing `unsubscribe_quotes()` method
- FFI FPSS functions used `.lock().unwrap()` -- changed to poison recovery
- WebSocket handler missing `OPEN_INTEREST` and `FULL_TRADES` dispatch
- `Credentials.password` changed from `pub` to `pub(crate)` with accessor
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

See [TODO.md](TODO.md) for the production readiness checklist and performance roadmap.

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

[Unreleased]: https://github.com/userFRM/ThetaDataDx/compare/v3.2.2...HEAD
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
