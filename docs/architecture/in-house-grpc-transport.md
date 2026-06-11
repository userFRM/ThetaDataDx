# In-house gRPC transport

## Context

ThetaDataDx ships a hand-written h2-over-rustls gRPC transport in `crates/thetadatadx/src/grpc/`. The transport plus the two-stage decode pipeline plus the per-channel decoder pool totals roughly 3.5K LOC. The industry-standard alternative (`tonic` + tokio) could replace the consumer-facing parts in roughly 200 LOC. <!-- VOCAB-OK: naming the alternative crate for engineering comparison -->

Automated audits have flagged this as "bench or collapse to standard stack."

## Decision

Keep the in-house transport for the current release line. Revisit when a real benchmark under sustained load proves the standard stack is within 10% at peak throughput.

## Rationale

**1. Decode pipeline fan-out.** The MDDS wire format is zstd-compressed protobuf `DataTable` frames; each frame can hold up to roughly 10 MB of data. The two-stage pipeline runs zstd decompress on the per-channel decoder thread and prost decode + Tick build on a shared worker pool. The standard `tonic`-based approach runs per-request decode on the request future's polling thread; in ThetaDataDx's workload that thread also runs h2 framing, which creates a head-of-line block on the slowest decode. The in-house pipeline decouples decompression latency from h2 framing by design. <!-- VOCAB-OK: naming the alternative crate for engineering comparison -->

**2. Ring-based backpressure.** The in-house transport's decoder pool uses a bounded ring per channel; the producer parks when the ring is full rather than buffering unboundedly. The standard `Streaming<T>` type buffers internally; saturating a stream blocks the underlying h2 frame poll, propagating slowness back to the server as window stall. Our model is per-channel-bounded, not per-request-bounded, which gives operators a predictable memory ceiling at any subscription fan-out. <!-- VOCAB-OK: naming the alternative crate type for engineering comparison -->

**3. No third-party error surface.** The dominant alternative pulls in `prost-derive` codegen at build time and surfaces `Status` / `Code` types that bleed through into consumer error types. The in-house transport exposes a typed `Error` enum with no external framework types in the SemVer commitment. Every type in the error hierarchy is one the project owns and can evolve independently.

## Open question

If a future profile under sustained 10K+ ev/s shows the in-house transport is not meaningfully faster than the standard stack at the same saturation level, collapse to the standard stack. The current verified workload tops out at roughly 500 ev/s, where rationale #3 (surface hygiene) is the dominant constraint.

## Revisit trigger

A benchmark run at peak load showing the standard gRPC stack within 10% of the in-house transport on end-to-end decode latency is the signal to collapse. Until that data exists, the in-house transport stays.

**Status (2026-06-11): the trigger condition is met.** The measured comparison below shows the reference stack at parity or ahead in every production-reachable cell.

**Status (2026-06-11, superseding): migrated.** The transport now rides the reference stack; see the "Migration (2026-06-11)" section below for the decision record, the fan-in and pool-topology measurements that closed the open questions, and the post-migration baseline. The sections above are retained as the historical record of the in-house design and the comparison that retired it.

## Measured comparison (2026-06-11)

Closed-loop benchmark: `benches/grpc_transport_comparison.rs` (run with `cargo bench -p thetadatadx --features __test-helpers --bench grpc_transport_comparison`). Both clients issue `GetStockHistoryEod`-shaped RPCs against the same in-process loopback mock h2 server, send the identical prost-encoded request, receive the identical zstd-compressed `ResponseData` frame, and perform the same decode work (zstd decompress + prost `DataTable` decode + row merge).

- **in-house** — `Channel`/`ChannelPool` with the production two-stage decoder pipeline attached exactly as `MddsClient::connect` wires it (stage-1 decoder threads with 256-slot rings, stage-2 prost workers = core count, queue depth = pool size x 64).
- **reference** — the reference Rust gRPC stack (`tonic` + tokio): hand-rolled `Grpc<Channel>` client with `ProstCodec`, decoding each chunk inline on the request task — the canonical shape a generated client produces, i.e. the ~200-LOC replacement this ADR weighs. <!-- VOCAB-OK: naming the alternative crate for engineering comparison -->

