---
title: option_snapshot_greeks_first_order
description: First-order Greeks snapshot for an option contract.
---

# option_snapshot_greeks_first_order

<TierBadge tier="professional" />

Get a snapshot of first-order Greeks for an option contract: delta, theta, vega, rho, epsilon, and lambda.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_snapshot_greeks_first_order("SPY", "20260417", "550", "C").await?;
for t in &data {
    println!("date={} ms_of_day={} implied_volatility={:.4} delta={:.4} theta={:.4} vega={:.4} rho={:.4} epsilon={:.4} lambda={:.4} expiration={} strike={:.2}",
        t.date, t.ms_of_day, t.implied_volatility, t.delta, t.theta, t.vega, t.rho, t.epsilon, t.lambda, t.expiration, t.strike);
}
```
```python [Python]
data = tdx.option_snapshot_greeks_first_order("SPY", "20260417", "550", "C")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} implied_volatility={t['implied_volatility']:.4f} delta={t['delta']:.4f} theta={t['theta']:.4f} "
          f"vega={t['vega']:.4f} rho={t['rho']:.4f} epsilon={t['epsilon']:.4f} lambda={t['lambda']:.4f} expiration={t['expiration']} strike={t['strike']:.2f}")
```
```go [Go]
data, _ := client.OptionSnapshotGreeksFirstOrder("SPY", "20260417", "550", "C")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d implied_volatility=%.4f delta=%.4f theta=%.4f vega=%.4f rho=%.4f epsilon=%.4f lambda=%.4f expiration=%d strike=%.2f\n",
        t.Date, t.MsOfDay, t.ImpliedVolatility, t.Delta, t.Theta, t.Vega, t.Rho, t.Epsilon, t.Lambda, t.Expiration, t.Strike)
}
```
```cpp [C++]
auto data = client.option_snapshot_greeks_first_order("SPY", "20260417", "550", "C");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d implied_volatility=%.4f delta=%.4f theta=%.4f vega=%.4f rho=%.4f epsilon=%.4f lambda=%.4f expiration=%d strike=%.2f\n",
        t.date, t.ms_of_day, t.implied_volatility, t.delta, t.theta, t.vega, t.rho, t.epsilon, t.lambda, t.expiration, t.strike);
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
<div class="param-desc">Percentage change of option price per percentage change of underlying</div>
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
    "date": 20260402, "ms_of_day": 58497982, "implied_volatility": 0.4091,
    "delta": 0.9855, "theta": -0.1205, "vega": 4.8813, "rho": 22.1671,
    "epsilon": -26.5693, "lambda": 6.0354,
    "expiration": 20260417, "strike": 550.0
  }
]
```

> First-order Greeks for SPY 2026-04-17 550 call.

