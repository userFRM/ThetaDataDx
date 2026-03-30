---
title: option_history_greeks_eod
description: End-of-day Greeks history for an option contract.
---

# option_history_greeks_eod

Retrieve end-of-day Greeks history for an option contract across a date range.

## Code Example

::: code-group
```rust [Rust]
let g = client.option_history_greeks_eod(
    "SPY", "20241220", "500000", "C", "20240101", "20240301"
).await?;
```
```python [Python]
g = client.option_history_greeks_eod("SPY", "20241220", "500000", "C",
                                      "20240101", "20240301")
```
```go [Go]
g, err := client.OptionHistoryGreeksEOD("SPY", "20241220", "500000", "C",
    "20240101", "20240301")
```
```cpp [C++]
auto g = client.option_history_greeks_eod("SPY", "20241220", "500000", "C",
                                           "20240101", "20240301");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Underlying symbol |
| `expiration` | string | Yes | Expiration date (`YYYYMMDD`) |
| `strike` | string | Yes | Strike price (scaled integer) |
| `right` | string | Yes | `"C"` or `"P"` |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |
| `annual_dividend` | float | No | Override annual dividend |
| `rate_type` | string | No | Interest rate type |
| `rate_value` | float | No | Override interest rate value |
| `version` | string | No | Greeks calculation version |
| `underlyer_use_nbbo` | bool | No | Use NBBO for underlying price |
| `max_dte` | int | No | Maximum days to expiration |
| `strike_range` | int | No | Strike range filter |

## Response

| Field | Type | Description |
|-------|------|-------------|
| `date` | string | Trading date |
| `implied_volatility` | float | Implied volatility |
| `delta` | float | Delta |
| `gamma` | float | Gamma |
| `theta` | float | Theta |
| `vega` | float | Vega |
| `rho` | float | Rho |
| `underlying_price` | float | Underlying close price |

## Subscription Tier

Pro

## Notes

- EOD Greeks are computed using the closing price. Use `underlyer_use_nbbo` to switch to the NBBO midpoint.
- This is ideal for building daily Greeks time series for backtesting or risk reporting.
