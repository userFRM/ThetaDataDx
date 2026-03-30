---
title: option_snapshot_quote
description: Latest NBBO quote snapshot for an option contract.
---

# option_snapshot_quote

Get the latest NBBO (National Best Bid and Offer) quote snapshot for an option contract.

## Code Example

::: code-group
```rust [Rust]
let quotes = client.option_snapshot_quote("SPY", "20241220", "500000", "C").await?;
```
```python [Python]
quotes = client.option_snapshot_quote("SPY", "20241220", "500000", "C")
```
```go [Go]
quotes, err := client.OptionSnapshotQuote("SPY", "20241220", "500000", "C")
```
```cpp [C++]
auto quotes = client.option_snapshot_quote("SPY", "20241220", "500000", "C");
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
