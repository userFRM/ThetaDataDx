---
title: option_history_trade
description: All trades for an option contract on a given date.
---

# option_history_trade

Retrieve all individual trades for an option contract on a given date.

## Code Example

::: code-group
```rust [Rust]
let trades: Vec<TradeTick> = client.option_history_trade(
    "SPY", "20241220", "500000", "C", "20240315"
).await?;
```
```python [Python]
trades = client.option_history_trade("SPY", "20241220", "500000", "C", "20240315")
```
```go [Go]
trades, err := client.OptionHistoryTrade("SPY", "20241220", "500000", "C", "20240315")
```
```cpp [C++]
auto trades = client.option_history_trade("SPY", "20241220", "500000", "C", "20240315");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer) |
| `right` | string | Yes | `"C"` or `"P"` |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `price` | float | Trade price |
| `size` | int | Trade size (number of contracts) |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |
| `sequence` | int | Sequence number |
| `condition` | int | Trade condition code |
| `exchange` | int | Exchange code |

## Subscription Tier

Standard+

## Notes

- For liquid contracts, this can return hundreds of thousands of rows. In Rust, use the `_stream` variant to process in chunks.
