---
title: index_snapshot_price
description: Latest price snapshot for one or more indices.
---

# index_snapshot_price

<TierBadge tier="value" />

Get the latest price snapshot for one or more index symbols. Returns the most recent price data as a raw DataTable.

## Code Example

::: code-group
```rust [Rust]
let table: proto::DataTable = client.index_snapshot_price(&["SPX", "NDX"]).await?;
```
```python [Python]
price = client.index_snapshot_price(["SPX", "NDX"])
```
```go [Go]
price, err := client.IndexSnapshotPrice([]string{"SPX"})
if err != nil {
    log.Fatal(err)
}
```
```cpp [C++]
auto price = client.index_snapshot_price({"SPX"});
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

Returns a `DataTable` with price fields:

<div class="param-list">
<div class="param">
<div class="param-header"><code>price</code><span class="param-type">f64</span></div>
<div class="param-desc">Current index price/level</div>
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

- Returns raw `DataTable` (protobuf) rather than typed ticks.
- For OHLC-structured data, use [index_snapshot_ohlc](./ohlc) instead.
