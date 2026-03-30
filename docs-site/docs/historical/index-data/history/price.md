---
title: index_history_price
description: Intraday price history for an index.
---

# index_history_price

<TierBadge tier="standard" />

Retrieve intraday price history for an index on a single date at a specified interval. Returns raw price data as a DataTable.

## Code Example

::: code-group
```rust [Rust]
let table: proto::DataTable = client.index_history_price("SPX", "20240315", "60000").await?;
```
```python [Python]
price = client.index_history_price("SPX", "20240315", "60000")
```
```go [Go]
priceHist, err := client.IndexHistoryPrice("SPX", "20240315", "60000")
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto price_hist = client.index_history_price("SPX", "20240315", "60000");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Index symbol (e.g. `"SPX"`) |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `interval` | string | Yes | Sampling interval in milliseconds |
| `start_time` | string | No | Start time of day (ms from midnight) |
| `end_time` | string | No | End time of day (ms from midnight) |

## Response

Returns a `DataTable` with price and time fields:

| Field | Type | Description |
|-------|------|-------------|
| `price` | f64 | Index price/level |
| `ms_of_day` | u32 | Milliseconds from midnight ET |
| `date` | u32 | Date as `YYYYMMDD` integer |

 - requires Standard plan or higher.

## Notes

- Returns raw `DataTable` (protobuf) rather than typed ticks.
- For OHLC-structured data across a date range, use [index_history_ohlc](./ohlc) instead.
- Operates on a single date only. For multi-day queries, use [index_history_eod](./eod) or [index_history_ohlc](./ohlc).
