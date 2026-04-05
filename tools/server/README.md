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

Routes follow the Java terminal's URL patterns:

### URL Patterns

| Pattern | Example |
|---------|---------|
| `/v3/list/roots/{sec_type}` | `/v3/list/roots/stock` |
| `/v3/list/dates/{sec_type}` | `/v3/list/dates/stock?request_type=EOD&symbol=AAPL` |
| `/v3/list/{what}` | `/v3/list/expirations?root=SPY` |
| `/v3/hist/{sec_type}/{what}` | `/v3/hist/stock/eod?root=AAPL&start_date=20240101&end_date=20240301` |
| `/v3/hist/{sec_type}/greeks/{what}` | `/v3/hist/option/greeks/all?root=SPY&exp=20240419&strike=500000&right=C&date=20240315&ivl=60000` |
| `/v3/hist/{sec_type}/trade_greeks/{what}` | `/v3/hist/option/trade_greeks/all?root=SPY&exp=20240419&strike=500000&right=C&date=20240315` |
| `/v3/snapshot/{sec_type}/{what}` | `/v3/snapshot/stock/quote?root=AAPL` |
| `/v3/snapshot/{sec_type}/greeks/{what}` | `/v3/snapshot/option/greeks/all?root=SPY&exp=20240419&strike=500000&right=C` |
| `/v3/at_time/{sec_type}/{what}` | `/v3/at_time/stock/trade?root=AAPL&start_date=20240101&end_date=20240301&time_of_day=34200000` |
| `/v3/calendar/{what}` | `/v3/calendar/open_today` |
| `/v3/system/{what}` | `/v3/system/mdds/status` |

### Stock Routes (14)

```
GET /v3/list/roots/stock
GET /v3/list/dates/stock?request_type=...&symbol=...
GET /v3/snapshot/stock/ohlc?root=...
GET /v3/snapshot/stock/trade?root=...
GET /v3/snapshot/stock/quote?root=...
GET /v3/snapshot/stock/market_value?root=...
GET /v3/hist/stock/eod?root=...&start_date=...&end_date=...
GET /v3/hist/stock/ohlc?root=...&date=...&ivl=...
GET /v3/hist/stock/ohlc_range?root=...&start_date=...&end_date=...&ivl=...
GET /v3/hist/stock/trade?root=...&date=...
GET /v3/hist/stock/quote?root=...&date=...&ivl=...
GET /v3/hist/stock/trade_quote?root=...&date=...
GET /v3/at_time/stock/trade?root=...&start_date=...&end_date=...&time_of_day=...
GET /v3/at_time/stock/quote?root=...&start_date=...&end_date=...&time_of_day=...
```

### Option Routes (34)

```
GET /v3/list/roots/option
GET /v3/list/dates/option?request_type=...&symbol=...&expiration=...&strike=...&right=...
GET /v3/list/expirations?root=...
GET /v3/list/strikes?root=...&exp=...
GET /v3/list/contracts?request_type=...&symbol=...&date=...
GET /v3/snapshot/option/ohlc?root=...&exp=...&strike=...&right=...
GET /v3/snapshot/option/trade?root=...&exp=...&strike=...&right=...
GET /v3/snapshot/option/quote?root=...&exp=...&strike=...&right=...
GET /v3/snapshot/option/open_interest?root=...&exp=...&strike=...&right=...
GET /v3/snapshot/option/market_value?root=...&exp=...&strike=...&right=...
GET /v3/snapshot/option/greeks/implied_volatility?root=...&exp=...&strike=...&right=...
GET /v3/snapshot/option/greeks/all?root=...&exp=...&strike=...&right=...
GET /v3/snapshot/option/greeks/first_order?root=...&exp=...&strike=...&right=...
GET /v3/snapshot/option/greeks/second_order?root=...&exp=...&strike=...&right=...
GET /v3/snapshot/option/greeks/third_order?root=...&exp=...&strike=...&right=...
GET /v3/hist/option/eod?root=...&exp=...&strike=...&right=...&start_date=...&end_date=...
GET /v3/hist/option/ohlc?root=...&exp=...&strike=...&right=...&date=...&ivl=...
GET /v3/hist/option/trade?root=...&exp=...&strike=...&right=...&date=...
GET /v3/hist/option/quote?root=...&exp=...&strike=...&right=...&date=...&ivl=...
GET /v3/hist/option/trade_quote?root=...&exp=...&strike=...&right=...&date=...
GET /v3/hist/option/open_interest?root=...&exp=...&strike=...&right=...&date=...
GET /v3/hist/option/greeks/eod?root=...&exp=...&strike=...&right=...&start_date=...&end_date=...
GET /v3/hist/option/greeks/all?root=...&exp=...&strike=...&right=...&date=...&ivl=...
GET /v3/hist/option/trade_greeks/all?root=...&exp=...&strike=...&right=...&date=...
GET /v3/hist/option/greeks/first_order?root=...&exp=...&strike=...&right=...&date=...&ivl=...
GET /v3/hist/option/trade_greeks/first_order?root=...&exp=...&strike=...&right=...&date=...
GET /v3/hist/option/greeks/second_order?root=...&exp=...&strike=...&right=...&date=...&ivl=...
GET /v3/hist/option/trade_greeks/second_order?root=...&exp=...&strike=...&right=...&date=...
GET /v3/hist/option/greeks/third_order?root=...&exp=...&strike=...&right=...&date=...&ivl=...
GET /v3/hist/option/trade_greeks/third_order?root=...&exp=...&strike=...&right=...&date=...
GET /v3/hist/option/greeks/implied_volatility?root=...&exp=...&strike=...&right=...&date=...&ivl=...
GET /v3/hist/option/trade_greeks/implied_volatility?root=...&exp=...&strike=...&right=...&date=...
GET /v3/at_time/option/trade?root=...&exp=...&strike=...&right=...&start_date=...&end_date=...&time_of_day=...
GET /v3/at_time/option/quote?root=...&exp=...&strike=...&right=...&start_date=...&end_date=...&time_of_day=...
```

### Index Routes (9)

```
GET /v3/list/roots/index
GET /v3/list/dates/index?symbol=...
GET /v3/snapshot/index/ohlc?root=...
GET /v3/snapshot/index/price?root=...
GET /v3/snapshot/index/market_value?root=...
GET /v3/hist/index/eod?root=...&start_date=...&end_date=...
GET /v3/hist/index/ohlc?root=...&start_date=...&end_date=...&ivl=...
GET /v3/hist/index/price?root=...&date=...&ivl=...
GET /v3/at_time/index/price?root=...&start_date=...&end_date=...&time_of_day=...
```

### Calendar Routes (3) + Rate Routes (1)

```
GET /v3/calendar/open_today
GET /v3/calendar/on_date?date=...
GET /v3/calendar/year?year=...
GET /v3/hist/rate/eod?root=...&start_date=...&end_date=...
```

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
