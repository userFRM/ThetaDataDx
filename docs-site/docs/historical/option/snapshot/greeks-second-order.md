---
title: option_snapshot_greeks_second_order
description: Second-order Greeks snapshot for an option contract.
---

# option_snapshot_greeks_second_order

<TierBadge tier="professional" />

Get a snapshot of second-order Greeks for an option contract: gamma, vanna, charm, vomma, and veta.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_snapshot_greeks_second_order("SPY", "20260417", "550", "C").await?;
for t in &data {
    println!("date={} ms_of_day={} gamma={:.4} vanna={:.4} charm={:.4} vomma={:.4} veta={:.4} expiration={} strike={:.2}",
        t.date, t.ms_of_day, t.gamma, t.vanna, t.charm, t.vomma, t.veta, t.expiration, t.strike);
}
```
```python [Python]
data = tdx.option_snapshot_greeks_second_order("SPY", "20260417", "550", "C")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} gamma={t['gamma']:.4f} vanna={t['vanna']:.4f} "
          f"charm={t['charm']:.4f} vomma={t['vomma']:.4f} veta={t['veta']:.4f} expiration={t['expiration']} strike={t['strike']:.2f}")
```
```go [Go]
data, _ := client.OptionSnapshotGreeksSecondOrder("SPY", "20260417", "550", "C")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d gamma=%.4f vanna=%.4f charm=%.4f vomma=%.4f veta=%.4f expiration=%d strike=%.2f\n",
        t.Date, t.MsOfDay, t.Gamma, t.Vanna, t.Charm, t.Vomma, t.Veta, t.Expiration, t.Strike)
}
```
```cpp [C++]
auto data = client.option_snapshot_greeks_second_order("SPY", "20260417", "550", "C");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d gamma=%.4f vanna=%.4f charm=%.4f vomma=%.4f veta=%.4f expiration=%d strike=%.2f\n",
        t.date, t.ms_of_day, t.gamma, t.vanna, t.charm, t.vomma, t.veta, t.expiration, t.strike);
}
```
:::

## Parameters

Parameters are identical to [option_snapshot_greeks_all](./greeks-all#parameters).

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
<div class="param-desc">Strike price in dollars as a string</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc"><code>"C"</code> for call, <code>"P"</code> for put</div>
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
<div class="param-header"><code>stock_price</code><span class="param-type">float</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Override underlying price</div>
</div>
<div class="param">
<div class="param-header"><code>version</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Greeks calculation version</div>
</div>
<div class="param">
<div class="param-header"><code>max_dte</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Maximum days to expiration</div>
</div>
<div class="param">
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike range filter</div>
</div>
<div class="param">
<div class="param-header"><code>min_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Minimum time of day as milliseconds from midnight</div>
</div>
<div class="param">
<div class="param-header"><code>use_market_value</code><span class="param-type">bool</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Use market value instead of last trade price</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>implied_volatility</code><span class="param-type">float</span></div>
<div class="param-desc">Implied volatility</div>
</div>
<div class="param">
<div class="param-header"><code>gamma</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of delta w.r.t. underlying price</div>
</div>
<div class="param">
<div class="param-header"><code>vanna</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of delta w.r.t. volatility</div>
</div>
<div class="param">
<div class="param-header"><code>charm</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of delta w.r.t. time (delta decay)</div>
</div>
<div class="param">
<div class="param-header"><code>vomma</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of vega w.r.t. volatility</div>
</div>
<div class="param">
<div class="param-header"><code>veta</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of vega w.r.t. time</div>
</div>
<div class="param">
<div class="param-header"><code>underlying_price</code><span class="param-type">float</span></div>
<div class="param-desc">Underlying price used</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span></div>
<div class="param-desc">Date</div>
</div>
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">int</span></div>
<div class="param-desc">Milliseconds from midnight</div>
</div>
</div>

### Sample Response

```json
[
  {
    "date": 20260402, "ms_of_day": 58497982,
    "gamma": 0.000412, "vanna": -0.023400, "charm": 0.001800,
    "vomma": 0.015600, "veta": -0.089200,
    "expiration": 20260417, "strike": 550.0
  }
]
```

> Second-order Greeks for SPY 2026-04-17 550 call. Requires Professional subscription.

