---
title: option_history_trade_quote
description: Combined trade and quote ticks for an option contract.
---

# option_history_trade_quote

Retrieve combined trade + quote ticks for an option contract on a given date. Each row contains both the trade data and the prevailing quote at the time of the trade.

## Code Example

::: code-group
```rust [Rust]
let tq = client.option_history_trade_quote(
    "SPY", "20241220", "500000", "C", "20240315"
).await?;
```
```python [Python]
tq = client.option_history_trade_quote("SPY", "20241220", "500000", "C", "20240315")
```
```go [Go]
tq, err := client.OptionHistoryTradeQuote("SPY", "20241220", "500000", "C", "20240315")
```
```cpp [C++]
auto tq = client.option_history_trade_quote("SPY", "20241220", "500000", "C", "20240315");
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
| `exclusive` | bool | No | Exclusive time bounds |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `price` | float | Trade price |
| `size` | int | Trade size |
| `condition` | int | Trade condition code |
| `exchange` | int | Trade exchange code |
| `bid_price` | float | Prevailing bid at time of trade |
| `bid_size` | int | Prevailing bid size |
| `ask_price` | float | Prevailing ask at time of trade |
| `ask_size` | int | Prevailing ask size |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |

## Subscription Tier

Pro

## Notes

- Useful for trade classification (e.g., determining if a trade hit the bid or lifted the offer).
