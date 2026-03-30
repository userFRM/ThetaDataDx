---
title: index_snapshot_price
description: Latest price snapshot for one or more indices.
---

# index_snapshot_price

Get the latest price snapshot for one or more index symbols. Returns the most recent price data as a raw DataTable.

## Code Example

::: code-group
```rust [Rust]
let table: proto::DataTable = client.index_snapshot_price(&["SPX", "NDX"]).await?;
```
```python [Python]
price = client.index_snapshot_price(["SPX", "NDX"])
```
```go [Go]
price, err := client.IndexSnapshotPrice([]string{"SPX"})
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto price = client.index_snapshot_price({"SPX"});
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more index symbols |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

## Response

Returns a `DataTable` with price fields:

| Field | Type | Description |
|-------|------|-------------|
| `price` | f64 | Current index price/level |
| `ms_of_day` | u32 | Milliseconds from midnight ET |
| `date` | u32 | Date as `YYYYMMDD` integer |

## Subscription Tier

Value -- requires Value plan or higher.

## Notes

- Returns raw `DataTable` (protobuf) rather than typed ticks.
- For OHLC-structured data, use [index_snapshot_ohlc](./ohlc) instead.
