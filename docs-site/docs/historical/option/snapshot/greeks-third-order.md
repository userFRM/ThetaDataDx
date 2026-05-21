---
title: option_snapshot_greeks_third_order
description: Third-order Greeks snapshot for an option contract.
---

# option_snapshot_greeks_third_order

<TierBadge tier="professional" />

Get a snapshot of third-order Greeks for an option contract: speed, zomma, color, and ultima.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_snapshot_greeks_third_order("SPY", "20260417", "550", "C").await?;
for t in &data {
    println!("date={} ms_of_day={} speed={:.4} zomma={:.4} color={:.4} ultima={:.4} expiration={} strike={:.2}",
        t.date, t.ms_of_day, t.speed, t.zomma, t.color, t.ultima, t.expiration, t.strike);
}
```
```python [Python]
data = tdx.option_snapshot_greeks_third_order("SPY", "20260417", "550", "C")
for t in data:
    print(f"date={t.date} ms_of_day={t.ms_of_day} speed={t.speed:.4f} zomma={t.zomma:.4f} "
          f"color={t.color:.4f} ultima={t.ultima:.4f} expiration={t.expiration} strike={t.strike:.2f}")
```
```typescript [TypeScript]
const data = tdx.optionSnapshotGreeksThirdOrder('SPY', '20260417', '550', 'C');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} speed=${t.speed} zomma=${t.zomma} color=${t.color} ultima=${t.ultima}`);
}
```
```cpp [C++]
auto data = client.option_snapshot_greeks_third_order("SPY", "20260417", "550", "C");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d speed=%.4f zomma=%.4f color=%.4f ultima=%.4f expiration=%d strike=%.2f\n",
        t.date, t.ms_of_day, t.speed, t.zomma, t.color, t.ultima, t.expiration, t.strike);
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
<div class="param-desc">Expiration date in <code>YYYYMMDD</code> or <code>YYYY-MM-DD</code> format, or <code>"*"</code> for all expirations</div>
</div>
<div class="param">
<div class="param-header"><code>strike</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike price in dollars (e.g. <code>"550"</code> or <code>"17.5"</code>), or <code>"*"</code> for all strikes. Default: <code>"*"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Option side: <code>"call"</code>, <code>"put"</code>, or <code>"both"</code>. SDK also accepts <code>"C"</code>/<code>"P"</code>. Default: <code>"both"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>annual_dividend</code><span class="param-type">float</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Annualized expected dividend amount used in the Greeks calculation.</div>
</div>
<div class="param">
<div class="param-header"><code>rate_type</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Risk-free rate curve. Allowed values: <code>sofr</code>, <code>treasury_m1</code>, <code>treasury_m3</code>, <code>treasury_m6</code>, <code>treasury_y1</code>, <code>treasury_y2</code>, <code>treasury_y3</code>, <code>treasury_y5</code>, <code>treasury_y7</code>, <code>treasury_y10</code>, <code>treasury_y20</code>, <code>treasury_y30</code>. Default: <code>"sofr"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>rate_value</code><span class="param-type">float</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Override the risk-free rate (as a percent). When set, takes precedence over <code>rate_type</code> for that call.</div>
</div>
<div class="param">
<div class="param-header"><code>stock_price</code><span class="param-type">float</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Override the underlying spot price used in the Greeks calculation.</div>
</div>
<div class="param">
<div class="param-header"><code>version</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Greeks methodology selector. Allowed values: <code>latest</code> (use real time-to-expiry), <code>1</code> (fix 0DTE to 0.15 days). Default: <code>"latest"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>max_dte</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Maximum days to expiration. Filters contracts returned when <code>expiration="*"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Returns <code>n</code> strikes above and below spot price plus one ATM strike (up to <code>2n + 1</code> strikes).</div>
</div>
<div class="param">
<div class="param-header"><code>min_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Filters snapshots to timestamps at or after this ET wall-clock time. Format <code>HH:MM:SS.SSS</code>; legacy millisecond strings are also accepted.</div>
</div>
<div class="param">
<div class="param-header"><code>use_market_value</code><span class="param-type">bool</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">When <code>true</code>, use the market value bid/ask/price inputs for the Greeks calculation. Default: <code>false</code>.</div>
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
    "speed": -0.00000841, "zomma": 0.00004523,
    "color": -0.00000312, "ultima": 0.00012845,
    "expiration": 20260417, "strike": 550.0
  }
]
```

> Third-order Greeks for SPY 2026-04-17 550 call. Requires Professional subscription.

## Notes

- Third-order Greeks are primarily used in advanced hedging and risk management scenarios involving large portfolios or exotic strategies.
