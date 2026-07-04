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

The cleaner way to sign in is an API key: generate one from your [ThetaData user portal](https://www.thetadata.net/) and pass it with `--api-key`, or set `THETADATA_API_KEY` in the environment and run the binary with no credential flag. You can also sign in with your email and password: pass `--email` / `--password`, set `THETADATA_EMAIL` + `THETADATA_PASSWORD` in the environment, or use a `--creds <path>` file (email line 1, password line 2). Otherwise, on the first launch the server prompts for the email and password of your ThetaData login, then saves them to a local `creds.txt` file so the prompt does not repeat.

Credentials resolve in this order, highest first: the `--api-key` flag, then `THETADATA_API_KEY`, then `THETADATA_EMAIL` + `THETADATA_PASSWORD`, then the `--creds` file (default `creds.txt`). These are the same names the SDK, the CLI, and the [MCP server](/mcp) read, so one login authenticates every tool.

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
| `--api-key <key>` | — | Authenticate with a ThetaData API key (or set `THETADATA_API_KEY`). Takes precedence over the environment variables and the email/password path. |
| `--creds <path>` | `creds.txt` | Credentials file (email line 1, password line 2). |
| `--email` / `--password` | — | Inline credentials (or set `THETADATA_EMAIL` + `THETADATA_PASSWORD`, or use `--creds`). |
| `--config <path>` | — | TOML [configuration](/articles/configuration) file. |
| `--historical-region <region>` | `production` | Historical environment: `production` or `stage`. |
| `--streaming-region <region>` | `production` | Streaming environment: `production` or `dev`. |
| `--http-port <port>` | `25503` | HTTP REST port. |
| `--ws-port <port>` | `25520` | WebSocket port. |
| `--bind <addr>` | `0.0.0.0` | Bind address. Defaults to all interfaces; pass `--bind 127.0.0.1` to restrict to loopback. |
| `--log-level <filter>` | `info` | Tracing filter (`info,tower_http=off` silences the access log). |
| `--log-file <path>` | — | Also write logs to `<path>.YYYY-MM-DD`, rotated daily. |
| `--log-format <fmt>` | `text` | `text`, `json` (one object per line), or `legacy` (`[YYYY-MM-DD HH:MM:SS] LEVEL: message`, UTC). |
| `--no-streaming` | — | Skip streaming startup (HTTP only). |

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `THETADATA_API_KEY` | — | API key used for authentication when `--api-key` is not passed. An explicit `--api-key` flag wins over this; both win over the email/password path. The key is never logged or echoed. |
| `THETADATA_EMAIL` | — | Account email. With `THETADATA_PASSWORD`, authenticates the server when no API key is supplied. Outranked by `--api-key` and `THETADATA_API_KEY`; wins over the `--creds` file. |
| `THETADATA_PASSWORD` | — | Account password, paired with `THETADATA_EMAIL`. Never logged or echoed. |
| `THETADATADX_WS_CLIENT_CAPACITY` | `4096` | Per-client WebSocket send-buffer capacity in events. A larger buffer trades memory for more headroom before a slow consumer starts dropping events; invalid or zero values keep the default. |

## Logging

- The access log (on by default) emits one `INFO` line per request with method, URI, status, and latency.
- `--log-file terminal.log` tees the same lines into a daily-rotated file through a non-blocking writer; stderr is unaffected.
- `--log-format legacy` matches the bracketed line shape older log-tailing tools parse; `--log-format json` suits log aggregators.

## Security defaults

- Binds all interfaces by default; pass `--bind 127.0.0.1` for loopback-only exposure. The server imposes no per-IP rate limit, matching the terminal it replaces — request limits are enforced upstream by the data service.
- The system routes mirror the terminal 1:1 and are unauthenticated, including `GET /v3/terminal/shutdown` — exactly as the terminal exposes them.
- Request bodies are capped at 64 KiB and query strings at 32 parameters.