### Setup

| Item | Value |
|---|---|
| Host | Intel Core i7-10700KF (8C/16T), 128 GB DDR4, Ubuntu 24.04, kernel 6.8.0 |
| Toolchain | rustc 1.96.0, release (bench) profile |
| Server | in-process mock h2 (same harness as the integration tests), loopback only, pre-framed response cloned per request |
| Connections | min(concurrency, 16) TCP connections per side — one per worker at production-reachable levels, multiplexed above |
| h2 windows | both stacks at the h2-crate defaults (65 535 B stream + connection initial windows, 16 384 B max frame) |
| Decode ceiling | 4 MiB both sides (small), 64 MiB both sides (large) |
| Protocol | closed loop, warmed (1.5-4 s warmup per cell), 3 measured repeats per cell (6 s small / 12 s large each) |
| Payloads | `small` = 1 014 B wire frame (10 rows); `large` = 10.0 MiB wire frame (130 926 rows, 12.7 MiB decoded table); deterministic seed |

**Concurrency context.** The upstream service caps each account at 16 concurrent requests, and the SDK's tier semaphore enforces `2^tier` (Free 1 / Value 2 / Standard 4 / Pro 8, `src/mdds/tier.rs`), so 1-16 is the verdict-driving range and 16 is "peak". The 100 / 1000 rows are a synthetic headroom appendix against the local mock — above the upstream per-account ceiling, transport headroom only, not customer-reachable.

### Small frames (1 014 B wire)

| concurrency | transport | p50 | p99 | p99.9 | mean | req/s (min..max across repeats) | wire MB/s | alloc/req | cpu/req |
|---|---|---|---|---|---|---|---|---|---|
| 1 | in-house | 87.7 us | 171.8 us | 187.0 us | 92.1 us | 10 736 (10 704..10 774) | 10.9 | 77.0 KiB | 127 us |
| 1 | reference | 81.3 us | 121.7 us | 155.5 us | 84.4 us | 11 782 (11 701..11 855) | 11.9 | 32.5 KiB | 115 us |
| 2 | in-house | 88.1 us | 172.7 us | 185.0 us | 93.6 us | 21 112 (21 101..21 121) | 21.4 | 77.0 KiB | 109 us |
| 2 | reference | 86.4 us | 117.5 us | 144.1 us | 87.7 us | 22 639 (22 488..22 729) | 23.0 | 32.5 KiB | 124 us |
| 4 | in-house | 89.8 us | 176.4 us | 201.2 us | 98.4 us | 40 067 (40 054..40 077) | 40.6 | 77.0 KiB | 107 us |
| 4 | reference | 91.9 us | 119.7 us | 177.3 us | 92.5 us | 42 873 (42 754..42 966) | 43.5 | 32.5 KiB | 129 us |
| 8 | in-house | 96.9 us | 194.8 us | 346.0 us | 118.8 us | 66 282 (66 200..66 418) | 67.2 | 77.0 KiB | 112 us |
| 8 | reference | 105.2 us | 192.3 us | 367.3 us | 108.0 us | 73 373 (73 086..73 564) | 74.4 | 32.5 KiB | 137 us |
| **16** | **in-house** | **171.4 us** | **420.4 us** | **725.6 us** | **176.4 us** | **89 592 (89 487..89 754)** | **90.8** | **77.0 KiB** | **108 us** |
| **16** | **reference** | **138.8 us** | **389.9 us** | **730.9 us** | **150.1 us** | **105 392 (104 717..105 911)** | **106.9** | **32.6 KiB** | **127 us** |
| 100 (synthetic) | in-house | 681.7 us | 1.53 ms | 4.06 ms | 720.6 us | 137 944 (135 043..140 112) | 139.9 | 78.0 KiB | 93 us |
| 100 (synthetic) | reference | 705.9 us | 1.46 ms | 1.99 ms | 727.9 us | 137 084 (131 488..141 231) | 139.0 | 33.6 KiB | 92 us |
| 1000 (synthetic) | in-house | 5.50 ms | 12.19 ms | 14.94 ms | 5.70 ms | 175 176 (174 926..175 334) | 177.6 | 78.3 KiB | 73 us |
| 1000 (synthetic) | reference | 5.10 ms | 10.46 ms | 12.69 ms | 5.27 ms | 189 510 (188 355..191 670) | 192.2 | 33.8 KiB | 71 us |

