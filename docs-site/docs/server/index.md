---
title: Server — Getting Started
description: Run the local HTTP REST and WebSocket server speaking the v3 route surface.
---

# Server

`thetadatadx-server` runs the full data surface as a **local HTTP REST + WebSocket server** on the v3 route scheme. Scripts and tools that already speak the v3 routes point at it unchanged — same ports, same paths, same response envelope.

## Install and run

```bash
cargo install thetadatadx-server --git https://github.com/userFRM/ThetaDataDx

thetadatadx-server --creds creds.txt
```

This starts:

- **HTTP REST** on `http://127.0.0.1:25503` — every `/v3/...` route; see [HTTP API](/server/http).
- **WebSocket streaming** on `ws://127.0.0.1:25520/v1/events`; see [WebSocket Streaming](/server/websocket).

First request:

```bash
curl 'http://127.0.0.1:25503/v3/stock/history/eod?symbol=AAPL&start_date=2025-03-03&end_date=2025-03-06'
```

Every [reference page](/reference/)'s HTTP tab is a ready-made request against this server.

## Flags

| Flag | Default | Description |
|---|---|---|
| `--creds <path>` | `creds.txt` | Credentials file (email line 1, password line 2). |
| `--email` / `--password` | — | Inline credentials, as an alternative to `--creds`. |
| `--config <path>` | — | TOML [configuration](/articles/configuration) file. |
| `--fpss-region <region>` | `production` | Streaming environment: `production`, `dev`, `stage`. |
| `--http-port <port>` | `25503` | HTTP REST port. |
| `--ws-port <port>` | `25520` | WebSocket port. |
| `--bind <addr>` | `127.0.0.1` | Bind address. |
| `--log-level <filter>` | `info` | Tracing filter (`info,tower_http=off` silences the access log). |
| `--log-file <path>` | — | Also write logs to `<path>.YYYY-MM-DD`, rotated daily. |
| `--log-format <fmt>` | `text` | `text`, `json` (one object per line), or `legacy` (`[YYYY-MM-DD HH:MM:SS] LEVEL: message`, UTC). |
| `--no-fpss` | — | Skip streaming startup (HTTP only). |
| `--no-ohlcvc` | — | Disable derived OHLCVC bars on the stream. |

## Logging

- The access log (on by default) emits one `INFO` line per request with method, URI, status, and latency.
- `--log-file terminal.log` tees the same lines into a daily-rotated file through a non-blocking writer; stderr is unaffected.
- `--log-format legacy` matches the bracketed line shape older log-tailing tools parse; `--log-format json` suits log aggregators.

## Security defaults

- Binds loopback by default. On non-loopback binds, a per-IP rate limit (20 req/s, burst 40) answers excess traffic with `429` and `Retry-After`; loopback binds are never rate-limited.
- `POST /v3/system/shutdown` requires the `X-Shutdown-Token` header — a random token printed once to stderr at startup; there is no flag or environment variable to set it.
- Request bodies are capped at 64 KiB and query strings at 32 parameters.
