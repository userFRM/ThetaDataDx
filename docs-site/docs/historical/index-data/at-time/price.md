---
title: index_at_time_price
description: Index price at a specific time of day across a date range.
---

# index_at_time_price

<TierBadge tier="standard" />

Retrieve the index price at a specific time of day for every trading day in a date range. Returns one data point per date, useful for consistent daily sampling.

## Code Example

::: code-group
```rust [Rust]
let table: proto::DataTable = client.index_at_time_price(
    "SPX", "20240101", "20240301", "34200000"  // 9:30 AM ET
).await?;
```
```python [Python]
result = client.index_at_time_price("SPX", "20240101", "20240301", "34200000")
```
```go [Go]
atTime, err := client.IndexAtTimePrice("SPX", "20240101", "20240301", "34200000")
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto at_time = client.index_at_time_price("SPX", "20240101", "20240301", "34200000");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Index symbol (e.g. `"SPX"`) |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `time_of_day` | string | Yes | Milliseconds from midnight ET (e.g. `"34200000"` for 9:30 AM) |

## Response

Returns a `DataTable` with one entry per trading day:

| Field | Type | Description |
|-------|------|-------------|
| `price` | f64 | Index price/level at the specified time |
| `ms_of_day` | u32 | Actual milliseconds from midnight ET |
| `date` | u32 | Date as `YYYYMMDD` integer |

 - requires Standard plan or higher.

## Time Reference

| Time (ET) | Milliseconds |
|-----------|-------------|
| 9:30 AM | `34200000` |
| 12:00 PM | `43200000` |
| 4:00 PM | `57600000` |

## Notes

- Returns the price at or just before the specified time of day.
- Useful for building daily time series at a consistent sample point (e.g. market open, noon, close).
- Non-trading days are excluded from the response.
