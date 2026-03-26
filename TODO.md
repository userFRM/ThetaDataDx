# TODO — Production Readiness Checklist

## Integration Testing (BLOCKING — nothing is verified against real servers)

- [x] Connect to `mdds-01.thetadata.us:443` with real credentials and verify gRPC handshake
- [x] Send a real `StockHistoryEod` request and verify response decompresses + parses correctly
- [x] Verify `QueryInfo.client_type = "rust-thetadatadx"` is accepted (ThetaData may reject unknown clients)
- [ ] Test terminal version negotiation — does the server care about `terminal_git_commit`?
- [ ] Connect to FPSS `nj-a.thetadata.us:20000` and verify TLS handshake (cipher suites, SNI)
- [ ] Send CREDENTIALS message and verify METADATA response
- [ ] Subscribe to a quote stream and verify FIT-decoded ticks match Java terminal output
- [ ] Verify delta decompression produces correct absolute values across multiple ticks
- [x] Test full auth chain: creds → Nexus POST → session UUID → gRPC request → data
- [ ] Run FPSS during market hours and verify no dropped messages at sustained volume
- [ ] Compare output byte-for-byte with Java terminal for the same query (EOD, OHLC, trade)
- [ ] Test reconnection: kill TCP connection mid-stream, verify auto-reconnect + re-subscribe
- [ ] Test rate limiting: trigger `TooManyRequests` and verify 130s backoff

## Code Review Findings

- [x] Add timeout to FPSS read loop (read_frame wrapped in tokio::time::timeout)
- [x] Add timeout to FPSS TLS handshake (entire connect+TLS wrapped)
- [x] Guard Greeks functions against `t==0` and `v==0` — early return with finite values
- [x] Precompute d1/d2 in `all_greeks` — shared intermediates, O(1) computation
- [x] Use checked arithmetic in FIT decoder — i64 accumulator with i32 saturation
- [x] Validate contract root length <= 244 in `protocol.rs` (assert before u8 cast)
- [x] Redact session UUID from logs (bearer token was at info! level)
- [x] Remove Debug from AuthRequest (contained raw password)
- [x] Add reqwest timeouts (10s request, 5s connect)
- [x] Wire DirectConfig knobs into gRPC channel (keepalive, max-message-size, connect-timeout)
- [x] Fix truncated FPSS frame header — no longer treated as clean EOF
- [x] Surface FPSS parse errors as FpssEvent::Error (was silently dropped)
- [x] Price::new uses assert! not debug_assert! for price_type validation
- [x] Price comparison overflow uses f64 fallback (was wrong for negatives)
- [x] Treat `InvalidCredentials` + 6 other codes as permanent disconnect
- [x] Add `contract_lookup(id)` to FpssClient — single entry lookup, no HashMap clone
- [x] Unify TLS stack on rustls — removed native-tls, FPSS now uses tokio-rustls + webpki-roots
- [x] Add date parameter validation — rejects anything not exactly 8 ASCII digits (YYYYMMDD)
- [x] Use `.exp()` instead of `E.powf()` in Greeks — removed `std::f64::consts::E` import entirely

## Runtime Configuration (JVM parity)

- [x] `fpss_queue_depth` — event channel buffer (JVM default: 1,000,000 via `FPSS_QUEUE_DEPTH`)
- [x] `tokio_worker_threads` — number of async worker threads (JVM equivalent: `-Xmx` + thread pool)
- [x] `mdds_max_message_size` — gRPC max inbound message (JVM: 4MB default, 10MB max)
- [x] `mdds_keepalive_secs` — gRPC keepalive interval (JVM: 30s)
- [x] `mdds_keepalive_timeout_secs` — gRPC keepalive timeout (JVM: 10s)
- [x] `fpss_ping_interval_ms` — heartbeat interval (JVM: 100ms)
- [x] `fpss_connect_timeout_ms` — per-server TCP connect timeout (JVM: 2s)
- [ ] `mdds_concurrent_requests` — max in-flight gRPC requests (JVM: 2^tier, 1-16)
- [x] `fpss_reconnect_wait_rate_limited_ms` — backoff for TooManyRequests (JVM: 130s)

