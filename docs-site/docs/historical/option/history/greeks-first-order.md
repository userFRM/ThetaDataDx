---
title: option_history_greeks_first_order
description: First-order Greeks history at a given interval.
---

# option_history_greeks_first_order

<TierBadge tier="professional" />

Retrieve first-order Greeks (delta, theta, vega, rho, epsilon, lambda) sampled at a given interval throughout a trading day.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_greeks_first_order("SPY", "20260417", "550", "C", "20260315", "60000").await?;
for t in &data {
    println!("date={} ms_of_day={} implied_volatility={:.4} delta={:.4} theta={:.4} vega={:.4} rho={:.4} epsilon={:.4} lambda={:.4}",
        t.date, t.ms_of_day, t.implied_volatility, t.delta, t.theta, t.vega, t.rho, t.epsilon, t.lambda);
}
```
```python [Python]
data = tdx.option_history_greeks_first_order("SPY", "20260417", "550", "C", "20260315", "60000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} implied_volatility={t['implied_volatility']:.4f} delta={t['delta']:.4f} "
          f"theta={t['theta']:.4f} vega={t['vega']:.4f} rho={t['rho']:.4f} epsilon={t['epsilon']:.4f} lambda={t['lambda']:.4f}")
```
```go [Go]
data, _ := client.OptionHistoryGreeksFirstOrder("SPY", "20260417", "550", "C", "20260315", "60000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d implied_volatility=%.4f delta=%.4f theta=%.4f vega=%.4f rho=%.4f epsilon=%.4f lambda=%.4f\n",
        t.Date, t.MsOfDay, t.ImpliedVolatility, t.Delta, t.Theta, t.Vega, t.Rho, t.Epsilon, t.Lambda)
}
```
```cpp [C++]
auto data = client.option_history_greeks_first_order("SPY", "20260417", "550", "C", "20260315", "60000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d implied_volatility=%.4f delta=%.4f theta=%.4f vega=%.4f rho=%.4f epsilon=%.4f lambda=%.4f\n",
        t.date, t.ms_of_day, t.implied_volatility, t.delta, t.theta, t.vega, t.rho, t.epsilon, t.lambda);
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
<div class="param-header"><code>delta</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of option price w.r.t. underlying price</div>
</div>
<div class="param">
<div class="param-header"><code>theta</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of option price w.r.t. time</div>
</div>
<div class="param">
<div class="param-header"><code>vega</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of option price w.r.t. volatility</div>
</div>
<div class="param">
<div class="param-header"><code>rho</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of option price w.r.t. interest rate</div>
</div>
<div class="param">
<div class="param-header"><code>epsilon</code><span class="param-type">float</span></div>
<div class="param-desc">Rate of change of option price w.r.t. dividend yield</div>
</div>
<div class="param">
<div class="param-header"><code>lambda</code><span class="param-type">float</span></div>
<div class="param-desc">Percentage change of option per percentage change of underlying</div>
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
  {"date": 20260402, "ms_of_day": 34260000, "implied_volatility": 0.4445, "delta": 0.9686, "theta": -0.1898, "vega": 9.2497, "rho": 21.7056, "epsilon": -25.7503, "lambda": 6.3665},
  {"date": 20260402, "ms_of_day": 34320000, "implied_volatility": 0.4350, "delta": 0.9718, "theta": -0.1757, "vega": 8.4631, "rho": 21.7944, "epsilon": -25.8555, "lambda": 6.3666}
]
```

> First-order Greeks at 1-minute intervals for SPY 2026-04-17 550 call.

