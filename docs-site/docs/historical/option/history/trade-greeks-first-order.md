---
title: option_history_trade_greeks_first_order
description: First-order Greeks computed on each individual trade.
---

# option_history_trade_greeks_first_order

Retrieve first-order Greeks computed on each individual trade for an option contract.

## Code Example

::: code-group
```rust [Rust]
let g = client.option_history_trade_greeks_first_order(
    "SPY", "20241220", "500000", "C", "20240315"
).await?;
```
```python [Python]
g = client.option_history_trade_greeks_first_order("SPY", "20241220", "500000", "C", "20240315")
```
```go [Go]
g, err := client.OptionHistoryTradeGreeksFirstOrder("SPY", "20241220", "500000", "C", "20240315")
```
```cpp [C++]
auto g = client.option_history_trade_greeks_first_order("SPY", "20241220", "500000", "C",
                                                          "20240315");
```
:::

## Parameters

Parameters are identical to [option_history_trade_greeks_all](./trade-greeks-all#parameters).

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer) |
| `right` | string | Yes | `"C"` or `"P"` |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `start_time` | string | No | Start time (ms from midnight) |
| `end_time` | string | No | End time (ms from midnight) |
| `annual_dividend` | float | No | Override annual dividend |
| `rate_type` | string | No | Interest rate type |
| `rate_value` | float | No | Override interest rate value |
| `version` | string | No | Greeks calculation version |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `price` | float | Trade price |
| `size` | int | Trade size |
| `condition` | int | Trade condition code |
| `exchange` | int | Exchange code |
| `implied_volatility` | float | IV at time of trade |
| `delta` | float | Delta |
| `theta` | float | Theta |
| `vega` | float | Vega |
| `rho` | float | Rho |
| `epsilon` | float | Epsilon |
| `lambda` | float | Lambda |
| `underlying_price` | float | Underlying price at time of trade |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |

## Subscription Tier

Pro
