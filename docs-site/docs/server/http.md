---
title: "Server: HTTP API"
description: The v3 REST route surface, response formats, and the error envelope.
---

# HTTP API

Every [reference endpoint](/reference/) is served as a `GET /v3/...` route on `http://127.0.0.1:25503`. The machine-readable contract is the bundled [OpenAPI file](/thetadatadx.yaml); the human-readable contract is each reference page's HTTP tab.

```bash
# Stock EOD across a date range
curl 'http://127.0.0.1:25503/v3/stock/history/eod?symbol=AAPL&start_date=2025-03-03&end_date=2025-03-06'

# Option quote snapshot for one contract
curl 'http://127.0.0.1:25503/v3/option/snapshot/quote?symbol=SPY&expiration=2025-03-21&strike=570&right=C'

# Intraday OHLC bars across a range (same route as single-date; supply start_date/end_date instead of date)
curl 'http://127.0.0.1:25503/v3/stock/history/ohlc?symbol=AAPL&start_date=2025-03-03&end_date=2025-03-06&interval=1m'

# Trading calendar
curl 'http://127.0.0.1:25503/v3/calendar/open_today'
```

## Parameters

Query parameters use the registry names shown on each reference page: `symbol`, `expiration`, `strike`, `right`, `interval`, `start_date`, `end_date`, … Date parameters accept both `YYYYMMDD` and ISO `YYYY-MM-DD`. Strikes are dollars (`570`, `17.5`); rights accept `C` / `P` / `call` / `put`.

## Response formats

Add `format=` to any route:

| `format` | Content type | Shape |
|---|---|---|
| `csv` (default) | `text/csv` | RFC 4180 with a header row. |
| `json` / `json_new` | `application/json` | The envelope below. |
| `json_legacy` | `application/json` | One array per column, keyed by column name. |
| `ndjson` / `jsonl` | `application/x-ndjson` | One JSON object per row, newline-delimited. |
| `html` | `text/html` | A browser-viewable `<table>`, served inline. |

The JSON envelope:

```json
{
    "response": [ ... ]
}
```

Option market-data routes group the rows by contract, one block per contract in the order the contracts arrived:

```json
{
    "response": [
        {
            "contract": { "symbol": "SPY", "expiration": "2025-03-21", "strike": 570.000, "right": "CALL" },
            "data": [ ... ]
        }
    ]
}
```

The CSV and NDJSON forms carry the contract columns inline on every row instead.

Each price is written at the precision the vendor sent it with, in every format, as the terminal writes it: a strike reads `570.000` and a greek `0.5800`.

Each response is written as its rows arrive from the vendor, as the terminal writes it. A response that ends within 8 KiB is sent whole, with its `Content-Length`; a longer one is sent with chunked transfer encoding from the moment it outgrows that, so its first rows leave before its last arrive and server memory stays flat at any response size. `json_legacy` is one array per column, so no column is complete before the last row arrives, and it is held whole until then. A failure after a response has begun aborts the transfer, so a truncated response cannot pass for a complete one.

A failed request is answered with an HTTP status and a plain-text description; see [server errors](/articles/error-codes#server-errors).

## Flat files

The `/v3/{option,stock,index}/flat_file/{dataset}` routes answer the way the terminal does: the file's columns under the terminal's names, text and prices quoted in CSV, each chunk written as it arrives so the first bytes leave at once and server memory stays flat at any file size. They take the same `format` values as every other route; `json_legacy`, one array per column, is the exception to flat memory, since no column is complete before the last row arrives:

```bash
curl 'http://127.0.0.1:25503/v3/option/flat_file/trade_quote?date=20261021&format=csv' -o trade_quotes.csv
```

See [Flat Files](/articles/flat-files) for the datasets and the format.

## Terminal system routes

Mirrored 1:1 from the JVM terminal: unauthenticated `GET`, bare `text/plain` bodies.

| Method | Route | Description |
|---|---|---|
| `GET` | `/v3/terminal/shutdown` | Kills the server process; returns the plain text `OK`. |
| `GET` | `/v3/terminal/fpss/status` | Streaming-channel health: `CONNECTED` / `DISCONNECTED`. |
| `GET` | `/v3/terminal/mdds/status` | Market-data-channel health: `CONNECTED` / `DISCONNECTED`. |

## Concurrency behavior

Bursts queue, they don't fail. The HTTP edge admits up to 256 simultaneous requests; beyond that they queue and are served in order, and a client that disconnects or times out frees its place immediately. Genuine upstream exhaustion surfaces as `503` with `Retry-After`.
