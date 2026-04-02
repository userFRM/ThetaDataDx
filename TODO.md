# TODO

Backlog of outstanding work, ordered by priority.

## High Priority

- [x] ~~Extend `build.rs` to generate C header from `endpoint_schema.toml`~~ — replaced by hand-written `thetadx.h` with `#[repr(C)]` struct definitions (v4.5.0, PR #41)
- [x] ~~Extend `build.rs` to generate C++ structs + DataTable parsers from TOML~~ — replaced by `#[repr(C)]` FFI; C++ reads native structs directly, no JSON parsing (v4.5.0, PR #41)
- [x] ~~Extend `build.rs` to generate Go structs + DataTable parsers from TOML~~ — replaced by `#[repr(C)]` FFI; Go reads native structs via unsafe.Slice (v4.5.0, PR #41)
- [x] ~~Extend `build.rs` to generate Python dict converters from TOML~~ — Python uses PyO3 field-by-field dict conversion (gold standard, no codegen needed)
- [x] ~~Extend `build.rs` to generate FFI JSON serializers from TOML~~ — eliminated entirely; FFI returns `#[repr(C)]` typed arrays, zero JSON (v4.5.0, PR #41)
- [ ] Verify Go SDK CGo builds on Linux + macOS
- [ ] Verify C++ SDK CMake build with FFI static library

## Medium Priority

- [x] ~~FPSS integration test: TLS handshake to `nj-a.thetadata.us:20000`~~ — verified live (v4.2.0); TLS cert skip added for expired certs
- [x] ~~FPSS integration test: CREDENTIALS -> METADATA round-trip~~ — verified live; login returns `STOCK.STANDARD, OPTION.STANDARD, INDEX.FREE`
- [ ] FPSS integration test: sustained volume during market hours (no dropped messages)
- [ ] FPSS integration test: kill TCP mid-stream, verify re-subscribe
- [ ] FPSS integration test: trigger `TooManyRequests`, verify 130s backoff
- [x] ~~MDDS integration test: terminal version negotiation (`terminal_git_commit`)~~ — sends empty string (documented deviation); server accepts it
- [ ] Optional RPS rate limiter in `DirectConfig` (requested by ThetaData for server protection)

## Low Priority

- [ ] Add `tracing` spans on all network operations
- [ ] Metrics export (request count, latency histograms, reconnect count)
- [ ] Runtime config loading from `config.toml` / `config.properties`
- [x] ~~Split wire format types into `thetadatadx-wire` crate~~ — done as `tdbe` crate (ThetaData Binary Encoding, v4.0.0)
