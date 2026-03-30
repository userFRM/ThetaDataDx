---
title: index_snapshot_market_value
description: Latest market value snapshot for one or more indices.
---

# index_snapshot_market_value

<TierBadge tier="value" />

Get the latest market value snapshot for one or more index symbols.

## Code Example

::: code-group
```rust [Rust]
let ticks: Vec<MarketValueTick> = tdx.index_snapshot_market_value(&["SPX"]).await?;
```
```python [Python]
mv = client.index_snapshot_market_value(["SPX"])
```
```go [Go]
mv, err := client.IndexSnapshotMarketValue([]string{"SPX"})
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto mv = client.index_snapshot_market_value({"SPX"});
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

Returns a `Vec<MarketValueTick>` with market value fields:

<div class="param-list">
<div class="param">
<div class="param-header"><code>market_value</code><span class="param-type">f64</span></div>
<div class="param-desc">Market capitalization / value</div>
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

- Returns `Vec<MarketValueTick>` in Rust.
- Market value represents the total capitalization of the index constituents.
