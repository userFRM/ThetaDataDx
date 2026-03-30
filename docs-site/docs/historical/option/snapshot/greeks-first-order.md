---
title: option_snapshot_greeks_first_order
description: First-order Greeks snapshot for an option contract.
---

# option_snapshot_greeks_first_order

Get a snapshot of first-order Greeks for an option contract: delta, theta, vega, rho, epsilon, and lambda.

## Code Example

::: code-group
```rust [Rust]
let g = client.option_snapshot_greeks_first_order(
    "SPY", "20241220", "500000", "C"
).await?;
```
```python [Python]
g = client.option_snapshot_greeks_first_order("SPY", "20241220", "500000", "C")
```
```go [Go]
g, err := client.OptionSnapshotGreeksFirstOrder("SPY", "20241220", "500000", "C")
```
```cpp [C++]
auto g = client.option_snapshot_greeks_first_order("SPY", "20241220", "500000", "C");
```
:::

## Parameters

Parameters are identical to [option_snapshot_greeks_all](./greeks-all#parameters).

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer) |
| `right` | string | Yes | `"C"` or `"P"` |
| `annual_dividend` | float | No | Override annual dividend |
| `rate_type` | string | No | Interest rate type |
| `rate_value` | float | No | Override interest rate value |
| `stock_price` | float | No | Override underlying price |
| `version` | string | No | Greeks calculation version |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day |
| `use_market_value` | bool | No | Use market value instead of last trade |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `implied_volatility` | float | Implied volatility |
| `delta` | float | Rate of change of option price w.r.t. underlying price |
| `theta` | float | Rate of change of option price w.r.t. time |
| `vega` | float | Rate of change of option price w.r.t. volatility |
| `rho` | float | Rate of change of option price w.r.t. interest rate |
| `epsilon` | float | Rate of change of option price w.r.t. dividend yield |
| `lambda` | float | Percentage change of option price per percentage change of underlying |
| `underlying_price` | float | Underlying price used |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |

## Subscription Tier

Pro
