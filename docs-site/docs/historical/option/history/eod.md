---
title: option_history_eod
description: End-of-day option data across a date range.
---

# option_history_eod

Retrieve end-of-day option data across a date range. Returns one row per trading day with OHLC, volume, and open interest.

## Code Example

::: code-group
```rust [Rust]
let eod: Vec<EodTick> = client.option_history_eod(
    "SPY", "20241220", "500000", "C", "20240101", "20240301"
).await?;
for t in &eod {
    println!("{}: O={} H={} L={} C={}", t.date, t.open_price(), t.high_price(),
        t.low_price(), t.close_price());
}
```
```python [Python]
eod = client.option_history_eod("SPY", "20241220", "500000", "C",
                                "20240101", "20240301")
```
```go [Go]
eod, err := client.OptionHistoryEOD("SPY", "20241220", "500000", "C",
    "20240101", "20240301")
```
```cpp [C++]
auto eod = client.option_history_eod("SPY", "20241220", "500000", "C",
                                      "20240101", "20240301");
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
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `date` | string | Trading date |
| `open` | float | Opening price |
| `high` | float | High price |
| `low` | float | Low price |
| `close` | float | Closing price |
| `volume` | int | Daily volume |
| `open_interest` | int | Open interest |

## Subscription Tier

Free
