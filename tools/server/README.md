<p align="center">
  <img src="../../assets/logo.svg" alt="ThetaDataDx" width="120" />
</p>

# thetadatadx-server

Runs a local HTTP REST server and WebSocket server that expose the ThetaData `/v3/*` route surface, backed by Rust gRPC (historical) and TCP (streaming) connections to ThetaData's upstream servers.

Existing clients using the current `/v3/*` local terminal routes can point at this binary on the same port.

> **FLATFILES coverage:** the REST server exposes FLATFILES whole-universe daily blobs at `GET /v3/flatfile/{sec_type}/{req_type}?date=YYYYMMDD&format=csv|jsonl` and `POST /v3/flatfile/request`. Bytes are streamed back via a chunked response body so large blobs do not pin server memory. Flat files are batch downloads, not streaming subscriptions; the WebSocket surface is unchanged.

## Quick start

```bash
# With an API key (or set THETADATA_API_KEY in the environment)
thetadatadx-server --api-key YOUR_API_KEY
export THETADATA_API_KEY="YOUR_API_KEY" && thetadatadx-server

# With email/password directly (no creds file needed)
thetadatadx-server --email you@example.com --password YOUR_PASSWORD

# Or with email/password in the environment
export THETADATA_EMAIL="you@example.com"
export THETADATA_PASSWORD="YOUR_PASSWORD"
thetadatadx-server

# With credentials file
echo "your@email.com" > creds.txt
echo "your_password" >> creds.txt
thetadatadx-server --creds creds.txt

# With a TOML config file
thetadatadx-server --email you@example.com --password YOUR_PASSWORD --config config.toml

# With a specific streaming region (the historical region is selected the same
# way with --historical-region)
thetadatadx-server --email you@example.com --password YOUR_PASSWORD --streaming-region dev
```

The server starts:
- HTTP REST API on `http://127.0.0.1:25503`
- WebSocket server on `ws://127.0.0.1:25520/v1/events`

## Configuration

| Flag | Default | Description |
|------|---------|-------------|
| `--api-key` | | Authenticate with a ThetaData API key (or set `THETADATA_API_KEY`). Takes precedence over the environment variables and the email/password path. |
| `--email` | | ThetaData email (or set `THETADATA_EMAIL` + `THETADATA_PASSWORD`, or use `--creds`) |
| `--password` | | ThetaData password (or set `THETADATA_EMAIL` + `THETADATA_PASSWORD`, or use `--creds`) |
| `--creds` | `creds.txt` | Path to credentials file (email line 1, password line 2) |
| `--config` | | Path to TOML config file |
| `--historical-region` | `production` | Historical region: `production` or `stage` |
| `--streaming-region` | `production` | Streaming region: `production` or `dev` |
| `--http-port` | `25503` | HTTP REST API port |
| `--ws-port` | `25520` | WebSocket server port |
| `--bind` | `127.0.0.1` | Bind address |
| `--log-level` | `info` | Log level (`debug`, `trace`, `thetadatadx=trace`; `info,tower_http=off` silences the access log) |
| `--log-file` | | Also write logs to `<path>.YYYY-MM-DD`, rotated daily |
| `--log-format` | `text` | Log line format: `text`, `json`, or `legacy` (`[YYYY-MM-DD HH:MM:SS] LEVEL: message`, UTC) |
| `--no-streaming` | | Skip the streaming connection at startup |
| `--no-ohlcvc` | | Disable OHLCVC bar derivation from trades on the streaming feed |

Every request emits one `INFO` access-log line (method, URI, status, latency) by default. The startup banner prints `thetadatadx-server v<version>`.

Credentials resolve in this order, highest first: the `--api-key` flag, then `THETADATA_API_KEY`, then `THETADATA_EMAIL` + `THETADATA_PASSWORD`, then the `--creds` file (default `creds.txt`: email on line 1, password on line 2). These are the same names the SDK, the CLI, and the MCP server read, so one login authenticates every tool.

### Environment variables

These variables are read from the environment. The credential variables (`THETADATA_API_KEY`, and the `THETADATA_EMAIL` + `THETADATA_PASSWORD` pair) authenticate the server; the rate-limit knobs have no flag. Per-IP rate limiting is off by default (matching the terminal it replaces); setting either rate-limit variable opts in. Full descriptions live in [`docs-site/docs/server/index.md`](../../docs-site/docs/server/index.md).

