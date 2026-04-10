---
title: option_snapshot_ohlc
description: Latest OHLC snapshot for an option contract.
---

# option_snapshot_ohlc

<TierBadge tier="free" />

Get the latest OHLC (open, high, low, close) snapshot for an option contract.

## Code Example

::: code-group
```rust [Rust]
let data = tdx.option_snapshot_ohlc("SPY", "20260417", "550", "C").await?;
for t in &data {
    println!("date={} ms_of_day={} open={:.2} high={:.2} low={:.2} close={:.2} volume={} expiration={} strike={:.2}",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close, t.volume, t.expiration, t.strike);
}
```
```python [Python]
data = tdx.option_snapshot_ohlc("SPY", "20260417", "550", "C")
for t in data:
    print(f"date={t['date']} ms_of_day={t['ms_of_day']} open={t['open']:.2f} high={t['high']:.2f} "
          f"low={t['low']:.2f} close={t['close']:.2f} volume={t['volume']} expiration={t['expiration']} strike={t['strike']:.2f}")
```
```go [Go]
data, _ := client.OptionSnapshotOHLC("SPY", "20260417", "550", "C")
for _, t := range data {
    fmt.Printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d expiration=%d strike=%.2f\n",
        t.Date, t.MsOfDay, t.Open, t.High, t.Low, t.Close, t.Volume, t.Expiration, t.Strike)
}
```
```cpp [C++]
auto data = client.option_snapshot_ohlc("SPY", "20260417", "550", "C");
for (const auto& t : data) {
    printf("date=%d ms_of_day=%d open=%.2f high=%.2f low=%.2f close=%.2f volume=%d expiration=%d strike=%.2f\n",
        t.date, t.ms_of_day, t.open, t.high, t.low, t.close, t.volume, t.expiration, t.strike);
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
<div class="param-desc">Strike price in dollars as a string (e.g. <code>"500"</code> or <code>"17.5"</code>)</div>
</div>
<div class="param">
<div class="param-header"><code>right</code><span class="param-type">string</span><span class="param-badge required">required</span></div>
<div class="param-desc"><code>"C"</code> for call, <code>"P"</code> for put</div>
</div>
<div class="param">
<div class="param-header"><code>max_dte</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Maximum days to expiration</div>
</div>
<div class="param">
<div class="param-header"><code>strike_range</code><span class="param-type">int</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Strike range filter</div>
</div>
<div class="param">
<div class="param-header"><code>min_time</code><span class="param-type">string</span><span class="param-badge optional">optional</span></div>
<div class="param-desc">Minimum time of day as milliseconds from midnight</div>
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
<div class="param-desc">Volume</div>
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
  {"date": 20260402, "ms_of_day": 34203497, "open": 98.59, "high": 98.59, "low": 98.59, "close": 98.59, "volume": 1, "expiration": 20260417, "strike": 550.0}
]
```

> SPY 2026-04-17 550 call OHLC snapshot. Wildcard queries return multiple contracts with `expiration`, `strike`, and `right` fields populated.

