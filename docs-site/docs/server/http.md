---
title: Server — HTTP API
description: The v3 REST route surface, response formats, and the error envelope.
---

# HTTP API

Every [reference endpoint](/reference/) is served as a `GET /v3/...` route on `http://127.0.0.1:25503`. The machine-readable contract is the bundled [OpenAPI file](/thetadatadx.yaml); the human-readable contract is each reference page's HTTP tab.

```bash
# Stock EOD across a date range
curl 'http://127.0.0.1:25503/v3/stock/history/eod?symbol=AAPL&start_date=2025-03-03&end_date=2025-03-06'

# Option quote snapshot for one contract
curl 'http://127.0.0.1:25503/v3/option/snapshot/quote?symbol=SPY&expiration=2025-03-21&strike=570&right=C'

# Intraday OHLC bars across a range
curl 'http://127.0.0.1:25503/v3/stock/history/ohlc_range?symbol=AAPL&start_date=2025-03-03&end_date=2025-03-06&interval=1m'

# Trading calendar
curl 'http://127.0.0.1:25503/v3/calendar/open_today'
```

## Parameters

Query parameters use the registry names shown on each reference page — `symbol`, `expiration`, `strike`, `right`, `interval`, `start_date`, `end_date`, … Date parameters accept both `YYYYMMDD` and ISO `YYYY-MM-DD`. Strikes are dollars (`570`, `17.5`); rights accept `C` / `P` / `call` / `put`.

## Response formats

Add `format=` to any route:

| `format` | Content type | Shape |
|---|---|---|
| `json` (default) | `application/json` | The envelope below. |
| `csv` | `text/csv` | RFC 4180 with a header row. |
| `ndjson` / `jsonl` | `application/x-ndjson` | One JSON object per row, newline-delimited. |

The JSON envelope:

```json
{
    "header": { "format": "json", "error_type": "null" },
    "response": [ ... ]
}
```

Failures use one envelope shape across every route — see the [error codes table](/articles/error-codes#server-error-envelope).

## Flat files

Whole-universe daily archives stream through dedicated routes with chunked bodies (bounded server memory at any download size):

```bash
curl 'http://127.0.0.1:25503/v3/flatfile/option/trade_quote?date=20250303&format=csv' -o trade_quotes.csv

curl -X POST 'http://127.0.0.1:25503/v3/flatfile/request' \
    -H 'Content-Type: application/json' \
    -d '{"sec_type":"OPTION","req_type":"TRADE_QUOTE","date":"20250303","format":"csv"}' \
    -o trade_quotes.csv
```

See [Flat Files](/articles/flat-files) for sizing guidance.

## Terminal system routes

Mirrored 1:1 from the JVM terminal — unauthenticated `GET`, bare `text/plain` bodies.

| Method | Route | Description |
|---|---|---|
| `GET` | `/v3/terminal/shutdown` | Kills the server process; returns the plain text `OK`. |
| `GET` | `/v3/terminal/fpss/status` | Streaming-channel health: `CONNECTED` / `DISCONNECTED`. |
| `GET` | `/v3/terminal/mdds/status` | Historical-channel health: `CONNECTED` / `DISCONNECTED`. |

## Concurrency behavior

Bursts queue, they don't fail. In-flight requests are capped at the HTTP edge (256) and by your [subscription tier's concurrency](/articles/concurrent-requests) inside the SDK; requests beyond either cap wait in order. Only genuine upstream exhaustion — after the SDK's own retries — surfaces as `503` with `Retry-After`. A client that disconnects or times out releases its slots immediately.
