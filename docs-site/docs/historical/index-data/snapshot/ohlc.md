---
title: index_snapshot_ohlc
description: Latest OHLC snapshot for one or more indices.
---

# index_snapshot_ohlc

<TierBadge tier="standard" />

Get the latest OHLC (open, high, low, close) snapshot for one or more index symbols. Returns the most recent bar data.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.index_snapshot_ohlc(&["SPX", "VIX"]).await?;
for t in &data {
    println!("date={} ms_of_day={} open={:.2} high={:.2} low={:.2} close={:.2} volume={}",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close, t.volume);
}
```
```python [Python]
data = tdx.index_snapshot_ohlc(["SPX", "VIX"])
for t in data:
    print(f"date={t.date} ms_of_day={t.ms_of_day} open={t.open:.2f} "
          f"high={t.high:.2f} low={t.low:.2f} close={t.close:.2f} volume={t.volume}")
```
```typescript [TypeScript]
const data = tdx.indexSnapshotOhlc(['SPX', 'VIX']);
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} open=${t.open} high=${t.high} low=${t.low} close=${t.close}`);
}
```
```go [Go]
data, _ := client.IndexSnapshotOHLC([]string{"SPX", "VIX"})
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d\n",
        t.Date, t.MsOfDay, t.Open, t.High, t.Low, t.Close, t.Volume)
}
```
```cpp [C++]
auto data = client.index_snapshot_ohlc({"SPX", "VIX"});
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d\n",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close, t.volume);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbols</code><span class="param-type">string[]</span><span class="param-badge required">required</span></div>
<div class="param-desc">One or more index symbols</div>
</div>
<div class="param">
<div class="param-header"><code>min_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Minimum time of day as milliseconds from midnight</div>
</div>
</div>

## Response

<div class="param-list">
<div class="param">
<div class="param-header"><code>open</code><span class="param-type">f64</span></div>
<div class="param-desc">Opening price</div>
</div>
<div class="param">
<div class="param-header"><code>high</code><span class="param-type">f64</span></div>
<div class="param-desc">High price</div>
</div>
<div class="param">
<div class="param-header"><code>low</code><span class="param-type">f64</span></div>
<div class="param-desc">Low price</div>
</div>
<div class="param">
<div class="param-header"><code>close</code><span class="param-type">f64</span></div>
<div class="param-desc">Closing price</div>
</div>
<div class="param">
<div class="param-header"><code>volume</code><span class="param-type">u64</span></div>
<div class="param-desc">Volume</div>
</div>
<div class="param">
<div class="param-header"><code>count</code><span class="param-type">u32</span></div>
<div class="param-desc">Number of trades in bar</div>
</div>
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">u32</span></div>
<div class="param-desc">Milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">u32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 57600000, "open": 6820.51, "high": 6916.23, "low": 6785.14, "close": 6899.87, "volume": 0},
  {"date": 20260402, "ms_of_day": 57600000, "open": 21045.88, "high": 21312.15, "low": 20891.34, "close": 21256.72, "volume": 0}
]
```

> OHLC snapshot for SPX and NDX. Indices do not have volume data.

## Notes

- Pass multiple symbols in a single call to batch requests efficiently.
- During market hours, the snapshot reflects the current partial bar.
