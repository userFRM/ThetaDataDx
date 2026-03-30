---
title: option_history_open_interest
description: Open interest history for an option contract.
---

# option_history_open_interest

Retrieve open interest history for an option contract.

## Code Example

::: code-group
```rust [Rust]
let oi = client.option_history_open_interest(
    "SPY", "20241220", "500000", "C", "20240315"
).await?;
```
```python [Python]
oi = client.option_history_open_interest("SPY", "20241220", "500000", "C", "20240315")
```
```go [Go]
oi, err := client.OptionHistoryOpenInterest("SPY", "20241220", "500000", "C", "20240315")
```
```cpp [C++]
auto oi = client.option_history_open_interest("SPY", "20241220", "500000", "C", "20240315");
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
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `open_interest` | int | Open interest |
| `date` | string | Date |

## Subscription Tier

Free

## Notes

- Open interest is typically reported once per day based on the previous day's settlement.
