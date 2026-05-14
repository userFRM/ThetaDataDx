---
title: option_snapshot_open_interest
description: Latest open interest snapshot for an option contract.
---

# option_snapshot_open_interest

<TierBadge tier="value" />

Get the latest open interest snapshot for an option contract.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_snapshot_open_interest("SPY", "20260417", "550", "C").await?;
for t in &data {
    println!("date={} open_interest={} expiration={} strike={:.2}",
        t.date, t.open_interest, t.expiration, t.strike);
}
```
```python [Python]
data = tdx.option_snapshot_open_interest("SPY", "20260417", "550", "C")
for t in data:
    print(f"date={t.date} open_interest={t.open_interest} "
          f"expiration={t.expiration} strike={t.strike:.2f}")
```
```typescript [TypeScript]
const data = tdx.optionSnapshotOpenInterest('SPY', '20260417', '550', 'C');
for (const t of data) {
    console.log(`date=${t.date} open_interest=${t.open_interest} expiration=${t.expiration} strike=${t.strike}`);
}
```
```cpp [C++]
auto data = client.option_snapshot_open_interest("SPY", "20260417", "550", "C");
for (const auto& t : data) {
    printf("date=%d open_interest=%d expiration=%d strike=%.2f\n",
        t.date, t.open_interest, t.expiration, t.strike);
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
<div class="param-header"><code>min_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Filters snapshots to timestamps at or after this ET wall-clock time. Format <code>HH:MM:SS.SSS</code>; legacy millisecond strings are also accepted.</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>open_interest</code><span class="param-type">int</span></div>
<div class="param-desc">Current open interest</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span></div>
<div class="param-desc">Date</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260402, "open_interest": 32, "expiration": 20260417, "strike": 550.0}
]
```

> Open interest for SPY 2026-04-17 550 call.

## Notes

- Open interest is reported once per day, typically reflecting the previous day's settlement.
