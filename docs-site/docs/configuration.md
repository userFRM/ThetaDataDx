---
title: Configuration
description: Configure ThetaDataDx connection settings, timeouts, concurrency, and server targets for MDDS and FPSS.
---

# Configuration

ThetaDataDx provides sensible defaults for production use, with full control over every parameter when you need it.

## Presets

Two built-in presets cover the common cases:

::: code-group
```rust [Rust]
use thetadatadx::DirectConfig;

// Production (ThetaData NJ datacenter, gRPC over TLS)
let config = DirectConfig::production();

// Dev (Dev FPSS servers (port 20200) -- infinite replay of historical day)
let config = DirectConfig::dev();
```
```python [Python]
from thetadatadx import Config

# Production (ThetaData NJ datacenter, gRPC over TLS)
config = Config.production()

# Dev (Dev FPSS servers (port 20200) -- infinite replay of historical day)
config = Config.dev()
```
:::

## Custom Configuration

Override specific fields while keeping production defaults:

::: code-group
```rust [Rust]
let config = DirectConfig {
    fpss_timeout_ms: 5_000,
    reconnect_wait_ms: 2_000,
    ..DirectConfig::production()
};
```
```python [Python]
config = Config.production()
config.fpss_timeout_ms = 5_000
config.reconnect_wait_ms = 2_000
```
:::

### Override gRPC Concurrency

By default, the number of concurrent gRPC requests is auto-detected from your subscription tier (`2^tier`). You can override this:

::: code-group
```rust [Rust]
let config = DirectConfig {
    mdds_concurrent_requests: 8,        // manual override
    ..DirectConfig::production()        // 0 = auto from tier
};
```
```python [Python]
config = Config.production()
config.mdds_concurrent_requests = 8  # manual override; 0 = auto from tier
```
:::

## Configuration Fields

```rust
pub struct DirectConfig {
    // MDDS (Historical gRPC)
    pub mdds_host: String,                  // "mdds-01.thetadata.us"
    pub mdds_port: u16,                     // 443
    pub mdds_tls: bool,                     // true
    pub mdds_max_message_size: usize,       // max gRPC message size
    pub mdds_keepalive_secs: u64,           // gRPC keepalive interval
    pub mdds_keepalive_timeout_secs: u64,   // gRPC keepalive timeout

    // FPSS (Real-Time TCP)
    pub fpss_hosts: Vec<(String, u16)>,     // server failover list
    pub fpss_timeout_ms: u64,               // read timeout (ms)
    pub fpss_ring_size: usize,              // Disruptor ring buffer size (events; power of two, >= 64)
    pub fpss_ping_interval_ms: u64,         // heartbeat interval (default 100ms)
    pub fpss_connect_timeout_ms: u64,       // TCP connect timeout (ms)

    // Reconnection
    pub reconnect_wait_ms: u64,             // base reconnect delay (2000ms)
    pub reconnect_wait_rate_limited_ms: u64,// rate-limit delay (130000ms)

    // Concurrency
    pub mdds_concurrent_requests: usize,           // 0 = auto (2^tier)

    // Threading
    pub tokio_worker_threads: usize,       // Tokio runtime thread count (0 = auto)
}
```

## FPSS Server List

The default FPSS server list matches the Java terminal:

| Server | Port |
|--------|------|
| `nj-a.thetadata.us` | 20000 |
| `nj-a.thetadata.us` | 20001 |
| `nj-b.thetadata.us` | 20000 |
| `nj-b.thetadata.us` | 20001 |

Servers are tried in order during connection. If the first server fails, the client automatically falls over to the next.

## Concurrency Model

The `mdds_concurrent_requests` field controls the maximum number of in-flight gRPC requests. Each endpoint method acquires a semaphore permit before sending and releases it when the response is fully consumed.

| Setting | Behavior |
|---------|----------|
| `0` / not set (default) | Auto-detected from subscription tier: `2^tier` |
| Explicit value | Fixed at `n`, **clamped to the tier cap** at connect time |

### Subscription-tier cap

ThetaData enforces a hard server-side cap on concurrent in-flight gRPC requests per tier. The SDK clamps explicit configured values to this cap at connect time and emits a single `tracing::warn!` so the local boundary surfaces the misconfiguration before any RPC fires:

| Tier | Server cap on `concurrent_requests` |
|---|---:|
| FREE | 1 |
| VALUE | 2 |
| STANDARD | 4 |
| PRO | 8 |

Setting `mdds.concurrent_requests = 32` on a PRO tier opens 8 channels (not 32) and logs:

```
mdds.concurrent_requests exceeds subscription tier cap — clamping to tier cap
  configured = 32, tier_cap = 8
```

The previous behaviour honoured the configured value unconditionally, which produced confusing `ResourceExhausted` rejections on the (cap + 1)-th channel that the SDK then retried on a different channel — making bulk-pull failures look like "everything fails intermittently". The local clamp surfaces the misconfiguration immediately. (Bypass requires `MddsConfig::override_tier_clamp = true`, intended for tests only.)

## Throughput Tuning (issue #584)

For large historical pulls (multi-day backfills, wide `strike_range`, `interval = 1s` / `tick`), three knobs on `MddsConfig` control the SDK-side throughput. Each is independently tunable:

| Workload | `concurrent_requests` | `decoder_threads` | `decoder_ring_size` |
|---|---|---|---|
| One-shot single-day single-strike query | `1` | auto | default (256) |
| Multi-day backfill, narrow strike scope (sr < 10) | `4` (PRO) | auto | default (256) |
| Wide `strike_range` or `1s` / `tick` interval bulk | `8` (PRO max) | `8` | default (256) |
| Reference: server-side tier caps | FREE=1 / VALUE=2 / STANDARD=4 / PRO=8 | | |

