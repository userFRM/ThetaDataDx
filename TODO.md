# TODO — Production Readiness Checklist

## Integration Testing

- [x] Connect to `mdds-01.thetadata.us:443` with real credentials and verify gRPC handshake
- [x] Send a real `StockHistoryEod` request and verify response decompresses + parses correctly
- [x] Verify `QueryInfo.client_type = "rust-thetadatadx"` is accepted
- [x] Test full auth chain: creds → Nexus POST → session UUID → gRPC request → data
- [ ] Test terminal version negotiation — does the server care about `terminal_git_commit`?
- [ ] Connect to FPSS `nj-a.thetadata.us:20000` and verify TLS handshake
- [ ] Send CREDENTIALS message and verify METADATA response
- [ ] Subscribe to a quote stream and verify FIT-decoded ticks match Java terminal output
- [ ] Verify delta decompression produces correct absolute values across multiple ticks
- [ ] Run FPSS during market hours and verify no dropped messages at sustained volume
- [ ] Compare output byte-for-byte with Java terminal for the same query
- [ ] Test reconnection: kill TCP connection mid-stream, verify re-subscribe
- [ ] Test rate limiting: trigger `TooManyRequests` and verify 130s backoff

## Code Review Findings

All 19 items resolved.

## Runtime Configuration (JVM parity)

- [x] All JVM-equivalent config knobs implemented
- [x] `mdds_concurrent_requests` — max in-flight gRPC requests (configurable semaphore, default 2)

## Performance (trading-grade)

All merged to main:
- [x] `#[repr(C, align(64))]` on tick types
- [x] Cached `QueryInfo` template in DirectClient
- [x] Precomputed DataTable column indices
- [x] `#[inline]` on all hot-path functions
- [x] Precomputed `10i64.pow()` lookup table for Price
- [x] Reusable thread-local zstd decompressor
- [x] Fully sync FPSS — `disruptor-rs` v4 LMAX ring buffer, zero tokio
- [x] AdaptiveWaitStrategy (spin/yield/hint)
- [x] Criterion benchmarks
- [x] Streaming `for_each_chunk` callback on DirectClient (streaming iterator alternative)
- [x] Faster `norm_cdf` — Horner-form Zelen & Severo approximation (~1e-7 accuracy)

## Architecture Improvements

- [ ] Split wire format types into `thetadatadx-wire` crate
- [x] Async-zstd streaming decompression (feature-gated)
- [ ] `tracing` spans on all network operations
- [ ] Metrics (request count, latency histograms, reconnect count)
- [ ] Load config from `config.toml` / `config.properties` at runtime

## SDK Completeness

- [x] All 61 endpoints in Python, Go, C++, C FFI
- [x] Python SDK: FPSS streaming (FpssClient with subscribe/next_event/shutdown)
- [x] Python SDK: pandas DataFrame conversion (`to_dataframe()` + `_df` variants)
- [x] FFI crate: 7 FPSS extern C functions
- [x] Go SDK: FpssClient struct wrapping FFI FPSS
- [x] C++ SDK: FpssClient RAII class wrapping FFI FPSS
- [ ] Go SDK: verify CGo builds on Linux + macOS
- [ ] C++ SDK: verify CMake build with FFI static library
- [x] Published to crates.io and PyPI
