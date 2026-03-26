# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — v1.2.0

### Fixed (Audit — PR #4, commit 39f2d5f)

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

**Observability**

15. **Column lookup warns instead of silent fallback** — `extract_*_column` functions now
    emit a `warn!` log when a requested column header is not found in the DataTable,
    instead of silently returning a vec of `None`s. This makes schema mismatches
    immediately visible in logs.

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

[Unreleased]: https://github.com/userFRM/ThetaDataDx/compare/v1.1.1...HEAD
[1.1.1]: https://github.com/userFRM/ThetaDataDx/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/userFRM/ThetaDataDx/compare/v1.0.1...v1.1.0
[1.0.1]: https://github.com/userFRM/ThetaDataDx/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/userFRM/ThetaDataDx/releases/tag/v1.0.0
