---
title: option_history_ohlc
description: Intraday OHLC bars for an option contract.
---

# option_history_ohlc

<TierBadge tier="free" />

Retrieve intraday OHLC bars for an option contract on a given date at a specified interval.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_history_ohlc("SPY", "20260417", "550", "C", "20260315", "60000").await?;
for t in &data {
    println!("date={} ms_of_day={} open={:.2} high={:.2} low={:.2} close={:.2} volume={} count={}",
        t.date, t.ms_of_day, t.open_f64(), t.high_f64(), t.low_f64(), t.close_f64(), t.volume, t.count);
}
```
```python [Python]
data = tdx.option_history_ohlc("SPY", "20260417", "550", "C", "20260315", "60000")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} open={t['open']:.2f} high={t['high']:.2f} "
          f"low={t['low']:.2f} close={t['close']:.2f} volume={t['volume']} count={t['count']}")
```
```go [Go]
data, _ := client.OptionHistoryOHLC("SPY", "20260417", "550", "C", "20260315", "60000")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d count=%d\n",
        t.Date, t.MsOfDay, t.Open, t.High, t.Low, t.Close, t.Volume, t.Count)
}
```
```cpp [C++]
auto data = client.option_history_ohlc("SPY", "20260417", "550", "C", "20260315", "60000");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d count=%d\n",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close, t.volume, t.count);
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
<div class="param-desc">Expiration date in <code>YYYYMMDD</code> format</div>
</div>
<div class="param">
<div class="param-header"><code>strike</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc">Strike price as scaled integer</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc"><code>"C"</code> for call, <code>"P"</code> for put</div>
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
<div class="param-desc">Start time as milliseconds from midnight</div>
</div>
<div class="param">
<div class="param-header"><code>end_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">End time as milliseconds from midnight</div>
</div>
<div class="param">
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike range filter</div>
</div>
</div>

## Response

<div class="param-list">
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
<div class="param-desc">Volume in interval</div>
</div>
<div class="param">
<div class="param-header"><code>count</code><span class="param-type">int</span></div>
<div class="param-desc">Number of trades</div>
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
  {"date": 20260402, "ms_of_day": 34200000, "open": 98.59, "high": 98.59, "low": 98.59, "close": 98.59, "volume": 1, "count": 1},
  {"date": 20260402, "ms_of_day": 34260000, "open": 0.00, "high": 0.00, "low": 0.00, "close": 0.00, "volume": 0, "count": 0},
  {"date": 20260402, "ms_of_day": 34320000, "open": 0.00, "high": 0.00, "low": 0.00, "close": 0.00, "volume": 0, "count": 0}
]
```

> 1-minute OHLC bars for SPY 2026-04-17 550 call. Deep ITM options are illiquid -- most bars show zero volume.

## Notes

- Shorthand is supported: `"1m"`, `"5m"`, `"15m"`, `"1h"`. Milliseconds (`"60000"`, `"300000"`, `"900000"`, `"3600000"`) are auto-converted to the nearest valid preset.
