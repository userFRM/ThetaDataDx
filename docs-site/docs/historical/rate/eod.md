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

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Rate symbol (e.g. <code>"SOFR"</code>, <code>"TREASURY_Y10"</code>)</div>
</div>
<div class="param">
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format</div>
</div>
</div>

## Response

Returns a `DataTable` with rate data per trading day:

<div class="param-list">
<div class="param">
<div class="param-header"><code>rate</code><span class="param-type">f64</span></div>
<div class="param-desc">Interest rate value (annualized, as decimal)</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">u32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>

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
