---
title: interest_rate_history_eod
description: End-of-day interest rate history for SOFR and Treasury yields.
---

# interest_rate_history_eod

<TierBadge tier="free" />

Retrieve end-of-day interest rate data across a date range. Supports SOFR and all standard Treasury maturities.

## Code Example

::: code-group
```rust [Rust]
let table: proto::DataTable = client.interest_rate_history_eod(
    "SOFR", "20240101", "20240301"
).await?;

// Treasury 10-year yield
let table: proto::DataTable = client.interest_rate_history_eod(
    "TREASURY_Y10", "20240101", "20240301"
).await?;
```
```python [Python]
result = client.interest_rate_history_eod("SOFR", "20240101", "20240301")

# Treasury 10-year yield
t10 = client.interest_rate_history_eod("TREASURY_Y10", "20240101", "20240301")
```
```go [Go]
result, err := client.InterestRateHistoryEOD("SOFR", "20240101", "20240301")
if err != nil {
    log.Fatal(err)
}

// Treasury 10-year yield
t10, err := client.InterestRateHistoryEOD("TREASURY_Y10", "20240101", "20240301")
```
```cpp [C++]
auto result = client.interest_rate_history_eod("SOFR", "20240101", "20240301");

// Treasury 10-year yield
auto t10 = client.interest_rate_history_eod("TREASURY_Y10", "20240101", "20240301");
```
:::

## Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `symbol` | string | Yes | Rate symbol (e.g. `"SOFR"`, `"TREASURY_Y10"`) |
| `start_date` | string | Yes | Start date (`YYYYMMDD`) |
| `end_date` | string | Yes | End date (`YYYYMMDD`) |

## Response

Returns a `DataTable` with rate data per trading day:

| Field | Type | Description |
|-------|------|-------------|
| `rate` | f64 | Interest rate value (annualized, as decimal) |
| `date` | u32 | Date as `YYYYMMDD` integer |

 - available on all plans.

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

## Notes

- Rates are published on trading days only. Non-trading days are excluded.
- Use SOFR as the risk-free rate for short-term options pricing.
- Use the appropriate Treasury maturity matching your option's time to expiration for more accurate Greeks.
- Query the full Treasury curve by calling this endpoint with each maturity symbol to build a yield curve snapshot.