### Large frames (10.0 MiB wire)

10 MiB frames above the 16-concurrent ceiling would measure allocator pressure rather than transport, so the synthetic levels are small-frame only.

| concurrency | transport | p50 | p99 | p99.9 | mean | req/s (min..max across repeats) | wire MB/s | alloc/req | cpu/req |
|---|---|---|---|---|---|---|---|---|---|
| 1 | in-house | 67.73 ms | 99.94 ms | 111.00 ms | 68.60 ms | 14 (13..14) | 143.5 | 126.4 MiB | 99 537 us |
| 1 | reference | 55.68 ms | 64.05 ms | 69.85 ms | 56.73 ms | 16 (16..17) | 171.5 | 94.6 MiB | 61 171 us |
| 2 | in-house | 65.77 ms | 82.56 ms | 91.58 ms | 66.51 ms | 28 (27..29) | 291.8 | 126.4 MiB | 85 107 us |
| 2 | reference | 60.14 ms | 78.58 ms | 84.54 ms | 60.73 ms | 30 (30..31) | 317.8 | 94.6 MiB | 66 707 us |
| 4 | in-house | 74.06 ms | 93.73 ms | 99.14 ms | 75.19 ms | 48 (47..48) | 502.0 | 126.4 MiB | 92 599 us |
| 4 | reference | 68.37 ms | 91.83 ms | 96.98 ms | 70.04 ms | 50 (50..51) | 528.6 | 94.5 MiB | 81 452 us |
| 8 | in-house | 99.90 ms | 123.40 ms | 136.74 ms | 100.37 ms | 69 (69..69) | 723.3 | 126.4 MiB | 123 681 us |
| 8 | reference | 95.60 ms | 127.21 ms | 138.07 ms | 97.76 ms | 69 (69..70) | 726.6 | 94.5 MiB | 118 401 us |
| **16** | **in-house** | **168.59 ms** | **220.21 ms** | **244.23 ms** | **169.68 ms** | **79 (79..79)** | **827.7** | **126.4 MiB** | **183 506 us** |
| **16** | **reference** | **161.09 ms** | **207.70 ms** | **239.75 ms** | **163.56 ms** | **81 (81..82)** | **854.2** | **94.6 MiB** | **186 332 us** |

### Reading

- **Peak production-reachable load (concurrency 16):** the reference stack delivers **+17.6% throughput** and **-19% p50** on small frames, and **+3.2% throughput** and **-4.4% p50** on large frames. It is at parity or ahead in every cell of the 1-16 range on every latency percentile and on throughput.
- **Allocation:** the reference stack allocates **2.4x less** per small request (32.5 KiB vs 77.0 KiB) and **1.34x less** per large request (94.6 MiB vs 126.4 MiB). The in-house decoder-pool handoff (ring slot, reply channel, cross-thread buffers) is the difference.
- **CPU:** on a single in-flight large frame the in-house pipeline burns **63% more CPU** per request (99.5 ms vs 61.2 ms) for a slower result — the two-stage handoff costs more than the head-of-line blocking it avoids when the caller is itself waiting on the decode. At concurrency 16 large, CPU per request converges (183.5 ms vs 186.3 ms). On small frames the in-house transport shows *lower* CPU per request at concurrency 8-16 (108 us vs 127 us) while delivering less throughput on an unsaturated box — a utilization ceiling in the in-house path (requests queue while cores idle), not an efficiency win; at full saturation (synthetic 1000) the two converge (73 us vs 71 us).
- **Variance:** throughput min..max across the 3 repeats is under 1.5% for nearly every cell (worst: 7% on synthetic small/100). One cell shows a notable run-to-run swing across full benchmark invocations: large/concurrency-1 in-house measured 58.3-67.7 ms p50 across runs; the reference stack stayed at 55.7 ms in both. The verdict does not hinge on it.
- **Decision rule from the audit:** "reference stack within 10% of the in-house transport at peak load means collapse to the standard stack." The condition is met with margin — it is not within 10% behind; it is ahead.

