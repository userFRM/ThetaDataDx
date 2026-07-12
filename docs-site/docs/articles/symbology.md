---
title: Symbology & Contract Identity
description: How symbols, option contracts, dates, prices, and timestamps are written across the SDK surface.
---

# Symbology & Contract Identity

## Symbols

A symbol identifies a stock, index, or option underlying: `AAPL`, `SPY`, `SPX`. Snapshot endpoints accept multiple symbols at once (a list in the SDKs, comma-separated over HTTP). Index option underlyings are distinct symbols per settlement style — `SPX` (AM-settled monthlies) and `SPXW` (PM-settled weeklies) are different roots; the same split applies to other index families (`VIX`/`VIXW`, `NDX`/`NDXP`). List the roots your account can see with the [option symbols](/reference/option/list/symbols) endpoint.

## Option contract identity

An option contract is always the four-tuple **(symbol, expiration, strike, right)**:

| Part | Form | Example |
|---|---|---|
| `symbol` | underlying root | `SPXW` |
| `expiration` | `YYYYMMDD` date | `20260618` |
| `strike` | dollars, as a string | `"5400"` or `"17.5"` |
| `right` | `C` / `P` (`call` / `put` accepted) | `C` |

Strikes are **human dollars everywhere on the SDK surface** — never scaled integers. Endpoints that take an optional `strike` / `right` treat omission as a wildcard: all strikes, both rights. Wildcard responses identify each row's contract via the `expiration`, `strike`, and `right` response fields.

Strikes are dollars across every surface, including the bundled server's [WebSocket subscribe envelope](/server/websocket) (`570` = $570.00). The scaled-integer form only exists on the raw upstream wire, which the SDK and the server both hide.

## Dates and times in

- Dates are `YYYYMMDD` strings (`"20250303"`). The HTTP server also accepts ISO `YYYY-MM-DD`.
- Date ranges are inclusive on both ends.
- Time-of-day inputs (`start_time`, `end_time`, `time_of_day`) are Eastern Time wall-clock `HH:MM:SS` (at-time endpoints take milliseconds: `HH:MM:SS.SSS`).
- `interval` is one of the presets: `tick`, `10ms`, `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`.

## Timestamps out

Response rows carry two integer time fields:

- `date` — the trading date as a `YYYYMMDD` integer.
- `ms_of_day` — milliseconds since midnight Eastern Time (`34200000` = 09:30:00 ET).

Integer timestamps compare and bucket cheaply in hot loops; convert to wall-clock datetimes only at display boundaries.
