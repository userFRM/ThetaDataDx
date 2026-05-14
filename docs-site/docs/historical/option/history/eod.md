---
title: option_history_eod
description: End-of-day option data across a date range.
---

# option_history_eod

<TierBadge tier="free" />

Retrieve end-of-day option data across a date range. Returns one row per trading day with OHLC, volume, and open interest.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_eod("SPY", "20260417", "550", "C", "20260101", "20260301").await?;
for t in &data {
    println!("date={} open={:.2} high={:.2} low={:.2} close={:.2} volume={} bid={:.2} ask={:.2}",
        t.date, t.open, t.high, t.low, t.close, t.volume, t.bid, t.ask);
}
```
```python [Python]
data = tdx.option_history_eod("SPY", "20260417", "550", "C", "20260101", "20260301")
for t in data:
    print(f"date={t.date} open={t.open:.2f} high={t.high:.2f} low={t.low:.2f} "
          f"close={t.close:.2f} volume={t.volume} bid={t.bid:.2f} ask={t.ask:.2f}")
```
```typescript [TypeScript]
const data = tdx.optionHistoryEOD('SPY', '20260417', '550', 'C', '20260101', '20260301');
for (const t of data) {
    console.log(`date=${t.date} open=${t.open} high=${t.high} low=${t.low} close=${t.close} volume=${t.volume}`);
}
```
```cpp [C++]
auto data = client.option_history_eod("SPY", "20260417", "550", "C", "20260101", "20260301");
for (const auto& t : data) {
    printf("date=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d bid=%.2f ask=%.2f\n",
        t.date, t.open, t.high, t.low, t.close, t.volume, t.bid, t.ask);
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
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format (inclusive)</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format (inclusive)</div>
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
</div>

## Response

> `strike_range` filters a wildcard bulk request. If you pin `strike` to one contract, the response stays single-strike. Pass `strike="*"` (or omit `strike`, which now defaults to `*`) when you want multi-strike EOD output.

<div class="param-list">
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span></div>
<div class="param-desc">Trading date</div>
</div>
<div class="param">
<div class="param-header"><code>open</code><span class="param-type">float</span></div>
<div class="param-desc">Opening price</div>
</div>
<div class="param">
<div class="param-header"><code>high</code><span class="param-type">float</span></div>
<div class="param-desc">High price</div>
</div>
<div class="param">
<div class="param-header"><code>low</code><span class="param-type">float</span></div>
<div class="param-desc">Low price</div>
</div>
<div class="param">
<div class="param-header"><code>close</code><span class="param-type">float</span></div>
<div class="param-desc">Closing price</div>
</div>
<div class="param">
<div class="param-header"><code>volume</code><span class="param-type">int</span></div>
<div class="param-desc">Daily volume</div>
</div>
<div class="param">
<div class="param-header"><code>open_interest</code><span class="param-type">int</span></div>
<div class="param-desc">Open interest</div>
</div>
</div>

### Sample Response

```json
[
  {"date": 20260302, "open": 131.57, "high": 138.32, "low": 131.57, "close": 138.32, "volume": 2, "bid": 136.89, "ask": 139.70},
  {"date": 20260303, "open": 0.00, "high": 0.00, "low": 0.00, "close": 0.00, "volume": 0, "bid": 131.05, "ask": 133.87},
  {"date": 20260305, "open": 129.73, "high": 129.73, "low": 129.73, "close": 129.73, "volume": 1, "bid": 131.75, "ask": 134.55}
]
```

> EOD data for SPY 2026-04-17 550 call. Days with no trades show `0.00` for OHLC but still have closing bid/ask.
