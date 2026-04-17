---
title: option_history_trade_greeks_first_order
description: First-order Greeks computed on each individual trade.
---

# option_history_trade_greeks_first_order

<TierBadge tier="professional" />

Retrieve first-order Greeks computed on each individual trade for an option contract.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_trade_greeks_first_order("SPY", "20260417", "550", "C", "20260315").await?;
for t in &data {
    println!("date={} ms_of_day={} implied_volatility={:.4} delta={:.4} theta={:.4} vega={:.4} rho={:.4}",
        t.date, t.ms_of_day, t.implied_volatility, t.delta, t.theta, t.vega, t.rho);
}
```
```python [Python]
data = tdx.option_history_trade_greeks_first_order("SPY", "20260417", "550", "C", "20260315")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} implied_volatility={t['implied_volatility']:.4f} "
          f"delta={t['delta']:.4f} theta={t['theta']:.4f} vega={t['vega']:.4f} rho={t['rho']:.4f}")
```
```typescript [TypeScript]
const data = tdx.optionHistoryTradeGreeksFirstOrder('SPY', '20260417', '550', 'C', '20260315');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} implied_volatility=${t.implied_volatility} delta=${t.delta} theta=${t.theta} vega=${t.vega}`);
}
```
```go [Go]
data, _ := client.OptionHistoryTradeGreeksFirstOrder("SPY", "20260417", "550", "C", "20260315")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d implied_volatility=%.4f delta=%.4f theta=%.4f vega=%.4f rho=%.4f\n",
        t.Date, t.MsOfDay, t.ImpliedVolatility, t.Delta, t.Theta, t.Vega, t.Rho)
}
```
```cpp [C++]
auto data = client.option_history_trade_greeks_first_order("SPY", "20260417", "550", "C", "20260315");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d implied_volatility=%.4f delta=%.4f theta=%.4f vega=%.4f rho=%.4f\n",
        t.date, t.ms_of_day, t.implied_volatility, t.delta, t.theta, t.vega, t.rho);
}
```
:::

## Parameters

Parameters are identical to [option_history_trade_greeks_all](./trade-greeks-all#parameters).

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
  {"date": 20260402, "ms_of_day": 34203497, "implied_volatility": 0.4290, "delta": 0.9742, "theta": -0.1645, "vega": 7.8120, "rho": 21.8812}
]
```

> First-order Greeks at each trade. Requires Professional subscription.

