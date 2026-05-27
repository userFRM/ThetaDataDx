---
title: option_history_greeks_all
description: All Greeks history at a given interval (intraday).
---

# option_history_greeks_all

<TierBadge tier="professional" />

Retrieve all Greeks (first, second, and third order) sampled at a given interval throughout a trading day.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_greeks_all("SPY", "20260417", "550", "C", "20260315", "60000").await?;
for t in &data {
    println!("date={} ms_of_day={} implied_volatility={:.4} delta={:.4} gamma={:.4} theta={:.4} vega={:.4} rho={:.4} vanna={:.4} charm={:.4} vomma={:.4} speed={:.4} zomma={:.4} color={:.4} ultima={:.4}",
        t.date, t.ms_of_day, t.implied_volatility, t.delta, t.gamma, t.theta, t.vega, t.rho, t.vanna, t.charm, t.vomma, t.speed, t.zomma, t.color, t.ultima);
}
```
```python [Python]
data = tdx.option_history_greeks_all("SPY", "20260417", "550", "C", "20260315", "60000")
for t in data:
    print(f"date={t.date} ms_of_day={t.ms_of_day} implied_volatility={t.implied_volatility:.4f} delta={t.delta:.4f} gamma={t.gamma:.4f} theta={t.theta:.4f} vega={t.vega:.4f} "
          f"rho={t.rho:.4f} vanna={t.vanna:.4f} charm={t.charm:.4f} vomma={t.vomma:.4f} speed={t.speed:.4f} zomma={t.zomma:.4f} color={t.color:.4f} ultima={t.ultima:.4f}")
```
```typescript [TypeScript]
const data = tdx.optionHistoryGreeksAll('SPY', '20260417', '550', 'C', '20260315', '60000');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} implied_volatility=${t.implied_volatility} delta=${t.delta} gamma=${t.gamma} theta=${t.theta}`);
}
```
```cpp [C++]
auto data = client.option_history_greeks_all("SPY", "20260417", "550", "C", "20260315", "60000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d implied_volatility=%.4f delta=%.4f gamma=%.4f theta=%.4f vega=%.4f rho=%.4f vanna=%.4f charm=%.4f vomma=%.4f speed=%.4f zomma=%.4f color=%.4f ultima=%.4f\n",
        t.date, t.ms_of_day, t.implied_volatility, t.delta, t.gamma, t.theta, t.vega, t.rho, t.vanna, t.charm, t.vomma, t.speed, t.zomma, t.color, t.ultima);
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
<div class="param-desc">Expiration date in <code>YYYYMMDD</code> or <code>YYYY-MM-DD</code> format, or <code>"*"</code> for all expirations</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format</div>
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
<div class="param-header"><code>interval</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Sampling interval. Allowed values: <code>tick</code>, <code>10ms</code>, <code>100ms</code>, <code>500ms</code>, <code>1s</code>, <code>5s</code>, <code>10s</code>, <code>15s</code>, <code>30s</code>, <code>1m</code>, <code>5m</code>, <code>10m</code>, <code>15m</code>, <code>30m</code>, <code>1h</code>. Millisecond strings (e.g. <code>"60000"</code>) are accepted and snapped to the nearest preset. Default: <code>"1s"</code>. Sub-minute intervals are available only for single-day requests.</div>
</div>
<div class="param">
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time (inclusive) in <code>HH:MM:SS.SSS</code> ET wall-clock format. Default: <code>"09:30:00"</code>. Legacy millisecond strings are also accepted.</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time (inclusive) in <code>HH:MM:SS.SSS</code> ET wall-clock format. Default: <code>"16:00:00"</code>.</div>
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
<div class="param-header"><code>version</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Greeks methodology selector. Allowed values: <code>latest</code> (use real time-to-expiry), <code>1</code> (fix 0DTE to 0.15 days). Default: <code>"latest"</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Returns <code>n</code> strikes above and below spot price plus one ATM strike (up to <code>2n + 1</code> strikes).</div>
</div>
<div class="param">
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format. Use with <code>end_date</code> for multi-day requests. The <code>date</code> argument overrides <code>start_date</code>/<code>end_date</code> when present.</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format.</div>
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
  {
    "date": 20260402, "ms_of_day": 34260000,
    "implied_volatility": 0.4445, "delta": 0.9686, "gamma": 0.000892,
    "theta": -0.1898, "vega": 9.2497, "rho": 21.7056,
    "vanna": -0.0312, "charm": 0.0025, "vomma": 0.0198,
    "speed": -0.0000112, "zomma": 0.0000612, "color": -0.0000042, "ultima": 0.000168
  },
  {
    "date": 20260402, "ms_of_day": 34320000,
    "implied_volatility": 0.4350, "delta": 0.9718, "gamma": 0.000815,
    "theta": -0.1757, "vega": 8.4631, "rho": 21.7944
  }
]
```

> All Greeks at 1-minute intervals for SPY 2026-04-17 550 call. Requires Professional subscription.

## Notes

- Multi-day requests are limited to one calendar month.
- If you only need a subset of Greeks, use [greeks-first-order](./greeks-first-order), [greeks-second-order](./greeks-second-order), or [greeks-third-order](./greeks-third-order) to reduce payload size.