### Honest limits of this measurement

- The mock server runs in-process over loopback; allocation and CPU figures include the (identical) server-side cost of each RPC. Absolute numbers are not WAN numbers; the A/B delta is the signal.
- Single-frame responses (the dominant production shape for these endpoints). Multi-chunk streams that interleave many channels into one decoder pool — the fan-in shape rationale #2 describes — were not measured here. (Closed during the migration; see "Open question 1" below.)
- TLS was off for both stacks (h2c); both production paths use rustls over the same h2 layer.
- Warmed measurements only; the cold-cache axis from the original brief was not measured.
- The box is shared; every reported cell ran in a verified-quiet window (a sampler re-ran any cell that overlapped background compile activity), with 3 repeats per cell.

## Migration (2026-06-11)

### Decision

Collapse to the reference stack. `crates/thetadatadx/src/grpc/` is now a thin client over `tonic` (client-side `channel` transport only): per-endpoint server-streaming calls with per-chunk zstd + prost decode inline on the request task — the measured-fastest shape. <!-- VOCAB-OK: naming the adopted crate in the decision record --> The in-house h2 transport, the two-stage decode pipeline, and the per-channel decoder pool are deleted (roughly 6.2K LOC of `src/grpc/` machinery plus 3.3K LOC of dedicated benches and reconnect-internals tests).

