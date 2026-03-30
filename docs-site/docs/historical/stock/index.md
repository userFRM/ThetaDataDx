---
title: Stock Endpoints
description: 14 stock data endpoints for listing symbols, snapshots, historical bars, tick data, and at-time queries.
---

# Stock Endpoints

ThetaDataDx provides 14 endpoints for US equity data: listing available symbols and dates, real-time snapshots, historical OHLC/trade/quote data, and point-in-time lookups.

## List

Discover what data is available before requesting it.

| Endpoint | Description |
|----------|-------------|
| [List Symbols](./list/symbols) | All available stock ticker symbols |
| [List Dates](./list/dates) | Available dates for a stock by request type |

## Snapshots

Latest market state for one or more symbols in a single call.

| Endpoint | Description |
|----------|-------------|
| [Snapshot OHLC](./snapshot/ohlc) | Current-day OHLC bar |
| [Snapshot Trade](./snapshot/trade) | Most recent trade |
| [Snapshot Quote](./snapshot/quote) | Current NBBO quote |
| [Snapshot Market Value](./snapshot/market-value) | Latest market value |

::: tip
Snapshot endpoints accept multiple symbols in a single call. Batch your requests to reduce round-trips.
:::

## History

Historical bars and tick-level data.

| Endpoint | Description |
|----------|-------------|
| [History EOD](./history/eod) | End-of-day OHLC + closing quote across a date range |
| [History OHLC](./history/ohlc) | Intraday OHLC bars (single date or date range) |
| [History Trade](./history/trade) | All trades for a date |
| [History Quote](./history/quote) | NBBO quotes at a configurable interval |
| [History Trade+Quote](./history/trade-quote) | Combined trade and prevailing quote ticks |

## At-Time

Point-in-time lookups across a date range.

| Endpoint | Description |
|----------|-------------|
| [At-Time Trade](./at-time/trade) | Trade at a specific time of day across dates |
| [At-Time Quote](./at-time/quote) | Quote at a specific time of day across dates |

## Streaming (Rust)

For endpoints returning millions of rows, the Rust SDK provides `_stream` variants that process data chunk by chunk without holding everything in memory:

```rust
tdx.stock_history_trade_stream("AAPL", "20240315", |chunk| {
    println!("Got {} trades in this chunk", chunk.len());
    Ok(())
}).await?;

tdx.stock_history_quote_stream("AAPL", "20240315", "0", |chunk| {
    println!("Got {} quotes in this chunk", chunk.len());
    Ok(())
}).await?;
```

::: tip
Use streaming variants when fetching tick-level data for active symbols. A single day of AAPL trades can exceed 100,000 rows.
:::
