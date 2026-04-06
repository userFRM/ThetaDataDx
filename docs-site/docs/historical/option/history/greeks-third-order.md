---
title: option_history_greeks_third_order
description: Third-order Greeks history at a given interval.
---

# option_history_greeks_third_order

<TierBadge tier="professional" />

Retrieve third-order Greeks (speed, zomma, color, ultima) sampled at a given interval throughout a trading day.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_greeks_third_order("SPY", "20260417", "550", "C", "20260315", "60000").await?;
for t in &data {
    println!("date={} ms_of_day={} speed={:.4} zomma={:.4} color={:.4} ultima={:.4}",
        t.date, t.ms_of_day, t.speed, t.zomma, t.color, t.ultima);
}
```
```python [Python]
data = tdx.option_history_greeks_third_order("SPY", "20260417", "550", "C", "20260315", "60000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} speed={t['speed']:.4f} "
          f"zomma={t['zomma']:.4f} color={t['color']:.4f} ultima={t['ultima']:.4f}")
```
```go [Go]
data, _ := client.OptionHistoryGreeksThirdOrder("SPY", "20260417", "550", "C", "20260315", "60000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d speed=%.4f zomma=%.4f color=%.4f ultima=%.4f\n",
        t.Date, t.MsOfDay, t.Speed, t.Zomma, t.Color, t.Ultima)
}
```
```cpp [C++]
auto data = client.option_history_greeks_third_order("SPY", "20260417", "550", "C", "20260315", "60000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d speed=%.4f zomma=%.4f color=%.4f ultima=%.4f\n",
        t.date, t.ms_of_day, t.speed, t.zomma, t.color, t.ultima);
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
<div class="param-desc">Strike price as scaled integer</div>
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
<div class="param-header"><code>speed</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of gamma w.r.t. underlying price</div>
</div>
<div class="param">
<div class="param-header"><code>zomma</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of gamma w.r.t. volatility</div>
</div>
<div class="param">
<div class="param-header"><code>color</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of gamma w.r.t. time</div>
</div>
<div class="param">
<div class="param-header"><code>ultima</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of vomma w.r.t. volatility</div>
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
  {"date": 20260402, "ms_of_day": 34260000, "speed": -0.00001120, "zomma": 0.00006120, "color": -0.00000420, "ultima": 0.00016800},
  {"date": 20260402, "ms_of_day": 34320000, "speed": -0.00001040, "zomma": 0.00005840, "color": -0.00000390, "ultima": 0.00015200}
]
```

> Third-order Greeks at 1-minute intervals. Requires Professional subscription.

