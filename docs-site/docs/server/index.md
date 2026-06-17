---
title: Server — Getting Started
description: Run the local HTTP REST and WebSocket server speaking the v3 route surface.
---

# Server

`thetadatadx-server` runs the full data surface as a **local HTTP REST + WebSocket server** on the v3 route scheme. Scripts and tools that already speak the v3 routes point at it unchanged — same ports, same paths, same response envelope.

## Download and run

Prebuilt binaries are attached to each [GitHub Release](https://github.com/userFRM/ThetaDataDx/releases). No Rust toolchain and nothing else to install locally — download the archive for your platform, unpack it, and run the binary.

The binaries are not code-signed yet, so each operating system asks for a one-time confirmation the first time you launch one.

### Windows

1. Download `thetadatadx-server-windows-x86_64.zip` and unzip it.
2. Double-click `thetadatadx-server.exe`. If a "Windows protected your PC" SmartScreen dialog appears, click **More info**, then **Run anyway**.

### macOS

1. Download `thetadatadx-server-macos-arm64.tar.gz` (Apple silicon) or `thetadatadx-server-macos-x86_64.tar.gz` (Intel) and unpack it.
2. Right-click (or Control-click) `thetadatadx-server` and choose **Open**, then confirm **Open** in the dialog. If macOS still blocks it, open **System Settings > Privacy & Security** and click **Open Anyway** next to the `thetadatadx-server` entry.

### Linux

1. Download `thetadatadx-server-linux-x86_64.tar.gz` and unpack it. The Linux binary is statically linked, so it runs on any distribution with no extra system libraries.
2. Make it executable and run it:

```bash
chmod +x thetadatadx-server
./thetadatadx-server
```

On the first launch the server prompts for the email and password of your ThetaData login, then saves them to a local `creds.txt` file so the prompt does not repeat. Pass `--creds <path>` to use a different file, or supply `--email` / `--password` directly.

This starts:

- **HTTP REST** on `http://127.0.0.1:25503` — every `/v3/...` route; see [HTTP API](/server/http).
- **WebSocket streaming** on `ws://127.0.0.1:25520/v1/events`; see [WebSocket Streaming](/server/websocket).

First request:

```bash
curl 'http://127.0.0.1:25503/v3/stock/history/eod?symbol=AAPL&start_date=2025-03-03&end_date=2025-03-06'
```

Every [reference page](/reference/)'s HTTP tab is a ready-made request against this server.

## Build from source (advanced)

If you have a Rust toolchain and prefer to build the binary yourself, install it from the repository and run it the same way:

```bash
cargo install thetadatadx-server --git https://github.com/userFRM/ThetaDataDx

thetadatadx-server --creds creds.txt
```

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
| `--no-streaming` | — | Skip streaming startup (HTTP only). |
| `--no-ohlcvc` | — | Disable derived OHLCVC bars on the stream. |

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `THETADATADX_RATE_LIMIT_PER_SECOND` | — (off) | Opt into per-IP rate limiting at this many requests per second. Setting either rate-limit variable turns the limiter on; see [Security defaults](#security-defaults). |
| `THETADATADX_RATE_LIMIT_BURST_SIZE` | — (off) | Burst size for the per-IP rate limiter. If you set only one of the two rate-limit variables, the other falls back to `20` req/s / `40` burst. |
| `THETADATADX_WS_CLIENT_CAPACITY` | `4096` | Per-client WebSocket send-buffer capacity in events. A larger buffer trades memory for more headroom before a slow consumer starts dropping events; invalid or zero values keep the default. |

## Logging

- The access log (on by default) emits one `INFO` line per request with method, URI, status, and latency.
- `--log-file terminal.log` tees the same lines into a daily-rotated file through a non-blocking writer; stderr is unaffected.
- `--log-format legacy` matches the bracketed line shape older log-tailing tools parse; `--log-format json` suits log aggregators.

## Security defaults

- Binds loopback by default. Per-IP rate limiting is **off by default** — the server imposes no per-IP limit, matching the terminal it replaces. Operators exposing the server as a relay opt in by setting `THETADATADX_RATE_LIMIT_PER_SECOND` and/or `THETADATADX_RATE_LIMIT_BURST_SIZE`; once on, excess traffic from a single IP is answered with `429` and `Retry-After`, on both the HTTP routes and the WebSocket upgrade.
- `POST /v3/system/shutdown` requires the `X-Shutdown-Token` header — a random token printed once to stderr at startup; there is no flag or environment variable to set it.
- Request bodies are capped at 64 KiB and query strings at 32 parameters.
