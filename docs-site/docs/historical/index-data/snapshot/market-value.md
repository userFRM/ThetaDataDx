---
title: index_snapshot_market_value
description: Latest market value snapshot for one or more indices.
---

# index_snapshot_market_value

<TierBadge tier="value" />

Get the latest market value snapshot for one or more index symbols.

## Code Example

::: code-group
```rust [Rust]
let table: proto::DataTable = client.index_snapshot_market_value(&["SPX"]).await?;
```
```python [Python]
mv = client.index_snapshot_market_value(["SPX"])
```
```go [Go]
mv, err := client.IndexSnapshotMarketValue([]string{"SPX"})
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto mv = client.index_snapshot_market_value({"SPX"});
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more index symbols |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

## Response

Returns a `DataTable` with market value fields:

| Field | Type | Description |
|-------|------|-------------|
| `market_value` | f64 | Market capitalization / value |
| `ms_of_day` | u32 | Milliseconds from midnight ET |
| `date` | u32 | Date as `YYYYMMDD` integer |

 - requires Value plan or higher.

## Notes

- Returns raw `DataTable` (protobuf) rather than typed ticks.
- Market value represents the total capitalization of the index constituents.
