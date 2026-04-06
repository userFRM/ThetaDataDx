---
title: History OHLC
description: Intraday OHLC bars for a single date or across a date range.
---

# stock_history_ohlc / stock_history_ohlc_range

Intraday OHLC bars at a configurable interval. Two variants are available:

- **stock_history_ohlc** - bars for a single date
- **stock_history_ohlc_range** - bars across a date range

<TierBadge tier="value" />

## Code Example (Single Date)

::: code-group
```rust [Rust]
let data = tdx.stock_history_ohlc("SPY", "20260315", "60000").await?;
for t in &data {
    println!("date={} ms_of_day={} open={:.2} high={:.2} low={:.2} close={:.2} volume={} count={}",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close, t.volume, t.count);
}
```
```python [Python]
data = tdx.stock_history_ohlc("SPY", "20260315", "60000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} open={t['open']:.2f} high={t['high']:.2f} "
          f"low={t['low']:.2f} close={t['close']:.2f} volume={t['volume']} count={t['count']}")
```
```go [Go]
data, _ := client.StockHistoryOHLC("SPY", "20260315", "60000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d count=%d\n",
        t.Date, t.MsOfDay, t.Open, t.High, t.Low, t.Close, t.Volume, t.Count)
}
```
```cpp [C++]
auto data = client.stock_history_ohlc("SPY", "20260315", "60000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d count=%d\n",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close, t.volume, t.count);
}
```
:::

## Code Example (Date Range)

::: code-group
```rust [Rust]
let data = tdx.stock_history_ohlc_range("SPY", "20260101", "20260301", "300000").await?;
for t in &data {
    println!("date={} ms_of_day={} open={:.2} high={:.2} low={:.2} close={:.2} volume={} count={}",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close, t.volume, t.count);
}
```
```python [Python]
data = tdx.stock_history_ohlc_range("SPY", "20260101", "20260301", "300000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} open={t['open']:.2f} high={t['high']:.2f} "
          f"low={t['low']:.2f} close={t['close']:.2f} volume={t['volume']} count={t['count']}")
```
```go [Go]
data, _ := client.StockHistoryOHLCRange("SPY", "20260101", "20260301", "300000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d count=%d\n",
        t.Date, t.MsOfDay, t.Open, t.High, t.Low, t.Close, t.Volume, t.Count)
}
```
```cpp [C++]
auto data = client.stock_history_ohlc_range("SPY", "20260101", "20260301", "300000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d count=%d\n",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close, t.volume, t.count);
}
```
:::

## Parameters (Single Date)

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Ticker symbol</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>interval</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Accepts milliseconds (<code>"60000"</code>) or shorthand (<code>"1m"</code>). Valid presets: <code>100ms</code>, <code>500ms</code>, <code>1s</code>, <code>5s</code>, <code>10s</code>, <code>15s</code>, <code>30s</code>, <code>1m</code>, <code>5m</code>, <code>10m</code>, <code>15m</code>, <code>30m</code>, <code>1h</code>.</div>
</div>
<div class="param">
<div class="param-header"><code>start_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Start time as milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time as milliseconds from midnight ET</div>
</div>
<div class="param">
<div class="param-header"><code>venue</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Data venue filter</div>
</div>
</div>

## Parameters (Date Range)

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbol</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Ticker symbol</div>
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
</div>

## Response Fields (OhlcTick)

<div class="param-list">
<div class="param">
<div class="param-header"><code>ms_of_day</code><span class="param-type">i32</span></div>
<div class="param-desc">Bar start time (milliseconds from midnight ET)</div>
</div>
<div class="param">
<div class="param-header"><code>open</code> / <code>high</code> / <code>low</code> / <code>close</code><span class="param-type">i32</span></div>
<div class="param-desc">OHLC prices (<code>f64</code>, decoded at parse time).</div>
</div>
<div class="param">
<div class="param-header"><code>volume</code><span class="param-type">i32</span></div>
<div class="param-desc">Total volume in the bar</div>
</div>
<div class="param">
<div class="param-header"><code>count</code><span class="param-type">i32</span></div>
<div class="param-desc">Number of trades in the bar</div>
</div>
<div class="param">
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 34200000, "open": 646.42, "high": 647.47, "low": 646.38, "close": 646.86, "volume": 727186, "count": 10708},
  {"date": 20260402, "ms_of_day": 34260000, "open": 646.85, "high": 647.44, "low": 646.69, "close": 647.37, "volume": 282666, "count": 3331},
  {"date": 20260402, "ms_of_day": 34320000, "open": 647.35, "high": 647.39, "low": 646.57, "close": 646.63, "volume": 329227, "count": 3967}
]
```

> SPY 1-minute bars on 2026-04-02. Full response contains 391 bars.

## Common Intervals

| Shorthand | Milliseconds |
|-----------|-------------|
| `"1m"` | `"60000"` |
| `"5m"` | `"300000"` |
| `"15m"` | `"900000"` |
| `"1h"` | `"3600000"` |

Milliseconds are auto-converted to the nearest valid preset internally. Either form can be used.

## Notes

- Use the single-date variant for intraday analysis of a specific session.
- Use the range variant for building multi-day bar charts or backtesting.
- Optional `start_time` / `end_time` parameters (single-date variant only) let you filter to regular trading hours or a custom window.
