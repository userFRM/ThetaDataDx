---
title: Snapshot Market Value
description: Latest market value snapshot for one or more stocks.
---

# stock_snapshot_market_value

Latest market value snapshot for one or more stocks.

**Tier:** Value+

## Code Example

::: code-group
```rust [Rust]
let mv: proto::DataTable = tdx.stock_snapshot_market_value(&["AAPL"]).await?;
```
```python [Python]
mv = tdx.stock_snapshot_market_value(["AAPL"])
```
```go [Go]
mv, err := client.StockSnapshotMarketValue([]string{"AAPL"})
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto mv = client.stock_snapshot_market_value({"AAPL"});
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbols` | string[] | Yes | One or more ticker symbols |
| `venue` | string | No | Data venue filter |
| `min_time` | string | No | Minimum time of day (ms from midnight ET) |

## Response

DataTable with market value fields. The exact fields depend on the data available for the requested symbols.

## Notes

- Accepts multiple symbols in a single call.
- Returns raw DataTable format rather than a typed tick structure.