### `decoder_threads`

Each decoder thread runs zstd decompress + protobuf decode on a dedicated `std::thread`, keeping CPU-bound work off the tokio reactor. The default (`0`) auto-sizes to `max(available_parallelism / 2, 1)`, leaving half the logical cores for the reactor and the application's own work.

**Override** on shared hosts where the auto-sizing reads the wrong number from `/proc`, or to widen the decode pipeline on historical backfills with wide `strike_range`.

::: tip Architectural caveat
Today's MDDS pool pins each gRPC channel to one decoder ring via round-robin distribution. **Decoder threads beyond `concurrent_requests` therefore sit idle** — no producer feeds them. The two-stage pipeline rewrite (separate PR) decouples the IO and decode sides so extra decoder threads become useful; until then, setting `decoder_threads > concurrent_requests` is wasted memory (each idle thread keeps a 256-slot ring allocated).
:::

### `decoder_ring_size`

Per-thread Disruptor ring depth, the buffer between the h2 receive task and each decoder thread. Must be a power of two, `>= 64`. Default `256` is enough headroom for a 64-way burst across 4 channels.

Larger rings absorb burstier IO without back-pressuring `try_publish`; smaller rings reduce memory footprint. Benchmark results (`bench_decoder_pool/ring`) show **ring depth is not the bottleneck at the default workload** — 64/256/1024/4096 land within ~5% of each other at 1024-row quote payloads. Tune this only if profiling shows producer-side `try_publish` retries in your specific workload.

### Concrete example — PRO tier, 16-core box, wide strike range backfill

```rust
use thetadatadx::DirectConfig;

let mut config = DirectConfig::production();
config.mdds.concurrent_requests = 8;        // hit PRO tier cap
config.mdds.decoder_threads = 8;            // match channel count exactly
config.mdds.decoder_ring_size = 256;        // default — bench-confirmed adequate
```

```python
import thetadatadx as m

cfg = m.Config.production()
cfg.concurrent_requests = 8
cfg.decoder_threads = 8
cfg.decoder_ring_size = 256
client = m.ThetaDataDxClient(creds, cfg)
```

```typescript
import { Config, ThetaDataDxClient } from 'thetadatadx';

const cfg = Config.production();
cfg.setConcurrentRequests(8);
cfg.setDecoderThreads(8);
cfg.setDecoderRingSize(256);
const client = await ThetaDataDxClient.connectWithConfig(email, password, cfg);
```

```cpp
#include "thetadx.hpp"

auto cfg = tdx::Config::production();
cfg.set_concurrent_requests(8);
cfg.set_decoder_threads(8);
cfg.set_decoder_ring_size(256);
auto creds = tdx::Credentials::from_email(email, password);
auto client = tdx::Client::connect(creds, cfg);
```

::: warning Rate limiting
Even with the SDK-side clamp, hitting `concurrent_requests = 8` on a PRO tier means every RPC dispatch holds a permit. Bursty traffic that exceeds tier capacity surfaces as gRPC `ResourceExhausted` (code 8) with a 130-second backoff. The clamp is the SDK's friendly boundary, not server-side throttling.
:::

## Timeouts

| Timeout | Default | Description |
|---------|---------|-------------|
| FPSS connect | 2000ms | TCP connection to FPSS servers |
| FPSS read | 10000ms | Frame read timeout |
| FPSS ping | 100ms | Heartbeat interval (required by server) |
| Reconnect (normal) | 2000ms | Delay before reconnecting after disconnect |
| Reconnect (rate-limited) | 130000ms | Delay after TooManyRequests (code 12) |
| Nexus HTTP connect | 5000ms | HTTP connection to auth server |
| Nexus HTTP request | 10000ms | Total auth request timeout |

## Environment Variables

For development and testing, you can override the server target:

```bash
export THETADX_HOST="127.0.0.1"
export THETADX_PORT="11000"
```

::: warning
Environment variable overrides are intended for local development only. In production, use the `DirectConfig` / `Config` presets which point to the correct ThetaData datacenter endpoints.
:::

## Performance Tuning

### mimalloc

The default system allocator (glibc malloc on Linux, jemalloc on macOS) handles the gRPC decode path correctly but is not optimised for the allocation pattern MDDS responses exhibit: many small fixed-size allocations during prost decode, interleaved with one large zstd output buffer per call. [`mimalloc`](https://crates.io/crates/mimalloc) is a drop-in replacement that reduces fragmentation and page-fault traffic on this shape, particularly when many gRPC calls fan out across many threads.

Opt in by enabling the `mimalloc-allocator` feature on the SDK and registering the allocator in your binary's entry point. Library crates cannot install a `#[global_allocator]` of their own — that lives in the binary — so the SDK only provides the re-export; the binary owns the choice.

In your binary's `Cargo.toml`:

```toml
[dependencies]
thetadatadx = { version = "10", features = ["mimalloc-allocator"] }
```

In your binary's `main.rs`:

```rust,ignore
#[global_allocator]
static GLOBAL: thetadatadx::mimalloc::MiMalloc = thetadatadx::mimalloc::MiMalloc;

fn main() {
    // ... your application ...
}
```

::: tip
The gain scales with response size and concurrency. Single-threaded clients hitting only a handful of endpoints at a time will see little benefit; multi-threaded fan-out across a `ChannelPool` consistently shows shorter p99 tails on tabular MDDS responses past ~1 KB.
:::

::: warning
Do not register a `#[global_allocator]` inside a library crate. Rust permits exactly one per binary, and a library that registers its own will conflict with any binary that registers a different one (or with any other library doing the same). Keep the registration in the executable's `main.rs`.
:::
