---
title: option_history_ohlc
description: Intraday OHLC bars for an option contract.
---

# option_history_ohlc

Retrieve intraday OHLC bars for an option contract on a given date at a specified interval.

## Code Example

::: code-group
```rust [Rust]
let bars: Vec<OhlcTick> = client.option_history_ohlc(
    "SPY", "20241220", "500000", "C", "20240315", "60000"
).await?;
```
```python [Python]
bars = client.option_history_ohlc("SPY", "20241220", "500000", "C",
                                  "20240315", "60000")
```
```go [Go]
bars, err := client.OptionHistoryOHLC("SPY", "20241220", "500000", "C",
    "20240315", "60000")
```
```cpp [C++]
auto bars = client.option_history_ohlc("SPY", "20241220", "500000", "C",
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
| `interval` | string | Yes | Bar interval in milliseconds (e.g. `"60000"` for 1 min) |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `strike_range` | int | No | Strike range filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `open` | float | Opening price |
| `high` | float | High price |
| `low` | float | Low price |
| `close` | float | Closing price |
| `volume` | int | Volume in interval |
| `count` | int | Number of trades |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |

## Subscription Tier

Free

## Notes

- Common intervals: `"60000"` (1 min), `"300000"` (5 min), `"900000"` (15 min), `"3600000"` (1 hour).
