# TODO

Backlog of outstanding work, ordered by priority.

## High Priority

- [ ] Extend `build.rs` to generate C header (`thetadatadx_types.h`) from `endpoint_schema.toml`
- [ ] Extend `build.rs` to generate C++ structs + DataTable parsers from TOML
- [ ] Extend `build.rs` to generate Go structs + DataTable parsers from TOML
- [ ] Extend `build.rs` to generate Python dict converters from TOML
- [ ] Extend `build.rs` to generate FFI JSON serializers from TOML
- [ ] Verify Go SDK CGo builds on Linux + macOS
- [ ] Verify C++ SDK CMake build with FFI static library

## Medium Priority

- [ ] FPSS integration test: TLS handshake to `nj-a.thetadata.us:20000`
- [ ] FPSS integration test: CREDENTIALS -> METADATA round-trip
- [ ] FPSS integration test: sustained volume during market hours (no dropped messages)
- [ ] FPSS integration test: kill TCP mid-stream, verify re-subscribe
- [ ] FPSS integration test: trigger `TooManyRequests`, verify 130s backoff
- [ ] MDDS integration test: terminal version negotiation (`terminal_git_commit`)
- [ ] Optional RPS rate limiter in `DirectConfig` (requested by ThetaData for server protection)

## Low Priority

- [ ] Add `tracing` spans on all network operations
- [ ] Metrics export (request count, latency histograms, reconnect count)
- [ ] Runtime config loading from `config.toml` / `config.properties`
- [ ] Split wire format types into `thetadatadx-wire` crate
