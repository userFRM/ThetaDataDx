<p align="center">
  <img src="../../assets/logo.svg" alt="ThetaDataDx" width="100%" />
</p>

# thetadatadx-server

Runs a local HTTP REST server and WebSocket server that expose the ThetaData `/v3/*` route surface, backed by Rust gRPC (market-data) and TCP (streaming) connections to ThetaData's upstream servers.

Existing clients using the current `/v3/*` local terminal routes can point at this binary on the same port.

> **Flat files:** the server answers the terminal's flat-file routes, `GET /v3/{option,stock,index}/flat_file/{dataset}?date=YYYYMMDD&format=csv|json|ndjson|jsonl|html`, for every file the terminal serves: option `eod`, `trade_quote`, `open_interest` and `quote`; stock `eod`, `trade_quote` and `quote`; index `eod`. Each file is written in the terminal's format, chunk by chunk as it arrives, so the first bytes leave at once and server memory stays flat at any file size.

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

# With a specific streaming region (the market-data region is selected the same
# way with --market-data-region)
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
| `--config` | | Path to a TOML config file in the SDK's format (see `thetadatadx-rs/config.default.toml`). The file's values are used as written, except `bulk_fetch`: the server sends each request upstream once, as the terminal does, so the rows come back in the terminal's order. Without it, the server uses production settings with one change suited to a long-running local process: the stream backs off when idle (`wait_mode = "backoff"`) instead of holding a core. |
| `--market-data-region` | `production` | Market-data region: `production` or `stage` |
| `--streaming-region` | `production` | Streaming region: `production` or `dev` |
| `--http-port` | `25503` | HTTP REST API port |
| `--ws-port` | `25520` | WebSocket server port |
| `--bind` | `0.0.0.0` | Bind address. Defaults to all interfaces, matching the terminal this server replaces; pass `--bind 127.0.0.1` to restrict to loopback. |
| `--log-level` | `info` | Log level (`debug`, `trace`, `thetadatadx=trace`; `info,tower_http=off` silences the access log) |
| `--log-file` | | Also write logs to `<path>.YYYY-MM-DD`, rotated daily |
| `--log-format` | `text` | Log line format: `text`, `json`, or `legacy` (`[YYYY-MM-DD HH:MM:SS] LEVEL: message`, UTC) |
| `--no-streaming` | | Skip the streaming connection at startup |

Every request emits one `INFO` access-log line (method, URI, status, latency) by default. The startup banner prints `thetadatadx-server v<version>`.

Credentials resolve in this order, highest first: the `--api-key` flag, then `THETADATA_API_KEY`, then `THETADATA_EMAIL` + `THETADATA_PASSWORD`, then the `--creds` file (default `creds.txt`: email on line 1, password on line 2). These are the same names the SDK, the CLI, and the MCP server read, so one login authenticates every tool.

### Environment variables

These variables are read from the environment. The credential variables (`THETADATA_API_KEY`, and the `THETADATA_EMAIL` + `THETADATA_PASSWORD` pair) authenticate the server. Full descriptions live in [`docs-site/docs/server/index.md`](../../docs-site/docs/server/index.md).

