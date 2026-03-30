---
title: option_snapshot_greeks_iv
description: Implied volatility snapshot for an option contract.
---

# option_snapshot_greeks_iv

Get the latest implied volatility (IV) snapshot for an option contract.

## Code Example

::: code-group
```rust [Rust]
let iv = client.option_snapshot_greeks_implied_volatility(
    "SPY", "20241220", "500000", "C"
).await?;
```
```python [Python]
iv = client.option_snapshot_greeks_implied_volatility("SPY", "20241220", "500000", "C")
```
```go [Go]
iv, err := client.OptionSnapshotGreeksIV("SPY", "20241220", "500000", "C")
```
```cpp [C++]
auto iv = client.option_snapshot_greeks_implied_volatility("SPY", "20241220", "500000", "C");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer) |
| `right` | string | Yes | `"C"` or `"P"` |
| `annual_dividend` | float | No | Override annual dividend |
| `rate_type` | string | No | Interest rate type (e.g. `"SOFR"`) |
| `rate_value` | float | No | Override interest rate value |
| `stock_price` | float | No | Override underlying price |
| `version` | string | No | Greeks calculation version |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |
| `min_time` | string | No | Minimum time of day (ms from midnight) |
| `use_market_value` | bool | No | Use market value instead of last trade |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `implied_volatility` | float | Implied volatility |
| `bid_iv` | float | Bid implied volatility |
| `ask_iv` | float | Ask implied volatility |
| `underlying_price` | float | Underlying price used in calculation |
| `iv_error` | float | IV solver error |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |

## Subscription Tier

Pro

## Notes

- Use the optional override parameters (`stock_price`, `rate_value`, `annual_dividend`) to compute IV under custom assumptions.
- The `use_market_value` flag switches the calculation from last trade price to mid-market value.
