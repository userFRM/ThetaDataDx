---
title: Rate Endpoints
description: Interest rate endpoints - historical risk-free rate data for SOFR and Treasury yields.
---

# Rate Endpoints (1)

Historical interest rate data for use in options pricing, Greeks computation, and risk analysis.

## Endpoints

| Endpoint | Description |
|----------|-------------|
| [interest_rate_history_eod](./eod) | End-of-day interest rate history |

## Quick Example

::: code-group
```rust [Rust]
let rates = tdx.interest_rate_history_eod(
    "SOFR", "20240101", "20240301"
).await?;
```
```python [Python]
result = tdx.interest_rate_history_eod("SOFR", "20240101", "20240301")
```
```go [Go]
result, _ := client.InterestRateHistoryEOD("SOFR", "20240101", "20240301")
```
```cpp [C++]
auto result = client.interest_rate_history_eod("SOFR", "20240101", "20240301");
```
:::

## Available Rate Symbols

| Symbol | Description |
|--------|-------------|
| `SOFR` | Secured Overnight Financing Rate |
| `TREASURY_M1` | 1-month Treasury |
| `TREASURY_M3` | 3-month Treasury |
| `TREASURY_M6` | 6-month Treasury |
| `TREASURY_Y1` | 1-year Treasury |
| `TREASURY_Y2` | 2-year Treasury |
| `TREASURY_Y3` | 3-year Treasury |
| `TREASURY_Y5` | 5-year Treasury |
| `TREASURY_Y7` | 7-year Treasury |
| `TREASURY_Y10` | 10-year Treasury |
| `TREASURY_Y20` | 20-year Treasury |
| `TREASURY_Y30` | 30-year Treasury |

::: tip
Use interest rate data to set the risk-free rate parameter when computing option Greeks locally.
:::
