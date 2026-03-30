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
let bars: Vec<OhlcTick> = client.index_history_ohlc(
    "SPX", "20240101", "20240301", "60000"  // 1-minute bars
).await?;
for bar in &bars {
    println!("{} {}: O={} H={} L={} C={}",
        bar.date, bar.ms_of_day, bar.open_price(), bar.high_price(),
        bar.low_price(), bar.close_price());
}
```
```python [Python]
bars = client.index_history_ohlc("SPX", "20240101", "20240301", "60000")
print(f"{len(bars)} 1-minute bars")

# 5-minute bars
bars_5m = client.index_history_ohlc("SPX", "20240101", "20240301", "300000")
```
```go [Go]
bars, err := client.IndexHistoryOHLC("SPX", "20240101", "20240301", "60000")
if err != nil {
    log.Fatal(err)
}
fmt.Printf("%d bars\n", len(bars))
```
```cpp [C++]
auto bars = client.index_history_ohlc("SPX", "20240101", "20240301", "60000");
for (auto& bar : bars) {
    std::cout << bar.date << " " << bar.ms_of_day
              << ": O=" << bar.open << " C=" << bar.close << std::endl;
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
<div class="param-desc">Bar interval in milliseconds (e.g. <code>"60000"</code> for 1-minute)</div>
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

## Notes

- Common intervals: `"60000"` (1 min), `"300000"` (5 min), `"900000"` (15 min), `"3600000"` (1 hour).
- Use `start_time` and `end_time` to filter to regular trading hours only (e.g. `"34200000"` to `"57600000"` for 9:30 AM to 4:00 PM ET).
- For end-of-day data only, use [index_history_eod](./eod) instead.
