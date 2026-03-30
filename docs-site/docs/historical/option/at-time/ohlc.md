---
title: option_at_time_ohlc
description: Quote at a specific time of day across a date range for an option contract.
---

# option_at_time_ohlc

Retrieve the NBBO quote at a specific time of day across a date range for an option contract. Returns one quote per date, the prevailing quote at the specified time.

## Code Example

::: code-group
```rust [Rust]
let quotes: Vec<QuoteTick> = client.option_at_time_quote(
    "SPY", "20241220", "500000", "C",
    "20240101", "20240301", "34200000"  // 9:30 AM ET
).await?;
```
```python [Python]
quotes = client.option_at_time_quote("SPY", "20241220", "500000", "C",
                                     "20240101", "20240301", "34200000")
```
```go [Go]
quotes, err := client.OptionAtTimeQuote("SPY", "20241220", "500000", "C",
    "20240101", "20240301", "34200000")
```
```cpp [C++]
auto quotes = client.option_at_time_quote("SPY", "20241220", "500000", "C",
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
| `bid_price` | float | Best bid price |
| `bid_size` | int | Bid size |
| `ask_price` | float | Best ask price |
| `ask_size` | int | Ask size |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |
| `bid_exchange` | int | Bid exchange code |
| `ask_exchange` | int | Ask exchange code |

## Subscription Tier

Free

## Notes

- Common time values: `"34200000"` (9:30 AM), `"46800000"` (1:00 PM), `"57600000"` (4:00 PM).
- Useful for building daily spread or mid-price time series at a consistent intraday timestamp.
