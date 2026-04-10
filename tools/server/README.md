# thetadatadx-server

Drop-in replacement for the ThetaData Java Terminal. Runs a local HTTP REST server and WebSocket server that expose the same API as the Java terminal, backed by native Rust gRPC (MDDS) and TCP (FPSS) connections to ThetaData's upstream servers.

Existing clients (Python SDK, Excel add-ins, curl scripts, browsers) work without any code changes - just swap the JAR for this binary.

## Quick start

```bash
# With email/password directly (no creds file needed)
thetadatadx-server --email you@example.com --password YOUR_PASSWORD

# With credentials file (same format as the Java terminal)
echo "your@email.com" > creds.txt
echo "your_password" >> creds.txt
thetadatadx-server --creds creds.txt

# With a TOML config file (same format as Java terminal's config.toml)
thetadatadx-server --email you@example.com --password YOUR_PASSWORD --config config.toml

# With a specific FPSS region
thetadatadx-server --email you@example.com --password YOUR_PASSWORD --fpss-region dev
```

The server starts:
- HTTP REST API on `http://127.0.0.1:25503` (same as Java terminal)
- WebSocket server on `ws://127.0.0.1:25520/v1/events` (same as Java terminal)

## Configuration

| Flag | Default | Description |
|------|---------|-------------|
| `--email` | | ThetaData email (alternative to `--creds`) |
| `--password` | | ThetaData password (alternative to `--creds`) |
| `--creds` | `creds.txt` | Path to credentials file |
| `--config` | | Path to TOML config file (same format as Java terminal) |
| `--fpss-region` | `production` | FPSS region: `production`, `dev`, `stage` |
| `--http-port` | `25503` | HTTP REST API port |
| `--ws-port` | `25520` | WebSocket server port |
| `--bind` | `127.0.0.1` | Bind address |
| `--log-level` | `info` | Log level (`debug`, `trace`, `thetadatadx=trace`) |
| `--no-fpss` | | Skip FPSS streaming connection at startup |

## REST API

All 61 registry endpoints are auto-generated into REST routes at startup from `ENDPOINTS`. Plus 4 system routes = 65 total HTTP routes.

Routes follow the current registry-driven v3 path scheme. The canonical checked-in contract is [`docs-site/public/thetadatadx.yaml`](../../docs-site/public/thetadatadx.yaml).

Representative examples:

```
GET /v3/stock/list/symbols
GET /v3/stock/list/dates?request_type=EOD&symbol=AAPL
GET /v3/stock/history/eod?symbol=AAPL&start_date=20240101&end_date=20240301
GET /v3/stock/history/ohlc?symbol=AAPL&date=20240315&interval=1m
GET /v3/stock/history/ohlc_range?symbol=AAPL&start_date=20240101&end_date=20240301&interval=1m
GET /v3/option/snapshot/quote?symbol=SPY&expiration=20240419&strike=500000&right=C
GET /v3/calendar/open_today
GET /v3/rate/history/eod?symbol=SOFR&start_date=20240101&end_date=20240301
```

Endpoint query parameters follow the registry names (`symbol`, `expiration`, `strike`, `right`, `interval`, etc.), not the legacy shorthand aliases (`root`, `exp`, `ivl`).

### System Routes (4)

```
GET /v3/system/status          # {"status":"CONNECTED","version":"5.3.1"}
GET /v3/system/mdds/status
GET /v3/system/fpss/status     # {"status":"CONNECTED","version":"5.3.1"}
GET /v3/system/shutdown
```

### Response format

Responses match the Java terminal exactly:

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

## WebSocket

Connect to `ws://127.0.0.1:25520/v1/events` to receive streaming events.

The server sends:
- `STATUS` messages every second with FPSS connection state
- `QUOTE`, `TRADE`, `OHLC` events when FPSS is connected and subscriptions are active

Send JSON commands to manage subscriptions:

```json
{
    "msg_type": "STREAM",
    "sec_type": "STOCK",
    "req_type": "QUOTE",
    "add": true,
    "id": 1,
    "contract": {"root": "AAPL"}
}
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
    |--- ThetaDataDx (MDDS gRPC + FPSS TCP)
    |    historical data + real-time streaming
    |
ThetaData upstream servers (NJ datacenter)
```

## Differences from the Java terminal

| | Java terminal | thetadatadx-server |
|---|---|---|
| Runtime | JVM (200+ MB) | Native binary (~10 MB) |
| Startup | 3-5 seconds | < 0.5 seconds |
| Memory | 400+ MB baseline | ~20 MB baseline |
| API | Same | Same |
| CORS | No | Yes (enabled by default) |
| Protocol | Same gRPC/TCP | Same gRPC/TCP |