## Performance (trading-grade low-latency)

The crate is correct and clean but not yet trading-grade. These optimizations follow
the ibx playbook (cache-line aligned structs, zero-alloc hot paths, pre-allocated buffers).

- [ ] `#[repr(C, align(64))]` on tick types — cache-line alignment prevents false sharing
- [ ] Cache `QueryInfo` template in DirectClient — `query_info()` currently allocates 3 Strings per request
- [ ] Precompute DataTable column indices once outside the row loop — currently O(headers) per row
- [ ] Stack-allocated `ArrayVec` in FIT decoder — `flush_digits` currently allocates on heap
- [ ] Precomputed `10i64.pow()` lookup table for Price comparison — currently computes per compare
- [ ] `#[inline]` / `#[inline(always)]` on hot-path functions (FIT nibble processing, price conversion, tick field accessors)
- [x] Lock-free ring buffer for FPSS events — implemented on `perf` branch using `disruptor-rs` v4 (LMAX Disruptor pattern); `main` branch retains `tokio::mpsc` for simplicity
- [ ] Reusable buffer pool for zstd decompression — currently allocates a new Vec per chunk
- [ ] Streaming iterator for `collect_stream` — currently materializes all rows into a Vec
- [ ] Faster `norm_cdf` in Greeks — current Abramowitz & Stegun uses 5 multiplies, could use rational approximation or lookup table

## Architecture Improvements

- [ ] Split wire format types into a separate `thetadatadx-wire` crate for reuse by downstream consumers
- [x] Add async-zstd streaming decompression (feature-gated via `async-zstd`)
- [ ] Add `tracing` spans to all network operations for observability
- [ ] Add metrics (request count, latency histograms, reconnect count)
- [ ] Support loading config from `config.toml` or `config.properties` at runtime (not just `DirectConfig` struct)
- ~~Add a ThetaClient trait~~ — removed: there is no legacy client. thetadatadx IS the terminal.

## SDK Completeness

- [ ] Python SDK: expose FPSS streaming (currently only DirectClient/historical)
- [ ] Python SDK: add pandas DataFrame conversion for tick vectors
- [ ] Go SDK: verify CGo builds on Linux + macOS
- [ ] C++ SDK: verify CMake build with the FFI static library
- [ ] Add integration test examples that run against dev servers
- [ ] Publish to crates.io, PyPI, pkg.go.dev

## Project Infrastructure

- [x] CHANGELOG.md — Keep a Changelog format with v0.1.0 + Unreleased
- [x] CONTRIBUTING.md — dev setup, code style, PR process, Discord link
- [x] SECURITY.md — disclosure policy, credential handling, TLS design
- [x] CODE_OF_CONDUCT.md — Contributor Covenant v2.1
- [x] deny.toml — cargo-deny advisory/license/ban/source controls
- [x] clippy.toml — threshold overrides
- [x] cliff.toml — git-cliff conventional commits
- [x] rust-toolchain.toml — stable + rustfmt + clippy
- [x] LICENSE (GPL-3.0-or-later)
- [x] .github/workflows/ci.yml — fmt, clippy, test, deny, FFI build
- [x] .github/ISSUE_TEMPLATE/ — bug report + feature request
- [x] .github/PULL_REQUEST_TEMPLATE.md — checklist
- [x] config.default.toml + config.default.properties — reference configs
- [x] README.md — badges (build, docs.rs, license, crates.io, pypi, Discord)
- [x] Cargo.toml metadata — authors, keywords, categories, docs.rs, repository
- [x] `#![cfg_attr(docsrs, feature(doc_auto_cfg))]` — feature visibility on docs.rs
