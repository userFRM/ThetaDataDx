---
title: Configuration
description: Environments, retries, timeouts, and the streaming knobs.
---

# Configuration

The configuration object (`DirectConfig` in Rust, `Config` elsewhere) ships sensible defaults; override individual fields when you need to.

## Environments

The SDK has two independent clients, and each has its own environment:

- The historical (MDDS) client runs in **production** or **staging**. The historical environment also sets the authentication marker, so staging authenticates against the staging cluster.
- The streaming (FPSS) client runs in **production** or **dev**. The dev environment replays a past trading day in a loop at full speed, so you can develop while markets are closed.

The two are chosen independently. There is no streaming staging cluster and no historical dev cluster, so a config can be historical-staging with streaming-production, historical-production with streaming-dev, and so on.

| Preset | Historical (MDDS) | Streaming (FPSS) |
|---|---|---|
| `production()` (default) | production | production |
| `stage()` | staging | production |
| `dev()` | production | dev |

`stage()` selects historical staging and leaves streaming on production; `dev()` selects streaming dev and leaves historical on production. To move both at once, select each channel:

```rust
use thetadatadx::config::{DirectConfig, HistoricalEnvironment, StreamingEnvironment};

let cfg = DirectConfig::production()
    .with_historical_environment(HistoricalEnvironment::Stage)
    .with_streaming_environment(StreamingEnvironment::Dev);
```

```python
from thetadatadx import Config

cfg = Config.production()
cfg.retry_max_attempts = 5
cfg.flush_mode = "immediate"
client = Client(creds, cfg)
```

### Selecting an environment

Each channel has its own selector, and you can pick whichever fits how you configure the rest of your deployment. All of them work the same whether you authenticate with an api-key or with email and password. The environment is independent of the credential.

1. Use a preset or the typed setters, in code. The presets are on every binding: `production()` / `stage()` / `dev()` (for example `Config.stage()` in Python and TypeScript, `thetadatadx::Config::stage()` in C++). For an explicit per-channel choice, use `DirectConfig::with_historical_environment(HistoricalEnvironment::Stage)` and `DirectConfig::with_streaming_environment(StreamingEnvironment::Dev)`.

2. Set environment variables. `THETADATA_MDDS_TYPE` selects the historical environment (`PROD` or `STAGE`); `THETADATA_FPSS_TYPE` selects the streaming environment (`PROD` or `DEV`). Both are case-insensitive, and an unset value keeps production. This steers an existing deployment without a code change, and it works with every binding because each one reads them when it builds the config from a preset:

```bash
export THETADATA_MDDS_TYPE=STAGE
export THETADATA_FPSS_TYPE=DEV
```

```python
from thetadatadx import Config

cfg = Config.production()  # reads THETADATA_MDDS_TYPE / THETADATA_FPSS_TYPE
client = Client(creds, cfg)
```

3. Put them in a `.env` file. `Config.from_dotenv(path)` reads `THETADATA_MDDS_TYPE` and `THETADATA_FPSS_TYPE` from a `.env`-format file and selects the matching environment on each channel:

```python
from thetadatadx import Config

cfg = Config.from_dotenv(".env")
client = Client(creds, cfg)
```

The same reader is on every binding: `DirectConfig::from_dotenv(path)` in Rust, `Config.fromDotenv(path)` in TypeScript, and `thetadatadx::Config::from_dotenv(path)` in C++. It reads the same `.env` file and the same keys that `Credentials.from_dotenv(path)` reads for the credential, so a single `.env` file can hold both the api key and the environment selectors:

```ini
THETADATA_API_KEY=your_api_key_here
THETADATA_MDDS_TYPE=STAGE
THETADATA_FPSS_TYPE=DEV
```

Load the credential with `Credentials.from_dotenv` and the environment with `Config.from_dotenv`, both pointed at that one file.

A value outside a selector's set is rejected rather than silently ignored: `THETADATA_MDDS_TYPE` must be `PROD` or `STAGE`, and `THETADATA_FPSS_TYPE` must be `PROD` or `DEV`.

You can also select environments inline at the client, without building a `Config` first. The fluent builder takes them alongside the credential: `Client::builder().api_key("...").stage().dev().connect()` in Rust and C++ (each shorthand selects its channel, and they compose), `Client(api_key="...", mdds_type="STAGE", fpss_type="DEV")` in Python, and `Client.connectWith({ apiKey: '...', mddsType: 'STAGE', fpssType: 'DEV' })` in TypeScript. The `Config` path above stays available when you need full control over the hosts and tuning knobs; the builder is a convenience over it.

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
