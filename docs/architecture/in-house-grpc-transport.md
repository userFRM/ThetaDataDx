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