What carries over unchanged at the `MddsClient` surface: the tier semaphore, the retry policy with jitter + wall-clock envelope, the `google.rpc.RetryInfo` cooldown clamp, per-call deadlines, the connection-level vs stream-level fault taxonomy (`ConnectionClosed` vs `H2Stream`, classified by downcasting the `h2::Error` carried in the stack's error source chains), and the N-channel pool with least-loaded picks. Connection recycling after GOAWAY moves from the hand-rolled single-flight reconnect to the stack's built-in lazy reconnect; the behavioral contract (transient fault, next dispatch lands on a fresh connection, pool slot identity preserved) is pinned by `tests/test_pool_reconnect.rs`.

Rationale #3 (surface hygiene) survives the migration intact: no third-party transport type appears in any public signature. The third-party status converts once, inside `src/grpc/status.rs`, into the crate's own `Status` (code, message, decoded RetryInfo hint), and every transport fault maps into the existing typed `Error` enum at the crate boundary. Rationales #1 and #2 (decode fan-out, ring backpressure) are retired by measurement: the cross-thread handoff cost more than the head-of-line blocking it avoided in every cell, including the fan-in shape below.

TLS rides a custom connector that reuses the existing single-provider rustls configuration (`ring`, webpki roots, `h2` ALPN) verbatim — the stack's own TLS features stay disabled, and `cargo tree --invert aws-lc-rs` stays empty across all five workspaces. The previously dormant `MddsConfig` HTTP/2 knobs (`window_size_kb`, `connection_window_size_kb`, `keepalive_secs`, `keepalive_timeout_secs`) are load-bearing again, threaded into the channel builder at connect time.

### Known caveat: status trailer parsing panics upstream (contained)

The reference stack's status parser `.expect()`s the base64 decode of `grpc-status-details-bin`, so a malformed trailer from the wire would panic whichever task polls the response. Two boundaries in `src/grpc/` can observe such a trailer: the open-phase await in `channel.rs` (a trailers-only response parses its status from the response head) and `ServerStreaming::poll_next` in `stream.rs` (end-of-stream trailers after data frames). Both poll the underlying future/stream inside `std::panic::catch_unwind`; a caught panic fuses the stream and surfaces as a terminal `ChannelError::Rpc` carrying the canonical `Internal` code and a message naming the undecodable trailer — the same protocol-shape-violation convention the decode layer's locally-synthesized statuses follow. The connection task is untouched by the unwind (the parse runs in the caller's poll), so the channel keeps serving subsequent RPCs. `tests/grpc_mock_server.rs` pins both shapes, including a clean follow-up RPC on the same connection after the contained panic.

### Open question 1: multi-chunk fan-in (closed)

The 2026-06-11 comparison left one shape unmeasured: many response chunks fanning into one decode path per request — the decoder pool's home turf. Measured with the harness's `multi` payload (16 zstd chunks of ~640 KiB per RPC, ~10 MiB total, same per-RPC bytes as `large`), same box and protocol as the recorded comparison:

| concurrency | transport | p50 | p99 | mean | req/s | wire MB/s | alloc/req | cpu/req |
|---|---|---|---|---|---|---|---|---|
| 8 | in-house (decoder pool) | 105.47 ms | 122.30 ms | 106.21 ms | 64 (63..64) | 666.1 | 100.8 MiB | 128 299 us |
| 8 | reference (inline decode) | 100.25 ms | 112.03 ms | 100.20 ms | 66 (65..66) | 689.7 | 90.8 MiB | 122 248 us |
| 16 | in-house (decoder pool) | 172.41 ms | 198.46 ms | 172.76 ms | 77 (77..77) | 807.7 | 100.8 MiB | 148 420 us |
| 16 | reference (inline decode) | 152.34 ms | 186.42 ms | 153.55 ms | 81 (81..82) | 852.6 | 90.8 MiB | 169 387 us |

Inline decode wins the fan-in shape too: +3.5% throughput / -5.0% p50 at concurrency 8, +5.6% throughput / -11.6% p50 at the 16-concurrent ceiling, with 10% less allocation per request. The pre-registered fallback (bounded `spawn_blocking` decode if inline regressed more than 10%) is not needed; the decoder pool is deleted without a replacement.

### Open question 2: N-channel pool vs one multiplexed channel (pool wins)

The reference stack multiplexes streams over one connection, so the migration re-evaluated whether the N-channel pool still earns its keep. Measured on the reference arm with `THETADATADX_BENCH_CONNS=1` (every worker multiplexed onto one connection) against the production shape (one connection per worker, both sides at the 64 KiB spec windows):

| shape | concurrency | conns | p50 | req/s | wire MB/s |
|---|---|---|---|---|---|
| small (1 KB) | 8 | 1 | 236.1 us | 32 978 | 33.4 |
| small (1 KB) | 8 | 8 | 109.1 us | 68 727 | 69.7 |
| small (1 KB) | 16 | 1 | 366.2 us | 42 926 | 43.5 |
| small (1 KB) | 16 | 16 | 163.1 us | 79 027 | 80.1 |
| large (10 MiB) | 8 | 1 | 185.31 ms | 39 | 411.6 |
| large (10 MiB) | 8 | 8 | 143.13 ms | 45 | 469.0 |
| large (10 MiB) | 16 | 1 | 411.75 ms | 37 | 384.7 |
| large (10 MiB) | 16 | 16 | 156.09 ms | 83 | 872.3 |

One multiplexed connection costs 1.8-2.3x of the throughput at the 16-concurrent ceiling (and 2.6x the large-frame p50): every stream shares a single connection-level flow-control window and one TCP pipe, exactly the contention the per-worker connection fan-out removes. The `ChannelPool` (one HTTP/2 connection per concurrent request, least-loaded picks, lease-based in-flight accounting) survives the migration unchanged.

### Post-migration baseline

The harness now drives the production transport surface (`Channel` / `ChannelPool` + the production dispatch + merge shape) — it is the regression pin going forward. Same box, toolchain rustc 1.96.0, same protocol as the recorded comparison; production-reachable cells (1-16) plus the synthetic headroom appendix:

**small (1 014 B wire, 1 chunk, decode ceiling 4 MiB):**

| concurrency | p50 | p99 | p99.9 | mean | req/s (min..max) | wire MB/s | alloc/req | cpu/req |
|---|---|---|---|---|---|---|---|---|
| 1 | 83.8 us | 123.5 us | 157.4 us | 86.7 us | 11 474 (11 401..11 512) | 11.6 | 32.8 KiB | 119 us |
| 2 | 89.0 us | 120.8 us | 147.5 us | 90.5 us | 21 947 (21 908..21 974) | 22.3 | 32.7 KiB | 127 us |
| 4 | 94.8 us | 123.0 us | 168.9 us | 95.4 us | 41 592 (41 520..41 699) | 42.2 | 32.7 KiB | 133 us |
| 8 | 108.4 us | 196.1 us | 366.9 us | 111.0 us | 71 445 (71 245..71 687) | 72.4 | 32.7 KiB | 140 us |
| **16** | **140.0 us** | **382.8 us** | **728.3 us** | **151.3 us** | **104 952 (104 701..105 448)** | **106.4** | **32.8 KiB** | **128 us** |
| 100 (synthetic) | 700.0 us | 1.44 ms | 1.92 ms | 721.8 us | 138 260 (136 512..140 933) | 140.2 | 33.8 KiB | 94 us |
| 1000 (synthetic) | 4.92 ms | 10.18 ms | 12.45 ms | 5.09 ms | 196 126 (195 783..196 751) | 198.9 | 34.0 KiB | 68 us |

**large (10.0 MiB wire, 1 chunk, decode ceiling 64 MiB):**

| concurrency | p50 | p99 | p99.9 | mean | req/s (min..max) | wire MB/s | alloc/req | cpu/req |
|---|---|---|---|---|---|---|---|---|
| 1 | 54.11 ms | 59.63 ms | 74.09 ms | 54.22 ms | 17 (17..17) | 179.3 | 94.6 MiB | 58 504 us |
| 2 | 60.70 ms | 80.09 ms | 83.89 ms | 61.26 ms | 30 (30..30) | 314.6 | 94.6 MiB | 67 775 us |
| 4 | 65.48 ms | 88.32 ms | 92.98 ms | 67.05 ms | 53 (53..54) | 560.1 | 94.6 MiB | 76 855 us |
| 8 | 85.26 ms | 115.95 ms | 126.23 ms | 88.61 ms | 79 (79..79) | 829.4 | 94.5 MiB | 104 791 us |
| **16** | **125.13 ms** | **167.64 ms** | **192.72 ms** | **127.32 ms** | **110 (110..110)** | **1 153.9** | **94.6 MiB** | **138 174 us** |

**multi (10.0 MiB wire across 16 chunks, decode ceiling 64 MiB):**

| concurrency | p50 | p99 | p99.9 | mean | req/s (min..max) | wire MB/s | alloc/req | cpu/req |
|---|---|---|---|---|---|---|---|---|
| 1 | 53.50 ms | 57.80 ms | 78.64 ms | 53.65 ms | 17 (17..17) | 181.1 | 90.9 MiB | 57 928 us |
| 2 | 58.36 ms | 75.87 ms | 82.32 ms | 59.63 ms | 30 (29..30) | 313.5 | 90.8 MiB | 67 013 us |
| 4 | 69.96 ms | 85.24 ms | 90.87 ms | 70.69 ms | 48 (48..48) | 506.9 | 90.8 MiB | 82 984 us |
| 8 | 90.60 ms | 103.49 ms | 108.66 ms | 90.73 ms | 73 (73..73) | 764.6 | 90.8 MiB | 110 928 us |
| **16** | **124.36 ms** | **144.22 ms** | **154.02 ms** | **125.21 ms** | **102 (102..102)** | **1 067.8** | **90.8 MiB** | **146 204 us** |

Reading against the recorded in-house baselines above: the migrated transport is at parity or ahead in every cell. At the 16-concurrent ceiling: small frames +17.1% throughput / -18.3% p50 (104 952 vs 89 592 req/s; 140.0 vs 171.4 us), large frames +39% throughput / -25.8% p50 (1 153.9 vs 827.7 MB/s; 125.13 vs 168.59 ms), allocation 2.3x lower per small request and 1.34x lower per large request. The small-frame cells reproduce the recorded reference-arm numbers within ~0.5% — the wrapper (pool accounting, error mapping, per-chunk merge) adds no measurable cost. The large-frame cells land 25-35% above the recorded reference-arm numbers; large-allocation cells on this shared box swing run to run (the recorded comparison already noted a 58.3-67.7 ms p50 band for in-house large/c1 across invocations), and the production pool's least-loaded picks replace the recorded run's static worker-to-connection pinning, so treat the large-frame rows as this run's pin rather than a like-for-like delta against the reference arm. The verdict cell (at-or-ahead of the in-house transport everywhere) holds in every run observed.

### Decode stage: typed tick build + bulk column extraction (2026-06-11)

Two changes after the baseline above was recorded, measured together on the same box and protocol.

**The harness now carries the full decode shape.** The measured loop runs the typed `EodTick` build on the merged table inside every request — the same parse call the generated `stock_history_eod` endpoint performs — so the per-frame tick decode is part of every cell going forward. The synthetic payload is valid tick input for it (`ms_of_day` inside the `i32` milliseconds-of-day window, real `YYYYMMDD` dates, both randomized so neither column compresses away), which moves the 10.0 MiB large cell from 130 926 to 148 052 rows. The baseline tables above predate both changes and are not directly comparable to the rows below.

**Bulk column extraction in the generated parsers.** The tick parsers switched from per-cell type dispatch (row-shaped loop, per-cell accept-set selection, `collect` into an unsized `Vec`) to schema-validated bulk column extraction (`decode/column.rs`): column layout resolved once per table, then per-column monomorphic extraction in 256-row blocks into an exact-size seeded output. Large cell (10.0 MiB wire, 148 052 rows), before → after on the full-decode harness:

| cell | metric | per-cell decode | bulk extraction | delta |
|---|---|---|---|---|
| large c=1 | p50 (median of runs) | 75.99 ms (75.40..76.91, 4 runs) | 70.76 ms (69.03..70.94, 3 runs) | -6.9% |
| large c=1 | cpu/req (mean) | 78 080 us | 72 361 us | -7.3% |
| large c=1 | alloc/req | 169.4 MiB | 123.5 MiB | -27.1% |
| large c=16 (interleaved A/B, 3 pairs) | p50 (median of pairs) | 229.64 ms (225.09..231.92) | 222.48 ms (208.11..223.14) | -3.1% |
| large c=16 (interleaved A/B, 3 pairs) | req/s (mean) | 66.7 | 70.3 | +5.5% |
| large c=16 (interleaved A/B, 3 pairs) | cpu/req (mean) | 212 298 us | 199 674 us | -5.9% |

The c=16 cell swings run to run on this shared box (both arms degraded together in one interleaved pair), so the c=16 rows come from back-to-back A/B runs of the two pinned binaries rather than separate sessions. The allocation drop is the exact-size output: collecting `Result` rows defeats the iterator's size hint, so the old parser grew the 28 MiB tick `Vec` geometrically; the bulk path allocates it once. Isolated to the parse stage alone (criterion, 100-row tables), trade ticks decode 3.4x faster, NBBO quotes 2.9x, OHLC bars 1.8x; on the memory-bound 130K-row EOD shape the parse stage alone improves ~8%, and whole-table column passes without the row blocking measured 2.1x slower than the row-shaped decode — the cache rationale for the 256-row blocks (see `decode/column.rs`).
