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
let eod: Vec<EodTick> = tdx.index_history_eod("SPX", "20240101", "20240301").await?;
for t in &eod {
    println!("{}: O={} H={} L={} C={}",
        t.date, t.open_price(), t.high_price(), t.low_price(), t.close_price());
}
```
```python [Python]
eod = tdx.index_history_eod("SPX", "20240101", "20240301")
for tick in eod:
    print(f"{tick['date']}: O={tick['open']:.2f} C={tick['close']:.2f}")

# DataFrame variant
df = tdx.index_history_eod_df("SPX", "20240101", "20240301")
print(df.describe())
```
```go [Go]
eod, err := client.IndexHistoryEOD("SPX", "20240101", "20240301")
if err != nil {
    log.Fatal(err)
}
for _, tick := range eod {
    fmt.Printf("%d: O=%.2f H=%.2f L=%.2f C=%.2f\n",
        tick.Date, tick.Open, tick.High, tick.Low, tick.Close)
}
```
```cpp [C++]
auto eod = client.index_history_eod("SPX", "20240101", "20240301");
for (auto& tick : eod) {
    std::cout << tick.date << ": O=" << tick.open << " H=" << tick.high
              << " L=" << tick.low << " C=" << tick.close << std::endl;
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

## Notes

- Returns one row per trading day in the range. Non-trading days are excluded.
- Python users can use the `_df` variant to get a pandas DataFrame directly.
