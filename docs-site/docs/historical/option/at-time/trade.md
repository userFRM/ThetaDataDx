---
title: option_at_time_trade
description: Trade at a specific time of day across a date range for an option contract.
---

# option_at_time_trade

Retrieve the trade at a specific time of day across a date range for an option contract. Returns one trade per date, the most recent trade at or before the specified time.

## Code Example

::: code-group
```rust [Rust]
let trades: Vec<TradeTick> = client.option_at_time_trade(
    "SPY", "20241220", "500000", "C",
    "20240101", "20240301", "34200000"  // 9:30 AM ET
).await?;
```
```python [Python]
trades = client.option_at_time_trade("SPY", "20241220", "500000", "C",
                                     "20240101", "20240301", "34200000")
```
```go [Go]
trades, err := client.OptionAtTimeTrade("SPY", "20241220", "500000", "C",
    "20240101", "20240301", "34200000")
```
```cpp [C++]
auto trades = client.option_at_time_trade("SPY", "20241220", "500000", "C",
                                           "20240101", "20240301", "34200000");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer) |
| `right` | string | Yes | `"C"` or `"P"` |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `time_of_day` | string | Yes | Milliseconds from midnight ET (e.g. `"34200000"` = 9:30 AM) |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `price` | float | Trade price |
| `size` | int | Trade size |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |
| `condition` | int | Trade condition code |
| `exchange` | int | Exchange code |

## Subscription Tier

Free

## Notes

- Common time values: `"34200000"` (9:30 AM), `"46800000"` (1:00 PM), `"57600000"` (4:00 PM).
- Useful for building daily time series at a consistent intraday timestamp (e.g., opening trade every day).
