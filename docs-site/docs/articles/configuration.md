---
title: Configuration
description: Environments, retries, timeouts, concurrency, and the streaming knobs.
---

# Configuration

The configuration object (`DirectConfig` in Rust, `Config` elsewhere) ships sensible defaults; override individual fields when you need to.

## Environments

| Preset | Use |
|---|---|
| `production()` | Live market data. |
| `dev()` | Streaming servers replay a past trading day in a loop at full speed — develop while markets are closed. Historical requests still hit production. |
| `stage()` | Vendor staging servers; expect reboots. |

```python
from thetadatadx import Config

cfg = Config.production()
cfg.retry_max_attempts = 5
cfg.flush_mode = "immediate"
tdx = ThetaDataDxClient(creds, cfg)
```

In Rust the same fields live on `DirectConfig` struct sub-configs (`config.retry.max_attempts`, `config.fpss.flush_mode`); TypeScript uses `Config` setters (`cfg.setRetryMaxAttempts(5)`); C++ uses `tdx::Config::set_retry_max_attempts(5)`.

## The knobs that matter

| Group | Fields | What they control |
|---|---|---|
| Request deadlines | `timeout_ms` per request (builder / kwarg) | Hard per-call deadline; expiry raises a timeout error and frees the slot. |
| Retries | `retry_initial_delay_ms`, `retry_max_delay_ms`, `retry_max_attempts`, `retry_jitter`, `retry_max_elapsed_secs` | Backoff schedule for transient historical-request faults. |
| Concurrency | `concurrent_requests` | Parallel historical requests; auto-set from your tier. See [Concurrent Requests](/articles/concurrent-requests). |
| Streaming reconnect | `reconnect_policy`, `reconnect_max_attempts`, `reconnect_wait_ms`, `reconnect_wait_max_ms`, `reconnect_jitter`, `reconnect_stable_window_secs`, … | Automatic streaming reconnection. See [Reconnection & Monitoring](/streaming/reliability). |
| Streaming latency | `flush_mode` (`"batched"` default / `"immediate"`), `fpss_ring_size`, `fpss_timeout_ms`, keepalive fields | Write-path flush behavior and event-buffer capacity. |
| Flat files | `flatfiles_max_attempts`, `flatfiles_initial_backoff_secs`, `flatfiles_max_backoff_secs`, `flatfiles_jitter` | Retry budget for bulk downloads. |
| Observability | `metrics_port` | Optional local Prometheus exporter port (off by default). |
| Runtime | `worker_threads` | Async worker-thread count for embedded bindings (0 = auto). |

Every field above is available on all four language surfaces under the naming convention shown earlier; unknown values fail at configuration time, not at first request.

## Config file (Rust)

With the `config-file` feature, Rust loads the same fields from TOML — useful for operating the [server binary](/server/) or any deployment where configuration should live outside code:

```toml
[retry]
max_attempts = 5

[fpss]
flush_mode = "immediate"
hosts = ["host-a.example.com:20000", "host-b.example.com:20000"]
```

Streaming host lists are configurable only at this layer (or via `DirectConfig` in Rust); the other bindings inherit them from the loaded config.
