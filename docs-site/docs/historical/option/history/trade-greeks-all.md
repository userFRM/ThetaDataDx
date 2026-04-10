---
title: option_history_trade_greeks_all
description: All Greeks computed on each individual trade.
---

# option_history_trade_greeks_all

<TierBadge tier="professional" />

Retrieve all Greeks (first, second, and third order) computed on each individual trade for an option contract. Unlike interval-sampled Greeks, these are calculated at the exact moment of each trade.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_trade_greeks_all("SPY", "20260417", "550", "C", "20260315").await?;
for t in &data {
    println!("date={} ms_of_day={} implied_volatility={:.4} delta={:.4} gamma={:.4} theta={:.4} vega={:.4} rho={:.4} vanna={:.4} charm={:.4} speed={:.4} zomma={:.4}",
        t.date, t.ms_of_day, t.implied_volatility, t.delta, t.gamma, t.theta, t.vega, t.rho, t.vanna, t.charm, t.speed, t.zomma);
}
```
```python [Python]
data = tdx.option_history_trade_greeks_all("SPY", "20260417", "550", "C", "20260315")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} implied_volatility={t['implied_volatility']:.4f} delta={t['delta']:.4f} gamma={t['gamma']:.4f} theta={t['theta']:.4f} "
          f"vega={t['vega']:.4f} rho={t['rho']:.4f} vanna={t['vanna']:.4f} charm={t['charm']:.4f} speed={t['speed']:.4f} zomma={t['zomma']:.4f}")
```
```go [Go]
data, _ := client.OptionHistoryTradeGreeksAll("SPY", "20260417", "550", "C", "20260315")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d implied_volatility=%.4f delta=%.4f gamma=%.4f theta=%.4f vega=%.4f rho=%.4f vanna=%.4f charm=%.4f speed=%.4f zomma=%.4f\n",
        t.Date, t.MsOfDay, t.ImpliedVolatility, t.Delta, t.Gamma, t.Theta, t.Vega, t.Rho, t.Vanna, t.Charm, t.Speed, t.Zomma)
}
```
```cpp [C++]
auto data = client.option_history_trade_greeks_all("SPY", "20260417", "550", "C", "20260315");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d implied_volatility=%.4f delta=%.4f gamma=%.4f theta=%.4f vega=%.4f rho=%.4f vanna=%.4f charm=%.4f speed=%.4f zomma=%.4f\n",
        t.date, t.ms_of_day, t.implied_volatility, t.delta, t.gamma, t.theta, t.vega, t.rho, t.vanna, t.charm, t.speed, t.zomma);
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
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time as milliseconds from midnight</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time as milliseconds from midnight</div>
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
<div class="param-header"><code>price</code><span class="param-type">float</span></div>
<div class="param-desc">Trade price</div>
</div>
<div class="param">
<div class="param-header"><code>size</code><span class="param-type">int</span></div>
<div class="param-desc">Trade size</div>
</div>
<div class="param">
<div class="param-header"><code>condition</code><span class="param-type">int</span></div>
<div class="param-desc">Trade condition code</div>
</div>
<div class="param">
<div class="param-header"><code>exchange</code><span class="param-type">int</span></div>
<div class="param-desc">Exchange code</div>
</div>
<div class="param">
<div class="param-header"><code>implied_volatility</code><span class="param-type">float</span></div>
<div class="param-desc">IV at time of trade</div>
</div>
<div class="param">
<div class="param-header"><code>delta</code><span class="param-type">float</span></div>
<div class="param-desc">Delta</div>
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
<div class="param-header"><code>epsilon</code><span class="param-type">float</span></div>
<div class="param-desc">Epsilon</div>
</div>
<div class="param">
<div class="param-header"><code>lambda</code><span class="param-type">float</span></div>
<div class="param-desc">Lambda</div>
</div>
<div class="param">
<div class="param-header"><code>gamma</code><span class="param-type">float</span></div>
<div class="param-desc">Gamma</div>
</div>
<div class="param">
<div class="param-header"><code>vanna</code><span class="param-type">float</span></div>
<div class="param-desc">Vanna</div>
</div>
<div class="param">
<div class="param-header"><code>charm</code><span class="param-type">float</span></div>
<div class="param-desc">Charm</div>
</div>
<div class="param">
<div class="param-header"><code>vomma</code><span class="param-type">float</span></div>
<div class="param-desc">Vomma</div>
</div>
<div class="param">
<div class="param-header"><code>veta</code><span class="param-type">float</span></div>
<div class="param-desc">Veta</div>
</div>
<div class="param">
<div class="param-header"><code>speed</code><span class="param-type">float</span></div>
<div class="param-desc">Speed</div>
</div>
<div class="param">
<div class="param-header"><code>zomma</code><span class="param-type">float</span></div>
<div class="param-desc">Zomma</div>
</div>
<div class="param">
<div class="param-header"><code>color</code><span class="param-type">float</span></div>
<div class="param-desc">Color</div>
</div>
<div class="param">
<div class="param-header"><code>ultima</code><span class="param-type">float</span></div>
<div class="param-desc">Ultima</div>
</div>
<div class="param">
<div class="param-header"><code>underlying_price</code><span class="param-type">float</span></div>
<div class="param-desc">Underlying price at time of trade</div>
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
    "date": 20260402, "ms_of_day": 34203497,
    "implied_volatility": 0.4290, "delta": 0.9742, "gamma": 0.000756,
    "theta": -0.1645, "vega": 7.8120, "rho": 21.8812,
    "vanna": -0.0267, "charm": 0.0019, "speed": -0.0000094, "zomma": 0.0000518
  }
]
```

> All Greeks computed at each trade execution. Requires Professional subscription.

## Notes

- Each row combines trade data with Greeks computed using the trade price and the underlying price at the exact moment of execution.
- For liquid contracts this can return a large number of rows. Use `start_time`/`end_time` to limit the window.
