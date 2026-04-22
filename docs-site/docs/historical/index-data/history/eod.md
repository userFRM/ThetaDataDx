---
title: index_history_eod
description: End-of-day index data across a date range.
---

# index_history_eod

<TierBadge tier="free" />

Retrieve end-of-day data for an index across a date range. Returns one row per trading day with open, high, low, close, and volume.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.index_history_eod("SPX", "20260101", "20260301").await?;
for t in &data {
    println!("date={} open={:.2} high={:.2} low={:.2} close={:.2} volume={}",
        t.date, t.open, t.high, t.low, t.close, t.volume);
}
```
```python [Python]
data = tdx.index_history_eod("SPX", "20260101", "20260301")
for t in data:
    print(f"date={t.date} open={t.open:.2f} high={t.high:.2f} "
          f"low={t.low:.2f} close={t.close:.2f} volume={t.volume}")
```
```typescript [TypeScript]
const data = tdx.indexHistoryEod('SPX', '20260101', '20260301');
for (const t of data) {
    console.log(`date=${t.date} open=${t.open} high=${t.high} low=${t.low} close=${t.close} volume=${t.volume}`);
}
```
```go [Go]
data, _ := client.IndexHistoryEOD("SPX", "20260101", "20260301")
for _, t := range data {
    fmt.Printf("date=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d\n",
        t.Date, t.Open, t.High, t.Low, t.Close, t.Volume)
}
```
```cpp [C++]
auto data = client.index_history_eod("SPX", "20260101", "20260301");
for (const auto& t : data) {
    printf("date=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d\n",
        t.date, t.open, t.high, t.low, t.close, t.volume);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Index symbol (e.g. <code>"SPX"</code>)</div>
</div>
<div class="param">
<div class="param-header"><code>start_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Start date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>end_date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">End date in <code>YYYYMMDD</code> format</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">u32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
<div class="param">
<div class="param-header"><code>open</code><span class="param-type">f64</span></div>
<div class="param-desc">Opening price/level</div>
</div>
<div class="param">
<div class="param-header"><code>high</code><span class="param-type">f64</span></div>
<div class="param-desc">High price/level</div>
</div>
<div class="param">
<div class="param-header"><code>low</code><span class="param-type">f64</span></div>
<div class="param-desc">Low price/level</div>
</div>
<div class="param">
<div class="param-header"><code>close</code><span class="param-type">f64</span></div>
<div class="param-desc">Closing price/level</div>
</div>
<div class="param">
<div class="param-header"><code>volume</code><span class="param-type">u64</span></div>
<div class="param-desc">Volume</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260302, "open": 6824.36, "high": 6901.01, "low": 6796.85, "close": 6881.62, "volume": 0},
  {"date": 20260303, "open": 6800.26, "high": 6840.05, "low": 6710.42, "close": 6816.63, "volume": 0},
  {"date": 20260304, "open": 6831.69, "high": 6885.94, "low": 6811.64, "close": 6869.50, "volume": 0}
]
```

> SPX end-of-day data for March 2026. Full response contains 24 rows.

## Notes

- Returns one row per trading day in the range. Non-trading days are excluded.
- Python users chain `.to_pandas()` on the return value for a pandas DataFrame.
