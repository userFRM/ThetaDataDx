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

All registry endpoints are exposed as HTTP routes following the current v3 path scheme. The canonical checked-in contract lives in the [OpenAPI file](/thetadatadx.yaml). SDK-only callback-based `*_stream` builders are documented separately in the API reference and are not exposed as HTTP routes.

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
Current REST parameters use the registry names (`symbol`, `expiration`, `strike`, `interval`, etc.), not older shorthand aliases like `root`, `exp`, or `ivl`. Date parameters (`date`, `start_date`, `end_date`, `expiration`) accept both `YYYYMMDD` and ISO `YYYY-MM-DD`.
:::

Responses use the terminal JSON envelope with `Content-Type: application/json` (bare media type, no `charset` parameter — UTF-8 is implied per RFC 8259):

```json
{
    "header": { "format": "json", "error_type": "null" },
    "response": [ ... ]
}
```

Every registry endpoint also accepts a `format` query parameter: `json` (default, the envelope above), `csv` (RFC 4180 with a header row), and `ndjson` / `jsonl` (one JSON object per row, `\n`-delimited, `Content-Type: application/x-ndjson; charset=utf-8`). Unknown `format` values return 400 with the supported set.

Failures use one canonical error envelope across every route family (registry endpoints, flat files, rate-limit rejections), so a single error parser covers the whole surface:

```json
{
    "header": { "error_type": "bad_request", "error_msg": "missing required parameter: 'date' (Date YYYYMMDD)" },
    "response": []
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

## Concurrency Model

Three admission gates compose on every historical request, and the first two **queue rather than reject**:

1. **HTTP edge cap (256).** `tower::limit::ConcurrencyLimitLayer::new(256)` bounds in-flight requests across all routes. The layer acquires a semaphore permit before the request runs — request 257 *waits* for a free slot; it is never rejected and produces no error response. The cap exists to shed pressure at the edge before the async task pool becomes the bottleneck.
2. **SDK tier semaphore.** Inside the SDK, every historical call acquires a permit from a semaphore sized to the resolved subscription tier's concurrency cap (the same size as the upstream channel pool). A burst larger than the tier cap queues FIFO and drains as upstream slots free — the same transparent queueing the legacy terminal performs. A client that disconnects (or applies its own timeout) drops the in-flight future, which releases both permits immediately.
3. **Upstream capacity.** If the upstream itself reports it is out of capacity, the SDK classifies the status as transient and retries with backoff before surfacing anything. Only when that retry budget is spent does the server answer — with `503 Service Unavailable`, `error_type: "upstream_exhausted"`, and a `Retry-After` header — never a bare 500, and never a 429 (429 is reserved for the per-IP rate limiter on non-loopback binds).

Practical consequence: a fan-out workload of hundreds of parallel requests against a loopback bind sees every request succeed, ordered by the two queues, with latency (not errors) absorbing the burst. There is no tuning knob equivalent to the legacy terminal's per-asset-class concurrency properties; the tier cap is detected from the subscription at startup.

## Security Hardening

The REST and WebSocket routers share the same hardening stack
(see `tools/server/src/router.rs::build()` and
`tools/server/src/ws.rs::router()`):

- **`POST /v3/system/shutdown` requires an `X-Shutdown-Token` header.** The token is generated as a fresh random UUID at server startup and printed once to stderr; the only way to learn it is to capture the startup log. There is no environment variable and no CLI flag for setting the token externally. Requests missing or mismatching the header get `401 Unauthorized`. Comparison uses `subtle::ConstantTimeEq` so the response latency doesn't leak prefix bytes. A route-scoped per-IP limiter caps attempts at roughly 3 per hour per IP.
- **Global per-IP rate limit on non-loopback binds** via `tower_governor::GovernorLayer` keyed on `PeerIpKeyExtractor` (the socket peer IP, NOT `X-Forwarded-For`): 20 requests per second with a burst of 40. Excess traffic is rejected with `429 Too Many Requests` carrying the canonical error envelope and a `Retry-After` header (seconds). On loopback binds (`127.0.0.1`, `::1` — the default) the general limiter is **disabled**: every local client shares one peer-IP bucket, so a parallel backtest or bulk pull would throttle itself as a group, which the legacy terminal never did. The shutdown-route limiter stays active on every bind.
- **256 max concurrent in-flight requests** enforced by `tower::limit::ConcurrencyLimitLayer::new(256)`. Requests past the cap **queue** on the layer's semaphore until a slot frees — they are not rejected.
- **64 KiB request-body limit** on every route via `axum::extract::DefaultBodyLimit::max(64 * 1024)`.
- **32 query-parameter cap** enforced by the custom `BoundedQuery<N>` extractor, which counts `&`-delimited pairs on the raw query string BEFORE `serde_urlencoded` allocates the HashMap. Attack shape: `?a=1&b=2&...` with thousands of unique keys — rejected during parse, HashMap never grows past the cap.
- **4 KiB WebSocket `Message::Text` cap** (`tools/server/src/ws.rs::WS_MAX_TEXT_BYTES`). Legitimate subscribe envelopes are under 200 bytes; larger frames are rejected before `sonic_rs::from_str` touches the buffer, closing the multi-megabyte JSON-bomb vector.
- **Dropped-events counter** exposed through every SDK surface when the FPSS bridge's bounded per-client channel fills up:

  | SDK | Accessor |
  |-----|----------|
  | Python | `tdx.dropped_event_count() -> int` |
  | TypeScript | `tdx.droppedEventCount(): bigint` |
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
