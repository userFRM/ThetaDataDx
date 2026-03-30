---
title: option_history_trade_greeks_iv
description: Implied volatility computed on each individual trade.
---

# option_history_trade_greeks_iv

Retrieve implied volatility computed on each individual trade for an option contract.

## Code Example

::: code-group
```rust [Rust]
let iv = client.option_history_trade_greeks_implied_volatility(
    "SPY", "20241220", "500000", "C", "20240315"
).await?;
```
```python [Python]
iv = client.option_history_trade_greeks_implied_volatility(
    "SPY", "20241220", "500000", "C", "20240315")
```
```go [Go]
iv, err := client.OptionHistoryTradeGreeksImpliedVolatility(
    "SPY", "20241220", "500000", "C", "20240315")
```
```cpp [C++]
auto iv = client.option_history_trade_greeks_implied_volatility(
    "SPY", "20241220", "500000", "C", "20240315");
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
| `implied_volatility` | float | IV computed from trade price |
| `bid_iv` | float | Bid IV |
| `ask_iv` | float | Ask IV |
| `underlying_price` | float | Underlying price at time of trade |
| `iv_error` | float | IV solver error |
| `date` | string | Date |
| `ms_of_day` | int | Milliseconds from midnight |

## Subscription Tier

Pro

## Notes

- Provides per-trade IV, which is useful for analyzing IV dynamics around large trades or sweeps.
- Compare `implied_volatility` against `bid_iv`/`ask_iv` to understand where in the spread the trade executed.
