---
title: option_history_greeks_all
description: All Greeks history at a given interval (intraday).
---

# option_history_greeks_all

Retrieve all Greeks (first, second, and third order) sampled at a given interval throughout a trading day.

## Code Example

::: code-group
```rust [Rust]
let g = client.option_history_greeks_all(
    "SPY", "20241220", "500000", "C", "20240315", "60000"
).await?;
```
```python [Python]
g = client.option_history_greeks_all("SPY", "20241220", "500000", "C",
                                      "20240315", "60000")
```
```go [Go]
g, err := client.OptionHistoryGreeksAll("SPY", "20241220", "500000", "C",
    "20240315", "60000")
```
```cpp [C++]
auto g = client.option_history_greeks_all("SPY", "20241220", "500000", "C",
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
| `delta` | float | Delta |
| `theta` | float | Theta |
| `vega` | float | Vega |
| `rho` | float | Rho |
| `epsilon` | float | Epsilon |
| `lambda` | float | Lambda |
| `gamma` | float | Gamma |
| `vanna` | float | Vanna |
| `charm` | float | Charm |
| `vomma` | float | Vomma |
| `veta` | float | Veta |
| `speed` | float | Speed |
| `zomma` | float | Zomma |
| `color` | float | Color |
| `ultima` | float | Ultima |
| `underlying_price` | float | Underlying price |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |

## Subscription Tier

Pro

## Notes

- If you only need a subset of Greeks, use [greeks-first-order](./greeks-first-order), [greeks-second-order](./greeks-second-order), or [greeks-third-order](./greeks-third-order) to reduce payload size.
