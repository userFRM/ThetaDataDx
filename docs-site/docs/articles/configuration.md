---
title: Configuration
description: Environments, retries, timeouts, and the streaming knobs.
---

# Configuration

The configuration object (`DirectConfig` in Rust, `Config` elsewhere) ships sensible defaults; override individual fields when you need to.

## Environments

| Preset | Use |
|---|---|
| `production()` | Live market data. The default. |
| `dev()` | Streaming servers replay a past trading day in a loop at full speed — develop while markets are closed. Historical requests still hit production. |
| `stage()` | Staging environment — points authentication, historical, and streaming all at the staging cluster. Used to validate against pre-release server changes; less stable than production and subject to reboots. |

```python
from thetadatadx import Config

cfg = Config.production()
cfg.retry_max_attempts = 5
cfg.flush_mode = "immediate"
client = Client(creds, cfg)
```

### Selecting the staging environment

There are three ways to point the SDK at the staging cluster, and you can pick whichever fits how you configure the rest of your deployment. All three select the staging environment across every channel (authentication, historical, and streaming) in one step, and all three work the same whether you authenticate with an api-key or with email and password. The environment is independent of the credential.

1. Pass it directly, in code. The typed selector is the most explicit option:

```rust
use thetadatadx::config::{DirectConfig, Environment};

let cfg = DirectConfig::production().with_environment(Environment::Stage);
```

`DirectConfig::production().with_environment(Environment::Stage)` is equivalent to the `stage()` preset; passing `Environment::Prod` restores production. The `stage()` preset is the same selection in one call, available on every binding: `DirectConfig::stage()` in Rust, `Config.stage()` in Python and TypeScript, and `thetadatadx::Config::stage()` in C++.

2. Set the `THETADATA_MDDS_TYPE` environment variable. Set it to `STAGE` to select the staging environment, or `PROD` (the default, also used when the variable is unset) for production. The value is case-insensitive. This steers an existing deployment at staging without a code change, and it works with every binding because each one reads it when it builds the config from a preset:

```bash
export THETADATA_MDDS_TYPE=STAGE
```

```python
from thetadatadx import Config

cfg = Config.production()  # reads THETADATA_MDDS_TYPE; STAGE selects staging
client = Client(creds, cfg)
```

3. Put it in a `.env` file. `Config.from_dotenv(path)` reads `THETADATA_MDDS_TYPE` (`STAGE` / `PROD`) from a `.env`-format file and selects the matching environment:

```python
from thetadatadx import Config

cfg = Config.from_dotenv(".env")  # THETADATA_MDDS_TYPE=STAGE selects staging
client = Client(creds, cfg)
```

The same reader is on every binding: `DirectConfig::from_dotenv(path)` in Rust, `Config.fromDotenv(path)` in TypeScript, and `thetadatadx::Config::from_dotenv(path)` in C++. It reads the same `.env` file and the same keys that `Credentials.from_dotenv(path)` reads for the credential, so a single `.env` file can hold both the api key and the environment selector:

```ini
THETADATA_API_KEY=your_api_key_here
THETADATA_MDDS_TYPE=STAGE
```

Load the credential with `Credentials.from_dotenv` and the environment with `Config.from_dotenv`, both pointed at that one file.

If you also set an explicit streaming or historical host (through `THETADATA_HISTORICAL_HOST` / `THETADATA_STREAMING_HOST`, in the environment, in the `.env` file, or in the config file), that explicit host wins over the environment's default for that channel.

In Rust the same fields live on `DirectConfig` struct sub-configs (`config.retry.max_attempts`, `config.streaming.flush_mode`); TypeScript uses `Config` setters (`cfg.setRetryMaxAttempts(5)`); C++ uses `thetadatadx::Config::set_retry_max_attempts(5)`.

## The knobs that matter

| Group | Fields | What they control |
|---|---|---|
| Request deadlines | `timeout_ms` per request (builder / kwarg) | Hard per-call deadline; expiry raises a timeout error and frees the slot. |
| Retries | `retry_initial_delay_ms`, `retry_max_delay_ms`, `retry_max_attempts`, `retry_jitter`, `retry_max_elapsed_secs` | Backoff schedule for transient historical-request faults. |
| Streaming reconnect | `reconnect_policy`, `reconnect_max_attempts`, `reconnect_wait_ms`, `reconnect_wait_max_ms`, `reconnect_jitter`, `reconnect_stable_window_secs`, … | Automatic streaming reconnection. See [Reconnection & Monitoring](/streaming/reliability). |
| Streaming latency | `flush_mode` (`"batched"` default / `"immediate"`), `streaming_ring_size`, `streaming_timeout_ms`, keepalive fields | Write-path flush behavior and event-buffer capacity. |
| Flat files | `flatfiles_max_attempts`, `flatfiles_initial_backoff_secs`, `flatfiles_max_backoff_secs`, `flatfiles_jitter` | Retry budget for bulk downloads. |
| Observability | `metrics_port` | Optional local Prometheus exporter port (off by default). |
| Runtime | `worker_threads` | Async worker-thread count for embedded bindings (0 = auto). |

Every field above is available on all four language surfaces under the naming convention shown earlier; unknown values fail at configuration time, not at first request.

Historical request concurrency is not in this table because it isn't configurable. The SDK sizes its historical connection pool automatically from your subscription tier at connect time. See [Concurrent Requests](/articles/concurrent-requests).

## Config file (Rust)

With the `config-file` feature, Rust loads the same fields from TOML — useful for operating the [server binary](/server/) or any deployment where configuration should live outside code:

```toml
[historical]
host = "mdds-01.thetadata.us"
port = 443

[streaming]
flush_mode = "immediate"
hosts = ["host-a.example.com:20000", "host-b.example.com:20000"]
```

Streaming host lists are configurable only at this layer (or via `DirectConfig` in Rust); the other bindings inherit them from the loaded config.
