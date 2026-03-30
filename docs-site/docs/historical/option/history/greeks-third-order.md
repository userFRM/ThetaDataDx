---
title: option_history_greeks_third_order
description: Third-order Greeks history at a given interval.
---

# option_history_greeks_third_order

Retrieve third-order Greeks (speed, zomma, color, ultima) sampled at a given interval throughout a trading day.

## Code Example

::: code-group
```rust [Rust]
let g = client.option_history_greeks_third_order(
    "SPY", "20241220", "500000", "C", "20240315", "60000"
).await?;
```
```python [Python]
g = client.option_history_greeks_third_order("SPY", "20241220", "500000", "C",
                                              "20240315", "60000")
```
```go [Go]
g, err := client.OptionHistoryGreeksThirdOrder("SPY", "20241220", "500000", "C",
    "20240315", "60000")
```
```cpp [C++]
auto g = client.option_history_greeks_third_order("SPY", "20241220", "500000", "C",
                                                    "20240315", "60000");
```
:::

## Parameters

Parameters are identical to [option_history_greeks_all](./greeks-all#parameters).

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer) |
| `right` | string | Yes | `"C"` or `"P"` |
| `date` | string | Yes | Date (`YYYYMMDD`) |
| `interval` | string | Yes | Sampling interval in ms |
| `annual_dividend` | float | No | Override annual dividend |
| `rate_type` | string | No | Interest rate type |
| `rate_value` | float | No | Override interest rate value |
| `version` | string | No | Greeks calculation version |
| `strike_range` | int | No | Strike range filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `implied_volatility` | float | Implied volatility |
| `speed` | float | Rate of change of gamma w.r.t. underlying price |
| `zomma` | float | Rate of change of gamma w.r.t. volatility |
| `color` | float | Rate of change of gamma w.r.t. time |
| `ultima` | float | Rate of change of vomma w.r.t. volatility |
| `underlying_price` | float | Underlying price |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |

## Subscription Tier

Pro
