---
title: option_history_greeks_second_order
description: Second-order Greeks history at a given interval.
---

# option_history_greeks_second_order

<TierBadge tier="professional" />

Retrieve second-order Greeks (gamma, vanna, charm, vomma, veta) sampled at a given interval throughout a trading day.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_greeks_second_order("SPY", "20260417", "550", "C", "20260315", "60000").await?;
for t in &data {
    println!("date={} ms_of_day={} gamma={:.4} vanna={:.4} charm={:.4} vomma={:.4} veta={:.4}",
        t.date, t.ms_of_day, t.gamma, t.vanna, t.charm, t.vomma, t.veta);
}
```
```python [Python]
data = tdx.option_history_greeks_second_order("SPY", "20260417", "550", "C", "20260315", "60000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} gamma={t['gamma']:.4f} "
          f"vanna={t['vanna']:.4f} charm={t['charm']:.4f} vomma={t['vomma']:.4f} veta={t['veta']:.4f}")
```
```go [Go]
data, _ := client.OptionHistoryGreeksSecondOrder("SPY", "20260417", "550", "C", "20260315", "60000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d gamma=%.4f vanna=%.4f charm=%.4f vomma=%.4f veta=%.4f\n",
        t.Date, t.MsOfDay, t.Gamma, t.Vanna, t.Charm, t.Vomma, t.Veta)
}
```
```cpp [C++]
auto data = client.option_history_greeks_second_order("SPY", "20260417", "550", "C", "20260315", "60000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d gamma=%.4f vanna=%.4f charm=%.4f vomma=%.4f veta=%.4f\n",
        t.date, t.ms_of_day, t.gamma, t.vanna, t.charm, t.vomma, t.veta);
}
```
:::

## Parameters

Parameters are identical to [option_history_greeks_all](./greeks-all#parameters).

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
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>interval</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Accepts milliseconds (<code>"60000"</code>) or shorthand (<code>"1m"</code>). Valid presets: <code>100ms</code>, <code>500ms</code>, <code>1s</code>, <code>5s</code>, <code>10s</code>, <code>15s</code>, <code>30s</code>, <code>1m</code>, <code>5m</code>, <code>10m</code>, <code>15m</code>, <code>30m</code>, <code>1h</code>.</div>
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
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike range filter</div>
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
<div class="param-desc">Rate of change of delta w.r.t. time</div>
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
<div class="param-desc">Underlying price</div>
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
  {"date": 20260402, "ms_of_day": 34260000, "gamma": 0.000892, "vanna": -0.031200, "charm": 0.002500, "vomma": 0.019800, "veta": -0.112000},
  {"date": 20260402, "ms_of_day": 34320000, "gamma": 0.000815, "vanna": -0.028400, "charm": 0.002100, "vomma": 0.017600, "veta": -0.098000}
]
```

> Second-order Greeks at 1-minute intervals. Requires Professional subscription.

