---
title: option_snapshot_trade
description: Latest trade snapshot for an option contract.
---

# option_snapshot_trade

Get the latest trade snapshot for an option contract.

## Code Example

::: code-group
```rust [Rust]
let trades = client.option_snapshot_trade("SPY", "20241220", "500000", "C").await?;
```
```python [Python]
trades = client.option_snapshot_trade("SPY", "20241220", "500000", "C")
```
```go [Go]
trades, err := client.OptionSnapshotTrade("SPY", "20241220", "500000", "C")
```
```cpp [C++]
auto trades = client.option_snapshot_trade("SPY", "20241220", "500000", "C");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer) |
| `right` | string | Yes | `"C"` or `"P"` |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `price` | float | Trade price |
| `size` | int | Trade size (number of contracts) |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |
| `condition` | int | Trade condition code |
| `exchange` | int | Exchange code |

## Subscription Tier

Free
