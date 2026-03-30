---
title: option_history_quote
description: NBBO quotes for an option contract at a given interval.
---

# option_history_quote

Retrieve NBBO quotes for an option contract, sampled at a specified interval.

## Code Example

::: code-group
```rust [Rust]
let quotes: Vec<QuoteTick> = client.option_history_quote(
    "SPY", "20241220", "500000", "C", "20240315", "60000"
).await?;
```
```python [Python]
quotes = client.option_history_quote("SPY", "20241220", "500000", "C",
                                     "20240315", "60000")
```
```go [Go]
quotes, err := client.OptionHistoryQuote("SPY", "20241220", "500000", "C",
    "20240315", "60000")
```
```cpp [C++]
auto quotes = client.option_history_quote("SPY", "20241220", "500000", "C",
                                           "20240315", "60000");
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
| `interval` | string | Yes | Sampling interval in ms (`"0"` for every change) |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `bid_price` | float | Best bid price |
| `bid_size` | int | Bid size |
| `ask_price` | float | Best ask price |
| `ask_size` | int | Ask size |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |
| `bid_exchange` | int | Bid exchange code |
| `ask_exchange` | int | Ask exchange code |

## Subscription Tier

Standard+

## Notes

- Use `"0"` as the interval to get every quote change (tick-by-tick).
- For liquid contracts with `"0"` interval, the response can be very large. In Rust, use the `_stream` variant.
