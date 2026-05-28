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
    print(f"date={t.date} ms_of_day={t.ms_of_day} gamma={t.gamma:.4f} "
          f"vanna={t.vanna:.4f} charm={t.charm:.4f} vomma={t.vomma:.4f} veta={t.veta:.4f}")
```
```typescript [TypeScript]
const data = tdx.optionHistoryGreeksSecondOrder('SPY', '20260417', '550', 'C', '20260315', '60000');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} gamma=${t.gamma} vanna=${t.vanna} charm=${t.charm} vomma=${t.vomma}`);
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

