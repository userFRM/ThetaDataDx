---
title: index_history_ohlc
description: Intraday OHLC bars for an index across a date range.
---

# index_history_ohlc

<TierBadge tier="standard" />

Retrieve intraday OHLC bars for an index across a date range at a specified interval.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.index_history_ohlc("SPX", "20260101", "20260301", "60000").await?;
for t in &data {
    println!("date={} ms_of_day={} open={:.2} high={:.2} low={:.2} close={:.2}",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close);
}
```
```python [Python]
data = tdx.index_history_ohlc("SPX", "20260101", "20260301", "60000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} open={t['open']:.2f} "
          f"high={t['high']:.2f} low={t['low']:.2f} close={t['close']:.2f}")
```
```typescript [TypeScript]
const data = tdx.indexHistoryOhlc('SPX', '20260101', '20260301', '60000');
for (const t of data) {
    console.log(`date=${t.date} ms_of_day=${t.ms_of_day} open=${t.open} high=${t.high} low=${t.low} close=${t.close}`);
}
```
```go [Go]
data, _ := client.IndexHistoryOHLC("SPX", "20260101", "20260301", "60000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f\n",
        t.Date, t.MsOfDay, t.Open, t.High, t.Low, t.Close)
}
```
```cpp [C++]
auto data = client.index_history_ohlc("SPX", "20260101", "20260301", "60000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f\n",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close);
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
<div class="param">
<div class="param-header"><code>interval</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Accepts milliseconds (<code>"60000"</code>) or shorthand (<code>"1m"</code>). Valid presets: <code>100ms</code>, <code>500ms</code>, <code>1s</code>, <code>5s</code>, <code>10s</code>, <code>15s</code>, <code>30s</code>, <code>1m</code>, <code>5m</code>, <code>10m</code>, <code>15m</code>, <code>30m</code>, <code>1h</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time of day as milliseconds from midnight</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time of day as milliseconds from midnight</div>
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
  {"date": 20260402, "ms_of_day": 34200000, "open": 6820.51, "high": 6825.40, "low": 6815.23, "close": 6823.18},
  {"date": 20260402, "ms_of_day": 34260000, "open": 6823.18, "high": 6830.55, "low": 6820.12, "close": 6828.42},
  {"date": 20260402, "ms_of_day": 34320000, "open": 6828.42, "high": 6835.67, "low": 6826.09, "close": 6834.15}
]
```

> SPX 1-minute OHLC bars. Requires Standard subscription.

## Notes

- Shorthand is supported: `"1m"`, `"5m"`, `"15m"`, `"1h"`. Milliseconds (`"60000"`, `"300000"`, `"900000"`, `"3600000"`) are auto-converted to the nearest valid preset.
- Use `start_time` and `end_time` to filter to regular trading hours only (e.g. `"34200000"` to `"57600000"` for 9:30 AM to 4:00 PM ET).
- For end-of-day data only, use [index_history_eod](./eod) instead.
