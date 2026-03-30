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
let bars: Vec<OhlcTick> = tdx.stock_snapshot_ohlc(&["AAPL", "MSFT"]).await?;
for bar in &bars {
    println!("O={} H={} L={} C={} V={}",
        bar.open_price(), bar.high_price(),
        bar.low_price(), bar.close_price(), bar.volume);
}
```
```python [Python]
bars = tdx.stock_snapshot_ohlc(["AAPL", "MSFT"])
for bar in bars:
    print(f"O={bar['open']:.2f} H={bar['high']:.2f} "
          f"L={bar['low']:.2f} C={bar['close']:.2f}")
```
```go [Go]
bars, err := client.StockSnapshotOHLC([]string{"AAPL", "MSFT"})
if err != nil {
    log.Fatal(err)
}
for _, bar := range bars {
    fmt.Printf("O=%.2f H=%.2f L=%.2f C=%.2f V=%d\n",
        bar.Open, bar.High, bar.Low, bar.Close, bar.Volume)
}
```
```cpp [C++]
auto bars = client.stock_snapshot_ohlc({"AAPL", "MSFT"});
for (auto& bar : bars) {
    std::cout << "O=" << bar.open << " H=" << bar.high
              << " L=" << bar.low << " C=" << bar.close
              << " V=" << bar.volume << std::endl;
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

## Notes

- Accepts multiple symbols in a single call. Batch requests to reduce round-trips.
- Prices are stored as fixed-point integers. Use the helper methods to get decoded float values.
