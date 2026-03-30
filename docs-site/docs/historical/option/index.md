---
title: Option Endpoints
description: 34 option data endpoints - list, snapshots, history, Greeks, trade Greeks, and at-time queries.
---

# Option Endpoints

ThetaDataDx provides 34 typed endpoints for option market data, organized into five categories.

## Contract Identification

Option contracts are identified by four parameters:

| Parameter | Description | Example |
|-----------|-------------|---------|
| `symbol` | Underlying ticker | `"SPY"` |
| `expiration` | Expiration date (`YYYYMMDD`) | `"20241220"` |
| `strike` | Strike price in tenths of a cent | `"500000"` ($500.00) |
| `right` | Call or put | `"C"` or `"P"` |

## Endpoint Categories

### [List](./list/roots) (5 endpoints)

Discover available symbols, expirations, strikes, dates, and contracts.

- [List Roots](./list/roots) - all option underlying symbols
- [List Dates](./list/dates) - available dates for a contract
- [List Strikes](./list/strikes) - strike prices for an expiration
- [List Expirations](./list/expirations) - expiration dates for an underlying
- [List Contracts](./list/contracts) - all contracts for a symbol on a date

### [Snapshot](./snapshot/ohlc) (9 endpoints)

Latest point-in-time data for a contract.

- [Snapshot OHLC](./snapshot/ohlc)
- [Snapshot Trade](./snapshot/trade)
- [Snapshot Quote](./snapshot/quote)
- [Snapshot Open Interest](./snapshot/open-interest)
- [Snapshot Greeks IV](./snapshot/greeks-iv)
- [Snapshot Greeks All](./snapshot/greeks-all)
- [Snapshot Greeks First Order](./snapshot/greeks-first-order)
- [Snapshot Greeks Second Order](./snapshot/greeks-second-order)
- [Snapshot Greeks Third Order](./snapshot/greeks-third-order)

### [History](./history/eod) (16 endpoints)

Historical time series data for a contract.

- [History EOD](./history/eod)
- [History OHLC](./history/ohlc)
- [History Trade](./history/trade)
- [History Quote](./history/quote)
- [History Trade + Quote](./history/trade-quote)
- [History Open Interest](./history/open-interest)
- [History Greeks EOD](./history/greeks-eod)
- [History Greeks All](./history/greeks-all)
- [History Greeks First Order](./history/greeks-first-order)
- [History Greeks Second Order](./history/greeks-second-order)
- [History Greeks Third Order](./history/greeks-third-order)
- [History Greeks IV](./history/greeks-iv)
- [History Trade Greeks All](./history/trade-greeks-all)
- [History Trade Greeks First Order](./history/trade-greeks-first-order)
- [History Trade Greeks Second Order](./history/trade-greeks-second-order)
- [History Trade Greeks Third Order](./history/trade-greeks-third-order)
- [History Trade Greeks IV](./history/trade-greeks-iv)

### [At-Time](./at-time/trade) (2 endpoints)

Data at a specific time of day across a date range.

- [At-Time Trade](./at-time/trade)
- [At-Time OHLC](./at-time/ohlc)

## Streaming (Rust only)

For endpoints returning millions of rows, the Rust SDK provides `_stream` variants:

```rust
client.option_history_trade_stream(
    "SPY", "20240419", "500000", "C", "20240315",
    |chunk| { Ok(()) }
).await?;

client.option_history_quote_stream(
    "SPY", "20240419", "500000", "C", "20240315", "0",
    |chunk| { Ok(()) }
).await?;
```
