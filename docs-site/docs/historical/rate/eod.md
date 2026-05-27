---
title: interest_rate_history_eod
description: End-of-day interest rate history for SOFR and Treasury yields.
---

# interest_rate_history_eod

<TierBadge tier="free" />

Retrieve end-of-day interest rate data across a date range. Supports SOFR and all standard Treasury maturities.

::: warning Units
Rates are published in **percent**, not decimal — `4.3600` means 4.36%, not 0.04360%. When using these values to discount cash flows or in Black-Scholes Greeks, divide by 100 first.
:::

## Code Example

::: code-group
```rust [Rust]
let data = tdx.interest_rate_history_eod("SOFR", "20260101", "20260301").await?;
for t in &data {
    // `rate` is published in percent; e.g. 4.3600 == 4.36%.
    println!("date={} rate={:.4}%", t.date, t.rate);
}
```
```python [Python]
data = tdx.interest_rate_history_eod("SOFR", "20260101", "20260301")
for t in data:
    # `rate` is published in percent; e.g. 4.3600 == 4.36%.
    print(f"date={t.date} rate={t.rate:.4f}%")
```
```typescript [TypeScript]
const data = tdx.interestRateHistoryEOD('SOFR', '20260101', '20260301');
for (const t of data) {
    // `rate` is published in percent; e.g. 4.3600 == 4.36%.
    console.log(`date=${t.date} rate=${t.rate}%`);
}
```
```cpp [C++]
auto data = client.interest_rate_history_eod("SOFR", "20260101", "20260301");
for (const auto& t : data) {
    // `rate` is published in percent; e.g. 4.3600 == 4.36%.
    printf("date=%d rate=%.4f%%\n", t.date, t.rate);
}
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

Returns an array of InterestRateTick records with rate data per trading day:

<div class="param-list">
<div class="param">
<div class="param-header"><code>rate</code><span class="param-type">f64</span></div>
<div class="param-desc">Annualized interest rate, published in <strong>percent</strong>. Example: <code>4.3600</code> = 4.36%. Divide by 100 before using as the discount factor in Black-Scholes Greeks.</div>
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


### Sample Response

```json
[
  {"date": 20260302, "rate": 4.3600},
  {"date": 20260303, "rate": 4.3600},
  {"date": 20260304, "rate": 4.3500}
]
```

> SOFR end-of-day rates for early March 2026 (~4.36% annualized).
> Values are in **percent** — divide by 100 to use as a decimal
> discount factor.

## Notes

- Rates are published on trading days only. Non-trading days are excluded.
- Values are annualized percentages (e.g. `4.3600` = 4.36%). Divide by 100 before applying as a decimal in pricing models.
- Use SOFR as the risk-free rate for short-term options pricing.
- Use the appropriate Treasury maturity matching your option's time to expiration for more accurate Greeks.
- Query the full Treasury curve by calling this endpoint with each maturity symbol to build a yield curve snapshot.
