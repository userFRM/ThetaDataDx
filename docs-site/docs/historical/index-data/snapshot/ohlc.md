---
title: index_snapshot_ohlc
description: Latest OHLC snapshot for one or more indices.
---

# index_snapshot_ohlc

<TierBadge tier="value" />

Get the latest OHLC (open, high, low, close) snapshot for one or more index symbols. Returns the most recent bar data.

## Code Example

::: code-group
```rust [Rust]
let bars: Vec<OhlcTick> = tdx.index_snapshot_ohlc(&["SPX", "VIX"]).await?;
for bar in &bars {
    println!("O={} H={} L={} C={}", bar.open_price(), bar.high_price(), bar.low_price(), bar.close_price());
}
```
```python [Python]
bars = tdx.index_snapshot_ohlc(["SPX", "VIX"])
for bar in bars:
    print(f"O={bar['open']:.2f} H={bar['high']:.2f} L={bar['low']:.2f} C={bar['close']:.2f}")
```
```go [Go]
bars, err := client.IndexSnapshotOHLC([]string{"SPX", "VIX"})
if err != nil {
    log.Fatal(err)
}
for _, bar := range bars {
    fmt.Printf("O=%.2f H=%.2f L=%.2f C=%.2f\n", bar.Open, bar.High, bar.Low, bar.Close)
}
```
```cpp [C++]
auto bars = client.index_snapshot_ohlc({"SPX", "VIX"});
for (auto& bar : bars) {
    std::cout << "O=" << bar.open << " H=" << bar.high
              << " L=" << bar.low << " C=" << bar.close << std::endl;
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

## Notes

- Pass multiple symbols in a single call to batch requests efficiently.
- During market hours, the snapshot reflects the current partial bar.
