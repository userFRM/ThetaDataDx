---
title: option_history_trade_greeks_iv
description: Implied volatility computed on each individual trade.
---

# option_history_trade_greeks_iv

<TierBadge tier="professional" />

Retrieve implied volatility computed on each individual trade for an option contract.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_trade_greeks_implied_volatility("SPY", "20260417", "550", "C", "20260315").await?;
for t in &data {
    println!("date={} ms_of_day={} implied_volatility={:.4} iv_error={:.4}",
        t.date, t.ms_of_day, t.implied_volatility, t.iv_error);
}
```
```python [Python]
data = tdx.option_history_trade_greeks_implied_volatility("SPY", "20260417", "550", "C", "20260315")
for t in data:
    print(f"date={t.date} ms_of_day={t.ms_of_day} "
          f"implied_volatility={t.implied_volatility:.4f} iv_error={t.iv_error:.4f}")
```
```typescript [TypeScript]
const data = tdx.optionHistoryTradeGreeksImpliedVolatility('SPY', '20260417', '550', 'C', '20260315');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} implied_volatility=${t.implied_volatility} iv_error=${t.iv_error}`);
}
```
```go [Go]
data, _ := client.OptionHistoryTradeGreeksImpliedVolatility("SPY", "20260417", "550", "C", "20260315")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d implied_volatility=%.4f iv_error=%.4f\n",
        t.Date, t.MsOfDay, t.ImpliedVolatility, t.IVError)
}
```
```cpp [C++]
auto data = client.option_history_trade_greeks_implied_volatility("SPY", "20260417", "550", "C", "20260315");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d implied_volatility=%.4f iv_error=%.4f\n",
        t.date, t.ms_of_day, t.implied_volatility, t.iv_error);
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
<div class="param-desc">IV computed from trade price</div>
</div>
<div class="param">
<div class="param-header"><code>bid_iv</code><span class="param-type">float</span></div>
<div class="param-desc">Bid IV</div>
</div>
<div class="param">
<div class="param-header"><code>ask_iv</code><span class="param-type">float</span></div>
<div class="param-desc">Ask IV</div>
</div>
<div class="param">
<div class="param-header"><code>underlying_price</code><span class="param-type">float</span></div>
<div class="param-desc">Underlying price at time of trade</div>
</div>
<div class="param">
<div class="param-header"><code>iv_error</code><span class="param-type">float</span></div>
<div class="param-desc">IV solver error</div>
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
  {"date": 20260402, "ms_of_day": 34203497, "implied_volatility": 0.429000, "iv_error": 0.0}
]
```

> IV computed at each trade execution. Requires Professional subscription.

## Notes

- Provides per-trade IV, which is useful for analyzing IV dynamics around large trades or sweeps.
- Compare `implied_volatility` against `bid_iv`/`ask_iv` to understand where in the spread the trade executed.
