---
title: option_history_greeks_eod
description: End-of-day Greeks history for an option contract.
---

# option_history_greeks_eod

<TierBadge tier="professional" />

Retrieve end-of-day Greeks history for an option contract across a date range.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_greeks_eod("SPY", "20260417", "550", "C", "20260101", "20260301").await?;
for t in &data {
    println!("date={} implied_volatility={:.4} delta={:.4} gamma={:.4} theta={:.4} vega={:.4} rho={:.4}",
        t.date, t.implied_volatility, t.delta, t.gamma, t.theta, t.vega, t.rho);
}
```
```python [Python]
data = tdx.option_history_greeks_eod("SPY", "20260417", "550", "C", "20260101", "20260301")
for t in data:
    print(f"date={t['date']} implied_volatility={t['implied_volatility']:.4f} delta={t['delta']:.4f} "
          f"gamma={t['gamma']:.4f} theta={t['theta']:.4f} vega={t['vega']:.4f} rho={t['rho']:.4f}")
```
```go [Go]
data, _ := client.OptionHistoryGreeksEOD("SPY", "20260417", "550", "C", "20260101", "20260301")
for _, t := range data {
    fmt.Printf("date=%d implied_volatility=%.4f delta=%.4f gamma=%.4f theta=%.4f vega=%.4f rho=%.4f\n",
        t.Date, t.ImpliedVolatility, t.Delta, t.Gamma, t.Theta, t.Vega, t.Rho)
}
```
```cpp [C++]
auto data = client.option_history_greeks_eod("SPY", "20260417", "550", "C", "20260101", "20260301");
for (const auto& t : data) {
    printf("date=%d implied_volatility=%.4f delta=%.4f gamma=%.4f theta=%.4f vega=%.4f rho=%.4f\n",
        t.date, t.implied_volatility, t.delta, t.gamma, t.theta, t.vega, t.rho);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Underlying symbol</div>
</div>
<div class="param">
<div class="param-header"><code>expiration</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Expiration date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>strike</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Strike price as scaled integer</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc"><code>"C"</code> for call, <code>"P"</code> for put</div>
</div>
<div class="param">
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>annual_dividend</code><span class="param-type">float</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Override annual dividend</div>
</div>
<div class="param">
<div class="param-header"><code>rate_type</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Interest rate type</div>
</div>
<div class="param">
<div class="param-header"><code>rate_value</code><span class="param-type">float</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Override interest rate value</div>
</div>
<div class="param">
<div class="param-header"><code>version</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Greeks calculation version</div>
</div>
<div class="param">
<div class="param-header"><code>underlyer_use_nbbo</code><span class="param-type">bool</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Use NBBO midpoint for underlying price instead of last trade</div>
</div>
<div class="param">
<div class="param-header"><code>max_dte</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Maximum days to expiration</div>
</div>
<div class="param">
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike range filter</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span></div>
<div class="param-desc">Trading date</div>
</div>
<div class="param">
<div class="param-header"><code>implied_volatility</code><span class="param-type">float</span></div>
<div class="param-desc">Implied volatility</div>
</div>
<div class="param">
<div class="param-header"><code>delta</code><span class="param-type">float</span></div>
<div class="param-desc">Delta</div>
</div>
<div class="param">
<div class="param-header"><code>gamma</code><span class="param-type">float</span></div>
<div class="param-desc">Gamma</div>
</div>
<div class="param">
<div class="param-header"><code>theta</code><span class="param-type">float</span></div>
<div class="param-desc">Theta</div>
</div>
<div class="param">
<div class="param-header"><code>vega</code><span class="param-type">float</span></div>
<div class="param-desc">Vega</div>
</div>
<div class="param">
<div class="param-header"><code>rho</code><span class="param-type">float</span></div>
<div class="param-desc">Rho</div>
</div>
<div class="param">
<div class="param-header"><code>underlying_price</code><span class="param-type">float</span></div>
<div class="param-desc">Underlying close price used</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260302, "implied_volatility": 0.0, "delta": 1.0, "gamma": 0.0, "theta": 0.0, "vega": 0.0, "rho": 0.0},
  {"date": 20260304, "implied_volatility": 0.2802, "delta": 0.9912, "gamma": 0.0003, "theta": -0.0725, "vega": 5.5669, "rho": 63.7867},
  {"date": 20260305, "implied_volatility": 0.2773, "delta": 0.9913, "gamma": 0.0003, "theta": -0.0704, "vega": 5.4231, "rho": 62.8102}
]
```

> EOD Greeks for SPY 2026-04-17 550 call. Deep ITM calls show delta near 1.0. IV of 0.0 indicates the solver could not converge.

## Notes

- EOD Greeks are computed using the closing price. Use `underlyer_use_nbbo` to switch to the NBBO midpoint.
- This is ideal for building daily Greeks time series for backtesting or risk reporting.