| Variable | Default | Description |
|----------|---------|-------------|
| `THETADATA_API_KEY` | | API key for authentication when `--api-key` is not passed. An explicit `--api-key` flag wins over this; both win over the email/password path. The key is never logged or echoed. |
| `THETADATA_EMAIL` | | Account email. With `THETADATA_PASSWORD`, authenticates the server when no API key is supplied. Outranked by `--api-key` and `THETADATA_API_KEY`; wins over the `--creds` file. |
| `THETADATA_PASSWORD` | | Account password, paired with `THETADATA_EMAIL`. Never logged or echoed. |
| `THETADATADX_RATE_LIMIT_PER_SECOND` | off | Opt into per-IP rate limiting at this many requests per second. Setting either rate-limit variable turns the limiter on. |
| `THETADATADX_RATE_LIMIT_BURST_SIZE` | off | Burst size for the per-IP rate limiter. If only one of the two rate-limit variables is set, the other falls back to `20` req/s / `40` burst. |
| `THETADATADX_WS_CLIENT_CAPACITY` | `4096` | Per-client WebSocket send-buffer capacity in events. A larger buffer trades memory for more headroom before a slow consumer drops events; invalid or zero values keep the default. |

## REST API

All registry endpoints are auto-generated into REST routes at startup from `ENDPOINTS`, alongside the hand-written system routes.

Routes follow the current registry-driven v3 path scheme. The canonical checked-in contract is [`docs-site/docs/public/thetadatadx.yaml`](../../docs-site/docs/public/thetadatadx.yaml).
SDK-only callback-based `*_stream` builders are documented in the API reference and are not exposed as HTTP routes.

Representative examples:

```
GET /v3/stock/list/symbols
GET /v3/stock/list/dates?request_type=EOD&symbol=AAPL
GET /v3/stock/history/eod?symbol=AAPL&start_date=20240101&end_date=20240301
GET /v3/stock/history/ohlc?symbol=AAPL&date=20240315&interval=1m
GET /v3/stock/history/ohlc_range?symbol=AAPL&start_date=20240101&end_date=20240301&interval=1m
GET /v3/option/snapshot/quote?symbol=SPY&expiration=20240419&strike=500&right=C
GET /v3/calendar/open_today
GET /v3/rate/history/eod?symbol=SOFR&start_date=20240101&end_date=20240301
```

Endpoint query parameters follow the registry names (`symbol`, `expiration`, `strike`, `right`, `interval`, etc.), not the legacy shorthand aliases (`root`, `exp`, `ivl`). Date parameters (`date`, `start_date`, `end_date`, `expiration`) accept both `YYYYMMDD` and ISO `YYYY-MM-DD`.

### System Routes (4)

```
GET  /v3/system/status          # {"status":"CONNECTED","version":"<crate version>"}
GET  /v3/system/historical/status    # envelope: {"header":{...},"response":["CONNECTED"]}
GET  /v3/system/streaming/status     # {"status":"CONNECTED","version":"<crate version>","broadcast_dropped":0,"json_serialize_failures":0}
POST /v3/system/shutdown        # requires X-Shutdown-Token header
```

### Response format

Every registry endpoint accepts a `format` query parameter: `json` (default), `csv` (RFC 4180 with a header row), and `ndjson` / `jsonl` (one JSON object per row, `\n`-delimited, `Content-Type: application/x-ndjson; charset=utf-8`). Unknown `format` values return 400 with the supported set.

JSON responses use the terminal envelope with `Content-Type: application/json` (bare media type, no `charset` parameter — UTF-8 is implied per RFC 8259):

```json
{
    "header": {
        "format": "json",
        "error_type": "null"
    },
    "response": [
        {"ms_of_day": 34200000, "open": 150.25, ...}
    ]
}
```

Failures use one canonical error envelope across every route family (registry endpoints, flat files, rate-limit rejections), so a single error parser covers the whole surface:

```json
{
    "header": {
        "error_type": "bad_request",
        "error_msg": "missing required parameter: 'date' (Date YYYYMMDD)"
    },
    "response": []
}
```

