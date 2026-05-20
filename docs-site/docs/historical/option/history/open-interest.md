---
title: option_history_open_interest
description: Open interest history for an option contract.
---

# option_history_open_interest

<TierBadge tier="value" />

Retrieve open interest history for an option contract.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_open_interest("SPY", "20260417", "550", "C", "20260315").await?;
for t in &data {
    println!("date={} open_interest={}", t.date, t.open_interest);
}
```
```python [Python]
data = tdx.option_history_open_interest("SPY", "20260417", "550", "C", "20260315")
for t in data:
    print(f"date={t.date} open_interest={t.open_interest}")
```
```typescript [TypeScript]
const data = tdx.optionHistoryOpenInterest('SPY', '20260417', '550', 'C', '20260315');
for (const t of data) {
    console.log(`date=${t.date} open_interest=${t.open_interest}`);
}
```
```cpp [C++]
auto data = client.option_history_open_interest("SPY", "20260417", "550", "C", "20260315");
for (const auto& t : data) {
    printf("date=%d open_interest=%d\n", t.date, t.open_interest);
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
<div class="param-header"><code>max_dte</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Maximum days to expiration. Filters contracts returned when <code>expiration="*"</code>.</div>
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
<div class="param-header"><code>open_interest</code><span class="param-type">int</span></div>
<div class="param-desc">Open interest</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span></div>
<div class="param-desc">Date</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260401, "open_interest": 28},
  {"date": 20260402, "open_interest": 32}
]
```

> Daily open interest for a specific option contract.

## Notes

- Open interest is typically reported once per day based on the previous day's settlement.