| Variable | Default | Description |
|----------|---------|-------------|
| `THETADATA_API_KEY` | | API key for authentication when `--api-key` is not passed. An explicit `--api-key` flag wins over this; both win over the email/password path. The key is never logged or echoed. |
| `THETADATA_EMAIL` | | Account email. With `THETADATA_PASSWORD`, authenticates the server when no API key is supplied. Outranked by `--api-key` and `THETADATA_API_KEY`; wins over the `--creds` file. |
| `THETADATA_PASSWORD` | | Account password, paired with `THETADATA_EMAIL`. Never logged or echoed. |
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
GET /v3/stock/history/ohlc?symbol=AAPL&start_date=20240101&end_date=20240301&interval=1m
GET /v3/option/snapshot/quote?symbol=SPY&expiration=20240419&strike=500&right=C
GET /v3/calendar/open_today
GET /v3/rate/history/eod?symbol=SOFR&start_date=20240101&end_date=20240301
```

Endpoint query parameters follow the registry names (`symbol`, `expiration`, `strike`, `right`, `interval`, etc.), not the legacy shorthand aliases (`root`, `exp`, `ivl`). Date parameters (`date`, `start_date`, `end_date`, `expiration`) accept both `YYYYMMDD` and ISO `YYYY-MM-DD`.

### Terminal system routes (3)

Mirrored 1:1 from the JVM terminal: unauthenticated `GET`, bare `text/plain` bodies.

```
GET /v3/terminal/shutdown       # "OK"; kills the server process
GET /v3/terminal/fpss/status    # streaming channel health: CONNECTED / DISCONNECTED
GET /v3/terminal/mdds/status    # market-data channel health: CONNECTED / DISCONNECTED
```

### Response format

Every registry endpoint accepts a `format` query parameter: `csv` (default, RFC 4180 with a header row and CRLF line endings), `json`, `ndjson` / `jsonl` (one JSON object per row, `\n`-delimited), and `html` (a browser-viewable `<table>`, served inline as `text/html`). Unknown `format` values return 400 with the supported set.

JSON responses carry the v3 body `{ "response": [ ... ] }` (no `header` key), `Content-Type: application/json`. Stock and index rows are flat; option / contract endpoints group their rows under the owning contract:

```json
{
    "response": [
        {
            "contract": {"symbol": "AAPL", "strike": 550.000, "expiration": "2026-06-18", "right": "CALL"},
            "data": [
                {"timestamp": "2024-01-02T17:17:53.606", "open": 150.25}
            ]
        }
    ]
}
```

Timestamps are ISO strings (the v2 `ms_of_day` / `date` columns are folded into them) and the option `right` is `CALL` / `PUT`. Each price is written at the precision the vendor sent it with, as the terminal writes it (`550.000`, `0.5800`).

Each response is written as its rows arrive from the vendor, as the terminal writes it. A response that ends within 8 KiB is sent whole, with its `Content-Length`; a longer one is sent with chunked transfer encoding from the moment it outgrows that, so its first rows leave before its last arrive and server memory stays flat at any response size. `json_legacy` is one array per column, so no column is complete before the last row arrives, and it is held whole until then. A failure after a response has begun aborts the transfer, so a truncated response cannot pass for a complete one.

Every failed request returns the HTTP status with a plain-text (`text/plain`) description, as the terminal answers it.

## WebSocket

Connect to `ws://127.0.0.1:25520/v1/events` to receive streaming events.

One client at a time: a second connection replaces the first. The existing client receives a Close frame (code 1000, reason `replaced by a new client connection`) and the new client takes over the stream, matching the legacy terminal.

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

- **256 simultaneous HTTP requests admitted at the edge:** requests past the cap queue and are served in order rather than rejected, so a connection flood can't exhaust the runtime. Genuine upstream exhaustion surfaces as `503` + `Retry-After`, not 500. **64 KiB body limit**, **4 KiB WebSocket `Message::Text` cap**.
- **`BoundedQuery<32>` extractor** counts `&`-delimited query-string pairs BEFORE `serde_urlencoded` runs, so a `?a=1&b=2&...` flood is rejected at parse time rather than after HashMap rehashing allocates MB+.
- **Streaming TLS** verifies every peer against a captured SubjectPublicKeyInfo pin (`PinnedVerifier`, constant-time SHA-256 compare); MITM presenting any other cert is rejected even if it chains to a trusted CA. See `docs-site/docs/streaming/index.md`.
- **Dropped events are logged:** when the WebSocket fan-out falls behind and the server's broadcast buffer is full, events are dropped rather than stalling the stream, and a warning with the running total is logged every 1,024 drops.

Example: shutting the server down, exactly as with the JVM terminal:

```bash
curl http://127.0.0.1:25503/v3/terminal/shutdown
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
    |--- ThetaDataDx (market-data + streaming)
    |    historical data + real-time streaming
    |
ThetaData upstream servers (NJ datacenter)
```
