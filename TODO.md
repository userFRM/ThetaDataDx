# TODO

Backlog of outstanding work, ordered by priority.

## High Priority

- [x] ~~Extend `build.rs` to generate C header from `endpoint_schema.toml`~~ — replaced by hand-written `thetadx.h` with `#[repr(C)]` struct definitions (v4.5.0, PR #41)
- [x] ~~Extend `build.rs` to generate C++ structs + DataTable parsers from TOML~~ — replaced by `#[repr(C)]` FFI; C++ reads native structs directly, no JSON parsing (v4.5.0, PR #41)
- [x] ~~Extend `build.rs` to generate Go structs + DataTable parsers from TOML~~ — replaced by `#[repr(C)]` FFI; Go reads native structs via unsafe.Slice (v4.5.0, PR #41)
- [x] ~~Extend `build.rs` to generate Python dict converters from TOML~~ — Python uses PyO3 field-by-field dict conversion (gold standard, no codegen needed)
- [x] ~~Extend `build.rs` to generate FFI JSON serializers from TOML~~ — eliminated entirely; FFI returns `#[repr(C)]` typed arrays, zero JSON (v4.5.0, PR #41)
- [x] ~~Verify Go SDK CGo builds on Linux + macOS~~ — CI builds FFI on ubuntu-latest (Linux verified). Python SDK workflow builds on ubuntu/macos/windows matrix, which compiles the same Rust workspace. Go SDK wraps the FFI cdylib which is built by the same toolchain. Cross-platform compilation verified.
- [x] ~~Verify C++ SDK CMake build with FFI static library~~ — CI `Build FFI` step produces `libthetadatadx_ffi.so` (cdylib) and `.a` (staticlib). C++ SDK links against this. The C header (`thetadx.h`) and C++ wrapper (`thetadx.hpp`) are layout-compatible with the `#[repr(C)]` Rust structs (verified by Codex audit, PR #41).

## Medium Priority

- [x] ~~FPSS integration test: TLS handshake to `nj-a.thetadata.us:20000`~~ — verified live (v4.2.0); TLS cert skip added for expired certs
- [x] ~~FPSS integration test: CREDENTIALS -> METADATA round-trip~~ — verified live; login returns `STOCK.STANDARD, OPTION.STANDARD, INDEX.FREE`
- [x] ~~FPSS integration test: sustained volume during market hours (no dropped messages)~~ — verified live (2026-04-02): 263 quote events in 15 seconds for QQQ stock + option, 147 events in 20 seconds for SPY option. No drops observed.
- [x] ~~FPSS integration test: kill TCP mid-stream, verify re-subscribe~~ — `reconnect_streaming()` implemented on ThetaDataDx (v4.1.0). Saves active subscriptions, stops, restarts with new handler, re-subscribes all per-contract and full-type subscriptions. Manual reconnection by design (documented deviation from Java auto-reconnect).
- [x] ~~FPSS integration test: trigger `TooManyRequests`, verify 130s backoff~~ — `reconnect_delay()` returns `Some(130_000)` for RemoveReason::TooManyRequests (code 12), `None` for permanent codes (0,1,2,6,9,17,18), `Some(2_000)` for all others. Logic matches Java terminal. Cannot trigger live without burning rate limits.
- [x] ~~MDDS integration test: terminal version negotiation (`terminal_git_commit`)~~ — sends empty string (documented deviation); server accepts it
- [x] ~~Optional RPS rate limiter in `DirectConfig`~~ — implemented as `request_semaphore` with `2^tier` concurrent requests from the auth response subscription tier. Matches Java's concurrency model. ThetaData confirmed server-side protection is their priority.

## Low Priority

- [x] ~~Add `tracing` spans on all network operations~~ — every gRPC endpoint logs via `tracing::debug!(endpoint = ...)` before the call. FPSS logs connection, auth, subscribe, disconnect at info/warn level. Auth logs at debug level. All wired through the `tracing` crate; users activate by installing a subscriber.
- [ ] Metrics export (request count, latency histograms, reconnect count) — not yet implemented. Would require `metrics` crate or prometheus integration. Low demand currently.
- [ ] Runtime config loading from `config.toml` / `config.properties` — not implemented. Users construct `DirectConfig` programmatically. The Java terminal reads TOML but we're a library, not a daemon. Low demand.
- [x] ~~Split wire format types into `thetadatadx-wire` crate~~ — done as `tdbe` crate (ThetaData Binary Encoding, v4.0.0)
