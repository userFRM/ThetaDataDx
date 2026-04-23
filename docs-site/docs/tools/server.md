---
title: REST Server
description: Drop-in replacement for the ThetaData Java Terminal v3 surface. Runs a local HTTP REST server and WebSocket server backed by ThetaDataDx.
---

# REST Server

Local HTTP REST and WebSocket server for the ThetaData v3 route surface, backed by ThetaDataDx. Existing scripts that target the terminal's current v3 routes can point at this binary on the same ports.

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

Responses use the terminal JSON envelope:

```json
{
    "header": { "format": "json", "error_type": "null" },
    "response": [ ... ]
}
```

::: tip
The REST server uses sonic-rs (SIMD-accelerated JSON) for response serialization.
:::

## WebSocket API

The WebSocket server at `/v1/events` uses the terminal streaming protocol:

- Single client at a time
- STATUS heartbeat every second
- Event types: QUOTE, TRADE, OHLC, OPEN_INTEREST, STATUS

::: warning
The WebSocket endpoint supports a single concurrent client connection. If a second client connects, the first connection will be dropped. For multi-client setups, run multiple server instances on different ports.
:::

## Security Hardening

The REST and WebSocket routers share the same defence-in-depth stack
(see `tools/server/src/router.rs::build()` and
`tools/server/src/ws.rs::router()`):

- **`POST /v3/system/shutdown` requires an `X-Shutdown-Token` header.** The token is generated as a fresh random UUID at server startup and printed once to stderr; the only way to learn it is to capture the startup log. There is no environment variable and no CLI flag for setting the token externally. Requests missing or mismatching the header get `401 Unauthorized`. Comparison uses `subtle::ConstantTimeEq` so the response latency doesn't leak prefix bytes. A route-scoped per-IP limiter caps attempts at roughly 3 per hour per IP.
- **Global per-IP rate limit** via `tower_governor::GovernorLayer` keyed on `PeerIpKeyExtractor` (the socket peer IP, NOT `X-Forwarded-For`): 20 requests per second with a burst of 40. Excess traffic is rejected with `429 Too Many Requests`.
- **256 max concurrent in-flight requests** enforced by `tower::limit::ConcurrencyLimitLayer::new(256)`.
- **64 KiB request-body limit** on every route via `axum::extract::DefaultBodyLimit::max(64 * 1024)`.
- **32 query-parameter cap** enforced by the custom `BoundedQuery<N>` extractor, which counts `&`-delimited pairs on the raw query string BEFORE `serde_urlencoded` allocates the HashMap. Attack shape: `?a=1&b=2&...` with thousands of unique keys — rejected during parse, HashMap never grows past the cap.
- **4 KiB WebSocket `Message::Text` cap** (`tools/server/src/ws.rs::WS_MAX_TEXT_BYTES`). Legitimate subscribe envelopes are under 200 bytes; larger frames are rejected before `sonic_rs::from_str` touches the buffer, closing the multi-megabyte JSON-bomb vector.
- **Dropped-events counter** exposed through every SDK surface when the FPSS bridge's bounded per-client channel fills up:

  | SDK | Accessor |
  |-----|----------|
  | Python | `tdx.dropped_events() -> int` |
  | TypeScript | `tdx.droppedEvents(): bigint` |
  | Go | `tdx.DroppedEvents() uint64` |
  | C / C++ (FFI) | `tdx_fpss_dropped_events(handle)`, `tdx_unified_dropped_events(handle)` |

  The counter increments instead of silently dropping, so slow consumers are observable without having to instrument the transport layer.

## System Routes

| Route | Description |
|-------|-------------|
| Method | Route | Description |
|--------|-------|-------------|
| `GET`  | `/v3/system/status` | Combined server status |
| `GET`  | `/v3/system/mdds/status` | MDDS connection status |
| `GET`  | `/v3/system/fpss/status` | FPSS connection status |
| `POST` | `/v3/system/shutdown` | Graceful shutdown — requires `X-Shutdown-Token` header |
