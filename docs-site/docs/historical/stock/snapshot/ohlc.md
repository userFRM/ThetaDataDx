---
title: Snapshot OHLC
description: Latest OHLC bar snapshot for one or more stocks.
---

# stock_snapshot_ohlc

Latest OHLC (open-high-low-close) snapshot for one or more stocks. Returns the current or most recent trading session's aggregated bar.

<TierBadge tier="value" />

## Code Example

::: code-group
```rust [Rust]
let data = tdx.stock_snapshot_ohlc(&["SPY", "MSFT"]).await?;
for t in &data {
    println!("date={} ms_of_day={} open={:.2} high={:.2} low={:.2} close={:.2} volume={} count={}",
        t.date, t.ms_of_day, t.open_f64(), t.high_f64(), t.low_f64(), t.close_f64(), t.volume, t.count);
}
```
```python [Python]
data = tdx.stock_snapshot_ohlc(["SPY", "MSFT"])
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} open={t['open']:.2f} high={t['high']:.2f} "
          f"low={t['low']:.2f} close={t['close']:.2f} volume={t['volume']} count={t['count']}")
```
```go [Go]
data, _ := client.StockSnapshotOHLC([]string{"SPY", "MSFT"})
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d count=%d\n",
        t.Date, t.MsOfDay, t.Open, t.High, t.Low, t.Close, t.Volume, t.Count)
}
```
```cpp [C++]
auto data = client.stock_snapshot_ohlc({"SPY", "MSFT"});
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d count=%d\n",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close, t.volume, t.count);
}
```
:::

## Parameters

<div class="param-list">
<div class="param">
<div class="param-header"><code>symbols</code><span class="param-type">string[]</span><span class="param-badge required">required</span></div>
<div class="param-desc">One or more ticker symbols</div>
</div>
<div class="param">
<div class="param-header"><code>venue</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Data venue filter</div>
</div>
<div class="param">
<div class="param-header"><code>min_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Minimum time of day as milliseconds from midnight ET</div>
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
<div class="param-desc">Fixed-point OHLC prices. Use <code>open_price()</code>, <code>high_price()</code>, <code>low_price()</code>, <code>close_price()</code> for decoded values.</div>
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
<div class="param-header"><code>price_type</code><span class="param-type">i32</span></div>
<div class="param-desc">Decimal type for price decoding</div>
</div>
<div class="param">
<div class="param-header"><code>date</code><span class="param-type">i32</span></div>
<div class="param-desc">Date as <code>YYYYMMDD</code> integer</div>
</div>
</div>


### Sample Response

```json
[
  {"date": 20260402, "ms_of_day": 71999485, "open": 367.20, "high": 373.60, "low": 364.15, "close": 373.46, "volume": 18370273, "count": 440751},
  {"date": 20260402, "ms_of_day": 71983113, "open": 254.14, "high": 256.13, "low": 250.65, "close": 255.92, "volume": 24286541, "count": 407209},
  {"date": 20260402, "ms_of_day": 71998357, "open": 646.42, "high": 657.92, "low": 645.11, "close": 655.94, "volume": 38832397, "count": 510918}
]
```

## Notes

- Accepts multiple symbols in a single call. Batch requests to reduce round-trips.
- Prices are stored as fixed-point integers. Use the helper methods to get decoded float values.
