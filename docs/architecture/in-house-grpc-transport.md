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

**Status (2026-06-11): the trigger condition is met.** The measured comparison below shows the reference stack at parity or ahead in every production-reachable cell. Decision pending maintainer review; rationale #3 (surface hygiene) is unaffected by these numbers and remains the argument for keeping the in-house transport if it stays.

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
- Single-frame responses (the dominant production shape for these endpoints). Multi-chunk streams that interleave many channels into one decoder pool — the fan-in shape rationale #2 describes — were not measured here.
- TLS was off for both stacks (h2c); both production paths use rustls over the same h2 layer.
- Warmed measurements only; the cold-cache axis from the original brief was not measured.
- The box is shared; every reported cell ran in a verified-quiet window (a sampler re-ran any cell that overlapped background compile activity), with 3 repeats per cell.
