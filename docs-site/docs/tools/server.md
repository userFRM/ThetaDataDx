---
title: REST Server
description: Drop-in replacement for the ThetaData Java Terminal v3 surface. Runs a local HTTP REST server and WebSocket server backed by ThetaDataDx.
---

# REST Server

Drop-in replacement for the ThetaData Java Terminal v3 surface. Runs a local HTTP REST server and WebSocket server backed by ThetaDataDx. Existing scripts that target the terminal's current v3 routes can point at this binary instead of the JVM terminal.

## Installation

```bash
cargo install thetadatadx-server --git https://github.com/userFRM/ThetaDataDx
```

Or build from source:

```bash
cargo install --path tools/server
```

## Quick Start

```bash
thetadatadx-server --creds creds.txt
```

This starts:
- **HTTP REST API** on `http://127.0.0.1:25503` (all `/v3/...` routes)
- **WebSocket server** on `ws://127.0.0.1:25520/v1/events`

## CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--creds <path>` | `creds.txt` | Credentials file |
| `--email <email>` | | ThetaData email (alternative to `--creds`) |
| `--password <password>` | | ThetaData password (alternative to `--creds`) |
| `--config <path>` | | TOML config file |
| `--fpss-region <region>` | `production` | FPSS region: `production`, `dev`, `stage` |
| `--http-port <port>` | `25503` | HTTP REST API port |
| `--ws-port <port>` | `25520` | WebSocket server port |
| `--bind <addr>` | `127.0.0.1` | Bind address |
| `--log-level <filter>` | `info` | Tracing filter |
| `--no-fpss` | | Skip FPSS streaming startup |
| `--no-ohlcvc` | | Disable derived OHLCVC bars |

## REST API

All 61 registry endpoints are exposed as HTTP routes following the current v3 path scheme. The canonical checked-in contract lives in the [OpenAPI file](/thetadatadx.yaml). SDK-only callback-based `*_stream` builders are documented separately in the API reference and are not exposed as HTTP routes.

```bash
# Stock EOD
curl "http://127.0.0.1:25503/v3/stock/history/eod?symbol=AAPL&start_date=20240101&end_date=20240301"

# Option snapshot
curl "http://127.0.0.1:25503/v3/option/snapshot/quote?symbol=SPY&expiration=20240419&strike=500&right=C"

# Stock OHLC range
curl "http://127.0.0.1:25503/v3/stock/history/ohlc_range?symbol=AAPL&start_date=20240101&end_date=20240301&interval=1m"

# Calendar
curl "http://127.0.0.1:25503/v3/calendar/open_today"
```

::: tip
Current REST parameters use the registry names (`symbol`, `expiration`, `strike`, `interval`, etc.), not older shorthand aliases like `root`, `exp`, or `ivl`.
:::

Response envelope matches the Java terminal:

```json
{
    "header": { "format": "json", "error_type": "null" },
    "response": [ ... ]
}
```

::: tip
The REST server uses sonic-rs (SIMD-accelerated JSON) for serialization, providing faster response times than the Java terminal on large payloads.
:::

## WebSocket API

The WebSocket server at `/v1/events` replicates the Java terminal's streaming protocol:

- Single client at a time
- STATUS heartbeat every second
- Event types: QUOTE, TRADE, OHLC, OPEN_INTEREST, STATUS

::: warning
The WebSocket endpoint supports a single concurrent client connection. If a second client connects, the first connection will be dropped. For multi-client setups, run multiple server instances on different ports.
:::

## Security Hardening

The REST server ships with defence-in-depth hardening applied in #377:

- **`POST /v3/system/shutdown` requires an `X-Shutdown-Token` header.** The token is set via the `THETADX_SHUTDOWN_TOKEN` environment variable (or `--shutdown-token`); requests missing or mismatching the header get `401 Unauthorized`. A separate per-IP limiter on the shutdown route caps attempts at ~3 per hour.
- **Global per-IP rate limit** via `tower_governor`: 20 requests per second with a burst of 40. Excess traffic is rejected with `429 Too Many Requests`.
- **256 max concurrent connections** enforced by a `tower::limit::GlobalConcurrencyLimitLayer`.
- **64 KB request-body limit** on every POST route via `RequestBodyLimitLayer`.
- **Dropped-events counter** exposed through every SDK surface:

  | SDK | Accessor |
  |-----|----------|
  | Python | `tdx.dropped_events()` |
  | TypeScript | `tdx.droppedEvents()` |
  | Go | `client.DroppedEvents()` |
  | C++ | `client.dropped_events()` |
  | Prometheus | `tdx_fpss_dropped_events`, `tdx_unified_dropped_events` |

  Every time the server publisher cannot keep up (the Disruptor ring buffer is full), the counter increments instead of silently dropping.

## System Routes

| Route | Description |
|-------|-------------|
| Method | Route | Description |
|--------|-------|-------------|
| `GET`  | `/v3/system/status` | Combined server status |
| `GET`  | `/v3/system/mdds/status` | MDDS connection status |
| `GET`  | `/v3/system/fpss/status` | FPSS connection status |
| `POST` | `/v3/system/shutdown` | Graceful shutdown — requires `X-Shutdown-Token` header |