## WebSocket

Connect to `ws://127.0.0.1:25520/v1/events` to receive streaming events.

One client at a time: a second connection replaces the first — the existing client receives a Close frame (code 1000, reason `replaced by a new client connection`) and the new client takes over the stream, matching the legacy terminal.

The server sends:
- `STATUS` messages every second with the streaming connection state
- `QUOTE`, `TRADE`, `OHLC` events when the streaming feed is connected and subscriptions are active

Send JSON commands to manage subscriptions:

```json
{
    "msg_type": "STREAM",
    "sec_type": "STOCK",
    "req_type": "QUOTE",
    "add": true,
    "id": 1,
    "contract": {"symbol": "AAPL"}
}
```

## Hardening

- **`POST /v3/system/shutdown`** requires a 128-bit random hex `X-Shutdown-Token` header (32 hex chars) printed once to stderr at startup. Token is compared in constant time so response latency does not leak the secret one byte at a time; no env var or CLI flag sets it externally. A route-scoped per-IP limiter caps attempts at roughly 3 per hour.
- **Opt-in global per-IP rate limit** via `tower_governor::GovernorLayer` keyed on `PeerIpKeyExtractor` (peer TCP socket, **not** `X-Forwarded-For`). The general limiter is **off by default on every bind regardless of address**; operators opt in by setting `THETADATADX_RATE_LIMIT_PER_SECOND` and/or `THETADATADX_RATE_LIMIT_BURST_SIZE` (setting either turns it on; a partially-set pair falls back to 20 rps / burst 40). Once on, excess traffic from a single IP is rejected as `429` with the canonical error envelope and a `Retry-After` header, on both the HTTP routes and the WS upgrade. The shutdown-route limiter stays active on every bind. The server runs without a trusted reverse proxy, so forwarded-header extractors would let an attacker cycle fake IPs.
- **256 concurrent in-flight requests** — requests past the cap queue on the layer's semaphore (they are not rejected), then queue again on the SDK's tier-sized request semaphore that matches the upstream concurrency cap. Bursts absorb as latency, not errors; see the Concurrency Model section in `docs-site/docs/server/http.md`. Upstream capacity rejections that survive the SDK's retry budget surface as `503` + `Retry-After`, not 500. **64 KiB body limit**, **4 KiB WebSocket `Message::Text` cap**.
- **`BoundedQuery<32>` extractor** counts `&`-delimited query-string pairs BEFORE `serde_urlencoded` runs, so a `?a=1&b=2&...` flood is rejected at parse time rather than after HashMap rehashing allocates MB+.
- **CSV output defuses formula injection** — cells whose first byte is `=` / `+` / `-` / `@` / `\t` are prefixed with a single-quote `'` and CSV-quoted.
- **Streaming TLS** verifies every peer against a captured SubjectPublicKeyInfo pin (`PinnedVerifier`, constant-time SHA-256 compare); MITM presenting any other cert is rejected even if it chains to a trusted CA. See `docs-site/docs/streaming/index.md`.
- **Dropped-events observability** — per-client mpsc channels surface a monotonic `AtomicU64` counter through every SDK (`client.dropped_events()` Python, `droppedEvents(): bigint` TS, `thetadatadx_streaming_dropped_events` / `thetadatadx_client_dropped_events` FFI) plus `tracing::debug!` on `thetadatadx::sdk::streaming`.

Example — initiating a graceful shutdown from the same machine:

```bash
# Server prints these lines once at startup on stderr (TOKEN is a
# 128-bit random hex string, 32 hex chars):
#   Shutdown token: <TOKEN>
#     curl -X POST http://127.0.0.1:25503/v3/system/shutdown -H 'X-Shutdown-Token: <TOKEN>'
curl -X POST -H "X-Shutdown-Token: <TOKEN>" http://127.0.0.1:25503/v3/system/shutdown
```

## Architecture

```
External apps (Python, Excel, browsers)
    |
    |--- HTTP REST :25503 (/v3/...)
    |--- WebSocket :25520 (/v1/events)
    |
thetadatadx-server (Rust binary)
    |
    |--- ThetaDataDx (historical + streaming)
    |    historical data + real-time streaming
    |
ThetaData upstream servers (NJ datacenter)
```
