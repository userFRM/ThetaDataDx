---
title: Snapshot Market Value
description: Latest market value snapshot for one or more stocks.
---

# stock_snapshot_market_value

Latest market value snapshot for one or more stocks.

<TierBadge tier="value" />

## Code Example

::: code-group
```rust [Rust]
let ticks: Vec<MarketValueTick> = tdx.stock_snapshot_market_value(&["AAPL"]).await?;
```
```python [Python]
mv = tdx.stock_snapshot_market_value(["AAPL"])
```
```go [Go]
mv, err := client.StockSnapshotMarketValue([]string{"AAPL"})
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto mv = client.stock_snapshot_market_value({"AAPL"});
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

## Response

`Vec<MarketValueTick>` with market value fields. The exact fields depend on the data available for the requested symbols.

## Notes

- Accepts multiple symbols in a single call.
- Returns `Vec<MarketValueTick>` in Rust.
