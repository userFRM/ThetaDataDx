---
title: option_snapshot_ohlc
description: Latest OHLC snapshot for an option contract.
---

# option_snapshot_ohlc

Get the latest OHLC (open, high, low, close) snapshot for an option contract.

## Code Example

::: code-group
```rust [Rust]
let bars = client.option_snapshot_ohlc("SPY", "20241220", "500000", "C").await?;
```
```python [Python]
bars = client.option_snapshot_ohlc("SPY", "20241220", "500000", "C")
```
```go [Go]
bars, err := client.OptionSnapshotOHLC("SPY", "20241220", "500000", "C")
```
```cpp [C++]
auto bars = client.option_snapshot_ohlc("SPY", "20241220", "500000", "C");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer, e.g. `"500000"` for $500) |
| `right` | string | Yes | `"C"` for call, `"P"` for put |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `open` | float | Opening price |
| `high` | float | High price |
| `low` | float | Low price |
| `close` | float | Closing price |
| `volume` | int | Volume |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |

## Subscription Tier

Free
