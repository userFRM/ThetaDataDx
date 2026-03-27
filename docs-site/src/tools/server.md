# REST Server

Drop-in replacement for the ThetaData Java Terminal. Runs a local HTTP REST server and WebSocket server with identical API compatibility. Existing Python SDK scripts, Excel add-ins, and curl commands work without changes.

## Installation

```bash
cargo install --path crates/thetadatadx-server
```

## Quick Start

```bash
thetadatadx-server --creds creds.txt
```

This starts:
- **HTTP REST API** on `http://127.0.0.1:25510` (all `/v2/...` routes)
- **WebSocket server** on `ws://127.0.0.1:25520/v1/events`

## CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--creds <path>` | `creds.txt` | Credentials file |
| `--http-port <port>` | `25510` | HTTP REST API port |
| `--ws-port <port>` | `25520` | WebSocket server port |
| `--bind <addr>` | `127.0.0.1` | Bind address |

## REST API

All 61 endpoints are exposed as HTTP routes following the Java terminal's URL patterns.

```bash
# Stock EOD
curl "http://127.0.0.1:25510/v2/hist/stock/eod?root=AAPL&start_date=20240101&end_date=20240301"

# Option snapshot
curl "http://127.0.0.1:25510/v2/snapshot/option/quote?root=SPY&exp=20240419&strike=500000&right=C"

# Calendar
curl "http://127.0.0.1:25510/v2/calendar/open_today"
```

Response envelope matches the Java terminal:

```json
{
    "header": { "format": "json", "error_type": "null" },
    "response": [ ... ]
}
```

## WebSocket API

The WebSocket server at `/v1/events` replicates the Java terminal's streaming protocol:

- Single client at a time
- STATUS heartbeat every second
- Event types: QUOTE, TRADE, OHLC, OPEN_INTEREST, STATUS

## System Routes

| Route | Description |
|-------|-------------|
| `/v2/system/mdds/status` | MDDS connection status |
| `/v2/system/fpss/status` | FPSS connection status |
| `/v2/system/shutdown` | Graceful shutdown |
