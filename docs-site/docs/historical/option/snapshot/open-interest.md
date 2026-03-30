---
title: option_snapshot_open_interest
description: Latest open interest snapshot for an option contract.
---

# option_snapshot_open_interest

Get the latest open interest snapshot for an option contract.

## Code Example

::: code-group
```rust [Rust]
let oi = client.option_snapshot_open_interest("SPY", "20241220", "500000", "C").await?;
```
```python [Python]
oi = client.option_snapshot_open_interest("SPY", "20241220", "500000", "C")
```
```go [Go]
oi, err := client.OptionSnapshotOpenInterest("SPY", "20241220", "500000", "C")
```
```cpp [C++]
auto oi = client.option_snapshot_open_interest("SPY", "20241220", "500000", "C");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer) |
| `right` | string | Yes | `"C"` or `"P"` |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `open_interest` | int | Current open interest |
| `date` | string | Date |

## Subscription Tier

Free

## Notes

- Open interest is reported once per day, typically reflecting the previous day's settlement.
