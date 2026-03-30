---
title: option_snapshot_greeks_second_order
description: Second-order Greeks snapshot for an option contract.
---

# option_snapshot_greeks_second_order

Get a snapshot of second-order Greeks for an option contract: gamma, vanna, charm, vomma, and veta.

## Code Example

::: code-group
```rust [Rust]
let g = client.option_snapshot_greeks_second_order(
    "SPY", "20241220", "500000", "C"
).await?;
```
```python [Python]
g = client.option_snapshot_greeks_second_order("SPY", "20241220", "500000", "C")
```
```go [Go]
g, err := client.OptionSnapshotGreeksSecondOrder("SPY", "20241220", "500000", "C")
```
```cpp [C++]
auto g = client.option_snapshot_greeks_second_order("SPY", "20241220", "500000", "C");
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
| `gamma` | float | Rate of change of delta w.r.t. underlying price |
| `vanna` | float | Rate of change of delta w.r.t. volatility |
| `charm` | float | Rate of change of delta w.r.t. time (delta decay) |
| `vomma` | float | Rate of change of vega w.r.t. volatility |
| `veta` | float | Rate of change of vega w.r.t. time |
| `underlying_price` | float | Underlying price used |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |

## Subscription Tier

Pro
