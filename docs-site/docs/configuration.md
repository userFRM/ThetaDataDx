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

// Dev (same servers, shorter timeouts for faster iteration)
let config = DirectConfig::dev();
```
```python [Python]
from thetadatadx import Config

# Production (ThetaData NJ datacenter, gRPC over TLS)
config = Config.production()

# Dev (same servers, shorter timeouts for faster iteration)
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
    pub fpss_timeout_ms: u64,               // read timeout
    pub fpss_queue_depth: usize,            // event ring buffer size
    pub fpss_ping_interval_ms: u64,         // heartbeat interval (100ms)
    pub fpss_connect_timeout_ms: u64,       // TCP connect timeout

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
| Explicit value | Fixed at `n` concurrent requests |

::: tip
Higher concurrency lets you fetch more data in parallel, but exceeding your tier's limit will trigger rate-limiting (error code 12) with a 130-second backoff.
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
